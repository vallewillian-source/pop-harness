//! Caixa de entrada (idempotência da mensagem recebida) e caixa de saída (fila de envio).
//!
//! Entrada: a chave única `(integration_id, external_message_id)` descarta a reentrega do webhook
//! sem perder o ACK. Saída: a chave `(turn_id, seq)` faz o reenvio na retomada não duplicar o que
//! já saiu (D11: entrega pelo menos uma vez).

use poph_types::{AuditId, ConversationId, InboxId, IntegrationId, OrgId, OutboxId, TurnId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{
    AttachmentRecord, InboxItem, Mensagem, NewChannelInboxItem, NewDeviceMessage, NewInboxItem,
    NewInboxItemWithMessage, NewOutboxItem, Origin, OutboxItem, OutboxState, Received,
    ReceivedFromDevice, ReceivedWithMessage, Role,
};
use crate::repos::conversas::gravar_nome_cliente;
use crate::repos::id_da_coluna;
use crate::repos::mensagens::Mensagens;
use poph_types::MessageId;

const COLUNAS_ENTRADA: &str = "id, org_id, integration_id, external_message_id, payload_json,
    received_at, processed_at, attempts, last_error";

pub(crate) const COLUNAS_SAIDA: &str =
    "id, org_id, conversation_id, turn_id, seq, kind, payload_json, state, attempts, last_error,
     created_at, sent_at";

/// Monta o item de saída recém-inserido (`pending`, sem tentativa).
///
/// Público no crate porque a gravação atômica da resposta do agente com o item da fila mora em
/// [`Mensagens`](crate::Mensagens) e precisa do mesmo formato de linha (HR-57).
pub(crate) fn item_de_saida_novo(
    id: OutboxId,
    org: OrgId,
    nova: NewOutboxItem,
    criado_em: i64,
) -> OutboxItem {
    OutboxItem {
        id,
        org_id: org,
        conversation_id: nova.conversation_id,
        turn_id: nova.turn_id,
        seq: nova.seq,
        tipo: nova.tipo,
        content_json: nova.content_json,
        estado: OutboxState::Pending,
        tentativas: 0,
        ultimo_erro: None,
        created_at: criado_em,
        enviado_em: None,
    }
}

/// Acesso à caixa de entrada.
pub struct CaixaEntrada<'a>(pub(crate) &'a Store);

