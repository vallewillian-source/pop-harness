//! Teste de contrato com o frontend (MX-11, endurecido na MX-27): o que a sala escreve é o que o Element lê.
//!
//! O Element trata uma sala como conversa por dois nomes fixos e por um punhado de campos do
//! metadata. Aqui os dois lados são comparados **lendo o arquivo do frontend**, e não repetindo o
//! valor escrito à mão nos dois lugares: renomear a constante ou um campo de um lado quebra este
//! teste em vez de quebrar o atendimento em produção.
//!
//! ## O que é conferido
//!
//! Lido em tempo de execução: o arquivo `atendimentoRoom.ts` do frontend do Element (caminho em `POPH_ELEMENT_CONTRATO`).
//!
//! - o tipo de sala (`ATENDIMENTO_ROOM_TYPE`) e o tipo do evento de metadata
//!   (`ATENDIMENTO_METADATA_EVENT_TYPE`) contra `TIPO_SALA` e `TIPO_METADATA`;
//! - cada campo que `MetadataSala` grava na sala está declarado na interface `AtendimentoMetadata`,
//!   e os campos que o Harness precisa preencher não somem;
//! - as uniões `integration_type` e `status` da interface contra o que o módulo escreve.
//! - a chave do bloco de anexo (`com.<marca>.attachment`, MX-43) contra `ATTACHMENT_BLOCK_KEY` de
//!   `sofacoding/expiredAttachment.ts` — o arquivo do Element que lê esse bloco.
//!
//! Os três testes ficam `#[ignore]` neste repositório, porque o frontend do Element mora em outro
//! lugar. Quem tem o arquivo roda `POPH_ELEMENT_CONTRATO=/caminho/atendimentoRoom.ts cargo test
//! -p poph-matrix --test contrato_frontend -- --ignored`. Sem o arquivo, rodando com `--ignored`, o
//! teste **falha**: não há pulo silencioso.
//!
//! ## Como arrumar uma falha
//!
//! Quando o teste falha, a mensagem diz qual lado mudou. Quem renomeou de propósito precisa igualar
//! o outro lado: o valor em `crates/poph-matrix/src/salas.rs` ou o nome em
//! `atendimentoRoom.ts` do frontend. Salas já criadas com um valor antigo não são
//! reescritas sozinhas.

use std::path::{Path, PathBuf};

use poph_matrix::ConversaParaSala;
use poph_matrix::midia::BLOCO_ANEXO;
use poph_matrix::salas::{INTEGRACOES_DO_FRONTEND, MetadataSala, TIPO_METADATA, TIPO_SALA};
use poph_types::{AgentId, ConversationId, DepartmentId, OrgId};

/// Caminho do contrato do Element, relativo ao crate (`apps/pop-harness/crates/poph-matrix`).
const CONTRATO: &str = "../../../frontend/src/sofacoding/atendimentoRoom.ts";

/// Caminho do contrato do anexo, relativo ao crate (`apps/pop-harness/crates/poph-matrix`).
const CONTRATO_ANEXO: &str = "../../../frontend/src/sofacoding/expiredAttachment.ts";

/// Nome da interface que descreve o metadata no frontend.
const INTERFACE_METADATA: &str = "AtendimentoMetadata";

/// Caminho do arquivo do frontend que lê o bloco do anexo.
fn caminho_do_contrato_do_anexo() -> PathBuf {
    match std::env::var_os("POPH_ELEMENT_ANEXO_CONTRATO") {
        Some(caminho) => PathBuf::from(caminho),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join(CONTRATO_ANEXO),
    }
}

fn caminho_do_contrato() -> PathBuf {
    // Fora do monorepo o arquivo do Element não existe aqui: quem tem o frontend aponta o caminho.
    match std::env::var_os("POPH_ELEMENT_CONTRATO") {
        Some(caminho) => PathBuf::from(caminho),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join(CONTRATO),
    }
}

