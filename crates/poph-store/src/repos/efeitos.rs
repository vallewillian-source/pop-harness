//! Fila de efeitos do fluxo (FX-21): a leitura e as transições de estado de `fluxo_efeitos`.
//!
//! A FX-20 enfileira o efeito na mesma escrita da transição, com a chave do motor (`nó + índice`)
//! protegida pelo índice único `(org_id, execucao_id, chave)`. Aqui ficam só as operações que o
//! **entregador** precisa para tirar o efeito da fila:
//!
//! | Operação | Transição |
//! |---|---|
//! | [`Efeitos::listar_pendentes`] | lê o que está por entregar (e o que já venceu a espera) |
//! | [`Efeitos::iniciar_envio`] | `pendente`/`enviando` → `enviando`, mais uma tentativa |
//! | [`Efeitos::confirmar_envio`] | → `enviado`, com o carimbo de entrega |
//! | [`Efeitos::falhar_envio`] | transitória → `pendente` com espera; definitiva → `falhou` |
//! | [`Efeitos::adiar`] | empurra a espera **sem** contar tentativa (falta integração) |
//!
//! # Por que `enviando` volta para a fila
//!
//! A tentativa é gravada **antes** da chamada externa (mesma regra da fila de saída, D11): se o
//! processo morrer entre a marcação e a confirmação, a passagem seguinte encontra o efeito em
//! `enviando` e entrega de novo. É entrega **pelo menos uma vez**: uma queda nessa janela pode
//! duplicar a mensagem, e isso é aceito e documentado (PRD, riscos).
//!
//! # O que esta camada não faz
//!
//! - **Rede**: nada aqui fala com a Evolution ou com o Synapse. Quem entrega é o laço do servidor
//!   (FX-21), e ele só chama estes métodos para marcar o que aconteceu.
//! - **Ficha**: o evento `efeito_enviado`/`efeito_falhou` é do repositório de Fichas
//!   ([`crate::Fichas::registrar_evento`]), não da fila.
//! - **Política de tentativas**: o teto e a espera crescente são de quem entrega (o [`Backoff`] da
//!   fila de saída); aqui só se grava o que ele decidir.
//!
//! [`Backoff`]: https://docs.rs/tokio/latest/tokio/time

use poph_types::{EffectId, EffectState, OrgId};
use rusqlite::{OptionalExtension, params};

use super::execucoes::{COLUNAS_EFEITO, linha_efeito};
use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::EfeitoDoFluxo;

/// Acesso à fila de efeitos do fluxo.
pub struct Efeitos<'a>(pub(crate) &'a Store);