impl CaixaEntrada<'_> {
    /// Grava o evento recebido. Reentrega do mesmo id externo devolve `Duplicado` e o item já
    /// gravado, sem erro (o webhook responde OK nos dois casos).
    pub fn receber(&self, org: OrgId, nova: NewInboxItem) -> Resultado<(Received, InboxItem)> {
        let id = InboxId::generate();
        let agora = self.0.now();
        let texto_org = org.to_string();
        let NewInboxItem {
            integration_id,
            id_externo,
            event_json: evento_json,
        } = nova;
        let texto_integracao = integration_id.to_string();
        self.0.write(move |conn| {
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO inbox
                        (id, org_id, integration_id, external_message_id, payload_json,
                         received_at, processed_at, attempts, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL)",
                    params![
                        id.to_string(),
                        &texto_org,
                        &texto_integracao,
                        &id_externo,
                        &evento_json,
                        agora
                    ],
                )
                .map_err(StoreError::from)?;

            if inseriu == 0 {
                let existente = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUNAS_ENTRADA} FROM inbox
                              WHERE integration_id = ?1 AND external_message_id = ?2"
                        ),
                        params![&texto_integracao, &id_externo],
                        linha_entrada,
                    )
                    .optional()
                    .map_err(StoreError::from)?;
                return match existente {
                    Some(item) => Ok((Received::Duplicate, item)),
                    None => Err(StoreError::Internal(
                        "inserção da entrada ignorada, mas a original não foi encontrada"
                            .to_owned(),
                    )),
                };
            }

            Ok((
                Received::New,
                InboxItem {
                    id,
                    org_id: org,
                    integration_id,
                    id_externo,
                    event_json: evento_json,
                    received_at: agora,
                    processed_at: None,
                    tentativas: 0,
                    ultimo_erro: None,
                },
            ))
        })
    }

    /// Marca como processado (com o erro, quando falhou). `processado_em` preenchido = sai da fila.
    pub fn marcar_processado(&self, org: OrgId, id: InboxId, erro: Option<&str>) -> Resultado<()> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let erro = erro.map(str::to_owned);
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE inbox
                    SET processed_at = ?3, attempts = attempts + 1, last_error = ?4
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, agora, erro],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    /// Grava a mensagem do cliente **e** o item da entrada na mesma transação.
    ///
    /// É o caminho das integrações: a resposta para o canal (o ACK) só deve sair depois que este
    /// método retorna, porque ele só retorna depois do commit. Se a mesma mensagem externa chegar
    /// de novo, a `inbox` recusa pela chave única e devolve o item e a mensagem já gravados.
    pub fn receber_com_mensagem(
        &self,
        org: OrgId,
        nova: NewInboxItemWithMessage,
    ) -> Resultado<ReceivedWithMessage> {
        let id = InboxId::generate();
        let agora = self.0.now();
        let texto_org = org.to_string();
        let NewInboxItemWithMessage {
            integration_id,
            id_externo,
            event_json: evento_json,
            conversation_id,
            message_id: mensagem_id,
            texto,
            attachments_json: anexos_json,
        } = nova;
        let texto_integracao = integration_id.to_string();
        let texto_conversa = conversation_id.to_string();
        self.0.write(move |conn| {
            // 1. A entrada primeiro: é ela que decide se a entrega é nova (chave única).
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO inbox
                        (id, org_id, integration_id, external_message_id, payload_json,
                         received_at, processed_at, attempts, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL)",
                    params![
                        id.to_string(),
                        &texto_org,
                        &texto_integracao,
                        &id_externo,
                        &evento_json,
                        agora
                    ],
                )
                .map_err(StoreError::from)?;

            // 2. A mensagem do cliente, na mesma transação (idempotente por conversa + id externo).
            conn.execute(
                "INSERT OR IGNORE INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at)
                 VALUES (?1, ?2, ?3, 'in', ?4, ?5, ?6, ?7, NULL, ?8)",
                params![
                    mensagem_id.to_string(),
                    &texto_org,
                    &texto_conversa,
                    Role::User.as_text(),
                    &texto,
                    &anexos_json,
                    &id_externo,
                    agora
                ],
            )
            .map_err(StoreError::from)?;

            let mensagem = conn
                .query_row(
                    "SELECT id, org_id, conversation_id, direction, role, text, attachments_json,
                            external_id, turn_id, created_at, origin, author_name
                       FROM messages
                      WHERE org_id = ?1 AND conversation_id = ?2 AND external_id = ?3",
                    params![&texto_org, &texto_conversa, &id_externo],
                    crate::repos::mensagens::linha,
                )
                .optional()
                .map_err(StoreError::from)?
                .ok_or_else(|| {
                    StoreError::Internal(
                        "mensagem recém-gravada não encontrada na mesma transação".to_owned(),
                    )
                })?;

            if inseriu == 0 {
                let existente = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUNAS_ENTRADA} FROM inbox
                              WHERE integration_id = ?1 AND external_message_id = ?2"
                        ),
                        params![&texto_integracao, &id_externo],
                        linha_entrada,
                    )
                    .optional()
                    .map_err(StoreError::from)?
                    .ok_or_else(|| {
                        StoreError::Internal(
                            "inserção da entrada ignorada, mas a original não foi encontrada"
                                .to_owned(),
                        )
                    })?;
                return Ok(ReceivedWithMessage {
                    situacao: Received::Duplicate,
                    item: existente,
                    mensagem,
                });
            }

            Ok(ReceivedWithMessage {
                situacao: Received::New,
                item: InboxItem {
                    id,
                    org_id: org,
                    integration_id,
                    id_externo,
                    event_json: evento_json,
                    received_at: agora,
                    processed_at: None,
                    tentativas: 0,
                    ultimo_erro: None,
                },
                mensagem,
            })
        })
    }

    /// Quantos itens da org ainda não foram processados (tamanho da fila de entrada).
    pub fn contar_pendentes(&self, org: OrgId) -> Resultado<u64> {
        let texto_org = org.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM inbox WHERE org_id = ?1 AND processed_at IS NULL",
                params![texto_org],
                |linha| linha.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(StoreError::from)
        })
    }

    /// Itens prontos para despachar: ainda não processados e sem adiamento, ou com o adiamento
    /// vencido. É o que o despacho lê; o item adiado por falha do modelo só volta depois do instante.
    pub fn listar_pendentes(&self, org: OrgId, limite: u32) -> Resultado<Vec<InboxItem>> {
        self.listar(org, limite, true)
    }

    /// Itens ainda não processados, adiados ou não. O turno usa esta lista para concluir os itens
    /// que atendeu: um adiado que entrou no grupo de um turno novo também precisa sair da fila.
    pub fn listar_abertos(&self, org: OrgId, limite: u32) -> Resultado<Vec<InboxItem>> {
        self.listar(org, limite, false)
    }

    fn listar(&self, org: OrgId, limite: u32, so_vencidos: bool) -> Resultado<Vec<InboxItem>> {
        let texto_org = org.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        let agora = self.0.now();
        let filtrar = i64::from(so_vencidos);
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_ENTRADA} FROM inbox
                  WHERE org_id = ?1 AND processed_at IS NULL
                    AND (?3 = 0 OR next_attempt_at IS NULL OR next_attempt_at <= ?4)
                  ORDER BY received_at, id LIMIT ?2"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, limite, filtrar, agora], linha_entrada)?;
            let mut itens = Vec::new();
            for linha in linhas {
                itens.push(linha?);
            }
            Ok(itens)
        })
    }

    /// Adia itens pendentes para depois de `proxima_em` (ms). Cada item conta mais uma tentativa e
    /// guarda o motivo. Não marca nada como processado: o item continua na fila, só não é despachado
    /// antes do instante. Item que já foi concluído fica como está.
    pub fn adiar(&self, org: OrgId, ids: &[InboxId], proxima_em: i64, erro: &str) -> Resultado<()> {
        let texto_org = org.to_string();
        let textos: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let erro = erro.to_owned();
        self.0.write(move |conn| {
            for texto_id in &textos {
                conn.execute(
                    "UPDATE inbox
                        SET attempts = attempts + 1, next_attempt_at = ?3, last_error = ?4
                      WHERE org_id = ?1 AND id = ?2 AND processed_at IS NULL",
                    params![texto_org, texto_id, proxima_em, erro],
                )
                .map_err(StoreError::from)?;
            }
            Ok(())
        })
    }

    /// Tentativas já contadas de cada item (para o turno decidir se ainda há nova tentativa).
    pub fn tentativas(&self, org: OrgId, ids: &[InboxId]) -> Resultado<Vec<u32>> {
        let mut total = Vec::with_capacity(ids.len());
        for id in ids {
            total.push(self.obter(org, *id)?.tentativas);
        }
        Ok(total)
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: InboxId) -> Resultado<InboxItem> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS_ENTRADA} FROM inbox WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha_entrada,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrado, "item de entrada", &id.to_string())
    }
}