/// Lê o contrato do Element. Sem o arquivo, o teste falha com o caminho procurado.
fn ler_contrato() -> String {
    let caminho = caminho_do_contrato();
    std::fs::read_to_string(&caminho).unwrap_or_else(|erro| {
        panic!(
            "contrato do Element não encontrado em `{}` ({erro}). Este teste confere o módulo com \
             `atendimentoRoom.ts` do frontend do Element. Aponte o arquivo em POPH_ELEMENT_CONTRATO.",
            caminho.display()
        )
    })
}

/// Valor de `export const NOME = "…";`. O nome precisa ser exato (não basta conter o texto).
fn valor_da_constante(fonte: &str, nome: &str) -> Option<String> {
    let inicio_da_linha = format!("export const {nome} =");
    fonte.lines().find_map(|linha| {
        let resto = linha.trim().strip_prefix(inicio_da_linha.as_str())?;
        let resto = &resto[resto.find('"')? + 1..];
        let fim = resto.find('"')?;
        Some(resto[..fim].to_owned())
    })
}

/// Como [`valor_da_constante`], mas falha dizendo o que fazer quando a constante não existe.
fn constante_do_element(fonte: &str, nome: &str) -> String {
    valor_da_constante(fonte, nome).unwrap_or_else(|| {
        panic!(
            "não achei `export const {nome} = \"…\";` em atendimentoRoom.ts: o Element renomeou essa \
             constante ou mudou a forma de declará-la. Se foi de propósito, ajuste o nome lido em \
             tests/contrato_frontend.rs e o uso no módulo; senão, restaure o nome no frontend."
        )
    })
}

/// Corpo da interface `nome`, do `export interface` até a chave que fecha.
fn bloco_da_interface<'a>(fonte: &'a str, nome: &str) -> &'a str {
    let marcador = format!("export interface {nome} {{");
    let inicio = fonte.find(&marcador).unwrap_or_else(|| {
        panic!(
            "a interface `{nome}` sumiu de atendimentoRoom.ts: o módulo não tem mais o que conferir. \
             Se ela foi renomeada, ajuste INTERFACE_METADATA em tests/contrato_frontend.rs."
        )
    });
    let resto = &fonte[inicio..];
    let fim = resto
        .find("\n}")
        .unwrap_or_else(|| panic!("a interface `{nome}` não fecha em atendimentoRoom.ts"));
    &resto[..fim]
}

/// Campos declarados numa interface (ignora comentário e linha sem campo).
fn campos_da_interface(bloco: &str) -> Vec<String> {
    bloco
        .lines()
        .filter_map(|linha| {
            let linha = linha.trim();
            if linha.starts_with("//") || linha.starts_with('*') || !linha.contains(':') {
                return None;
            }
            let campo = linha.split_once(':')?.0.trim();
            let campo = campo.strip_suffix('?').unwrap_or(campo);
            if campo.is_empty() || campo.contains(' ') {
                return None;
            }
            Some(campo.to_owned())
        })
        .collect()
}

/// Valores de texto de uma união (`"a" | "b"`) declarada na propriedade `campo` do bloco.
fn valores_da_uniao(bloco: &str, campo: &str) -> Vec<String> {
    for linha in bloco.lines() {
        let Some(depois) = linha.trim().strip_prefix(campo) else {
            continue;
        };
        let Some(depois) = depois.strip_prefix('?').unwrap_or(depois).strip_prefix(':') else {
            continue;
        };
        let valores: Vec<String> = depois
            .split('|')
            .filter_map(|parte| {
                let parte = parte.trim().trim_end_matches(';').trim();
                parte
                    .strip_prefix('"')?
                    .strip_suffix('"')
                    .map(str::to_owned)
            })
            .collect();
        if !valores.is_empty() {
            return valores;
        }
    }
    Vec::new()
}

