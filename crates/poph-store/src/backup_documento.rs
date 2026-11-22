//! O arquivo de backup de uma org: montagem e validação (HR-53, reaproveitado pela HR-48e-1).
//!
//! O mesmo formato serve à CLI (`poph backup export`/`import`) e à API administrativa
//! (`GET …/backup` e `POST …/backup/import`). Aqui ficam só as duas pontas que não dependem de
//! quem chama: montar o documento JSON a partir de um [`RetratoOrg`] e os blobs, e validar um
//! arquivo de volta para retrato + blobs. A leitura e a escrita das tabelas continuam no
//! `crate::backup`; os bytes dos arquivos continuam no `poph-files` (quem chama lê e grava).
//!
//! Regras do documento:
//! - `formato` e `versao_formato` declarados; versão desconhecida é recusada com a versão lida;
//! - a org de origem vai identificada (`org_origem`) e a linha da org vai inteira em `org`;
//! - os blobs vão em base64, deduplicados por `sha256`, e o sha do conteúdo é conferido na
//!   leitura (arquivo alterado não entra);
//! - segredo nunca viaja: a integração leva só `tem_segredo` e os **nomes** dos campos
//!   preenchidos (`segredos_campos`). Quem decifra é o chamador, pela closure recebida.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::backup::{FORMATO_BACKUP, RetratoOrg, VERSAO_FORMATO};

/// Um arquivo de backup lido e validado.
#[derive(Debug)]
pub struct DocumentoBackup {
    /// A org como linhas de banco, pronta para o `importar_retrato`.
    pub retrato: RetratoOrg,
    /// sha256 → bytes dos arquivos da biblioteca e dos anexos.
    pub blobs: BTreeMap<String, Vec<u8>>,
    /// O arquivo diz que levava conversas (informativo: as tabelas é que mandam).
    pub com_conversas: bool,
}

/// Por que um arquivo não serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErroDocumento {
    /// Não é JSON legível.
    Json(String),
    /// É JSON, mas não é um backup deste formato (ou o conteúdo está adulterado).
    Formato(String),
    /// O formato existe, mas a versão é outra.
    Versao {
        /// Versão gravada no arquivo.
        lida: u64,
        /// Versão que esta instalação entende.
        entendida: u32,
    },
}

impl fmt::Display for ErroDocumento {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErroDocumento::Json(motivo) => write!(f, "JSON inválido: {motivo}"),
            ErroDocumento::Formato(motivo) => write!(f, "{motivo}"),
            ErroDocumento::Versao { lida, entendida } => write!(
                f,
                "versão de formato desconhecida: {lida}; esta instalação entende a {entendida}"
            ),
        }
    }
}

impl std::error::Error for ErroDocumento {}

/// Monta o documento que vai para o arquivo.
///
/// `nomes_do_segredo` recebe o blob cifrado de uma integração e devolve os nomes dos campos
/// preenchidos (ou `None` quando não consegue decifrar). O valor do segredo nunca é pedido.
pub fn documento(
    retrato: &RetratoOrg,
    com_conversas: bool,
    exportado_em: i64,
    blobs: &BTreeMap<String, Vec<u8>>,
    nomes_do_segredo: &dyn Fn(&[u8]) -> Option<Vec<String>>,
) -> Value {
    let mut tabelas = retrato.tabelas.clone();
    marcar_segredos(&mut tabelas, &retrato.segredos_cifrados, nomes_do_segredo);
    json!({
        "formato": FORMATO_BACKUP,
        "versao_formato": VERSAO_FORMATO,
        "exportado_em": exportado_em,
        "org_origem": {
            "id": retrato.org.get("id").cloned().unwrap_or(Value::Null),
            "slug": retrato.org.get("slug").cloned().unwrap_or(Value::Null),
            "nome": retrato.org.get("nome").cloned().unwrap_or(Value::Null),
        },
        "com_conversas": com_conversas,
        "org": retrato.org,
        "tabelas": tabelas,
        "blobs": blobs
            .iter()
            .map(|(sha, bytes)| json!({"sha256": sha, "conteudo_base64": codificar(bytes)}))
            .collect::<Vec<_>>(),
    })
}

