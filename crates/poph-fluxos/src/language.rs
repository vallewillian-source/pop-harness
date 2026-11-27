use std::time::{Duration, Instant};

use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect, DialectTypes};
use starlark::values::Value;
use starlark::values::none::NoneType;
use starlark::values::tuple::UnpackTuple;

pub const DECLARATIONS: &[&str] = &["fluxo", "etapa", "decisao"];

pub const VALUES: &[&str] = &["gatilho", "campo", "acao", "fim", "funcao"];

pub const EFFECTS: &[&str] = &["mensagem_ao_cliente", "notificar_whatsapp", "encaminhar"];

pub const COMBINATORS: &[&str] = &[
    "igual",
    "diferente",
    "maior",
    "menor",
    "contem",
    "vazio",
    "e",
    "ou",
    "nao",
];

pub const CONSTANTS: &[&str] = &["True", "False", "None"];

pub const PURE_FUNCTIONS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "dict",
    "enumerate",
    "float",
    "int",
    "len",
    "list",
    "max",
    "min",
    "range",
    "reversed",
    "sorted",
    "str",
    "tuple",
    "zip",
];

pub fn allowed_names() -> Vec<&'static str> {
    let mut names = Vec::new();
    names.extend_from_slice(DECLARATIONS);
    names.extend_from_slice(VALUES);
    names.extend_from_slice(EFFECTS);
    names.extend_from_slice(COMBINATORS);
    names.extend_from_slice(CONSTANTS);
    names.extend_from_slice(PURE_FUNCTIONS);
    names
}

pub fn is_allowed_name(nome: &str) -> bool {
    DECLARATIONS.contains(&nome)
        || VALUES.contains(&nome)
        || EFFECTS.contains(&nome)
        || COMBINATORS.contains(&nome)
        || CONSTANTS.contains(&nome)
        || PURE_FUNCTIONS.contains(&nome)
}

pub fn dialect() -> Dialect {
    Dialect {
        enable_def: true,
        enable_lambda: false,
        enable_load: false,
        enable_keyword_only_arguments: true,
        enable_positional_only_arguments: false,
        enable_types: DialectTypes::Disable,
        enable_load_reexport: false,
        enable_top_level_stmt: false,
        enable_f_strings: false,
        _non_exhaustive: (),
    }
}

#[starlark::starlark_module]
fn declarations(builder: &mut GlobalsBuilder) {
    fn fluxo<'v>(#[starlark(kwargs)] _kwargs: Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn etapa<'v>(#[starlark(kwargs)] _kwargs: Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn decisao<'v>(#[starlark(kwargs)] _kwargs: Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn gatilho<'v>(#[starlark(kwargs)] _kwargs: Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn campo<'v>(
        _id: &str,
        _label: &str,
        #[starlark(kwargs)] _kwargs: Value<'v>,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn acao<'v>(
        _id: &str,
        _label: &str,
        #[starlark(kwargs)] _kwargs: Value<'v>,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn fim<'v>(_id: &str, #[starlark(kwargs)] _kwargs: Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn funcao(_name: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn mensagem_ao_cliente(_text: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn notificar_whatsapp<'v>(
        #[starlark(kwargs)] _kwargs: Value<'v>,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn encaminhar(_department: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn igual(_field: &str, _value: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn diferente(_field: &str, _value: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn maior(_field: &str, _value: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn menor(_field: &str, _value: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn contem(_field: &str, _value: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn vazio(_field: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn e<'v>(#[starlark(args)] _args: UnpackTuple<Value<'v>>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn ou<'v>(#[starlark(args)] _args: UnpackTuple<Value<'v>>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn nao<'v>(_a: Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }
}

pub fn environment() -> Globals {
    GlobalsBuilder::extended_by(&[LibraryExtension::StructType])
        .with(declarations)
        .build()
}

pub struct Deadlines {
    pub until: Instant,
}

pub fn parse_ast(fonte: &str, nome: &str) -> Result<AstModule, starlark::Error> {
    AstModule::parse(nome, fonte.to_owned(), &dialect())
}

pub struct StarlarkLimits {
    pub max_ticks: u64,
    pub max_heap_bytes: usize,
    pub max_callstack: usize,
    pub deadline: Duration,
}

pub fn load_module(
    fonte: &str,
    nome: &str,
    limites: &StarlarkLimits,
) -> Result<FrozenModule, starlark::Error> {
    let ast = parse_ast(fonte, nome)?;
    let environment = environment();
    let deadline = Instant::now() + limites.deadline;
    let (max_ticks, max_heap, max_callstack) = (
        limites.max_ticks,
        limites.max_heap_bytes,
        limites.max_callstack,
    );
    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.set_max_tick_count(max_ticks)
            .map_err(starlark::Error::new_other)?;
        eval.set_max_heap_size(max_heap)
            .map_err(starlark::Error::new_other)?;
        eval.set_max_callstack_size(max_callstack)
            .map_err(starlark::Error::new_other)?;
        eval.set_check_cancelled(Box::new(move || Instant::now() > deadline));
        eval.eval_module(ast, &environment)?;
        drop(eval);
        module.freeze().map_err(starlark::Error::new_other)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_list_is_exactly_the_document_one() {
        let expected = [
            "fluxo",
            "etapa",
            "decisao",
            "gatilho",
            "campo",
            "acao",
            "fim",
            "funcao",
            "mensagem_ao_cliente",
            "notificar_whatsapp",
            "encaminhar",
            "igual",
            "diferente",
            "maior",
            "menor",
            "contem",
            "vazio",
            "e",
            "ou",
            "nao",
            "True",
            "False",
            "None",
            "abs",
            "all",
            "any",
            "bool",
            "dict",
            "enumerate",
            "float",
            "int",
            "len",
            "list",
            "max",
            "min",
            "range",
            "reversed",
            "sorted",
            "str",
            "tuple",
            "zip",
        ];
        assert_eq!(allowed_names(), expected.to_vec());
        assert_eq!(allowed_names().len(), 41);
        for nome in [
            "time", "open", "random", "os", "socket", "print", "load", "eval", "fail",
        ] {
            assert!(!is_allowed_name(nome), "{nome} não pode existir no dialeto");
        }
    }

    #[test]
    fn dialect_refuses_what_the_parser_already_refuses() {
        for fonte in [
            "load(\"x.star\", \"y\")\n",
            "for x in [1]:\n    pass\n",
            "if x == 1:\n    pass\n",
            "f = lambda: 1\n",
            "s = f\"{1}\"\n",
            "def gira(dados):\n    while True: pass\n",
        ] {
            assert!(
                parse_ast(fonte, "t.star").is_err(),
                "o dialeto tinha de recusar: {fonte:?}"
            );
        }
        assert!(parse_ast("x = 1\n", "t.star").is_ok());
    }
}
