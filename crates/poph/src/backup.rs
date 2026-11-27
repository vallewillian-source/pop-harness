use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_files::FileConfig;
use poph_server::CipherKey;
use poph_store::Store;
use poph_store::backup::{ImportTarget, OrgSnapshot};
use poph_store::backup_document;
use poph_types::{HarnessConfig, OrgId};
use serde_json::{Map, Value, json};

use crate::config;
use crate::support::{failure, parse_id, print_json, require_offline, resolve_org};

#[derive(Debug, Subcommand)]
pub enum BackupAction {
    #[command(about = "Exporta uma org inteira num arquivo único")]
    Export(ExportOrg),
    #[command(about = "Reconstrói a org a partir de um arquivo de `backup export`")]
    Import(ImportOrg),
}

#[derive(Debug, Args)]
pub struct ExportOrg {
    #[arg(help = "Org a exportar: id (UUID v7) ou slug")]
    pub org: String,

    #[arg(
        long,
        value_name = "ARQUIVO",
        help = "Arquivo de saída (um só, com tudo dentro)"
    )]
    pub out: PathBuf,

    #[arg(
        long = "com-conversas",
        help = "Inclui conversas, mensagens, turnos, as duas filas e os carrinhos"
    )]
    pub with_conversations: bool,

    #[arg(
        long = "sobrescrever",
        help = "Sobrescreve o arquivo se ele já existir"
    )]
    pub overwrite: bool,

    #[arg(
        long,
        help = "Saída em JSON (formato estável, sem segredo e sem caminho de blob)"
    )]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ImportOrg {
    #[arg(
        value_name = "ARQUIVO",
        help = "Arquivo gerado por `poph backup export`"
    )]
    pub file: PathBuf,

    #[arg(
        long,
        help = "Org de destino já existente (id ou slug). Sem ele, vale a org de origem do arquivo"
    )]
    pub org: Option<String>,

    #[arg(
        long = "sobrescrever",
        help = "Autoriza atualizar linhas que já existem no destino (padrão: a importação recusa)"
    )]
    pub overwrite: bool,

    #[arg(long, help = "Saída em JSON (formato estável)")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    action: BackupAction,
) -> ExitCode {
    if let Err(error) = require_offline(offline) {
        return failure(error);
    }
    match action {
        BackupAction::Export(args) => export(flag, environment, args),
        BackupAction::Import(args) => import(flag, environment, args),
    }
}

fn export(flag: Option<PathBuf>, environment: &config::Environment, args: ExportOrg) -> ExitCode {
    if args.out.exists() && !args.overwrite {
        return failure(format!(
            "o arquivo {} já existe; escolha outro caminho ou use --sobrescrever",
            args.out.display()
        ));
    }
    let (store, configuration) = match open_store(flag, environment) {
        Ok(pair) => pair,
        Err(error) => return failure(error),
    };
    let org = match resolve_org(&store, &args.org) {
        Ok(org) => org,
        Err(error) => return failure(error),
    };
    let snapshot = match store.org_snapshot(org, args.with_conversations) {
        Ok(snapshot) => snapshot,
        Err(error) => return failure(crate::support::describe(&error)),
    };

    let blob_dir = configuration.blobs();
    let limit = FileConfig::default(&blob_dir).byte_limit;
    let blobs = match read_blobs(&snapshot, org, &blob_dir, limit) {
        Ok(blobs) => blobs,
        Err(error) => return failure(error),
    };

    let key = config::secrets(environment)
        .cipher
        .as_ref()
        .and_then(|value| CipherKey::derive(value.expose()).ok());

    let secret_names = |blob: &[u8]| -> Option<Vec<String>> {
        let map = key.as_ref()?.decifrar(blob).ok()?;
        Some(map.keys().cloned().collect())
    };
    let document = backup_document::document(
        &snapshot,
        args.with_conversations,
        store.now(),
        &blobs,
        &secret_names,
    );
    let bytes = match serde_json::to_vec(&document) {
        Ok(bytes) => bytes,
        Err(error) => return failure(format!("não consegui montar o arquivo: {error}")),
    };
    if let Err(error) = write_atomically(&args.out, &bytes) {
        return failure(error);
    }

    let size = bytes.len() as u64;
    let counts: Map<String, Value> = snapshot
        .tables
        .iter()
        .map(|(name, rows)| (name.clone(), Value::from(rows.len())))
        .collect();
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "arquivo": args.out.display().to_string(),
            "tamanho_bytes": size,
            "com_conversas": args.with_conversations,
            "org": {
                "id": snapshot.org.get("id").cloned().unwrap_or(Value::Null),
                "slug": snapshot.org.get("slug").cloned().unwrap_or(Value::Null),
            },
            "tabelas": counts,
            "blobs": blobs.len(),
        }));
        return ExitCode::SUCCESS;
    }
    println!(
        "Org exportada para {} ({} KB).",
        args.out.display(),
        size.div_ceil(1024)
    );
    println!();
    for (name, rows) in &snapshot.tables {
        if !rows.is_empty() {
            println!("  {name}: {}", rows.len());
        }
    }
    println!();
    println!(
        "Blobs no arquivo: {} (um por sha256; o caminho é recalculado na importação).",
        blobs.len()
    );
    if !args.with_conversations {
        println!("Conversas não entraram (use --com-conversas para levá-las).");
    }
    ExitCode::SUCCESS
}

