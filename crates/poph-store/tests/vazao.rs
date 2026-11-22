//! Medição de vazão e latência de escrita do armazenamento (comparação com as metas da HR-01).
//!
//! Não roda no CI: são minutos de carga. Rodar assim:
//!
//! ```text
//! cargo test --release -p poph-store --test vazao -- --ignored --nocapture
//! ```
//!
//! Metas da HR-01 que este teste confere:
//! - escrita ≥ 50 mil mensagens/s com lote;
//! - p95 da escrita ≤ 5 ms com chegada constante de 2.000 mensagens/s.

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use poph_store::{Direcao, NovaConversa, NovaMensagem, NovaOrg, NovoAgente, Org, Papel, Store};

struct Carga {
    pasta: std::path::PathBuf,
    store: Arc<Store>,
    org: Org,
    conversa: poph_types::ConversationId,
}

impl Carga {
    fn preparar(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-vazao-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria pasta");
        let store =
            Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre banco");
        let store = Arc::new(store);
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: "Org de vazão".to_owned(),
                slug: "vazao".to_owned(),
                config_json: String::new(),
            })
            .expect("org");
        let novo = NovoAgente {
            nome: "Agente de vazão".to_owned(),
            slug: "vazao".to_owned(),
            ..NovoAgente::default()
        };
        let agente = store.agentes().criar(org.id, novo).expect("agente");
        let conversa = store
            .conversas()
            .criar(
                org.id,
                NovaConversa {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("vazao".to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("conversa")
            .id;
        Self {
            pasta,
            store,
            org,
            conversa,
        }
    }

    fn escrever(&self) {
        self.store
            .mensagens()
            .gravar(
                self.org.id,
                NovaMensagem {
                    conversation_id: self.conversa,
                    direcao: Direcao::Entrada,
                    papel: Papel::Usuario,
                    texto: "mensagem de carga".to_owned(),
                    anexos_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                    origem: poph_store::Origem::Agente,
                    autor_nome: None,
                },
            )
            .expect("gravação");
    }
}

impl Drop for Carga {
    fn drop(&mut self) {
        // Linux permite remover arquivo aberto; a pasta some mesmo com a thread de escrita viva.
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn percentil(amostras: &mut [u128], p: f64) -> u128 {
    if amostras.is_empty() {
        return 0;
    }
    amostras.sort_unstable();
    let indice = ((amostras.len() as f64 - 1.0) * p).round() as usize;
    amostras[indice]
}

#[test]
#[ignore = "carga de minutos; rodar com --ignored --nocapture"]
fn vazao_e_latencia_contra_as_metas_da_hr01() {
    // 1) Uma thread, uma transação por escrita: o piso do SQLite com WAL.
    {
        let carga = Carga::preparar("uma-thread");
        let duracao = Duration::from_secs(2);
        let inicio = Instant::now();
        let mut total = 0_u64;
        while inicio.elapsed() < duracao {
            carga.escrever();
            total += 1;
        }
        let segundos = inicio.elapsed().as_secs_f64();
        println!(
            "uma thread, sem lote: {total} escritas em {segundos:.2} s = {:.0} msg/s",
            total as f64 / segundos
        );
    }

    // 2) N threads ao mesmo tempo: o escritor agrupa em lote. 32 é o desenho do motor
    //    (poucos produtores, muitos emissores); 256 é o caso de pico.
    for threads in [32_usize, 256] {
        let carga = Carga::preparar(&format!("{threads}-threads"));
        let parar = Arc::new(AtomicBool::new(false));
        let contador = Arc::new(AtomicU64::new(0));
        let inicio = Instant::now();
        let mut tarefas = Vec::with_capacity(threads);
        for _ in 0..threads {
            let store = Arc::clone(&carga.store);
            let org = carga.org.id;
            let conversa = carga.conversa;
            let parar = Arc::clone(&parar);
            let contador = Arc::clone(&contador);
            tarefas.push(thread::spawn(move || {
                let mut locais = 0_u64;
                while !parar.load(Ordering::Relaxed) {
                    store
                        .mensagens()
                        .gravar(
                            org,
                            NovaMensagem {
                                conversation_id: conversa,
                                direcao: Direcao::Entrada,
                                papel: Papel::Usuario,
                                texto: "lote".to_owned(),
                                anexos_json: "[]".to_owned(),
                                id_externo: None,
                                turn_id: None,
                                origem: poph_store::Origem::Agente,
                                autor_nome: None,
                            },
                        )
                        .expect("gravação concorrente");
                    locais += 1;
                }
                contador.fetch_add(locais, Ordering::Relaxed);
            }));
        }
        thread::sleep(Duration::from_secs(3));
        parar.store(true, Ordering::Relaxed);
        for tarefa in tarefas {
            tarefa.join().expect("thread");
        }
        let segundos = inicio.elapsed().as_secs_f64();
        let total = contador.load(Ordering::Relaxed);
        println!(
            "{threads} concorrentes, com lote: {total} escritas em {segundos:.2} s = {:.0} msg/s",
            total as f64 / segundos
        );
        assert_eq!(
            carga
                .store
                .mensagens()
                .contar(carga.org.id, carga.conversa)
                .expect("conta"),
            total,
            "todas as escritas contabilizadas precisam estar no banco"
        );
    }

    // 3) Chegada constante de 2.000 msg/s por 3 s: latência de cada escrita.
    {
        let carga = Carga::preparar("chegada-constante");
        const TAXA_POR_SEGUNDO: u64 = 2_000;
        let duracao = Duration::from_secs(3);
        let total_a_escrever = TAXA_POR_SEGUNDO * duracao.as_secs();
        let inicio = Instant::now();
        let mut latencias = Vec::with_capacity(total_a_escrever as usize);
        for indice in 0..total_a_escrever {
            let alvo = Duration::from_micros(indice * 1_000_000 / TAXA_POR_SEGUNDO);
            let agora = inicio.elapsed();
            if alvo > agora {
                thread::sleep(alvo - agora);
            }
            let antes = Instant::now();
            carga.escrever();
            latencias.push(antes.elapsed().as_micros());
        }
        let segundos = inicio.elapsed().as_secs_f64();
        let mut ordenadas = latencias.clone();
        let p50 = percentil(&mut ordenadas, 0.50);
        let p95 = percentil(&mut latencias, 0.95);
        println!(
            "chegada de {TAXA_POR_SEGUNDO} msg/s por {:.1} s: {:.0} msg/s efetivas, escrita p50 {p50} µs, p95 {p95} µs",
            segundos,
            total_a_escrever as f64 / segundos
        );
        assert!(
            p95 <= 5_000,
            "p95 de escrita {p95} µs passou da meta de 5 ms (D-07)"
        );
    }
}
