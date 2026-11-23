//! Exportar e importar uma organização pela API administrativa (HR-48e-1).
//!
//! Duas rotas, no mesmo formato da CLI (`poph backup export`/`import`, HR-53):
//!
//! - `GET /admin/v1/orgs/{org_id}/backup?com_conversas=true|false` devolve o arquivo do retrato da
//!   org (JSON, `Content-Disposition: attachment`). Sem o parâmetro, as conversas **não** entram —
//!   é o mesmo padrão do CLI.
//! - `POST /admin/v1/orgs/{org_id}/backup/import` recebe esse arquivo no corpo cru
//!   (`application/json`, teto de [`LIMITE_IMPORTACAO_BYTES`]) e reconstrói o estado na org do
//!   caminho, devolvendo o relatório do que entrou.
//!
//! As regras são as da importação que o `poph-store` já executa (e que a CLI usa):
//! conflito de chave natural é recusado **antes** de gravar (`409`, com a lista, via
//! [`poph_store::Store::conferir_conflitos_importacao`]); arquivo corrompido ou de versão
//! desconhecida é recusado (`400`, nada gravado); importar não duplica nem apaga o que já existe;
//! segredo de integração nunca viaja (só a marca e os nomes dos campos); os ids são preservados.
//!
//! **Sobrescrita é recusada por padrão (HR-48e-3).** Reimportar um arquivo antigo na mesma org
//! reverteria o que mudou depois dele, então a importação que atualizaria linha existente responde
//! `409 importacao_sobrescreve` — com tabelas, total e até cinco exemplos, e **nada gravado** — a
//! menos que venha `?sobrescrever=true` (decisão do mantenedor, 21/11/2026). Destino limpo entra sem
//! o parâmetro. Conflito de chave natural (mesmo slug com outro id) continua recusado mesmo com o
//! parâmetro: não há "merge".
//!
//! O que este módulo acrescenta à CLI é o transporte: limite de corpo, org de destino vindo do
//! caminho, cabeçalhos de download, `403`/`401` das outras rotas e o relatório em JSON. O formato
//! do arquivo é o de [`poph_store::backup_documento`], o mesmo nos dois lados.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use poph_store::ErroStore;
use poph_store::backup::{
    AtualizacoesImportacao, DestinoImportacao, RelatorioImportacao, RetratoOrg,
};
use poph_store::backup_documento::{self, DocumentoBackup};
use poph_types::{DomainError, OrgId};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::auth::Identidade;
use crate::erro::{ApiError, id_da_url};
use crate::estado::Estado;

use super::auditoria;

/// Teto do arquivo aceito na importação (64 MB).
///
/// O arquivo leva os blobs da biblioteca e dos anexos em base64 (~33% maior que os bytes). O teto
/// existe para o servidor não carregar um corpo sem limite na memória; acima dele a resposta é
/// `413` **antes** de ler o corpo inteiro (quando o `Content-Length` vem no pedido).
pub const LIMITE_IMPORTACAO_BYTES: u64 = 64 * 1024 * 1024;

/// Teto do corpo em bytes, no tipo que o `DefaultBodyLimit` usa.
pub const LIMITE_IMPORTACAO: usize = LIMITE_IMPORTACAO_BYTES as usize;

#[derive(Deserialize)]
pub struct CaminhoOrg {
    org_id: String,
}

/// Parâmetros da exportação. `com_conversas` é lido como texto para o erro sair no formato da API
/// (o `Query` do axum responderia texto puro num valor inválido).
#[derive(Debug, Default, Deserialize)]
pub struct ParametrosExportacao {
    #[serde(default)]
    pub com_conversas: Option<String>,
}

/// Parâmetros da importação. `sobrescrever` é lido como texto pelo mesmo motivo de
/// `com_conversas`: valor que não é booleano vira `400 validation_error`, não erro do axum.
#[derive(Debug, Default, Deserialize)]
pub struct ParametrosImportacao {
    #[serde(default)]
    pub sobrescrever: Option<String>,
}