/// Lê e valida um arquivo de backup inteiro.
pub fn ler_documento(bytes: &[u8]) -> Result<DocumentoBackup, ErroDocumento> {
    let documento: Value =
        serde_json::from_slice(bytes).map_err(|erro| ErroDocumento::Json(erro.to_string()))?;
    let retrato = retrato_do_documento(&documento)?;
    let blobs = blobs_do_documento(&documento)?;
    let com_conversas = documento
        .get("com_conversas")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(DocumentoBackup {
        retrato,
        blobs,
        com_conversas,
    })
}

/// Valida o documento e devolve o retrato pronto para o `importar_retrato`.
pub fn retrato_do_documento(documento: &Value) -> Result<RetratoOrg, ErroDocumento> {
    let formato = documento
        .get("formato")
        .and_then(Value::as_str)
        .unwrap_or("");
    if formato != FORMATO_BACKUP {
        return Err(ErroDocumento::Formato(format!(
            "este arquivo não é um backup do POP Harness (formato `{formato}`)"
        )));
    }
    let versao = documento
        .get("versao_formato")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if versao != u64::from(VERSAO_FORMATO) {
        return Err(ErroDocumento::Versao {
            lida: versao,
            entendida: VERSAO_FORMATO,
        });
    }
    let org = documento
        .get("org")
        .cloned()
        .filter(Value::is_object)
        .ok_or_else(|| ErroDocumento::Formato("o arquivo não traz a linha da org".to_owned()))?;
    let tabelas = documento
        .get("tabelas")
        .and_then(Value::as_object)
        .ok_or_else(|| ErroDocumento::Formato("o arquivo não traz as tabelas da org".to_owned()))?;
    let mut retrato = BTreeMap::new();
    for (nome, linhas) in tabelas {
        let linhas = linhas.as_array().ok_or_else(|| {
            ErroDocumento::Formato(format!("a tabela `{nome}` do arquivo não é uma lista"))
        })?;
        retrato.insert(nome.clone(), linhas.clone());
    }
    Ok(RetratoOrg {
        org,
        tabelas: retrato,
        segredos_cifrados: BTreeMap::new(),
    })
}

/// Decodifica e confere os blobs do arquivo (sha256 calculado tem de bater com o gravado).
pub fn blobs_do_documento(documento: &Value) -> Result<BTreeMap<String, Vec<u8>>, ErroDocumento> {
    let lista = documento
        .get("blobs")
        .and_then(Value::as_array)
        .ok_or_else(|| ErroDocumento::Formato("o arquivo não traz a lista de blobs".to_owned()))?;
    let mut blobs = BTreeMap::new();
    for item in lista {
        let sha = item
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| ErroDocumento::Formato("blob do arquivo sem sha256".to_owned()))?;
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ErroDocumento::Formato(format!(
                "o sha256 `{sha}` do arquivo não é válido"
            )));
        }
        let texto = item
            .get("conteudo_base64")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ErroDocumento::Formato(format!("o blob {sha} do arquivo não tem conteúdo"))
            })?;
        let bytes = descodificar(texto).map_err(|erro| {
            ErroDocumento::Formato(format!(
                "o conteúdo do blob {sha} do arquivo não abre: {erro}"
            ))
        })?;
        if sha256_hex(&bytes) != sha {
            return Err(ErroDocumento::Formato(format!(
                "o conteúdo de um arquivo do backup não confere com o sha256 gravado ({sha}); \
                 o arquivo está corrompido ou foi alterado"
            )));
        }
        blobs.insert(sha.to_owned(), bytes);
    }
    Ok(blobs)
}