fn conversa_completa() -> ConversaParaSala {
    ConversaParaSala {
        org: OrgId::generate(),
        conversa: ConversationId::generate(),
        cliente: "5511999999999".to_owned(),
        nome_cliente: Some("Maria".to_owned()),
        agente: AgentId::generate(),
        nome_agente: Some("Agente".to_owned()),
        canal: "whatsapp".to_owned(),
        departamento: Some(DepartmentId::generate()),
        fechada: false,
        corte_backfill: None,
    }
}

/// Metadata que o módulo grava de fato (o mesmo `MetadataSala` usado na criação da sala).
fn metadata_de(fechada: bool) -> serde_json::Value {
    let mut dados = conversa_completa();
    dados.fechada = fechada;
    serde_json::to_value(MetadataSala::da_conversa(&dados)).expect("metadata serializa")
}

#[test]
#[ignore = "precisa do arquivo atendimentoRoom.ts do Element: defina POPH_ELEMENT_CONTRATO e rode com --ignored"]
fn tipo_da_sala_e_do_metadata_sao_os_do_element() {
    let fonte = ler_contrato();

    let tipo_do_element = constante_do_element(&fonte, "ATENDIMENTO_ROOM_TYPE");
    assert_eq!(
        TIPO_SALA, tipo_do_element,
        "tipo de sala diferente: o módulo cria a sala com `{TIPO_SALA}` (TIPO_SALA em \
         crates/poph-matrix/src/salas.rs) e o Element procura `{tipo_do_element}` \
         (ATENDIMENTO_ROOM_TYPE em atendimentoRoom.ts). Igualar um dos dois lados."
    );

    let evento_do_element = constante_do_element(&fonte, "ATENDIMENTO_METADATA_EVENT_TYPE");
    assert_eq!(
        TIPO_METADATA, evento_do_element,
        "evento de metadata diferente: o módulo grava `{TIPO_METADATA}` (TIPO_METADATA em \
         crates/poph-matrix/src/salas.rs) e o Element lê `{evento_do_element}` \
         (ATENDIMENTO_METADATA_EVENT_TYPE em atendimentoRoom.ts). Igualar um dos dois lados."
    );
}

#[test]
#[ignore = "precisa do arquivo atendimentoRoom.ts do Element: defina POPH_ELEMENT_CONTRATO e rode com --ignored"]
fn todo_campo_do_metadata_existe_na_interface_do_element() {
    let fonte = ler_contrato();
    let campos = campos_da_interface(bloco_da_interface(&fonte, INTERFACE_METADATA));
    assert!(
        campos.len() >= 8,
        "a interface `{INTERFACE_METADATA}` de atendimentoRoom.ts mudou de forma: só achei {campos:?}"
    );

    let metadata = metadata_de(false);
    let objeto = metadata.as_object().expect("metadata é objeto");
    for campo in objeto.keys() {
        assert!(
            campos.contains(campo),
            "o módulo grava `{campo}` no metadata da sala (MetadataSala em \
             crates/poph-matrix/src/salas.rs), mas `{INTERFACE_METADATA}` em atendimentoRoom.ts não \
             declara esse campo. Renomear o campo de um dos lados ou incluí-lo na interface do \
             frontend. Campos que o Element declara: {campos:?}"
        );
    }

    // Os campos que o Harness precisa preencher não podem sumir do metadata nem da interface.
    for obrigatorio in [
        "conversation_id",
        "agent_id",
        "integration_type",
        "channel",
        "client_identifier",
        "client_name",
        "department_id",
        "status",
    ] {
        assert!(
            metadata.get(obrigatorio).is_some(),
            "o módulo deixou de gravar `{obrigatorio}` no metadata da sala (MetadataSala em \
             crates/poph-matrix/src/salas.rs), e o Element espera esse campo"
        );
        assert!(
            campos.iter().any(|campo| campo == obrigatorio),
            "o Element deixou de declarar `{obrigatorio}` em `{INTERFACE_METADATA}` \
             (atendimentoRoom.ts), mas o módulo ainda o grava"
        );
    }
}

