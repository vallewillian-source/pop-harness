use std::fs;
use std::path::PathBuf;

use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Origin, Role, Store};
use poph_types::{HarnessConfig, OrgId};

fn variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn numero(name: &str, default: u64) -> u64 {
    match variable(name) {
        Some(value) => value
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{name} precisa ser um número inteiro")),
        None => default,
    }
}

fn create_org(store: &Store) -> OrgId {
    store
        .orgs()
        .create(NewOrg {
            name: "Loja da medição".to_owned(),
            slug: "loja".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

#[test]
#[ignore = "semeadura da medição MX-30; roda por scripts/measure-memory.sh"]
fn seeds_conversations_for_memory_measurement() {
    let dir = PathBuf::from(variable("MX30_PASTA").expect("MX30_PASTA é obrigatória"));
    let rooms = numero("MX30_SALAS", 100);
    let messages_per_room = numero("MX30_MENSAGENS_POR_SALA", 1);
    let enabled = numero("MX30_LIGADO", 1) == 1;

    let dados = dir.join("dados");
    fs::create_dir_all(&dados).expect("pasta de dados");

    let mut config = HarnessConfig::default();
    config.data.directory = dados;
    let store = Store::open(&config).expect("abre o banco da medição");

    let org = create_org(&store);
    if enabled {
        store.matrix().enable(org).expect("liga a org no módulo");
    }
    let agent = store
        .agents()
        .create(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id;

    for index in 0..rooms {
        let conversation = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agent,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(format!("5511{:09}", index)),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        for n in 0..messages_per_room {
            store
                .messages()
                .write(
                    org,
                    NewMessage {
                        conversation_id: conversation,
                        direction: Direction::Inbound,
                        role: Role::User,
                        origin: Origin::Agente,
                        author_name: None,
                        texto: format!("mensagem {n} do cliente {index}"),
                        attachments_json: "[]".to_owned(),
                        external_id: None,
                        turn_id: None,
                    },
                )
                .expect("grava a mensagem do cliente");
        }
    }
    eprintln!(
        "semeadas {rooms} conversas com {messages_per_room} mensagem(ns) cada (ligado={enabled})"
    );
}
