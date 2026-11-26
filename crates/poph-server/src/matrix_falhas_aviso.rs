//! Aviso na sala quando a resposta humana não chega ao cliente (MX-19).
//!
//! Quando o atendente escreve no Element e a resposta não consegue sair pelo WhatsApp de vez (o
//! item da fila vira `failed`), o atendente precisa saber, na sala, que o cliente não recebeu.
//! Este serviço faz isso, uma vez por falha:
//!
//! 1. acha as falhas definitivas de resposta humana (item `failed` cuja mensagem é `origin =
//!    human`, pela junção por `turn_id`) das orgs com o módulo ligado, que ainda não têm desfecho;
//! 2. escreve na sala `Não consegui entregar sua mensagem ao cliente: <motivo curto>`, em `m.notice`
//!    (o classificador da MX-17 ignora aviso, então o aviso nunca vira resposta de volta);
//! 3. grava o desfecho em `matrix_avisos_falha`. Essa linha é o que impede o segundo aviso depois
//!    de reinício do processo: a passagem seguinte nem chega ao Synapse para aquela falha.
//!
//! ## Regras
//!
//! - **Falha transitória não avisa.** Um item em `sending` (tentativa que volta para a fila) não é
//!   definitivo, então não aparece. Se a tentativa seguinte der certo, nada é escrito na sala.
//! - **Aviso idempotente nos dois sentidos.** O `txnId` do aviso é derivado do item de saída, e a
//!   linha de desfecho é gravada **depois** de o Synapse aceitar o aviso. Se o processo morrer entre
//!   os dois passos, a passagem seguinte repete com o mesmo `txnId`, e o Synapse não duplica (o
//!   mesmo mecanismo que a MX-02 comprovou para o envio com `txnId` repetido).
//! - **Erro do Synapse transitório não marca nada.** A próxima passagem tenta de novo. Erro
//!   definitivo marca `recusado`: repetir não resolve, e fica registrado no log e nas métricas.
//! - **Conversa sem sala** marca `sem_sala`: não há onde avisar, e não se consulta de novo a cada
//!   passagem.
//! - **Nada de texto de cliente no aviso, no log ou na métrica.** O aviso leva só o motivo curto,
//!   escolhido por um código fixo a partir do último erro da fila. O erro bruto, que pode trazer
//!   dado do canal, é lido só para essa classificação.
//!
//! ## Fronteira
//!
//! A persistência entra pela porta [`PortaFalhasDeResposta`] (a mesma escolha das portas da MX-11,
//! MX-12, MX-15 e MX-18): a implementação real é [`PortaFalhasDoHarness`], sobre o `Store`; os
//! testes injetam uma porta que sabe falhar. O lado do Synapse é o [`AvisoNoSynapse`], sobre o
//! cliente da MX-09.
//!
//! ## Ligação
//!
//! Quem liga é o `matrix_inicio` (MX-20), junto com os outros laços: `AvisoDeFalhas::novo`,
//! `tokio::spawn` de [`AvisoDeFalhas::executar`] e `pedir_parada` no desligamento. Com o módulo
//! desligado não há passagem nenhuma.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use poph_matrix::{ClienteMatrix, ErroChamada, IdsFantasmas, erro::Classificacao};
use poph_store::{HumanResponseFailure, OutcomeNotice, Store, StoreError};
use poph_types::{ConversationId, OrgId, OutboxId};
use serde_json::json;

use crate::matrix_metricas::MetricasMatrix;
use crate::saida_whatsapp::MOTIVO_TENTATIVAS_ESGOTADAS;

/// Começo do aviso na sala. O motivo curto vem depois.
pub const PREFIXO_AVISO: &str = "Não consegui entregar sua mensagem ao cliente: ";

/// Quantas falhas uma passagem olha no máximo (todas as orgs somadas).
pub const LOTE_PADRAO: u32 = 50;

/// Intervalo entre passagens do laço de fundo. Falha de resposta é rara: cinco segundos basta para
/// o atendente ver o aviso logo depois da falha.
pub const PERIODO_PADRAO: Duration = Duration::from_secs(5);

/// Motivo curto do aviso, classificado a partir do último erro da fila de saída.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotivoFalha {
    /// O WhatsApp não respondeu depois de todas as tentativas.
    TentativasEsgotadas,
    /// O canal recusou o envio de vez (erro definitivo).
    RecusadoPeloCanal,
    /// A fila não guardou motivo nenhum (não deveria acontecer num item `failed`).
    NaoInformado,
}

