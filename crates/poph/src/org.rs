//! `poph org`: cria, lista, mostra, altera e exclui orgs (tenants). HR-08.
//!
//! Excluir org apaga tudo que está dentro dela. Por isso a exclusão confere antes o conteúdo
//! (agentes, categorias, departamentos e tokens): se houver qualquer coisa, exige `--force`.
//! A conferência acontece antes de chamar o repositório, porque ele apaga e só depois informa
//! quantos agentes havia.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_store::{NewOrg, Org, OrgChange, Store};
use serde_json::{Value, json};

use crate::agente::{derivar_slug, slug_valido};
use crate::apoio::{
    abrir_banco, contar_todos, descrever, exigir_offline, falha, imprimir_json, json_de_texto,
    resolver_org, status_de_texto, status_em_portugues, validar_config_json, validar_limite,
    validar_texto,
};
use crate::config;

/// Subcomandos de `poph org`.
#[derive(Debug, Subcommand)]
pub enum AcaoOrg {
    /// Cria uma org.
    Create(CriarOrg),
    /// Lista as orgs (primeira página, por id).
    List(ListarOrgs),
    /// Mostra uma org e quantos agentes ela tem.
    Show(ReferenciaOrg),
    /// Altera uma org com `--if-match` (a versão lida antes).
    Update(AtualizarOrg),
    /// Exclui a org. Se ela tiver conteúdo, exige `--force` e apaga tudo junto.
    Delete(ExcluirOrg),
}

/// `poph org create`.
#[derive(Debug, Args)]
pub struct CriarOrg {
    /// Nome de exibição (até 120 caracteres).
    #[arg(long)]
    pub name: String,

    /// Slug único (`^[a-z0-9-]{3,64}$`). Sem ele, é derivado do nome.
    #[arg(long)]
    pub slug: Option<String>,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph org list`.
#[derive(Debug, Args)]
pub struct ListarOrgs {
    /// Quantas orgs trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Referência a uma org por id ou slug.
#[derive(Debug, Args)]
pub struct ReferenciaOrg {
    /// Id (UUID v7) ou slug da org.
    pub org: String,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph org update`.
#[derive(Debug, Args)]
pub struct AtualizarOrg {
    /// Id (UUID v7) ou slug da org.
    pub org: String,

    /// Versão lida com `show` (`If-Match`). Obrigatória.
    #[arg(long)]
    pub if_match: u64,

    /// Novo nome.
    #[arg(long)]
    pub name: Option<String>,

    /// Nova situação: `active`, `inactive` ou `archived`.
    #[arg(long)]
    pub status: Option<String>,

    /// Nova configuração da org (objeto JSON: retenção, limites, padrões de modelo).
    #[arg(long)]
    pub config_json: Option<String>,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// `poph org delete`.
#[derive(Debug, Args)]
pub struct ExcluirOrg {
    /// Id (UUID v7) ou slug da org.
    pub org: String,

    /// Confirma a exclusão quando a org tem conteúdo (agentes, categorias, departamentos, tokens).
    #[arg(long)]
    pub force: bool,

