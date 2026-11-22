//! Caixa de entrada (idempotência da mensagem recebida) e caixa de saída (fila de envio).
//!
//! Entrada: a chave única `(integration_id, external_message_id)` descarta a reentrega do webhook
//! sem perder o ACK. Saída: a chave `(turn_id, seq)` faz o reenvio na retomada não duplicar o que
//! já saiu (D11: entrega pelo menos uma vez).

use poph_types::{ConversationId, InboxId, IntegrationId, OrgId, OutboxId, TurnId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{
    AnexoRegistro, EstadoSaida, ItemEntrada, ItemSaida, Mensagem, NovaEntrada, NovaEntradaCanal,
    NovaEntradaComMensagem, NovaSaida, Papel, Recebido, RecebidoComMensagem,
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
    nova: NovaSaida,
    criado_em: i64,
) -> ItemSaida {
    ItemSaida {
        id,
        org_id: org,
        conversation_id: nova.conversation_id,
        turn_id: nova.turn_id,
        seq: nova.seq,
        tipo: nova.tipo,
        conteudo_json: nova.conteudo_json,
        estado: EstadoSaida::Pendente,
        tentativas: 0,
        ultimo_erro: None,
        criado_em,
        enviado_em: None,
    }
}

/// Acesso à caixa de entrada.
pub struct CaixaEntrada<'a>(pub(crate) &'a Store);