fn read_blobs(
    snapshot: &OrgSnapshot,
    org: OrgId,
    blob_dir: &Path,
    limit: u64,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut names = BTreeMap::new();
    for table in ["file_library", "attachments"] {
        for row in snapshot.tables.get(table).into_iter().flatten() {
            let (Some(sha), Some(name)) = (
                row.get("sha256").and_then(Value::as_str),
                row.get("name").and_then(Value::as_str),
            ) else {
                continue;
            };
            names
                .entry(sha.to_owned())
                .or_insert_with(|| name.to_owned());
        }
    }
    let mut blobs = BTreeMap::new();
    for (sha, name) in names {
        let bytes = poph_files::blob::read(blob_dir, org, &sha, limit).map_err(|error| {
            format!("o conteúdo do arquivo `{name}` (sha256 {sha}) não está disponível: {error}")
        })?;
        blobs.insert(sha, bytes);
    }
    Ok(blobs)
}

fn import(flag: Option<PathBuf>, environment: &config::Environment, args: ImportOrg) -> ExitCode {
    let (store, configuration) = match open_store(flag, environment) {
        Ok(pair) => pair,
        Err(error) => return failure(error),
    };
    let bytes = match fs::read(&args.file) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(format!("não consegui ler {}: {error}", args.file.display()));
        }
    };
    let (snapshot, blobs) = match backup_document::read_document(&bytes) {
        Ok(document) => (document.snapshot, document.blobs),
        Err(poph_store::backup_document::DocumentError::Json(reason)) => {
            return failure(format!(
                "o arquivo {} não é um backup do POP Harness (JSON inválido: {reason})",
                args.file.display()
            ));
        }
        Err(error) => return failure(error.to_string()),
    };

    let target = match &args.org {
        Some(text) => match resolve_org(&store, text) {
            Ok(org) => ImportTarget::Existing(org),
            Err(error) => return failure(error),
        },
        None => ImportTarget::Origin,
    };
    let target_org = match target {
        ImportTarget::Existing(org) => org,
        ImportTarget::Origin => match snapshot_org(&snapshot) {
            Ok(org) => org,
            Err(error) => return failure(error),
        },
    };

    if !args.overwrite
        && let Err(error) = refuse_overwrite(&store, &snapshot, target)
    {
        return failure(error);
    }

    let blob_dir = configuration.blobs();
    for (sha, content) in &blobs {
        match poph_files::blob::write(&blob_dir, target_org, content) {
            Ok(written) if written.sha256 == *sha => {}
            Ok(_) => {
                return failure(format!(
                    "o conteúdo do blob {sha} foi gravado com outro sha256; nada foi importado"
                ));
            }
            Err(error) => {
                return failure(format!(
                    "não consegui gravar o conteúdo de um arquivo do backup: {error}"
                ));
            }
        }
    }

    let store = Arc::new(store);
    let report = match store.import_snapshot(snapshot, target) {
        Ok(report) => report,
        Err(error) => return failure(crate::support::describe(&error)),
    };

    let (index, notice) = reindex(&configuration, &store, report.org);

    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "arquivo": args.file.display().to_string(),
            "org": {
                "id": report.org.to_string(),
                "criada": report.created_org,
            },
            "tabelas": report.tables.iter().map(|(name, written)| {
                (name.clone(), json!({"inseridas": written.inserted, "atualizadas": written.updated}))
            }).collect::<Map<String, Value>>(),
            "total": report.total(),
            "segredos_a_redigitar": report.secrets_to_reenter,
            "indice": index,
            "aviso": notice,
        }));
        return ExitCode::SUCCESS;
    }

    println!(
        "Org {} {}.",
        report.org,
        if report.created_org {
            "criada"
        } else {
            "atualizada"
        }
    );
    println!();
    for (name, written) in &report.tables {
        if written.inserted + written.updated > 0 {
            println!(
                "  {name}: {} nova(s), {} atualizada(s)",
                written.inserted, written.updated
            );
        }
    }
    if report.secrets_to_reenter > 0 {
        println!();
        println!(
            "{} integração(ões) vieram sem o segredo: o valor não entra no arquivo. \
             Digite o segredo de novo na aba Integrações do Console.",
            report.secrets_to_reenter
        );
    }
    println!();
    println!(
        "Índice: base de conhecimento com {} trecho(s) em {} agente(s); produtos com {} trecho(s).",
        index["base_conhecimento"]["trechos"],
        index["base_conhecimento"]["agentes"],
        index["produtos"]["trechos"]
    );
    if let Some(notice) = notice {
        eprintln!("[aviso] {notice}");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
fn snapshot_from_document(document: &Value) -> Result<poph_store::backup::OrgSnapshot, String> {
    backup_document::document_snapshot(document).map_err(|error| error.to_string())
}

#[cfg(test)]
fn blobs_from_document(document: &Value) -> Result<BTreeMap<String, Vec<u8>>, String> {
    backup_document::document_blobs(document).map_err(|error| error.to_string())
}

#[cfg(test)]
fn mark_secrets(
    tables: &mut BTreeMap<String, Vec<Value>>,
    encrypted: &BTreeMap<String, Vec<u8>>,
    key: Option<&CipherKey>,
) {
    let names = |blob: &[u8]| -> Option<Vec<String>> {
        let map = key?.decifrar(blob).ok()?;
        Some(map.keys().cloned().collect())
    };
    backup_document::mark_secrets(tables, encrypted, &names);
}

fn refuse_overwrite(
    store: &Store,
    snapshot: &OrgSnapshot,
    target: ImportTarget,
) -> Result<(), String> {
    let updates = store
        .check_import_updates(snapshot, target)
        .map_err(|error| crate::support::describe(&error))?;
    if updates.empty() {
        return Ok(());
    }
    let conflicts = store
        .check_import_conflicts(snapshot, target)
        .map_err(|error| crate::support::describe(&error))?;
    if !conflicts.is_empty() {
        return Ok(());
    }
    let mut examples = updates.examples.join("; ");
    let remaining = updates.total - updates.examples.len() as u64;
    if remaining > 0 {
        examples.push_str(&format!("; e mais {remaining}"));
    }
    Err(format!(
        "a importação atualizaria {} linha(s) que já existem no destino ({}): {examples}. \
         Nada foi gravado. Se é isso que você quer, repita com --sobrescrever",
        updates.total,
        updates.summary(),
    ))
}

fn snapshot_org(snapshot: &OrgSnapshot) -> Result<OrgId, String> {
    let id = snapshot
        .org
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "o arquivo não traz o id da org de origem".to_owned())?;
    parse_id(id)
}