/// Acesso à caixa de saída.
pub struct CaixaSaida<'a>(pub(crate) &'a Store);

impl CaixaSaida<'_> {
    /// Quantos itens da saída (texto, arquivo) este turno já enfileirou. Serve para não repetir um
    /// turno que já mandou algo ao cliente.
    pub fn contar_do_turno(&self, org: OrgId, turno: TurnId) -> Resultado<u64> {
        let texto_org = org.to_string();
        let texto_turno = turno.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND turn_id = ?2",
                params![texto_org, texto_turno],
                |linha| linha.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(StoreError::from)
        })
    }

    /// Enfileira a saída. Devolve `false` quando `(turn_id, seq)` já existia (nada foi duplicado).
    pub fn enfileirar(&self, org: OrgId, nova: NewOutboxItem) -> Resultado<(bool, OutboxItem)> {
        let id = OutboxId::generate();
        let agora = self.0.now();
        let texto_org = org.to_string();
        let NewOutboxItem {
            conversation_id,
            turn_id,
            seq,
            tipo,
            content_json: conteudo_json,
        } = nova;
        let texto_turno = turn_id.to_string();
        self.0.write(move |conn| {
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO outbox
                        (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                         attempts, last_error, created_at, sent_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, NULL, ?8, NULL)",
                    params![
                        id.to_string(),
                        &texto_org,
                        conversation_id.to_string(),
                        &texto_turno,
                        i64::from(seq),
                        tipo,
                        &conteudo_json,
                        agora
                    ],
                )
                .map_err(StoreError::from)?;

            if inseriu == 0 {
                let existente = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUNAS_SAIDA} FROM outbox WHERE turn_id = ?1 AND seq = ?2"
                        ),
                        params![&texto_turno, i64::from(seq)],
                        linha_saida,
                    )
                    .optional()
                    .map_err(StoreError::from)?;
                return match existente {
                    Some(item) => Ok((false, item)),
                    None => Err(StoreError::Internal(
                        "inserção da saída ignorada, mas a original não foi encontrada".to_owned(),
                    )),
                };
            }

            Ok((
                true,
                item_de_saida_novo(
                    id,
                    org,
                    NewOutboxItem {
                        conversation_id,
                        turn_id,
                        seq,
                        tipo,
                        content_json: conteudo_json,
                    },
                    agora,
                ),
            ))
        })
    }

    /// Muda o estado do item e conta uma tentativa (atalho de baixo nível).
    ///
    /// O fluxo de entrada/saída (HR-12) **não** usa este método: ele usa
    /// [`iniciar_envio`](Self::iniciar_envio), [`confirmar_envio`](Self::confirmar_envio) e
    /// [`falhar_envio`](Self::falhar_envio), que separam "tentativa" de "transição".
    pub fn mudar_estado(
        &self,
        org: OrgId,
        id: OutboxId,
        estado: OutboxState,
        erro: Option<&str>,
    ) -> Resultado<()> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let erro = erro.map(str::to_owned);
        let enviado = matches!(estado, OutboxState::Sent).then_some(agora);
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE outbox
                    SET state = ?3, attempts = attempts + 1, last_error = ?4,
                        sent_at = COALESCE(?5, sent_at)
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, estado.as_text(), erro, enviado],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    /// Marca o começo de **uma** tentativa de envio: `sending`, mais uma tentativa, e devolve o
    /// número de tentativas já gastas.
    ///
    /// É a transição que precisa estar gravada **antes** do efeito externo: se o processo morrer
    /// depois dela, a retomada encontra o item em `sending` e reenvia (D11).
    pub fn iniciar_envio(&self, org: OrgId, id: OutboxId) -> Resultado<u32> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.write(move |conn| {
            let tentativas = conn
                .query_row(
                    "UPDATE outbox
                        SET state = 'sending', attempts = attempts + 1, last_error = NULL
                      WHERE org_id = ?1 AND id = ?2 AND state IN ('pending', 'sending')
                      RETURNING attempts",
                    params![texto_org, texto_id],
                    |linha| linha.get::<_, i64>(0),
                )
                .optional()
                .map_err(StoreError::from)?
                .ok_or_else(|| {
                    StoreError::Internal(format!(
                        "item de saída `{texto_id}` não estava aguardando envio"
                    ))
                })?;
            Ok(tentativas.max(0) as u32)
        })
    }

    /// Confirma a entrega: `sending` → `sent`, com o carimbo de envio.
    pub fn confirmar_envio(&self, org: OrgId, id: OutboxId) -> Resultado<()> {
        self.confirmar_envio_com_id(org, id, None)
    }

    /// Confirma a entrega guardando o id que o canal deu à mensagem (MX-40).
    ///
    /// `id_externo` `None` grava o envio sem id — é o caso do canal que não devolve id (plugin) ou
    /// de um envio em que a resposta não trouxe um. Nome vazio conta como ausente. A coluna só é
    /// escrita com a confirmação: envio que falha não deixa id.
    pub fn confirmar_envio_com_id(
        &self,
        org: OrgId,
        id: OutboxId,
        id_externo: Option<&str>,
    ) -> Resultado<()> {
        let agora = self.0.now();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let id_externo = id_externo
            .map(str::trim)
            .filter(|valor| !valor.is_empty())
            .map(str::to_owned);
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE outbox
                    SET state = 'sent', sent_at = ?3, last_error = NULL,
                        external_message_id = COALESCE(?4, external_message_id)
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, agora, id_externo],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    /// Registra a falha do envio. Transitória volta para a fila (`sending`, para a retomada e o
    /// backoff pegarem); definitiva vai para `failed` com o motivo legível.
    pub fn falhar_envio(
        &self,
        org: OrgId,
        id: OutboxId,
        motivo: &str,
        definitivo: bool,
    ) -> Resultado<()> {
        let estado = if definitivo {
            OutboxState::Failed
        } else {
            OutboxState::Sending
        };
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let motivo = motivo.to_owned();
        self.0.write(move |conn| {
            conn.execute(
                "UPDATE outbox SET state = ?3, last_error = ?4 WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, estado.as_text(), motivo],
            )
            .map(|_| ())
            .map_err(StoreError::from)
        })
    }

    /// Quantos itens da org estão num estado (tamanho da fila por estado).
    pub fn contar_por_estado(&self, org: OrgId, estado: OutboxState) -> Resultado<u64> {
        let texto_org = org.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND state = ?2",
                params![texto_org, estado.as_text()],
                |linha| linha.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(StoreError::from)
        })
    }

    /// Itens que precisam de envio: `pending` e `sending` (o `sending` é o reenvio da retomada, D11).
    pub fn listar_pendentes(&self, org: OrgId, limite: u32) -> Resultado<Vec<OutboxItem>> {
        let texto_org = org.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_SAIDA} FROM outbox
                  WHERE org_id = ?1 AND state IN ('pending', 'sending')
                  ORDER BY created_at, id LIMIT ?2"
            ))?;
            let linhas = consulta.query_map(params![texto_org, limite], linha_saida)?;
            let mut itens = Vec::new();
            for linha in linhas {
                itens.push(linha?);
            }
            Ok(itens)
        })
    }

    /// Id que o canal deu à mensagem enviada (MX-40), se o item já foi confirmado com id.
    ///
    /// Fica fora de [`ItemSaida`] de propósito: o formato do item é usado por outros crates e a
    /// coluna é detalhe do canal. A fase 2 lê por aqui para reconhecer o eco do próprio envio.
    pub fn id_externo(&self, org: OrgId, id: OutboxId) -> Resultado<Option<String>> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.read(move |conn| {
            conn.query_row(
                "SELECT external_message_id FROM outbox WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
                |linha| linha.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|valor| valor.flatten())
            .map_err(StoreError::from)
        })
    }

    /// Algum envio nosso já confirmado com este id externo? (MX-41)
    ///
    /// É a pergunta que reconhece o **eco** da Evolution: o que o Harness mandou voltar como
    /// `fromMe` no webhook traz o mesmo id que a MX-40 guardou ao confirmar o envio. Sem casar com
    /// um envio nosso, um `fromMe` é mensagem digitada no aparelho do cliente.
    pub fn enviado_com_id_externo(&self, org: OrgId, id_externo: &str) -> Resultado<bool> {
        let texto_org = org.to_string();
        let id_externo = id_externo.trim().to_owned();
        self.0.read(move |conn| {
            let achado: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM outbox
                      WHERE org_id = ?1 AND external_message_id = ?2
                      LIMIT 1",
                    params![texto_org, id_externo],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            Ok(achado.is_some())
        })
    }

    /// Itens de saída de **arquivo** de uma conversa, do mais antigo para o mais novo (HR-46d).
    ///
    /// Só `kind = 'file'`: texto e presença não entram. É a leitura que deixa o Console saber qual
    /// arquivo o agente mandou em cada turno — o `payload_json` fica bruto aqui; quem decide o que
    /// expor na resposta é o chamador (o servidor nunca devolve o `blob_ref`).
    pub fn listar_arquivos_da_conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
        limite: u32,
    ) -> Resultado<Vec<OutboxItem>> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS_SAIDA} FROM outbox
                  WHERE org_id = ?1 AND conversation_id = ?2 AND kind = 'file'
                  ORDER BY created_at, id LIMIT ?3"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, texto_conversa, limite], linha_saida)?;
            let mut itens = Vec::new();
            for linha in linhas {
                itens.push(linha?);
            }
            Ok(itens)
        })
    }

    /// Busca pelo id dentro da org.
    pub fn obter(&self, org: OrgId, id: OutboxId) -> Resultado<OutboxItem> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS_SAIDA} FROM outbox WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha_saida,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(encontrado, "item de saída", &id.to_string())
    }
}

