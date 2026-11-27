use std::path::{Path, PathBuf};

use poph_matrix::ConversationToRoom;
use poph_matrix::asset::ATTACHMENT_BLOCK;
use poph_matrix::expiration::{CATEGORY_FIELD, EXPIRED_BLOCK, NAME_FIELD, SENT_AT_FIELD};
use poph_matrix::rooms::{FRONTEND_INTEGRATIONS, METADATA_KIND, ROOM_KIND, RoomMetadata};
use poph_types::{AgentId, ConversationId, DepartmentId, OrgId};

const CONTRACT: &str = "../../../frontend/src/sofacoding/atendimentoRoom.ts";

const ATTACHMENT_CONTRACT: &str = "../../../frontend/src/sofacoding/expiredAttachment.ts";

const EXPIRED_CONTRACT: &str = "../../../frontend/src/sofacoding/expiredAttachmentMarker.ts";

const INTERFACE_METADATA: &str = "AtendimentoMetadata";

fn attachment_contract_path() -> PathBuf {
    match std::env::var_os("POPH_ELEMENT_ANEXO_CONTRATO") {
        Some(path) => PathBuf::from(path),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join(ATTACHMENT_CONTRACT),
    }
}

fn contract_path() -> PathBuf {
    match std::env::var_os("POPH_ELEMENT_CONTRATO") {
        Some(path) => PathBuf::from(path),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join(CONTRACT),
    }
}

fn read_contract() -> String {
    let path = contract_path();
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "contrato do Element não encontrado em `{}` ({error}). Este teste confere o módulo com \
             `atendimentoRoom.ts` do frontend do Element. Aponte o arquivo em POPH_ELEMENT_CONTRATO.",
            path.display()
        )
    })
}

fn constant_value(source: &str, name: &str) -> Option<String> {
    let line_beginning = format!("export const {name} =");
    source.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(line_beginning.as_str())?;
        let rest = &rest[rest.find('"')? + 1..];
        let end = rest.find('"')?;
        Some(rest[..end].to_owned())
    })
}

fn constante_do_element(source: &str, name: &str) -> String {
    constant_value(source, name).unwrap_or_else(|| {
        panic!(
            "não achei `export const {name} = \"…\";` em atendimentoRoom.ts: o Element renomeou essa \
             constante ou mudou a forma de declará-la. Se foi de propósito, ajuste o nome lido em \
             tests/contrato_frontend.rs e o uso no módulo; senão, restaure o nome no frontend."
        )
    })
}

fn interface_block<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("export interface {name} {{");
    let beginning = source.find(&marker).unwrap_or_else(|| {
        panic!(
            "a interface `{name}` sumiu de atendimentoRoom.ts: o módulo não tem mais o que conferir. \
             Se ela foi renomeada, ajuste INTERFACE_METADATA em tests/contrato_frontend.rs."
        )
    });
    let rest = &source[beginning..];
    let end = rest
        .find("\n}")
        .unwrap_or_else(|| panic!("a interface `{name}` não fecha em atendimentoRoom.ts"));
    &rest[..end]
}

fn interface_fields(block: &str) -> Vec<String> {
    block
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with("//") || line.starts_with('*') || !line.contains(':') {
                return None;
            }
            let field = line.split_once(':')?.0.trim();
            let field = field.strip_suffix('?').unwrap_or(field);
            if field.is_empty() || field.contains(' ') {
                return None;
            }
            Some(field.to_owned())
        })
        .collect()
}

fn union_values(block: &str, field: &str) -> Vec<String> {
    for line in block.lines() {
        let Some(after) = line.trim().strip_prefix(field) else {
            continue;
        };
        let Some(after) = after.strip_prefix('?').unwrap_or(after).strip_prefix(':') else {
            continue;
        };
        let values: Vec<String> = after
            .split('|')
            .filter_map(|part| {
                let part = part.trim().trim_end_matches(';').trim();
                part.strip_prefix('"')?.strip_suffix('"').map(str::to_owned)
            })
            .collect();
        if !values.is_empty() {
            return values;
        }
    }
    Vec::new()
}

