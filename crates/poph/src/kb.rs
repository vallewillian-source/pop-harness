//! `poph kb`: base de conhecimento do agente (HR-17).
//!
//! Grupos, itens (até 1000 caracteres), vínculo grupo↔agente e o estado do índice. Toda escrita que
//! muda o que o agente deve saber reindexa os agentes ligados na hora — se o modelo de embedding
//! estiver instalado. Sem o modelo, a escrita vale e o comando avisa que o índice ficou pendente.
//!
//! `poph kb reindex` reconstrói o índice de um agente (ou de todos os agentes da org).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_agent::BaseConhecimento;
use poph_rag::trechos::hash_origem;
use poph_store::{
    KnowledgeGroup, KnowledgeGroupChange, KnowledgeItem, KnowledgeItemChange, NewKnowledgeGroup,
    NewKnowledgeItem, Page, Status, Store,
};
use poph_types::{AgentId, HarnessConfig, KnowledgeGroupId, KnowledgeItemId, OrgId};
use serde_json::{Value, json};

use crate::apoio::{
    descrever, falha, parse_id, status_de_texto, status_em_portugues, validar_texto,
};
use crate::config;

/// Limite do conteúdo de um item, em caracteres (PARIDADE_AGENTE.md §3).
const LIMITE_CONTEUDO: usize = 1000;

/// Subcomandos de `poph kb`.
#[derive(Debug, Subcommand)]
pub enum AcaoKb {
    /// Grupos de conhecimento: criar, listar, mostrar, alterar e excluir.
    #[command(subcommand)]
    Group(AcaoGrupo),
    /// Itens da base: criar, listar, mostrar, alterar e excluir.
    #[command(subcommand)]
    Item(AcaoItem),
    /// Liga um grupo a um agente (o agente passa a consultar os itens dele).
    Link(LinkArgs),
    /// Desliga um grupo de um agente.
    Unlink(LinkArgs),
    /// Reconstrói o índice de um agente (ou de todos os agentes da org).
    Reindex(ReindexArgs),
    /// Consulta a base como o agente faria (busca híbrida).
    Search(SearchArgs),
    /// Mostra o estado do índice dos itens (geral ou de um grupo).
    Status(StatusArgs),
}

/// Subcomandos de `poph kb group`.
#[derive(Debug, Subcommand)]
pub enum AcaoGrupo {
    /// Cria um grupo.
    Create(CriarGrupo),
    /// Lista os grupos da org.
    List(ListarGrupos),
    /// Mostra um grupo e os agentes que o usam.
    Show(RefGrupo),
    /// Altera um grupo com `--if-match`.
    Update(AtualizarGrupo),
    /// Exclui o grupo e os itens dele. Exige `--force`.
    Delete(ExcluirGrupo),
}

/// Subcomandos de `poph kb item`.
#[derive(Debug, Subcommand)]
pub enum AcaoItem {
    /// Cria um item (conteúdo até 1000 caracteres).
    Create(CriarItem),
    /// Lista os itens de um grupo, com o estado no índice.
    List(ListarItens),
    /// Mostra um item, com a fonte e o estado no índice.
    Show(RefItem),
    /// Altera um item com `--if-match`.
    Update(AtualizarItem),
    /// Exclui o item.
    Delete(ExcluirItem),
}