/// Acrescenta a cada integração os nomes dos segredos que estavam preenchidos.
///
/// Sem a chave de cifra (ou com uma chave diferente da que gravou), os nomes não são legíveis:
/// fica `null`, e só a marca `tem_segredo` diz que havia segredo. O valor nunca aparece.
pub fn marcar_segredos(
    tabelas: &mut BTreeMap<String, Vec<Value>>,
    cifrados: &BTreeMap<String, Vec<u8>>,
    nomes_do_segredo: &dyn Fn(&[u8]) -> Option<Vec<String>>,
) {
    let Some(linhas) = tabelas.get_mut("integrations") else {
        return;
    };
    for linha in linhas {
        let Some(id) = linha.get("id").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let campos = cifrados
            .get(&id)
            .and_then(|blob| nomes_do_segredo(blob))
            .map(|nomes| nomes.into_iter().map(Value::String).collect::<Vec<_>>());
        if let Some(objeto) = linha.as_object_mut() {
            objeto.insert(
                "segredos_campos".to_owned(),
                campos.map(Value::Array).unwrap_or(Value::Null),
            );
        }
    }
}

/// sha256 em hexadecimal minúsculo.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Base64 padrão (RFC 4648, com preenchimento).
pub fn codificar(bytes: &[u8]) -> String {
    const ALFABETO: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut saida = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for bloco in bytes.chunks(3) {
        let b0 = u32::from(bloco[0]);
        let b1 = bloco.get(1).map_or(0, |b| u32::from(*b));
        let b2 = bloco.get(2).map_or(0, |b| u32::from(*b));
        let numero = (b0 << 16) | (b1 << 8) | b2;
        saida.push(char::from(ALFABETO[((numero >> 18) & 63) as usize]));
        saida.push(char::from(ALFABETO[((numero >> 12) & 63) as usize]));
        if bloco.len() > 1 {
            saida.push(char::from(ALFABETO[((numero >> 6) & 63) as usize]));
        } else {
            saida.push('=');
        }
        if bloco.len() > 2 {
            saida.push(char::from(ALFABETO[(numero & 63) as usize]));
        } else {
            saida.push('=');
        }
    }
    saida
}

