//! Apoio comum dos comandos `org`, `token`, `category` e `department` (HR-08).
//!
//! Regras que valem para os quatro:
//! - `--offline` é obrigatório: o modo com servidor ainda não existe, então estes comandos só
//!   operam direto no banco;
//! - `--json` tem formato estável, com `versao_relatorio` = 1, e nunca traz segredo (o valor do
//!   token só sai na criação);
//! - erro de negócio sai com código 1 e mensagem em português no erro padrão; código 2 é uso
//!   incorreto da linha de comando (o clap decide).

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use poph_store::{Fluxo, Page, Status, Store, StoreError};
use poph_types::{AgentId, DomainError, FlowId, HarnessConfig, OrgId};
use serde_json::Value;

use crate::config;

/// Recusa o comando quando `--offline` não foi passado.
pub fn require_offline(offline: bool) -> Result<(), String> {
    if offline {
        Ok(())
    } else {
        Err(
            "use `--offline`: por enquanto estes comandos só funcionam direto no banco \
             (o modo com servidor ainda não existe)"
                .to_owned(),
        )
    }
}

/// Abre a configuração e o banco (migrando se preciso).
pub fn abrir_banco(flag: Option<PathBuf>, ambiente: &config::Environment) -> Result<Store, String> {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolve_file(flag.as_deref(), ambiente, &pasta);
    let configuracao = config::load(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    Store::open(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))
}

/// Abre a configuração e o banco e devolve as duas coisas. Os comandos de fluxo precisam da
/// configuração `[fluxos]` (tetos e tamanho do cache) além do banco.
pub fn abrir_com_configuracao(
    flag: Option<PathBuf>,
    ambiente: &config::Environment,
) -> Result<(Store, HarnessConfig), String> {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolve_file(flag.as_deref(), ambiente, &pasta);
    let configuracao = config::load(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    let store =
        Store::open(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))?;
    Ok((store, configuracao))
}

/// Acha o agente pelo id (UUID v7) ou pelo slug, dentro da org.
pub fn resolver_agente(
    store: &Store,
    org: OrgId,
    texto: &str,
) -> Result<poph_store::Agente, String> {
    let encontrado = if uuid::Uuid::parse_str(texto).is_ok() {
        let id: AgentId = parse_id(texto)?;
        store.agents().get(org, id)
    } else {
        store.agents().by_slug(org, texto)
    };
    encontrado.map_err(|erro| {
        if erro.is_not_found() {
            format!("não existe agente `{texto}` nesta org")
        } else {
            describe(&erro)
        }
    })
}

/// Acha o fluxo pelo id (UUID v7) ou pelo slug, dentro da org.
pub fn resolver_fluxo(store: &Store, org: OrgId, texto: &str) -> Result<Fluxo, String> {
    let achado = if uuid::Uuid::parse_str(texto).is_ok() {
        let id: FlowId = parse_id(texto)?;
        store.flows().get(org, id).map(Some)
    } else {
        store.flows().by_slug(org, texto)
    };
    match achado {
        Ok(Some(fluxo)) => Ok(fluxo),
        Ok(None) => Err(format!("não existe fluxo `{texto}` nesta org")),
        Err(erro) if erro.is_not_found() => Err(format!("não existe fluxo `{texto}` nesta org")),
        Err(erro) => Err(describe(&erro)),
    }
}

/// Página pedida na linha de comando: `--limit` de 1 a 200 e o cursor (id da página anterior).
pub fn pagina_de(limite: u32, cursor: Option<&str>) -> Result<Page, String> {
    let mut pagina = validar_limite(limite)?;
    pagina.after =
        match cursor {
            None | Some("") => None,
            Some(texto) => Some(uuid::Uuid::parse_str(texto).map_err(|_| {
                "cursor inválido: use o `next_cursor` da página anterior".to_owned()
            })?),
        };
    Ok(pagina)
}

/// Para os comandos que trabalham dentro de uma org: exige `--offline` e `--org`, e resolve a org.
pub fn abrir_com_org(
    flag: Option<PathBuf>,
    ambiente: &config::Environment,
    offline: bool,
    org: Option<&str>,
) -> Result<(Store, OrgId), String> {
    require_offline(offline)?;
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let store = abrir_banco(flag, ambiente)?;
    let id = resolve_org(&store, texto)?;
    Ok((store, id))
}

/// Acha a org pelo id (UUID v7) ou pelo slug. Um texto com cara de UUID que não é v7 é recusado
/// com o motivo, em vez de ser tratado como slug.
pub fn resolve_org(store: &Store, texto: &str) -> Result<OrgId, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: OrgId = parse_id(texto)?;
        return match store.orgs().get(id) {
            Ok(org) => Ok(org.id),
            Err(erro) if erro.is_not_found() => Err(format!("não existe org com o id `{texto}`")),
            Err(erro) => Err(describe(&erro)),
        };
    }
    match store.orgs().by_slug(texto) {
        Ok(org) => Ok(org.id),
        Err(erro) if erro.is_not_found() => Err(format!("não existe org com o slug `{texto}`")),
        Err(erro) => Err(describe(&erro)),
    }
}

/// Lê um id tipado (UUID v7) vindo da linha de comando.
pub fn parse_id<T>(texto: &str) -> Result<T, String>
where
    T: FromStr<Err = DomainError>,
{
    texto.parse::<T>().map_err(|erro| erro.to_string())
}

