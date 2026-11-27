use std::time::Duration;

pub(crate) mod form;

use poph_types::config::FlowsConfig;

use crate::diagnostics::{Code, Diagnostic};
use crate::ir::Ir;
use crate::language::{StarlarkLimits, load_module, parse_ast};

pub const MAX_FONTE_BYTES: usize = 32 * 1024;
pub const MAX_TICKS: u64 = 5_000_000;
pub const MAX_HEAP_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CALLSTACK: usize = 128;
pub const LOAD_DEADLINE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limites {
    pub max_fonte_bytes: usize,
    pub max_nos: usize,
    pub max_ticks: u64,
    pub max_heap_bytes: usize,
    pub max_callstack: usize,
    pub load_deadline: Duration,
}

impl Default for Limites {
    fn default() -> Self {
        Self {
            max_fonte_bytes: MAX_FONTE_BYTES,
            max_nos: form::MAX_NOS,
            max_ticks: MAX_TICKS,
            max_heap_bytes: MAX_HEAP_BYTES,
            max_callstack: MAX_CALLSTACK,
            load_deadline: LOAD_DEADLINE,
        }
    }
}

impl Limites {
    pub fn from_config(fluxos: &FlowsConfig) -> Self {
        Self {
            max_fonte_bytes: usize::try_from(fluxos.max_source_bytes).unwrap_or(usize::MAX),
            max_nos: usize::try_from(fluxos.max_nodes).unwrap_or(usize::MAX),
            ..Self::default()
        }
    }