fn linha_entrada(linha: &Row<'_>) -> rusqlite::Result<InboxItem> {
    Ok(InboxItem {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        integration_id: id_da_coluna::<IntegrationId>(linha, 2)?,
        id_externo: linha.get(3)?,
        event_json: linha.get(4)?,
        received_at: linha.get(5)?,
        processed_at: linha.get(6)?,
        tentativas: linha.get::<_, u32>(7)?,
        ultimo_erro: linha.get(8)?,
    })
}

pub(crate) fn linha_saida(linha: &Row<'_>) -> rusqlite::Result<OutboxItem> {
    Ok(OutboxItem {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        conversation_id: id_da_coluna::<ConversationId>(linha, 2)?,
        turn_id: id_da_coluna::<TurnId>(linha, 3)?,
        seq: linha.get::<_, u32>(4)?,
        tipo: linha.get(5)?,
        content_json: linha.get(6)?,
        estado: OutboxState::from_text(&linha.get::<_, String>(7)?).map_err(|erro| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(erro),
            )
        })?,
        tentativas: linha.get(8)?,
        ultimo_erro: linha.get(9)?,
        created_at: linha.get(10)?,
        enviado_em: linha.get(11)?,
    })
}

// ───────────────────── Entrada de canal (HR-26b) ─────────────────────