fn complete_conversation() -> ConversationToRoom {
    ConversationToRoom {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        client: "5511999999999".to_owned(),
        client_name: Some("Maria".to_owned()),
        agent: AgentId::generate(),
        agent_name: Some("Agente".to_owned()),
        channel: "whatsapp".to_owned(),
        department: Some(DepartmentId::generate()),
        closed: false,
        backfill_cutoff: None,
    }
}

fn metadata_of(closed: bool) -> serde_json::Value {
    let mut givens = complete_conversation();
    givens.closed = closed;
    serde_json::to_value(RoomMetadata::of_conversation(&givens)).expect("metadata serializa")
}

#[test]
#[ignore = "precisa do arquivo atendimentoRoom.ts do Element: defina POPH_ELEMENT_CONTRATO e rode com --ignored"]
fn room_and_metadata_kind_are_the_element_ones() {
    let source = read_contract();

    let element_kind = constante_do_element(&source, "ATENDIMENTO_ROOM_TYPE");
    assert_eq!(
        ROOM_KIND, element_kind,
        "tipo de sala diferente: o módulo cria a sala com `{ROOM_KIND}` (TIPO_SALA em \
         crates/poph-matrix/src/salas.rs) e o Element procura `{element_kind}` \
         (ATENDIMENTO_ROOM_TYPE em atendimentoRoom.ts). Igualar um dos dois lados."
    );

    let element_event = constante_do_element(&source, "ATENDIMENTO_METADATA_EVENT_TYPE");
    assert_eq!(
        METADATA_KIND, element_event,
        "evento de metadata diferente: o módulo grava `{METADATA_KIND}` (TIPO_METADATA em \
         crates/poph-matrix/src/salas.rs) e o Element lê `{element_event}` \
         (ATENDIMENTO_METADATA_EVENT_TYPE em atendimentoRoom.ts). Igualar um dos dois lados."
    );
}

#[test]
#[ignore = "precisa do arquivo atendimentoRoom.ts do Element: defina POPH_ELEMENT_CONTRATO e rode com --ignored"]
fn every_metadata_field_exists_in_element_interface() {
    let source = read_contract();
    let fields = interface_fields(interface_block(&source, INTERFACE_METADATA));
    assert!(
        fields.len() >= 8,
        "a interface `{INTERFACE_METADATA}` de atendimentoRoom.ts mudou de forma: só achei {fields:?}"
    );

    let metadata = metadata_of(false);
    let object = metadata.as_object().expect("metadata é objeto");
    for field in object.keys() {
        assert!(
            fields.contains(field),
            "o módulo grava `{field}` no metadata da sala (MetadataSala em \
             crates/poph-matrix/src/salas.rs), mas `{INTERFACE_METADATA}` em atendimentoRoom.ts não \
             declara esse campo. Renomear o campo de um dos lados ou incluí-lo na interface do \
             frontend. Campos que o Element declara: {fields:?}"
        );
    }

    for required in [
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
            metadata.get(required).is_some(),
            "o módulo deixou de gravar `{required}` no metadata da sala (MetadataSala em \
             crates/poph-matrix/src/salas.rs), e o Element espera esse campo"
        );
        assert!(
            fields.iter().any(|field| field == required),
            "o Element deixou de declarar `{required}` em `{INTERFACE_METADATA}` \
             (atendimentoRoom.ts), mas o módulo ainda o grava"
        );
    }
}

#[test]
#[ignore = "precisa do arquivo atendimentoRoom.ts do Element: defina POPH_ELEMENT_CONTRATO e rode com --ignored"]
fn integrations_and_module_status_are_in_element_unions() {
    let source = read_contract();
    let block = interface_block(&source, INTERFACE_METADATA);

    let integrations = union_values(block, "integration_type");
    assert!(
        !integrations.is_empty(),
        "não achei a união `integration_type` em `{INTERFACE_METADATA}` (atendimentoRoom.ts)"
    );
    assert_eq!(
        integrations, FRONTEND_INTEGRATIONS,
        "os tipos de integração divergiram: o módulo aceita {FRONTEND_INTEGRATIONS:?} \
         (INTEGRACOES_DO_FRONTEND em salas.rs) e o Element declara {integrations:?}. Igualar um dos dois lados."
    );

    let mut status_do_element = union_values(block, "status");
    status_do_element.sort();
    assert!(
        !status_do_element.is_empty(),
        "não achei a união `status` em `{INTERFACE_METADATA}` (atendimentoRoom.ts)"
    );
    let mut module_status: Vec<String> = [false, true]
        .into_iter()
        .map(|closed| {
            metadata_of(closed)["status"]
                .as_str()
                .expect("status do metadata é texto")
                .to_owned()
        })
        .collect();
    module_status.sort();
    module_status.dedup();
    assert_eq!(
        module_status, status_do_element,
        "os estados de conversa divergiram: o módulo grava {module_status:?} (MetadataSala em \
         salas.rs) e o Element declara {status_do_element:?} (`status` em `{INTERFACE_METADATA}`, \
         atendimentoRoom.ts). Igualar um dos dois lados."
    );
}

