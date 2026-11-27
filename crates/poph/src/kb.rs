use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Subcommand};
use poph_agent::KnowledgeBase;
use poph_rag::chunks::source_hash;
use poph_store::{
    KnowledgeGroup, KnowledgeGroupChange, KnowledgeItem, KnowledgeItemChange, NewKnowledgeGroup,
    NewKnowledgeItem, Page, Status, Store,
};
use poph_types::{AgentId, HarnessConfig, KnowledgeGroupId, KnowledgeItemId, OrgId};
use serde_json::{Value, json};

use crate::config;
use crate::support::{
    describe, failure, parse_id, parse_status, status_in_portuguese, validate_text,
};

const CONTENT_LIMIT: usize = 1000;

#[derive(Debug, Subcommand)]
pub enum KbAction {
    #[command(about = "Grupos de conhecimento: criar, listar, mostrar, alterar e excluir")]
    #[command(subcommand)]
    Group(GroupAction),
    #[command(about = "Itens da base: criar, listar, mostrar, alterar e excluir")]
    #[command(subcommand)]
    Item(ItemAction),
    #[command(about = "Liga um grupo a um agente (o agente passa a consultar os itens dele)")]
    Link(LinkArgs),
    #[command(about = "Desliga um grupo de um agente")]
    Unlink(LinkArgs),
    #[command(about = "Reconstrói o índice de um agente (ou de todos os agentes da org)")]
    Reindex(ReindexArgs),
    #[command(about = "Consulta a base como o agente faria (busca híbrida)")]
    Search(SearchArgs),
    #[command(about = "Mostra o estado do índice dos itens (geral ou de um grupo)")]
    Status(StatusArgs),
}

#[derive(Debug, Subcommand)]
pub enum GroupAction {
    #[command(about = "Cria um grupo")]
    Create(CreateGroupArgs),
    #[command(about = "Lista os grupos da org")]
    List(ListGroupsArgs),
    #[command(about = "Mostra um grupo e os agentes que o usam")]
    Show(GroupRef),
    #[command(about = "Altera um grupo com `--if-match`")]
    Update(UpdateGroupArgs),
    #[command(about = "Exclui o grupo e os itens dele. Exige `--force`")]
    Delete(DeleteGroupArgs),
}

#[derive(Debug, Subcommand)]
pub enum ItemAction {
    #[command(about = "Cria um item (conteúdo até 1000 caracteres)")]
    Create(CreateItemArgs),
    #[command(about = "Lista os itens de um grupo, com o estado no índice")]
    List(ListItemsArgs),
    #[command(about = "Mostra um item, com a fonte e o estado no índice")]
    Show(RefItem),
    #[command(about = "Altera um item com `--if-match`")]
    Update(UpdateItemArgs),
    #[command(about = "Exclui o item")]
    Delete(DeleteItemArgs),
}

