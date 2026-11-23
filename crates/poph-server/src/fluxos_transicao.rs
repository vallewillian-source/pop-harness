//! Ponte entre o motor de fluxos (`poph-fluxos`, puro) e a gravação atômica (`poph-store`, FX-20).
//!
//! O motor diz **o que** aconteceu (estado novo, eventos, efeitos, pedido de encerramento). O
//! repositório grava no vocabulário das colunas, sem importar o motor (regra de camadas do
//! `AGENTS.md`). Quem traduz de um para o outro é este módulo, e só ele: a API de ações humanas
//! (FX-30) e o adaptador do turno usam estas funções, para não existirem dois formatos.
//!
//! Nenhuma decisão do motor é refeita aqui: são conversões de vocabulário.

use poph_fluxos::motor::{Estado as EstadoMotor, Resultado as ResultadoMotor};
use poph_store::{EstadoDaExecucao, EventoDaTransicao, TransicaoDeFluxo};

use crate::fluxos_efeitos::efeito_da_transicao;

/// O estado do motor no vocabulário das colunas de `fluxo_execucoes` (e da etapa de entrada).
///
/// O nó vai pelo **id**: é o que a coluna `no_atual` guarda. `chaves` não é persistido (a
/// idempotência é o índice único de `ficha_eventos`, FX-20).
pub fn estado_para_banco(estado: &EstadoMotor) -> EstadoDaExecucao {
    EstadoDaExecucao {
        status: estado.status,
        no_atual: estado.no_atual.id().to_owned(),
        dados_json: serde_json::Value::Object(estado.dados.clone()).to_string(),
        atores_que_agiram: estado.atores_que_agiram.clone(),
        resultado: estado.resultado.clone(),
        motivo: estado.motivo.clone(),
    }
}

/// O desfecho de uma passagem do motor (ação, abandono) pronto para a operação atômica do store.
///
/// `repetida` vem do motor, que não guarda as chaves: quem confere repetição de verdade é o store
/// (pela marca em `ficha_eventos`), e a API checa antes de chamar o motor (ver `api::fichas`).
pub fn transicao_do_motor(resultado: &ResultadoMotor) -> TransicaoDeFluxo {
    TransicaoDeFluxo {
        novo_estado: estado_para_banco(&resultado.novo_estado),
        eventos: resultado.eventos.iter().map(evento_para_banco).collect(),
        efeitos: resultado.efeitos.iter().map(efeito_da_transicao).collect(),
        encerra_atendimento: resultado.encerra_atendimento,
        repetida: resultado.repetida,
    }
}

/// Um evento do motor no vocabulário de `ficha_eventos`.
fn evento_para_banco(evento: &poph_fluxos::motor::Evento) -> EventoDaTransicao {
    EventoDaTransicao {
        tipo: evento.tipo,
        ator_tipo: evento.ator,
        payload_json: evento.payload.to_string(),
    }
}
