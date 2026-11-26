//! `poph backup`: exporta e importa uma org num arquivo único (HR-53).
//!
//! O arquivo é JSON, com versão de formato declarada, a org de origem identificada, as linhas das
//! tabelas da org e os blobs (biblioteca e anexos) em base64 — deduplicados por sha256. A leitura
//! e a escrita das tabelas ficam no `poph-store` (`backup::RetratoOrg`); aqui ficam o arquivo, os
//! blobs (`poph-files`) e a reindexação depois da importação.
//!
//! Regras que valem aqui:
//! - segredo nunca sai: de integração o arquivo leva só os **nomes** dos campos preenchidos e a
//!   marca `tem_segredo`; o valor cifrado fica no banco (`secrets_enc`) e não entra no arquivo;
//! - caminho de blob nunca sai: o arquivo guarda `sha256` e os bytes, e o caminho é recalculado
//!   na importação (`<org de destino>/<sha256>`);
//! - os dois comandos exigem `--offline` (rodam direto no banco, com o servidor parado);
//! - o arquivo é escrito numa pasta temporária ao lado e renomeado no fim: não fica pela metade.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_files::FileConfig;
use poph_server::ChaveDeCifra;
use poph_store::Store;
use poph_store::backup::{ImportTarget, OrgSnapshot};
use poph_store::backup_document;
use poph_types::{HarnessConfig, OrgId};
use serde_json::{Map, Value, json};

use crate::apoio::{exigir_offline, falha, imprimir_json, parse_id, resolver_org};
use crate::config;

/// Subcomandos de `poph backup`.
#[derive(Debug, Subcommand)]
pub enum AcaoBackup {
    /// Exporta uma org inteira num arquivo único.
    Export(ExportarOrg),
    /// Reconstrói a org a partir de um arquivo de `backup export`.
    Import(ImportarOrg),
}

/// `poph backup export <org> --out <arquivo>`.
#[derive(Debug, Args)]
pub struct ExportarOrg {
    /// Org a exportar: id (UUID v7) ou slug.
    pub org: String,

    /// Arquivo de saída (um só, com tudo dentro).
    #[arg(long, value_name = "ARQUIVO")]
    pub out: PathBuf,

    /// Inclui conversas, mensagens, turnos, as duas filas e os carrinhos.
    #[arg(long)]
    pub com_conversas: bool,

    /// Sobrescreve o arquivo se ele já existir.
    #[arg(long)]
    pub sobrescrever: bool,

    /// Saída em JSON (formato estável, sem segredo e sem caminho de blob).
    #[arg(long)]
    pub json: bool,
}

/// `poph backup import <arquivo>`.
#[derive(Debug, Args)]
pub struct ImportarOrg {
    /// Arquivo gerado por `poph backup export`.
    pub arquivo: PathBuf,

    /// Org de destino já existente (id ou slug). Sem ele, vale a org de origem do arquivo.
    #[arg(long)]
    pub org: Option<String>,

    /// Autoriza atualizar linhas que já existem no destino (padrão: a importação recusa).
    #[arg(long)]
    pub sobrescrever: bool,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Executa `poph backup`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    acao: AcaoBackup,
) -> ExitCode {
    if let Err(erro) = exigir_offline(offline) {
        return falha(erro);
    }
    match acao {
        AcaoBackup::Export(args) => exportar(flag, ambiente, args),
        AcaoBackup::Import(args) => importar(flag, ambiente, args),
    }
}

// ───────────────────────────────── Exportar ─────────────────────────────────

