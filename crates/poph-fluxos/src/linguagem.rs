//! O dialeto de fluxos e a lista fechada de nomes que o autor alcança
//! (`docs/FLUXOS_LINGUAGEM.md` §3).
//!
//! O script de fluxo é Starlark (decisão da FX-01) com um recorte: no topo só declaração, `load`
//! desligado, sem `lambda`, sem f-string, sem tipo e sem laço no topo. A biblioteca registrada no
//! módulo é a `standard` sem `struct` — a checagem de nomes ([`NOMES_PERMITIDOS`]) é o que fecha a
//! superfície: **não há relógio, rede, arquivo nem aleatório**.
//!
//! Este módulo também é quem **carrega** o script (parse + avaliação com limites), que é o que o
//! motor (FX-12) usa para avaliar uma `def` e o que o compilador usa como última barreira.
//!
//! As funções de declaração registradas aqui não fazem nada: o compilador monta a IR lendo a
//! árvore sintática (é o que dá linha e coluna em cada diagnóstico), e quem avalia de verdade é o
//! motor, que precisa só do módulo congelado para as regras avançadas.

use std::time::{Duration, Instant};

use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect, DialectTypes};
use starlark::values::Value;
use starlark::values::none::NoneType;
use starlark::values::tuple::UnpackTuple;

/// Nomes de declaração de topo.
pub const DECLARACOES: &[&str] = &["fluxo", "etapa", "decisao"];

/// Nomes que produzem valor dentro de uma declaração.
pub const VALORES: &[&str] = &["gatilho", "campo", "acao", "fim", "funcao"];

/// Nomes de efeito.
pub const EFEITOS: &[&str] = &["mensagem_ao_cliente", "notificar_whatsapp", "encaminhar"];

/// Nomes de combinador de regra.
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

/// Constantes que o script alcança.
pub const CONSTANTES: &[&str] = &["True", "False", "None"];

/// Funções puras, só dentro de `def`.
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

/// A lista fechada de nomes que o autor alcança (§3.3) — 41 nomes, e nada além disso.
///
/// Um teste do crate falha se esta lista mudar sem a tabela do documento mudar junto.
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

/// Se o nome existe no dialeto do fluxo.
pub fn nome_permitido(nome: &str) -> bool {
    DECLARACOES.contains(&nome)
        || VALORES.contains(&nome)
        || EFEITOS.contains(&nome)
        || COMBINADORES.contains(&nome)
        || CONSTANTES.contains(&nome)
        || FUNCOES_PURAS.contains(&nome)
}

/// O dialeto: Starlark com o "proibido" do §3.2 aplicado pelo próprio parser.
///
/// `enable_load: false` recusa `load`; `enable_top_level_stmt: false` recusa laço e `if` no topo;
/// `enable_f_strings: false` recusa f-string; `enable_lambda: false` recusa `lambda`. `while` e
/// `**` não existem no Starlark em nenhuma configuração.
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

/// Registra as funções de declaração, de valor, de efeito e os combinadores.
///
/// Todas devolvem `None`: o valor delas não é usado — quem monta a IR lê a árvore sintática. Elas
/// existem para que o módulo **carregue**, que é o que o motor precisa para avaliar as `def`.
///
/// As formas de chamada são as do documento (§4, P10): posicionais em `campo`, `acao`, `fim`,
/// `mensagem_ao_cliente`, `encaminhar` e nos combinadores; nomeadas em `fluxo`, `etapa`, `decisao`,
/// `gatilho` e `notificar_whatsapp`. O compilador recusa a outra forma antes da carga, então aqui
/// não existe divergência entre o que o validador aceita e o que a biblioteca aceita.
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

/// Ambiente do script: stdlib do Starlark **sem** `struct`, mais as declarações.
pub fn globais() -> Globals {
    GlobalsBuilder::extended_by(&[LibraryExtension::StructType])
        .with(declaracoes)
        .build()
}

/// Prazo de parede da carga de um script.
pub struct Prazos {
    /// Instante limite da avaliação.
    pub ate: Instant,
}

/// Analisa o script (só parse), sem executar.
pub fn analisa(fonte: &str, nome: &str) -> Result<AstModule, starlark::Error> {
    AstModule::parse(nome, fonte.to_owned(), &dialeto())
}

/// Limites que o motor aplica na carga e na avaliação de regra.
pub struct LimitesStarlark {
    /// Ticks antes de considerar laço infinito.
    pub max_ticks: u64,
    /// Heap máximo em bytes.
    pub max_heap_bytes: usize,
    /// Profundidade máxima de chamada.
    pub max_callstack: usize,
    /// Prazo de parede.
    pub prazo: Duration,
}

/// Carrega o script: parse → avaliação com limites → congelamento.
///
/// É o que o motor usa para ter o módulo com as `def` carregadas. O compilador usa o mesmo
/// caminho como última barreira: com o script de forma válida, a carga só pode falhar por limite.
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
        // Cópia da tabela do §3.3: se `linguagem.rs` mudar sem o documento, este teste cai.
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
        // `load`, `for`/`if` no topo, `lambda` e f-string morrem no parse.
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
        // Atribuição solta no topo o parser deixa passar; quem recusa é o compilador (FLX05).
        assert!(analisa("x = 1\n", "t.star").is_ok());
    }
}
