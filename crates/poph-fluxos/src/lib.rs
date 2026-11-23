//! Módulo de Fluxos e Fichas do POP Harness (FX-04: esqueleto do crate).
//!
//! Este crate é **puro**: não faz I/O, não fala com banco nem com rede. Ele compila o script de
//! um fluxo (Starlark, decidido na FX-01) para a estrutura compilada (IR `schema_versao = 1`,
//! `docs/FLUXOS_LINGUAGEM.md` §8), valida a estrutura, avalia as regras e conduz a execução.
//! Quem guarda os dados é o `poph-store` (tabelas `fichas`, `fluxos`, `fluxo_execucoes` etc.),
//! quem liga o motor no turno é o `poph-agent` e quem expõe a API é o `poph-server`.
//!
//! Dependências permitidas: só `poph-types` e a biblioteca de script. Este crate **não** importa
//! `poph-store`, `poph-agent`, `poph-server` nem outro crate do workspace.
//!
//! Módulos, e a tarefa que implementa cada um:
//!
//! - [`compilador`] — FX-10: script → IR, com diagnósticos de forma (feito).
//! - [`validador`] — FX-11: diagnósticos semânticos sobre a IR e resolução de departamentos (feito).
//! - [`regras`] — FX-12: combinadores e funções avançadas (`def`) (feito).
//! - [`motor`] — FX-13 (feito: iniciar, registrar campos, executar ação, avançar) e FX-14
//!   (decisões, fim, abandono e efeitos).
//! - [`prompt`] — FX-18: bloco do prompt e visualização da etapa.
//! - [`diff`] — FX-17: diferença entre revisões em linguagem comum.
//! - [`formatador`] — FX-16: forma canônica do script (script ⇄ estrutura) (feito).
//! - [`cache`] — FX-19: cache das revisões compiladas (LRU com teto em MB, single-flight e falha
//!   fechada).
//!
//! Os módulos ainda vazios existem para fixar o desenho e a fronteira: `compilador`, `validador` e
//! `regras` têm função pública.

// `blake3` vem junto do starlark e continua sem uso direto neste crate: a chave do cache usa
// `sha2`, que já roda no workspace (o mesmo `sha256` de `fluxo_revisoes.fonte_sha256`). O
// `use ... as _` só marca a dependência como usada, porque o lint `unused_dependencies` do Rust
// 1.100 quebra o clippy com `-D warnings`.
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
pub use diagnostico::{Codigo, Diagnostico, Severidade};
pub use diff::{Impacto, Mudanca, TipoMudanca, diferenca};
pub use formatador::{FalhaDeFormato, LARGURA, formatar, imprimir};
pub use ir::Ir;
pub use motor::{
    Campos, Contexto, EfeitoProduzido, Estado, Evento, MAX_SALTOS_DECISAO, Recibo, Resultado,
    abandonar, atores_exigidos, campos_faltando, dados_da_etapa, efeitos_de_entrada, executar_acao,
    iniciar,
};
pub use prompt::{TETO_DO_BLOCO, bloco_da_etapa, renderizar_bloco};
pub use regras::{Avaliador, Dados, ErroRegra, LimitesRegra, avaliar, dados_de_json};
pub use validador::{Departamento, MAX_NOS, Resolucao, resolver_departamentos, validar};
