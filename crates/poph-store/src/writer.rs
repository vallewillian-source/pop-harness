use std::any::Any;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};
use tracing::debug;

use crate::connection::open_for_write;
use crate::error::{Resultado, StoreError};

type Boxed = Box<dyn Any + Send>;
type Task = Box<dyn FnOnce(&Connection) -> Resultado<Boxed> + Send>;

const MAX_BATCH: usize = 512;
const POLL_INTERVAL: Duration = Duration::from_millis(2);

struct Command {
    task: Task,
    response: Sender<Resultado<Boxed>>,
}

enum Mensagem {
    Command(Command),
    Shutdown(Sender<()>),
}

pub(crate) struct Writer {
    canal: Mutex<Option<Sender<Mensagem>>>,
    tratador: Option<JoinHandle<()>>,
}

impl Writer {
    pub fn start(path: PathBuf) -> Resultado<Self> {
        let mut conn = open_for_write(&path)?;
        let (canal, receiver) = mpsc::channel();
        let tratador = thread::Builder::new()
            .name("poph-writer".to_owned())
            .spawn(move || run_loop(&mut conn, &receiver))
            .map_err(|error| {
                StoreError::Internal(format!("não consegui criar a thread de escrita: {error}"))
            })?;
        let writer = Self {
            canal: Mutex::new(Some(canal)),
            tratador: Some(tratador),
        };
        writer.run(|conn| {
            conn.query_row("SELECT 1", [], |l| l.get::<_, i64>(0))
                .map_err(StoreError::from)
        })?;
        Ok(writer)
    }

    pub fn run<T: Send + 'static>(
        &self,
        task: impl FnOnce(&Connection) -> Resultado<T> + Send + 'static,
    ) -> Resultado<T> {
        let (response, recebedor) = mpsc::channel();
        let delivery = {
            let canal = self.canal.lock().unwrap_or_else(|e| e.into_inner());
            canal
                .as_ref()
                .ok_or(StoreError::WriterClosed)?
                .send(Mensagem::Command(Command {
                    task: Box::new(move |conn| task(conn).map(|value| Box::new(value) as Boxed)),
                    response,
                }))
        };
        delivery.map_err(|_| StoreError::WriterClosed)?;
        let boxed = recebedor.recv().map_err(|_| StoreError::WriterClosed)??;
        boxed
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| StoreError::Internal("o escritor devolveu um tipo inesperado".to_owned()))
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let canal = self
            .canal
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(canal) = canal {
            let (notice, wait) = mpsc::channel();
            if canal.send(Mensagem::Shutdown(notice)).is_ok() {
                let _ = wait.recv_timeout(Duration::from_secs(5));
            }
        }
        if let Some(tratador) = self.tratador.take()
            && let Err(error) = tratador.join()
        {
            debug!("thread de escrita terminou de forma inesperada: {error:?}");
        }
    }
}

fn run_loop(conn: &mut Connection, receiver: &Receiver<Mensagem>) {
    loop {
        let first = match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(mensagem) => mensagem,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH {
            match receiver.try_recv() {
                Ok(mensagem) => batch.push(mensagem),
                Err(_) => break,
            }
        }

        let mut commands = Vec::with_capacity(batch.len());
        let mut shutdown = None;
        for mensagem in batch {
            match mensagem {
                Mensagem::Command(command) => commands.push(command),
                Mensagem::Shutdown(notice) => shutdown = Some(notice),
            }
        }

        if !commands.is_empty() {
            run_batch(conn, commands);
        }
        if let Some(notice) = shutdown {
            let _ = notice.send(());
            return;
        }
    }
}

fn run_batch(conn: &mut Connection, commands: Vec<Command>) {
    let transaction = match conn.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(transaction) => transaction,
        Err(error) => {
            let reason = error.to_string();
            for command in commands {
                let _ = command.response.send(Err(StoreError::Internal(format!(
                    "não consegui abrir a transação: {reason}"
                ))));
            }
            return;
        }
    };

    let mut responses = Vec::with_capacity(commands.len());
    for command in commands {
        let result = match transaction.execute_batch("SAVEPOINT poph_command") {
            Ok(()) => {
                let result = (command.task)(&transaction);
                let desfecho = if result.is_ok() {
                    transaction.execute_batch("RELEASE poph_command")
                } else {
                    transaction.execute_batch("ROLLBACK TO poph_command; RELEASE poph_command")
                };
                match desfecho {
                    Ok(()) => result,
                    Err(error) => Err(StoreError::Internal(format!(
                        "não consegui fechar o savepoint: {error}"
                    ))),
                }
            }
            Err(error) => Err(StoreError::Sqlite(error)),
        };
        responses.push((command.response, result));
    }

    match transaction.commit() {
        Ok(()) => {
            for (response, result) in responses {
                let _ = response.send(result);
            }
        }
        Err(error) => {
            let reason = error.to_string();
            debug!(reason, "commit do lote falhou");
            for (response, _) in responses {
                let _ = response.send(Err(StoreError::Internal(format!(
                    "commit do lote falhou; nenhuma escrita desta chamada foi gravada: {reason}"
                ))));
            }
        }
    }
}