impl CaixaEntrada<'_> {
    /// Entrega já gravada (mesma integração e mesmo id externo), com a mensagem.
    ///
    /// O servidor consulta isto **antes** de baixar mídia: reentrega do webhook não baixa o
    /// arquivo de novo nem grava nada.
    pub fn entrega_recebida(
        &self,
        integracao: poph_types::IntegrationId,
        id_externo: &str,
    ) -> Resultado<Option<ReceivedWithMessage>> {
        let texto_integracao = integracao.to_string();
        let id_externo = id_externo.to_owned();
        self.0.read(move |conn| {
            let item = conn
                .query_row(
                    &format!(
                        "SELECT {COLUNAS_ENTRADA} FROM inbox
                          WHERE integration_id = ?1 AND external_message_id = ?2"
                    ),
                    params![&texto_integracao, &id_externo],
                    linha_entrada,
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some(item) = item else {
                return Ok(None);
            };
            let Some(mensagem) = mensagem_da_entrega(conn, &texto_integracao, &id_externo)? else {
                return Ok(None);
            };
            Ok(Some(ReceivedWithMessage {
                situacao: Received::Duplicate,
                item,
                mensagem,
            }))
        })
    }

    /// Grava a mensagem recebida de um canal, com a conversa e os anexos, numa transação só.
    ///
    /// Ordem dentro do comando: entrada (chave única), conversa do contato (acha a aberta ou
    /// transferida; sem nenhuma, abre uma), mensagem, anexos e o vínculo deles na mensagem. Um erro
    /// em qualquer passo desfaz tudo: a reentrega do webhook então entra de novo, inteira. Quando a
    /// entrega já existe, devolve o mesmo item e a mesma mensagem, sem gravar nada.
    pub fn receber_do_canal(
        &self,
        org: OrgId,
        nova: NewChannelInboxItem,
    ) -> Resultado<ReceivedWithMessage> {
        let id_entrada = InboxId::generate();
        let agora = self.0.now();
        let texto_org = org.to_string();
        let NewChannelInboxItem {
            integration_id,
            agent_id,
            canal,
            id_externo,
            peer,
            event_json: evento_json,
            texto,
            anexos,
            customer_name: nome_cliente,
        } = nova;
        let texto_integracao = integration_id.to_string();
        self.0.write(move |conn| {
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO inbox
                        (id, org_id, integration_id, external_message_id, payload_json,
                         received_at, processed_at, attempts, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL)",
                    params![
                        id_entrada.to_string(),
                        &texto_org,
                        &texto_integracao,
                        &id_externo,
                        &evento_json,
                        agora
                    ],
                )
                .map_err(StoreError::from)?;

            if inseriu == 0 {
                let item = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUNAS_ENTRADA} FROM inbox
                              WHERE integration_id = ?1 AND external_message_id = ?2"
                        ),
                        params![&texto_integracao, &id_externo],
                        linha_entrada,
                    )
                    .map_err(StoreError::from)?;
                let mensagem = mensagem_da_entrega(conn, &texto_integracao, &id_externo)?
                    .ok_or_else(|| {
                        StoreError::Internal(
                            "entrada já gravada sem a mensagem correspondente".to_owned(),
                        )
                    })?;
                return Ok(ReceivedWithMessage {
                    situacao: Received::Duplicate,
                    item,
                    mensagem,
                });
            }

            let conversa_id = conversa_do_contato(
                conn,
                &texto_org,
                &texto_integracao,
                &agent_id.to_string(),
                &canal,
                &peer,
                agora,
            )?;

            // Nome do cliente (MX-32), no caminho que o WhatsApp usa de fato. Savepoint próprio:
            // falha aqui vira log e a mensagem entra do mesmo jeito.
            if let Some(nome) = nome_cliente.as_deref() {
                gravar_nome_sem_derrubar(conn, &texto_org, &conversa_id, nome, agora);
            }

            let mensagem_id = poph_types::MessageId::generate().to_string();
            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at)
                 VALUES (?1, ?2, ?3, 'in', ?4, ?5, '[]', ?6, NULL, ?7)",
                params![
                    &mensagem_id,
                    &texto_org,
                    &conversa_id,
                    Role::User.as_text(),
                    &texto,
                    &id_externo,
                    agora
                ],
            )
            .map_err(StoreError::from)?;

            // O turno lê da fila a conversa e a mensagem (HR-26h). Os dois ids só existem agora,
            // dentro da transação: o payload recebe `conversa` e `mensagem`, e o evento do canal
            // fica junto em `canal`, sem mudança.
            let payload_turno =
                payload_da_entrada_do_canal(&conversa_id, &mensagem_id, &evento_json)?;
            conn.execute(
                "UPDATE inbox SET payload_json = ?2 WHERE id = ?1",
                params![id_entrada.to_string(), payload_turno],
            )
            .map_err(StoreError::from)?;

            let mut ids_anexos: Vec<String> = Vec::with_capacity(anexos.len());
            for anexo in &anexos {
                let id_anexo = poph_types::AttachmentId::generate().to_string();
                conn.execute(
                    "INSERT INTO attachments
                        (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path,
                         extracted_text, extraction_status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        &id_anexo,
                        &texto_org,
                        &mensagem_id,
                        &anexo.name,
                        &anexo.mime,
                        anexo.size_bytes as i64,
                        &anexo.sha256,
                        &anexo.blob_path,
                        &anexo.extracted_text,
                        anexo.extraction_status,
                        agora
                    ],
                )
                .map_err(StoreError::from)?;
                ids_anexos.push(id_anexo);
            }
            let anexos_json = serde_json::to_string(&ids_anexos).map_err(|erro| {
                StoreError::Internal(format!("lista de anexos não serializa: {erro}"))
            })?;
            conn.execute(
                "UPDATE messages SET attachments_json = ?2 WHERE id = ?1",
                params![&mensagem_id, anexos_json],
            )
            .map_err(StoreError::from)?;
            conn.execute(
                "UPDATE conversations SET last_message_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![&conversa_id, agora],
            )
            .map_err(StoreError::from)?;

            let mensagem = conn
                .query_row(
                    "SELECT id, org_id, conversation_id, direction, role, text, attachments_json,
                            external_id, turn_id, created_at, origin, author_name
                       FROM messages WHERE id = ?1",
                    params![&mensagem_id],
                    crate::repos::mensagens::linha,
                )
                .map_err(StoreError::from)?;

            Ok(ReceivedWithMessage {
                situacao: Received::New,
                item: InboxItem {
                    id: id_entrada,
                    org_id: org,
                    integration_id,
                    id_externo,
                    event_json: evento_json,
                    received_at: agora,
                    processed_at: None,
                    tentativas: 0,
                    ultimo_erro: None,
                },
                mensagem,
            })
        })
    }

    /// Grava a mensagem **digitada no aparelho** do cliente no WhatsApp (MX-41), numa escrita só.
    ///
    /// O que entra na mesma transação: a mensagem (`direction out`, `role assistant`,
    /// `origin device`, `external_id` = id do WhatsApp, **sem** `turn_id`), a passagem da conversa
    /// de `open` para `transferred` (a versão sobe uma vez só, como na MX-04) e a auditoria
    /// `device_takeover`. O que **não** entra, de propósito:
    ///
    /// - a `inbox`: um item lá é o que o laço do turno pega, e o agente não deve responder a uma
    ///   mensagem que o próprio cliente já mandou;
    /// - a `outbox`: a mensagem já saiu pelo celular; reenviá-la duplicaria no WhatsApp do cliente.
    ///
    /// A conversa é a do contato, com a mesma regra da entrada do canal (aberta ou transferida;
    /// fechada não conta, abre-se outra), e a Ficha nasce junto quando ela é aberta aqui. Reentrega
    /// do mesmo id externo devolve `Duplicado` com a mensagem já gravada, sem gravar nada de novo.
    pub fn receber_do_aparelho(
        &self,
        org: OrgId,
        nova: NewDeviceMessage,
    ) -> Resultado<ReceivedFromDevice> {
        let agora = self.0.now();
        let NewDeviceMessage {
            integration_id,
            agent_id,
            canal,
            peer,
            id_externo,
            texto,
        } = nova;
        let texto_org = org.to_string();
        let texto_integracao = integration_id.to_string();
        let texto_agente = agent_id.to_string();
        self.0.write(move |conn| {
            // Reentrega: o mesmo id externo nesta integração devolve a mensagem que já está lá,
            // em qualquer conversa dela (a conversa anterior pode já ter fechado).
            let ja_gravada: Option<(String, String)> = conn
                .query_row(
                    "SELECT m.id, m.conversation_id
                       FROM messages m
                       JOIN conversations c ON c.org_id = m.org_id AND c.id = m.conversation_id
                      WHERE m.org_id = ?1 AND m.external_id = ?2 AND c.integration_id = ?3
                      LIMIT 1",
                    params![&texto_org, &id_externo, &texto_integracao],
                    |linha| Ok((linha.get::<_, String>(0)?, linha.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            if let Some((mensagem_id, conversa_id)) = ja_gravada {
                let mensagem = conn
                    .query_row(
                        "SELECT id, org_id, conversation_id, direction, role, text,
                                attachments_json, external_id, turn_id, created_at, origin,
                                author_name
                           FROM messages WHERE id = ?1",
                        params![&mensagem_id],
                        crate::repos::mensagens::linha,
                    )
                    .map_err(StoreError::from)?;
                return Ok(ReceivedFromDevice {
                    situacao: Received::Duplicate,
                    conversa: conversa_de_texto(&conversa_id)?,
                    mensagem,
                    just_transferred: false,
                });
            }

            let conversa = conversa_do_contato(
                conn,
                &texto_org,
                &texto_integracao,
                &texto_agente,
                &canal,
                &peer,
                agora,
            )?;

            let mensagem_id = poph_types::MessageId::generate().to_string();
            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, 'out', ?4, ?5, '[]', ?6, NULL, ?7, ?8, NULL)",
                params![
                    &mensagem_id,
                    &texto_org,
                    &conversa,
                    Role::Assistant.as_text(),
                    &texto,
                    &id_externo,
                    agora,
                    Origin::Device.as_text()
                ],
            )
            .map_err(StoreError::from)?;

            // Pausa o agente na mesma regra da resposta humana (MX-04): a conversa aberta passa a
            // `transferred` e a versão sobe uma vez só; já transferida, só o carimbo de atividade.
            let transferiu_agora = conn
                .execute(
                    "UPDATE conversations
                        SET status = 'transferred', version = version + 1,
                            last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2 AND status = 'open'",
                    params![&texto_org, &conversa, agora],
                )
                .map_err(StoreError::from)?
                > 0;
            if !transferiu_agora {
                conn.execute(
                    "UPDATE conversations SET last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2",
                    params![&texto_org, &conversa, agora],
                )
                .map_err(StoreError::from)?;
            }

            let detalhe = serde_json::json!({
                "transferiu": transferiu_agora,
                "mensagem_id": mensagem_id,
            });
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'system', NULL, 'device_takeover', 'conversation', ?3, ?4, ?5)",
                params![
                    AuditId::generate().to_string(),
                    &texto_org,
                    &conversa,
                    detalhe.to_string(),
                    agora
                ],
            )
            .map_err(StoreError::from)?;

            let mensagem = conn
                .query_row(
                    "SELECT id, org_id, conversation_id, direction, role, text, attachments_json,
                            external_id, turn_id, created_at, origin, author_name
                       FROM messages WHERE id = ?1",
                    params![&mensagem_id],
                    crate::repos::mensagens::linha,
                )
                .map_err(StoreError::from)?;

            Ok(ReceivedFromDevice {
                situacao: Received::New,
                conversa: conversa_de_texto(&conversa)?,
                mensagem,
                just_transferred: transferiu_agora,
            })
        })
    }
}

