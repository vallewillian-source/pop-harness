//! `poph file`: biblioteca de arquivos do agente (HR-24, decisão D13).
//!
//! É a única origem de arquivo que o agente pode enviar ao cliente. O upload passa pelo crate
//! `poph-files` (limite de 16 MB, tipo real pelo conteúdo, blob por hash) e o registro vai para a
//! biblioteca do agente. O agente nunca escolhe caminho: só o id cadastrado aqui.
//!
//! Todos os comandos exigem `--offline` (como `org`, `token`, `kb`), `--org` e `--agent`, e aceitam
//! `--json`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_files::ConfigArquivos;
use poph_store::{ArquivoBiblioteca, NovoArquivoBiblioteca, Pagina, Store};
use poph_types::{AgentId, FileId, HarnessConfig, OrgId};
use serde_json::{Value, json};

use crate::apoio::{
    descrever, exigir_offline, falha, imprimir_json, parse_id, resolver_org, validar_limite,
};
use crate::config;

/// Subcomandos de `poph file`.
#[derive(Debug, Subcommand)]
pub enum AcaoArquivo {
    /// Põe um arquivo na biblioteca do agente (limite de 16 MB, tipo conferido pelo conteúdo).
    Upload(EnviarArquivoBiblioteca),
    /// Lista os arquivos da biblioteca do agente.
    List(ListarArquivos),
    /// Mostra um arquivo da biblioteca.
    Show(ReferenciaArquivo),
    /// Tira um arquivo da biblioteca (o blob fica; a retenção apaga órfão).
    Delete(ExcluirArquivo),
}

/// `poph file upload`.
#[derive(Debug, Args)]
pub struct EnviarArquivoBiblioteca {
    /// Caminho do arquivo no disco.
    pub caminho: PathBuf,

    /// Agente dono do arquivo: id (UUID v7) ou slug.
    #[arg(long)]
    pub agent: String,

    /// Nome mostrado ao cliente (padrão: o nome do arquivo no disco).
    #[arg(long, default_value = "")]
    pub name: String,

    /// Descrição que o modelo lê para decidir quando enviar.
    #[arg(long, default_value = "")]
    pub description: String,

    /// Saída em JSON (formato estável, sem segredo).
    #[arg(long)]
    pub json: bool,
}

/// `poph file list`.
#[derive(Debug, Args)]
pub struct ListarArquivos {
    /// Agente dono dos arquivos: id (UUID v7) ou slug.
    #[arg(long)]
    pub agent: String,

    /// Quantos arquivos trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph file show <id>`.
#[derive(Debug, Args)]
pub struct ReferenciaArquivo {
    /// Id do arquivo (UUID v7).
    pub id: String,