impl MotivoFalha {
    /// Classifica pelo texto que a fila gravou. Só a comparação com a constante da fila é exata;
    /// qualquer outro texto de erro definitivo vira [`MotivoFalha::RecusadoPeloCanal`], sem que o
    /// texto em si saia daqui.
    pub fn classificar(ultimo_erro: Option<&str>) -> Self {
        match ultimo_erro {
            Some(texto) if texto == MOTIVO_TENTATIVAS_ESGOTADAS => Self::TentativasEsgotadas,
            Some(_) => Self::RecusadoPeloCanal,
            None => Self::NaoInformado,
        }
    }

    /// Código curto para log e para a coluna `motivo` de `matrix_avisos_falha`.
    pub fn codigo(self) -> &'static str {
        match self {
            Self::TentativasEsgotadas => "tentativas_esgotadas",
            Self::RecusadoPeloCanal => "recusado_pelo_canal",
            Self::NaoInformado => "nao_informado",
        }
    }

    /// Texto que vai à sala, depois do prefixo.
    pub fn texto(self) -> &'static str {
        match self {
            Self::TentativasEsgotadas => "o WhatsApp não respondeu depois de várias tentativas",
            Self::RecusadoPeloCanal => "o WhatsApp recusou o envio",
            Self::NaoInformado => "motivo não informado",
        }
    }
}

/// Corpo do aviso na sala para um motivo.
pub fn texto_do_aviso(motivo: MotivoFalha) -> String {
    format!("{PREFIXO_AVISO}{}", motivo.texto())
}

/// `txnId` do aviso de um item de saída: sempre o mesmo para o mesmo item, para o reenvio não
/// duplicar o aviso.
pub fn txn_do_aviso(item: OutboxId) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in item.to_string().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("ph_falha_{hash:016x}")
}

/// Sala e agente de uma conversa, o que o aviso precisa para sair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextoDaFalha {
    /// Id da sala no Synapse (`!…:servidor`).
    pub sala: String,
    /// Id do agente da conversa (para achar o fantasma que escreve o aviso).
    pub agente: String,
}

/// O que o aviso precisa do banco. Porta para o teste injetar falha.
pub trait PortaFalhasDeResposta: Send + Sync {
    /// Falhas definitivas de resposta humana sem desfecho, das orgs com o módulo ligado, até
    /// `limite` no total.
    fn pendentes(&self, limite: u32) -> Result<Vec<HumanResponseFailure>, StoreError>;

    /// Sala e agente da conversa, ou `None` quando a conversa não tem sala.
    fn contexto(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<ContextoDaFalha>, StoreError>;

    /// Grava o desfecho do aviso. Repetir a mesma gravação não muda nada.
    fn registrar(
        &self,
        org: OrgId,
        falha: OutboxId,
        desfecho: OutcomeNotice,
        motivo: MotivoFalha,
    ) -> Result<(), StoreError>;
}

/// A porta de verdade, sobre o banco do Harness.
#[derive(Debug, Clone)]
pub struct PortaFalhasDoHarness {
    store: Arc<Store>,
}

impl PortaFalhasDoHarness {
    /// Monta a porta sobre o banco.
    pub fn nova(store: Arc<Store>) -> Self {
        Self { store }
    }
}

impl PortaFalhasDeResposta for PortaFalhasDoHarness {
    fn pendentes(&self, limite: u32) -> Result<Vec<HumanResponseFailure>, StoreError> {
        let mut falhas = Vec::new();
        for estado in self.store.matrix().linked_orgs()? {
            let faltam = limite.saturating_sub(falhas.len() as u32);
            if faltam == 0 {
                break;
            }
            falhas.extend(
                self.store
                    .matrix()
                    .human_response_failures(estado.org_id, faltam)?,
            );
        }
        Ok(falhas)
    }

    fn contexto(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<ContextoDaFalha>, StoreError> {
        let Some(sala) = self.store.matrix().room(org, conversa)? else {
            return Ok(None);
        };
        let agente = self.store.conversations().get(org, conversa)?.agent_id;
        Ok(Some(ContextoDaFalha {
            sala: sala.room_id,
            agente: agente.to_string(),
        }))
    }

    fn registrar(
        &self,
        org: OrgId,
        falha: OutboxId,
        desfecho: OutcomeNotice,
        motivo: MotivoFalha,
    ) -> Result<(), StoreError> {
        self.store
            .matrix()
            .record_notice_outcome(org, falha, desfecho, motivo.codigo())
            .map(|_| ())
    }
}

/// Lado do Synapse: escreve o aviso na sala em nome do fantasma do agente.
///
/// Quem escreve é o fantasma do agente, como na criação da sala (MX-11) e nos avisos de estado
/// (MX-15): é o dono da sala e está dentro dela. O bot do AS não é membro das salas de atendimento
/// hoje, então não entra aqui.
pub struct AvisoNoSynapse {
    cliente: ClienteMatrix,
    ids: IdsFantasmas,
}

impl AvisoNoSynapse {
    /// Monta a partir do cliente da MX-09 e dos ids da instância.
    pub fn novo(cliente: ClienteMatrix, ids: IdsFantasmas) -> Self {
        Self { cliente, ids }
    }

