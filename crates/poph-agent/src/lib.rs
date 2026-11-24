//! Estado quente e concorrência do POP Harness (`poph-agent`, decisão D-08).
//!
//! O que este crate faz (HR-11):
//! - [`compositor`]: o texto do prefixo estável do prompt, porte fiel do compositor do POP Atende
//!   (veio do `poph-core` na HR-11, porque é aqui que o turno vai morar a partir da HR-13);
//! - [`config_quente`]: cache da config do agente (agente + instruções + ferramentas + top-k),
//!   invalidado por versão e limitado por memória, particionado por org;
//! - [`atores`]: uma tarefa tokio e uma fila por conversa, com um turno por vez e descarte de
//!   ociosos;
//! - [`contexto`]: a janela de mensagens recentes com orçamento de tokens;
//! - [`limites`]: simultaneidade por org, por agente e por provedor, mais a cota por minuto da org;
//! - [`entrada_saida`]: a `inbox` idempotente, o despacho para o ator da conversa, a `outbox` com
//!   estados e retomada na subida, o backoff e o transporte de saída (HR-12);
//! - [`turno`]: o motor do turno (HR-13) — janela de agrupamento, prompt estável, laço de
//!   ferramentas, streaming, registro em `turns` e resposta pela `outbox`.
//!
//! O que ele **não** faz: o resumo rolante (HR-14), o RAG (HR-17), o Jev (HR-28 em diante) e o
//! transporte real de cada canal (HR-25/HR-26). Não depende de axum (D-08), então o turno é
//! testável pela CLI.

pub mod atores;
pub mod base_conhecimento;
pub mod biblioteca;
pub mod carrinho;
pub mod compositor;
pub mod config_quente;
pub mod contexto;
pub mod entrada_saida;
pub mod erro;
pub mod fila_integracao;
pub mod fluxos_transicao;
pub mod limites;
pub mod metricas;
pub mod produtos;
pub mod provedores;
pub mod resumo;
pub mod turno;

pub use atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
pub use base_conhecimento::{
    BaseConhecimento, BuscadorBase, ConsultarBase, FuturoBusca, ParametrosBusca, TrechoRecuperado,
    formatar_trechos,
};
pub use biblioteca::{EnviarArquivo, NOME_FERRAMENTA as NOME_ENVIAR_ARQUIVO, anexar_biblioteca};
pub use compositor::compor_instrucoes;
pub use config_quente::{AgenteQuente, ConfigQuente, OpcoesConfigQuente};
pub use contexto::{Janela, estimar_tokens, montar as montar_janela};
pub use entrada_saida::{
    Backoff, Estatisticas, EventoEntrada, FalhaEnvio, Fluxo, FuturoEnvio, Laco, MensagemRecebida,
    OpcoesFluxo, RelatoDespacho, RelatoEnvio, RelatoRetomada, TransporteSaida,
};
pub use erro::{ErroAgente, Resultado};
pub use fila_integracao::{
    ConversaDoCanal, EnvioPeloCanal, InboxDoCanal, RelatoEntrada, ResolvedorConversa,
    chave_idempotencia, comando_do_item,
};
pub use limites::{Limitador, OpcoesLimites, Vaga, VagaProvedor};
pub use metricas::MetricasG9;
pub use metricas::{MetricasFluxo, MetricasMemoria, MetricasTurno};
pub use produtos::{
    BuscadorProdutos, BuscarProduto, CatalogoProdutos, ParametrosProdutos, ProdutoEncontrado,
    formatar_produtos, preco_texto, registro_com,
};
pub use provedores::{ProvedorResolvido, Provedores};
pub use resumo::{OpcoesResumo, Resumo};
pub use turno::{
    ContextoFerramenta, Entrada, EventoTurno, FerramentaInterna, Motor, Observador, ObservadorNulo,
    OpcoesTurno, Pausa, PausaReal, ProcessadorTurno, RegistroFerramentas, ResumoTurno,
    SemFerramentas, Tempos, TransporteConsole,
};
