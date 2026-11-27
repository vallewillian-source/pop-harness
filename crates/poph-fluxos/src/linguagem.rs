
use std::time::{Duration, Instant};

use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect, DialectTypes};
use starlark::values::Value;
use starlark::values::none::NoneType;
use starlark::values::tuple::UnpackTuple;

pub const DECLARACOES: &[&str] = &["fluxo", "etapa", "decisao"];

pub const VALORES: &[&str] = &["gatilho", "campo", "acao", "fim", "funcao"];

pub const EFEITOS: &[&str] = &["mensagem_ao_cliente", "notificar_whatsapp", "encaminhar"];

pub const COMBINADORES: &[&str] = &[
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

pub const CONSTANTES: &[&str] = &["True", "False", "None"];

pub const FUNCOES_PURAS: &[&str] = &[
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

pub fn nomes_permitidos() -> Vec<&'static str> {
    let mut nomes = Vec::new();
    nomes.extend_from_slice(DECLARACOES);
    nomes.extend_from_slice(VALORES);
    nomes.extend_from_slice(EFEITOS);
    nomes.extend_from_slice(COMBINADORES);
    nomes.extend_from_slice(CONSTANTES);
    nomes.extend_from_slice(FUNCOES_PURAS);
    nomes
}

pub fn nome_permitido(nome: &str) -> bool {
    DECLARACOES.contains(&nome)
        || VALORES.contains(&nome)
        || EFEITOS.contains(&nome)
        || COMBINADORES.contains(&nome)
        || CONSTANTES.contains(&nome)
        || FUNCOES_PURAS.contains(&nome)
}

pub fn dialeto() -> Dialect {
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
fn declaracoes(builder: &mut GlobalsBuilder) {
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
        _rotulo: &str,
        #[starlark(kwargs)] _kwargs: Value<'v>,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn acao<'v>(
        _id: &str,
        _rotulo: &str,
        #[starlark(kwargs)] _kwargs: Value<'v>,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn fim<'v>(_id: &str, #[starlark(kwargs)] _kwargs: Value<'v>) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn funcao(_nome: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn mensagem_ao_cliente(_texto: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn notificar_whatsapp<'v>(
        #[starlark(kwargs)] _kwargs: Value<'v>,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn encaminhar(_departamento: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn igual(_campo: &str, _valor: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn diferente(_campo: &str, _valor: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn maior(_campo: &str, _valor: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn menor(_campo: &str, _valor: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn contem(_campo: &str, _valor: &str) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }

    fn vazio(_campo: &str) -> starlark::Result<NoneType> {
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

pub fn globais() -> Globals {
    GlobalsBuilder::extended_by(&[LibraryExtension::StructType])
        .with(declaracoes)
        .build()
}

pub struct Prazos {
    pub ate: Instant,
}

pub fn analisa(fonte: &str, nome: &str) -> Result<AstModule, starlark::Error> {
    AstModule::parse(nome, fonte.to_owned(), &dialeto())
}

pub struct LimitesStarlark {
    pub max_ticks: u64,
    pub max_heap_bytes: usize,
    pub max_callstack: usize,
    pub prazo: Duration,
}

pub fn carregar_modulo(
    fonte: &str,
    nome: &str,
    limites: &LimitesStarlark,
) -> Result<FrozenModule, starlark::Error> {
    let ast = analisa(fonte, nome)?;
    let globais = globais();
    let prazo = Instant::now() + limites.prazo;
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
        eval.set_check_cancelled(Box::new(move || Instant::now() > prazo));
        eval.eval_module(ast, &globais)?;
        drop(eval);
        module.freeze().map_err(starlark::Error::new_other)
    })
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn a_lista_fechada_e_exatamente_a_do_documento() {
        let esperado = [
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
        assert_eq!(nomes_permitidos(), esperado.to_vec());
        assert_eq!(nomes_permitidos().len(), 41);
        for nome in [
            "time", "open", "random", "os", "socket", "print", "load", "eval", "fail",
        ] {
            assert!(!nome_permitido(nome), "{nome} não pode existir no dialeto");
        }
    }

    #[test]
    fn o_dialeto_recusa_o_que_o_parser_ja_recusa() {
        for fonte in [
            "load(\"x.star\", \"y\")\n",
            "for x in [1]:\n    pass\n",
            "if x == 1:\n    pass\n",
            "f = lambda: 1\n",
            "s = f\"{1}\"\n",
            "def gira(dados):\n    while True: pass\n",
        ] {
            assert!(
                analisa(fonte, "t.star").is_err(),
                "o dialeto tinha de recusar: {fonte:?}"
            );
        }
        assert!(analisa("x = 1\n", "t.star").is_ok());
    }
}