/// Acha (ou abre) a conversa do contato de um canal: a `open` ou `transferred` mais recente, e,
/// quando não há nenhuma, uma nova com a Ficha do atendimento.
///
/// A regra é a mesma para a mensagem recebida do cliente (HR-26b) e para a digitada no aparelho
/// (MX-41): fechada não conta, porque conversa encerrada não se reabre por mensagem.
fn conversa_do_contato(
    conn: &rusqlite::Connection,
    org: &str,
    integracao: &str,
    agente: &str,
    canal: &str,
    peer: &str,
    agora: i64,
) -> Resultado<String> {
    let existente: Option<String> = conn
        .query_row(
            "SELECT id FROM conversations
              WHERE org_id = ?1 AND integration_id = ?2 AND external_peer = ?3
                AND status IN ('open', 'transferred')
              ORDER BY updated_at DESC, id LIMIT 1",
            params![org, integracao, peer],
            |linha| linha.get::<_, String>(0),
        )
        .optional()
        .map_err(StoreError::from)?;
    if let Some(id) = existente {
        return Ok(id);
    }
    let id = ConversationId::generate().to_string();
    conn.execute(
        "INSERT INTO conversations
            (id, org_id, agent_id, integration_id, channel, external_peer, status,
             target_department_id, summary, version, created_at, updated_at,
             last_message_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open',
             (SELECT default_department_id FROM agents
               WHERE org_id = ?2 AND id = ?3),
             NULL, 1, ?7, ?7, NULL)",
        params![&id, org, agente, integracao, canal, peer, agora],
    )
    .map_err(StoreError::from)?;
    // A Ficha do atendimento nasce com a conversa, no mesmo savepoint (PRD L4, FX-07). Este é o
    // caminho que o WhatsApp usa de fato: se a Ficha fosse criada só em `Conversas::criar`, o
    // atendimento real ficaria sem registro.
    super::fichas::criar_ficha_da_conversa(conn, org, &id, agente, canal, Some(peer), agora)?;
    Ok(id)
}

