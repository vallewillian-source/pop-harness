
use blake3 as _;

pub mod cache;
pub mod compilador;
pub mod diagnostico;
pub mod diff;
pub mod formatador;
pub mod ir;
pub mod linguagem;
pub mod motor;
pub mod prompt;
pub mod regras;
pub mod validador;

pub use cache::{
    CUSTO_FIXO_DA_ENTRADA, CacheRevisoes, ChaveRevisao, ESPERA_MAXIMA_DO_LOTE, FATOR_DO_PESO,
    Metricas, TETO_PADRAO_MB, peso_da_ir, sha256_da_fonte,
};
pub use compilador::{Compilado, FalhaCarregamento, Limites, carregar, compilar};
pub use diagnostico::{Code, Diagnostic, Severity};
pub use diff::{Impacto, Mudanca, TipoMudanca, diferenca};
pub use formatador::{FalhaDeFormato, LARGURA, formatar, formatar_com, imprimir};
pub use ir::Ir;
pub use motor::{
    Campos, Contexto, EfeitoProduzido, Estado, Evento, MAX_SALTOS_DECISAO, Recibo, Resultado,
    abandonar, atores_exigidos, campos_faltando, dados_da_etapa, efeitos_de_entrada, executar_acao,
    iniciar,
};
pub use prompt::{TETO_DO_BLOCO, bloco_da_etapa, renderizar_bloco};
pub use regras::{Avaliador, Dados, ErroRegra, LimitesRegra, avaliar, dados_de_json};
pub use validador::{Departamento, MAX_NOS, Resolucao, resolver_departamentos, validar};
