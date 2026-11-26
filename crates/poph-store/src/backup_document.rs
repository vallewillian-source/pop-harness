use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::backup::{BACKUP_FORMAT, OrgSnapshot, VERSAO_FORMATO};

#[derive(Debug)]
pub struct BackupDocument {
    pub snapshot: OrgSnapshot,
    pub blobs: BTreeMap<String, Vec<u8>>,
    pub com_conversas: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    Json(String),
    Formato(String),
    Versao { read: u64, entendida: u32 },
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentError::Json(reason) => write!(f, "JSON inválido: {reason}"),
            DocumentError::Formato(reason) => write!(f, "{reason}"),
            DocumentError::Versao { read, entendida } => write!(
                f,
                "versão de formato desconhecida: {read}; esta instalação entende a {entendida}"
            ),
        }
    }
}

impl std::error::Error for DocumentError {}

pub fn document(
    snapshot: &OrgSnapshot,
    com_conversas: bool,
    exportado_em: i64,
    blobs: &BTreeMap<String, Vec<u8>>,
    secret_names: &dyn Fn(&[u8]) -> Option<Vec<String>>,
) -> Value {
    let mut tables = snapshot.tables.clone();
    mark_secrets(&mut tables, &snapshot.encrypted_secrets, secret_names);
    json!({
        "formato": BACKUP_FORMAT,
        "versao_formato": VERSAO_FORMATO,
        "exportado_em": exportado_em,
        "org_origem": {
            "id": snapshot.org.get("id").cloned().unwrap_or(Value::Null),
            "slug": snapshot.org.get("slug").cloned().unwrap_or(Value::Null),
            "nome": snapshot.org.get("nome").cloned().unwrap_or(Value::Null),
        },
        "com_conversas": com_conversas,
        "org": snapshot.org,
        "tabelas": tables,
        "blobs": blobs
            .iter()
            .map(|(sha, bytes)| json!({"sha256": sha, "conteudo_base64": encode(bytes)}))
            .collect::<Vec<_>>(),
    })
}

pub fn read_document(bytes: &[u8]) -> Result<BackupDocument, DocumentError> {
    let document: Value =
        serde_json::from_slice(bytes).map_err(|error| DocumentError::Json(error.to_string()))?;
    let snapshot = document_snapshot(&document)?;
    let blobs = document_blobs(&document)?;
    let com_conversas = document
        .get("com_conversas")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(BackupDocument {
        snapshot,
        blobs,
        com_conversas,
    })
}

pub fn document_snapshot(document: &Value) -> Result<OrgSnapshot, DocumentError> {
    let formato = document
        .get("formato")
        .and_then(Value::as_str)
        .unwrap_or("");
    if formato != BACKUP_FORMAT {
        return Err(DocumentError::Formato(format!(
            "este arquivo não é um backup do POP Harness (formato `{formato}`)"
        )));
    }
    let version = document
        .get("versao_formato")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if !(1..=u64::from(VERSAO_FORMATO)).contains(&version) {
        return Err(DocumentError::Versao {
            read: version,
            entendida: VERSAO_FORMATO,
        });
    }
    let org = document
        .get("org")
        .cloned()
        .filter(Value::is_object)
        .ok_or_else(|| DocumentError::Formato("o arquivo não traz a linha da org".to_owned()))?;
    let tables = document
        .get("tabelas")
        .and_then(Value::as_object)
        .ok_or_else(|| DocumentError::Formato("o arquivo não traz as tabelas da org".to_owned()))?;
    let mut snapshot = BTreeMap::new();
    for (name, rows) in tables {
        let rows = rows.as_array().ok_or_else(|| {
            DocumentError::Formato(format!("a tabela `{name}` do arquivo não é uma lista"))
        })?;
        snapshot.insert(name.clone(), rows.clone());
    }
    Ok(OrgSnapshot {
        org,
        tables: snapshot,
        encrypted_secrets: BTreeMap::new(),
    })
}

