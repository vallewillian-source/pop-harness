use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_files::FileConfig;
use poph_store::{LibraryFile, NewLibraryFile, Page, Store};
use poph_types::{AgentId, FileId, HarnessConfig, OrgId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, parse_id, print_json, require_offline, resolve_org, validate_limit,
};

#[derive(Debug, Subcommand)]
pub enum FileAction {
    #[command(
        about = "Põe um arquivo na biblioteca do agente (limite de 16 MB, tipo conferido pelo conteúdo)"
    )]
    Upload(UploadLibraryFile),
    #[command(about = "Lista os arquivos da biblioteca do agente")]
    List(ListFiles),
    #[command(about = "Mostra um arquivo da biblioteca")]
    Show(FileRef),
    #[command(about = "Tira um arquivo da biblioteca (o blob fica; a retenção apaga órfão)")]
    Delete(DeleteFile),
}

#[derive(Debug, Args)]
pub struct UploadLibraryFile {
    #[arg(value_name = "CAMINHO", help = "Caminho do arquivo no disco")]
    pub path: PathBuf,

    #[arg(long, help = "Agente dono do arquivo: id (UUID v7) ou slug")]
    pub agent: String,

    #[arg(
        long,
        default_value = "",
        help = "Nome mostrado ao cliente (padrão: o nome do arquivo no disco)"
    )]
    pub name: String,

    #[arg(
        long,
        default_value = "",
        help = "Descrição que o modelo lê para decidir quando enviar"
    )]
    pub description: String,

    #[arg(long, help = "Saída em JSON (formato estável, sem segredo)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListFiles {
    #[arg(long, help = "Agente dono dos arquivos: id (UUID v7) ou slug")]
    pub agent: String,

    #[arg(long, default_value_t = 50, help = "Quantos arquivos trazer (1 a 200)")]
    pub limit: u32,

    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct FileRef {
    #[arg(help = "Id do arquivo (UUID v7)")]
    pub id: String,

    #[arg(long, help = "Agente dono do arquivo: id (UUID v7) ou slug")]
    pub agent: String,

    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteFile {
    #[arg(help = "Id do arquivo (UUID v7)")]
    pub id: String,

    #[arg(long, help = "Agente dono do arquivo: id (UUID v7) ou slug")]
    pub agent: String,

    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    action: FileAction,
) -> ExitCode {
    if let Err(error) = require_offline(offline) {
        return failure(error);
    }
    let (store, org, configuration) = match open(flag, environment, org) {
        Ok(triple) => triple,
        Err(error) => return failure(error),
    };
    match action {
        FileAction::Upload(args) => upload(&store, org, &configuration, args),
        FileAction::List(args) => list(&store, org, args),
        FileAction::Show(args) => show(&store, org, args),
        FileAction::Delete(args) => delete(&store, org, args),
    }
}

fn open(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
) -> Result<(Store, OrgId, HarnessConfig), String> {
    let text = org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &folder);
    let configuration =
        config::load(file.as_deref(), environment).map_err(|error| error.to_string())?;
    let store = Store::open(&configuration)
        .map_err(|error| format!("não consegui abrir o banco: {error}"))?;
    let org = resolve_org(&store, text)?;
    Ok((store, org, configuration))
}

fn upload(
    store: &Store,
    org: OrgId,
    configuration: &HarnessConfig,
    args: UploadLibraryFile,
) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    let config = FileConfig::default(configuration.blobs());
    let bytes = match read_capped(&args.path, config.byte_limit) {
        Ok(bytes) => bytes,
        Err(error) => return failure(error),
    };
    let file_name = args
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "arquivo".to_owned());
    let received = match poph_files::receive(&config, org, &file_name, &bytes) {
        Ok(received) => received,
        Err(error) => return failure(error.to_string()),
    };
    let metadata = received.metadados;
    let name = if args.name.trim().is_empty() {
        metadata.original_name.clone()
    } else {
        args.name
    };
    let blob_relative = blob_relative_path(&config.blob_folder, &metadata.blob_path);
    let new_file = NewLibraryFile {
        agent_id: agent,
        name,
        description: args.description,
        mime: metadata.mime_real.clone(),
        size_bytes: metadata.size_bytes,
        sha256: metadata.sha256.clone(),
        blob_path: blob_relative,
    };
    match store.library().create(org, new_file) {
        Ok(file) => print_file(
            &file,
            args.json,
            if metadata.blob_already_existed {
                "O conteúdo já estava guardado para esta org: um blob só, deduplicado por hash."
            } else {
                "Blob gravado."
            },
        ),
        Err(error) => failure(describe(&error)),
    }
}

