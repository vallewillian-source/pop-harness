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

use poph_store::{ErroStore, Fluxo, Pagina, Status, Store};
use poph_types::{AgentId, DomainError, FluxoId, HarnessConfig, OrgId};
use serde_json::Value;

use crate::config;

/// Recusa o comando quando `--offline` não foi passado.
pub fn exigir_offline(offline: bool) -> Result<(), String> {
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
pub fn abrir_banco(flag: Option<PathBuf>, ambiente: &config::Ambiente) -> Result<Store, String> {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao = config::carregar(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    Store::abrir(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))
}

/// Abre a configuração e o banco e devolve as duas coisas. Os comandos de fluxo precisam da
/// configuração `[fluxos]` (tetos e tamanho do cache) além do banco.
pub fn abrir_com_configuracao(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
) -> Result<(Store, HarnessConfig), String> {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao = config::carregar(arquivo.as_deref(), ambiente)
        .map_err(|erro| format!("configuração inválida: {erro}"))?;
    let store = Store::abrir(&configuracao)
        .map_err(|erro| format!("não consegui abrir o banco: {erro}"))?;
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
        store.agentes().obter(org, id)
    } else {
        store.agentes().por_slug(org, texto)
    };
    encontrado.map_err(|erro| {
        if erro.eh_nao_encontrado() {
            format!("não existe agente `{texto}` nesta org")
        } else {
            descrever(&erro)
        }
    })
}

/// Acha o fluxo pelo id (UUID v7) ou pelo slug, dentro da org.
pub fn resolver_fluxo(store: &Store, org: OrgId, texto: &str) -> Result<Fluxo, String> {
    let achado = if uuid::Uuid::parse_str(texto).is_ok() {
        let id: FluxoId = parse_id(texto)?;
        store.fluxos().obter(org, id).map(Some)
    } else {
        store.fluxos().por_slug(org, texto)
    };
    match achado {
        Ok(Some(fluxo)) => Ok(fluxo),
        Ok(None) => Err(format!("não existe fluxo `{texto}` nesta org")),
        Err(erro) if erro.eh_nao_encontrado() => {
            Err(format!("não existe fluxo `{texto}` nesta org"))
        }
        Err(erro) => Err(descrever(&erro)),
    }
}

/// Página pedida na linha de comando: `--limit` de 1 a 200 e o cursor (id da página anterior).
pub fn pagina_de(limite: u32, cursor: Option<&str>) -> Result<Pagina, String> {
    let mut pagina = validar_limite(limite)?;
    pagina.apos =
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
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
) -> Result<(Store, OrgId), String> {
    exigir_offline(offline)?;
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let store = abrir_banco(flag, ambiente)?;
    let id = resolver_org(&store, texto)?;
    Ok((store, id))
}

/// Acha a org pelo id (UUID v7) ou pelo slug. Um texto com cara de UUID que não é v7 é recusado
/// com o motivo, em vez de ser tratado como slug.
pub fn resolver_org(store: &Store, texto: &str) -> Result<OrgId, String> {
    if uuid::Uuid::parse_str(texto).is_ok() {
        let id: OrgId = parse_id(texto)?;
        return match store.orgaos().obter(id) {
            Ok(org) => Ok(org.id),
            Err(erro) if erro.eh_nao_encontrado() => {
                Err(format!("não existe org com o id `{texto}`"))
            }
            Err(erro) => Err(descrever(&erro)),
        };
    }
    match store.orgaos().por_slug(texto) {
        Ok(org) => Ok(org.id),
        Err(erro) if erro.eh_nao_encontrado() => {
            Err(format!("não existe org com o slug `{texto}`"))
        }
        Err(erro) => Err(descrever(&erro)),
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
pub fn descrever(erro: &ErroStore) -> String {
    match erro {
        ErroStore::Dominio(DomainError::ConflitoDeVersao {
            esperada, atual, ..
        }) => format!(
            "a versão não confere: você informou a versão {esperada}, mas o registro está na \
             versão {atual}. Leia de novo com `show` e repita com a versão atual"
        ),
        ErroStore::Dominio(DomainError::NaoEncontrado { entidade, id }) => {
            format!("{entidade} `{id}` não existe nesta org")
        }
        ErroStore::Dominio(DomainError::Validacao { campo, motivo }) => {
            format!("{campo}: {motivo}")
        }
        ErroStore::Dominio(DomainError::Conflito { motivo, .. }) => motivo.clone(),
        ErroStore::Dominio(outro) => outro.to_string(),
        outro => format!("não consegui gravar no banco: {outro}"),
    }
}

/// Lê a situação digitada (`active`, `inactive` ou `archived`, como no banco).
pub fn status_de_texto(texto: &str) -> Result<Status, String> {
    match texto {
        "active" => Ok(Status::Ativo),
        "inactive" => Ok(Status::Inativo),
        "archived" => Ok(Status::Arquivado),
        outro => Err(format!(
            "--status `{outro}` inválido: use active, inactive ou archived"
        )),
    }
}

/// Situação em português para a saída de texto.
pub fn status_em_portugues(status: Status) -> &'static str {
    match status {
        Status::Ativo => "ativo",
        Status::Inativo => "inativo",
        Status::Arquivado => "arquivado",
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
pub fn validar_limite(limite: u32) -> Result<Pagina, String> {
    if (1..=200).contains(&limite) {
        Ok(Pagina::com_limite(limite))
    } else {
        Err("--limit precisa estar entre 1 e 200".to_owned())
    }
}

/// Conta todos os registros de uma listagem paginada (de 200 em 200, pelo cursor de id).
pub fn contar_todos<T>(
    mut listar: impl FnMut(Pagina) -> Result<Vec<T>, ErroStore>,
    id_de: impl Fn(&T) -> uuid::Uuid,
) -> Result<usize, String> {
    let mut total = 0;
    let mut apos = None;
    loop {
        let lote = listar(Pagina { apos, limite: 200 }).map_err(|erro| descrever(&erro))?;
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
pub fn imprimir_json(valor: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(valor).unwrap_or_else(|_| valor.to_string())
    );
}

/// Mensagem no erro padrão e código de saída 1.
pub fn falha(mensagem: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", mensagem.as_ref());
    ExitCode::from(1)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn status_aceita_so_os_textos_do_banco() {
        assert_eq!(status_de_texto("active").unwrap(), Status::Ativo);
        assert_eq!(status_de_texto("inactive").unwrap(), Status::Inativo);
        assert_eq!(status_de_texto("archived").unwrap(), Status::Arquivado);
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
        assert_eq!(validar_limite(200).unwrap().limite, 200);
    }

    #[test]
    fn config_json_precisa_ser_objeto() {
        assert!(validar_config_json("{}").is_ok());
        assert!(validar_config_json("[1]").is_err());
        assert!(validar_config_json("não é json").is_err());
    }

    #[test]
    fn exigir_offline_recusa_sem_a_flag() {
        assert!(exigir_offline(true).is_ok());
        assert!(exigir_offline(false).unwrap_err().contains("--offline"));
    }

    #[test]
    fn conflito_de_versao_diz_as_duas_versoes() {
        let erro = ErroStore::Dominio(DomainError::ConflitoDeVersao {
            entidade: "categoria de produto",
            id: "x".to_owned(),
            esperada: 2,
            atual: 5,
        });
        let texto = descrever(&erro);
        assert!(texto.contains("informou a versão 2"), "{texto}");
        assert!(texto.contains("versão 5"), "{texto}");
    }
}
