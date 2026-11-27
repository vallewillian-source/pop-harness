use std::collections::BTreeMap;
use std::fmt;

use poph_types::ActionActor;
use starlark::syntax::ast::{Argument, AstLiteral, AstNoPayload, AstStmt, CallArgsP, Expr, Stmt};

use crate::compilador::{Limites, compile};
use crate::diagnostics::Diagnostic;
use crate::ir::{Acao, Campo, Decisao, Destination, Effect, Etapa, Gatilho, Ir, Rule};
use crate::language::parse_ast;

pub const WIDTH: usize = 112;

const INDENT: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatFailure {
    pub diagnostics: Vec<Diagnostic>,
}

impl FormatFailure {
    pub fn code(&self) -> &'static str {
        "fluxo_invalido"
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }
}

impl fmt::Display for FormatFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut errors = self.errors();
        match errors.next() {
            Some(first) => {
                f.write_str("- ")?;
                if let (Some(line), Some(column)) = (first.line, first.column) {
                    write!(f, "{line}:{column} ")?;
                }
                f.write_str(&first.mensagem)
            }
            None => f.write_str("o script não compila"),
        }
    }
}

impl std::error::Error for FormatFailure {}

pub fn format(fonte: &str) -> Result<String, FormatFailure> {
    format_with(fonte, &Limites::default())
}

pub fn format_with(fonte: &str, limites: &Limites) -> Result<String, FormatFailure> {
    let compiled = compile(fonte, limites);
    let Some(ir) = compiled.ir else {
        return Err(FormatFailure {
            diagnostics: compiled.diagnostics,
        });
    };
    Ok(assemble(&ir, &comments(fonte)))
}

pub fn print(ir: &Ir) -> String {
    assemble(ir, &BTreeMap::new())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Flow,
    Function(String),
    Stage(String),
    Decision(String),
}

fn assemble(ir: &Ir, comments: &BTreeMap<Key, Vec<String>>) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(ir.stages.len() + ir.decisions.len() + 2);
    parts.push(block(Key::Flow, body(&flow_value(ir)), comments));
    for definition in &ir.defs {
        parts.push(block(
            Key::Function(definition.nome.clone()),
            definition.fonte.clone(),
            comments,
        ));
    }
    for etapa in &ir.stages {
        parts.push(block(
            Key::Stage(etapa.id.clone()),
            body(&stage_value(etapa)),
            comments,
        ));
    }
    for decisao in &ir.decisions {
        parts.push(block(
            Key::Decision(decisao.id.clone()),
            body(&decision_value(decisao)),
            comments,
        ));
    }
    let mut output = parts.join("\n\n");
    output.push('\n');
    output
}

fn block(key: Key, texto: String, comments: &BTreeMap<Key, Vec<String>>) -> String {
    match comments.get(&key) {
        Some(lines) if !lines.is_empty() => format!("{}\n{texto}", lines.join("\n")),
        _ => texto,
    }
}

fn comments(fonte: &str) -> BTreeMap<Key, Vec<String>> {
    let mut map = BTreeMap::new();
    let Ok(module) = parse_ast(fonte, "fluxo.star") else {
        return map;
    };
    let Stmt::Statements(items) = &module.statement().node else {
        return map;
    };
    for item in items {
        let Some(key) = declaration_key(item) else {
            continue;
        };
        let start_byte = item.span.begin().get() as usize;
        let lines = comment_above(fonte, start_byte);
        if !lines.is_empty() {
            map.insert(key, lines);
        }
    }
    map
}

fn declaration_key(item: &AstStmt) -> Option<Key> {
    match &item.node {
        Stmt::Def(definition) => Some(Key::Function(definition.name.node.ident.clone())),
        Stmt::Expression(expr) => {
            let Expr::Call(target, args) = &expr.node else {
                return None;
            };
            let Expr::Identifier(nome) = &target.node else {
                return None;
            };
            match nome.node.ident.as_str() {
                "fluxo" => Some(Key::Flow),
                "etapa" => id_argument(args).map(Key::Stage),
                "decisao" => id_argument(args).map(Key::Decision),
                _ => None,
            }
        }
        _ => None,
    }
}