    fn starlark(&self) -> StarlarkLimits {
        StarlarkLimits {
            max_ticks: self.max_ticks,
            max_heap_bytes: self.max_heap_bytes,
            max_callstack: self.max_callstack,
            deadline: self.load_deadline,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Compiled {
    pub ir: Option<Ir>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Compiled {
    pub fn has_error(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| !d.is_error())
    }

    pub fn ir(&self) -> Result<&Ir, &[Diagnostic]> {
        match &self.ir {
            Some(ir) => Ok(ir),
            None => Err(&self.diagnostics),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadFailure {
    Form(Vec<Diagnostic>),
    Divergent { fluxo: String, motivo: String },
}

impl LoadFailure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Form(_) => "fluxo_invalido",
            Self::Divergent { .. } => "ir_divergente",
        }
    }
}

pub fn compile(fonte: &str, limites: &Limites) -> Compiled {
    let mut diagnostics = Vec::new();

    if fonte.len() > limites.max_fonte_bytes {
        diagnostics.push(Diagnostic::error(
            Code::SourceTooLarge,
            format!(
                "O script tem {} bytes e o limite é {} bytes ({} KB). Tire textos longos das instruções ou divida o fluxo em dois.",
                fonte.len(),
                limites.max_fonte_bytes,
                limites.max_fonte_bytes / 1024
            ),
            1,
            1,
        ));
        return Compiled {
            ir: None,
            diagnostics,
        };
    }

    let module = match parse_ast(fonte, "fluxo.star") {
        Ok(module) => module,
        Err(error) => {
            diagnostics.push(parse_diagnostic(&error));
            return Compiled {
                ir: None,
                diagnostics,
            };
        }
    };

    let (ir, mut form_diagnostics) = form::compile(fonte, &module);
    diagnostics.append(&mut form_diagnostics);

    let ir = match ir {
        Some(ir) if ir.stages.len() + ir.decisions.len() + ir.ends.len() > limites.max_nos => {
            let nodes = ir.stages.len() + ir.decisions.len() + ir.ends.len();
            diagnostics.push(Diagnostic::error(
                Code::NodeLimit,
                format!(
                    "O fluxo tem {nodes} nós e o limite é {}. Junte etapas parecidas ou divida o fluxo.",
                    limites.max_nos
                ),
                1,
                1,
            ));
            None
        }
        other => other,
    };

    if ir.is_some()
        && let Err(error) = load_module(fonte, "fluxo.star", &limites.starlark())
    {
        diagnostics.push(load_diagnostic(&error));
        return Compiled {
            ir: None,
            diagnostics,
        };
    }

    Compiled { ir, diagnostics }
}

pub fn load(stored_ir: &Ir, fonte: &str, limites: &Limites) -> Result<Ir, LoadFailure> {
    let compiled = compile(fonte, limites);
    let Some(recompiled) = compiled.ir else {
        return Err(LoadFailure::Form(compiled.diagnostics));
    };
    if recompiled == *stored_ir {
        return Ok(recompiled);
    }
    Err(LoadFailure::Divergent {
        fluxo: stored_ir.fluxo.id.clone(),
        motivo: differences(stored_ir, &recompiled),
    })
}

fn differences(stored: &Ir, recompiled: &Ir) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if stored.schema_versao != recompiled.schema_versao {
        parts.push("a versão do esquema");
    }
    if stored.fluxo != recompiled.fluxo {
        parts.push("o cabeçalho do fluxo");
    }
    if stored.gatilho != recompiled.gatilho {
        parts.push("o gatilho");
    }
    if stored.start != recompiled.start {
        parts.push("a etapa de início");
    }
    if stored.stages != recompiled.stages {
        parts.push("as etapas");
    }
    if stored.decisions != recompiled.decisions {
        parts.push("as decisões");
    }
    if stored.ends != recompiled.ends {
        parts.push("os fins");
    }
    if stored.defs != recompiled.defs {
        parts.push("as funções de regra");
    }
    if parts.is_empty() {
        return "a estrutura gravada difere da recompilação do texto".to_owned();
    }
    format!(
        "a estrutura gravada difere da recompilação do texto em {}",
        parts.join(", ")
    )
}

fn parse_diagnostic(error: &starlark::Error) -> Diagnostic {
    let (line, column) = error_position(error);
    let texto = error.to_string();
    let lowercase = texto.to_lowercase();
    let (code, mensagem) = if lowercase.contains("not allowed in this dialect")
        || lowercase.contains("reserved keyword")
        || lowercase.contains("cannot be used outside")
        || lowercase.contains("must only occur at the top")
        || lowercase.contains("not supported")
    {
        let what = if lowercase.contains("load") {
            "`load`"
        } else if lowercase.contains("lambda") {
            "`lambda`"
        } else if lowercase.contains("f-string") || lowercase.contains("f_string") {
            "f-string"
        } else if lowercase.contains("for") {
            "`for`"
        } else if lowercase.contains("if") {
            "`if`"
        } else if lowercase.contains("while") {
            "`while`"
        } else if lowercase.contains("class") {
            "`class`"
        } else if lowercase.contains("try") {
            "`try`"
        } else {
            "esta construção"
        };
        (
            Code::Dialect,
            format!(
                "{what} não faz parte da linguagem de fluxos. Um fluxo descreve etapas, campos, ações e regras; cálculo livre só dentro de `def {}(dados)`.",
                "minha_regra"
            ),
        )
    } else {
        let summary = texto.lines().next().unwrap_or(&texto).trim().to_owned();
        (
            Code::Syntax,
            format!("Erro de sintaxe no script: {summary}"),
        )
    };
    Diagnostic::error(code, mensagem, line, column)
}

fn load_diagnostic(error: &starlark::Error) -> Diagnostic {
    let (line, column) = error_position(error);
    let texto = error.to_string();
    let lowercase = texto.to_lowercase();
    if lowercase.contains("tick")
        || lowercase.contains("heap")
        || lowercase.contains("callstack")
        || lowercase.contains("cancelled")
        || lowercase.contains("stack overflow")
        || lowercase.contains("recursion")
    {
        return Diagnostic::error(
            Code::LoadLimit,
            "O script executou demais ao ser carregado (ticks, memória, profundidade ou prazo de 1 s). No topo do arquivo deve haver só declarações; confira se alguma `def` tem laço longo.",
            line,
            column,
        );
    }
    if lowercase.contains("not found") {
        return Diagnostic::error(
            Code::UnknownName,
            format!("O script usa um nome que não existe na linguagem de fluxos: {texto}"),
            line,
            column,
        );
    }
    let summary = texto.lines().next().unwrap_or(&texto).trim().to_owned();
    Diagnostic::error(
        Code::Syntax,
        format!("O script não carregou: {summary}"),
        line,
        column,
    )
}

fn error_position(error: &starlark::Error) -> (u32, u32) {
    match error.span() {
        Some(span) => {
            let resolved = span.resolve_span();
            (
                resolved.begin.line as u32 + 1,
                resolved.begin.column as u32 + 1,
            )
        }
        None => (1, 1),
    }
}