    async fn escrever(
        &self,
        contexto: &ContextoDaFalha,
        txn: &str,
        corpo: &str,
    ) -> Result<(), ErroChamada> {
        let fantasma = self.ids.agente(&contexto.agente).map_err(|_| {
            ErroChamada::Configuracao("id de agente fora do padrão do módulo Matrix".to_owned())
        })?;
        // `m.notice`: o classificador da MX-17 ignora avisos, então o aviso não volta como resposta.
        self.cliente
            .enviar_evento(
                fantasma.user_id(),
                &contexto.sala,
                "m.room.message",
                txn,
                &json!({ "msgtype": "m.notice", "body": corpo }),
            )
            .await
            .map(|_| ())
    }
}

/// Resultado de uma passagem. Só contagens: nada de texto de cliente nem de sala.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoAviso {
    /// Avisos que entraram na sala.
    pub avisados: u32,
    /// Falhas sem sala: marcadas, sem aviso.
    pub sem_sala: u32,
    /// Falhas em que o Synapse recusou o aviso de vez.
    pub recusados: u32,
    /// Falhas que ficaram para a próxima passagem (Synapse fora do ar ou erro transitório).
    pub adiados: u32,
    /// Falhas que não puderam ser lidas ou marcadas agora (banco).
    pub erros_de_banco: u32,
}

/// O serviço de aviso de falha de resposta humana.
pub struct AvisoDeFalhas {
    porta: Arc<dyn PortaFalhasDeResposta>,
    aviso: AvisoNoSynapse,
    lote: u32,
    /// Números do módulo (MX-26). Sem injeção, cai num registro descartável.
    metricas: Arc<MetricasMatrix>,
    /// Pedido de parada do laço de fundo (`executar`).
    parar: watch::Sender<bool>,
}

impl AvisoDeFalhas {
    /// Monta o serviço com o lote padrão e métricas descartáveis.
    pub fn novo(porta: Arc<dyn PortaFalhasDeResposta>, aviso: AvisoNoSynapse) -> Self {
        let (parar, _) = watch::channel(false);
        Self {
            porta,
            aviso,
            lote: LOTE_PADRAO,
            metricas: MetricasMatrix::descartaveis(),
            parar,
        }
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }

    /// Falhas por passagem (teto de trabalho). Zero vale um.
    pub fn com_lote(mut self, lote: u32) -> Self {
        self.lote = lote.max(1);
        self
    }

    /// Uma passagem: avisa as falhas definitivas que ainda não têm desfecho.
    pub async fn passar(&self) -> RelatoAviso {
        let mut relato = RelatoAviso::default();
        let falhas = match self.porta.pendentes(self.lote) {
            Ok(falhas) => falhas,
            Err(_) => {
                tracing::warn!(
                    target: "poph_server::matrix_falhas_aviso",
                    motivo = "banco_indisponivel",
                    "não consegui listar as falhas de resposta humana; tenta na próxima passagem"
                );
                relato.erros_de_banco += 1;
                return relato;
            }
        };
        for falha in &falhas {
            if self.tratar(falha, &mut relato).await.is_err() {
                relato.erros_de_banco += 1;
            }
        }
        relato
    }

    /// Pede a parada do laço de fundo. Quem está no meio de uma passagem termina a falha atual.
    pub fn pedir_parada(&self) {
        self.parar.send_replace(true);
    }

    /// Laço de fundo: uma passagem a cada `intervalo`, até [`AvisoDeFalhas::pedir_parada`]. Erro
    /// não derruba o laço: a passagem seguinte tenta de novo.
    pub async fn executar(self: Arc<Self>, intervalo: Duration) {
        let mut parar = self.parar.subscribe();
        loop {
            if *parar.borrow() {
                break;
            }
            self.passar().await;
            tokio::select! {
                _ = tokio::time::sleep(intervalo) => {}
                _ = parar.changed() => break,
            }
        }
    }

