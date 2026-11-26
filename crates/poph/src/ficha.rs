//! `poph ficha`: leitura das Fichas da org pela linha de comando (FX-31, PRD §7.3 e W6).
//!
//! Subcomandos: `listar` (com filtros e cursor) e `mostrar` (a Ficha, as execuções com as etapas e os
//! campos respondidos). Lê direto no banco e usa as mesmas funções que a API usa
//! (`poph_server::fichas_servico`), então o JSON é o mesmo.
//!
//! Saída: 0 quando deu certo; 1 para erro de negócio (Ficha inexistente ou de outra org, filtro
//! inválido); 2 uso incorreto (o clap decide). Com `--json`, `versao_relatorio` = 1.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_server::fichas_servico::{detalhe, ficha_json};
use poph_store::{FiltroFichas, Store};
use poph_types::{OrgId, RecordId, RecordStatus};
use serde_json::{Value, json};

use crate::apoio::{
    abrir_com_configuracao, descrever, falha, imprimir_json, pagina_de, parse_id, resolver_agente,
    resolver_org,
};
use crate::config;

/// Versão do formato JSON desta saída (muda só se a forma mudar).
const VERSAO_RELATORIO: u64 = 1;

/// Subcomandos de `poph ficha`.
#[derive(Debug, Subcommand)]
pub enum AcaoFicha {
    /// Lista as Fichas da org, com filtros (primeira página, em ordem de id).
    Listar(ListarArgs),
    /// Mostra uma Ficha: as execuções de fluxo, as etapas e os campos respondidos.
    Mostrar(MostrarArgs),
}

/// Opções de `listar`.
#[derive(Debug, Args)]
pub struct ListarArgs {
    /// Situação: `open` ou `closed`.
    #[arg(long)]
    pub status: Option<String>,

    /// Agente da Ficha: id (UUID v7) ou slug.
    #[arg(long)]
    pub agente: Option<String>,

    /// Canal da conversa, por exemplo `whatsapp`.
    #[arg(long)]
    pub canal: Option<String>,

    /// Só Fichas abertas a partir deste instante, em milissegundos desde 1970.
    #[arg(long)]
    pub desde: Option<i64>,

    /// Quantas Fichas trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// `next_cursor` da página anterior.
    #[arg(long)]
    pub cursor: Option<String>,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Opções de `mostrar`.
#[derive(Debug, Args)]
pub struct MostrarArgs {
    /// Id (UUID v7) da Ficha.
    pub ficha: String,

    /// Saída em JSON (formato estável, com `versao_relatorio`).
    #[arg(long)]
    pub json: bool,
}

/// Executa um subcomando de `poph ficha`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
    acao: AcaoFicha,
) -> ExitCode {
    let resultado = abrir_com_configuracao(flag, ambiente).and_then(|(store, _configuracao)| {
        let org = org
            .ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())
            .and_then(|texto| resolver_org(&store, texto))?;
        match acao {
            AcaoFicha::Listar(args) => listar(&store, org, args),
            AcaoFicha::Mostrar(args) => mostrar(&store, org, args),
        }
    });
    match resultado {
        Ok(codigo) => codigo,
        Err(mensagem) => falha(mensagem),
    }
}

fn listar(store: &Store, org: OrgId, args: ListarArgs) -> Result<ExitCode, String> {
    let pagina = pagina_de(args.limit, args.cursor.as_deref())?;
    let filtro = FiltroFichas {
        status: args
            .status
            .as_deref()
            .map(RecordStatus::from_text)
            .transpose()
            .map_err(|erro| erro.to_string())?,
        agente: match args.agente.as_deref() {
            Some(texto) => Some(resolver_agente(store, org, texto)?.id),
            None => None,
        },
        canal: match args.canal.as_deref() {
            Some(texto) if texto.trim().is_empty() => {
                return Err("informe o canal, por exemplo whatsapp".to_owned());
            }
            Some(texto) => Some(texto.trim().to_owned()),
            None => None,
        },
        desde: args.desde,
    };
    let fichas = store
        .fichas()
        .listar(org, &filtro, pagina)
        .map_err(|erro| descrever(&erro))?;
    let next_cursor = if fichas.len() < pagina.limite as usize {
        None
    } else {
        fichas.last().map(|ficha| ficha.id.to_string())
    };
    let itens: Vec<Value> = fichas.iter().map(ficha_json).collect();
    if args.json {
        imprimir_json(&json!({
            "versao_relatorio": VERSAO_RELATORIO,
            "org_id": org.to_string(),
            "items": itens,
            "next_cursor": next_cursor,
        }));
    } else if itens.is_empty() {
        println!("Nenhuma Ficha com esses filtros.");
    } else {
        for item in &itens {
            println!(
                "{}  {:<7} agente {}  canal {}  aberta em {} (ms desde 1970)",
                texto_de(&item["id"]),
                texto_de(&item["status"]),
                texto_de(&item["agent_id"]),
                texto_de(&item["canal"]),
                texto_de(&item["aberta_em"]),
            );
        }
        if let Some(cursor) = &next_cursor {
            println!("\nPróxima página: --cursor {cursor}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn mostrar(store: &Store, org: OrgId, args: MostrarArgs) -> Result<ExitCode, String> {
    let ficha_id: RecordId = parse_id(&args.ficha)?;
    let corpo = detalhe(store, org, ficha_id).map_err(|erro| {
        if erro.eh_nao_encontrado() {
            format!("não existe Ficha `{}` nesta org", args.ficha)
        } else {
            descrever(&erro)
        }
    })?;
    if args.json {
        let mut saida = corpo;
        saida["versao_relatorio"] = json!(VERSAO_RELATORIO);
        imprimir_json(&saida);
        return Ok(ExitCode::SUCCESS);
    }
    println!("Ficha {}", texto_de(&corpo["id"]));
    println!(
        "  situação: {}, agente {}, canal {}",
        texto_de(&corpo["status"]),
        texto_de(&corpo["agent_id"]),
        texto_de(&corpo["canal"]),
    );
    let execucoes = corpo["execucoes"].as_array().cloned().unwrap_or_default();
    if execucoes.is_empty() {
        println!("  sem execuções de fluxo.");
    }
    for execucao in &execucoes {
        println!(
            "\n  execução {} (fluxo {}): {}, no atual {}",
            texto_de(&execucao["id"]),
            texto_de(&execucao["fluxo_id"]),
            texto_de(&execucao["status"]),
            texto_de(&execucao["no_atual"]),
        );
        for etapa in execucao["etapas"].as_array().cloned().unwrap_or_default() {
            let campos = etapa["dados"]
                .as_object()
                .map(|mapa| {
                    mapa.iter()
                        .map(|(nome, valor)| format!("{nome}={}", texto_de(valor)))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!(
                "    etapa {} ({}): campos [{campos}]",
                texto_de(&etapa["no_id"]),
                texto_de(&etapa["status"]),
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Texto de um campo JSON, sem as aspas (para a saída de texto).
fn texto_de(valor: &Value) -> String {
    match valor {
        Value::String(texto) => texto.clone(),
        Value::Null => "-".to_owned(),
        outro => outro.to_string(),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn texto_de_tira_as_aspas_e_marca_o_vazio() {
        assert_eq!(texto_de(&json!("open")), "open");
        assert_eq!(texto_de(&Value::Null), "-");
    }
}