fn reindex(
    configuration: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
) -> (Value, Option<String>) {
    let pending = json!({
        "base_conhecimento": {"agentes": 0, "trechos": 0},
        "produtos": {"trechos": 0},
    });
    let base = match crate::rag::open_queued_knowledge_base(configuration, Arc::clone(store), org) {
        Ok(base) => base,
        Err(error) => {
            return (
                pending,
                Some(format!(
                    "{error}. O material foi importado, mas o índice não ficou pronto: rode \
                     `poph kb reindex` e `poph product reindex` quando o modelo estiver disponível"
                )),
            );
        }
    };
    let catalog = match crate::rag::open_catalog(configuration, Arc::clone(store)) {
        Ok(catalog) => catalog,
        Err(error) => {
            return (
                pending,
                Some(format!(
                    "{error}. O material foi importado, mas o índice de produtos não ficou pronto: \
                     rode `poph product reindex` quando o modelo estiver disponível"
                )),
            );
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return (
                pending,
                Some(format!(
                    "não consegui iniciar o runtime da reindexação ({error}); rode `poph kb \
                     reindex` e `poph product reindex`"
                )),
            );
        }
    };
    let base = match runtime.block_on(base.synchronise_org(org)) {
        Ok(reports) => reports,
        Err(error) => {
            return (
                pending,
                Some(format!(
                    "não consegui reindexar a base de conhecimento ({error}); rode `poph kb \
                     reindex`"
                )),
            );
        }
    };
    let products = match runtime.block_on(catalog.synchronise_org(org)) {
        Ok(reports) => reports,
        Err(error) => {
            return (
                pending,
                Some(format!(
                    "não consegui reindexar os produtos ({error}); rode `poph product reindex`"
                )),
            );
        }
    };
    (
        json!({
            "base_conhecimento": {
                "agentes": base.len(),
                "trechos": base.iter().map(|(_, report)| report.stored_chunks).sum::<usize>(),
            },
            "produtos": {
                "trechos": products.iter().map(|(_, report)| report.stored_chunks).sum::<usize>(),
            },
        }),
        None,
    )
}