impl ParametrosImportacao {
    /// `true` só com `sobrescrever=true`; ausente é `false` (recusa sobrescrita).
    fn sobrescrever(&self) -> Result<bool, ApiError> {
        match self.sobrescrever.as_deref() {
            None => Ok(false),
            Some(texto) if texto.eq_ignore_ascii_case("true") => Ok(true),
            Some(texto) if texto.eq_ignore_ascii_case("false") => Ok(false),
            Some(outro) => Err(ApiError::validacao(
                "sobrescrever",
                format!("`{outro}` não é um booleano; use true ou false"),
            )),
        }
    }
}

/// `GET …/backup`: devolve o arquivo do retrato da org.
#[utoipa::path(get, operation_id="backup_exportar", path="/admin/v1/orgs/{org_id}/backup", tag="backup", security(("bearer" = [])), params(("org_id" = String, Path), ("com_conversas" = Option<bool>, Query, description = "Inclui conversas, mensagens, turnos, as duas filas e os carrinhos (padrão: `false`)")), responses((status = 200, description = "Arquivo do backup da org (JSON no formato `poph-backup-org`, versão 1), como anexo. Segredo de integração sai só com a marca e o nome do campo; caminho de blob não sai", body = Value, content_type = "application/json"), (status = 400, description = "`com_conversas` inválido", body = crate::erro::RespostaErro), (status = 401, description = "Sem token", body = crate::erro::RespostaErro), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro), (status = 404, description = "Org não encontrada", body = crate::erro::RespostaErro), (status = 409, description = "Arquivo da biblioteca sem conteúdo no disco", body = crate::erro::RespostaErro)))]
pub async fn exportar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    Query(q): Query<ParametrosExportacao>,
) -> Result<Response, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let com_conversas = match q.com_conversas.as_deref() {
        None => false,
        Some(texto) if texto.eq_ignore_ascii_case("true") => true,
        Some(texto) if texto.eq_ignore_ascii_case("false") => false,
        Some(outro) => {
            return Err(ApiError::validacao(
                "com_conversas",
                format!("`{outro}` não é um booleano; use true ou false"),
            ));
        }
    };
    e.store.orgaos().obter(org)?;

    let estado = e.clone();
    let (bytes, nome) =
        tokio::task::spawn_blocking(move || montar_arquivo(&estado, org, com_conversas))
            .await
            .map_err(|_| ApiError::interno())??;

    let mut resposta = (StatusCode::OK, bytes).into_response();
    let cabecalhos = resposta.headers_mut();
    cabecalhos.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    cabecalhos.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{nome}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    cabecalhos.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    cabecalhos.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(resposta)
}

/// Monta o arquivo (retrato + blobs + documento) e devolve os bytes e o nome sugerido.
///
/// Roda fora do worker do tokio: lê o banco e o disco e serializa os blobs em base64.
fn montar_arquivo(
    e: &Estado,
    org: OrgId,
    com_conversas: bool,
) -> Result<(Vec<u8>, String), ApiError> {
    let retrato = e.store.retrato_da_org(org, com_conversas)?;
    let blobs = ler_blobs(e, &retrato, org)?;
    let chave = e.chave_de_cifra.clone();
    // A chave só serve para ler os **nomes** dos campos de segredo; o valor nunca sai do banco.
    let nomes_do_segredo = move |blob: &[u8]| -> Option<Vec<String>> {
        let mapa = chave.as_ref()?.decifrar(blob).ok()?;
        Some(mapa.keys().cloned().collect())
    };
    let documento = backup_documento::documento(
        &retrato,
        com_conversas,
        e.store.agora(),
        &blobs,
        &nomes_do_segredo,
    );
    let bytes = serde_json::to_vec(&documento).map_err(|_| {
        ApiError::novo(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "não consegui montar o arquivo de backup",
        )
    })?;
    let slug = retrato
        .org
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or("org");
    Ok((bytes, nome_do_arquivo(slug, e.store.agora())))
}

