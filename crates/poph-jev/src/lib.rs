//! Jev (TypeSafe System One) no POP Harness — HR-28.
//!
//! O Jev decide o que é escolha entre opções conhecidas (transferir, esperar, precisa de base,
//! departamento) em ~255–315 ms e custo de entrada, sem gastar o modelo de texto. Aqui ficam:
//!
//! - [`cliente`]: `POST /v1/systemone` com uma retentativa, prazo por tentativa e a chave só no
//!   cabeçalho — mais o [`cliente::JevFalso`] que os testes usam;
//! - [`estado`]: o que o Jev enxerga (conversa recente, política da empresa, capacidades) e o hash
//!   que identifica a decisão no cache;
//! - [`gate`]: o molde `Gate` (pergunta, aplicabilidade, limiares, interpretação, fallback);
//! - [`gates`]: G1 a G9 (G1–G4 obrigatórios da V1, G5 na HR-33, G6 na HR-34, G7 na HR-35, G8 na
//!   HR-36 e G9 na FX-22, que escolhe o fluxo a iniciar);
//! - [`motor`]: [`motor::MotorJev`], que manda **um lote só** com as perguntas de todos os gates
//!   ligados, distribui as respostas, guarda no cache e nunca derruba o turno;
//! - [`handoff`]: a segunda pergunta do G1 (tipo de risco) e o texto de orientação ao cliente
//!   antes da transferência (HR-29);
//! - [`flags`]: flags global e por agente (`agents.jev_json`);
//! - [`eval`]: conjunto rotulado e medição (`poph jev eval`).
//!
//! O que o gate faz é **decidir**. Transferir, esperar, pular a busca e escolher o departamento são
//! das tarefas HR-29 a HR-36, que medem antes de ligar.

pub mod cache;
pub mod cliente;
pub mod estado;
pub mod eval;
pub mod flags;
pub mod gate;
pub mod gates;
pub mod handoff;
pub mod motor;
pub mod tipos;

pub use cache::{CacheDecisoes, OpcoesCache};
pub use cliente::{ClienteJev, ConfigJev, JevFalso, Pedido, TransporteJev};
pub use estado::{
    CandidatoProduto, EstadoJev, FluxoJev, MAX_CANDIDATOS_PRODUTO, MAX_CHARS_ARQUIVO,
    MAX_CHARS_GATILHO, MAX_CHARS_MENSAGEM, MAX_CHARS_RESPOSTA, MAX_FLUXOS_LIBERADOS, MAX_MENSAGENS,
    MensagemJev, PapelJev,
};
pub use eval::{
    CasoRotulado, DetalheCaso, ErroConjunto, RelatorioEval, avaliar_conjunto, ler_conjunto,
};
pub use flags::{
    AcaoG7, AjusteGates, acao_g7_do_agente, despedida_do_agente, do_agente as ajuste_do_agente,
    resposta_g7_do_agente, resposta_segura_do_agente, situacoes_do_agente,
};
pub use gate::{CONFIANCA_PADRAO, Decisao, Gate, Limiares, MARGEM_PADRAO, OrigemDecisao};
pub use gates::{
    G1, G1_CONTINUAR, G1_TRANSFERIR, G2, G2_ESPERAR, G2_PROCESSAR, G3, G3_AMBOS, G3_BASE, G3_NADA,
    G3_PRODUTO, G3_TUDO, G4, G5, G5_NENHUM, G6, G6_APROVADA, G6_REVISAR, G6_REVISAR_ESCOPO,
    G6_REVISAR_LONGA, G6_REVISAR_PRECO, G6_REVISAR_TOM, G6_REVISAR_TRANSFERENCIA, G7, G7_ABUSO,
    G7_INJECAO, G7_NORMAL, G7_RESPOSTA_PADRAO, G8, G8_CONTINUA, G8_DESPEDIDA_PADRAO, G8_ENCERRADA,
    G9, G9_NENHUM, casar_candidato, criterio_do_fluxo, g6_orientacao, g6_precisa_revisar,
    g7_pede_acao,
};
pub use handoff::{
    Orientacao, PRAZO_PADRAO, RISCO_ANIMAL, RISCO_INGESTAO, RISCO_NENHUM, RISCO_OUTRO,
    RISCO_PERGUNTA_ID, ResultadoOrientacao, TIPOS_DE_RISCO, pergunta_risco,
};
pub use motor::{
    AvaliadorGates, MotorJev, OpcoesJev, RelatoChamada, ResultadoJev, TEMPO_MAXIMO_PADRAO,
};
pub use tipos::{ErroJev, IdGate, LoteResposta, Pergunta, Resposta, TipoPergunta, UsageTokens};