fn open_store(
    flag: Option<PathBuf>,
    environment: &config::Environment,
) -> Result<(Store, HarnessConfig), String> {
    let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &directory);
    let configuration =
        config::load(file.as_deref(), environment).map_err(|error| error.to_string())?;
    let store = Store::open(&configuration)
        .map_err(|error| format!("não consegui abrir o banco: {error}"))?;
    Ok((store, configuration))
}

fn write_atomically(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = target
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "backup".to_owned());
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .map_err(|error| format!("não consegui escrever {}: {error}", temporary.display()))?;
    fs::rename(&temporary, target).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "não consegui renomear o arquivo para {}: {error}",
            target.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use poph_store::backup::BACKUP_FORMAT;
    use poph_store::backup_document::{descodificar, encode};

    #[test]
    fn base64_round_trips() {
        let cases: &[&[u8]] = &[
            b"",
            b"f",
            b"fo",
            b"foo",
            "conteudo de um arquivo maior, com acentos: ação".as_bytes(),
        ];
        for case in cases {
            let text = encode(case);
            let decoded = descodificar(&text).expect("decodifica");
            assert_eq!(&decoded, case, "base64 de {:?}", case);
        }
    }

    #[test]
    fn base64_rejects_invalid_input() {
        for text in ["Zm9vYg", "Zm9v-YI==", "Zg==Zg==", "Zm9=v"] {
            assert!(descodificar(text).is_err(), "{text} devia ser recusado");
        }
    }

    #[test]
    fn document_with_other_format_is_rejected() {
        let document = json!({"formato": "outra-coisa", "versao_formato": 1});
        assert!(snapshot_from_document(&document).is_err());
    }

    #[test]
    fn unknown_format_version_is_rejected() {
        let document = json!({"formato": BACKUP_FORMAT, "versao_formato": 99, "org": {}});
        let error = snapshot_from_document(&document).expect_err("devia recusar");
        assert!(error.contains("versão de formato"), "{error}");
    }

    #[test]
    fn tampered_blob_is_rejected() {
        let document = json!({
            "blobs": [{
                "sha256": "0".repeat(64),
                "conteudo_base64": encode(b"conteudo"),
            }],
        });
        let error = blobs_from_document(&document).expect_err("devia recusar");
        assert!(error.contains("não confere"), "{error}");
    }

    #[test]
    fn readable_secret_yields_names_and_unreadable_secret_yields_null() {
        let key = CipherKey::derive("chave-do-teste").expect("deriva");
        let mut map = std::collections::BTreeMap::new();
        map.insert("token".to_owned(), "valor-secreto".to_owned());
        let blob = key.cifrar(&map).expect("cifra");
        let mut encrypted = BTreeMap::new();
        encrypted.insert("integ-1".to_owned(), blob.clone());
        encrypted.insert("integ-2".to_owned(), vec![1, 2, 3]);
        let mut tables: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        tables.insert(
            "integrations".to_owned(),
            vec![
                json!({"id": "integ-1", "tem_segredo": true}),
                json!({"id": "integ-2", "tem_segredo": true}),
            ],
        );
        mark_secrets(&mut tables, &encrypted, Some(&key));
        let rows = &tables["integrations"];
        assert_eq!(rows[0]["segredos_campos"], json!(["token"]));
        assert!(rows[1]["segredos_campos"].is_null());

        assert!(
            !serde_json::to_string(&tables)
                .unwrap()
                .contains("valor-secreto")
        );
    }
}