    /// Saída em JSON (formato estável).
    #[arg(long)]
    pub json: bool,
}

/// Executa um subcomando de `poph org`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    acao: AcaoOrg,
) -> ExitCode {
    if let Err(erro) = exigir_offline(offline) {
        return falha(erro);
    }
    let store = match abrir_banco(flag, ambiente) {
        Ok(store) => store,
        Err(erro) => return falha(erro),
    };
    match acao {
        AcaoOrg::Create(args) => criar(&store, args),
        AcaoOrg::List(args) => listar(&store, args),
        AcaoOrg::Show(args) => mostrar(&store, args),
        AcaoOrg::Update(args) => atualizar(&store, args),
        AcaoOrg::Delete(args) => excluir(&store, args),
    }
}

fn criar(store: &Store, args: CriarOrg) -> ExitCode {
    let nome = match validar_texto("nome", &args.name, 120) {
        Ok(nome) => nome,
        Err(erro) => return falha(erro),
    };
    let slug = args.slug.clone().unwrap_or_else(|| derivar_slug(&nome));
    if !slug_valido(&slug) {
        return falha(format!(
            "slug `{slug}` inválido: use de 3 a 64 caracteres, só a-z, 0-9 e `-`"
        ));
    }
    match store.orgs().por_slug(&slug) {
        Ok(_) => return falha(format!("já existe uma org com o slug `{slug}`")),
        Err(erro) if erro.is_not_found() => {}
        Err(erro) => return falha(descrever(&erro)),
    }
    let nova = NewOrg {
        name: nome,
        slug,
        config_json: String::new(),
    };
    match store.orgs().criar(nova) {
        Ok(org) => saida(&org, args.json, "org criada", None),
        Err(erro) => falha(descrever(&erro)),
    }
}

fn listar(store: &Store, args: ListarOrgs) -> ExitCode {
    let pagina = match validar_limite(args.limit) {
        Ok(pagina) => pagina,
        Err(erro) => return falha(erro),
    };
    match store.orgs().listar(pagina) {
        Ok(orgs) => {
            if args.json {
                let itens: Vec<Value> = orgs.iter().map(json_org).collect();
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "total": itens.len(),
                    "orgs": itens,
                }));
            } else if orgs.is_empty() {
                println!("Nenhuma org cadastrada.");
            } else {
                for org in &orgs {
                    println!(
                        "{}  slug={:<24} status={:<9} {}",
                        org.id,
                        org.slug,
                        status_em_portugues(org.status),
                        org.name
                    );
                }
                println!("\n{} org(s).", orgs.len());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(descrever(&erro)),
    }
}

fn mostrar(store: &Store, args: ReferenciaOrg) -> ExitCode {
    let id = match resolver_org(store, &args.org) {
        Ok(id) => id,
        Err(erro) => return falha(erro),
    };
    let org = match store.orgs().get(id) {
        Ok(org) => org,
        Err(erro) => return falha(descrever(&erro)),
    };
    let agentes = match contar_agentes(store, id) {
        Ok(total) => total,
        Err(erro) => return falha(erro),
    };
    saida(&org, args.json, "", Some(agentes))
}

fn atualizar(store: &Store, args: AtualizarOrg) -> ExitCode {
    let id = match resolver_org(store, &args.org) {
        Ok(id) => id,
        Err(erro) => return falha(erro),
    };
    let nome = match args
        .name
        .as_deref()
        .map(|nome| validar_texto("nome", nome, 120))
        .transpose()
    {
        Ok(nome) => nome,
        Err(erro) => return falha(erro),
    };
    let status = match args.status.as_deref().map(status_de_texto).transpose() {
        Ok(status) => status,
        Err(erro) => return falha(erro),
    };
    let config_json = match args
        .config_json
        .as_deref()
        .map(validar_config_json)
        .transpose()
    {
        Ok(config) => config,
        Err(erro) => return falha(erro),
    };
    if nome.is_none() && status.is_none() && config_json.is_none() {
        return falha("nada a alterar: informe --name, --status ou --config-json");
    }
    let mudanca = OrgChange {
        name: nome,
        status,
        config_json,
    };
    match store.orgs().atualizar(id, args.if_match, mudanca) {
        Ok(org) => saida(&org, args.json, "org alterada", None),
        Err(erro) => falha(descrever(&erro)),
    }
}

fn excluir(store: &Store, args: ExcluirOrg) -> ExitCode {
    let id = match resolver_org(store, &args.org) {
        Ok(id) => id,
        Err(erro) => return falha(erro),
    };
    let conteudo = match conteudo_da_org(store, id) {
        Ok(conteudo) => conteudo,
        Err(erro) => return falha(erro),
    };
    if conteudo.total() > 0 && !args.force {
        return falha(format!(
            "a org tem conteúdo ({}). Excluir a org apaga tudo isso junto, e os produtos das \
             categorias. Para confirmar, repita o comando com --force",
            conteudo.resumo()
        ));
    }
    let resultado = if args.force {
        store
            .orgs()
            .excluir_forcado(id)
            .map_err(|erro| descrever(&erro))
    } else {
        store.orgs().excluir(id).map(|()| 0u64).map_err(|erro| {
            if matches!(
                erro,
                poph_store::StoreError::Domain(poph_types::DomainError::Conflict { .. })
            ) {
                "a org tem agentes: exclua os agentes antes, ou repita com --force".to_owned()
            } else {
                descrever(&erro)
            }
        })
    };
    match resultado {
        Ok(agentes_removidos) => {
            if args.json {
                imprimir_json(&json!({
                    "versao_relatorio": 1,
                    "removida": true,
                    "org_id": id.to_string(),
                    "agentes_removidos": agentes_removidos,
                    "categorias_removidas": conteudo.categorias,
                    "departamentos_removidos": conteudo.departamentos,
                    "tokens_removidos": conteudo.tokens,
                }));
            } else {
                println!("org {id} excluída ({})", conteudo.resumo());
            }
            ExitCode::SUCCESS
        }
        Err(erro) => falha(erro),
    }
}