/// `poph kb group create`.
#[derive(Debug, Args)]
pub struct CriarGrupo {
    /// Nome (até 120 caracteres; não repete na org).
    #[arg(long)]
    pub name: String,
    /// Descrição curta.
    #[arg(long, default_value = "")]
    pub description: String,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb group list`.
#[derive(Debug, Args)]
pub struct ListarGrupos {
    /// Quantos grupos trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// Referência a um grupo.
#[derive(Debug, Args)]
pub struct RefGrupo {
    /// Id (UUID v7) do grupo.
    pub grupo: String,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb group update`.
#[derive(Debug, Args)]
pub struct AtualizarGrupo {
    /// Id (UUID v7) do grupo.
    pub grupo: String,
    /// Versão lida com `show` (`If-Match`). Obrigatória.
    #[arg(long)]
    pub if_match: u64,
    /// Novo nome.
    #[arg(long)]
    pub name: Option<String>,
    /// Nova descrição.
    #[arg(long)]
    pub description: Option<String>,
    /// Nova situação: `active`, `inactive` ou `archived` (inativo sai do índice, D-05).
    #[arg(long)]
    pub status: Option<String>,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb group delete`.
#[derive(Debug, Args)]
pub struct ExcluirGrupo {
    /// Id (UUID v7) do grupo.
    pub grupo: String,
    /// Confirma: os itens do grupo saem junto.
    #[arg(long)]
    pub force: bool,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb item create`.
#[derive(Debug, Args)]
pub struct CriarItem {
    /// Grupo dono (id UUID v7).
    #[arg(long)]
    pub group: String,
    /// Título (vira a fonte mostrada ao modelo).
    #[arg(long)]
    pub title: String,
    /// Conteúdo do item.
    #[arg(long, conflicts_with = "content_file")]
    pub content: Option<String>,
    /// Arquivo com o conteúdo do item.
    #[arg(long, value_name = "ARQUIVO")]
    pub content_file: Option<PathBuf>,
    /// Etiquetas separadas por vírgula.
    #[arg(long, value_delimiter = ',')]
    pub tag: Vec<String>,
    /// Posição dentro do grupo.
    #[arg(long, default_value_t = 0)]
    pub position: i64,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb item list`.
#[derive(Debug, Args)]
pub struct ListarItens {
    /// Grupo dono (id UUID v7).
    #[arg(long)]
    pub group: String,
    /// Quantos itens trazer (1 a 200).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// Referência a um item.
#[derive(Debug, Args)]
pub struct RefItem {
    /// Id (UUID v7) do item.
    pub item: String,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb item update`.
#[derive(Debug, Args)]
pub struct AtualizarItem {
    /// Id (UUID v7) do item.
    pub item: String,
    /// Versão lida com `show` (`If-Match`). Obrigatória.
    #[arg(long)]
    pub if_match: u64,
    /// Move para outro grupo.
    #[arg(long)]
    pub group: Option<String>,
    /// Novo título.
    #[arg(long)]
    pub title: Option<String>,
    /// Novo conteúdo.
    #[arg(long, conflicts_with = "content_file")]
    pub content: Option<String>,
    /// Arquivo com o novo conteúdo.
    #[arg(long, value_name = "ARQUIVO")]
    pub content_file: Option<PathBuf>,
    /// Novas etiquetas separadas por vírgula (substitui as anteriores).
    #[arg(long, value_delimiter = ',')]
    pub tag: Vec<String>,
    /// Nova posição.
    #[arg(long)]
    pub position: Option<i64>,
    /// Nova situação: `active`, `inactive` ou `archived`.
    #[arg(long)]
    pub status: Option<String>,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb item delete`.
#[derive(Debug, Args)]
pub struct ExcluirItem {
    /// Id (UUID v7) do item.
    pub item: String,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb link` e `poph kb unlink`.
#[derive(Debug, Args)]
pub struct LinkArgs {
    /// Grupo (id UUID v7).
    pub grupo: String,
    /// Agente (id UUID v7 ou slug).
    #[arg(long)]
    pub agent: String,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb reindex`.
#[derive(Debug, Args)]
pub struct ReindexArgs {
    /// Agente (id UUID v7 ou slug). Sem ele, reindexa todos os agentes da org.
    #[arg(long)]
    pub agent: Option<String>,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Texto da consulta.
    pub consulta: String,
    /// Agente cuja base será consultada (id UUID v7 ou slug).
    #[arg(long)]
    pub agent: String,
    /// Quantos trechos trazer (padrão: o top-k do agente).
    #[arg(long)]
    pub top_k: Option<usize>,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// `poph kb status`.
#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Limita a um grupo (id UUID v7).
    #[arg(long)]
    pub group: Option<String>,
    /// Saída em JSON.
    #[arg(long)]
    pub json: bool,
}

/// Executa um subcomando de `poph kb`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    acao: AcaoKb,
) -> ExitCode {
    if !offline {
        return falha(
            "use `--offline`: a base de conhecimento é mantida direto no banco enquanto o modo \
             com servidor não existe",
        );
    }
    let (store, org, configuracao) = match abrir(flag, ambiente, org) {
        Ok(trio) => trio,
        Err(erro) => return falha(erro),
    };
    let contexto = Contexto {
        store,
        org,
        configuracao,
    };
    match acao {
        AcaoKb::Group(AcaoGrupo::Create(args)) => contexto.criar_grupo(args),
        AcaoKb::Group(AcaoGrupo::List(args)) => contexto.listar_grupos(args),
        AcaoKb::Group(AcaoGrupo::Show(args)) => contexto.mostrar_grupo(args),
        AcaoKb::Group(AcaoGrupo::Update(args)) => contexto.atualizar_grupo(args),
        AcaoKb::Group(AcaoGrupo::Delete(args)) => contexto.excluir_grupo(args),
        AcaoKb::Item(AcaoItem::Create(args)) => contexto.criar_item(args),
        AcaoKb::Item(AcaoItem::List(args)) => contexto.listar_itens(args),
        AcaoKb::Item(AcaoItem::Show(args)) => contexto.mostrar_item(args),
        AcaoKb::Item(AcaoItem::Update(args)) => contexto.atualizar_item(args),
        AcaoKb::Item(AcaoItem::Delete(args)) => contexto.excluir_item(args),
        AcaoKb::Link(args) => contexto.vincular(args, true),
        AcaoKb::Unlink(args) => contexto.vincular(args, false),
        AcaoKb::Reindex(args) => contexto.reindexar(args),
        AcaoKb::Search(args) => contexto.buscar(args),
        AcaoKb::Status(args) => contexto.status(args),
    }
}

fn abrir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    org: Option<&str>,
) -> Result<(Arc<Store>, OrgId, HarnessConfig), String> {
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let configuracao =
        config::carregar(arquivo.as_deref(), ambiente).map_err(|erro| format!("{erro}"))?;
    let store = Arc::new(
        Store::open(&configuracao).map_err(|erro| format!("não consegui abrir o banco: {erro}"))?,
    );
    let org = crate::apoio::resolver_org(&store, texto)?;
    Ok((store, org, configuracao))
}

/// Estado do índice de um item.
///
/// - `indexado`: o hash do conteúdo atual está gravado no `rag.db` para todos os agentes ligados;
/// - `pendente`: o conteúdo mudou (ou nunca foi indexado) e falta sincronizar;
/// - `inativo`: item ou grupo inativo — fora do índice de propósito (D-05);
/// - `nao_vinculado`: nenhum agente usa o grupo;
/// - `erro`: não deu para conferir (modelo de embedding indisponível, índice ilegível).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EstadoIndice {
    Indexado,
    Pendente,
    Inativo,
    NaoVinculado,
    Erro,
}

impl EstadoIndice {
    fn como_texto(self) -> &'static str {
        match self {
            EstadoIndice::Indexado => "indexado",
            EstadoIndice::Pendente => "pendente",
            EstadoIndice::Inativo => "inativo",
            EstadoIndice::NaoVinculado => "nao_vinculado",
            EstadoIndice::Erro => "erro",
        }
    }
}

struct Contexto {
    store: Arc<Store>,
    org: OrgId,
    configuracao: HarnessConfig,
}

impl Contexto {
    // ───────────────────────────── Grupos ─────────────────────────────

    fn criar_grupo(&self, args: CriarGrupo) -> ExitCode {
        let nome = match validar_texto("nome do grupo", &args.name, 120) {
            Ok(nome) => nome,
            Err(erro) => return falha(erro),
        };
        match self.store.knowledge().create_group(
            self.org,
            NewKnowledgeGroup {
                name: nome,
                description: args.description.trim().to_owned(),
            },
        ) {
            Ok(grupo) => {
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "group": json_grupo(&grupo),
                    }));
                } else {
                    println!(
                        "grupo criado: {}  {}  (versão {})\nuse `poph kb item create --group {}` \
                         para adicionar itens",
                        grupo.id, grupo.name, grupo.version, grupo.id
                    );
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn listar_grupos(&self, args: ListarGrupos) -> ExitCode {
        let pagina = Page::with_limit(args.limit);
        match self.store.knowledge().list_groups(self.org, pagina) {
            Ok(grupos) => {
                if args.json {
                    let itens: Vec<Value> = grupos.iter().map(json_grupo).collect();
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "org_id": self.org.to_string(),
                        "total": itens.len(),
                        "groups": itens,
                    }));
                } else if grupos.is_empty() {
                    println!("Nenhum grupo de conhecimento na org.");
                } else {
                    for grupo in &grupos {
                        println!(
                            "{}  {:<9} {}",
                            grupo.id,
                            status_em_portugues(grupo.status),
                            grupo.name
                        );
                    }
                    println!("\n{} grupo(s).", grupos.len());
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn mostrar_grupo(&self, args: RefGrupo) -> ExitCode {
        let id: KnowledgeGroupId = match parse_id(&args.grupo) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        match self.store.knowledge().get_group(self.org, id) {
            Ok(grupo) => {
                let agentes = self
                    .store
                    .knowledge()
                    .agents_from_group(self.org, id)
                    .unwrap_or_default();
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "group": json_grupo(&grupo),
                        "agents": agentes.iter().map(AgentId::to_string).collect::<Vec<_>>(),
                    }));
                } else {
                    println!("id:          {}", grupo.id);
                    println!("nome:        {}", grupo.name);
                    println!("descrição:   {}", grupo.description);
                    println!("situação:    {}", status_em_portugues(grupo.status));
                    println!("versão:      {}", grupo.version);
                    println!("itens:       {}", self.total_itens(id));
                    println!(
                        "agentes:     {}",
                        if agentes.is_empty() {
                            "nenhum (use `poph kb link`)".to_owned()
                        } else {
                            agentes
                                .iter()
                                .map(AgentId::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    );
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn atualizar_grupo(&self, args: AtualizarGrupo) -> ExitCode {
        let id: KnowledgeGroupId = match parse_id(&args.grupo) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        let status = match args.status.as_deref().map(status_de_texto).transpose() {
            Ok(status) => status,
            Err(erro) => return falha(erro),
        };
        let mudanca = KnowledgeGroupChange {
            name: args.name,
            description: args.description,
            status,
        };
        match self
            .store
            .knowledge()
            .update_group(self.org, id, args.if_match, mudanca)
        {
            Ok(grupo) => {
                let aviso = self.sincronizar_grupo(id);
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "group": json_grupo(&grupo),
                        "index": aviso,
                    }));
                } else {
                    println!(
                        "grupo atualizado: {}  {}  (versão {})",
                        grupo.id, grupo.name, grupo.version
                    );
                    if let Some(aviso) = aviso {
                        println!("[aviso] índice não atualizado: {aviso}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn excluir_grupo(&self, args: ExcluirGrupo) -> ExitCode {
        let id: KnowledgeGroupId = match parse_id(&args.grupo) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        if !args.force {
            return falha(format!(
                "o grupo {id} pode ter itens e agentes ligados; confirme com --force"
            ));
        }
        let agentes = self
            .store
            .knowledge()
            .agents_from_group(self.org, id)
            .unwrap_or_default();
        match self.store.knowledge().delete_group(self.org, id) {
            Ok(()) => {
                // O grupo saiu; os agentes que o usavam precisam perder os trechos do índice.
                let aviso = self.reindexar_agentes(&agentes);
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "deleted": id.to_string(),
                        "index": aviso,
                    }));
                } else {
                    println!("grupo {id} excluído (itens e vínculos saíram junto)");
                    if let Some(aviso) = aviso {
                        println!("[aviso] índice não atualizado: {aviso}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    // ───────────────────────────── Itens ─────────────────────────────

    fn criar_item(&self, args: CriarItem) -> ExitCode {
        let grupo: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        let conteudo = match ler_conteudo(args.content.as_deref(), args.content_file.as_deref()) {
            Ok(conteudo) => conteudo,
            Err(erro) => return falha(erro),
        };
        let titulo = match validar_texto("título do item", &args.title, 200) {
            Ok(titulo) => titulo,
            Err(erro) => return falha(erro),
        };
        let novo = NewKnowledgeItem {
            group_id: grupo,
            title: titulo,
            content: conteudo,
            tags: args.tag,
            position: args.position,
        };
        match self.store.knowledge().create_item(self.org, novo) {
            Ok(item) => {
                let estado = self.sincronizar_grupo(grupo);
                self.saida_item(&item, args.json, "item criado", estado)
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn listar_itens(&self, args: ListarItens) -> ExitCode {
        let grupo: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        let pagina = Page::with_limit(args.limit);
        match self.store.knowledge().list_items(self.org, grupo, pagina) {
            Ok(itens) => {
                if args.json {
                    let base = self.base().ok();
                    let lista: Vec<Value> = itens
                        .iter()
                        .map(|item| {
                            let estado = base
                                .as_ref()
                                .map(|base| self.estado_do_item(base, item))
                                .unwrap_or(EstadoIndice::Erro);
                            json_item(item, estado)
                        })
                        .collect();
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "group_id": grupo.to_string(),
                        "total": lista.len(),
                        "items": lista,
                    }));
                } else if itens.is_empty() {
                    println!("Nenhum item neste grupo.");
                } else {
                    let base = self.base().ok();
                    for item in &itens {
                        let estado = base
                            .as_ref()
                            .map(|base| self.estado_do_item(base, item))
                            .unwrap_or(EstadoIndice::Erro);
                        println!(
                            "{}  {:<9} {:<14} {}",
                            item.id,
                            status_em_portugues(item.status),
                            estado.como_texto(),
                            item.title
                        );
                    }
                    println!("\n{} item(s).", itens.len());
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn mostrar_item(&self, args: RefItem) -> ExitCode {
        let id: KnowledgeItemId = match parse_id(&args.item) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        match self.store.knowledge().get_item(self.org, id) {
            Ok(item) => {
                let base = self.base().ok();
                let estado = base
                    .as_ref()
                    .map(|base| self.estado_do_item(base, &item))
                    .unwrap_or(EstadoIndice::Erro);
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "item": json_item(&item, estado),
                    }));
                } else {
                    println!("id:        {}", item.id);
                    println!("grupo:     {}", item.group_id);
                    println!("título:    {}", item.title);
                    println!("situação:  {}", status_em_portugues(item.status));
                    println!("índice:    {}", estado.como_texto());
                    println!("versão:    {}", item.version);
                    println!("tags:      {}", item.tags.join(", "));
                    println!("\n{}\n", item.content);
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn atualizar_item(&self, args: AtualizarItem) -> ExitCode {
        let id: KnowledgeItemId = match parse_id(&args.item) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        let grupo = match args
            .group
            .as_deref()
            .map(parse_id::<KnowledgeGroupId>)
            .transpose()
        {
            Ok(grupo) => grupo,
            Err(erro) => return falha(erro),
        };
        let conteudo = match (args.content.as_deref(), args.content_file.as_deref()) {
            (None, None) => None,
            (inline, arquivo) => match ler_conteudo(inline, arquivo) {
                Ok(conteudo) => Some(conteudo),
                Err(erro) => return falha(erro),
            },
        };
        let status = match args.status.as_deref().map(status_de_texto).transpose() {
            Ok(status) => status,
            Err(erro) => return falha(erro),
        };
        let tags = (!args.tag.is_empty()).then_some(args.tag);
        let mudanca = KnowledgeItemChange {
            group_id: grupo,
            title: args.title,
            content: conteudo,
            tags,
            position: args.position,
            status,
        };
        match self
            .store
            .knowledge()
            .update_item(self.org, id, args.if_match, mudanca)
        {
            Ok(item) => {
                let estado = self.sincronizar_grupo(item.group_id);
                self.saida_item(&item, args.json, "item atualizado", estado)
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn excluir_item(&self, args: ExcluirItem) -> ExitCode {
        let id: KnowledgeItemId = match parse_id(&args.item) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        let grupo = self
            .store
            .knowledge()
            .get_item(self.org, id)
            .map(|item| item.group_id)
            .ok();
        match self.store.knowledge().delete_item(self.org, id) {
            Ok(()) => {
                let aviso = grupo.and_then(|grupo| self.sincronizar_grupo(grupo));
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "deleted": id.to_string(),
                        "index": aviso,
                    }));
                } else {
                    println!("item {id} excluído");
                    if let Some(aviso) = aviso {
                        println!("[aviso] índice não atualizado: {aviso}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    // ───────────────────────────── Vínculo, busca, índice ─────────────────────────────

    fn vincular(&self, args: LinkArgs, ligar: bool) -> ExitCode {
        let grupo: KnowledgeGroupId = match parse_id(&args.grupo) {
            Ok(id) => id,
            Err(erro) => return falha(erro),
        };
        let agente = match self.resolver_agente(&args.agent) {
            Ok(agente) => agente,
            Err(erro) => return falha(erro),
        };
        let resultado = if ligar {
            self.store
                .knowledge()
                .link_agent(self.org, agente, grupo)
                .map(|()| true)
        } else {
            self.store.knowledge().unlink_agent(self.org, agente, grupo)
        };
        match resultado {
            Ok(mudou) => {
                let aviso = self.reindexar_agentes(&[agente]);
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "agent": agente.to_string(),
                        "group": grupo.to_string(),
                        "linked": ligar,
                        "changed": mudou,
                        "index": aviso,
                    }));
                } else {
                    println!(
                        "{}: grupo {} {} agente {}",
                        if ligar { "vínculo" } else { "desvínculo" },
                        grupo,
                        if ligar { "ligado ao" } else { "desligado do" },
                        agente
                    );
                    if let Some(aviso) = aviso {
                        println!("[aviso] índice não atualizado: {aviso}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(descrever(&erro)),
        }
    }

    fn reindexar(&self, args: ReindexArgs) -> ExitCode {
        let base = match self.base() {
            Ok(base) => base,
            Err(erro) => return falha(erro),
        };
        let agentes = match &args.agent {
            Some(referencia) => match self.resolver_agente(referencia) {
                Ok(agente) => vec![agente],
                Err(erro) => return falha(erro),
            },
            None => match self.todos_os_agentes() {
                Ok(agentes) => agentes,
                Err(erro) => return falha(erro),
            },
        };
        let mut relatorios = Vec::new();
        for agente in agentes {
            match self.sincronizar(&base, agente) {
                Ok(relatorio) => relatorios.push(json!({
                    "agent": agente.to_string(),
                    "indexed": json_indexacao(&relatorio),
                })),
                Err(erro) => return falha(erro),
            }
        }
        if args.json {
            imprimir(&json!({
                "versao_relatorio": 1,
                "agents": relatorios,
            }));
        } else if relatorios.is_empty() {
            println!("Nenhum agente na org para reindexar.");
        } else {
            for relatorio in &relatorios {
                println!(
                    "agente {}: novos {}, alterados {}, removidos {}, trechos {}",
                    relatorio["agent"].as_str().unwrap_or("?"),
                    relatorio["indexed"]["itens_novos"],
                    relatorio["indexed"]["itens_alterados"],
                    relatorio["indexed"]["itens_removidos"],
                    relatorio["indexed"]["trechos_gravados"],
                );
            }
        }
        ExitCode::SUCCESS
    }

    fn buscar(&self, args: SearchArgs) -> ExitCode {
        let base = match self.base() {
            Ok(base) => base,
            Err(erro) => return falha(erro),
        };
        let agente = match self.resolver_agente(&args.agent) {
            Ok(agente) => agente,
            Err(erro) => return falha(erro),
        };
        let agente_registro = match self.store.agents().get(self.org, agente) {
            Ok(agente) => agente,
            Err(erro) => return falha(descrever(&erro)),
        };
        let top_k = args.top_k.unwrap_or_else(|| {
            usize::try_from(agente_registro.rag_top_k)
                .unwrap_or(3)
                .max(1)
        });
        let runtime = match runtime() {
            Ok(runtime) => runtime,
            Err(erro) => return falha(erro),
        };
        let trechos = runtime.block_on(async {
            use poph_agent::{BuscadorBase, ParametrosBusca};
            let busca: &dyn BuscadorBase = base.as_ref();
            busca
                .buscar(
                    self.org,
                    agente,
                    args.consulta.clone(),
                    ParametrosBusca {
                        top_k,
                        corte: agente_registro.rag_cutoff as f32,
                    },
                )
                .await
        });
        match trechos {
            Ok(trechos) => {
                if args.json {
                    imprimir(&json!({
                        "versao_relatorio": 1,
                        "agent": agente.to_string(),
                        "query": args.consulta,
                        "total": trechos.len(),
                        "chunks": trechos,
                    }));
                } else if trechos.is_empty() {
                    println!("Nenhum trecho acima do corte de relevância.");
                } else {
                    for (indice, trecho) in trechos.iter().enumerate() {
                        println!(
                            "[{}] {:.3}  fonte: {} ({})",
                            indice + 1,
                            trecho.similaridade,
                            trecho.titulo,
                            trecho.grupo
                        );
                        println!("    {}\n", trecho.texto.replace('\n', " "));
                    }
                }
                ExitCode::SUCCESS
            }
            Err(erro) => falha(format!("a busca falhou: {erro}")),
        }
    }

    fn status(&self, args: StatusArgs) -> ExitCode {
        let grupos = match &args.group {
            Some(texto) => match parse_id::<KnowledgeGroupId>(texto) {
                Ok(id) => match self.store.knowledge().get_group(self.org, id) {
                    Ok(grupo) => vec![grupo],
                    Err(erro) => return falha(descrever(&erro)),
                },
                Err(erro) => return falha(erro),
            },
            None => match self
                .store
                .knowledge()
                .list_groups(self.org, Page::with_limit(200))
            {
                Ok(grupos) => grupos,
                Err(erro) => return falha(descrever(&erro)),
            },
        };
        let base = self.base().ok();
        let mut linhas = Vec::new();
        for grupo in &grupos {
            let itens = self
                .store
                .knowledge()
                .list_items(self.org, grupo.id, Page::with_limit(200))
                .unwrap_or_default();
            for item in itens {
                let estado = base
                    .as_ref()
                    .map(|base| self.estado_do_item(base, &item))
                    .unwrap_or(EstadoIndice::Erro);
                linhas.push(json!({
                    "item": item.id.to_string(),
                    "group": grupo.name,
                    "title": item.title,
                    "status": item.status.as_text(),
                    "index": estado.como_texto(),
                }));
            }
        }
        if args.json {
            imprimir(&json!({
                "versao_relatorio": 1,
                "groups": grupos.len(),
                "items": linhas,
            }));
        } else if linhas.is_empty() {
            println!("Nenhum item na base.");
        } else {
            for linha in &linhas {
                println!(
                    "{}  {:<11} {:<14} {} ({})",
                    linha["item"].as_str().unwrap_or("?"),
                    linha["index"].as_str().unwrap_or("?"),
                    linha["status"].as_str().unwrap_or("?"),
                    linha["title"].as_str().unwrap_or("?"),
                    linha["group"].as_str().unwrap_or("?"),
                );
            }
            println!("\n{} item(s).", linhas.len());
        }
        ExitCode::SUCCESS
    }

    // ───────────────────────────── Apoio ─────────────────────────────

    /// Base pronta para escrever e reindexar. O embedding passa pela fila da HR-15, com o limite
    /// por org: escrever um item de uma org não ocupa o modelo das outras.
    fn base(&self) -> Result<Arc<BaseConhecimento>, String> {
        crate::rag::abrir_base_com_fila(&self.configuracao, Arc::clone(&self.store), self.org)
    }

    fn sincronizar(
        &self,
        base: &BaseConhecimento,
        agente: AgentId,
    ) -> Result<poph_rag::RelatorioIndexacao, String> {
        let runtime = runtime()?;
        runtime
            .block_on(base.sincronizar_agente(self.org, agente))
            .map_err(|erro| format!("não consegui reindexar o agente {agente}: {erro}"))
    }

    /// Sincroniza os agentes ligados ao grupo. Devolve um aviso quando o índice não pôde ser
    /// atualizado (modelo ausente, por exemplo) — a escrita já valeu.
    fn sincronizar_grupo(&self, grupo: KnowledgeGroupId) -> Option<String> {
        let agentes = self
            .store
            .knowledge()
            .agents_from_group(self.org, grupo)
            .ok()?;
        self.reindexar_agentes(&agentes)
    }

    fn reindexar_agentes(&self, agentes: &[AgentId]) -> Option<String> {
        if agentes.is_empty() {
            return None;
        }
        let base = match self.base() {
            Ok(base) => base,
            Err(erro) => return Some(format!("{erro}; rode `poph kb reindex` depois")),
        };
        for agente in agentes {
            if let Err(erro) = self.sincronizar(&base, *agente) {
                return Some(format!("{erro}; rode `poph kb reindex` depois"));
            }
        }
        None
    }

    /// Todos os agentes da org, em páginas (o teto de uma página é 200).
    fn todos_os_agentes(&self) -> Result<Vec<AgentId>, String> {
        let mut agentes = Vec::new();
        let mut pagina = Page::with_limit(200);
        loop {
            let lote = self
                .store
                .agents()
                .list(self.org, pagina)
                .map_err(|erro| descrever(&erro))?;
            if lote.is_empty() {
                return Ok(agentes);
            }
            pagina.after = lote.last().map(|agente| agente.id.as_uuid());
            agentes.extend(lote.into_iter().map(|agente| agente.id));
        }
    }

    fn estado_do_item(&self, base: &BaseConhecimento, item: &KnowledgeItem) -> EstadoIndice {
        let grupo_inativo = self
            .store
            .knowledge()
            .get_group(self.org, item.group_id)
            .map(|grupo| grupo.status != Status::Active)
            .unwrap_or(true);
        if item.status != Status::Active || grupo_inativo {
            return EstadoIndice::Inativo;
        }
        let agentes = match self
            .store
            .knowledge()
            .agents_from_group(self.org, item.group_id)
        {
            Ok(agentes) => agentes,
            Err(_) => return EstadoIndice::Erro,
        };
        if agentes.is_empty() {
            return EstadoIndice::NaoVinculado;
        }
        let esperado = hash_origem(base.indice().identidade(), &item.title, &item.content);
        for agente in agentes {
            let Ok(mapa) = base.indice().itens_indexados(self.org, agente) else {
                return EstadoIndice::Erro;
            };
            if mapa.get(&item.id) != Some(&esperado) {
                return EstadoIndice::Pendente;
            }
        }
        EstadoIndice::Indexado
    }

    fn total_itens(&self, grupo: KnowledgeGroupId) -> usize {
        self.store
            .knowledge()
            .list_items(self.org, grupo, Page::with_limit(200))
            .map(|itens| itens.len())
            .unwrap_or(0)
    }

    fn resolver_agente(&self, texto: &str) -> Result<AgentId, String> {
        if uuid::Uuid::parse_str(texto).is_ok() {
            let id: AgentId = parse_id(texto)?;
            return self
                .store
                .agents()
                .get(self.org, id)
                .map(|agente| agente.id)
                .map_err(|erro| descrever(&erro));
        }
        self.store
            .agents()
            .by_slug(self.org, texto)
            .map(|agente| agente.id)
            .map_err(|erro| descrever(&erro))
    }

    fn saida_item(
        &self,
        item: &KnowledgeItem,
        json_saida: bool,
        rotulo: &str,
        aviso: Option<String>,
    ) -> ExitCode {
        let estado = self
            .base()
            .ok()
            .map(|base| self.estado_do_item(&base, item))
            .unwrap_or(EstadoIndice::Erro);
        if json_saida {
            imprimir(&json!({
                "versao_relatorio": 1,
                "item": json_item(item, estado),
                "index": aviso,
            }));
        } else {
            println!(
                "{rotulo}: {}  {}\n  grupo: {}\n  versão: {}\n  índice: {}",
                item.id,
                item.title,
                item.group_id,
                item.version,
                estado.como_texto()
            );
            if let Some(aviso) = aviso {
                println!("[aviso] índice não atualizado: {aviso}");
            }
        }
        ExitCode::SUCCESS
    }
}

fn ler_conteudo(inline: Option<&str>, arquivo: Option<&std::path::Path>) -> Result<String, String> {
    let texto = match (inline, arquivo) {
        (Some(texto), None) => texto.to_owned(),
        (None, Some(caminho)) => std::fs::read_to_string(caminho)
            .map_err(|erro| format!("não consegui ler {}: {erro}", caminho.display()))?,
        (None, None) => {
            return Err("informe o conteúdo com --content ou --content-file".to_owned());
        }
        (Some(_), Some(_)) => unreachable!("o clap impede --content com --content-file"),
    };
    let caracteres = texto.trim().chars().count();
    if caracteres == 0 {
        return Err("o conteúdo do item não pode ficar vazio".to_owned());
    }
    if caracteres > LIMITE_CONTEUDO {
        return Err(format!(
            "o conteúdo do item passa de {LIMITE_CONTEUDO} caracteres (vieram {caracteres})"
        ));
    }
    Ok(texto.trim().to_owned())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|erro| format!("não consegui iniciar o runtime: {erro}"))
}

fn imprimir(valor: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(valor).unwrap_or_else(|_| valor.to_string())
    );
}

fn json_grupo(grupo: &KnowledgeGroup) -> Value {
    json!({
        "id": grupo.id.to_string(),
        "name": grupo.name,
        "description": grupo.description,
        "status": grupo.status.as_text(),
        "version": grupo.version,
        "created_at": grupo.created_at,
        "updated_at": grupo.updated_at,
    })
}

fn json_indexacao(relatorio: &poph_rag::RelatorioIndexacao) -> Value {
    json!({
        "items_new": relatorio.itens_novos,
        "items_changed": relatorio.itens_alterados,
        "items_unchanged": relatorio.itens_inalterados,
        "items_removed": relatorio.itens_removidos,
        "embeddings": relatorio.embeddings_gerados,
        "chunks": relatorio.trechos_gravados,
    })
}

fn json_item(item: &KnowledgeItem, estado: EstadoIndice) -> Value {
    json!({
        "id": item.id.to_string(),
        "group_id": item.group_id.to_string(),
        "title": item.title,
        "content": item.content,
        "tags": item.tags,
        "position": item.position,
        "status": item.status.as_text(),
        "index": estado.como_texto(),
        "version": item.version,
        "created_at": item.created_at,
        "updated_at": item.updated_at,
    })
}
