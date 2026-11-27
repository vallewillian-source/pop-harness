use blake3 as _;

pub mod cache;
pub mod compilador;
pub mod diagnostics;
pub mod diff;
pub mod formatador;
pub mod ir;
pub mod language;
pub mod motor;
pub mod prompt;
pub mod regras;
pub mod validador;

pub use cache::{
    DEFAULT_CAP_MB, ENTRY_FIXED_COST, MAX_BATCH_WAIT, Metrics, RevisionCache, RevisionKey,
    WEIGHT_FACTOR, entry_weight, sha256_of_source,
};
pub use compilador::{Compiled, Limites, LoadFailure, compile, load};
pub use diagnostics::{Code, Diagnostic, Severity};
pub use diff::{Impacto, Mudanca, TipoMudanca, diferenca};
pub use formatador::{FalhaDeFormato, LARGURA, formatar, formatar_com, imprimir};
pub use ir::Ir;
pub use motor::{
    Campos, Contexto, EfeitoProduzido, Estado, Evento, MAX_SALTOS_DECISAO, Recibo, Resultado,
    abandonar, atores_exigidos, campos_faltando, dados_da_etapa, efeitos_de_entrada, executar_acao,
    iniciar,
};
pub use prompt::{BLOCK_CAP, render_block, stage_block};
pub use regras::{Dados, RuleError, RuleEvaluator, RuleLimits, evaluate, snapshot_from_json};
pub use validador::{Departamento, MAX_NOS, Resolucao, resolver_departamentos, validar};