fn exportar(flag: Option<PathBuf>, ambiente: &config::Ambiente, args: ExportarOrg) -> ExitCode {
    if args.out.exists() && !args.sobrescrever {
        return falha(format!(
            "o arquivo {} já existe; escolha outro caminho ou use --sobrescrever",
            args.out.display()
        ));
    }
    let (store, configuracao) = match abrir(flag, ambiente) {
        Ok(par) => par,
        Err(erro) => return falha(erro),
    };
    let org = match resolver_org(&store, &args.org) {
        Ok(org) => org,
        Err(erro) => return falha(erro),
    };
    let retrato = match store.org_snapshot(org, args.com_conversas) {
        Ok(retrato) => retrato,
        Err(erro) => return falha(crate::apoio::descrever(&erro)),
    };

    let pasta_blobs = configuracao.blobs();
    let limite = FileConfig::default(&pasta_blobs).byte_limit;
    let blobs = match ler_blobs(&retrato, org, &pasta_blobs, limite) {
        Ok(blobs) => blobs,
        Err(erro) => return falha(erro),
    };

    let chave = config::segredos(ambiente)
        .cifra
        .as_ref()
        .and_then(|valor| ChaveDeCifra::derivar(valor.expose()).ok());
    // A chave só serve para ler os **nomes** dos campos de segredo preenchidos; o valor nunca sai.
    let nomes_do_segredo = |blob: &[u8]| -> Option<Vec<String>> {
        let mapa = chave.as_ref()?.decifrar(blob).ok()?;
        Some(mapa.keys().cloned().collect())
    };
    let documento = backup_document::document(
        &retrato,
        args.com_conversas,
        store.now(),
        &blobs,
        &nomes_do_segredo,
    );
    let bytes = match serde_json::to_vec(&documento) {
        Ok(bytes) => bytes,
        Err(erro) => return falha(format!("não consegui montar o arquivo: {erro}")),
    };
    if let Err(erro) = escrever_atomico(&args.out, &bytes) {
        return falha(erro);
    }

    let tamanho = bytes.len() as u64;
    let contagens: Map<String, Value> = retrato
        .tables
        .iter()
        .map(|(nome, linhas)| (nome.clone(), Value::from(linhas.len())))
        .collect();
    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "arquivo": args.out.display().to_string(),
            "tamanho_bytes": tamanho,
            "com_conversas": args.com_conversas,
            "org": {
                "id": retrato.org.get("id").cloned().unwrap_or(Value::Null),
                "slug": retrato.org.get("slug").cloned().unwrap_or(Value::Null),
            },
            "tabelas": contagens,
            "blobs": blobs.len(),
        }));
        return ExitCode::SUCCESS;
    }
    println!(
        "Org exportada para {} ({} KB).",
        args.out.display(),
        tamanho.div_ceil(1024)
    );
    println!();
    for (nome, linhas) in &retrato.tables {
        if !linhas.is_empty() {
            println!("  {nome}: {}", linhas.len());
        }
    }
    println!();
    println!(
        "Blobs no arquivo: {} (um por sha256; o caminho é recalculado na importação).",
        blobs.len()
    );
    if !args.com_conversas {
        println!("Conversas não entraram (use --com-conversas para levá-las).");
    }
    ExitCode::SUCCESS
}

/// Lê do disco o conteúdo de cada blob citado no retrato (biblioteca e anexos).
fn ler_blobs(
    retrato: &OrgSnapshot,
    org: OrgId,
    pasta_blobs: &Path,
    limite: u64,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut nomes = BTreeMap::new();
    for tabela in ["file_library", "attachments"] {
        for linha in retrato.tables.get(tabela).into_iter().flatten() {
            let (Some(sha), Some(nome)) = (
                linha.get("sha256").and_then(Value::as_str),
                linha.get("name").and_then(Value::as_str),
            ) else {
                continue;
            };
            nomes
                .entry(sha.to_owned())
                .or_insert_with(|| nome.to_owned());
        }
    }
    let mut blobs = BTreeMap::new();
    for (sha, nome) in nomes {
        let bytes = poph_files::blob::read(pasta_blobs, org, &sha, limite).map_err(|erro| {
            format!("o conteúdo do arquivo `{nome}` (sha256 {sha}) não está disponível: {erro}")
        })?;
        blobs.insert(sha, bytes);
    }
    Ok(blobs)
}

// ───────────────────────────────── Importar ─────────────────────────────────

