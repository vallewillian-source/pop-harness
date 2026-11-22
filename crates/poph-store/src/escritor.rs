//! Escritor dedicado: uma thread recebe comandos por canal e grava em lote.
//!
//! Por que assim (e não uma conexão por chamada):
//! - SQLite tem **um escritor por vez**. Um canal único elimina `SQLITE_BUSY` entre escritas da
//!   própria instância e deixa o agrupamento acontecer de graça;
//! - um lote = uma transação (`BEGIN IMMEDIATE`), que é o que faz a vazão subir de ~17 mil para
//!   ~118 mil mensagens/s (medido na HR-01);
//! - cada comando roda dentro de um `SAVEPOINT`: um erro de um não derruba os outros do lote;
//! - as respostas só são entregues **depois** do commit: nenhum chamador recebe "ok" de uma
//!   escrita que não foi para o disco.

use std::any::Any;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};
use tracing::debug;

use crate::conexao::abrir_escrita;
use crate::erro::{ErroStore, Resultado};

type Caixa = Box<dyn Any + Send>;
type Tarefa = Box<dyn FnOnce(&Connection) -> Resultado<Caixa> + Send>;

/// Quantos comandos no máximo entram numa transação.
///
/// Medido com 256 concorrentes (`tests/vazao.rs`, release): 256 → 55 mil msg/s, 512 → 60 mil,
/// 1024 → 60 mil. Acima de 512 não ganha, então fica em 512 (menos memória por lote).
const LOTE_MAX: usize = 512;
/// Quanto tempo o escritor espera o primeiro comando antes de considerar o lote fechado.
const ESPERA: Duration = Duration::from_millis(2);

struct Comando {
    tarefa: Tarefa,
    resposta: Sender<Resultado<Caixa>>,
}

enum Mensagem {
    Comando(Comando),
    Encerrar(Sender<()>),
}

/// Dono da thread de escrita.
pub(crate) struct Escritor {
    /// `Sender` do std não é `Sync`; o `Mutex` deixa o `Store` ser compartilhado entre threads.
    canal: Mutex<Option<Sender<Mensagem>>>,
    tratador: Option<JoinHandle<()>>,
}

impl Escritor {
    /// Abre o banco e sobe a thread de escrita.
    pub fn iniciar(caminho: PathBuf) -> Resultado<Self> {
        let mut conn = abrir_escrita(&caminho)?;
        let (canal, receptor) = mpsc::channel();
        let tratador = thread::Builder::new()
            .name("poph-escritor".to_owned())
            .spawn(move || laco(&mut conn, &receptor))
            .map_err(|erro| {
                ErroStore::Interno(format!("não consegui criar a thread de escrita: {erro}"))
            })?;
        let escritor = Self {
            canal: Mutex::new(Some(canal)),
            tratador: Some(tratador),
        };
        // Prova na subida que a thread está viva antes de qualquer escrita de verdade.
        escritor.executar(|conn| {
            conn.query_row("SELECT 1", [], |l| l.get::<_, i64>(0))
                .map_err(ErroStore::from)
        })?;
        Ok(escritor)
    }

    /// Envia a tarefa para a thread e espera o resultado (depois do commit).
    pub fn executar<T: Send + 'static>(
        &self,
        tarefa: impl FnOnce(&Connection) -> Resultado<T> + Send + 'static,
    ) -> Resultado<T> {
        let (resposta, recebedor) = mpsc::channel();
        let envio = {
            let canal = self.canal.lock().unwrap_or_else(|e| e.into_inner());
            canal
                .as_ref()
                .ok_or(ErroStore::EscritorEncerrado)?
                .send(Mensagem::Comando(Comando {
                    tarefa: Box::new(move |conn| {
                        tarefa(conn).map(|valor| Box::new(valor) as Caixa)
                    }),
                    resposta,
                }))
        };
        envio.map_err(|_| ErroStore::EscritorEncerrado)?;
        let caixa = recebedor
            .recv()
            .map_err(|_| ErroStore::EscritorEncerrado)??;
        caixa
            .downcast::<T>()
            .map(|valor| *valor)
            .map_err(|_| ErroStore::Interno("o escritor devolveu um tipo inesperado".to_owned()))
    }
}

impl Drop for Escritor {
    fn drop(&mut self) {
        let canal = self
            .canal
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
            .take();
        if let Some(canal) = canal {
            let (aviso, espera) = mpsc::channel();
            if canal.send(Mensagem::Encerrar(aviso)).is_ok() {
                // Se o lote atual demorar, não trava o encerramento do processo.
                let _ = espera.recv_timeout(Duration::from_secs(5));
            }
        }
        if let Some(tratador) = self.tratador.take()
            && let Err(erro) = tratador.join()
        {
            debug!("thread de escrita terminou de forma inesperada: {erro:?}");
        }
    }
}

fn laco(conn: &mut Connection, receptor: &Receiver<Mensagem>) {
    loop {
        let primeiro = match receptor.recv_timeout(ESPERA) {
            Ok(mensagem) => mensagem,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut lote = vec![primeiro];
        while lote.len() < LOTE_MAX {
            match receptor.try_recv() {
                Ok(mensagem) => lote.push(mensagem),
                Err(_) => break,
            }
        }

        let mut comandos = Vec::with_capacity(lote.len());
        let mut encerrar = None;
        for mensagem in lote {
            match mensagem {
                Mensagem::Comando(comando) => comandos.push(comando),
                Mensagem::Encerrar(aviso) => encerrar = Some(aviso),
            }
        }

        if !comandos.is_empty() {
            executar_lote(conn, comandos);
        }
        if let Some(aviso) = encerrar {
            let _ = aviso.send(());
            return;
        }
    }
}

fn executar_lote(conn: &mut Connection, comandos: Vec<Comando>) {
    let transacao = match conn.transaction_with_behavior(TransactionBehavior::Immediate) {
        Ok(transacao) => transacao,
        Err(erro) => {
            let motivo = erro.to_string();
            for comando in comandos {
                let _ = comando.resposta.send(Err(ErroStore::Interno(format!(
                    "não consegui abrir a transação: {motivo}"
                ))));
            }
            return;
        }
    };

    let mut respostas = Vec::with_capacity(comandos.len());
    for comando in comandos {
        let resultado = match transacao.execute_batch("SAVEPOINT poph_comando") {
            Ok(()) => {
                let resultado = (comando.tarefa)(&transacao);
                let desfecho = if resultado.is_ok() {
                    transacao.execute_batch("RELEASE poph_comando")
                } else {
                    transacao.execute_batch("ROLLBACK TO poph_comando; RELEASE poph_comando")
                };
                match desfecho {
                    Ok(()) => resultado,
                    Err(erro) => Err(ErroStore::Interno(format!(
                        "não consegui fechar o savepoint: {erro}"
                    ))),
                }
            }
            Err(erro) => Err(ErroStore::Sqlite(erro)),
        };
        respostas.push((comando.resposta, resultado));
    }

    match transacao.commit() {
        Ok(()) => {
            for (resposta, resultado) in respostas {
                let _ = resposta.send(resultado);
            }
        }
        Err(erro) => {
            let motivo = erro.to_string();
            debug!(motivo, "commit do lote falhou");
            for (resposta, _) in respostas {
                let _ = resposta.send(Err(ErroStore::Interno(format!(
                    "commit do lote falhou; nenhuma escrita desta chamada foi gravada: {motivo}"
                ))));
            }
        }
    }
}