/// Id de conversa lido do banco, como tipo do domínio.
fn conversa_de_texto(valor: &str) -> Resultado<ConversationId> {
    valor
        .parse()
        .map_err(|_| StoreError::Internal("conversa gravada sem id válido".to_owned()))
}

/// Grava o nome do cliente num savepoint próprio, dentro da transação da mensagem (MX-32).
///
/// Falha vira log com código curto (sem o nome, sem texto do banco) e **não** desfaz a entrada: o
/// próximo `pushName` do cliente tenta de novo.
fn gravar_nome_sem_derrubar(
    conn: &rusqlite::Connection,
    org: &str,
    conversa: &str,
    nome: &str,
    agora: i64,
) {
    if let Err(erro) = conn.execute_batch("SAVEPOINT nome_cliente") {
        let codigo = codigo_erro_nome(&StoreError::Sqlite(erro));
        tracing::warn!(target: "poph_store", org, conversa, codigo, "não consegui abrir o savepoint do nome do cliente; a mensagem segue");
        return;
    }
    match gravar_nome_cliente(conn, org, conversa, nome, agora) {
        Ok(_) => {
            if let Err(erro) = conn.execute_batch("RELEASE nome_cliente") {
                let codigo = codigo_erro_nome(&StoreError::Sqlite(erro));
                tracing::warn!(target: "poph_store", org, conversa, codigo, "não consegui fechar o savepoint do nome do cliente; a mensagem segue");
            }
        }
        Err(erro) => {
            let codigo = codigo_erro_nome(&erro);
            let _ = conn.execute_batch("ROLLBACK TO nome_cliente; RELEASE nome_cliente");
            tracing::warn!(target: "poph_store", org, conversa, codigo, "não consegui gravar o nome do cliente; a mensagem segue");
        }
    }
}

