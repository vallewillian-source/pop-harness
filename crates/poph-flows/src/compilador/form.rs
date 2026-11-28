use std::collections::{BTreeMap, BTreeSet};

use poph_types::{ActionActor, FieldKind};
use starlark::codemap::Span;
use starlark::syntax::AstModule;
use starlark::syntax::ast::{
    Argument, AstAssignTarget, AstExpr, AstIdent, AstLiteral, AstNoPayload, AstStmt, CallArgsP,
    Clause, DefP, Expr, Parameter, Stmt,
};

use crate::diagnostics::{Code, Diagnostic};
use crate::ir::{
    Acao, Campo, Decisao, Def, Destination, Effect, Etapa, Fim, Fluxo, Gatilho, Ir, Rule,
    SCHEMA_VERSAO,
};
use crate::language::is_allowed_name;

type ArgsAst<'a> = &'a CallArgsP<AstNoPayload>;

pub const MAX_NOS: usize = 60;
const MAX_TRIGGER_TEXT: usize = 400;
const MAX_INSTRUCTION_TEXT: usize = 1000;
const ID_MIN: usize = 2;
const ID_MAX: usize = 64;

const ARGS_FLOW: &[&str] = &["id", "nome", "descricao", "inicio", "gatilho"];
const ARGS_TRIGGER: &[&str] = &["quando", "nao_quando", "exemplos"];
const ARGS_STEP: &[&str] = &[
    "id",
    "nome",
    "instrucao",
    "atores",
    "campos",
    "acoes",
    "ao_entrar",
];
const ARGS_FIELD: &[&str] = &["id", "rotulo", "tipo", "obrigatorio", "opcoes", "ajuda"];
const ARGS_ACTION: &[&str] = &["id", "rotulo", "vai_para", "exige", "ator", "efeitos"];
const ARGS_END: &[&str] = &["id", "resultado", "encerra_atendimento"];
const ARGS_DECISION: &[&str] = &["id", "se", "entao", "senao"];
const ARGS_NOTIFY: &[&str] = &["para", "mensagem"];
const ARGS_CALENDAR: &[&str] = &["inicio", "fim", "titulo", "local", "executor"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pos {
    line: u32,
    column: u32,
}

struct Item<'a> {
    nome: Option<String>,
    expr: &'a AstExpr,
}

#[derive(Clone)]
enum RawDestination {
    Target { id: String, pos: Pos },
    Fim { fim: Fim, pos: Pos },
    Invalid,
}

#[derive(Clone)]
enum RawRule {
    Leaf {
        rule: Rule,
        pos: Pos,
    },
    Funcao {
        nome: String,
        pos: Pos,
    },
    Group {
        rule: Rule,
        children: Vec<RawRule>,
        pos: Pos,
    },
    Invalid,
}

#[derive(Clone)]
struct RawAction {
    acao: Acao,
    destination: RawDestination,
}

#[derive(Clone)]
struct CollectedStep {
    etapa: Etapa,
    actions: Vec<RawAction>,
    pos: Pos,
    fields_pos: BTreeMap<String, Pos>,
}

#[derive(Clone)]
struct CollectedDecision {
    id: String,
    rule: RawRule,
    then: RawDestination,
    otherwise: RawDestination,
    pos: Pos,
}

struct CollectedEffect {
    effect: Effect,
    texts: Vec<(String, Pos)>,
    etapa: String,
    on_enter: bool,
}

#[derive(Default, Clone)]
struct FieldUses {
    required: BTreeSet<String>,
    read: BTreeSet<String>,
}

struct Form<'a> {
    fonte: &'a str,
    module: &'a AstModule,
    diags: Vec<Diagnostic>,
    fluxo: Option<Fluxo>,
    flow_pos: Pos,
    gatilho: Option<Gatilho>,
    start: Option<(String, Pos)>,
    stages: Vec<CollectedStep>,
    idx_stages: BTreeMap<String, usize>,
    decisions: Vec<CollectedDecision>,
    idx_decisions: BTreeMap<String, usize>,
    ends: Vec<Fim>,
    ends_pos: BTreeMap<String, Pos>,
    defs: Vec<Def>,
    pos_defs: BTreeMap<String, Pos>,
    effects: Vec<CollectedEffect>,
    uses: BTreeMap<String, FieldUses>,
}

pub(crate) fn compile(fonte: &str, module: &AstModule) -> (Option<Ir>, Vec<Diagnostic>) {
    let um = Pos { line: 1, column: 1 };
    let mut form = Form {
        fonte,
        module,
        diags: Vec::new(),
        fluxo: None,
        flow_pos: um,
        gatilho: None,
        start: None,
        stages: Vec::new(),
        idx_stages: BTreeMap::new(),
        decisions: Vec::new(),
        idx_decisions: BTreeMap::new(),
        ends: Vec::new(),
        ends_pos: BTreeMap::new(),
        defs: Vec::new(),
        pos_defs: BTreeMap::new(),
        effects: Vec::new(),
        uses: BTreeMap::new(),
    };
    form.collect_top();
    let ir = form.close();
    (ir, form.diags)
}

impl<'a> Form<'a> {
    fn pos(&self, span: Span) -> Pos {
        let resolvido = self.module.file_span(span).resolve_span();
        Pos {
            line: resolvido.begin.line as u32 + 1,
            column: resolvido.begin.column as u32 + 1,
        }
    }

    fn error(&mut self, code: Code, mensagem: impl Into<String>, pos: Pos) {
        self.diags
            .push(Diagnostic::error(code, mensagem, pos.line, pos.column));
    }

    fn warning(&mut self, code: Code, mensagem: impl Into<String>, pos: Pos) {
        self.diags
            .push(Diagnostic::warning(code, mensagem, pos.line, pos.column));
    }

    fn has_error(&self) -> bool {
        self.diags.iter().any(Diagnostic::is_error)
    }

    fn collect_top(&mut self) {
        self.collect_list(self.module.statement());
    }