#[derive(Debug, Args)]
pub struct CreateGroupArgs {
    #[arg(long, help = "Nome (até 120 caracteres; não repete na org)")]
    pub name: String,
    #[arg(long, default_value = "", help = "Descrição curta")]
    pub description: String,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListGroupsArgs {
    #[arg(long, default_value_t = 50, help = "Quantos grupos trazer (1 a 200)")]
    pub limit: u32,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct GroupRef {
    #[arg(value_name = "GRUPO", help = "Id (UUID v7) do grupo")]
    pub group: String,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UpdateGroupArgs {
    #[arg(value_name = "GRUPO", help = "Id (UUID v7) do grupo")]
    pub group: String,
    #[arg(long, help = "Versão lida com `show` (`If-Match`). Obrigatória")]
    pub if_match: u64,
    #[arg(long, help = "Novo nome")]
    pub name: Option<String>,
    #[arg(long, help = "Nova descrição")]
    pub description: Option<String>,
    #[arg(
        long,
        help = "Nova situação: `active`, `inactive` ou `archived` (inativo sai do índice, D-05)"
    )]
    pub status: Option<String>,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteGroupArgs {
    #[arg(value_name = "GRUPO", help = "Id (UUID v7) do grupo")]
    pub group: String,
    #[arg(long, help = "Confirma: os itens do grupo saem junto")]
    pub force: bool,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CreateItemArgs {
    #[arg(long, help = "Grupo dono (id UUID v7)")]
    pub group: String,
    #[arg(long, help = "Título (vira a fonte mostrada ao modelo)")]
    pub title: String,
    #[arg(long, conflicts_with = "content_file", help = "Conteúdo do item")]
    pub content: Option<String>,
    #[arg(long, value_name = "ARQUIVO", help = "Arquivo com o conteúdo do item")]
    pub content_file: Option<PathBuf>,
    #[arg(long, value_delimiter = ',', help = "Etiquetas separadas por vírgula")]
    pub tag: Vec<String>,
    #[arg(long, default_value_t = 0, help = "Posição dentro do grupo")]
    pub position: i64,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListItemsArgs {
    #[arg(long, help = "Grupo dono (id UUID v7)")]
    pub group: String,
    #[arg(long, default_value_t = 50, help = "Quantos itens trazer (1 a 200)")]
    pub limit: u32,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RefItem {
    #[arg(value_name = "ITEM", help = "Id (UUID v7) do item")]
    pub item: String,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UpdateItemArgs {
    #[arg(value_name = "ITEM", help = "Id (UUID v7) do item")]
    pub item: String,
    #[arg(long, help = "Versão lida com `show` (`If-Match`). Obrigatória")]
    pub if_match: u64,
    #[arg(long, help = "Move para outro grupo")]
    pub group: Option<String>,
    #[arg(long, help = "Novo título")]
    pub title: Option<String>,
    #[arg(long, conflicts_with = "content_file", help = "Novo conteúdo")]
    pub content: Option<String>,
    #[arg(long, value_name = "ARQUIVO", help = "Arquivo com o novo conteúdo")]
    pub content_file: Option<PathBuf>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Novas etiquetas separadas por vírgula (substitui as anteriores)"
    )]
    pub tag: Vec<String>,
    #[arg(long, help = "Nova posição")]
    pub position: Option<i64>,
    #[arg(long, help = "Nova situação: `active`, `inactive` ou `archived`")]
    pub status: Option<String>,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DeleteItemArgs {
    #[arg(value_name = "ITEM", help = "Id (UUID v7) do item")]
    pub item: String,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LinkArgs {
    #[arg(value_name = "GRUPO", help = "Grupo (id UUID v7)")]
    pub group: String,
    #[arg(long, help = "Agente (id UUID v7 ou slug)")]
    pub agent: String,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ReindexArgs {
    #[arg(
        long,
        help = "Agente (id UUID v7 ou slug). Sem ele, reindexa todos os agentes da org"
    )]
    pub agent: Option<String>,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    #[arg(value_name = "CONSULTA", help = "Texto da consulta")]
    pub query: String,
    #[arg(long, help = "Agente cuja base será consultada (id UUID v7 ou slug)")]
    pub agent: String,
    #[arg(long, help = "Quantos trechos trazer (padrão: o top-k do agente)")]
    pub top_k: Option<usize>,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(long, help = "Limita a um grupo (id UUID v7)")]
    pub group: Option<String>,
    #[arg(long, help = "Saída em JSON")]
    pub json: bool,
}

pub fn run(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    offline: bool,
    org: Option<&str>,
    acao: KbAction,
) -> ExitCode {
    if !offline {
        return failure(
            "use `--offline`: a base de conhecimento é mantida direto no banco enquanto o modo \
             com servidor não existe",
        );
    }
    let (store, org, configuracao) = match open_base(flag, environment, org) {
        Ok(trio) => trio,
        Err(error) => return failure(error),
    };
    let context = Context {
        store,
        org,
        configuracao,
    };
    match acao {
        KbAction::Group(GroupAction::Create(args)) => context.create_group(args),
        KbAction::Group(GroupAction::List(args)) => context.list_groups(args),
        KbAction::Group(GroupAction::Show(args)) => context.show_group(args),
        KbAction::Group(GroupAction::Update(args)) => context.update_group(args),
        KbAction::Group(GroupAction::Delete(args)) => context.delete_group(args),
        KbAction::Item(ItemAction::Create(args)) => context.criar_item(args),
        KbAction::Item(ItemAction::List(args)) => context.listar_itens(args),
        KbAction::Item(ItemAction::Show(args)) => context.mostrar_item(args),
        KbAction::Item(ItemAction::Update(args)) => context.atualizar_item(args),
        KbAction::Item(ItemAction::Delete(args)) => context.excluir_item(args),
        KbAction::Link(args) => context.vincular(args, true),
        KbAction::Unlink(args) => context.vincular(args, false),
        KbAction::Reindex(args) => context.reindexar(args),
        KbAction::Search(args) => context.search(args),
        KbAction::Status(args) => context.status(args),
    }
}

fn open_base(
    flag: Option<PathBuf>,
    environment: &config::Environment,
    org: Option<&str>,
) -> Result<(Arc<Store>, OrgId, HarnessConfig), String> {
    let texto =
        org.ok_or_else(|| "informe --org com o id (UUID v7) ou o slug da org".to_owned())?;
    let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file = config::resolve_file(flag.as_deref(), environment, &dir);
    let configuracao =
        config::load(file.as_deref(), environment).map_err(|error| format!("{error}"))?;
    let store = Arc::new(
        Store::open(&configuracao)
            .map_err(|error| format!("não consegui abrir o banco: {error}"))?,
    );
    let org = crate::support::resolve_org(&store, texto)?;
    Ok((store, org, configuracao))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexState {
    Indexed,
    Pending,
    Inactive,
    NotLinked,
    Error,
}

impl IndexState {
    fn as_text(self) -> &'static str {
        match self {
            IndexState::Indexed => "indexado",
            IndexState::Pending => "pendente",
            IndexState::Inactive => "inativo",
            IndexState::NotLinked => "nao_vinculado",
            IndexState::Error => "erro",
        }
    }
}

struct Context {
    store: Arc<Store>,
    org: OrgId,
    configuracao: HarnessConfig,
}

impl Context {
    fn create_group(&self, args: CreateGroupArgs) -> ExitCode {
        let nome = match validate_text("nome do grupo", &args.name, 120) {
            Ok(nome) => nome,
            Err(error) => return failure(error),
        };
        match self.store.knowledge().create_group(
            self.org,
            NewKnowledgeGroup {
                name: nome,
                description: args.description.trim().to_owned(),
            },
        ) {
            Ok(group) => {
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "group": group_json(&group),
                    }));
                } else {
                    println!(
                        "grupo criado: {}  {}  (versão {})\nuse `poph kb item create --group {}` \
                         para adicionar itens",
                        group.id, group.name, group.version, group.id
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn list_groups(&self, args: ListGroupsArgs) -> ExitCode {
        let page = Page::with_limit(args.limit);
        match self.store.knowledge().list_groups(self.org, page) {
            Ok(groups) => {
                if args.json {
                    let items: Vec<Value> = groups.iter().map(group_json).collect();
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "org_id": self.org.to_string(),
                        "total": items.len(),
                        "groups": items,
                    }));
                } else if groups.is_empty() {
                    println!("Nenhum grupo de conhecimento na org.");
                } else {
                    for group in &groups {
                        println!(
                            "{}  {:<9} {}",
                            group.id,
                            status_in_portuguese(group.status),
                            group.name
                        );
                    }
                    println!("\n{} grupo(s).", groups.len());
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn show_group(&self, args: GroupRef) -> ExitCode {
        let id: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        match self.store.knowledge().get_group(self.org, id) {
            Ok(group) => {
                let agents = self
                    .store
                    .knowledge()
                    .agents_from_group(self.org, id)
                    .unwrap_or_default();
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "group": group_json(&group),
                        "agents": agents.iter().map(AgentId::to_string).collect::<Vec<_>>(),
                    }));
                } else {
                    println!("id:          {}", group.id);
                    println!("nome:        {}", group.name);
                    println!("descrição:   {}", group.description);
                    println!("situação:    {}", status_in_portuguese(group.status));
                    println!("versão:      {}", group.version);
                    println!("itens:       {}", self.total_items(id));
                    println!(
                        "agentes:     {}",
                        if agents.is_empty() {
                            "nenhum (use `poph kb link`)".to_owned()
                        } else {
                            agents
                                .iter()
                                .map(AgentId::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn update_group(&self, args: UpdateGroupArgs) -> ExitCode {
        let id: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        let status = match args.status.as_deref().map(parse_status).transpose() {
            Ok(status) => status,
            Err(error) => return failure(error),
        };
        let change = KnowledgeGroupChange {
            name: args.name,
            description: args.description,
            status,
        };
        match self
            .store
            .knowledge()
            .update_group(self.org, id, args.if_match, change)
        {
            Ok(group) => {
                let notice = self.sync_group(id);
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "group": group_json(&group),
                        "index": notice,
                    }));
                } else {
                    println!(
                        "grupo atualizado: {}  {}  (versão {})",
                        group.id, group.name, group.version
                    );
                    if let Some(notice) = notice {
                        println!("[aviso] índice não atualizado: {notice}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn delete_group(&self, args: DeleteGroupArgs) -> ExitCode {
        let id: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        if !args.force {
            return failure(format!(
                "o grupo {id} pode ter itens e agentes ligados; confirme com --force"
            ));
        }
        let agents = self
            .store
            .knowledge()
            .agents_from_group(self.org, id)
            .unwrap_or_default();
        match self.store.knowledge().delete_group(self.org, id) {
            Ok(()) => {
                let notice = self.reindex_agents(&agents);
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "deleted": id.to_string(),
                        "index": notice,
                    }));
                } else {
                    println!("grupo {id} excluído (itens e vínculos saíram junto)");
                    if let Some(notice) = notice {
                        println!("[aviso] índice não atualizado: {notice}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn criar_item(&self, args: CreateItemArgs) -> ExitCode {
        let group: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        let content = match read_content(args.content.as_deref(), args.content_file.as_deref()) {
            Ok(content) => content,
            Err(error) => return failure(error),
        };
        let title = match validate_text("título do item", &args.title, 200) {
            Ok(title) => title,
            Err(error) => return failure(error),
        };
        let new_item = NewKnowledgeItem {
            group_id: group,
            title,
            content,
            tags: args.tag,
            position: args.position,
        };
        match self.store.knowledge().create_item(self.org, new_item) {
            Ok(item) => {
                let estado = self.sync_group(group);
                self.output_item(&item, args.json, "item criado", estado)
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn listar_itens(&self, args: ListItemsArgs) -> ExitCode {
        let group: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        let page = Page::with_limit(args.limit);
        match self.store.knowledge().list_items(self.org, group, page) {
            Ok(items) => {
                if args.json {
                    let base = self.base().ok();
                    let list: Vec<Value> = items
                        .iter()
                        .map(|item| {
                            let estado = base
                                .as_ref()
                                .map(|base| self.item_state(base, item))
                                .unwrap_or(IndexState::Error);
                            json_item(item, estado)
                        })
                        .collect();
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "group_id": group.to_string(),
                        "total": list.len(),
                        "items": list,
                    }));
                } else if items.is_empty() {
                    println!("Nenhum item neste grupo.");
                } else {
                    let base = self.base().ok();
                    for item in &items {
                        let estado = base
                            .as_ref()
                            .map(|base| self.item_state(base, item))
                            .unwrap_or(IndexState::Error);
                        println!(
                            "{}  {:<9} {:<14} {}",
                            item.id,
                            status_in_portuguese(item.status),
                            estado.as_text(),
                            item.title
                        );
                    }
                    println!("\n{} item(s).", items.len());
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn mostrar_item(&self, args: RefItem) -> ExitCode {
        let id: KnowledgeItemId = match parse_id(&args.item) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        match self.store.knowledge().get_item(self.org, id) {
            Ok(item) => {
                let base = self.base().ok();
                let estado = base
                    .as_ref()
                    .map(|base| self.item_state(base, &item))
                    .unwrap_or(IndexState::Error);
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "item": json_item(&item, estado),
                    }));
                } else {
                    println!("id:        {}", item.id);
                    println!("grupo:     {}", item.group_id);
                    println!("título:    {}", item.title);
                    println!("situação:  {}", status_in_portuguese(item.status));
                    println!("índice:    {}", estado.as_text());
                    println!("versão:    {}", item.version);
                    println!("tags:      {}", item.tags.join(", "));
                    println!("\n{}\n", item.content);
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn atualizar_item(&self, args: UpdateItemArgs) -> ExitCode {
        let id: KnowledgeItemId = match parse_id(&args.item) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        let group = match args
            .group
            .as_deref()
            .map(parse_id::<KnowledgeGroupId>)
            .transpose()
        {
            Ok(group) => group,
            Err(error) => return failure(error),
        };
        let content = match (args.content.as_deref(), args.content_file.as_deref()) {
            (None, None) => None,
            (inline, file) => match read_content(inline, file) {
                Ok(content) => Some(content),
                Err(error) => return failure(error),
            },
        };
        let status = match args.status.as_deref().map(parse_status).transpose() {
            Ok(status) => status,
            Err(error) => return failure(error),
        };
        let tags = (!args.tag.is_empty()).then_some(args.tag);
        let change = KnowledgeItemChange {
            group_id: group,
            title: args.title,
            content,
            tags,
            position: args.position,
            status,
        };
        match self
            .store
            .knowledge()
            .update_item(self.org, id, args.if_match, change)
        {
            Ok(item) => {
                let estado = self.sync_group(item.group_id);
                self.output_item(&item, args.json, "item atualizado", estado)
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn excluir_item(&self, args: DeleteItemArgs) -> ExitCode {
        let id: KnowledgeItemId = match parse_id(&args.item) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        let group = self
            .store
            .knowledge()
            .get_item(self.org, id)
            .map(|item| item.group_id)
            .ok();
        match self.store.knowledge().delete_item(self.org, id) {
            Ok(()) => {
                let notice = group.and_then(|group| self.sync_group(group));
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "deleted": id.to_string(),
                        "index": notice,
                    }));
                } else {
                    println!("item {id} excluído");
                    if let Some(notice) = notice {
                        println!("[aviso] índice não atualizado: {notice}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn vincular(&self, args: LinkArgs, ligar: bool) -> ExitCode {
        let group: KnowledgeGroupId = match parse_id(&args.group) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        let agente = match self.resolve_agent(&args.agent) {
            Ok(agente) => agente,
            Err(error) => return failure(error),
        };
        let resultado = if ligar {
            self.store
                .knowledge()
                .link_agent(self.org, agente, group)
                .map(|()| true)
        } else {
            self.store.knowledge().unlink_agent(self.org, agente, group)
        };
        match resultado {
            Ok(changed) => {
                let notice = self.reindex_agents(&[agente]);
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "agent": agente.to_string(),
                        "group": group.to_string(),
                        "linked": ligar,
                        "changed": changed,
                        "index": notice,
                    }));
                } else {
                    println!(
                        "{}: grupo {} {} agente {}",
                        if ligar { "vínculo" } else { "desvínculo" },
                        group,
                        if ligar { "ligado ao" } else { "desligado do" },
                        agente
                    );
                    if let Some(notice) = notice {
                        println!("[aviso] índice não atualizado: {notice}");
                    }
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(describe(&error)),
        }
    }

    fn reindexar(&self, args: ReindexArgs) -> ExitCode {
        let base = match self.base() {
            Ok(base) => base,
            Err(error) => return failure(error),
        };
        let agents = match &args.agent {
            Some(reference) => match self.resolve_agent(reference) {
                Ok(agente) => vec![agente],
                Err(error) => return failure(error),
            },
            None => match self.all_agents() {
                Ok(agents) => agents,
                Err(error) => return failure(error),
            },
        };
        let mut reports = Vec::new();
        for agente in agents {
            match self.sync(&base, agente) {
                Ok(report) => reports.push(json!({
                    "agent": agente.to_string(),
                    "indexed": index_json(&report),
                })),
                Err(error) => return failure(error),
            }
        }
        if args.json {
            print_json(&json!({
                "versao_relatorio": 1,
                "agents": reports,
            }));
        } else if reports.is_empty() {
            println!("Nenhum agente na org para reindexar.");
        } else {
            for report in &reports {
                println!(
                    "agente {}: novos {}, alterados {}, removidos {}, trechos {}",
                    report["agent"].as_str().unwrap_or("?"),
                    report["indexed"]["itens_novos"],
                    report["indexed"]["itens_alterados"],
                    report["indexed"]["itens_removidos"],
                    report["indexed"]["trechos_gravados"],
                );
            }
        }
        ExitCode::SUCCESS
    }

    fn search(&self, args: SearchArgs) -> ExitCode {
        let base = match self.base() {
            Ok(base) => base,
            Err(error) => return failure(error),
        };
        let agente = match self.resolve_agent(&args.agent) {
            Ok(agente) => agente,
            Err(error) => return failure(error),
        };
        let agent_record = match self.store.agents().get(self.org, agente) {
            Ok(agente) => agente,
            Err(error) => return failure(describe(&error)),
        };
        let top_k = args
            .top_k
            .unwrap_or_else(|| usize::try_from(agent_record.rag_top_k).unwrap_or(3).max(1));
        let runtime = match runtime() {
            Ok(runtime) => runtime,
            Err(error) => return failure(error),
        };
        let chunks = runtime.block_on(async {
            use poph_agent::{KnowledgeSearcher, SearchParameters};
            let searcher: &dyn KnowledgeSearcher = base.as_ref();
            searcher
                .search(
                    self.org,
                    agente,
                    args.query.clone(),
                    SearchParameters {
                        top_k,
                        cutoff: agent_record.rag_cutoff as f32,
                    },
                )
                .await
        });
        match chunks {
            Ok(chunks) => {
                if args.json {
                    print_json(&json!({
                        "versao_relatorio": 1,
                        "agent": agente.to_string(),
                        "query": args.query,
                        "total": chunks.len(),
                        "chunks": chunks,
                    }));
                } else if chunks.is_empty() {
                    println!("Nenhum trecho acima do corte de relevância.");
                } else {
                    for (index, chunk) in chunks.iter().enumerate() {
                        println!(
                            "[{}] {:.3}  fonte: {} ({})",
                            index + 1,
                            chunk.similarity,
                            chunk.title,
                            chunk.group
                        );
                        println!("    {}\n", chunk.texto.replace('\n', " "));
                    }
                }
                ExitCode::SUCCESS
            }
            Err(error) => failure(format!("a busca falhou: {error}")),
        }
    }

    fn status(&self, args: StatusArgs) -> ExitCode {
        let groups = match &args.group {
            Some(texto) => match parse_id::<KnowledgeGroupId>(texto) {
                Ok(id) => match self.store.knowledge().get_group(self.org, id) {
                    Ok(group) => vec![group],
                    Err(error) => return failure(describe(&error)),
                },
                Err(error) => return failure(error),
            },
            None => match self
                .store
                .knowledge()
                .list_groups(self.org, Page::with_limit(200))
            {
                Ok(groups) => groups,
                Err(error) => return failure(describe(&error)),
            },
        };
        let base = self.base().ok();
        let mut lines = Vec::new();
        for group in &groups {
            let items = self
                .store
                .knowledge()
                .list_items(self.org, group.id, Page::with_limit(200))
                .unwrap_or_default();
            for item in items {
                let estado = base
                    .as_ref()
                    .map(|base| self.item_state(base, &item))
                    .unwrap_or(IndexState::Error);
                lines.push(json!({
                    "item": item.id.to_string(),
                    "group": group.name,
                    "title": item.title,
                    "status": item.status.as_text(),
                    "index": estado.as_text(),
                }));
            }
        }
        if args.json {
            print_json(&json!({
                "versao_relatorio": 1,
                "groups": groups.len(),
                "items": lines,
            }));
        } else if lines.is_empty() {
            println!("Nenhum item na base.");
        } else {
            for line in &lines {
                println!(
                    "{}  {:<11} {:<14} {} ({})",
                    line["item"].as_str().unwrap_or("?"),
                    line["index"].as_str().unwrap_or("?"),
                    line["status"].as_str().unwrap_or("?"),
                    line["title"].as_str().unwrap_or("?"),
                    line["group"].as_str().unwrap_or("?"),
                );
            }
            println!("\n{} item(s).", lines.len());
        }
        ExitCode::SUCCESS
    }

    fn base(&self) -> Result<Arc<KnowledgeBase>, String> {
        crate::rag::open_queued_knowledge_base(
            &self.configuracao,
            Arc::clone(&self.store),
            self.org,
        )
    }

    fn sync(&self, base: &KnowledgeBase, agente: AgentId) -> Result<poph_rag::IndexReport, String> {
        let runtime = runtime()?;
        runtime
            .block_on(base.synchronise_agent(self.org, agente))
            .map_err(|error| format!("não consegui reindexar o agente {agente}: {error}"))
    }

    fn sync_group(&self, group: KnowledgeGroupId) -> Option<String> {
        let agents = self
            .store
            .knowledge()
            .agents_from_group(self.org, group)
            .ok()?;
        self.reindex_agents(&agents)
    }

    fn reindex_agents(&self, agents: &[AgentId]) -> Option<String> {
        if agents.is_empty() {
            return None;
        }
        let base = match self.base() {
            Ok(base) => base,
            Err(error) => return Some(format!("{error}; rode `poph kb reindex` depois")),
        };
        for agente in agents {
            if let Err(error) = self.sync(&base, *agente) {
                return Some(format!("{error}; rode `poph kb reindex` depois"));
            }
        }
        None
    }

    fn all_agents(&self) -> Result<Vec<AgentId>, String> {
        let mut agents = Vec::new();
        let mut page = Page::with_limit(200);
        loop {
            let batch = self
                .store
                .agents()
                .list(self.org, page)
                .map_err(|error| describe(&error))?;
            if batch.is_empty() {
                return Ok(agents);
            }
            page.after = batch.last().map(|agente| agente.id.as_uuid());
            agents.extend(batch.into_iter().map(|agente| agente.id));
        }
    }

    fn item_state(&self, base: &KnowledgeBase, item: &KnowledgeItem) -> IndexState {
        let inactive_group = self
            .store
            .knowledge()
            .get_group(self.org, item.group_id)
            .map(|group| group.status != Status::Active)
            .unwrap_or(true);
        if item.status != Status::Active || inactive_group {
            return IndexState::Inactive;
        }
        let agents = match self
            .store
            .knowledge()
            .agents_from_group(self.org, item.group_id)
        {
            Ok(agents) => agents,
            Err(_) => return IndexState::Error,
        };
        if agents.is_empty() {
            return IndexState::NotLinked;
        }
        let expected = source_hash(base.index().identity(), &item.title, &item.content);
        for agente in agents {
            let Ok(map) = base.index().indexed_items(self.org, agente) else {
                return IndexState::Error;
            };
            if map.get(&item.id) != Some(&expected) {
                return IndexState::Pending;
            }
        }
        IndexState::Indexed
    }

    fn total_items(&self, group: KnowledgeGroupId) -> usize {
        self.store
            .knowledge()
            .list_items(self.org, group, Page::with_limit(200))
            .map(|items| items.len())
            .unwrap_or(0)
    }

    fn resolve_agent(&self, texto: &str) -> Result<AgentId, String> {
        if uuid::Uuid::parse_str(texto).is_ok() {
            let id: AgentId = parse_id(texto)?;
            return self
                .store
                .agents()
                .get(self.org, id)
                .map(|agente| agente.id)
                .map_err(|error| describe(&error));
        }
        self.store
            .agents()
            .by_slug(self.org, texto)
            .map(|agente| agente.id)
            .map_err(|error| describe(&error))
    }

    fn output_item(
        &self,
        item: &KnowledgeItem,
        json_output: bool,
        label: &str,
        notice: Option<String>,
    ) -> ExitCode {
        let estado = self
            .base()
            .ok()
            .map(|base| self.item_state(&base, item))
            .unwrap_or(IndexState::Error);
        if json_output {
            print_json(&json!({
                "versao_relatorio": 1,
                "item": json_item(item, estado),
                "index": notice,
            }));
        } else {
            println!(
                "{label}: {}  {}\n  grupo: {}\n  versão: {}\n  índice: {}",
                item.id,
                item.title,
                item.group_id,
                item.version,
                estado.as_text()
            );
            if let Some(notice) = notice {
                println!("[aviso] índice não atualizado: {notice}");
            }
        }
        ExitCode::SUCCESS
    }
}

fn read_content(inline: Option<&str>, file: Option<&std::path::Path>) -> Result<String, String> {
    let texto = match (inline, file) {
        (Some(texto), None) => texto.to_owned(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|error| format!("não consegui ler {}: {error}", path.display()))?,
        (None, None) => {
            return Err("informe o conteúdo com --content ou --content-file".to_owned());
        }
        (Some(_), Some(_)) => unreachable!("o clap impede --content com --content-file"),
    };
    let char_count = texto.trim().chars().count();
    if char_count == 0 {
        return Err("o conteúdo do item não pode ficar vazio".to_owned());
    }
    if char_count > CONTENT_LIMIT {
        return Err(format!(
            "o conteúdo do item passa de {CONTENT_LIMIT} caracteres (vieram {char_count})"
        ));
    }
    Ok(texto.trim().to_owned())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("não consegui iniciar o runtime: {error}"))
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

fn group_json(group: &KnowledgeGroup) -> Value {
    json!({
        "id": group.id.to_string(),
        "name": group.name,
        "description": group.description,
        "status": group.status.as_text(),
        "version": group.version,
        "created_at": group.created_at,
        "updated_at": group.updated_at,
    })
}

fn index_json(report: &poph_rag::IndexReport) -> Value {
    json!({
        "items_new": report.new_items,
        "items_changed": report.changed_items,
        "items_unchanged": report.unchanged_items,
        "items_removed": report.removed_items,
        "embeddings": report.generated_embeddings,
        "chunks": report.stored_chunks,
    })
}

fn json_item(item: &KnowledgeItem, estado: IndexState) -> Value {
    json!({
        "id": item.id.to_string(),
        "group_id": item.group_id.to_string(),
        "title": item.title,
        "content": item.content,
        "tags": item.tags,
        "position": item.position,
        "status": item.status.as_text(),
        "index": estado.as_text(),
        "version": item.version,
        "created_at": item.created_at,
        "updated_at": item.updated_at,
    })
}