    /// Agente dono do arquivo: id (UUID v7) ou slug.
    #[arg(long)]
    pub agent: String,

    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph file delete <id>`.
#[derive(Debug, Args)]
pub struct ExcluirArquivo {
    /// Id do arquivo (UUID v7).
    pub id: String,

    /// Agente dono do arquivo: id (UUID v7) ou slug.
    #[arg(long)]
    pub agent: String,

    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// Executa `poph file`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    acao: AcaoArquivo,
) -> ExitCode {
    if let Err(erro) = exigir_offline(offline) {
        return falha(erro);
    }
    let (store, org, configuracao) = match abrir(flag, ambiente, org) {
        Ok(trio) => trio,
        Err(erro) => return falha(erro),
    };
    match acao {
        AcaoArquivo::Upload(args) => enviar(&store, org, &configuracao, args),
        AcaoArquivo::List(args) => listar(&store, org, args),
        AcaoArquivo::Show(args) => mostrar(&store, org, args),
        AcaoArquivo::Delete(args) => excluir(&store, org, args),
    }
}

fn abrir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
) -> Result<(Store, OrgId, HarnessConfig), String> {
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao =
        config::carregar(arquivo.as_deref(), ambiente).map_err(|erro| erro.to_string())?;
    let store = Store::abrir(&configuracao)
        .map_err(|erro| format!("não consegui abrir o banco: {erro}"))?;
    let org = resolver_org(&store, texto)?;
    Ok((store, org, configuracao))
}

/// Lê o arquivo do disco com teto, entrega ao `poph-files` e grava na biblioteca do agente.
fn enviar(
    store: &Store,
    org: OrgId,
    configuracao: &HarnessConfig,
    args: EnviarArquivoBiblioteca,
) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agent) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    let config = ConfigArquivos::padrao(configuracao.blobs());
    // Lê no máximo um byte além do limite: arquivo gigante é recusado sem carregar tudo na memória.
    let bytes = match ler_com_teto(&args.caminho, config.limite_bytes) {
        Ok(bytes) => bytes,
        Err(erro) => return falha(erro),
    };
    let nome_arquivo = args
        .caminho
        .file_name()
        .map(|nome| nome.to_string_lossy().into_owned())
        .unwrap_or_else(|| "arquivo".to_owned());
    let recebido = match poph_files::receber(&config, org, &nome_arquivo, &bytes) {
        Ok(recebido) => recebido,
        Err(erro) => return falha(erro.to_string()),
    };
    let metadados = recebido.metadados;
    let nome = if args.name.trim().is_empty() {
        metadados.nome_original.clone()
    } else {
        args.name
    };
    let blob_relativo = relativo_ao_blobs(&config.pasta_blobs, &metadados.caminho_blob);
    let novo = NovoArquivoBiblioteca {
        agent_id: agente,
        nome,
        descricao: args.description,
        mime: metadados.mime_real.clone(),
        tamanho_bytes: metadados.tamanho_bytes,
        sha256: metadados.sha256.clone(),
        blob_path: blob_relativo,
    };
    match store.biblioteca().criar(org, novo) {
        Ok(arquivo) => saida_arquivo(
            &arquivo,
            args.json,
            if metadados.blob_ja_existia {
                "O conteúdo já estava guardado para esta org: um blob só, deduplicado por hash."
            } else {
                "Blob gravado."
            },
        ),
        Err(erro) => falha(descrever(&erro)),
    }
}

fn listar(store: &Store, org: OrgId, args: ListarArquivos) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agent) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    let pagina: Pagina = match validar_limite(args.limit) {
        Ok(pagina) => pagina,
        Err(erro) => return falha(erro),
    };
    let total = match store.biblioteca().contar(org, agente) {
        Ok(total) => total,
        Err(erro) => return falha(descrever(&erro)),
    };
    let arquivos = match store.biblioteca().listar(org, agente, pagina) {
        Ok(arquivos) => arquivos,
        Err(erro) => return falha(descrever(&erro)),
    };
    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "total": total,
            "arquivos": arquivos.iter().map(json_arquivo).collect::<Vec<_>>(),
        }));
        return ExitCode::SUCCESS;
    }
    if arquivos.is_empty() {
        println!(
            "A biblioteca deste agente está vazia. Use `poph file upload <caminho> --agent` para \
             cadastrar um arquivo."
        );
        return ExitCode::SUCCESS;
    }
    println!("Arquivos do agente ({total} no total):\n");
    for arquivo in &arquivos {
        println!(
            "{}  {}  ({}, {})",
            arquivo.id,
            arquivo.nome,
            arquivo.mime,
            poph_agent::biblioteca::tamanho_legivel(arquivo.tamanho_bytes)
        );
        if !arquivo.descricao.trim().is_empty() {
            println!("    descrição: {}", arquivo.descricao.trim());
        }
    }
    if total > arquivos.len() as u64 {
        println!("\n(mostrando {} de {total})", arquivos.len());
    }
    ExitCode::SUCCESS
}

fn mostrar(store: &Store, org: OrgId, args: ReferenciaArquivo) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agent) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    let id: FileId = match parse_id(&args.id) {
        Ok(id) => id,
        Err(erro) => return falha(erro),
    };
    match store.biblioteca().obter(org, agente, id) {
        Ok(arquivo) => saida_arquivo(&arquivo, args.json, ""),
        Err(erro) => falha(descrever(&erro)),
    }
}

fn excluir(store: &Store, org: OrgId, args: ExcluirArquivo) -> ExitCode {
    let agente = match resolver_agente(store, org, &args.agent) {
        Ok(agente) => agente,
        Err(erro) => return falha(erro),
    };
    let id: FileId = match parse_id(&args.id) {
        Ok(id) => id,
        Err(erro) => return falha(erro),
    };
    let arquivo = match store.biblioteca().obter(org, agente, id) {
        Ok(arquivo) => arquivo,
        Err(erro) => return falha(descrever(&erro)),
    };
    if let Err(erro) = store.biblioteca().excluir(org, agente, id) {
        return falha(descrever(&erro));
    }
    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "excluido": json_arquivo(&arquivo),
        }));
    } else {
        println!(
            "Arquivo removido da biblioteca: {} ({}).",
            arquivo.nome, arquivo.id
        );
        println!("O conteúdo guardado fica no disco até a retenção apagar o que não tem dono.");
    }
    ExitCode::SUCCESS
}

fn saida_arquivo(arquivo: &ArquivoBiblioteca, json_saida: bool, aviso: &str) -> ExitCode {
    if json_saida {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "arquivo": json_arquivo(arquivo),
        }));
        return ExitCode::SUCCESS;
    }
    println!("Arquivo na biblioteca do agente:\n");
    println!("  id:          {}", arquivo.id);
    println!("  nome:        {}", arquivo.nome);
    if !arquivo.descricao.trim().is_empty() {
        println!("  descrição:   {}", arquivo.descricao.trim());
    }
    println!("  tipo:        {}", arquivo.mime);
    println!(
        "  tamanho:     {}",
        poph_agent::biblioteca::tamanho_legivel(arquivo.tamanho_bytes)
    );
    println!("  sha256:      {}", arquivo.sha256);
    println!("  blob:        {}", arquivo.blob_path);
    println!("\nO agente envia este arquivo com a ferramenta `enviar_arquivo` passando este id.");
    if !aviso.is_empty() {
        println!("{aviso}");
    }
    ExitCode::SUCCESS
}

fn json_arquivo(arquivo: &ArquivoBiblioteca) -> Value {
    json!({
        "id": arquivo.id.to_string(),
        "agent_id": arquivo.agent_id.to_string(),
        "nome": arquivo.nome,
        "descricao": arquivo.descricao,
        "mime": arquivo.mime,
        "tamanho_bytes": arquivo.tamanho_bytes,
        "sha256": arquivo.sha256,
        "blob_path": arquivo.blob_path,
        "criado_em": arquivo.criado_em,
    })
}

/// Lê o arquivo do disco, no máximo `limite + 1` bytes.
///
/// Um byte a mais é suficiente para o `poph-files` recusar por tamanho: assim um arquivo de vários
/// GB nunca entra inteiro na memória.
fn ler_com_teto(caminho: &Path, limite: u64) -> Result<Vec<u8>, String> {
    let arquivo = std::fs::File::open(caminho)
        .map_err(|erro| format!("não consegui abrir {}: {erro}", caminho.display()))?;
    let mut bytes = Vec::new();
    arquivo
        .take(limite.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|erro| format!("não consegui ler {}: {erro}", caminho.display()))?;
    Ok(bytes)
}

/// Caminho do blob relativo à pasta de blobs (`<org>/<sha256>`), como a coluna guarda.
fn relativo_ao_blobs(pasta: &Path, caminho: &Path) -> String {
    caminho
        .strip_prefix(pasta)
        .unwrap_or(caminho)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Agente pelo id (UUID v7) ou slug, dentro da org.
fn resolver_agente(store: &Store, org: OrgId, texto: &str) -> Result<AgentId, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: AgentId = parse_id(texto)?;
        return store
            .agentes()
            .obter(org, id)
            .map(|agente| agente.id)
            .map_err(|erro| descrever(&erro));
    }
    store
        .agentes()
        .por_slug(org, texto)
        .map(|agente| agente.id)
        .map_err(|erro| descrever(&erro))
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn caminho_relativo_ao_blobs() {
        let pasta = Path::new("/dados/blobs");
        let arquivo = pasta.join("org-1").join("abc123");
        assert_eq!(relativo_ao_blobs(pasta, &arquivo), "org-1/abc123");
        // Fora da pasta: devolve o caminho como veio, sem inventar prefixo.
        assert_eq!(
            relativo_ao_blobs(pasta, Path::new("/outro/lugar")),
            "/outro/lugar"
        );
    }
}