/// O que existe dentro da org (o que a exclusão levaria junto).
struct Conteudo {
    agentes: usize,
    categorias: usize,
    departamentos: usize,
    tokens: usize,
}

impl Conteudo {
    fn total(&self) -> usize {
        self.agentes + self.categorias + self.departamentos + self.tokens
    }

    fn resumo(&self) -> String {
        format!(
            "{} agente(s), {} categoria(s), {} departamento(s), {} token(s)",
            self.agentes, self.categorias, self.departamentos, self.tokens
        )
    }
}

fn conteudo_da_org(store: &Store, org: poph_types::OrgId) -> Result<Conteudo, String> {
    Ok(Conteudo {
        agentes: contar_agentes(store, org)?,
        categorias: contar_todos(
            |pagina| store.categories().listar(org, pagina),
            |item| item.id.as_uuid(),
        )?,
        departamentos: contar_todos(
            |pagina| store.departments().listar(org, pagina),
            |item| item.id.as_uuid(),
        )?,
        tokens: contar_todos(
            |pagina| store.tokens().listar(org, pagina),
            |item| item.id.as_uuid(),
        )?,
    })
}

fn contar_agentes(store: &Store, org: poph_types::OrgId) -> Result<usize, String> {
    contar_todos(
        |pagina| store.agents().listar(org, pagina),
        |item| item.id.as_uuid(),
    )
}

/// Saída de uma org: JSON (com `agents_count` quando foi pedido) ou texto.
fn saida(org: &Org, json: bool, mensagem: &str, agentes: Option<usize>) -> ExitCode {
    if json {
        let mut valor = json_org(org);
        if let (Some(total), Value::Object(mapa)) = (agentes, &mut valor) {
            mapa.insert("agents_count".to_owned(), json!(total));
        }
        imprimir_json(&json!({
            "versao_relatorio": 1,
            "org": valor,
        }));
    } else {
        if !mensagem.is_empty() {
            println!("{mensagem}\n");
        }
        print!("{}", texto_org(org, agentes));
    }
    ExitCode::SUCCESS
}

fn json_org(org: &Org) -> Value {
    json!({
        "id": org.id.to_string(),
        "name": org.name,
        "slug": org.slug,
        "status": org.status.as_text(),
        "config": json_de_texto(&org.config_json),
        "version": org.version,
        "created_at": org.created_at,
        "updated_at": org.updated_at,
    })
}

fn texto_org(org: &Org, agentes: Option<usize>) -> String {
    let mut texto = format!(
        "org {}\n  nome: {}\n  slug: {}\n  situação: {}\n",
        org.id,
        org.name,
        org.slug,
        status_em_portugues(org.status)
    );
    if let Some(total) = agentes {
        texto.push_str(&format!("  agentes: {total}\n"));
    }
    texto.push_str(&format!("  versão: {}\n", org.version));
    texto
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn conteudo_zerado_nao_pede_force() {
        let vazio = Conteudo {
            agentes: 0,
            categorias: 0,
            departamentos: 0,
            tokens: 0,
        };
        assert_eq!(vazio.total(), 0);
        let com_categoria = Conteudo {
            categorias: 1,
            ..vazio
        };
        assert_eq!(com_categoria.total(), 1);
        assert!(com_categoria.resumo().contains("1 categoria(s)"));
    }
}