/// Lê do disco o conteúdo de cada blob citado no retrato (biblioteca e anexos).
fn ler_blobs(
    e: &Estado,
    retrato: &RetratoOrg,
    org: OrgId,
) -> Result<BTreeMap<String, Vec<u8>>, ApiError> {
    let mut nomes = BTreeMap::new();
    for tabela in ["file_library", "attachments"] {
        for linha in retrato.tabelas.get(tabela).into_iter().flatten() {
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
    let pasta = &e.arquivos.pasta_blobs;
    let limite = e.arquivos.limite_bytes;
    let mut blobs = BTreeMap::new();
    for (sha, nome) in nomes {
        let bytes = poph_files::blob::ler(pasta, org, &sha, limite).map_err(|erro| {
            // O motivo do `ErroArquivo` é escrito sem caminho e sem conteúdo.
            tracing::warn!(org = %org, sha = %sha, motivo = %erro, "exportação sem o conteúdo de um arquivo");
            ApiError::novo(
                StatusCode::CONFLICT,
                "backup_incompleto",
                format!(
                    "o conteúdo do arquivo `{nome}` da biblioteca não está no disco: a \
                     exportação não sai pela metade; restaure o arquivo e exporte de novo"
                ),
            )
            .com_detalhes(json!({ "name": nome }))
        })?;
        blobs.insert(sha, bytes);
    }
    Ok(blobs)
}

/// Nome do arquivo entregue: `<slug>-backup-<AAAAMMDD-HHMM>.json`.
fn nome_do_arquivo(slug: &str, agora_ms: i64) -> String {
    let slug: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = if slug.trim_matches('-').is_empty() {
        "org".to_owned()
    } else {
        slug
    };
    format!("{slug}-backup-{}.json", carimbo(agora_ms))
}

/// Carimbo `AAAAMMDD-HHMM` em UTC a partir de milissegundos desde a época.
fn carimbo(agora_ms: i64) -> String {
    let segundos = agora_ms.div_euclid(1000);
    let dias = segundos.div_euclid(86_400);
    let no_dia = segundos.rem_euclid(86_400);
    let (ano, mes, dia) = data_civil(dias);
    let hora = no_dia / 3_600;
    let minuto = (no_dia % 3_600) / 60;
    format!("{ano:04}{mes:02}{dia:02}-{hora:02}{minuto:02}")
}

/// Data civil (ano, mês, dia) de um número de dias desde 1970-01-01, em UTC.
///
/// Algoritmo de Howard Hinnant (`civil_from_days`): aritmética inteira, sem dependência de data.
fn data_civil(dias: i64) -> (i64, u32, u32) {
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let dia_da_era = z.rem_euclid(146_097);
    let ano_da_era =
        (dia_da_era - dia_da_era / 1_460 + dia_da_era / 36_524 - dia_da_era / 146_096) / 365;
    let ano = ano_da_era + era * 400;
    let dia_do_ano = dia_da_era - (365 * ano_da_era + ano_da_era / 4 - ano_da_era / 100);
    let mes_ajustado = (5 * dia_do_ano + 2) / 153;
    let dia = dia_do_ano - (153 * mes_ajustado + 2) / 5 + 1;
    let mes = if mes_ajustado < 10 {
        mes_ajustado + 3
    } else {
        mes_ajustado - 9
    };
    let ano = if mes <= 2 { ano + 1 } else { ano };
    (ano, mes as u32, dia as u32)
}

/// `POST …/backup/import`: reconstrói a org do caminho a partir do arquivo do corpo.
#[utoipa::path(post, operation_id="backup_importar", path="/admin/v1/orgs/{org_id}/backup/import", tag="backup", security(("bearer" = [])), params(("org_id" = String, Path), ("sobrescrever" = Option<bool>, Query, description = "Autoriza a importação a atualizar linhas que já existem na org (padrão: `false`, que recusa com `409 importacao_sobrescreve` e nada grava)")), request_body(content = Value, content_type = "application/json", description = "Arquivo gerado por `GET …/backup` (ou `poph backup export`), até 64 MB"), responses((status = 200, description = "Relatório da importação: linhas inseridas/atualizadas por tabela, segredos a digitar de novo e o índice", body = Value), (status = 400, description = "Arquivo corrompido, formato ou versão desconhecidos, ou `sobrescrever` que não é booleano", body = crate::erro::RespostaErro), (status = 401, description = "Sem token", body = crate::erro::RespostaErro), (status = 403, description = "Org de outro token", body = crate::erro::RespostaErro), (status = 404, description = "Org de destino não encontrada", body = crate::erro::RespostaErro), (status = 409, description = "`importacao_sobrescreve` (atualizaria linhas existentes sem `sobrescrever=true`), conflito de chave natural ou arquivo da biblioteca sem conteúdo no disco; nada foi gravado", body = crate::erro::RespostaErro), (status = 413, description = "Arquivo acima do limite", body = crate::erro::RespostaErro)))]
pub async fn importar(
    State(e): State<Estado>,
    ident: Identidade,
    Path(c): Path<CaminhoOrg>,
    Query(q): Query<ParametrosImportacao>,
    cabecalhos: HeaderMap,
    corpo: Result<Bytes, BytesRejection>,
) -> Result<Json<Value>, ApiError> {
    let org = id_da_url::<OrgId>(&c.org_id)?;
    ident.exigir_org(org)?;
    let sobrescrever = q.sobrescrever()?;

    // Teto antes de ler o corpo: o `Content-Length` já diz que não cabe.
    if let Some(tamanho) = cabecalhos
        .get(header::CONTENT_LENGTH)
        .and_then(|valor| valor.to_str().ok())
        .and_then(|texto| texto.parse::<u64>().ok())
        && tamanho > LIMITE_IMPORTACAO_BYTES
    {
        return Err(arquivo_grande());
    }
    let bytes = match corpo {
        Ok(bytes) => bytes,
        Err(rejeicao) => {
            let resposta = rejeicao.into_response();
            return Err(if resposta.status() == StatusCode::PAYLOAD_TOO_LARGE {
                arquivo_grande()
            } else {
                ApiError::invalido("não consegui ler o corpo da requisição")
            });
        }
    };
    if bytes.len() as u64 > LIMITE_IMPORTACAO_BYTES {
        return Err(arquivo_grande());
    }

    // A org de destino tem de existir (o caminho é o destino, não a origem do arquivo).
    e.store.orgaos().obter(org)?;

    let documento = tokio::task::spawn_blocking(move || backup_documento::ler_documento(&bytes))
        .await
        .map_err(|_| ApiError::interno())?
        .map_err(arquivo_recusado)?;

    let DocumentoBackup {
        retrato,
        blobs,
        com_conversas,
    } = documento;
    // Conflito e sobrescrita são conferidos **antes** de gravar, e na mesma thread de trabalho (o
    // retrato não é clonado só para conferir). Conflito de chave natural vem primeiro: ele recusa a
    // importação mesmo com `sobrescrever=true`.
    let estado = e.clone();
    let resultado = tokio::task::spawn_blocking(move || {
        let conflitos = estado
            .store
            .conferir_conflitos_importacao(&retrato, DestinoImportacao::Existente(org))
            .map_err(erro_da_conferencia)?;
        if !conflitos.is_empty() {
            return Ok(Resultado::Conflito(conflitos));
        }
        if !sobrescrever {
            let atualizacoes = estado
                .store
                .conferir_atualizacoes_importacao(&retrato, DestinoImportacao::Existente(org))
                .map_err(erro_da_conferencia)?;
            if !atualizacoes.vazia() {
                return Ok(Resultado::Sobrescreve(atualizacoes));
            }
        }
        gravar_no_destino(&estado, org, retrato, &blobs, sobrescrever).map(|resultado| {
            match resultado {
                Ok(relatorio) => Resultado::Importado(relatorio),
                Err(atualizacoes) => Resultado::Sobrescreve(atualizacoes),
            }
        })
    })
    .await
    .map_err(|_| ApiError::interno())??;
    let relatorio = match resultado {
        Resultado::Conflito(conflitos) => return Err(conflito_da_importacao(&conflitos)),
        Resultado::Sobrescreve(atualizacoes) => {
            return Err(sobrescreve_a_importacao(&atualizacoes));
        }
        Resultado::Importado(relatorio) => relatorio,
    };

    auditoria::registrar(
        &e,
        &ident,
        Some(org),
        "backup_import",
        "org",
        org.to_string(),
    )?;

    let (indice, aviso) = reindexar(&e, org).await;
    Ok(Json(json!({
        "org": {"id": relatorio.org.to_string(), "criada": relatorio.org_criada},
        "com_conversas": com_conversas,
        "tabelas": tabelas_do_relatorio(&relatorio),
        "total": relatorio.total(),
        "segredos_a_redigitar": relatorio.segredos_a_redigitar,
        "indice": indice,
        "aviso": aviso,
    })))
}

/// O que a thread de trabalho devolveu: conflito ou sobrescrita (nada gravado) ou o relatório.
enum Resultado {
    Conflito(Vec<String>),
    Sobrescreve(AtualizacoesImportacao),
    Importado(RelatorioImportacao),
}

/// `413` — arquivo acima do teto.
fn arquivo_grande() -> ApiError {
    ApiError::novo(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        format!(
            "o arquivo passa do limite de {} MB por pedido; exporte sem as conversas (ou com \
             `com_conversas=false`) ou importe numa instalação com o arquivo menor",
            LIMITE_IMPORTACAO_BYTES / (1024 * 1024)
        ),
    )
}

/// Traduz o erro do formato do arquivo em `400` com mensagem clara.
fn arquivo_recusado(erro: backup_documento::ErroDocumento) -> ApiError {
    match &erro {
        backup_documento::ErroDocumento::Json(_) => ApiError::novo(
            StatusCode::BAD_REQUEST,
            "invalid_backup",
            format!("o corpo não é um backup do POP Harness ({erro})"),
        ),
        backup_documento::ErroDocumento::Formato(_) => {
            ApiError::novo(StatusCode::BAD_REQUEST, "invalid_backup", erro.to_string())
        }
        backup_documento::ErroDocumento::Versao { lida, entendida } => {
            ApiError::novo(StatusCode::BAD_REQUEST, "invalid_backup", erro.to_string())
                .com_detalhes(json!({ "version": lida, "supported_version": entendida }))
        }
    }
}

/// `409` — a importação bateria em recurso que já existe no destino ou em id de outra org.
fn conflito_da_importacao(conflitos: &[String]) -> ApiError {
    let mostrados: Vec<Value> = conflitos
        .iter()
        .take(5)
        .cloned()
        .map(Value::String)
        .collect();
    let resto = conflitos.len().saturating_sub(mostrados.len());
    let mut motivo = format!(
        "a importação foi recusada antes de gravar: {}",
        conflitos
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
    if resto > 0 {
        motivo.push_str(&format!("; e mais {resto}"));
    }
    motivo.push_str(
        ". Nada foi gravado: renomeie um dos dois lados ou importe numa instância sem esses recursos",
    );
    ApiError::novo(StatusCode::CONFLICT, "conflict", motivo).com_detalhes(json!({
        "conflicts": mostrados,
        "conflicts_more": resto,
    }))
}

/// `409 importacao_sobrescreve` — sem `sobrescrever=true`, a importação não regrava linha existente.
///
/// Mensagem curta (o detalhe vai nos campos); `details` traz a tabela → linhas, o total e até cinco
/// exemplos legíveis. Nada de segredo, caminho de blob ou conteúdo de arquivo aqui: só nome de
/// tabela e slug/nome/id.
fn sobrescreve_a_importacao(atualizacoes: &AtualizacoesImportacao) -> ApiError {
    let exemplos: Vec<Value> = atualizacoes
        .exemplos
        .iter()
        .cloned()
        .map(Value::String)
        .collect();
    let tabelas: Map<String, Value> = atualizacoes
        .tabelas
        .iter()
        .map(|(tabela, linhas)| (tabela.clone(), Value::from(*linhas)))
        .collect();
    ApiError::novo(
        StatusCode::CONFLICT,
        "importacao_sobrescreve",
        format!(
            "a importação atualizaria {} linha(s) que já existem nesta org; nada foi gravado. Repita com `sobrescrever=true` para autorizar",
            atualizacoes.total
        ),
    )
    .com_detalhes(json!({
        "tabelas": tabelas,
        "linhas": atualizacoes.total,
        "exemplos": exemplos,
    }))
}

/// Grava os blobs no destino e, depois, importa o retrato (uma transação).
fn gravar_no_destino(
    e: &Estado,
    org: OrgId,
    retrato: RetratoOrg,
    blobs: &BTreeMap<String, Vec<u8>>,
    sobrescrever: bool,
) -> Result<Result<RelatorioImportacao, AtualizacoesImportacao>, ApiError> {
    let pasta = &e.arquivos.pasta_blobs;
    for (sha, conteudo) in blobs {
        match poph_files::blob::gravar(pasta, org, conteudo) {
            Ok(gravado) if gravado.sha256 == *sha => {}
            Ok(_) => {
                return Err(ApiError::novo(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "o conteúdo de um arquivo do backup foi gravado com outro sha256; nada foi importado",
                ));
            }
            Err(erro) => {
                tracing::warn!(org = %org, motivo = %erro, "falha ao gravar o blob da importação");
                return Err(ApiError::novo(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "não consegui gravar o conteúdo de um arquivo do backup",
                ));
            }
        }
    }
    e.store
        .importar_retrato_protegido(retrato, DestinoImportacao::Existente(org), sobrescrever)
        .map_err(erro_da_importacao)
}

/// Erro do banco ao conferir os conflitos (é leitura: só falha de banco).
fn erro_da_conferencia(erro: ErroStore) -> ApiError {
    match erro {
        ErroStore::Dominio(DomainError::NaoEncontrado { entidade, id }) => {
            ApiError::nao_encontrado(entidade, &id)
        }
        outro => ApiError::from(outro),
    }
}

/// Erro do banco na importação: o que é do arquivo sai como `400`; o resto, como nas outras rotas.
fn erro_da_importacao(erro: ErroStore) -> ApiError {
    match erro {
        ErroStore::Dominio(DomainError::Validacao {
            campo: "backup",
            motivo,
        }) => ApiError::novo(StatusCode::BAD_REQUEST, "invalid_backup", motivo)
            .com_detalhes(json!({ "field": "backup" })),
        outro => ApiError::from(outro),
    }
}

/// Tabela → linhas inseridas/atualizadas, no mesmo formato da CLI.
fn tabelas_do_relatorio(relatorio: &RelatorioImportacao) -> Map<String, Value> {
    relatorio
        .tabelas
        .iter()
        .map(|(nome, gravado)| {
            (
                nome.clone(),
                json!({"inseridas": gravado.inseridas, "atualizadas": gravado.atualizadas}),
            )
        })
        .collect()
}

/// Reindexa a org importada: base de conhecimento e produtos.
///
/// Com o modelo instalado, a org fica buscável sem passo manual, e o relatório diz quantos trechos
/// entraram. Sem o modelo, o material importado continua gravado, o índice **não** é dado como
/// pronto e o relatório traz o aviso com o comando para reindexar depois.
async fn reindexar(e: &Estado, org: OrgId) -> (Value, Option<String>) {
    use poph_rag::{ConfigIndice, ConfigModelo, Embedder, EmbedderOrt, Indice};

    static SINCRONIZACAO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guarda = SINCRONIZACAO.lock().await;

    let pendente = || {
        json!({
            "base_conhecimento": {"agentes": 0, "trechos": 0},
            "produtos": {"trechos": 0},
        })
    };
    let config = ConfigModelo {
        diretorio: e.config.modelos(),
        ..ConfigModelo::default()
    };
    let modelo = match tokio::task::spawn_blocking(move || EmbedderOrt::abrir(&config, None)).await
    {
        Ok(Ok(modelo)) => modelo,
        Ok(Err(erro)) => {
            return (
                pendente(),
                Some(format!(
                    "modelo de embedding indisponível ({erro}). O material foi importado, mas o \
                     índice não ficou pronto: rode `poph kb reindex` e `poph product reindex` \
                     quando o modelo estiver disponível"
                )),
            );
        }
        Err(_) => {
            return (
                pendente(),
                Some(
                    "não consegui abrir o modelo de embedding. O material foi importado, mas o \
                     índice não ficou pronto: rode `poph kb reindex` e `poph product reindex`"
                        .to_owned(),
                ),
            );
        }
    };
    let embedder: Arc<dyn Embedder> = Arc::new(modelo);
    let mut avisos: Vec<String> = Vec::new();

    let mut agentes = 0usize;
    let mut trechos = 0usize;
    match Indice::abrir(
        e.store.caminho_rag(),
        Arc::clone(&embedder),
        ConfigIndice::default(),
    ) {
        Ok((indice, _)) => {
            let base = poph_agent::BaseConhecimento::nova(Arc::clone(&e.store), Arc::new(indice));
            match base.sincronizar_org(org).await {
                Ok(relatorios) => {
                    agentes = relatorios.len();
                    trechos = relatorios
                        .iter()
                        .map(|(_, relatorio)| relatorio.trechos_gravados)
                        .sum();
                }
                Err(erro) => {
                    tracing::warn!(org = %org, motivo = %erro, "falha ao reindexar a base da org importada");
                    avisos.push(
                        "não consegui reindexar a base de conhecimento; rode `poph kb reindex`"
                            .to_owned(),
                    );
                }
            }
        }
        Err(erro) => {
            tracing::warn!(org = %org, motivo = %erro, "falha ao abrir o índice da base");
            avisos.push("não consegui abrir o índice da base; rode `poph kb reindex`".to_owned());
        }
    }

    let mut trechos_produtos = 0usize;
    match Indice::abrir(
        e.store.caminho_rag(),
        Arc::clone(&embedder),
        ConfigIndice::default(),
    ) {
        Ok((indice, _)) => {
            let catalogo =
                poph_agent::CatalogoProdutos::novo(Arc::clone(&e.store), Arc::new(indice));
            match catalogo.sincronizar_org(org).await {
                Ok(relatorios) => {
                    trechos_produtos = relatorios
                        .iter()
                        .map(|(_, relatorio)| relatorio.trechos_gravados)
                        .sum();
                }
                Err(erro) => {
                    tracing::warn!(org = %org, motivo = %erro, "falha ao reindexar os produtos da org importada");
                    avisos.push(
                        "não consegui reindexar os produtos; rode `poph product reindex`"
                            .to_owned(),
                    );
                }
            }
        }
        Err(erro) => {
            tracing::warn!(org = %org, motivo = %erro, "falha ao abrir o índice de produtos");
            avisos.push(
                "não consegui abrir o índice de produtos; rode `poph product reindex`".to_owned(),
            );
        }
    }

    (
        json!({
            "base_conhecimento": {"agentes": agentes, "trechos": trechos},
            "produtos": {"trechos": trechos_produtos},
        }),
        if avisos.is_empty() {
            None
        } else {
            Some(avisos.join("; "))
        },
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn carimbo_e_data_civil_em_utc() {
        assert_eq!(carimbo(0), "19700101-0000");
        // 2023-11-14T22:13:20Z
        assert_eq!(carimbo(1_700_000_000_000), "20231114-2213");
        // 2024-02-29T12:34:56Z (ano bissexto)
        assert_eq!(carimbo(1_709_210_096_000), "20240229-1234");
        assert_eq!(data_civil(0), (1970, 1, 1));
    }

    #[test]
    fn nome_do_arquivo_usa_slug_e_carimbo() {
        assert_eq!(
            nome_do_arquivo("org-origem", 1_700_000_000_000),
            "org-origem-backup-20231114-2213.json"
        );
        // Caractere fora do alfabeto seguro vira `-`; slug vazio vira `org`.
        assert_eq!(
            nome_do_arquivo("org origem!", 0),
            "org-origem--backup-19700101-0000.json"
        );
        assert_eq!(nome_do_arquivo("", 0), "org-backup-19700101-0000.json");
    }
}