fn importar(flag: Option<PathBuf>, ambiente: &config::Ambiente, args: ImportarOrg) -> ExitCode {
    let (store, configuracao) = match abrir(flag, ambiente) {
        Ok(par) => par,
        Err(erro) => return falha(erro),
    };
    let bytes = match fs::read(&args.arquivo) {
        Ok(bytes) => bytes,
        Err(erro) => {
            return falha(format!(
                "não consegui ler {}: {erro}",
                args.arquivo.display()
            ));
        }
    };
    let (retrato, blobs) = match backup_document::read_document(&bytes) {
        Ok(documento) => (documento.snapshot, documento.blobs),
        Err(poph_store::backup_document::DocumentError::Json(motivo)) => {
            return falha(format!(
                "o arquivo {} não é um backup do POP Harness (JSON inválido: {motivo})",
                args.arquivo.display()
            ));
        }
        Err(erro) => return falha(erro.to_string()),
    };

    let destino = match &args.org {
        Some(texto) => match resolver_org(&store, texto) {
            Ok(org) => ImportTarget::Existing(org),
            Err(erro) => return falha(erro),
        },
        None => ImportTarget::Origin,
    };
    let org_destino = match destino {
        ImportTarget::Existing(org) => org,
        ImportTarget::Origin => match org_do_retrato(&retrato) {
            Ok(org) => org,
            Err(erro) => return falha(erro),
        },
    };

    // Recusa de sobrescrita antes de gravar qualquer coisa (nem blob): só `--sobrescrever` autoriza
    // a importação a regravar linhas que já existem no destino. Conflito de chave natural (mesmo
    // slug com outro id) tem precedência na mensagem e é recusado mesmo com o flag.
    if !args.sobrescrever
        && let Err(erro) = recusar_sobrescrita(&store, &retrato, destino)
    {
        return falha(erro);
    }

    // Blobs primeiro: são content-addressed (regravar o mesmo hash não muda nada) e ficam no disco
    // mesmo se a gravação no banco falhar — a retenção apaga órfão depois.
    let pasta_blobs = configuracao.blobs();
    for (sha, conteudo) in &blobs {
        match poph_files::blob::write(&pasta_blobs, org_destino, conteudo) {
            Ok(gravado) if gravado.sha256 == *sha => {}
            Ok(_) => {
                return falha(format!(
                    "o conteúdo do blob {sha} foi gravado com outro sha256; nada foi importado"
                ));
            }
            Err(erro) => {
                return falha(format!(
                    "não consegui gravar o conteúdo de um arquivo do backup: {erro}"
                ));
            }
        }
    }

    let store = Arc::new(store);
    let relatorio = match store.import_snapshot(retrato, destino) {
        Ok(relatorio) => relatorio,
        Err(erro) => return falha(crate::apoio::descrever(&erro)),
    };

    let (indice, aviso) = reindexar(&configuracao, &store, relatorio.org);

    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "arquivo": args.arquivo.display().to_string(),
            "org": {
                "id": relatorio.org.to_string(),
                "criada": relatorio.created_org,
            },
            "tabelas": relatorio.tables.iter().map(|(nome, gravado)| {
                (nome.clone(), json!({"inseridas": gravado.inserted, "atualizadas": gravado.updated}))
            }).collect::<Map<String, Value>>(),
            "total": relatorio.total(),
            "segredos_a_redigitar": relatorio.secrets_to_reenter,
            "indice": indice,
            "aviso": aviso,
        }));
        return ExitCode::SUCCESS;
    }

    println!(
        "Org {} {}.",
        relatorio.org,
        if relatorio.created_org {
            "criada"
        } else {
            "atualizada"
        }
    );
    println!();
    for (nome, gravado) in &relatorio.tables {
        if gravado.inserted + gravado.updated > 0 {
            println!(
                "  {nome}: {} nova(s), {} atualizada(s)",
                gravado.inserted, gravado.updated
            );
        }
    }
    if relatorio.secrets_to_reenter > 0 {
        println!();
        println!(
            "{} integração(ões) vieram sem o segredo: o valor não entra no arquivo. \
             Digite o segredo de novo na aba Integrações do Console.",
            relatorio.secrets_to_reenter
        );
    }
    println!();
    println!(
        "Índice: base de conhecimento com {} trecho(s) em {} agente(s); produtos com {} trecho(s).",
        indice["base_conhecimento"]["trechos"],
        indice["base_conhecimento"]["agentes"],
        indice["produtos"]["trechos"]
    );
    if let Some(aviso) = aviso {
        eprintln!("[aviso] {aviso}");
    }
    ExitCode::SUCCESS
}

/// Valida o documento (formato, versão, org, tabelas) — só para os testes locais da CLI.
#[cfg(test)]
fn retrato_do_documento(documento: &Value) -> Result<poph_store::backup::OrgSnapshot, String> {
    backup_document::document_snapshot(documento).map_err(|erro| erro.to_string())
}