impl Efeitos<'_> {
    /// Orgs com efeito aguardando entrega agora.
    ///
    /// É o ponto de partida do laço: ele não tem lista fixa de orgs (diferente da saída do
    /// WhatsApp, que parte das instâncias configuradas) — qualquer org com efeito na fila é
    /// atendida, porque `encaminhar` e `mensagem_ao_cliente` não dependem de integração.
    pub fn orgs_com_pendentes(&self) -> Resultado<Vec<OrgId>> {
        let agora = self.0.agora();
        self.0.leitura(move |conn| {
            let mut consulta = conn
                .prepare(
                    "SELECT DISTINCT org_id FROM fluxo_efeitos
                      WHERE estado IN ('pendente', 'enviando')
                        AND (proxima_tentativa IS NULL OR proxima_tentativa <= ?1)
                      ORDER BY org_id",
                )
                .map_err(ErroStore::from)?;
            let linhas = consulta
                .query_map(params![agora], |linha| linha.get::<_, String>(0))
                .map_err(ErroStore::from)?;
            let mut orgs = Vec::new();
            for linha in linhas {
                let texto = linha.map_err(ErroStore::from)?;
                orgs.push(texto.parse().map_err(ErroStore::Dominio)?);
            }
            Ok(orgs)
        })
    }

    /// Efeitos da org que podem ser tentados agora, do mais antigo para o mais novo.
    ///
    /// Inclui `enviando` (o reenvio da retomada, D11) e respeita a espera: efeito adiado por falha
    /// transitória só volta quando `proxima_tentativa` chega.
    pub fn listar_pendentes(&self, org: OrgId, limite: u32) -> Resultado<Vec<EfeitoDoFluxo>> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        self.0.leitura(move |conn| {
            let mut consulta = conn
                .prepare(&format!(
                    "SELECT {COLUNAS_EFEITO} FROM fluxo_efeitos
                      WHERE org_id = ?1 AND estado IN ('pendente', 'enviando')
                        AND (proxima_tentativa IS NULL OR proxima_tentativa <= ?2)
                      ORDER BY created_at, id LIMIT ?3"
                ))
                .map_err(ErroStore::from)?;
            let linhas = consulta
                .query_map(params![texto_org, agora, limite], linha_efeito)
                .map_err(ErroStore::from)?;
            let mut efeitos = Vec::new();
            for linha in linhas {
                efeitos.push(linha.map_err(ErroStore::from)?);
            }
            Ok(efeitos)
        })
    }

    /// Um efeito pelo id (para o relatório e para o teste).
    pub fn obter(&self, org: OrgId, id: EffectId) -> Resultado<EfeitoDoFluxo> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS_EFEITO} FROM fluxo_efeitos WHERE org_id = ?1 AND id = ?2"
                ),
                params![texto_org, texto_id],
                linha_efeito,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "efeito", &id.to_string())
    }

    /// Marca o começo de **uma** tentativa de entrega e devolve o número de tentativas já gastas.
    ///
    /// Precisa estar gravada **antes** da chamada externa: é o que faz a queda no meio reenviar.
    pub fn iniciar_envio(&self, org: OrgId, id: EffectId) -> Resultado<u32> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            let tentativas = conn
                .query_row(
                    "UPDATE fluxo_efeitos
                        SET estado = 'enviando', tentativas = tentativas + 1, ultimo_erro = NULL
                      WHERE org_id = ?1 AND id = ?2 AND estado IN ('pendente', 'enviando')
                      RETURNING tentativas",
                    params![texto_org, texto_id],
                    |linha| linha.get::<_, i64>(0),
                )
                .optional()
                .map_err(ErroStore::from)?
                .ok_or_else(|| {
                    ErroStore::Interno(format!("efeito `{texto_id}` não estava aguardando entrega"))
                })?;
            Ok(tentativas.max(0) as u32)
        })
    }

    /// Confirma a entrega: → `enviado`, com o carimbo. A espera pendente é limpa.
    pub fn confirmar_envio(&self, org: OrgId, id: EffectId) -> Resultado<()> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE fluxo_efeitos
                    SET estado = 'enviado', enviado_em = ?3, proxima_tentativa = NULL
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, agora],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
        })
    }

    /// Registra a falha da tentativa.
    ///
    /// Transitória volta para `pendente` com a espera em `proxima_tentativa` (o laço não insiste no
    /// ciclo seguinte); definitiva vai para `falhou` com o motivo legível. Em nenhum dos dois casos
    /// a transição do fluxo é desfeita (PRD L11): o efeito é consequência, não parte da transição.
    pub fn falhar_envio(
        &self,
        org: OrgId,
        id: EffectId,
        motivo: &str,
        definitivo: bool,
        proxima_tentativa: Option<i64>,
    ) -> Resultado<()> {
        let estado = if definitivo {
            EffectState::Failed
        } else {
            EffectState::Pending
        };
        let proxima = if definitivo { None } else { proxima_tentativa };
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let motivo = motivo.to_owned();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE fluxo_efeitos
                    SET estado = ?3, ultimo_erro = ?4, proxima_tentativa = ?5, enviado_em = NULL
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, estado.as_text(), motivo, proxima],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
        })
    }

    /// Empurra a tentativa **sem** contar tentativa: é o caso do efeito que está esperando algo que
    /// não é falha (a integração do WhatsApp não está configurada, o turno da conversa ainda está
    /// em andamento). O efeito continua `pendente` e o limite de tentativas não é consumido.
    pub fn adiar(&self, org: OrgId, id: EffectId, proxima_tentativa: i64) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE fluxo_efeitos
                    SET proxima_tentativa = ?3
                  WHERE org_id = ?1 AND id = ?2 AND estado = 'pendente'",
                params![texto_org, texto_id, proxima_tentativa],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
        })
    }
}
