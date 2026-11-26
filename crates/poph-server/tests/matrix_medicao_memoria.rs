//! Semeadura da medição de RAM e vazão do módulo Matrix (MX-30).
//!
//! Não é teste de comportamento: é `#[ignore]` e só roda por `scripts/medir-memoria.sh`, que
//! chama este arquivo com as variáveis abaixo e depois sobe o `poph serve` de verdade sobre o
//! banco que ele gravou. A semeadura usa a API do `poph-store` (o mesmo caminho dos testes do
//! módulo), para não depender de SQL escrito à mão.
//!
//! Variáveis (obrigatórias, exceto onde indicado):
//! - `MX30_PASTA`: pasta que recebe `dados/harness.db`; deve existir e ficar vazia;
//! - `MX30_SALAS`: quantas conversas (cada uma vira uma sala do Element quando o módulo está ligado);
//! - `MX30_MENSAGENS_POR_SALA`: mensagens do cliente por conversa (padrão 1);
//! - `MX30_LIGADO`: `1` liga a org no módulo antes das mensagens (o espelho pega tudo que vier
//!   depois do corte); `0` deixa a org desligada, como o cenário "módulo desligado".

use std::fs;
use std::path::PathBuf;

use poph_store::{Direction, NewAgent, NewConversation, NewMessage, NewOrg, Origin, Role, Store};
use poph_types::{HarnessConfig, OrgId};

fn variavel(nome: &str) -> Option<String> {
    std::env::var(nome)
        .ok()
        .filter(|valor| !valor.trim().is_empty())
}

fn numero(nome: &str, padrao: u64) -> u64 {
    match variavel(nome) {
        Some(valor) => valor
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{nome} precisa ser um número inteiro")),
        None => padrao,
    }
}

fn criar_org(store: &Store) -> OrgId {
    store
        .orgs()
        .criar(NewOrg {
            name: "Loja da medição".to_owned(),
            slug: "loja".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

#[test]
#[ignore = "semeadura da medição MX-30; roda por scripts/medir-memoria.sh"]
fn semeia_conversas_para_a_medicao_de_memoria() {
    let pasta = PathBuf::from(variavel("MX30_PASTA").expect("MX30_PASTA é obrigatória"));
    let salas = numero("MX30_SALAS", 100);
    let mensagens_por_sala = numero("MX30_MENSAGENS_POR_SALA", 1);
    let ligado = numero("MX30_LIGADO", 1) == 1;

    let dados = pasta.join("dados");
    fs::create_dir_all(&dados).expect("pasta de dados");

    let mut config = HarnessConfig::default();
    config.data.directory = dados;
    let store = Store::open(&config).expect("abre o banco da medição");

    let org = criar_org(&store);
    if ligado {
        // Antes de qualquer mensagem: o corte do backfill fica para trás e tudo que vier depois é
        // espelhado pelo laço do módulo quando o `poph serve` subir.
        store.matrix().ligar(org).expect("liga a org no módulo");
    }
    let agente = store
        .agents()
        .criar(
            org,
            NewAgent {
                name: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente")
        .id;

    for indice in 0..salas {
        let conversa = store
            .conversas()
            .criar(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(format!("5511{:09}", indice)),
                    target_department: None,
                },
            )
            .expect("cria a conversa")
            .id;
        for n in 0..mensagens_por_sala {
            store
                .messages()
                .gravar(
                    org,
                    NewMessage {
                        conversation_id: conversa,
                        direction: Direction::Inbound,
                        role: Role::User,
                        origin: Origin::Agente,
                        author_name: None,
                        texto: format!("mensagem {n} do cliente {indice}"),
                        attachments_json: "[]".to_owned(),
                        id_externo: None,
                        turn_id: None,
                    },
                )
                .expect("grava a mensagem do cliente");
        }
    }
    eprintln!(
        "semeadas {salas} conversas com {mensagens_por_sala} mensagem(ns) cada (ligado={ligado})"
    );
}