/// Decodifica e confere os blobs — só para os testes locais da CLI.
#[cfg(test)]
fn blobs_do_documento(documento: &Value) -> Result<BTreeMap<String, Vec<u8>>, String> {
    backup_document::document_blobs(documento).map_err(|erro| erro.to_string())
}

/// Marca os segredos — só para os testes locais da CLI.
#[cfg(test)]
fn marcar_segredos(
    tabelas: &mut BTreeMap<String, Vec<Value>>,
    cifrados: &BTreeMap<String, Vec<u8>>,
    chave: Option<&ChaveDeCifra>,
) {
    let nomes = |blob: &[u8]| -> Option<Vec<String>> {
        let mapa = chave?.decifrar(blob).ok()?;
        Some(mapa.keys().cloned().collect())
    };
    backup_document::mark_secrets(tabelas, cifrados, &nomes);
}

/// Recusa a importação quando ela atualizaria linha que já existe no destino.
///
/// Conflito de chave natural vem primeiro: se o destino tem o mesmo slug com outro id, a mensagem
/// é a do conflito (a importação seria recusada de qualquer forma). Sem conflito, a mensagem diz
/// quantas linhas, em quais tabelas e até cinco exemplos, e como autorizar.
fn recusar_sobrescrita(
    store: &Store,
    retrato: &OrgSnapshot,
    destino: ImportTarget,
) -> Result<(), String> {
    let atualizacoes = store
        .check_import_updates(retrato, destino)
        .map_err(|erro| crate::apoio::descrever(&erro))?;
    if atualizacoes.empty() {
        return Ok(());
    }
    let conflitos = store
        .check_import_conflicts(retrato, destino)
        .map_err(|erro| crate::apoio::descrever(&erro))?;
    if !conflitos.is_empty() {
        // A gravação recusa com a mensagem de conflito, que é mais específica.
        return Ok(());
    }
    let mut exemplos = atualizacoes.examples.join("; ");
    let resto = atualizacoes.total - atualizacoes.examples.len() as u64;
    if resto > 0 {
        exemplos.push_str(&format!("; e mais {resto}"));
    }
    Err(format!(
        "a importação atualizaria {} linha(s) que já existem no destino ({}): {exemplos}. \
         Nada foi gravado. Se é isso que você quer, repita com --sobrescrever",
        atualizacoes.total,
        atualizacoes.summary(),
    ))
}

fn org_do_retrato(retrato: &OrgSnapshot) -> Result<OrgId, String> {
    let id = retrato
        .org
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "o arquivo não traz o id da org de origem".to_owned())?;
    parse_id(id)
}

// ───────────────────────────────── Reindexação ─────────────────────────────────