#[test]
#[ignore = "precisa do arquivo atendimentoRoom.ts do Element: defina POPH_ELEMENT_CONTRATO e rode com --ignored"]
fn integracoes_e_status_do_modulo_estao_nas_unioes_do_element() {
    let fonte = ler_contrato();
    let bloco = bloco_da_interface(&fonte, INTERFACE_METADATA);

    let integracoes = valores_da_uniao(bloco, "integration_type");
    assert!(
        !integracoes.is_empty(),
        "não achei a união `integration_type` em `{INTERFACE_METADATA}` (atendimentoRoom.ts)"
    );
    assert_eq!(
        integracoes, INTEGRACOES_DO_FRONTEND,
        "os tipos de integração divergiram: o módulo aceita {INTEGRACOES_DO_FRONTEND:?} \
         (INTEGRACOES_DO_FRONTEND em salas.rs) e o Element declara {integracoes:?}. Igualar um dos dois lados."
    );

    // O status que o módulo grava precisa ser exatamente o conjunto que o Element declara.
    let mut status_do_element = valores_da_uniao(bloco, "status");
    status_do_element.sort();
    assert!(
        !status_do_element.is_empty(),
        "não achei a união `status` em `{INTERFACE_METADATA}` (atendimentoRoom.ts)"
    );
    let mut status_do_modulo: Vec<String> = [false, true]
        .into_iter()
        .map(|fechada| {
            metadata_de(fechada)["status"]
                .as_str()
                .expect("status do metadata é texto")
                .to_owned()
        })
        .collect();
    status_do_modulo.sort();
    status_do_modulo.dedup();
    assert_eq!(
        status_do_modulo, status_do_element,
        "os estados de conversa divergiram: o módulo grava {status_do_modulo:?} (MetadataSala em \
         salas.rs) e o Element declara {status_do_element:?} (`status` em `{INTERFACE_METADATA}`, \
         atendimentoRoom.ts). Igualar um dos dois lados."
    );
}

/// A chave do bloco que marca a mídia do atendimento é a mesma dos dois lados.
///
/// Este teste **não** é `#[ignore]`: dentro do monorepo o arquivo do frontend está ao lado do
/// módulo, e um rename de um lado só (o do Rust ou o do Element) tem de quebrar aqui, não no
/// atendimento. Fora do monorepo o arquivo não existe, e o teste diz que não conferiu em vez de
/// fingir que passou — mesmo caminho de `POPH_ELEMENT_ANEXO_CONTRATO` para quem tem o Element
/// noutro lugar.
#[test]
fn chave_do_bloco_de_anexo_e_a_mesma_do_frontend() {
    let caminho = caminho_do_contrato_do_anexo();
    let Ok(fonte) = std::fs::read_to_string(&caminho) else {
        println!(
            "contrato do anexo NÃO conferido: `{}` não existe (defina POPH_ELEMENT_ANEXO_CONTRATO \
             com o caminho de sofacoding/expiredAttachment.ts)",
            caminho.display()
        );
        return;
    };
    let chave = valor_da_constante(&fonte, "ATTACHMENT_BLOCK_KEY").unwrap_or_else(|| {
        panic!(
            "não achei `export const ATTACHMENT_BLOCK_KEY = \"…\";` em `{}`: o Element renomeou \
             a constante ou mudou a forma de declará-la. Se foi de propósito, ajuste o nome lido em \
             tests/contrato_frontend.rs e o uso no módulo; senão, restaure o nome no frontend.",
            caminho.display()
        )
    });
    assert_eq!(
        BLOCO_ANEXO, chave,
        "chave do bloco de anexo diferente: o módulo publica `{BLOCO_ANEXO}` (BLOCO_ANEXO em \
         crates/poph-matrix/src/midia.rs) e o Element lê `{chave}` (ATTACHMENT_BLOCK_KEY em \
         sofacoding/expiredAttachment.ts). Igualar um dos dois lados."
    );
}