    fn collect_list(&mut self, stmt: &'a AstStmt) {
        let items: Vec<&'a AstStmt> = match &stmt.node {
            Stmt::Statements(items) => items.iter().collect(),
            _ => vec![stmt],
        };
        for item in items {
            self.collect_item(item);
        }
    }

    fn collect_item(&mut self, item: &'a AstStmt) {
        let pos = self.pos(item.span);
        match &item.node {
            Stmt::Def(def) => self.collect_def(def, item, pos),
            Stmt::Expression(expr) => match &expr.node {
                Expr::Call(target, args) => {
                    let nome = match &target.node {
                        Expr::Identifier(ident) => ident.node.ident.clone(),
                        _ => {
                            self.error(
                                Code::InvalidTopLevel,
                                "No topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`; isto é uma chamada de atributo.",
                                pos,
                            );
                            return;
                        }
                    };
                    match nome.as_str() {
                        "fluxo" => self.collect_flow(args, pos),
                        "etapa" => self.collect_step(args, pos),
                        "decisao" => self.collect_decision(args, pos),
                        other => self.error(
                            Code::InvalidTopLevel,
                            format!(
                                "`{other}(...)` não é declaração de fluxo: no topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`."
                            ),
                            pos,
                        ),
                    }
                }
                _ => self.error(
                    Code::InvalidTopLevel,
                    "No topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`. Tire esta expressão solta.",
                    pos,
                ),
            },
            Stmt::Assign(_) => self.error(
                Code::InvalidTopLevel,
                "No topo do arquivo não existe atribuição (`x = ...`): só `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
            Stmt::AssignModify(..) => self.error(
                Code::InvalidTopLevel,
                "No topo do arquivo não existe atribuição (`x += ...`): só `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
            Stmt::For(_) => self.error(
                Code::Dialect,
                "Laço (`for`) não existe no topo de um fluxo: só `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
            Stmt::If(..) | Stmt::IfElse(..) => self.error(
                Code::Dialect,
                "Condição (`if`) não existe no topo de um fluxo: a bifurcação do atendimento é `decisao(...)`.",
                pos,
            ),
            Stmt::Load(_) => self.error(
                Code::Dialect,
                "`load` não faz parte da linguagem de fluxos: um fluxo não importa outro arquivo.",
                pos,
            ),
            Stmt::Return(_) | Stmt::Break | Stmt::Continue | Stmt::Pass | Stmt::Statements(_) => self.error(
                Code::InvalidTopLevel,
                "No topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
        }
    }

    fn items_of(&mut self, args: ArgsAst<'a>, location: &str, pos: Pos) -> Vec<Item<'a>> {
        let mut items = Vec::new();
        for arg in &args.args {
            match &arg.node {
                Argument::Positional(expr) => items.push(Item { nome: None, expr }),
                Argument::Named(nome, expr) => items.push(Item {
                    nome: Some(nome.node.clone()),
                    expr,
                }),
                Argument::Args(_) | Argument::KwArgs(_) => self.error(
                    Code::UnknownArgument,
                    format!(
                        "{location} não aceita `*`/`**` na chamada: escreva os argumentos um a um."
                    ),
                    pos,
                ),
            }
        }
        items
    }

    fn collect_flow(&mut self, args: ArgsAst<'a>, pos: Pos) {
        if self.fluxo.is_some() {
            self.error(
                Code::MissingFlow,
                "O arquivo declara `fluxo(...)` mais de uma vez. Um arquivo é um fluxo: deixe só um.",
                pos,
            );
            return;
        }
        self.flow_pos = pos;
        let items = self.items_of(args, "`fluxo`", pos);
        let mut reader = Reader::new("`fluxo`", pos, ARGS_FLOW, items);
        let id = reader.texto(self, "id");
        let nome = reader.texto(self, "nome");
        let descricao = reader.texto(self, "descricao");
        let start = reader.texto(self, "inicio");
        let gatilho = reader.item(self, "gatilho");
        let pos_id = reader.pos_of("id").unwrap_or(pos);
        let start_pos = reader.pos_of("inicio");
        reader.close(self);

        if let Some(id) = &id
            && !valid_id(id)
        {
            self.error(
                Code::InvalidId,
                format!(
                    "O id `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres, por exemplo `agendar_visita`."
                ),
                pos_id,
            );
        }
        if let Some(start) = &start {
            self.start = Some((start.clone(), start_pos.unwrap_or(pos)));
        }
        if let Some(expr) = gatilho {
            let trigger_pos = self.pos(expr.span);
            self.collect_trigger(expr, trigger_pos);
        }
        self.fluxo = Some(Fluxo {
            id: id.unwrap_or_default(),
            nome: nome.unwrap_or_default(),
            descricao,
        });
    }

    fn collect_trigger(&mut self, expr: &'a AstExpr, pos: Pos) {
        let Some((nome, items)) = self.as_call(expr, "o `gatilho` do fluxo") else {
            return;
        };
        if nome != "gatilho" {
            self.error(
                Code::WrongType,
                format!("O `gatilho` do fluxo precisa ser `gatilho(quando = ..., nao_quando = ..., exemplos = [...])`, não `{nome}(...)`."),
                pos,
            );
            return;
        }
        let mut reader = Reader::new("`gatilho`", pos, ARGS_TRIGGER, items);
        let when_text = reader.texto(self, "quando");
        let not_when_text = reader.texto(self, "nao_quando");
        let examples_expr = reader.item(self, "exemplos");
        let when_pos = reader.pos_of("quando").unwrap_or(pos);
        let not_when_pos = reader.pos_of("nao_quando").unwrap_or(pos);
        let examples = match examples_expr {
            Some(expr) => self.list_text(expr, "`exemplos` do gatilho"),
            None => Vec::new(),
        };
        reader.close(self);

        for (chave, texto, text_pos) in [
            ("quando", when_text.as_deref(), when_pos),
            ("nao_quando", not_when_text.as_deref(), not_when_pos),
        ] {
            let Some(texto) = texto else { continue };
            if !variables(texto).is_empty() {
                self.error(
                    Code::ForbiddenInterpolation,
                    format!("`gatilho.{chave}` tem `{{...}}`, mas esse texto não interpola: o Jev lê o texto como está."),
                    text_pos,
                );
            }
            if texto.chars().count() > MAX_TRIGGER_TEXT {
                self.warning(
                    Code::LongText,
                    format!(
                        "`gatilho.{chave}` tem {} caracteres; o recomendado é até {MAX_TRIGGER_TEXT}.",
                        texto.chars().count()
                    ),
                    text_pos,
                );
            }
        }
        if examples.is_empty() {
            self.warning(
                Code::TriggerWithoutExamples,
                "O gatilho não tem `exemplos`: com 2 ou 3 falas típicas do cliente o Jev acerta mais.",
                pos,
            );
        }
        if not_when_text.is_none() {
            self.warning(
                Code::TriggerWithoutNegative,
                "O gatilho não tem `nao_quando`: diga em que situação o fluxo NÃO deve começar.",
                pos,
            );
        }
        self.gatilho = Some(Gatilho {
            when: when_text.unwrap_or_default(),
            not_when: not_when_text,
            examples,
        });
    }

    fn collect_step(&mut self, args: ArgsAst<'a>, pos: Pos) {
        let items = self.items_of(args, "`etapa`", pos);
        let mut reader = Reader::new("`etapa`", pos, ARGS_STEP, items);
        let id = reader.texto(self, "id");
        let nome = reader.texto(self, "nome");
        let instruction = reader.texto(self, "instrucao");
        let actors_expr = reader.item(self, "atores");
        let fields_expr = reader.item(self, "campos");
        let actions_expr = reader.item(self, "acoes");
        let on_enter_expr = reader.item(self, "ao_entrar");
        let pos_id = reader.pos_of("id").unwrap_or(pos);
        let instruction_pos = reader.pos_of("instrucao").unwrap_or(pos);
        reader.close(self);

        let id = id.unwrap_or_default();
        if !id.is_empty() {
            if !valid_id(&id) {
                self.error(
                    Code::InvalidId,
                    format!("O id de etapa `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos_id,
                );
            }
            if self.id_ocupado(&id) {
                self.error(
                    Code::DuplicateId,
                    format!("O id `{id}` está declarado mais de uma vez. Cada etapa e cada decisão tem id único no fluxo."),
                    pos_id,
                );
            }
        }
        if let Some(texto) = &instruction {
            if !variables(texto).is_empty() {
                self.error(
                    Code::ForbiddenInterpolation,
                    format!(
                        "A `instrucao` da etapa `{id}` tem `{{...}}`, mas instrução não interpola: o agente lê o texto como está. Use variável só em `mensagem_ao_cliente`, `notificar_whatsapp` e `sincronizar_agenda`."
                    ),
                    instruction_pos,
                );
            }
            if texto.chars().count() > MAX_INSTRUCTION_TEXT {
                self.warning(
                    Code::LongText,
                    format!(
                        "A `instrucao` da etapa `{id}` tem {} caracteres; o recomendado é até {MAX_INSTRUCTION_TEXT}, porque entra no prompt a cada turno.",
                        texto.chars().count()
                    ),
                    instruction_pos,
                );
            }
        }

        let mut acting_actors: Vec<ActionActor> = Vec::new();
        if let Some(expr) = actors_expr {
            let actors_pos = self.pos(expr.span);
            for actor in self.list_text(expr, &format!("`atores` da etapa `{id}`")) {
                match ActionActor::from_text(&actor) {
                    Ok(actor) => acting_actors.push(actor),
                    Err(_) => self.error(
                        Code::InvalidRuleOrActor,
                        format!("A etapa `{id}` tem o ator `{actor}`; use `agente` ou `atendente` (o cliente não é ator)."),
                        actors_pos,
                    ),
                }
            }
        }
        if acting_actors.is_empty() {
            acting_actors.push(ActionActor::Agent);
        }
        let mut unique: Vec<ActionActor> = Vec::new();
        for actor in acting_actors {
            if !unique.contains(&actor) {
                unique.push(actor);
            }
        }
        let acting_actors = unique;

        self.uses.insert(id.clone(), FieldUses::default());

        let (campos, fields_pos) = match fields_expr {
            Some(expr) => self.collect_fields(expr, &id),
            None => (Vec::new(), BTreeMap::new()),
        };
        let actions = match actions_expr {
            Some(expr) => self.collect_actions(expr, &id, &campos, &acting_actors),
            None => Vec::new(),
        };
        let mut on_enter = Vec::new();
        if let Some(expr) = on_enter_expr {
            let location = format!("`ao_entrar` da etapa `{id}`");
            for effect in self.collect_effects(expr, &location) {
                on_enter.push(effect.effect.clone());
                self.effects.push(CollectedEffect {
                    effect: effect.effect,
                    texts: effect.texts,
                    etapa: id.clone(),
                    on_enter: true,
                });
            }
        }

        if !actions.is_empty() && actions.iter().all(|a| a.acao.actor.is_some()) {
            for actor in &acting_actors {
                if actions.iter().any(|a| a.acao.actor == Some(*actor)) {
                    continue;
                }
                let actors_pos = actors_expr.map(|expr| self.pos(expr.span)).unwrap_or(pos);
                self.warning(
                    Code::ActorWithoutAction,
                    format!(
                        "A etapa `{id}` permite o `{}`, mas nenhuma ação é dele: na prática ele não tem o que fazer aqui.",
                        actor.as_text()
                    ),
                    actors_pos,
                );
            }
        }

        let index = self.stages.len();
        self.idx_stages.insert(id.clone(), index);
        self.stages.push(CollectedStep {
            etapa: Etapa {
                id,
                nome: nome.unwrap_or_default(),
                instruction,
                action_actors: acting_actors,
                campos,
                actions: Vec::new(),
                on_enter,
            },
            actions,
            pos,
            fields_pos,
        });
    }

    fn id_ocupado(&self, id: &str) -> bool {
        self.idx_stages.contains_key(id) || self.idx_decisions.contains_key(id)
    }

    fn collect_fields(
        &mut self,
        expr: &'a AstExpr,
        etapa: &str,
    ) -> (Vec<Campo>, BTreeMap<String, Pos>) {
        let mut campos = Vec::new();
        let mut positions = BTreeMap::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let location_list = format!("`campos` da etapa `{etapa}`");
        for item in self.list_items(expr, &location_list) {
            let pos = self.pos(item.span);
            let Some((nome, items)) =
                self.as_call(item, &format!("um item de `campos` da etapa `{etapa}`"))
            else {
                continue;
            };
            if nome != "campo" {
                self.error(
                    Code::WrongType,
                    format!(
                        "`campos` da etapa `{etapa}` aceita só `campo(...)`, não `{nome}(...)`."
                    ),
                    pos,
                );
                continue;
            }
            let mut reader = Reader::new("`campo`", pos, ARGS_FIELD, items);
            let id = reader.positional_text(self, 0);
            let label = reader.positional_text(self, 1);
            let kind_expr = reader.item(self, "tipo");
            let required_expr = reader.item(self, "obrigatorio");
            let options_expr = reader.item(self, "opcoes");
            let help = reader.texto(self, "ajuda");
            let pos_id = reader.pos_of("__pos0").unwrap_or(pos);
            let label_pos = reader.pos_of("__pos1").unwrap_or(pos);
            let kind_pos = reader.pos_of("tipo").unwrap_or(pos);
            let _required_pos = reader.pos_of("obrigatorio").unwrap_or(pos);
            let options_pos = reader.pos_of("opcoes").unwrap_or(pos);
            reader.close(self);

            let id = id.unwrap_or_default();
            if !id.is_empty() && !valid_id(&id) {
                self.error(
                    Code::InvalidId,
                    format!("O id do campo `{id}` na etapa `{etapa}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos_id,
                );
            }
            let mut repeated = false;
            if !id.is_empty() && !seen.insert(id.clone()) {
                self.error(
                    Code::DuplicateId,
                    format!("O campo `{id}` está declarado duas vezes na etapa `{etapa}`."),
                    pos_id,
                );
                repeated = true;
            }
            if label.is_none() {
                self.error(
                    Code::MissingArgument,
                    format!(
                        "O campo `{etapa}.{id}` não recebeu o rótulo (segundo argumento): `campo(\"{id}\", \"Rótulo\")`."
                    ),
                    label_pos,
                );
            }
            let tipo = match kind_expr {
                Some(expr) => {
                    let raw = self
                        .texto(expr, &format!("`tipo` do campo `{etapa}.{id}`"))
                        .unwrap_or_default();
                    match FieldKind::from_text(&raw) {
                        Ok(tipo) => tipo,
                        Err(_) => {
                            self.error(
                                Code::UnknownType,
                                format!(
                                    "O campo `{etapa}.{id}` tem tipo `{raw}`; use texto, numero, data, booleano, opcao ou anexo."
                                ),
                                kind_pos,
                            );
                            FieldKind::Text
                        }
                    }
                }
                None => FieldKind::Text,
            };
            let is_required = match required_expr {
                Some(expr) => self
                    .boolean(expr, &format!("`obrigatorio` do campo `{etapa}.{id}`"))
                    .unwrap_or(false),
                None => false,
            };
            let mut options = Vec::new();
            if let Some(expr) = options_expr {
                options = self.list_text(expr, &format!("`opcoes` do campo `{etapa}.{id}`"));
                let mut seen_options = BTreeSet::new();
                for option in &options {
                    if !seen_options.insert(option.clone()) {
                        self.error(
                            Code::InvalidOptions,
                            format!("O campo `{etapa}.{id}` repete a opção `{option}`."),
                            options_pos,
                        );
                    }
                }
            }
            if tipo == FieldKind::Option && options.len() < 2 {
                self.error(
                    Code::InvalidOptions,
                    format!(
                        "O campo `{etapa}.{id}` é `opcao` e precisa de pelo menos duas `opcoes`."
                    ),
                    if options_expr.is_some() {
                        options_pos
                    } else {
                        pos_id
                    },
                );
            }
            if tipo != FieldKind::Option && !options.is_empty() {
                self.error(
                    Code::InvalidOptions,
                    format!(
                        "O campo `{etapa}.{id}` tem `opcoes`, mas o tipo é `{}`: `opcoes` só vale com `tipo = \"opcao\"`.",
                        tipo.as_text()
                    ),
                    options_pos,
                );
            }
            if repeated {
                continue;
            }
            positions.insert(id.clone(), pos_id);
            campos.push(Campo {
                id,
                label: label.unwrap_or_default(),
                tipo,
                required: is_required,
                options,
                help,
            });
        }
        (campos, positions)
    }

    fn collect_actions(
        &mut self,
        expr: &'a AstExpr,
        etapa: &str,
        campos: &[Campo],
        actors: &[ActionActor],
    ) -> Vec<RawAction> {
        let mut actions = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let field_ids: BTreeSet<&str> = campos.iter().map(|c| c.id.as_str()).collect();
        let location_list = format!("`acoes` da etapa `{etapa}`");
        for item in self.list_items(expr, &location_list) {
            let pos = self.pos(item.span);
            let Some((nome, items)) =
                self.as_call(item, &format!("um item de `acoes` da etapa `{etapa}`"))
            else {
                continue;
            };
            if nome != "acao" {
                self.error(
                    Code::WrongType,
                    format!("`acoes` da etapa `{etapa}` aceita só `acao(...)`, não `{nome}(...)`."),
                    pos,
                );
                continue;
            }
            let mut reader = Reader::new("`acao`", pos, ARGS_ACTION, items);
            let id = reader.positional_text(self, 0);
            let label = reader.positional_text(self, 1);
            let goes_to = reader.item(self, "vai_para");
            let requires_expr = reader.item(self, "exige");
            let actor_expr = reader.item(self, "ator");
            let effects_expr = reader.item(self, "efeitos");
            let goes_to_pos = reader.pos_of("vai_para").unwrap_or(pos);
            let requires_pos = reader.pos_of("exige").unwrap_or(pos);
            let actor_pos = reader.pos_of("ator").unwrap_or(pos);
            reader.close(self);

            let id = id.unwrap_or_default();
            if !id.is_empty() && !valid_id(&id) {
                self.error(
                    Code::InvalidId,
                    format!("O id da ação `{id}` na etapa `{etapa}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos,
                );
            }
            if label.is_none() {
                self.error(
                    Code::MissingArgument,
                    format!(
                        "A ação `{etapa}.{id}` não recebeu o rótulo (segundo argumento): `acao(\"{id}\", \"Rótulo\", vai_para = ...)`."
                    ),
                    pos,
                );
            }
            let mut requires = Vec::new();
            if let Some(expr) = requires_expr {
                let requires_location = format!("`exige` da ação `{etapa}.{id}`");
                for campo in self.list_text(expr, &requires_location) {
                    if !field_ids.contains(campo.as_str()) {
                        self.error(
                            Code::UnknownRequirement,
                            format!("A ação `{etapa}.{id}` exige o campo `{campo}`, que não existe na etapa `{etapa}`."),
                            requires_pos,
                        );
                        continue;
                    }
                    if let Some(usage) = self.uses.get_mut(etapa) {
                        usage.required.insert(campo.clone());
                    }
                    requires.push(campo);
                }
            }
            let actor = match actor_expr {
                Some(expr) => {
                    let raw = self
                        .texto(expr, &format!("`ator` da ação `{etapa}.{id}`"))
                        .unwrap_or_default();
                    match ActionActor::from_text(&raw) {
                        Ok(actor) => {
                            if !actors.contains(&actor) {
                                self.error(
                                    Code::InvalidRuleOrActor,
                                    format!(
                                        "A ação `{etapa}.{id}` é do ator `{raw}`, que não está em `atores` da etapa `{etapa}`. Inclua `{raw}` em `atores` ou tire o `ator` da ação."
                                    ),
                                    actor_pos,
                                );
                            }
                            Some(actor)
                        }
                        Err(_) => {
                            self.error(
                                Code::InvalidRuleOrActor,
                                format!("A ação `{etapa}.{id}` tem `ator = \"{raw}\"`; use `agente` ou `atendente` (ou deixe vazio)."),
                                actor_pos,
                            );
                            None
                        }
                    }
                }
                None => None,
            };
            let destination = match goes_to {
                Some(expr) => self.destination(expr, &format!("a ação `{etapa}.{id}`")),
                None => {
                    self.error(
                        Code::MissingArgument,
                        format!("A ação `{etapa}.{id}` não recebeu `vai_para`: diga para qual etapa, decisão ou `fim(...)` ela leva."),
                        goes_to_pos,
                    );
                    RawDestination::Invalid
                }
            };
            let mut effects = Vec::new();
            if let Some(expr) = effects_expr {
                let effects_location = format!("`efeitos` da ação `{etapa}.{id}`");
                for effect in self.collect_effects(expr, &effects_location) {
                    effects.push(effect.effect.clone());
                    self.effects.push(CollectedEffect {
                        effect: effect.effect,
                        texts: effect.texts,
                        etapa: etapa.to_owned(),
                        on_enter: false,
                    });
                }
            }
            let mut follows = true;
            if !id.is_empty() && !seen.insert(id.clone()) {
                self.error(
                    Code::DuplicateId,
                    format!("A ação `{id}` está declarada duas vezes na etapa `{etapa}`."),
                    pos,
                );
                follows = false;
            }
            if !follows {
                continue;
            }
            actions.push(RawAction {
                acao: Acao {
                    id,
                    label: label.unwrap_or_default(),
                    requires,
                    actor,
                    goes_to: Destination::Etapa { id: String::new() },
                    effects,
                },
                destination,
            });
        }
        actions
    }

    fn collect_effects(&mut self, expr: &'a AstExpr, location: &str) -> Vec<ReadEffect> {
        let mut saida = Vec::new();
        for item in self.list_items(expr, location) {
            let pos = self.pos(item.span);
            let Some((nome, items)) = self.as_call(item, location) else {
                continue;
            };
            let mut texts: Vec<(String, Pos)> = Vec::new();
            let effect = match nome.as_str() {
                "mensagem_ao_cliente" => {
                    let mut reader = Reader::new("`mensagem_ao_cliente`", pos, &[], items);
                    let texto = reader.positional_text(self, 0);
                    let text_pos = reader.pos_of("__pos0").unwrap_or(pos);
                    reader.close(self);
                    let Some(texto) = texto else {
                        self.error(
                            Code::MissingArgument,
                            format!("{location} tem `mensagem_ao_cliente()` sem o texto: `mensagem_ao_cliente(\"Texto enviado ao cliente\")`."),
                            pos,
                        );
                        continue;
                    };
                    self.check_variable_format(&texto, location, text_pos);
                    texts.push((texto.clone(), text_pos));
                    Effect::MessageToCustomer { texto }
                }
                "notificar_whatsapp" => {
                    let mut reader = Reader::new("`notificar_whatsapp`", pos, ARGS_NOTIFY, items);
                    let para = reader.texto(self, "para");
                    let mensagem = reader.texto(self, "mensagem");
                    let to_pos = reader.pos_of("para").unwrap_or(pos);
                    let message_pos = reader.pos_of("mensagem").unwrap_or(pos);
                    reader.close(self);
                    let para = match para {
                        Some(para) => {
                            if para.len() < 10 || !para.chars().all(|c| c.is_ascii_digit()) {
                                self.error(
                                    Code::InvalidNumber,
                                    format!("O número `{para}` não serve: use dígitos com DDI, por exemplo `5521999990000`."),
                                    to_pos,
                                );
                            }
                            para
                        }
                        None => {
                            self.error(
                                Code::MissingArgument,
                                format!("{location} tem `notificar_whatsapp` sem `para`: indique o número que recebe a notificação."),
                                pos,
                            );
                            String::new()
                        }
                    };
                    let mensagem = match mensagem {
                        Some(mensagem) => {
                            self.check_variable_format(&mensagem, location, message_pos);
                            if variables(&mensagem).is_empty() {
                                self.warning(
                                    Code::NotificationWithoutVariable,
                                    format!("{location} notifica a equipe sem nenhuma variável: ela recebe sempre o mesmo texto, sem saber de qual atendimento se trata."),
                                    message_pos,
                                );
                            }
                            texts.push((mensagem.clone(), message_pos));
                            mensagem
                        }
                        None => {
                            self.error(
                                Code::MissingArgument,
                                format!("{location} tem `notificar_whatsapp` sem `mensagem`: escreva o texto da notificação."),
                                pos,
                            );
                            String::new()
                        }
                    };
                    Effect::NotifyWhatsApp { para, mensagem }
                }
                "encaminhar" => {
                    let mut reader = Reader::new("`encaminhar`", pos, &[], items);
                    let departamento = reader.positional_text(self, 0);
                    reader.close(self);
                    match departamento {
                        Some(departamento) => Effect::Encaminhar { departamento },
                        None => {
                            self.error(
                                Code::MissingArgument,
                                format!("{location} tem `encaminhar()` sem o departamento: `encaminhar(\"Logística\")`."),
                                pos,
                            );
                            continue;
                        }
                    }
                }
                "sincronizar_agenda" => {
                    let Some((effect, collected)) = self.collect_calendar(location, pos, items)
                    else {
                        continue;
                    };
                    texts = collected;
                    effect
                }
                other => {
                    self.error(
                        Code::InvalidTopLevel,
                        format!("{location} usa o efeito `{other}(...)`; use `mensagem_ao_cliente`, `notificar_whatsapp`, `encaminhar` ou `sincronizar_agenda`."),
                        pos,
                    );
                    continue;
                }
            };
            saida.push(ReadEffect { effect, texts });
        }
        saida
    }

    fn collect_calendar(
        &mut self,
        location: &str,
        pos: Pos,
        items: Vec<Item<'a>>,
    ) -> Option<(Effect, Vec<(String, Pos)>)> {
        let remove = matches!(
            items.iter().find(|item| item.nome.is_none()).map(|item| &item.expr.node),
            Some(Expr::Literal(AstLiteral::String(text))) if text.node == "remove"
        );
        let known: &'static [&'static str] = if remove { &[] } else { ARGS_CALENDAR };
        let mut reader = Reader::new("`sincronizar_agenda`", pos, known, items);
        let operation = reader.positional_text(self, 0);
        let operation_pos = reader.pos_of("__pos0").unwrap_or(pos);
        if remove {
            reader.refuse_leftovers(
                self,
                "`sincronizar_agenda(\"remove\")` não aceita outros argumentos: a remoção do evento usa só a operação.",
            );
            return Some((
                Effect::SyncCalendar {
                    operation: operation.unwrap_or_else(|| "remove".to_owned()),
                    start: None,
                    end: None,
                    title: None,
                    location: None,
                    executor: None,
                },
                Vec::new(),
            ));
        }
        let start = reader.texto(self, "inicio");
        let start_pos = reader.pos_of("inicio").unwrap_or(pos);
        let end = reader.texto(self, "fim");
        let end_pos = reader.pos_of("fim").unwrap_or(pos);
        let title = reader.texto(self, "titulo");
        let title_pos = reader.pos_of("titulo").unwrap_or(pos);
        let location_value = reader.texto(self, "local");
        let location_pos = reader.pos_of("local").unwrap_or(pos);
        let executor = reader.texto(self, "executor");
        let executor_pos = reader.pos_of("executor").unwrap_or(pos);
        reader.close(self);

        let mut texts: Vec<(String, Pos)> = Vec::new();
        for (value, value_pos) in [
            (&start, start_pos),
            (&end, end_pos),
            (&title, title_pos),
            (&location_value, location_pos),
            (&executor, executor_pos),
        ] {
            let Some(value) = value else { continue };
            self.check_variable_format(value, location, value_pos);
            if !value.trim().is_empty() {
                texts.push((value.clone(), value_pos));
            }
        }

        let operation = match operation {
            Some(operation) if matches!(operation.as_str(), "create" | "update" | "remove") => {
                operation
            }
            Some(operation) => {
                self.error(
                    Code::InvalidEffect,
                    format!(
                        "`sincronizar_agenda` não tem a operação `{operation}`: use `create`, `update` ou `remove`."
                    ),
                    operation_pos,
                );
                return None;
            }
            None => {
                self.error(
                    Code::MissingArgument,
                    format!(
                        "{location} tem `sincronizar_agenda` sem a operação: use `sincronizar_agenda(\"create\", inicio = \"...\")`, `\"update\"` ou `\"remove\"`."
                    ),
                    pos,
                );
                return None;
            }
        };

        self.check_calendar_window(
            &operation,
            start.as_deref(),
            start_pos,
            end.as_deref(),
            end_pos,
            location,
        );

        Some((
            Effect::SyncCalendar {
                operation,
                start: non_empty(start),
                end: non_empty(end),
                title: non_empty(title),
                location: non_empty(location_value),
                executor: non_empty(executor),
            },
            texts,
        ))
    }

    fn check_calendar_window(
        &mut self,
        operation: &str,
        start: Option<&str>,
        start_pos: Pos,
        end: Option<&str>,
        end_pos: Pos,
        location: &str,
    ) {
        use crate::engine::CalendarIssue;
        match crate::engine::calendar_window_issue(operation, start, end) {
            None => {}
            Some(CalendarIssue::MissingStart) => self.error(
                Code::MissingArgument,
                format!(
                    "{location} tem `sincronizar_agenda(\"{operation}\")` sem `inicio`: o evento precisa do começo em ISO 8601 com fuso, por exemplo `2027-01-10T18:00:00-03:00` ou `{{agendamento.data}}`."
                ),
                start_pos,
            ),
            Some(CalendarIssue::MalformedStart) => self.error(
                Code::InvalidEffect,
                format!(
                    "O `inicio` de `sincronizar_agenda` é `{}`, que não é ISO 8601 com fuso: use `AAAA-MM-DDTHH:MM:SS-03:00` (ou uma variável que resolva nesse formato).",
                    start.unwrap_or_default().trim()
                ),
                start_pos,
            ),
            Some(CalendarIssue::MalformedEnd) => self.error(
                Code::InvalidEffect,
                format!(
                    "O `fim` de `sincronizar_agenda` é `{}`, que não é ISO 8601 com fuso: use `AAAA-MM-DDTHH:MM:SS-03:00` (ou deixe vazio para o serviço usar a duração do fluxo).",
                    end.unwrap_or_default().trim()
                ),
                end_pos,
            ),
            Some(CalendarIssue::EndNotAfterStart) => self.error(
                Code::InvalidEffect,
                format!(
                    "O `fim` de `sincronizar_agenda` (`{}`) não é depois do `inicio` (`{}`): corrija a janela do evento.",
                    end.unwrap_or_default().trim(),
                    start.unwrap_or_default().trim()
                ),
                end_pos,
            ),
        }
    }

    fn collect_decision(&mut self, args: ArgsAst<'a>, pos: Pos) {
        let items = self.items_of(args, "`decisao`", pos);
        let mut reader = Reader::new("`decisao`", pos, ARGS_DECISION, items);
        let id = reader.texto(self, "id");
        let se_expr = reader.item(self, "se");
        let then_expr = reader.item(self, "entao");
        let otherwise_expr = reader.item(self, "senao");
        let pos_id = reader.pos_of("id").unwrap_or(pos);
        let pos_se = reader.pos_of("se").unwrap_or(pos);
        let then_pos = reader.pos_of("entao").unwrap_or(pos);
        let otherwise_pos = reader.pos_of("senao").unwrap_or(pos);
        reader.close(self);

        let id = id.unwrap_or_default();
        if !id.is_empty() {
            if !valid_id(&id) {
                self.error(
                    Code::InvalidId,
                    format!("O id de decisão `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos_id,
                );
            }
            if self.id_ocupado(&id) {
                self.error(
                    Code::DuplicateId,
                    format!("O id `{id}` está declarado mais de uma vez. Cada etapa e cada decisão tem id único no fluxo."),
                    pos_id,
                );
            }
        }
        let rule = match se_expr {
            Some(expr) => self.rule(expr, &format!("a decisão `{id}`")),
            None => {
                self.error(
                    Code::MissingArgument,
                    format!("A decisão `{id}` não recebeu `se`: escreva a regra, por exemplo `se = igual(\"coleta.periodo\", \"manhã\")`."),
                    pos_se,
                );
                RawRule::Invalid
            }
        };
        let then = match then_expr {
            Some(expr) => self.destination(expr, &format!("o ramo `entao` da decisão `{id}`")),
            None => {
                self.error(
                    Code::MissingArgument,
                    format!("A decisão `{id}` não recebeu `entao`: diga para onde vai quando a regra é verdadeira."),
                    then_pos,
                );
                RawDestination::Invalid
            }
        };
        let otherwise = match otherwise_expr {
            Some(expr) => self.destination(expr, &format!("o ramo `senao` da decisão `{id}`")),
            None => {
                self.error(
                    Code::MissingArgument,
                    format!("A decisão `{id}` não recebeu `senao`: toda decisão tem os dois ramos (`senao` é obrigatório)."),
                    otherwise_pos,
                );
                RawDestination::Invalid
            }
        };

        let index = self.decisions.len();
        self.idx_decisions.insert(id.clone(), index);
        self.decisions.push(CollectedDecision {
            id,
            rule,
            then,
            otherwise,
            pos,
        });
    }

    fn rule(&mut self, expr: &'a AstExpr, location: &str) -> RawRule {
        let pos = self.pos(expr.span);
        let Some((nome, items)) = self.as_call(expr, &format!("a regra de {location}")) else {
            return RawRule::Invalid;
        };
        match nome.as_str() {
            "igual" | "diferente" | "maior" | "menor" | "contem" => {
                let mut reader = Reader::new(&format!("`{nome}`"), pos, &[], items);
                let campo = reader.positional_text(self, 0);
                let value = reader.positional_text(self, 1);
                let field_pos = reader.pos_of("__pos0").unwrap_or(pos);
                let value_pos = reader.pos_of("__pos1").unwrap_or(pos);
                reader.close(self);
                let without_field = campo.is_none();
                let (Some(campo), Some(value)) = (campo, value) else {
                    let error_pos = if without_field { pos } else { value_pos };
                    self.error(
                        Code::MalformedCombinator,
                        format!("{location} usa `{nome}` sem o campo e o valor: `{nome}(\"etapa.campo\", \"valor\")`."),
                        error_pos,
                    );
                    return RawRule::Invalid;
                };
                let rule = match nome.as_str() {
                    "igual" => Rule::Igual { campo, value },
                    "diferente" => Rule::Diferente { campo, value },
                    "maior" => Rule::Maior { campo, value },
                    "menor" => Rule::Menor { campo, value },
                    _ => Rule::Contem { campo, value },
                };
                RawRule::Leaf {
                    rule,
                    pos: field_pos,
                }
            }
            "vazio" => {
                let mut reader = Reader::new("`vazio`", pos, &[], items);
                let campo = reader.positional_text(self, 0);
                let field_pos = reader.pos_of("__pos0").unwrap_or(pos);
                reader.close(self);
                let Some(campo) = campo else {
                    self.error(
                        Code::MalformedCombinator,
                        format!("{location} usa `vazio` sem o campo: `vazio(\"etapa.campo\")`."),
                        pos,
                    );
                    return RawRule::Invalid;
                };
                RawRule::Leaf {
                    rule: Rule::Vazio { campo },
                    pos: field_pos,
                }
            }
            "funcao" => {
                let mut reader = Reader::new("`funcao`", pos, &[], items);
                let def_name = reader.positional_text(self, 0);
                reader.close(self);
                let Some(def_name) = def_name else {
                    self.error(
                        Code::InvalidRuleOrActor,
                        format!("{location} usa `funcao()` sem o nome: `funcao(\"minha_regra\")`."),
                        pos,
                    );
                    return RawRule::Invalid;
                };
                RawRule::Funcao {
                    nome: def_name,
                    pos,
                }
            }
            "e" | "ou" | "nao" => {
                let mut posicionais = Vec::new();
                for item in &items {
                    if item.nome.is_some() {
                        let p = self.pos(item.expr.span);
                        self.error(
                            Code::MalformedCombinator,
                            format!("{location} usa `{nome}` com argumento nomeado; `{nome}` recebe os combinadores em sequência."),
                            p,
                        );
                        continue;
                    }
                    posicionais.push(item.expr);
                }
                let valid = if nome == "nao" {
                    posicionais.len() == 1
                } else {
                    posicionais.len() >= 2
                };
                if !valid {
                    let expected = if nome == "nao" {
                        "exatamente 1 argumento".to_owned()
                    } else {
                        "2 ou mais argumentos".to_owned()
                    };
                    self.error(
                        Code::MalformedCombinator,
                        format!(
                            "{location} usa `{nome}` com {} argumento(s); `{nome}` recebe {expected}.",
                            posicionais.len()
                        ),
                        pos,
                    );
                    return RawRule::Invalid;
                }
                let mut children = Vec::new();
                for child in posicionais {
                    children.push(self.rule(child, location));
                }
                let rule = match nome.as_str() {
                    "e" => Rule::E { args: Vec::new() },
                    "ou" => Rule::Ou { args: Vec::new() },
                    _ => Rule::Nao { args: Vec::new() },
                };
                RawRule::Group {
                    rule,
                    children,
                    pos,
                }
            }
            other => {
                self.error(
                    Code::InvalidRuleOrActor,
                    format!(
                        "{location} tem `se = {other}(...)`, que não é combinador nem `funcao(\"nome\")`. Use `igual`, `diferente`, `maior`, `menor`, `contem`, `vazio`, `e`, `ou`, `nao` ou `funcao`."
                    ),
                    pos,
                );
                RawRule::Invalid
            }
        }
    }

    fn destination(&mut self, expr: &'a AstExpr, location: &str) -> RawDestination {
        let pos = self.pos(expr.span);
        match &expr.node {
            Expr::Literal(AstLiteral::String(texto)) => RawDestination::Target {
                id: texto.node.clone(),
                pos,
            },
            Expr::Call(target, args) => {
                let nome = match &target.node {
                    Expr::Identifier(ident) => ident.node.ident.clone(),
                    _ => String::new(),
                };
                if nome != "fim" {
                    self.error(
                        Code::WrongType,
                        format!("{location} precisa apontar para o id de uma etapa, de uma decisão ou para `fim(\"id\")`, não para `{nome}(...)`."),
                        pos,
                    );
                    return RawDestination::Invalid;
                }
                let items = self.items_of(args, "`fim`", pos);
                let mut reader = Reader::new("`fim`", pos, ARGS_END, items);
                let id = reader.positional_text(self, 0);
                let resultado = reader.texto(self, "resultado");
                let end_expr = reader.item(self, "encerra_atendimento");
                let pos_id = reader.pos_of("__pos0").unwrap_or(pos);
                reader.close(self);
                let Some(id) = id else {
                    self.error(
                        Code::MissingArgument,
                        format!(
                            "{location} usa `fim()` sem id: `fim(\"concluido\", resultado = \"...\")`."
                        ),
                        pos,
                    );
                    return RawDestination::Invalid;
                };
                if !valid_id(&id) {
                    self.error(
                        Code::InvalidId,
                        format!("O id do fim `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                        pos_id,
                    );
                }
                let ends_service = match end_expr {
                    Some(expr) => self
                        .boolean(expr, &format!("`encerra_atendimento` do fim `{id}`"))
                        .unwrap_or(false),
                    None => false,
                };
                RawDestination::Fim {
                    fim: Fim {
                        id,
                        resultado,
                        ends_service,
                    },
                    pos,
                }
            }
            _ => {
                self.error(
                    Code::WrongType,
                    format!("{location} precisa apontar para o id de uma etapa, de uma decisão ou para `fim(\"id\")`, não para {}.", descreve(expr)),
                    pos,
                );
                RawDestination::Invalid
            }
        }
    }

    fn collect_def(&mut self, def: &'a DefP<AstNoPayload>, item: &'a AstStmt, pos: Pos) {
        let nome = def.name.node.ident.clone();
        if is_allowed_name(&nome) {
            self.error(
                Code::InvalidDef,
                format!("`{nome}` é nome reservado da linguagem de fluxos; escolha outro nome para a função de regra."),
                pos,
            );
            return;
        }
        if self.pos_defs.contains_key(&nome) {
            self.error(
                Code::InvalidDef,
                format!("Existe mais de uma `def {nome}`: cada função de regra tem nome único."),
                pos,
            );
            return;
        }
        let params: Vec<String> = def
            .params
            .iter()
            .filter_map(|parametro| parametro.ident().map(|i| i.node.ident.clone()))
            .collect();
        let only_data = def.params.len() == 1
            && def
                .params
                .first()
                .map(|parametro| matches!(parametro.node, Parameter::Normal(..)))
                .unwrap_or(false)
            && params == vec!["dados".to_owned()];
        if !only_data {
            let escrito = params.join(", ");
            self.error(
                Code::InvalidDef,
                format!("`def {nome}({escrito})` precisa receber um único argumento chamado `dados`: `def {nome}(dados):`."),
                pos,
            );
        }
        let start = item.span.begin().get() as usize;
        let fim = (def.body.span.end().get() as usize).max(item.span.end().get() as usize);
        let fonte = self
            .fonte
            .get(start..fim)
            .map(|excerpt| excerpt.trim_end().to_owned())
            .unwrap_or_default();
        self.pos_defs.insert(nome.clone(), pos);
        self.defs.push(Def { nome, fonte });
    }

    fn as_call(&mut self, expr: &'a AstExpr, location: &str) -> Option<(String, Vec<Item<'a>>)> {
        match &expr.node {
            Expr::Call(target, args) => {
                let nome = match &target.node {
                    Expr::Identifier(ident) => ident.node.ident.clone(),
                    _ => {
                        let pos = self.pos(expr.span);
                        self.error(
                            Code::InvalidTopLevel,
                            format!("{location} precisa ser a chamada de uma declaração da linguagem, não uma chamada de atributo."),
                            pos,
                        );
                        return None;
                    }
                };
                let pos = self.pos(expr.span);
                let items = self.items_of(args, &format!("`{nome}`"), pos);
                Some((nome, items))
            }
            _ => {
                let pos = self.pos(expr.span);
                self.error(
                    Code::WrongType,
                    format!(
                        "{location} precisa ser uma chamada de declaração, não {}.",
                        descreve(expr)
                    ),
                    pos,
                );
                None
            }
        }
    }

    fn list_items(&mut self, expr: &'a AstExpr, location: &str) -> Vec<&'a AstExpr> {
        match &expr.node {
            Expr::List(items) => items.iter().collect(),
            Expr::Tuple(items) => items.iter().collect(),
            _ => {
                let pos = self.pos(expr.span);
                self.error(
                    Code::WrongType,
                    format!(
                        "{location} precisa ser uma lista `[...]`, não {}.",
                        descreve(expr)
                    ),
                    pos,
                );
                Vec::new()
            }
        }
    }

    fn list_text(&mut self, expr: &'a AstExpr, location: &str) -> Vec<String> {
        let mut saida = Vec::new();
        for item in self.list_items(expr, location) {
            if let Some(texto) = self.texto(item, location) {
                saida.push(texto);
            }
        }
        saida
    }

    fn texto(&mut self, expr: &'a AstExpr, location: &str) -> Option<String> {
        if let Expr::Literal(AstLiteral::String(texto)) = &expr.node {
            return Some(texto.node.clone());
        }
        let pos = self.pos(expr.span);
        let mensagem = if expr_calculada(expr) {
            format!(
                "{location} precisa ser um valor literal (texto entre aspas): calcule o valor antes de escrever o fluxo."
            )
        } else {
            format!(
                "{location} precisa ser um texto entre aspas, não {}.",
                descreve(expr)
            )
        };
        self.error(Code::WrongType, mensagem, pos);
        None
    }

    fn boolean(&mut self, expr: &'a AstExpr, location: &str) -> Option<bool> {
        if let Expr::Identifier(ident) = &expr.node {
            match ident.node.ident.as_str() {
                "True" => return Some(true),
                "False" => return Some(false),
                _ => {}
            }
        }
        let pos = self.pos(expr.span);
        self.error(
            Code::WrongType,
            format!(
                "{location} precisa ser `True` ou `False`, não {}.",
                descreve(expr)
            ),
            pos,
        );
        None
    }

    fn close(&mut self) -> Option<Ir> {
        self.check_names();

        let Some(fluxo) = self.fluxo.clone() else {
            self.error(
                Code::MissingFlow,
                "O arquivo não declara `fluxo(...)`. Um arquivo é um fluxo: comece por `fluxo(id = ..., nome = ..., inicio = ..., gatilho = gatilho(...))`.",
                self.flow_pos,
            );
            return None;
        };

        let mut start = String::new();
        match self.start.clone() {
            Some((id, pos)) => {
                if self.idx_stages.contains_key(&id) {
                    start = id;
                } else {
                    self.error(
                        Code::UnknownDestination,
                        format!("`inicio = \"{id}\"` não é uma etapa declarada. Use o id de uma `etapa(...)` do arquivo."),
                        pos,
                    );
                }
            }
            None => self.error(
                Code::MissingArgument,
                "`fluxo` não recebeu `inicio`: indique o id da etapa em que o atendimento começa.",
                self.flow_pos,
            ),
        }

        let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();

        let collected_steps = self.stages.clone();
        let mut stages: Vec<Etapa> = Vec::with_capacity(collected_steps.len());
        for collected in collected_steps {
            let id = collected.etapa.id.clone();
            let mut etapa = collected.etapa;
            let mut actions = Vec::with_capacity(collected.actions.len());
            for raw_action in collected.actions {
                let location = format!("a ação `{id}.{}`", raw_action.acao.id);
                let destination = self.resolve(raw_action.destination, &location);
                if let Some(target) = destination.is_open() {
                    graph.entry(id.clone()).or_default().push(target.to_owned());
                }
                let mut acao = raw_action.acao;
                acao.goes_to = destination;
                actions.push(acao);
            }
            etapa.actions = actions;
            stages.push(etapa);
        }

        let collected_decisions = self.decisions.clone();
        let mut decisions: Vec<Decisao> = Vec::with_capacity(collected_decisions.len());
        for collected in &collected_decisions {
            let id = collected.id.clone();
            let then = self.resolve(
                clone_raw(&collected.then),
                &format!("o ramo `entao` da decisão `{id}`"),
            );
            let otherwise = self.resolve(
                clone_raw(&collected.otherwise),
                &format!("o ramo `senao` da decisão `{id}`"),
            );
            for destination in [&then, &otherwise] {
                if let Some(target) = destination.is_open() {
                    graph.entry(id.clone()).or_default().push(target.to_owned());
                }
            }
            let se = self.close_rule(&collected.rule, &format!("a decisão `{id}`"));
            decisions.push(Decisao {
                id,
                se,
                then_branch: then,
                else_branch: otherwise,
            });
        }

        self.check_cycle(&graph);
        self.check_reach(&graph, &start, &stages, &collected_decisions);
        self.check_field_uses(&stages);
        self.check_def_uses(&decisions);
        self.check_effects(&stages);
        self.check_node_cap();

        if self.has_error() {
            return None;
        }

        Some(Ir {
            schema_versao: SCHEMA_VERSAO,
            fluxo,
            gatilho: self.gatilho.clone().unwrap_or(Gatilho {
                when: String::new(),
                not_when: None,
                examples: Vec::new(),
            }),
            start,
            stages,
            decisions,
            ends: self.ends.clone(),
            defs: self.defs.clone(),
        })
    }

    fn resolve(&mut self, raw_dest: RawDestination, location: &str) -> Destination {
        match raw_dest {
            RawDestination::Target { id, pos } => {
                if self.idx_stages.contains_key(&id) {
                    Destination::Etapa { id }
                } else if self.idx_decisions.contains_key(&id) {
                    Destination::Decisao { id }
                } else {
                    self.error(
                        Code::UnknownDestination,
                        format!("{location} aponta para `{id}`, que não é etapa nem decisão declarada. Use o id de uma `etapa(...)` ou de uma `decisao(...)`."),
                        pos,
                    );
                    Destination::Etapa { id }
                }
            }
            RawDestination::Fim { fim, pos } => {
                match self.ends_pos.get(&fim.id).copied() {
                    Some(previous_pos) => {
                        let previous = self.ends.iter().find(|f| f.id == fim.id).cloned();
                        if previous.as_ref() != Some(&fim) {
                            self.error(
                                Code::DivergentEnd,
                                format!(
                                    "O fim `{}` aparece com argumentos diferentes aqui e na linha {}: use o mesmo `fim(...)` nos dois lugares.",
                                    fim.id, previous_pos.line
                                ),
                                pos,
                            );
                        }
                    }
                    None => {
                        self.ends_pos.insert(fim.id.clone(), pos);
                        self.ends.push(fim.clone());
                    }
                }
                Destination::Fim {
                    id: fim.id,
                    resultado: fim.resultado,
                    ends_service: fim.ends_service,
                }
            }
            RawDestination::Invalid => Destination::Etapa { id: String::new() },
        }
    }

    fn close_rule(&mut self, raw_action: &RawRule, place: &str) -> Rule {
        match raw_action {
            RawRule::Leaf { rule, pos } => {
                if let Some(campo) = field_of_rule(rule) {
                    self.check_rule_field(&campo, rule, place, *pos);
                }
                rule.clone()
            }
            RawRule::Funcao { nome, pos } => {
                if !self.pos_defs.contains_key(nome) {
                    self.error(
                        Code::InvalidRuleOrActor,
                        format!("{place} usa `funcao(\"{nome}\")`, mas não existe `def {nome}(dados)` no arquivo."),
                        *pos,
                    );
                }
                Rule::Funcao { nome: nome.clone() }
            }
            RawRule::Group {
                rule,
                children,
                pos,
            } => {
                self.check_contradiction(rule, children, place, *pos);
                let mut args = Vec::new();
                for child in children {
                    args.push(self.close_rule(child, place));
                }
                match rule {
                    Rule::E { .. } => Rule::E { args },
                    Rule::Ou { .. } => Rule::Ou { args },
                    _ => Rule::Nao { args },
                }
            }
            RawRule::Invalid => Rule::Vazio {
                campo: String::new(),
            },
        }
    }

    fn check_rule_field(&mut self, campo: &str, rule: &Rule, location: &str, pos: Pos) {
        let Some((etapa, nome)) = campo.split_once('.') else {
            self.error(
                Code::MalformedCombinator,
                format!(
                    "{location} compara `{campo}`; use `etapa.campo`, por exemplo `coleta.endereco`."
                ),
                pos,
            );
            return;
        };
        if !valid_id(etapa) || !valid_id(nome) {
            self.error(
                Code::MalformedCombinator,
                format!("{location} compara `{campo}`; use `etapa.campo`, com ids em minúsculas (por exemplo `coleta.endereco`)."),
                pos,
            );
            return;
        }
        let Some(index) = self.idx_stages.get(etapa).copied() else {
            self.error(
                Code::UnknownField,
                format!("{location} compara `{campo}`, mas a etapa `{etapa}` não existe."),
                pos,
            );
            return;
        };
        let field_info = self.stages[index]
            .etapa
            .campos
            .iter()
            .find(|c| c.id == nome)
            .cloned();
        let Some(field_info) = field_info else {
            self.error(
                Code::UnknownField,
                format!(
                    "{location} compara `{campo}`, mas a etapa `{etapa}` não tem o campo `{nome}`."
                ),
                pos,
            );
            return;
        };
        if let Some(usage) = self.uses.get_mut(etapa) {
            usage.read.insert(nome.to_owned());
        }
        match rule {
            Rule::Igual { value, .. } | Rule::Diferente { value, .. } => {
                if !field_info.options.is_empty() && !field_info.options.iter().any(|o| o == value)
                {
                    self.warning(
                        Code::OutsideOptions,
                        format!(
                            "{location} compara `{campo}` com `{value}`, que não é uma das opções ({}): o teste nunca casa.",
                            field_info.options.join(", ")
                        ),
                        pos,
                    );
                }
            }
            Rule::Maior { .. } | Rule::Menor { .. } if field_info.tipo != FieldKind::Number => {
                self.warning(
                    Code::TextComparison,
                    format!(
                        "{location} usa `maior`/`menor` com `{campo}`, mas o campo é `{}`: mude o tipo para `numero` ou use `igual`.",
                        field_info.tipo.as_text()
                    ),
                    pos,
                );
            }
            _ => {}
        }
    }

    fn check_contradiction(&mut self, rule: &Rule, children: &[RawRule], location: &str, pos: Pos) {
        let leaves: Vec<&Rule> = children
            .iter()
            .filter_map(|child| match child {
                RawRule::Leaf { rule, .. } => Some(rule),
                _ => None,
            })
            .collect();
        let eh_e = matches!(rule, Rule::E { .. });
        let eh_ou = matches!(rule, Rule::Ou { .. });
        let mut warnings: Vec<String> = Vec::new();
        for (i, a) in leaves.iter().enumerate() {
            for b in leaves.iter().skip(i + 1) {
                let (field_a, value_a, kind_a) = leaf_key(a);
                let (field_b, value_b, kind_b) = leaf_key(b);
                if eh_e
                    && field_a.is_some()
                    && field_a == field_b
                    && value_a == value_b
                    && matches!(
                        (kind_a, kind_b),
                        ("igual", "diferente") | ("diferente", "igual")
                    )
                {
                    warnings.push(format!(
                        "{location} não pode ser verdadeira com `{campo}` igual e diferente de `{value}` ao mesmo tempo.",
                        campo = field_a.unwrap_or(""),
                        value = value_a.unwrap_or("")
                    ));
                }
                if eh_ou && a == b {
                    warnings.push(format!(
                        "{location} repete o mesmo teste duas vezes na mesma disjunção (`ou`)."
                    ));
                }
            }
        }
        for mensagem in warnings {
            self.warning(Code::ContradictoryCondition, mensagem, pos);
        }
    }

    fn check_cycle(&mut self, graph: &BTreeMap<String, Vec<String>>) {
        let mut estado: BTreeMap<String, u8> = BTreeMap::new();
        let mut path: Vec<String> = Vec::new();
        let mut cycle: Option<Vec<String>> = None;
        let roots: Vec<String> = self.idx_decisions.keys().cloned().collect();
        for root in &roots {
            if cycle.is_some() {
                break;
            }
            self.visit_cycle(root, graph, &mut estado, &mut path, &mut cycle);
        }
        let Some(cycle) = cycle else { return };
        let pos = roots
            .iter()
            .position(|r| Some(r) == cycle.first())
            .and_then(|_| {
                self.idx_decisions
                    .get(&cycle[0])
                    .and_then(|i| self.decisions.get(*i))
                    .map(|d| d.pos)
            })
            .unwrap_or(self.flow_pos);
        self.error(
            Code::DecisionCycle,
            format!(
                "As decisões formam um ciclo ({}): um fluxo não pode voltar a uma decisão já avaliada.",
                cycle.join(" → ")
            ),
            pos,
        );
    }

    fn visit_cycle(
        &self,
        no: &str,
        graph: &BTreeMap<String, Vec<String>>,
        estado: &mut BTreeMap<String, u8>,
        path: &mut Vec<String>,
        cycle: &mut Option<Vec<String>>,
    ) {
        if cycle.is_some() || !self.idx_decisions.contains_key(no) {
            return;
        }
        match estado.get(no).copied().unwrap_or(0) {
            1 => {
                let mut found: Vec<String> = path
                    .iter()
                    .skip_while(|current| current.as_str() != no)
                    .cloned()
                    .collect();
                found.push(no.to_owned());
                *cycle = Some(found);
                return;
            }
            2 => return,
            _ => {}
        }
        estado.insert(no.to_owned(), 1);
        path.push(no.to_owned());
        if let Some(destinations) = graph.get(no) {
            for destination in destinations {
                self.visit_cycle(destination, graph, estado, path, cycle);
            }
        }
        path.pop();
        estado.insert(no.to_owned(), 2);
    }

    fn check_reach(
        &mut self,
        graph: &BTreeMap<String, Vec<String>>,
        start: &str,
        stages: &[Etapa],
        decisions: &[CollectedDecision],
    ) {
        let mut reached: BTreeSet<String> = BTreeSet::new();
        if !start.is_empty() {
            let mut pilha = vec![start.to_owned()];
            while let Some(no) = pilha.pop() {
                if no.is_empty() || !reached.insert(no.clone()) {
                    continue;
                }
                if let Some(destinations) = graph.get(&no) {
                    for destination in destinations {
                        pilha.push(destination.clone());
                    }
                }
            }
        }
        let mut positions: BTreeMap<String, Pos> = BTreeMap::new();
        for etapa in &self.stages {
            positions.insert(etapa.etapa.id.clone(), etapa.pos);
        }
        let mut warnings: Vec<(Code, String, Pos)> = Vec::new();
        for etapa in stages {
            let pos = positions.get(&etapa.id).copied().unwrap_or(self.flow_pos);
            if etapa.actions.is_empty() {
                warnings.push((
                    Code::StageWithoutAction,
                    format!(
                        "A etapa `{}` não tem nenhuma `acao(...)`: o atendimento não sai dela. Acrescente uma ação com `vai_para`.",
                        etapa.id
                    ),
                    pos,
                ));
            }
            if !start.is_empty() && !reached.contains(&etapa.id) {
                warnings.push((
                    Code::Unreachable,
                    format!(
                        "A etapa `{}` não é alcançável a partir de `inicio = \"{start}\"`. Confira os `vai_para`.",
                        etapa.id
                    ),
                    pos,
                ));
            }
        }
        for decisao in decisions {
            if !start.is_empty() && !reached.contains(&decisao.id) {
                warnings.push((
                    Code::Unreachable,
                    format!(
                        "A decisão `{}` não é alcançável a partir de `inicio = \"{start}\"`.",
                        decisao.id
                    ),
                    decisao.pos,
                ));
            }
        }
        for (code, mensagem, pos) in warnings {
            self.warning(code, mensagem, pos);
        }
    }

    fn check_field_uses(&mut self, stages: &[Etapa]) {
        let mut warnings: Vec<(String, Pos)> = Vec::new();
        for etapa in stages {
            let uses = self.uses.get(&etapa.id).cloned().unwrap_or_default();
            let step_pos = self
                .idx_stages
                .get(&etapa.id)
                .and_then(|i| self.stages.get(*i))
                .map(|e| e.pos)
                .unwrap_or(self.flow_pos);
            let positions: BTreeMap<String, Pos> = self
                .idx_stages
                .get(&etapa.id)
                .and_then(|i| self.stages.get(*i))
                .map(|e| e.fields_pos.clone())
                .unwrap_or_default();
            for campo in &etapa.campos {
                if uses.required.contains(&campo.id) || uses.read.contains(&campo.id) {
                    continue;
                }
                let pos = positions.get(&campo.id).copied().unwrap_or(step_pos);
                warnings.push((
                    format!(
                        "O campo `{}.{}` não é exigido por nenhuma ação nem lido por nenhuma regra: ninguém usa o valor.",
                        etapa.id, campo.id
                    ),
                    pos,
                ));
            }
        }
        for (mensagem, pos) in warnings {
            self.warning(Code::UnusedField, mensagem, pos);
        }
    }

    fn check_def_uses(&mut self, decisions: &[Decisao]) {
        let mut used_functions: BTreeSet<String> = BTreeSet::new();
        for decisao in decisions {
            collect_functions(&decisao.se, &mut used_functions);
        }
        let defs: Vec<(String, Pos)> = self
            .defs
            .iter()
            .map(|def| {
                let pos = self
                    .pos_defs
                    .get(&def.nome)
                    .copied()
                    .unwrap_or(self.flow_pos);
                (def.nome.clone(), pos)
            })
            .collect();
        for (nome, pos) in defs {
            if used_functions.contains(&nome) {
                continue;
            }
            self.warning(
                Code::UnusedDef,
                format!(
                    "A função `{nome}` está definida e nenhuma decisão usa `funcao(\"{nome}\")`. Chame ou apague."
                ),
                pos,
            );
        }
    }

    fn check_effects(&mut self, stages: &[Etapa]) {
        let mut campos: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for etapa in stages {
            campos.insert(
                etapa.id.as_str(),
                etapa.campos.iter().map(|c| c.id.as_str()).collect(),
            );
        }
        let mut pending: Vec<(String, String, Pos, String, bool)> = Vec::new();
        for coletado in &self.effects {
            let what = match &coletado.effect {
                Effect::MessageToCustomer { .. } => "mensagem ao cliente",
                Effect::NotifyWhatsApp { .. } => "notificação para a equipe",
                Effect::SyncCalendar { .. } => "sincronização de agenda",
                Effect::Encaminhar { .. } => continue,
            };
            for (texto, text_pos) in &coletado.texts {
                pending.push((
                    texto.clone(),
                    what.to_owned(),
                    *text_pos,
                    coletado.etapa.clone(),
                    coletado.on_enter,
                ));
            }
        }
        let mut warnings: Vec<(Code, String, Pos)> = Vec::new();
        for (texto, what, pos, effect_step, on_enter) in pending {
            for variable in variables(&texto) {
                let Some((step_name, field_name)) = variable.split_once('.') else {
                    continue;
                };
                if variable == "ficha.id" || variable == "cliente.peer" {
                    continue;
                }
                let exists = campos
                    .get(step_name)
                    .map(|campos| campos.contains(field_name))
                    .unwrap_or(false);
                if !exists {
                    warnings.push((
                        Code::VariableWithoutField,
                        format!(
                            "A {what} usa `{{{variable}}}`, mas a etapa `{step_name}` não tem o campo `{field_name}`: no atendimento sai vazio."
                        ),
                        pos,
                    ));
                    continue;
                }
                if on_enter && step_name == effect_step {
                    warnings.push((
                        Code::VariableFromOwnStep,
                        format!(
                            "`ao_entrar` da etapa `{effect_step}` usa `{{{variable}}}`, que só é preenchido dentro da própria etapa: a mensagem sai com o texto vazio."
                        ),
                        pos,
                    ));
                }
            }
        }
        for (code, mensagem, pos) in warnings {
            self.warning(code, mensagem, pos);
        }
    }

    fn check_node_cap(&mut self) {
        let nodes = self.stages.len() + self.idx_decisions.len() + self.ends.len();
        if nodes > MAX_NOS {
            let pos = self.flow_pos;
            self.error(
                Code::NodeLimit,
                format!("O fluxo tem {nodes} nós e o limite é {MAX_NOS}. Junte etapas parecidas ou divida o fluxo."),
                pos,
            );
        }
    }

    fn check_names(&mut self) {
        let locations = BTreeSet::new();
        self.check_names_stmt(self.module.statement(), &locations);
    }

    fn check_names_stmt(&mut self, stmt: &'a AstStmt, locations: &BTreeSet<String>) {
        match &stmt.node {
            Stmt::Statements(items) => {
                for item in items {
                    self.check_names_stmt(item, locations);
                }
            }
            Stmt::Def(def) => {
                let mut scope = locations.clone();
                for parametro in &def.params {
                    if let Some(ident) = parametro.ident() {
                        scope.insert(ident.node.ident.clone());
                    }
                }
                collect_assigns_stmt(&def.body, &mut scope);
                self.check_names_stmt(&def.body, &scope);
            }
            Stmt::Expression(expr) => self.check_names_expr(expr, locations),
            Stmt::Assign(assign) => self.check_names_expr(&assign.rhs, locations),
            Stmt::AssignModify(_, _, expr) => self.check_names_expr(expr, locations),
            Stmt::Return(Some(expr)) => self.check_names_expr(expr, locations),
            Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::Pass | Stmt::Load(_) => {}
            Stmt::If(cond, body) => {
                self.check_names_expr(cond, locations);
                self.check_names_stmt(body, locations);
            }
            Stmt::IfElse(cond, ramos) => {
                self.check_names_expr(cond, locations);
                self.check_names_stmt(&ramos.0, locations);
                self.check_names_stmt(&ramos.1, locations);
            }
            Stmt::For(para) => {
                self.check_names_expr(&para.over, locations);
                self.check_names_stmt(&para.body, locations);
            }
        }
    }

    fn check_names_expr(&mut self, expr: &'a AstExpr, locations: &BTreeSet<String>) {
        match &expr.node {
            Expr::Identifier(ident) => self.check_name(ident, locations),
            Expr::Tuple(items) | Expr::List(items) => {
                for item in items {
                    self.check_names_expr(item, locations);
                }
            }
            Expr::Dict(pairs) => {
                for (chave, value) in pairs {
                    self.check_names_expr(chave, locations);
                    self.check_names_expr(value, locations);
                }
            }
            Expr::Dot(target, _) => self.check_names_expr(target, locations),
            Expr::Call(target, args) => {
                self.check_names_expr(target, locations);
                for arg in &args.args {
                    self.check_names_expr(arg.expr(), locations);
                }
            }
            Expr::Index(pair) => {
                self.check_names_expr(&pair.0, locations);
                self.check_names_expr(&pair.1, locations);
            }
            Expr::Index2(pair) => {
                self.check_names_expr(&pair.0, locations);
                self.check_names_expr(&pair.1, locations);
                self.check_names_expr(&pair.2, locations);
            }
            Expr::Slice(target, a, b, c) => {
                self.check_names_expr(target, locations);
                for part in [a, b, c].into_iter().flatten() {
                    self.check_names_expr(part, locations);
                }
            }
            Expr::Lambda(lambda) => self.check_names_expr(&lambda.body, locations),
            Expr::Not(expr) | Expr::Minus(expr) | Expr::Plus(expr) | Expr::BitNot(expr) => {
                self.check_names_expr(expr, locations);
            }
            Expr::Op(a, _, b) => {
                self.check_names_expr(a, locations);
                self.check_names_expr(b, locations);
            }
            Expr::If(trio) => {
                self.check_names_expr(&trio.0, locations);
                self.check_names_expr(&trio.1, locations);
                self.check_names_expr(&trio.2, locations);
            }
            Expr::ListComprehension(body, para, clausulas) => {
                let mut scope = locations.clone();
                collect_assigns_target(&para.var, &mut scope);
                self.check_names_expr(&para.over, &scope);
                for clausula in clausulas {
                    match clausula {
                        Clause::For(para) => {
                            collect_assigns_target(&para.var, &mut scope);
                            self.check_names_expr(&para.over, &scope);
                        }
                        Clause::If(cond) => self.check_names_expr(cond, &scope),
                    }
                }
                self.check_names_expr(body, &scope);
            }
            Expr::DictComprehension(pair, para, clausulas) => {
                let mut scope = locations.clone();
                collect_assigns_target(&para.var, &mut scope);
                self.check_names_expr(&para.over, &scope);
                for clausula in clausulas {
                    match clausula {
                        Clause::For(para) => {
                            collect_assigns_target(&para.var, &mut scope);
                            self.check_names_expr(&para.over, &scope);
                        }
                        Clause::If(cond) => self.check_names_expr(cond, &scope),
                    }
                }
                self.check_names_expr(&pair.0, &scope);
                self.check_names_expr(&pair.1, &scope);
            }
            Expr::FString(f) => {
                for part in &f.expressions {
                    self.check_names_expr(part, locations);
                }
            }
            Expr::Literal(_) => {}
        }
    }

    fn check_name(&mut self, ident: &'a AstIdent, locations: &BTreeSet<String>) {
        let nome = ident.node.ident.as_str();
        if is_allowed_name(nome) || locations.contains(nome) {
            return;
        }
        let pos = self.pos(ident.span);
        self.error(
            Code::UnknownName,
            format!(
                "`{nome}` não existe na linguagem de fluxos: o script não tem relógio, rede, arquivo nem aleatório. Use as etapas, os campos e os combinadores do fluxo."
            ),
            pos,
        );
    }

    fn check_variable_format(&mut self, texto: &str, location: &str, pos: Pos) {
        for variable in variables(texto) {
            let valid_var = variable == "ficha.id"
                || variable == "cliente.peer"
                || variable
                    .split_once('.')
                    .map(|(etapa, campo)| {
                        etapa != "ficha" && etapa != "cliente" && valid_id(etapa) && valid_id(campo)
                    })
                    .unwrap_or(false);
            if valid_var {
                continue;
            }
            self.error(
                Code::InvalidVariable,
                format!(
                    "{location} usa `{{{variable}}}`, que não existe. As variáveis são `{{etapa.campo}}`, `{{ficha.id}}` e `{{cliente.peer}}`."
                ),
                pos,
            );
        }
    }
}

struct ReadEffect {
    effect: Effect,
    texts: Vec<(String, Pos)>,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    id.len() >= ID_MIN
        && id.len() <= ID_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub(crate) fn variables(texto: &str) -> Vec<String> {
    let mut saida = Vec::new();
    let mut rest = texto;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(fim) => {
                saida.push(after[..fim].to_owned());
                rest = &after[fim + 1..];
            }
            None => break,
        }
    }
    saida
}

fn descreve(expr: &AstExpr) -> String {
    match &expr.node {
        Expr::Literal(AstLiteral::String(_)) => "um texto".to_owned(),
        Expr::Literal(AstLiteral::Int(_)) => "um número inteiro".to_owned(),
        Expr::Literal(AstLiteral::Float(_)) => "um número".to_owned(),
        Expr::Literal(_) => "um valor literal".to_owned(),
        Expr::List(_) => "uma lista".to_owned(),
        Expr::Tuple(_) => "uma tupla".to_owned(),
        Expr::Dict(_) => "um dicionário".to_owned(),
        Expr::Identifier(ident) => match ident.node.ident.as_str() {
            "True" | "False" => "um booleano".to_owned(),
            "None" => "`None`".to_owned(),
            other => format!("o nome `{other}`"),
        },
        Expr::Call(target, _) => match &target.node {
            Expr::Identifier(ident) => format!("a chamada `{}(...)`", ident.node.ident),
            _ => "uma chamada".to_owned(),
        },
        Expr::FString(_) => "uma f-string".to_owned(),
        _ => "um valor calculado".to_owned(),
    }
}

fn expr_calculada(expr: &AstExpr) -> bool {
    matches!(
        expr.node,
        Expr::Op(..) | Expr::FString(_) | Expr::Index(..) | Expr::Lambda(_)
    )
}

fn field_of_rule(rule: &Rule) -> Option<String> {
    match rule {
        Rule::Igual { campo, .. }
        | Rule::Diferente { campo, .. }
        | Rule::Maior { campo, .. }
        | Rule::Menor { campo, .. }
        | Rule::Contem { campo, .. }
        | Rule::Vazio { campo } => Some(campo.clone()),
        _ => None,
    }
}

fn clone_raw(raw: &RawDestination) -> RawDestination {
    match raw {
        RawDestination::Target { id, pos } => RawDestination::Target {
            id: id.clone(),
            pos: *pos,
        },
        RawDestination::Fim { fim, pos } => RawDestination::Fim {
            fim: fim.clone(),
            pos: *pos,
        },
        RawDestination::Invalid => RawDestination::Invalid,
    }
}

fn leaf_key(rule: &Rule) -> (Option<&str>, Option<&str>, &'static str) {
    match rule {
        Rule::Igual { campo, value } => (Some(campo), Some(value), "igual"),
        Rule::Diferente { campo, value } => (Some(campo), Some(value), "diferente"),
        Rule::Maior { campo, value } => (Some(campo), Some(value), "maior"),
        Rule::Menor { campo, value } => (Some(campo), Some(value), "menor"),
        Rule::Contem { campo, value } => (Some(campo), Some(value), "contem"),
        Rule::Vazio { campo } => (Some(campo), None, "vazio"),
        _ => (None, None, ""),
    }
}

fn collect_functions(rule: &Rule, saida: &mut BTreeSet<String>) {
    match rule {
        Rule::Funcao { nome } => {
            saida.insert(nome.clone());
        }
        Rule::E { args } | Rule::Ou { args } | Rule::Nao { args } => {
            for child in args {
                collect_functions(child, saida);
            }
        }
        _ => {}
    }
}

fn collect_assigns_stmt(stmt: &AstStmt, scope: &mut BTreeSet<String>) {
    match &stmt.node {
        Stmt::Statements(items) => {
            for item in items {
                collect_assigns_stmt(item, scope);
            }
        }
        Stmt::Assign(assign) => collect_assigns_target(&assign.lhs, scope),
        Stmt::AssignModify(target, _, _) => collect_assigns_target(target, scope),
        Stmt::If(_, body) => collect_assigns_stmt(body, scope),
        Stmt::IfElse(_, ramos) => {
            collect_assigns_stmt(&ramos.0, scope);
            collect_assigns_stmt(&ramos.1, scope);
        }
        Stmt::For(para) => {
            collect_assigns_target(&para.var, scope);
            collect_assigns_stmt(&para.body, scope);
        }
        Stmt::Def(def) => {
            scope.insert(def.name.node.ident.clone());
        }
        _ => {}
    }
}

fn collect_assigns_target(target: &AstAssignTarget, scope: &mut BTreeSet<String>) {
    use starlark::syntax::ast::AssignTarget;
    match &target.node {
        AssignTarget::Identifier(ident) => {
            scope.insert(ident.node.ident.clone());
        }
        AssignTarget::Tuple(items) => {
            for item in items {
                collect_assigns_target(item, scope);
            }
        }
        AssignTarget::Index(_) | AssignTarget::Dot(..) => {}
    }
}

struct Reader<'a> {
    place: String,
    conhecidos: &'static [&'static str],
    items: Vec<Item<'a>>,
    used_names: Vec<bool>,
    positions: BTreeMap<String, Pos>,
}

impl<'a> Reader<'a> {
    fn new(
        location: &str,
        _pos: Pos,
        conhecidos: &'static [&'static str],
        items: Vec<Item<'a>>,
    ) -> Self {
        let used_names = vec![false; items.len()];
        Self {
            place: location.to_owned(),
            conhecidos,
            items,
            used_names,
            positions: BTreeMap::new(),
        }
    }

    fn item(&mut self, form: &mut Form<'a>, nome: &str) -> Option<&'a AstExpr> {
        for (i, item) in self.items.iter().enumerate() {
            if item.nome.as_deref() == Some(nome) && !self.used_names[i] {
                self.used_names[i] = true;
                let pos = form.pos(item.expr.span);
                self.positions.insert(nome.to_owned(), pos);
                return Some(item.expr);
            }
        }
        None
    }

    fn positional(&mut self, index: usize) -> Option<&'a AstExpr> {
        let mut visto = 0usize;
        for (i, item) in self.items.iter().enumerate() {
            if item.nome.is_some() {
                continue;
            }
            if visto == index {
                if self.used_names[i] {
                    return None;
                }
                self.used_names[i] = true;
                return Some(item.expr);
            }
            visto += 1;
        }
        None
    }

    fn pos_of(&self, chave: &str) -> Option<Pos> {
        self.positions.get(chave).copied()
    }

    fn texto(&mut self, form: &mut Form<'a>, nome: &str) -> Option<String> {
        let expr = self.item(form, nome)?;
        let location = format!("o argumento `{nome}` de {}", self.place);
        form.texto(expr, &location)
    }

    fn positional_text(&mut self, form: &mut Form<'a>, index: usize) -> Option<String> {
        let expr = self.positional(index)?;
        let pos = form.pos(expr.span);
        self.positions.insert(format!("__pos{index}"), pos);
        let place = format!("o argumento posicional {} de {}", index + 1, self.place);
        form.texto(expr, &place)
    }

    fn refuse_leftovers(&mut self, form: &mut Form<'a>, message: &str) {
        for (i, item) in self.items.iter().enumerate() {
            if self.used_names.get(i).copied().unwrap_or(false) {
                continue;
            }
            let pos = form.pos(item.expr.span);
            form.error(Code::UnknownArgument, message.to_owned(), pos);
        }
        self.used_names = vec![true; self.items.len()];
    }

    fn close(&mut self, form: &mut Form<'a>) {
        let mut sobras: Vec<(Option<String>, Pos)> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            if self.used_names.get(i).copied().unwrap_or(false) {
                continue;
            }
            let pos = form.pos(item.expr.span);
            sobras.push((item.nome.clone(), pos));
        }
        let conhecidos = if self.conhecidos.is_empty() {
            String::new()
        } else {
            format!(" Os argumentos são: {}.", self.conhecidos.join(", "))
        };
        let location = self.place.clone();
        for (nome, pos) in sobras {
            let mensagem = match nome {
                Some(nome) => format!(
                    "{location} não tem o argumento `{nome}`.{conhecidos} Confira a escrita: argumento com nome errado é recusado em vez de ignorado."
                ),
                None => {
                    format!("{location} não aceita esse argumento posicional a mais.{conhecidos}")
                }
            };
            form.error(Code::UnknownArgument, mensagem, pos);
        }
    }
}