pub fn document_blobs(document: &Value) -> Result<BTreeMap<String, Vec<u8>>, DocumentError> {
    let list = document
        .get("blobs")
        .and_then(Value::as_array)
        .ok_or_else(|| DocumentError::Formato("o arquivo não traz a lista de blobs".to_owned()))?;
    let mut blobs = BTreeMap::new();
    for item in list {
        let sha = item
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| DocumentError::Formato("blob do arquivo sem sha256".to_owned()))?;
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DocumentError::Formato(format!(
                "o sha256 `{sha}` do arquivo não é válido"
            )));
        }
        let texto = item
            .get("conteudo_base64")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DocumentError::Formato(format!("o blob {sha} do arquivo não tem conteúdo"))
            })?;
        let bytes = descodificar(texto).map_err(|error| {
            DocumentError::Formato(format!(
                "o conteúdo do blob {sha} do arquivo não abre: {error}"
            ))
        })?;
        if sha256_hex(&bytes) != sha {
            return Err(DocumentError::Formato(format!(
                "o conteúdo de um arquivo do backup não confere com o sha256 gravado ({sha}); \
                 o arquivo está corrompido ou foi alterado"
            )));
        }
        blobs.insert(sha.to_owned(), bytes);
    }
    Ok(blobs)
}

pub fn mark_secrets(
    tables: &mut BTreeMap<String, Vec<Value>>,
    encrypted: &BTreeMap<String, Vec<u8>>,
    secret_names: &dyn Fn(&[u8]) -> Option<Vec<String>>,
) {
    let Some(rows) = tables.get_mut("integrations") else {
        return;
    };
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let campos = encrypted
            .get(&id)
            .and_then(|blob| secret_names(blob))
            .map(|names| names.into_iter().map(Value::String).collect::<Vec<_>>());
        if let Some(object) = row.as_object_mut() {
            object.insert(
                "segredos_campos".to_owned(),
                campos.map(Value::Array).unwrap_or(Value::Null),
            );
        }
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn encode(bytes: &[u8]) -> String {
    const ALFABETO: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut saida = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for block in bytes.chunks(3) {
        let b0 = u32::from(block[0]);
        let b1 = block.get(1).map_or(0, |b| u32::from(*b));
        let b2 = block.get(2).map_or(0, |b| u32::from(*b));
        let numero = (b0 << 16) | (b1 << 8) | b2;
        saida.push(char::from(ALFABETO[((numero >> 18) & 63) as usize]));
        saida.push(char::from(ALFABETO[((numero >> 12) & 63) as usize]));
        if block.len() > 1 {
            saida.push(char::from(ALFABETO[((numero >> 6) & 63) as usize]));
        } else {
            saida.push('=');
        }
        if block.len() > 2 {
            saida.push(char::from(ALFABETO[(numero & 63) as usize]));
        } else {
            saida.push('=');
        }
    }
    saida
}

pub fn descodificar(texto: &str) -> Result<Vec<u8>, String> {
    let clean = texto.trim().as_bytes();
    if !clean.len().is_multiple_of(4) {
        return Err("tamanho de base64 inválido".to_owned());
    }
    let blocks = clean.len() / 4;
    let mut saida = Vec::with_capacity(blocks * 3);
    for (index, block) in clean.chunks(4).enumerate() {
        let last = index + 1 == blocks;
        let mut values = [0u8; 4];
        let mut preenchimento = 0usize;
        for (position, byte) in block.iter().enumerate() {
            if *byte == b'=' {
                if !last || position < 2 {
                    return Err("preenchimento de base64 fora do fim".to_owned());
                }
                preenchimento += 1;
                continue;
            }
            if preenchimento > 0 {
                return Err("base64 com caractere depois do preenchimento".to_owned());
            }
            values[position] = base64_value(*byte)?;
        }
        let numero = (u32::from(values[0]) << 18)
            | (u32::from(values[1]) << 12)
            | (u32::from(values[2]) << 6)
            | u32::from(values[3]);
        saida.push((numero >> 16) as u8);
        if preenchimento < 2 {
            saida.push((numero >> 8) as u8);
        }
        if preenchimento < 1 {
            saida.push(numero as u8);
        }
    }
    Ok(saida)
}

fn base64_value(byte: u8) -> Result<u8, String> {
    Ok(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return Err("caractere inválido em base64".to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_snapshot() -> OrgSnapshot {
        let mut tables: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        tables.insert(
            "agents".to_owned(),
            vec![json!({"id": "0192f4b0-0000-7000-8000-000000000001", "slug": "bia"})],
        );
        tables.insert(
            "integrations".to_owned(),
            vec![json!({"id": "integ-1", "tem_segredo": true})],
        );
        OrgSnapshot {
            org: json!({
                "id": "0192f4b0-0000-7000-8000-0000000000aa",
                "slug": "org-teste",
                "nome": "Org de teste",
            }),
            tables,
            encrypted_secrets: BTreeMap::from([("integ-1".to_owned(), b"cifrado".to_vec())]),
        }
    }

    #[test]
    fn document_carries_format_version_org_and_secret_names() {
        let snapshot = test_snapshot();
        let blobs = BTreeMap::from([("a".repeat(64), b"conteudo".to_vec())]);
        let names = |_blob: &[u8]| Some(vec!["token".to_owned()]);
        let doc = document(&snapshot, true, 1_700_000_000_000, &blobs, &names);
        assert_eq!(doc["formato"], BACKUP_FORMAT);
        assert_eq!(doc["versao_formato"], VERSAO_FORMATO);
        assert_eq!(doc["org_origem"]["slug"], "org-teste");
        assert_eq!(doc["com_conversas"], true);
        assert_eq!(
            doc["tabelas"]["integrations"][0]["segredos_campos"],
            json!(["token"])
        );
        assert!(!doc.to_string().contains("cifrado"));
    }

    #[test]
    fn read_document_rejects_json_version_and_tampered_content() {
        let error = read_document(b"nao e json").expect_err("json invalido");
        assert!(matches!(error, DocumentError::Json(_)), "{error}");

        let doc = json!({"formato": "outra-coisa", "versao_formato": 1});
        let error = read_document(doc.to_string().as_bytes()).expect_err("formato");
        assert!(matches!(error, DocumentError::Formato(_)), "{error}");

        let doc = json!({"formato": BACKUP_FORMAT, "versao_formato": 99, "org": {}});
        let error = read_document(doc.to_string().as_bytes()).expect_err("versao");
        assert!(error.to_string().contains("versão de formato"), "{error}");

        let doc = json!({
            "formato": BACKUP_FORMAT,
            "versao_formato": VERSAO_FORMATO,
            "org": {"id": "0192f4b0-0000-7000-8000-0000000000aa"},
            "tabelas": {},
            "blobs": [{"sha256": "0".repeat(64), "conteudo_base64": encode(b"conteudo")}],
        });
        let error = read_document(doc.to_string().as_bytes()).expect_err("blob adulterado");
        assert!(error.to_string().contains("não confere"), "{error}");
    }

    #[test]
    fn document_round_trip() {
        let snapshot = test_snapshot();
        let blobs = BTreeMap::from([
            (
                "a".repeat(64),
                "conteudo com acentos: ação".as_bytes().to_vec(),
            ),
            ("b".repeat(64), Vec::new()),
        ]);
        let names = |_blob: &[u8]| None;
        let real_blobs = blobs
            .values()
            .map(|bytes| (sha256_hex(bytes), bytes.clone()))
            .collect::<BTreeMap<_, _>>();
        let doc = document(&snapshot, false, 1_700_000_000_000, &real_blobs, &names);
        let read = read_document(doc.to_string().as_bytes()).expect("ler o que acabou de montar");
        assert_eq!(read.blobs, real_blobs);
        assert!(!read.com_conversas);
        assert_eq!(read.snapshot.tables["agents"], snapshot.tables["agents"]);
        let integration = &read.snapshot.tables["integrations"][0];
        assert_eq!(integration["tem_segredo"], true);
        assert!(integration["segredos_campos"].is_null());
        assert!(read.snapshot.encrypted_secrets.is_empty());
    }

    #[test]
    fn base64_round_trip() {
        let cases: &[&[u8]] = &[
            b"",
            b"f",
            b"fo",
            b"foo",
            b"conteudo maior, com acentos: acao",
        ];
        for case in cases {
            let texto = encode(case);
            let returned = descodificar(&texto).expect("decodifica");
            assert_eq!(&returned, case, "base64 de {:?}", case);
        }
    }

    #[test]
    fn base64_rejects_invalid_input() {
        for texto in ["Zm9vYg", "Zm9v-YI==", "Zg==Zg==", "Zm9=v"] {
            assert!(descodificar(texto).is_err(), "{texto} devia ser recusado");
        }
    }
}