/// Código curto do erro de gravação do nome, para log.
fn codigo_erro_nome(erro: &StoreError) -> &'static str {
    match erro {
        StoreError::Domain(_) => "dominio",
        StoreError::Sqlite(_) => "banco",
        StoreError::WriterClosed => "escritor_encerrado",
        _ => "armazenamento",
    }
}

/// Mensagem do cliente que veio de uma entrega (integração + id externo), se já foi gravada.
/// Payload da `inbox` para uma entrada de canal: o formato que o turno lê (`conversa`, `mensagem`)
/// mais o evento original do canal em `canal`. O turno ignora campos que não conhece.
fn payload_da_entrada_do_canal(
    conversa: &str,
    mensagem: &str,
    evento_json: &str,
) -> Resultado<String> {
    let canal: serde_json::Value = serde_json::from_str(evento_json).map_err(|erro| {
        StoreError::Internal(format!("evento do canal não lê como JSON: {erro}"))
    })?;
    serde_json::to_string(&serde_json::json!({
        "conversa": conversa,
        "mensagem": mensagem,
        "canal": canal,
    }))
    .map_err(|erro| StoreError::Internal(format!("payload da entrada não serializa: {erro}")))
}

fn mensagem_da_entrega(
    conn: &rusqlite::Connection,
    integracao: &str,
    id_externo: &str,
) -> Resultado<Option<Mensagem>> {
    conn.query_row(
        "SELECT m.id, m.org_id, m.conversation_id, m.direction, m.role, m.text, m.attachments_json,
                m.external_id, m.turn_id, m.created_at, m.origin, m.author_name
           FROM messages m
           JOIN conversations c ON c.id = m.conversation_id
          WHERE c.integration_id = ?1 AND m.external_id = ?2 AND m.direction = 'in'
          LIMIT 1",
        params![integracao, id_externo],
        crate::repos::mensagens::linha,
    )
    .optional()
    .map_err(StoreError::from)
}

impl Mensagens<'_> {
    /// Anexos de uma mensagem, na ordem em que chegaram.
    pub fn anexos(&self, org: OrgId, mensagem: MessageId) -> Resultado<Vec<AttachmentRecord>> {
        let texto_org = org.to_string();
        let texto_mensagem = mensagem.to_string();
        self.0.read(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT id, message_id, name, mime, size_bytes, sha256, blob_path,
                        extraction_status, extracted_text
                   FROM attachments
                  WHERE org_id = ?1 AND message_id = ?2
                  ORDER BY created_at, id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_mensagem], |linha| {
                Ok(AttachmentRecord {
                    id: id_da_coluna(linha, 0)?,
                    message_id: id_da_coluna(linha, 1)?,
                    name: linha.get(2)?,
                    mime: linha.get(3)?,
                    size_bytes: linha.get::<_, i64>(4)?.max(0) as u64,
                    sha256: linha.get(5)?,
                    blob_path: linha.get(6)?,
                    extraction_status: linha.get(7)?,
                    extracted_text: linha.get(8)?,
                })
            })?;
            let mut anexos = Vec::new();
            for linha in linhas {
                anexos.push(linha?);
            }
            Ok(anexos)
        })
    }
}