fn list(store: &Store, org: OrgId, args: ListFiles) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    let page: Page = match validate_limit(args.limit) {
        Ok(page) => page,
        Err(error) => return failure(error),
    };
    let total = match store.library().count_rows(org, agent) {
        Ok(total) => total,
        Err(error) => return failure(describe(&error)),
    };
    let files = match store.library().list(org, agent, page) {
        Ok(files) => files,
        Err(error) => return failure(describe(&error)),
    };
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "total": total,
            "arquivos": files.iter().map(file_json).collect::<Vec<_>>(),
        }));
        return ExitCode::SUCCESS;
    }
    if files.is_empty() {
        println!(
            "A biblioteca deste agente está vazia. Use `poph file upload <caminho> --agent` para \
             cadastrar um arquivo."
        );
        return ExitCode::SUCCESS;
    }
    println!("Arquivos do agente ({total} no total):\n");
    for file in &files {
        println!(
            "{}  {}  ({}, {})",
            file.id,
            file.name,
            file.mime,
            poph_agent::library::size_readable(file.size_bytes)
        );
        if !file.description.trim().is_empty() {
            println!("    descrição: {}", file.description.trim());
        }
    }
    if total > files.len() as u64 {
        println!("\n(mostrando {} de {total})", files.len());
    }
    ExitCode::SUCCESS
}

fn show(store: &Store, org: OrgId, args: FileRef) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    let id: FileId = match parse_id(&args.id) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    match store.library().get(org, agent, id) {
        Ok(file) => print_file(&file, args.json, ""),
        Err(error) => failure(describe(&error)),
    }
}

fn delete(store: &Store, org: OrgId, args: DeleteFile) -> ExitCode {
    let agent = match resolve_agent(store, org, &args.agent) {
        Ok(agent) => agent,
        Err(error) => return failure(error),
    };
    let id: FileId = match parse_id(&args.id) {
        Ok(id) => id,
        Err(error) => return failure(error),
    };
    let file = match store.library().get(org, agent, id) {
        Ok(file) => file,
        Err(error) => return failure(describe(&error)),
    };
    if let Err(error) = store.library().delete(org, agent, id) {
        return failure(describe(&error));
    }
    if args.json {
        print_json(&json!({
            "versao_relatorio": 1,
            "excluido": file_json(&file),
        }));
    } else {
        println!(
            "Arquivo removido da biblioteca: {} ({}).",
            file.name, file.id
        );
        println!("O conteúdo guardado fica no disco até a retenção apagar o que não tem dono.");
    }
    ExitCode::SUCCESS
}

fn print_file(file: &LibraryFile, json_output: bool, notice: &str) -> ExitCode {
    if json_output {
        print_json(&json!({
            "versao_relatorio": 1,
            "arquivo": file_json(file),
        }));
        return ExitCode::SUCCESS;
    }
    println!("Arquivo na biblioteca do agente:\n");
    println!("  id:          {}", file.id);
    println!("  nome:        {}", file.name);
    if !file.description.trim().is_empty() {
        println!("  descrição:   {}", file.description.trim());
    }
    println!("  tipo:        {}", file.mime);
    println!(
        "  tamanho:     {}",
        poph_agent::library::size_readable(file.size_bytes)
    );
    println!("  sha256:      {}", file.sha256);
    println!("  blob:        {}", file.blob_path);
    println!("\nO agente envia este arquivo com a ferramenta `enviar_arquivo` passando este id.");
    if !notice.is_empty() {
        println!("{notice}");
    }
    ExitCode::SUCCESS
}

fn file_json(file: &LibraryFile) -> Value {
    json!({
        "id": file.id.to_string(),
        "agent_id": file.agent_id.to_string(),
        "nome": file.name,
        "descricao": file.description,
        "mime": file.mime,
        "tamanho_bytes": file.size_bytes,
        "sha256": file.sha256,
        "blob_path": file.blob_path,
        "criado_em": file.created_at,
    })
}

fn read_capped(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("não consegui abrir {}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("não consegui ler {}: {error}", path.display()))?;
    Ok(bytes)
}

fn blob_relative_path(folder: &Path, path: &Path) -> String {
    path.strip_prefix(folder)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn resolve_agent(store: &Store, org: OrgId, text: &str) -> Result<AgentId, String> {
    if uuid::Uuid::parse_str(text).is_ok() {
        let id: AgentId = parse_id(text)?;
        return store
            .agents()
            .get(org, id)
            .map(|agent| agent.id)
            .map_err(|error| describe(&error));
    }
    store
        .agents()
        .by_slug(org, text)
        .map(|agent| agent.id)
        .map_err(|error| describe(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_blob_path() {
        let folder = Path::new("/dados/blobs");
        let file = folder.join("org-1").join("abc123");
        assert_eq!(blob_relative_path(folder, &file), "org-1/abc123");
        assert_eq!(
            blob_relative_path(folder, Path::new("/outro/lugar")),
            "/outro/lugar"
        );
    }
}