fn reindexar(
    configuracao: &HarnessConfig,
    store: &Arc<Store>,
    org: OrgId,
) -> (Value, Option<String>) {
    let pendente = json!({
        "base_conhecimento": {"agentes": 0, "trechos": 0},
        "produtos": {"trechos": 0},
    });
    let base = match crate::rag::abrir_base_com_fila(configuracao, Arc::clone(store), org) {
        Ok(base) => base,
        Err(erro) => {
            return (
                pendente,
                Some(format!(
                    "{erro}. O material foi importado, mas o índice não ficou pronto: rode \
                     `poph kb reindex` e `poph product reindex` quando o modelo estiver disponível"
                )),
            );
        }
    };
    let catalogo = match crate::rag::abrir_catalogo(configuracao, Arc::clone(store)) {
        Ok(catalogo) => catalogo,
        Err(erro) => {
            return (
                pendente,
                Some(format!(
                    "{erro}. O material foi importado, mas o índice de produtos não ficou pronto: \
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
        Err(erro) => {
            return (
                pendente,
                Some(format!(
                    "não consegui iniciar o runtime da reindexação ({erro}); rode `poph kb \
                     reindex` e `poph product reindex`"
                )),
            );
        }
    };
    let base = match runtime.block_on(base.synchronise_org(org)) {
        Ok(relatorios) => relatorios,
        Err(erro) => {
            return (
                pendente,
                Some(format!(
                    "não consegui reindexar a base de conhecimento ({erro}); rode `poph kb \
                     reindex`"
                )),
            );
        }
    };
    let produtos = match runtime.block_on(catalogo.synchronise_org(org)) {
        Ok(relatorios) => relatorios,
        Err(erro) => {
            return (
                pendente,
                Some(format!(
                    "não consegui reindexar os produtos ({erro}); rode `poph product reindex`"
                )),
            );
        }
    };
    (
        json!({
            "base_conhecimento": {
                "agentes": base.len(),
                "trechos": base.iter().map(|(_, relatorio)| relatorio.stored_chunks).sum::<usize>(),
            },
            "produtos": {
                "trechos": produtos.iter().map(|(_, relatorio)| relatorio.stored_chunks).sum::<usize>(),
            },
        }),
        None,
    )
}

// ───────────────────────────────── Apoio ─────────────────────────────────

fn abrir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
) -> Result<(Store, HarnessConfig), String> {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao =
        config::carregar(arquivo.as_deref(), ambiente).map_err(|erro| erro.to_string())?;
    let store =
        Store::open(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))?;
    Ok((store, configuracao))
}

/// Grava o arquivo inteiro num temporário ao lado e renomeia no fim.
fn escrever_atomico(destino: &Path, bytes: &[u8]) -> Result<(), String> {
    let pasta = destino
        .parent()
        .filter(|pasta| !pasta.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let nome = destino
        .file_name()
        .map(|nome| nome.to_string_lossy().into_owned())
        .unwrap_or_else(|| "backup".to_owned());
    let temporario = pasta.join(format!(".{nome}.{}.tmp", std::process::id()));
    fs::write(&temporario, bytes)
        .map_err(|erro| format!("não consegui escrever {}: {erro}", temporario.display()))?;
    fs::rename(&temporario, destino).map_err(|erro| {
        let _ = fs::remove_file(&temporario);
        format!(
            "não consegui renomear o arquivo para {}: {erro}",
            destino.display()
        )
    })
}

#[cfg(test)]
mod testes {
    use super::*;
    use poph_store::backup::BACKUP_FORMAT;
    use poph_store::backup_document::{descodificar, encode};

    #[test]
    fn base64_vai_e_volta() {
        let casos: &[&[u8]] = &[
            b"",
            b"f",
            b"fo",
            b"foo",
            "conteudo de um arquivo maior, com acentos: ação".as_bytes(),
        ];
        for caso in casos {
            let texto = encode(caso);
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

    #[test]
    fn documento_de_outro_formato_e_recusado() {
        let documento = json!({"formato": "outra-coisa", "versao_formato": 1});
        assert!(retrato_do_documento(&documento).is_err());
    }

    #[test]
    fn versao_de_formato_desconhecida_e_recusada() {
        let documento = json!({"formato": BACKUP_FORMAT, "versao_formato": 99, "org": {}});
        let erro = retrato_do_documento(&documento).expect_err("devia recusar");
        assert!(erro.contains("versão de formato"), "{erro}");
    }

    #[test]
    fn blob_com_conteudo_adulterado_e_recusado() {
        let documento = json!({
            "blobs": [{
                "sha256": "0".repeat(64),
                "conteudo_base64": encode(b"conteudo"),
            }],
        });
        let erro = blobs_do_documento(&documento).expect_err("devia recusar");
        assert!(erro.contains("não confere"), "{erro}");
    }

    #[test]
    fn segredo_legivel_vira_nomes_e_ilegivel_vira_nulo() {
        let chave = ChaveDeCifra::derivar("chave-do-teste").expect("deriva");
        let mut mapa = std::collections::BTreeMap::new();
        mapa.insert("token".to_owned(), "valor-secreto".to_owned());
        let blob = chave.cifrar(&mapa).expect("cifra");
        let mut cifrados = BTreeMap::new();
        cifrados.insert("integ-1".to_owned(), blob.clone());
        cifrados.insert("integ-2".to_owned(), vec![1, 2, 3]);
        let mut tabelas: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        tabelas.insert(
            "integrations".to_owned(),
            vec![
                json!({"id": "integ-1", "tem_segredo": true}),
                json!({"id": "integ-2", "tem_segredo": true}),
            ],
        );
        marcar_segredos(&mut tabelas, &cifrados, Some(&chave));
        let linhas = &tabelas["integrations"];
        assert_eq!(linhas[0]["segredos_campos"], json!(["token"]));
        assert!(linhas[1]["segredos_campos"].is_null());
        // O valor do segredo não aparece em nenhum lugar do que vai para o arquivo.
        assert!(
            !serde_json::to_string(&tabelas)
                .unwrap()
                .contains("valor-secreto")
        );
    }
}
