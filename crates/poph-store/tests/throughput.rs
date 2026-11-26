use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Org, Role, Store};

struct Load {
    folder: std::path::PathBuf,
    store: Arc<Store>,
    org: Org,
    conversation: poph_types::ConversationId,
}

impl Load {
    fn prepare(label: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("poph-vazao-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("cria pasta");
        let store =
            Store::open_at(&folder.join("harness.db"), &folder.join("rag.db")).expect("abre banco");
        let store = Arc::new(store);
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org de vazão".to_owned(),
                slug: "vazao".to_owned(),
                config_json: String::new(),
            })
            .expect("org");
        let new = NewAgent {
            name: "Agente de vazão".to_owned(),
            slug: "vazao".to_owned(),
            ..NewAgent::default()
        };
        let agent = store.agents().create(org.id, new).expect("agente");
        let conversation = store
            .conversations()
            .create(
                org.id,
                NewConversation {
                    agent_id: agent.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: Some("vazao".to_owned()),
                    target_department: None,
                },
            )
            .expect("conversa")
            .id;
        Self {
            folder,
            store,
            org,
            conversation,
        }
    }

    fn write(&self) {
        self.store
            .messages()
            .write(
                self.org.id,
                NewMessage {
                    conversation_id: self.conversation,
                    direction: Direction::Inbound,
                    role: Role::User,
                    texto: "mensagem de carga".to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
            )
            .expect("gravação");
    }
}

impl Drop for Load {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

fn percentile(samples: &mut [u128], p: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let index = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[index]
}

#[test]
#[ignore = "carga de minutos; rodar com --ignored --nocapture"]
fn throughput_and_latency_against_hr01_targets() {
    {
        let load = Load::prepare("uma-thread");
        let duration = Duration::from_secs(2);
        let beginning = Instant::now();
        let mut total = 0_u64;
        while beginning.elapsed() < duration {
            load.write();
            total += 1;
        }
        let seconds = beginning.elapsed().as_secs_f64();
        println!(
            "uma thread, sem lote: {total} escritas em {seconds:.2} s = {:.0} msg/s",
            total as f64 / seconds
        );
    }

    for threads in [32_usize, 256] {
        let load = Load::prepare(&format!("{threads}-threads"));
        let stop = Arc::new(AtomicBool::new(false));
        let counter = Arc::new(AtomicU64::new(0));
        let beginning = Instant::now();
        let mut tasks = Vec::with_capacity(threads);
        for _ in 0..threads {
            let store = Arc::clone(&load.store);
            let org = load.org.id;
            let conversation = load.conversation;
            let stop = Arc::clone(&stop);
            let counter = Arc::clone(&counter);
            tasks.push(thread::spawn(move || {
                let mut local = 0_u64;
                while !stop.load(Ordering::Relaxed) {
                    store
                        .messages()
                        .write(
                            org,
                            NewMessage {
                                conversation_id: conversation,
                                direction: Direction::Inbound,
                                role: Role::User,
                                texto: "lote".to_owned(),
                                attachments_json: "[]".to_owned(),
                                external_id: None,
                                turn_id: None,
                                origin: poph_store::Origin::Agente,
                                author_name: None,
                            },
                        )
                        .expect("gravação concorrente");
                    local += 1;
                }
                counter.fetch_add(local, Ordering::Relaxed);
            }));
        }
        thread::sleep(Duration::from_secs(3));
        stop.store(true, Ordering::Relaxed);
        for task in tasks {
            task.join().expect("thread");
        }
        let seconds = beginning.elapsed().as_secs_f64();
        let total = counter.load(Ordering::Relaxed);
        println!(
            "{threads} concorrentes, com lote: {total} escritas em {seconds:.2} s = {:.0} msg/s",
            total as f64 / seconds
        );
        assert_eq!(
            load.store
                .messages()
                .count_rows(load.org.id, load.conversation)
                .expect("conta"),
            total,
            "todas as escritas contabilizadas precisam estar no banco"
        );
    }

    {
        let load = Load::prepare("chegada-constante");
        const RATE_BY_SECOND: u64 = 2_000;
        let duration = Duration::from_secs(3);
        let total_to_write = RATE_BY_SECOND * duration.as_secs();
        let beginning = Instant::now();
        let mut latencies = Vec::with_capacity(total_to_write as usize);
        for index in 0..total_to_write {
            let target = Duration::from_micros(index * 1_000_000 / RATE_BY_SECOND);
            let now = beginning.elapsed();
            if target > now {
                thread::sleep(target - now);
            }
            let before = Instant::now();
            load.write();
            latencies.push(before.elapsed().as_micros());
        }
        let seconds = beginning.elapsed().as_secs_f64();
        let mut sorted = latencies.clone();
        let p50 = percentile(&mut sorted, 0.50);
        let p95 = percentile(&mut latencies, 0.95);
        println!(
            "chegada de {RATE_BY_SECOND} msg/s por {:.1} s: {:.0} msg/s efetivas, escrita p50 {p50} µs, p95 {p95} µs",
            seconds,
            total_to_write as f64 / seconds
        );
        assert!(
            p95 <= 5_000,
            "p95 de escrita {p95} µs passou da meta de 5 ms (D-07)"
        );
    }
}