/// Traduz erro do banco para uma frase que o operador entende.
pub fn describe(erro: &StoreError) -> String {
    match erro {
        StoreError::Domain(DomainError::VersionConflict {
            expected: esperada,
            current: atual,
            ..
        }) => format!(
            "a versão não confere: você informou a versão {esperada}, mas o registro está na \
             versão {atual}. Leia de novo com `show` e repita com a versão atual"
        ),
        StoreError::Domain(DomainError::NotFound {
            entity: entidade,
            id,
        }) => {
            format!("{entidade} `{id}` não existe nesta org")
        }
        StoreError::Domain(DomainError::Validation { field, reason }) => {
            format!("{field}: {reason}")
        }
        StoreError::Domain(DomainError::Conflict { reason, .. }) => reason.clone(),
        StoreError::Domain(outro) => outro.to_string(),
        outro => format!("não consegui gravar no banco: {outro}"),
    }
}

/// Lê a situação digitada (`active`, `inactive` ou `archived`, como no banco).
pub fn status_de_texto(texto: &str) -> Result<Status, String> {
    match texto {
        "active" => Ok(Status::Active),
        "inactive" => Ok(Status::Inactive),
        "archived" => Ok(Status::Archived),
        outro => Err(format!(
            "--status `{outro}` inválido: use active, inactive ou archived"
        )),
    }
}

/// Situação em português para a saída de texto.
pub fn status_em_portugues(status: Status) -> &'static str {
    match status {
        Status::Active => "ativo",
        Status::Inactive => "inativo",
        Status::Archived => "arquivado",
    }
}

/// Valida um texto obrigatório: sem espaços nas pontas, não vazio, até `maximo` caracteres.
pub fn validar_texto(campo: &str, valor: &str, maximo: usize) -> Result<String, String> {
    let limpo = valor.trim();
    if limpo.is_empty() {
        return Err(format!("{campo} não pode ficar vazio"));
    }
    if limpo.chars().count() > maximo {
        return Err(format!("{campo} passa de {maximo} caracteres"));
    }
    Ok(limpo.to_owned())
}

/// `--limit` de listagem: 1 a 200.
pub fn validar_limite(limite: u32) -> Result<Page, String> {
    if (1..=200).contains(&limite) {
        Ok(Page::with_limit(limite))
    } else {
        Err("--limit precisa estar entre 1 e 200".to_owned())
    }
}

/// Conta todos os registros de uma listagem paginada (de 200 em 200, pelo cursor de id).
pub fn contar_todos<T>(
    mut listar: impl FnMut(Page) -> Result<Vec<T>, StoreError>,
    id_de: impl Fn(&T) -> uuid::Uuid,
) -> Result<usize, String> {
    let mut total = 0;
    let mut apos = None;
    loop {
        let lote = listar(Page {
            after: apos,
            limit: 200,
        })
        .map_err(|erro| describe(&erro))?;
        total += lote.len();
        if lote.len() < 200 {
            return Ok(total);
        }
        apos = lote.last().map(&id_de);
    }
}

/// Converte o texto de `--config-json` (precisa ser um objeto JSON).
pub fn validar_config_json(texto: &str) -> Result<String, String> {
    match serde_json::from_str::<Value>(texto) {
        Ok(valor) if valor.is_object() => Ok(texto.to_owned()),
        _ => Err("--config-json precisa ser um objeto JSON, por exemplo {}".to_owned()),
    }
}

/// Configuração guardada como texto, devolvida como objeto JSON (vazia se não der para ler).
pub fn json_de_texto(texto: &str) -> Value {
    serde_json::from_str::<Value>(texto).unwrap_or_else(|_| serde_json::json!({}))
}

/// Imprime o JSON bonito no stdout.
pub fn print_json(valor: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(valor).unwrap_or_else(|_| valor.to_string())
    );
}

/// Mensagem no erro padrão e código de saída 1.
pub fn failure(mensagem: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", mensagem.as_ref());
    ExitCode::from(1)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn status_aceita_so_os_textos_do_banco() {
        assert_eq!(status_de_texto("active").unwrap(), Status::Active);
        assert_eq!(status_de_texto("inactive").unwrap(), Status::Inactive);
        assert_eq!(status_de_texto("archived").unwrap(), Status::Archived);
        let erro = status_de_texto("ativo").unwrap_err();
        assert!(erro.contains("active, inactive ou archived"), "{erro}");
    }

    #[test]
    fn texto_obrigatorio_recusa_vazio_e_longo() {
        assert_eq!(validar_texto("nome", "  Vendas  ", 10).unwrap(), "Vendas");
        assert!(
            validar_texto("nome", "   ", 10)
                .unwrap_err()
                .contains("não pode ficar vazio")
        );
        let longo = "x".repeat(11);
        assert!(
            validar_texto("nome", &longo, 10)
                .unwrap_err()
                .contains("passa de 10")
        );
    }

    #[test]
    fn limite_fica_entre_1_e_200() {
        assert!(validar_limite(0).is_err());
        assert!(validar_limite(201).is_err());
        assert_eq!(validar_limite(200).unwrap().limit, 200);
    }

    #[test]
    fn config_json_precisa_ser_objeto() {
        assert!(validar_config_json("{}").is_ok());
        assert!(validar_config_json("[1]").is_err());
        assert!(validar_config_json("não é json").is_err());
    }

    #[test]
    fn exigir_offline_recusa_sem_a_flag() {
        assert!(require_offline(true).is_ok());
        assert!(require_offline(false).unwrap_err().contains("--offline"));
    }

    #[test]
    fn conflito_de_versao_diz_as_duas_versoes() {
        let erro = StoreError::Domain(DomainError::VersionConflict {
            entity: "categoria de produto",
            id: "x".to_owned(),
            expected: 2,
            current: 5,
        });
        let texto = describe(&erro);
        assert!(texto.contains("informou a versão 2"), "{texto}");
        assert!(texto.contains("versão 5"), "{texto}");
    }
}