impl CaixaEntrada<'_> {
    /// Grava o evento recebido. Reentrega do mesmo id externo devolve `Duplicado` e o item já
    /// gravado, sem erro (o webhook responde OK nos dois casos).
    pub fn receber(&self, org: OrgId, nova: NovaEntrada) -> Resultado<(Recebido, ItemEntrada)> {
        let id = InboxId::generate();
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let NovaEntrada {
            integration_id,
            id_externo,
            evento_json,
        } = nova;
        let texto_integracao = integration_id.to_string();
        self.0.escrita(move |conn| {
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
                .map_err(ErroStore::from)?;

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
                    .map_err(ErroStore::from)?;
                return match existente {
                    Some(item) => Ok((Recebido::Duplicado, item)),
                    None => Err(ErroStore::Interno(
                        "inserção da entrada ignorada, mas a original não foi encontrada"
                            .to_owned(),
                    )),
                };
            }

            Ok((
                Recebido::Novo,
                ItemEntrada {
                    id,
                    org_id: org,
                    integration_id,
                    id_externo,
                    evento_json,
                    recebido_em: agora,
                    processado_em: None,
                    tentativas: 0,
                    ultimo_erro: None,
                },
            ))
        })
    }

    /// Marca como processado (com o erro, quando falhou). `processado_em` preenchido = sai da fila.
    pub fn marcar_processado(&self, org: OrgId, id: InboxId, erro: Option<&str>) -> Resultado<()> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let erro = erro.map(str::to_owned);
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE inbox
                    SET processed_at = ?3, attempts = attempts + 1, last_error = ?4
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, agora, erro],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
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
        nova: NovaEntradaComMensagem,
    ) -> Resultado<RecebidoComMensagem> {
        let id = InboxId::generate();
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let NovaEntradaComMensagem {
            integration_id,
            id_externo,
            evento_json,
            conversation_id,
            mensagem_id,
            texto,
            anexos_json,
        } = nova;
        let texto_integracao = integration_id.to_string();
        let texto_conversa = conversation_id.to_string();
        self.0.escrita(move |conn| {
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
                .map_err(ErroStore::from)?;

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
                    Papel::Usuario.como_texto(),
                    &texto,
                    &anexos_json,
                    &id_externo,
                    agora
                ],
            )
            .map_err(ErroStore::from)?;

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
                .map_err(ErroStore::from)?
                .ok_or_else(|| {
                    ErroStore::Interno(
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
                    .map_err(ErroStore::from)?
                    .ok_or_else(|| {
                        ErroStore::Interno(
                            "inserção da entrada ignorada, mas a original não foi encontrada"
                                .to_owned(),
                        )
                    })?;
                return Ok(RecebidoComMensagem {
                    situacao: Recebido::Duplicado,
                    item: existente,
                    mensagem,
                });
            }

            Ok(RecebidoComMensagem {
                situacao: Recebido::Novo,
                item: ItemEntrada {
                    id,
                    org_id: org,
                    integration_id,
                    id_externo,
                    evento_json,
                    recebido_em: agora,
                    processado_em: None,
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
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM inbox WHERE org_id = ?1 AND processed_at IS NULL",
                params![texto_org],
                |linha| linha.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(ErroStore::from)
        })
    }

    /// Itens prontos para despachar: ainda não processados e sem adiamento, ou com o adiamento
    /// vencido. É o que o despacho lê; o item adiado por falha do modelo só volta depois do instante.
    pub fn listar_pendentes(&self, org: OrgId, limite: u32) -> Resultado<Vec<ItemEntrada>> {
        self.listar(org, limite, true)
    }

    /// Itens ainda não processados, adiados ou não. O turno usa esta lista para concluir os itens
    /// que atendeu: um adiado que entrou no grupo de um turno novo também precisa sair da fila.
    pub fn listar_abertos(&self, org: OrgId, limite: u32) -> Resultado<Vec<ItemEntrada>> {
        self.listar(org, limite, false)
    }

    fn listar(&self, org: OrgId, limite: u32, so_vencidos: bool) -> Resultado<Vec<ItemEntrada>> {
        let texto_org = org.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        let agora = self.0.agora();
        let filtrar = i64::from(so_vencidos);
        self.0.leitura(move |conn| {
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
        self.0.escrita(move |conn| {
            for texto_id in &textos {
                conn.execute(
                    "UPDATE inbox
                        SET attempts = attempts + 1, next_attempt_at = ?3, last_error = ?4
                      WHERE org_id = ?1 AND id = ?2 AND processed_at IS NULL",
                    params![texto_org, texto_id, proxima_em, erro],
                )
                .map_err(ErroStore::from)?;
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
    pub fn obter(&self, org: OrgId, id: InboxId) -> Resultado<ItemEntrada> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS_ENTRADA} FROM inbox WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha_entrada,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "item de entrada", &id.to_string())
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
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND turn_id = ?2",
                params![texto_org, texto_turno],
                |linha| linha.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(ErroStore::from)
        })
    }

    /// Enfileira a saída. Devolve `false` quando `(turn_id, seq)` já existia (nada foi duplicado).
    pub fn enfileirar(&self, org: OrgId, nova: NovaSaida) -> Resultado<(bool, ItemSaida)> {
        let id = OutboxId::generate();
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let NovaSaida {
            conversation_id,
            turn_id,
            seq,
            tipo,
            conteudo_json,
        } = nova;
        let texto_turno = turn_id.to_string();
        self.0.escrita(move |conn| {
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
                .map_err(ErroStore::from)?;

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
                    .map_err(ErroStore::from)?;
                return match existente {
                    Some(item) => Ok((false, item)),
                    None => Err(ErroStore::Interno(
                        "inserção da saída ignorada, mas a original não foi encontrada".to_owned(),
                    )),
                };
            }

            Ok((
                true,
                item_de_saida_novo(
                    id,
                    org,
                    NovaSaida {
                        conversation_id,
                        turn_id,
                        seq,
                        tipo,
                        conteudo_json,
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
        estado: EstadoSaida,
        erro: Option<&str>,
    ) -> Resultado<()> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let erro = erro.map(str::to_owned);
        let enviado = matches!(estado, EstadoSaida::Enviado).then_some(agora);
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE outbox
                    SET state = ?3, attempts = attempts + 1, last_error = ?4,
                        sent_at = COALESCE(?5, sent_at)
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, estado.como_texto(), erro, enviado],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
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
        self.0.escrita(move |conn| {
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
                .map_err(ErroStore::from)?
                .ok_or_else(|| {
                    ErroStore::Interno(format!(
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
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let id_externo = id_externo
            .map(str::trim)
            .filter(|valor| !valor.is_empty())
            .map(str::to_owned);
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE outbox
                    SET state = 'sent', sent_at = ?3, last_error = NULL,
                        external_message_id = COALESCE(?4, external_message_id)
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, agora, id_externo],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
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
            EstadoSaida::Falhou
        } else {
            EstadoSaida::Enviando
        };
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let motivo = motivo.to_owned();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE outbox SET state = ?3, last_error = ?4 WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, estado.como_texto(), motivo],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
        })
    }

    /// Quantos itens da org estão num estado (tamanho da fila por estado).
    pub fn contar_por_estado(&self, org: OrgId, estado: EstadoSaida) -> Resultado<u64> {
        let texto_org = org.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND state = ?2",
                params![texto_org, estado.como_texto()],
                |linha| linha.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(ErroStore::from)
        })
    }

    /// Itens que precisam de envio: `pending` e `sending` (o `sending` é o reenvio da retomada, D11).
    pub fn listar_pendentes(&self, org: OrgId, limite: u32) -> Resultado<Vec<ItemSaida>> {
        let texto_org = org.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        self.0.leitura(move |conn| {
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
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT external_message_id FROM outbox WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
                |linha| linha.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|valor| valor.flatten())
            .map_err(ErroStore::from)
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
    ) -> Resultado<Vec<ItemSaida>> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        self.0.leitura(move |conn| {
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
    pub fn obter(&self, org: OrgId, id: OutboxId) -> Resultado<ItemSaida> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS_SAIDA} FROM outbox WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha_saida,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "item de saída", &id.to_string())
    }
}

fn linha_entrada(linha: &Row<'_>) -> rusqlite::Result<ItemEntrada> {
    Ok(ItemEntrada {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        integration_id: id_da_coluna::<IntegrationId>(linha, 2)?,
        id_externo: linha.get(3)?,
        evento_json: linha.get(4)?,
        recebido_em: linha.get(5)?,
        processado_em: linha.get(6)?,
        tentativas: linha.get::<_, u32>(7)?,
        ultimo_erro: linha.get(8)?,
    })
}

pub(crate) fn linha_saida(linha: &Row<'_>) -> rusqlite::Result<ItemSaida> {
    Ok(ItemSaida {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        conversation_id: id_da_coluna::<ConversationId>(linha, 2)?,
        turn_id: id_da_coluna::<TurnId>(linha, 3)?,
        seq: linha.get::<_, u32>(4)?,
        tipo: linha.get(5)?,
        conteudo_json: linha.get(6)?,
        estado: EstadoSaida::de_texto(&linha.get::<_, String>(7)?).map_err(|erro| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(erro),
            )
        })?,
        tentativas: linha.get(8)?,
        ultimo_erro: linha.get(9)?,
        criado_em: linha.get(10)?,
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
    ) -> Resultado<Option<RecebidoComMensagem>> {
        let texto_integracao = integracao.to_string();
        let id_externo = id_externo.to_owned();
        self.0.leitura(move |conn| {
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
                .map_err(ErroStore::from)?;
            let Some(item) = item else {
                return Ok(None);
            };
            let Some(mensagem) = mensagem_da_entrega(conn, &texto_integracao, &id_externo)? else {
                return Ok(None);
            };
            Ok(Some(RecebidoComMensagem {
                situacao: Recebido::Duplicado,
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
        nova: NovaEntradaCanal,
    ) -> Resultado<RecebidoComMensagem> {
        let id_entrada = InboxId::generate();
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let NovaEntradaCanal {
            integration_id,
            agent_id,
            canal,
            id_externo,
            peer,
            evento_json,
            texto,
            anexos,
            nome_cliente,
        } = nova;
        let texto_integracao = integration_id.to_string();
        self.0.escrita(move |conn| {
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
                .map_err(ErroStore::from)?;

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
                    .map_err(ErroStore::from)?;
                let mensagem = mensagem_da_entrega(conn, &texto_integracao, &id_externo)?
                    .ok_or_else(|| {
                        ErroStore::Interno(
                            "entrada já gravada sem a mensagem correspondente".to_owned(),
                        )
                    })?;
                return Ok(RecebidoComMensagem {
                    situacao: Recebido::Duplicado,
                    item,
                    mensagem,
                });
            }

            // Conversa do contato: a aberta ou a transferida (D12: grava, não responde). Fechada
            // não conta; nesse caso abre-se outra.
            let existente: Option<String> = conn
                .query_row(
                    "SELECT id FROM conversations
                      WHERE org_id = ?1 AND integration_id = ?2 AND external_peer = ?3
                        AND status IN ('open', 'transferred')
                      ORDER BY updated_at DESC, id LIMIT 1",
                    params![&texto_org, &texto_integracao, &peer],
                    |linha| linha.get::<_, String>(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let conversa_id = match existente {
                Some(id) => id,
                None => {
                    let id = poph_types::ConversationId::generate().to_string();
                    conn.execute(
                        "INSERT INTO conversations
                            (id, org_id, agent_id, integration_id, channel, external_peer, status,
                             target_department_id, summary, version, created_at, updated_at,
                             last_message_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open',
                             (SELECT default_department_id FROM agents
                               WHERE org_id = ?2 AND id = ?3),
                             NULL, 1, ?7, ?7, NULL)",
                        params![
                            id,
                            &texto_org,
                            agent_id.to_string(),
                            &texto_integracao,
                            &canal,
                            &peer,
                            agora
                        ],
                    )
                    .map_err(ErroStore::from)?;
                    // A Ficha do atendimento nasce com a conversa, no mesmo savepoint (PRD L4,
                    // FX-07). Este é o caminho que o WhatsApp usa de fato: se a Ficha fosse criada
                    // só em `Conversas::criar`, o atendimento real ficaria sem registro.
                    super::fichas::criar_ficha_da_conversa(
                        conn,
                        &texto_org,
                        &id,
                        &agent_id.to_string(),
                        &canal,
                        Some(&peer),
                        agora,
                    )?;
                    id
                }
            };

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
                    Papel::Usuario.como_texto(),
                    &texto,
                    &id_externo,
                    agora
                ],
            )
            .map_err(ErroStore::from)?;

            // O turno lê da fila a conversa e a mensagem (HR-26h). Os dois ids só existem agora,
            // dentro da transação: o payload recebe `conversa` e `mensagem`, e o evento do canal
            // fica junto em `canal`, sem mudança.
            let payload_turno =
                payload_da_entrada_do_canal(&conversa_id, &mensagem_id, &evento_json)?;
            conn.execute(
                "UPDATE inbox SET payload_json = ?2 WHERE id = ?1",
                params![id_entrada.to_string(), payload_turno],
            )
            .map_err(ErroStore::from)?;

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
                        &anexo.nome,
                        &anexo.mime,
                        anexo.tamanho_bytes as i64,
                        &anexo.sha256,
                        &anexo.blob_path,
                        &anexo.texto_extraido,
                        anexo.status_extracao,
                        agora
                    ],
                )
                .map_err(ErroStore::from)?;
                ids_anexos.push(id_anexo);
            }
            let anexos_json = serde_json::to_string(&ids_anexos).map_err(|erro| {
                ErroStore::Interno(format!("lista de anexos não serializa: {erro}"))
            })?;
            conn.execute(
                "UPDATE messages SET attachments_json = ?2 WHERE id = ?1",
                params![&mensagem_id, anexos_json],
            )
            .map_err(ErroStore::from)?;
            conn.execute(
                "UPDATE conversations SET last_message_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![&conversa_id, agora],
            )
            .map_err(ErroStore::from)?;

            let mensagem = conn
                .query_row(
                    "SELECT id, org_id, conversation_id, direction, role, text, attachments_json,
                            external_id, turn_id, created_at, origin, author_name
                       FROM messages WHERE id = ?1",
                    params![&mensagem_id],
                    crate::repos::mensagens::linha,
                )
                .map_err(ErroStore::from)?;

            Ok(RecebidoComMensagem {
                situacao: Recebido::Novo,
                item: ItemEntrada {
                    id: id_entrada,
                    org_id: org,
                    integration_id,
                    id_externo,
                    evento_json,
                    recebido_em: agora,
                    processado_em: None,
                    tentativas: 0,
                    ultimo_erro: None,
                },
                mensagem,
            })
        })
    }
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
        let codigo = codigo_erro_nome(&ErroStore::Sqlite(erro));
        tracing::warn!(target: "poph_store", org, conversa, codigo, "não consegui abrir o savepoint do nome do cliente; a mensagem segue");
        return;
    }
    match gravar_nome_cliente(conn, org, conversa, nome, agora) {
        Ok(_) => {
            if let Err(erro) = conn.execute_batch("RELEASE nome_cliente") {
                let codigo = codigo_erro_nome(&ErroStore::Sqlite(erro));
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
fn codigo_erro_nome(erro: &ErroStore) -> &'static str {
    match erro {
        ErroStore::Dominio(_) => "dominio",
        ErroStore::Sqlite(_) => "banco",
        ErroStore::EscritorEncerrado => "escritor_encerrado",
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
    let canal: serde_json::Value = serde_json::from_str(evento_json)
        .map_err(|erro| ErroStore::Interno(format!("evento do canal não lê como JSON: {erro}")))?;
    serde_json::to_string(&serde_json::json!({
        "conversa": conversa,
        "mensagem": mensagem,
        "canal": canal,
    }))
    .map_err(|erro| ErroStore::Interno(format!("payload da entrada não serializa: {erro}")))
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
    .map_err(ErroStore::from)
}

impl Mensagens<'_> {
    /// Anexos de uma mensagem, na ordem em que chegaram.
    pub fn anexos(&self, org: OrgId, mensagem: MessageId) -> Resultado<Vec<AnexoRegistro>> {
        let texto_org = org.to_string();
        let texto_mensagem = mensagem.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT id, message_id, name, mime, size_bytes, sha256, blob_path,
                        extraction_status, extracted_text
                   FROM attachments
                  WHERE org_id = ?1 AND message_id = ?2
                  ORDER BY created_at, id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_mensagem], |linha| {
                Ok(AnexoRegistro {
                    id: id_da_coluna(linha, 0)?,
                    mensagem_id: id_da_coluna(linha, 1)?,
                    nome: linha.get(2)?,
                    mime: linha.get(3)?,
                    tamanho_bytes: linha.get::<_, i64>(4)?.max(0) as u64,
                    sha256: linha.get(5)?,
                    blob_path: linha.get(6)?,
                    status_extracao: linha.get(7)?,
                    texto_extraido: linha.get(8)?,
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