fn id_argument(args: &CallArgsP<AstNoPayload>) -> Option<String> {
    for argument in &args.args {
        if let Argument::Named(nome, expr) = &argument.node
            && nome.node == "id"
            && let Expr::Literal(AstLiteral::String(texto)) = &expr.node
        {
            return Some(texto.node.clone());
        }
    }
    None
}

fn comment_above(fonte: &str, start_byte: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line_start = match fonte
        .get(..start_byte)
        .and_then(|before| before.rfind('\n'))
    {
        Some(newline) => newline + 1,
        None => 0,
    };
    while line_start > 0 {
        let previous_end = line_start - 1;
        let previous_start = match fonte
            .get(..previous_end)
            .and_then(|before| before.rfind('\n'))
        {
            Some(newline) => newline + 1,
            None => 0,
        };
        let Some(line) = fonte.get(previous_start..previous_end) else {
            break;
        };
        let trimmed = line.trim();
        if !trimmed.starts_with('#') {
            break;
        }
        lines.push(trimmed.to_owned());
        line_start = previous_start;
    }
    lines.reverse();
    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Literal(String),
    List(List),
    Call(Call),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct List {
    items: Vec<Value>,
    open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    nome: &'static str,
    args: Vec<CallArg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CallArg {
    nome: Option<&'static str>,
    value: Value,
}

fn call(nome: &'static str, args: Vec<CallArg>) -> Value {
    Value::Call(Call { nome, args })
}

fn positional(value: Value) -> CallArg {
    CallArg { nome: None, value }
}

fn named(nome: &'static str, value: Value) -> CallArg {
    CallArg {
        nome: Some(nome),
        value,
    }
}

fn literal(texto: &str) -> Value {
    Value::Literal(quoted_text(texto))
}

fn boolean(value: bool) -> Value {
    Value::Literal(if value { "True" } else { "False" }.to_owned())
}

fn list(items: Vec<Value>) -> Value {
    Value::List(List { items, open: false })
}

fn open_list(items: Vec<Value>) -> Value {
    Value::List(List { items, open: true })
}

fn text_list(items: &[String]) -> Value {
    list(items.iter().map(|item| literal(item)).collect())
}

fn quoted_text(texto: &str) -> String {
    let mut output = String::with_capacity(texto.len() + 2);
    output.push('"');
    for character in texto.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other if other.is_control() => {
                output.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => output.push(other),
        }
    }
    output.push('"');
    output
}

fn flow_value(ir: &Ir) -> Value {
    let mut args = vec![
        named("id", literal(&ir.fluxo.id)),
        named("nome", literal(&ir.fluxo.nome)),
    ];
    if let Some(descricao) = &ir.fluxo.descricao {
        args.push(named("descricao", literal(descricao)));
    }
    args.push(named("inicio", literal(&ir.start)));
    args.push(named("gatilho", trigger_value(&ir.gatilho)));
    call("fluxo", args)
}

fn trigger_value(gatilho: &Gatilho) -> Value {
    let mut args = vec![named("quando", literal(&gatilho.when))];
    if let Some(not_when) = &gatilho.not_when {
        args.push(named("nao_quando", literal(not_when)));
    }
    if !gatilho.examples.is_empty() {
        args.push(named("exemplos", text_list(&gatilho.examples)));
    }
    call("gatilho", args)
}

fn stage_value(etapa: &Etapa) -> Value {
    let mut args = vec![
        named("id", literal(&etapa.id)),
        named("nome", literal(&etapa.nome)),
    ];
    if let Some(instruction) = &etapa.instruction {
        args.push(named("instrucao", literal(instruction)));
    }
    if etapa.action_actors != [ActionActor::Agent] {
        args.push(named(
            "atores",
            list(
                etapa
                    .action_actors
                    .iter()
                    .map(|actor| literal(actor.as_text()))
                    .collect(),
            ),
        ));
    }
    if !etapa.campos.is_empty() {
        args.push(named(
            "campos",
            open_list(etapa.campos.iter().map(field_value).collect()),
        ));
    }
    if !etapa.on_enter.is_empty() {
        args.push(named(
            "ao_entrar",
            open_list(etapa.on_enter.iter().map(effect_value).collect()),
        ));
    }
    if !etapa.actions.is_empty() {
        args.push(named(
            "acoes",
            open_list(etapa.actions.iter().map(action_value).collect()),
        ));
    }
    call("etapa", args)
}

fn field_value(campo: &Campo) -> Value {
    let mut args = vec![
        positional(literal(&campo.id)),
        positional(literal(&campo.label)),
        named("tipo", literal(campo.tipo.as_text())),
    ];
    if !campo.options.is_empty() {
        args.push(named("opcoes", text_list(&campo.options)));
    }
    if campo.required {
        args.push(named("obrigatorio", boolean(true)));
    }
    if let Some(help) = &campo.help {
        args.push(named("ajuda", literal(help)));
    }
    call("campo", args)
}

fn action_value(acao: &Acao) -> Value {
    let mut args = vec![
        positional(literal(&acao.id)),
        positional(literal(&acao.label)),
    ];
    if !acao.requires.is_empty() {
        args.push(named("exige", text_list(&acao.requires)));
    }
    args.push(named("vai_para", destination_value(&acao.goes_to)));
    if let Some(actor) = acao.actor {
        args.push(named("ator", literal(actor.as_text())));
    }
    if !acao.effects.is_empty() {
        args.push(named(
            "efeitos",
            list(acao.effects.iter().map(effect_value).collect()),
        ));
    }
    call("acao", args)
}

fn destination_value(destination: &Destination) -> Value {
    match destination {
        Destination::Etapa { id } | Destination::Decisao { id } => literal(id),
        Destination::Fim {
            id,
            resultado,
            ends_service,
        } => end_value(id, resultado.as_deref(), *ends_service),
    }
}

fn end_value(id: &str, resultado: Option<&str>, ends_service: bool) -> Value {
    let mut args = vec![positional(literal(id))];
    if let Some(resultado) = resultado {
        args.push(named("resultado", literal(resultado)));
    }
    if ends_service {
        args.push(named("encerra_atendimento", boolean(ends_service)));
    }
    call("fim", args)
}

fn decision_value(decisao: &Decisao) -> Value {
    call(
        "decisao",
        vec![
            named("id", literal(&decisao.id)),
            named("se", rule_value(&decisao.se)),
            named("entao", destination_value(&decisao.then_branch)),
            named("senao", destination_value(&decisao.else_branch)),
        ],
    )
}

fn rule_value(rule: &Rule) -> Value {
    match rule {
        Rule::Igual { campo, value } => two_texts("igual", campo, value),
        Rule::Diferente { campo, value } => two_texts("diferente", campo, value),
        Rule::Maior { campo, value } => two_texts("maior", campo, value),
        Rule::Menor { campo, value } => two_texts("menor", campo, value),
        Rule::Contem { campo, value } => two_texts("contem", campo, value),
        Rule::Vazio { campo } => call("vazio", vec![positional(literal(campo))]),
        Rule::E { args } => combiner("e", args),
        Rule::Ou { args } => combiner("ou", args),
        Rule::Nao { args } => combiner("nao", args),
        Rule::Funcao { nome } => call("funcao", vec![positional(literal(nome))]),
    }
}

fn two_texts(nome: &'static str, campo: &str, value: &str) -> Value {
    call(
        nome,
        vec![positional(literal(campo)), positional(literal(value))],
    )
}

fn combiner(nome: &'static str, rules: &[Rule]) -> Value {
    call(
        nome,
        rules
            .iter()
            .map(|rule| positional(rule_value(rule)))
            .collect(),
    )
}

fn effect_value(effect: &Effect) -> Value {
    match effect {
        Effect::MessageToCustomer { texto } => {
            call("mensagem_ao_cliente", vec![positional(literal(texto))])
        }
        Effect::NotifyWhatsApp { para, mensagem } => call(
            "notificar_whatsapp",
            vec![
                named("para", literal(para)),
                named("mensagem", literal(mensagem)),
            ],
        ),
        Effect::Encaminhar { departamento } => {
            call("encaminhar", vec![positional(literal(departamento))])
        }
    }
}

fn body(value: &Value) -> String {
    let mut output = String::new();
    write_open(value, 0, &mut output);
    output
}

fn write_open(value: &Value, indent: usize, output: &mut String) {
    let Value::Call(call) = value else {
        write_value(value, indent, indent, output);
        return;
    };
    if call.args.is_empty() {
        output.push_str(call.nome);
        output.push_str("()");
        return;
    }
    output.push_str(call.nome);
    output.push_str("(\n");
    let inner_indent = indent + INDENT;
    for argument in &call.args {
        output.push_str(&" ".repeat(inner_indent));
        write_argument(argument, inner_indent, output);
        output.push_str(",\n");
    }
    output.push_str(&" ".repeat(indent));
    output.push(')');
}

fn write_argument(argument: &CallArg, indent: usize, output: &mut String) {
    let line = argument_in_one_line(argument);
    if fits(&line, indent) && !always_open(&argument.value) {
        output.push_str(&line);
        return;
    }
    let prefix = match argument.nome {
        Some(nome) => {
            output.push_str(nome);
            output.push_str(" = ");
            nome.chars().count() + 3
        }
        None => 0,
    };
    write_value(&argument.value, indent, indent + prefix, output);
}

fn write_value(value: &Value, indent: usize, column: usize, output: &mut String) {
    let line = value_in_one_line(value);
    if fits(&line, column) && !always_open(value) {
        output.push_str(&line);
        return;
    }
    match value {
        Value::Literal(_) => output.push_str(&line),
        Value::List(list) => {
            if list.items.is_empty() {
                output.push_str("[]");
                return;
            }
            output.push_str("[\n");
            let inner_indent = indent + INDENT;
            for item in &list.items {
                output.push_str(&" ".repeat(inner_indent));
                write_value(item, inner_indent, inner_indent, output);
                output.push_str(",\n");
            }
            output.push_str(&" ".repeat(indent));
            output.push(']');
        }
        Value::Call(_) => write_open(value, indent, output),
    }
}

fn value_in_one_line(value: &Value) -> String {
    match value {
        Value::Literal(texto) => texto.clone(),
        Value::List(list) => {
            let inside: Vec<String> = list.items.iter().map(value_in_one_line).collect();
            format!("[{}]", inside.join(", "))
        }
        Value::Call(call) => {
            let inside: Vec<String> = call.args.iter().map(argument_in_one_line).collect();
            format!("{}({})", call.nome, inside.join(", "))
        }
    }
}

fn argument_in_one_line(argument: &CallArg) -> String {
    match argument.nome {
        Some(nome) => format!("{nome} = {}", value_in_one_line(&argument.value)),
        None => value_in_one_line(&argument.value),
    }
}

fn always_open(value: &Value) -> bool {
    match value {
        Value::List(list) => list.open,
        Value::Call(call) => matches!(call.nome, "e" | "ou" | "nao"),
        Value::Literal(_) => false,
    }
}

fn fits(texto: &str, column: usize) -> bool {
    column + texto.chars().count() <= WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Fim;
    use poph_types::FieldKind;

    #[test]
    fn quoted_text_escapes_what_the_dialect_understands() {
        assert_eq!(quoted_text("simples"), "\"simples\"");
        assert_eq!(quoted_text("com \"aspas\""), "\"com \\\"aspas\\\"\"");
        assert_eq!(
            quoted_text("barra \\ invertida"),
            "\"barra \\\\ invertida\""
        );
        assert_eq!(quoted_text("linha\nnova\ttab"), "\"linha\\nnova\\ttab\"");
        assert_eq!(quoted_text("acento ção"), "\"acento ção\"");
        assert_eq!(quoted_text("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn width_counts_characters_not_bytes() {
        assert!(fits(&"ç".repeat(WIDTH), 0));
        assert!(!fits(&"ç".repeat(WIDTH + 1), 0));
        assert!(fits("abc", WIDTH - 3));
        assert!(!fits("abcd", WIDTH - 3));
    }

    #[test]
    fn comment_above_takes_only_the_attached_block() {
        let fonte = "# primeiro\n# segundo\nfluxo(\n    id = \"x\",\n)\n";
        assert_eq!(
            comment_above(fonte, fonte.find("fluxo").unwrap()),
            vec!["# primeiro".to_owned(), "# segundo".to_owned()]
        );
        let with_gap = "# solto\n\n# ligado\nfluxo(\n)\n";
        assert_eq!(
            comment_above(with_gap, with_gap.find("fluxo").unwrap()),
            vec!["# ligado".to_owned()]
        );
        assert!(comment_above("fluxo()\n", 0).is_empty());
    }

    #[test]
    fn declaration_key_reads_the_argument_id() {
        let fonte = "fluxo(id = \"x\")\netapa(id = \"passo\", nome = \"P\")\ndecisao(id = \"d\", se = vazio(\"a\"), entao = \"passo\", senao = \"passo\")\ndef regra(dados):\n    return True\n";
        let module = parse_ast(fonte, "fluxo.star").expect("parseia");
        let Stmt::Statements(items) = &module.statement().node else {
            panic!("o topo é uma lista de declarações");
        };
        let keys: Vec<Key> = items.iter().filter_map(declaration_key).collect();
        assert_eq!(
            keys,
            vec![
                Key::Flow,
                Key::Stage("passo".to_owned()),
                Key::Decision("d".to_owned()),
                Key::Function("regra".to_owned()),
            ]
        );
    }

    #[test]
    fn field_booleans_and_numbers_come_out_in_script_form() {
        let campo = Campo {
            id: "idade".to_owned(),
            label: "Idade".to_owned(),
            tipo: FieldKind::Number,
            required: false,
            options: Vec::new(),
            help: None,
        };
        assert_eq!(
            value_in_one_line(&field_value(&campo)),
            "campo(\"idade\", \"Idade\", tipo = \"numero\")"
        );
    }

    #[test]
    fn printed_def_is_the_source_the_ir_kept() {
        let ir = Ir {
            schema_versao: crate::ir::SCHEMA_VERSAO,
            fluxo: crate::ir::Fluxo {
                id: "com_def".to_owned(),
                nome: "Com def".to_owned(),
                descricao: None,
            },
            gatilho: Gatilho {
                when: "quando".to_owned(),
                not_when: None,
                examples: Vec::new(),
            },
            start: "unica".to_owned(),
            stages: vec![Etapa {
                id: "unica".to_owned(),
                nome: "Única".to_owned(),
                instruction: None,
                action_actors: vec![ActionActor::Agent],
                campos: Vec::new(),
                actions: vec![Acao {
                    id: "seguir".to_owned(),
                    label: "Seguir".to_owned(),
                    requires: Vec::new(),
                    actor: None,
                    goes_to: Destination::Fim {
                        id: "fim".to_owned(),
                        resultado: None,
                        ends_service: false,
                    },
                    effects: Vec::new(),
                }],
                on_enter: Vec::new(),
            }],
            decisions: Vec::new(),
            ends: vec![Fim {
                id: "fim".to_owned(),
                resultado: None,
                ends_service: false,
            }],
            defs: vec![crate::ir::Def {
                nome: "regra".to_owned(),
                fonte: "def regra(dados):\n    return True".to_owned(),
            }],
        };
        let texto = print(&ir);
        assert!(
            texto.contains("def regra(dados):\n    return True"),
            "a def tem de sair literal:\n{texto}"
        );
        let def_position = texto.find("def regra").expect("def impressa");
        let stage_position = texto.find("etapa(").expect("etapa impressa");
        assert!(
            def_position < stage_position,
            "def depois da etapa:\n{texto}"
        );
    }

    #[test]
    fn the_ir_end_list_prints_the_same() {
        let fim = Fim {
            id: "desistiu".to_owned(),
            resultado: Some("Cliente desistiu".to_owned()),
            ends_service: true,
        };
        assert_eq!(
            value_in_one_line(&end_value(
                &fim.id,
                fim.resultado.as_deref(),
                fim.ends_service
            )),
            "fim(\"desistiu\", resultado = \"Cliente desistiu\", encerra_atendimento = True)"
        );
    }
}