#[test]
fn attachment_block_key_is_same_as_frontend() {
    let path = attachment_contract_path();
    let Ok(source) = std::fs::read_to_string(&path) else {
        println!(
            "contrato do anexo NÃO conferido: `{}` não existe (defina POPH_ELEMENT_ANEXO_CONTRATO \
             com o caminho de sofacoding/expiredAttachment.ts)",
            path.display()
        );
        return;
    };
    let key = constant_value(&source, "ATTACHMENT_BLOCK_KEY").unwrap_or_else(|| {
        panic!(
            "não achei `export const ATTACHMENT_BLOCK_KEY = \"…\";` em `{}`: o Element renomeou \
             a constante ou mudou a forma de declará-la. Se foi de propósito, ajuste o nome lido em \
             tests/contrato_frontend.rs e o uso no módulo; senão, restaure o nome no frontend.",
            path.display()
        )
    });
    assert_eq!(
        ATTACHMENT_BLOCK, key,
        "chave do bloco de anexo diferente: o módulo publica `{ATTACHMENT_BLOCK}` (BLOCO_ANEXO em \
         crates/poph-matrix/src/midia.rs) e o Element lê `{key}` (ATTACHMENT_BLOCK_KEY em \
         sofacoding/expiredAttachment.ts). Igualar um dos dois lados."
    );
}

fn expiration_contract_path() -> PathBuf {
    match std::env::var_os("POPH_ELEMENT_EXPIRADO_CONTRATO") {
        Some(path) => PathBuf::from(path),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join(EXPIRED_CONTRACT),
    }
}

#[test]
fn expiration_key_and_fields_are_the_frontend_ones() {
    let path = expiration_contract_path();
    let Ok(source) = std::fs::read_to_string(&path) else {
        println!(
            "contrato da expiração NÃO conferido: `{}` não existe (defina \
             POPH_ELEMENT_EXPIRADO_CONTRATO com o caminho de sofacoding/expiredAttachmentMarker.ts)",
            path.display()
        );
        return;
    };

    let key = constant_value(&source, "ATTACHMENT_EXPIRED_KEY").unwrap_or_else(|| {
        panic!(
            "não achei `export const ATTACHMENT_EXPIRED_KEY = \"…\";` em `{}`: o Element renomeou \
             a constante ou mudou a forma de declará-la. Se foi de propósito, ajuste o nome lido em \
             tests/contrato_frontend.rs e o uso no módulo; senão, restaure o nome no frontend.",
            path.display()
        )
    });
    assert_eq!(
        EXPIRED_BLOCK, key,
        "chave do bloco da expiração diferente: o módulo publica `{EXPIRED_BLOCK}` (BLOCO_EXPIRADO \
         em crates/poph-matrix/src/expiracao.rs) e o Element lê `{key}` (ATTACHMENT_EXPIRED_KEY em \
         sofacoding/expiredAttachmentMarker.ts). Igualar um dos dois lados."
    );

    for field in [CATEGORY_FIELD, NAME_FIELD, SENT_AT_FIELD] {
        let read = format!("payload.{field}");
        assert!(
            source.contains(&read),
            "o módulo grava o campo `{field}` no bloco da expiração \
             (crates/poph-matrix/src/expiracao.rs), mas `{}` não lê `{read}`: o aviso de \
             \"não está mais disponível\" sairia sem esse dado. Igualar o nome do campo dos dois lados.",
            path.display()
        );
    }
}