    async fn tratar(
        &self,
        falha: &HumanResponseFailure,
        relato: &mut RelatoAviso,
    ) -> Result<(), StoreError> {
        let motivo = MotivoFalha::classificar(falha.ultimo_erro.as_deref());
        let Some(contexto) = self.porta.contexto(falha.org_id, falha.conversation)? else {
            self.porta.registrar(
                falha.org_id,
                falha.outbox_id,
                OutcomeNotice::WithoutRoom,
                motivo,
            )?;
            relato.sem_sala += 1;
            tracing::info!(
                target: "poph_server::matrix_falhas_aviso",
                org = %falha.org_id,
                conversa = %falha.conversation,
                saida = %falha.outbox_id,
                motivo = motivo.codigo(),
                "falha de resposta humana sem sala: nada a avisar"
            );
            return Ok(());
        };

        let txn = txn_do_aviso(falha.outbox_id);
        match self
            .aviso
            .escrever(&contexto, &txn, &texto_do_aviso(motivo))
            .await
        {
            Ok(()) => {
                self.porta.registrar(
                    falha.org_id,
                    falha.outbox_id,
                    OutcomeNotice::Notify,
                    motivo,
                )?;
                relato.avisados += 1;
                tracing::info!(
                    target: "poph_server::matrix_falhas_aviso",
                    org = %falha.org_id,
                    conversa = %falha.conversation,
                    saida = %falha.outbox_id,
                    motivo = motivo.codigo(),
                    "aviso de falha de resposta humana escrito na sala"
                );
            }
            Err(erro) if erro.classificacao() == Classificacao::Transitoria => {
                self.contar_erro(&erro);
                relato.adiados += 1;
                tracing::warn!(
                    target: "poph_server::matrix_falhas_aviso",
                    org = %falha.org_id,
                    saida = %falha.outbox_id,
                    motivo = motivo.codigo(),
                    classe = "transitoria",
                    "aviso de falha não escrito agora; tenta na próxima passagem"
                );
            }
            Err(erro) => {
                self.contar_erro(&erro);
                self.porta.registrar(
                    falha.org_id,
                    falha.outbox_id,
                    OutcomeNotice::Refused,
                    motivo,
                )?;
                relato.recusados += 1;
                tracing::warn!(
                    target: "poph_server::matrix_falhas_aviso",
                    org = %falha.org_id,
                    saida = %falha.outbox_id,
                    motivo = motivo.codigo(),
                    classe = "definitiva",
                    "o Synapse recusou o aviso de falha de resposta humana: não será repetido"
                );
            }
        }
        Ok(())
    }

    /// Conta a falha por classe (e o `429`), como o restante do módulo.
    fn contar_erro(&self, erro: &ErroChamada) {
        match erro.classificacao() {
            Classificacao::Transitoria => self.metricas.erros_transitorios.increment(1),
            Classificacao::Definitiva => self.metricas.erros_definitivos.increment(1),
        }
        if matches!(erro, ErroChamada::Resposta { status: 429, .. }) {
            self.metricas.respostas_429.increment(1);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn so_o_texto_exato_da_fila_vira_tentativas_esgotadas() {
        assert_eq!(
            MotivoFalha::classificar(Some(MOTIVO_TENTATIVAS_ESGOTADAS)),
            MotivoFalha::TentativasEsgotadas
        );
        assert_eq!(
            MotivoFalha::classificar(Some("a Evolution recusou o número 5511999990000")),
            MotivoFalha::RecusadoPeloCanal,
            "qualquer outro erro definitivo vira o motivo genérico, sem o texto"
        );
        assert_eq!(MotivoFalha::classificar(None), MotivoFalha::NaoInformado);
    }

    #[test]
    fn o_aviso_leva_so_o_motivo_curto() {
        assert_eq!(
            texto_do_aviso(MotivoFalha::RecusadoPeloCanal),
            "Não consegui entregar sua mensagem ao cliente: o WhatsApp recusou o envio"
        );
        for motivo in [
            MotivoFalha::TentativasEsgotadas,
            MotivoFalha::RecusadoPeloCanal,
            MotivoFalha::NaoInformado,
        ] {
            let texto = texto_do_aviso(motivo);
            assert!(texto.starts_with(PREFIXO_AVISO));
            assert!(!texto.contains('@'), "nada de id de usuário no aviso");
            assert!(!texto.contains("5511"), "nada de telefone no aviso");
        }
    }

    #[test]
    fn codigos_de_motivo_sao_curtos_e_distintos() {
        let codigos = [
            MotivoFalha::TentativasEsgotadas.codigo(),
            MotivoFalha::RecusadoPeloCanal.codigo(),
            MotivoFalha::NaoInformado.codigo(),
        ];
        for codigo in codigos {
            assert!(codigo.len() <= 64, "cabe na coluna `motivo`");
            assert!(codigo.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
        assert_ne!(codigos[0], codigos[1]);
        assert_ne!(codigos[1], codigos[2]);
    }

    #[test]
    fn txn_do_aviso_e_estavel_por_item_e_distinto_entre_itens() {
        let um = OutboxId::generate();
        let outro = OutboxId::generate();
        assert_eq!(txn_do_aviso(um), txn_do_aviso(um));
        assert_ne!(txn_do_aviso(um), txn_do_aviso(outro));
        assert!(txn_do_aviso(um).starts_with("ph_falha_"));
    }
}