/// Decodifica base64 padrão. Recusa tamanho, caractere ou preenchimento fora de lugar.
pub fn descodificar(texto: &str) -> Result<Vec<u8>, String> {
    let limpo = texto.trim().as_bytes();
    if !limpo.len().is_multiple_of(4) {
        return Err("tamanho de base64 inválido".to_owned());
    }
    let blocos = limpo.len() / 4;
    let mut saida = Vec::with_capacity(blocos * 3);
    for (indice, bloco) in limpo.chunks(4).enumerate() {
        let ultimo = indice + 1 == blocos;
        let mut valores = [0u8; 4];
        let mut preenchimento = 0usize;
        for (posicao, byte) in bloco.iter().enumerate() {
            if *byte == b'=' {
                if !ultimo || posicao < 2 {
                    return Err("preenchimento de base64 fora do fim".to_owned());
                }
                preenchimento += 1;
                continue;
            }
            if preenchimento > 0 {
                return Err("base64 com caractere depois do preenchimento".to_owned());
            }
            valores[posicao] = valor_base64(*byte)?;
        }
        let numero = (u32::from(valores[0]) << 18)
            | (u32::from(valores[1]) << 12)
            | (u32::from(valores[2]) << 6)
            | u32::from(valores[3]);
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

fn valor_base64(byte: u8) -> Result<u8, String> {
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
mod testes {
    use super::*;

    fn retrato_de_teste() -> RetratoOrg {
        let mut tabelas: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        tabelas.insert(
            "agents".to_owned(),
            vec![json!({"id": "0192f4b0-0000-7000-8000-000000000001", "slug": "bia"})],
        );
        tabelas.insert(
            "integrations".to_owned(),
            vec![json!({"id": "integ-1", "tem_segredo": true})],
        );
        RetratoOrg {
            org: json!({
                "id": "0192f4b0-0000-7000-8000-0000000000aa",
                "slug": "org-teste",
                "nome": "Org de teste",
            }),
            tabelas,
            segredos_cifrados: BTreeMap::from([("integ-1".to_owned(), b"cifrado".to_vec())]),
        }
    }

    #[test]
    fn documento_leva_formato_versao_org_e_nomes_de_segredo() {
        let retrato = retrato_de_teste();
        let blobs = BTreeMap::from([("a".repeat(64), b"conteudo".to_vec())]);
        let nomes = |_blob: &[u8]| Some(vec!["token".to_owned()]);
        let doc = documento(&retrato, true, 1_700_000_000_000, &blobs, &nomes);
        assert_eq!(doc["formato"], FORMATO_BACKUP);
        assert_eq!(doc["versao_formato"], VERSAO_FORMATO);
        assert_eq!(doc["org_origem"]["slug"], "org-teste");
        assert_eq!(doc["com_conversas"], true);
        assert_eq!(
            doc["tabelas"]["integrations"][0]["segredos_campos"],
            json!(["token"])
        );
        // O conteúdo cifrado não sai no documento.
        assert!(!doc.to_string().contains("cifrado"));
    }

    #[test]
    fn ler_documento_recusa_json_versao_e_conteudo_adulterado() {
        let erro = ler_documento(b"nao e json").expect_err("json invalido");
        assert!(matches!(erro, ErroDocumento::Json(_)), "{erro}");

        let doc = json!({"formato": "outra-coisa", "versao_formato": 1});
        let erro = ler_documento(doc.to_string().as_bytes()).expect_err("formato");
        assert!(matches!(erro, ErroDocumento::Formato(_)), "{erro}");

        let doc = json!({"formato": FORMATO_BACKUP, "versao_formato": 99, "org": {}});
        let erro = ler_documento(doc.to_string().as_bytes()).expect_err("versao");
        assert!(erro.to_string().contains("versão de formato"), "{erro}");

        let doc = json!({
            "formato": FORMATO_BACKUP,
            "versao_formato": VERSAO_FORMATO,
            "org": {"id": "0192f4b0-0000-7000-8000-0000000000aa"},
            "tabelas": {},
            "blobs": [{"sha256": "0".repeat(64), "conteudo_base64": codificar(b"conteudo")}],
        });
        let erro = ler_documento(doc.to_string().as_bytes()).expect_err("blob adulterado");
        assert!(erro.to_string().contains("não confere"), "{erro}");
    }

    #[test]
    fn documento_vai_e_volta() {
        let retrato = retrato_de_teste();
        let blobs = BTreeMap::from([
            (
                "a".repeat(64),
                "conteudo com acentos: ação".as_bytes().to_vec(),
            ),
            ("b".repeat(64), Vec::new()),
        ]);
        let nomes = |_blob: &[u8]| None;
        // Sha falso não confere na leitura: o laço usa o sha real calculado do conteúdo.
        let blobs_reais = blobs
            .values()
            .map(|bytes| (sha256_hex(bytes), bytes.clone()))
            .collect::<BTreeMap<_, _>>();
        let doc = documento(&retrato, false, 1_700_000_000_000, &blobs_reais, &nomes);
        let lido = ler_documento(doc.to_string().as_bytes()).expect("ler o que acabou de montar");
        assert_eq!(lido.blobs, blobs_reais);
        assert!(!lido.com_conversas);
        assert_eq!(lido.retrato.tabelas["agents"], retrato.tabelas["agents"]);
        // A integração volta com a marca de segredo e sem o valor (o blob cifrado não viaja).
        let integracao = &lido.retrato.tabelas["integrations"][0];
        assert_eq!(integracao["tem_segredo"], true);
        assert!(integracao["segredos_campos"].is_null());
        assert!(lido.retrato.segredos_cifrados.is_empty());
    }

    #[test]
    fn base64_vai_e_volta() {
        let casos: &[&[u8]] = &[
            b"",
            b"f",
            b"fo",
            b"foo",
            b"conteudo maior, com acentos: acao",
        ];
        for caso in casos {
            let texto = codificar(caso);
            let voltou = descodificar(&texto).expect("decodifica");
            assert_eq!(&voltou, caso, "base64 de {:?}", caso);
        }
    }

    #[test]
    fn base64_recusa_entrada_invalida() {
        for texto in ["Zm9vYg", "Zm9v-YI==", "Zg==Zg==", "Zm9=v"] {
            assert!(descodificar(texto).is_err(), "{texto} devia ser recusado");
        }
    }
}
