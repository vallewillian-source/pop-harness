//! Repositório de mensagens.
//!
//! A mensagem recebida é idempotente pelo par `(conversation_id, external_id)` (índice único
//! parcial). Gravar a mesma mensagem de novo devolve a que já existe, em vez de duplicar — é o
//! que o webhook precisa ao ser reentregue.

use poph_types::{AttachmentId, AuditId, ConversationId, MessageId, OrgId, OutboxId, TurnId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{
    Direcao, GravacaoHumana, ItemSaida, Mensagem, NovaMensagem, NovaSaida, NovoAnexoCanal, Origem,
    Pagina, Papel, RespostaHumana, RespostaHumanaComArquivo, RespostaHumanaNova, StatusConversa,
};
use crate::repos::caixas::{COLUNAS_SAIDA, item_de_saida_novo, linha_saida};
use crate::repos::{id_da_coluna, id_opcional};

const COLUNAS: &str = "id, org_id, conversation_id, direction, role, text, attachments_json,
    external_id, turn_id, created_at, origin, author_name";

/// Acesso às mensagens.
pub struct Mensagens<'a>(pub(crate) &'a Store);

/// O que toda resposta do atendente tem em comum, venha ela com texto (MX-04) ou com arquivo
/// (MX-44).
struct EventoHumano<'a> {
    /// Id do evento Matrix: chave de idempotência da mensagem.
    evento_id: &'a str,
    /// Corpo que o atendente escreveu (vazio quando ele só mandou o arquivo).
    texto_bruto: &'a str,
    /// Id Matrix do atendente, para a auditoria.
    autor_id: &'a str,
    /// Nome do atendente, para `author_name`.
    autor_nome: &'a str,
}

impl Mensagens<'_> {
    /// Grava a mensagem. Devolve a mensagem gravada (ou a equivalente, quando é duplicata).
    ///
    /// Não toca no marcador de atividade da conversa de propósito: reescrever a mesma linha a
    /// cada mensagem levava a vazão de 83 mil para 54 mil msg/s (medido na HR-07). Quem move o
    /// marcador é o turno, uma vez por lote, com [`Conversas::tocar`](crate::Conversas::tocar).
    pub fn gravar(&self, org: OrgId, nova: NovaMensagem) -> Resultado<Mensagem> {
        let agora = self.0.agora();
        let id = MessageId::generate();
        let texto_org = org.to_string();
        self.0.escrita(move |conn| {
            let NovaMensagem {
                conversation_id,
                direcao,
                papel,
                origem,
                autor_nome,
                texto,
                anexos_json,
                id_externo,
                turn_id,
            } = nova;
            let conversa = conversation_id.to_string();
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO messages
                        (id, org_id, conversation_id, direction, role, text, attachments_json,
                         external_id, turn_id, created_at, origin, author_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        id.to_string(),
                        &texto_org,
                        &conversa,
                        direcao.como_texto(),
                        papel.como_texto(),
                        &texto,
                        &anexos_json,
                        id_externo.as_deref(),
                        turn_id.map(|turno| turno.to_string()),
                        agora,
                        origem.como_texto(),
                        autor_nome.as_deref()
                    ],
                )
                .map_err(ErroStore::from)?;

            if inseriu == 0 {
                let existente = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUNAS} FROM messages
                              WHERE conversation_id = ?1 AND external_id = ?2"
                        ),
                        params![&conversa, id_externo.as_deref()],
                        linha,
                    )
                    .optional()
                    .map_err(ErroStore::from)?;
                return existente.ok_or_else(|| {
                    ErroStore::Interno(
                        "a inserção da mensagem foi ignorada, mas a original não foi encontrada"
                            .to_owned(),
                    )
                });
            }

            Ok(Mensagem {
                id,
                org_id: org,
                conversation_id,
                direcao,
                papel,
                origem,
                autor_nome,
                texto,
                anexos_json,
                id_externo,
                turn_id,
                criado_em: agora,
            })
        })
    }

    /// Grava a mensagem do cliente **com os anexos** numa única transação (HR-22, chat de teste).
    ///
    /// Mesma idempotência de [`Mensagens::gravar`] (par `conversation_id`/`external_id`): o
    /// `external_id` é `None` no chat de teste, então cada chamada é uma linha nova. Cada anexo
    /// vira uma linha em `attachments` e a lista dos ids vai para `messages.attachments_json`, no
    /// mesmo formato que o turno lê (`Mensagens::anexos`).
    pub fn gravar_com_anexos(
        &self,
        org: OrgId,
        nova: NovaMensagem,
        anexos: &[NovoAnexoCanal],
    ) -> Resultado<Mensagem> {
        let agora = self.0.agora();
        let id = MessageId::generate();
        let texto_org = org.to_string();
        let anexos = anexos.to_vec();
        self.0.escrita(move |conn| {
            let NovaMensagem {
                conversation_id,
                direcao,
                papel,
                origem,
                autor_nome,
                texto,
                anexos_json,
                id_externo,
                turn_id,
            } = nova;
            let conversa = conversation_id.to_string();
            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO messages
                        (id, org_id, conversation_id, direction, role, text, attachments_json,
                         external_id, turn_id, created_at, origin, author_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        id.to_string(),
                        &texto_org,
                        &conversa,
                        direcao.como_texto(),
                        papel.como_texto(),
                        &texto,
                        &anexos_json,
                        id_externo.as_deref(),
                        turn_id.map(|turno| turno.to_string()),
                        agora,
                        origem.como_texto(),
                        autor_nome.as_deref()
                    ],
                )
                .map_err(ErroStore::from)?;
            if inseriu == 0 {
                let existente = conn
                    .query_row(
                        &format!(
                            "SELECT {COLUNAS} FROM messages
                              WHERE conversation_id = ?1 AND external_id = ?2"
                        ),
                        params![&conversa, id_externo.as_deref()],
                        linha,
                    )
                    .optional()
                    .map_err(ErroStore::from)?;
                return existente.ok_or_else(|| {
                    ErroStore::Interno(
                        "a inserção da mensagem foi ignorada, mas a original não foi encontrada"
                            .to_owned(),
                    )
                });
            }

            let mut ids: Vec<String> = Vec::with_capacity(anexos.len());
            for anexo in &anexos {
                let id_anexo = AttachmentId::generate().to_string();
                conn.execute(
                    "INSERT INTO attachments
                        (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path,
                         extracted_text, extraction_status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        &id_anexo,
                        &texto_org,
                        id.to_string(),
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
                ids.push(id_anexo);
            }
            let anexos_json = if ids.is_empty() {
                "[]".to_owned()
            } else {
                serde_json::to_string(&ids).map_err(|erro| {
                    ErroStore::Interno(format!("lista de anexos não serializa: {erro}"))
                })?
            };
            conn.execute(
                "UPDATE messages SET attachments_json = ?2 WHERE id = ?1",
                params![id.to_string(), &anexos_json],
            )
            .map_err(ErroStore::from)?;
            conn.execute(
                "UPDATE conversations SET last_message_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![&conversa, agora],
            )
            .map_err(ErroStore::from)?;

            Ok(Mensagem {
                id,
                org_id: org,
                conversation_id,
                direcao,
                papel,
                origem,
                autor_nome,
                texto,
                anexos_json,
                id_externo,
                turn_id,
                criado_em: agora,
            })
        })
    }

    /// Grava a resposta do agente **e** o item da fila de saída numa transação só (HR-57).
    ///
    /// Antes eram duas escritas separadas (mensagem, depois `outbox`). Queda do processo entre as
    /// duas deixava a resposta no histórico sem nunca ser enviada ao cliente, e o turno parecia
    /// concluído. Agora ou as duas linhas entram, ou nenhuma: a retomada reabre o turno e o cliente
    /// recebe a resposta uma única vez.
    ///
    /// A idempotência de cada parte é a mesma de [`Mensagens::gravar`] e de
    /// [`CaixaSaida::enfileirar`](crate::CaixaSaida::enfileirar): a mensagem pela chave
    /// `(conversation_id, external_id)` (que aqui é `None`, então sempre nova) e o item por
    /// `(turn_id, seq)`.
    pub fn gravar_com_saida(
        &self,
        org: OrgId,
        nova: NovaMensagem,
        saida: NovaSaida,
    ) -> Resultado<(Mensagem, ItemSaida)> {
        let agora = self.0.agora();
        let id = MessageId::generate();
        let id_saida = OutboxId::generate();
        let texto_org = org.to_string();
        self.0.escrita(move |conn| {
            let NovaMensagem {
                conversation_id,
                direcao,
                papel,
                origem,
                autor_nome,
                texto,
                anexos_json,
                id_externo,
                turn_id,
            } = nova;
            let NovaSaida {
                conversation_id: conversa_da_saida,
                turn_id: turno_da_saida,
                seq,
                tipo,
                conteudo_json,
            } = saida;
            if conversa_da_saida != conversation_id || turn_id != Some(turno_da_saida) {
                return Err(ErroStore::Interno(
                    "a resposta e o item de saída precisam ser da mesma conversa e do mesmo turno"
                        .to_owned(),
                ));
            }
            let conversa = conversation_id.to_string();
            let texto_turno = turno_da_saida.to_string();
            conn.execute(
                "INSERT OR IGNORE INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id.to_string(),
                    &texto_org,
                    &conversa,
                    direcao.como_texto(),
                    papel.como_texto(),
                    &texto,
                    &anexos_json,
                    id_externo.as_deref(),
                    turn_id.map(|turno| turno.to_string()),
                    agora,
                    origem.como_texto(),
                    autor_nome.as_deref()
                ],
            )
            .map_err(ErroStore::from)?;

            let inseriu = conn
                .execute(
                    "INSERT OR IGNORE INTO outbox
                        (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                         attempts, last_error, created_at, sent_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, NULL, ?8, NULL)",
                    params![
                        id_saida.to_string(),
                        &texto_org,
                        &conversa,
                        &texto_turno,
                        i64::from(seq),
                        &tipo,
                        &conteudo_json,
                        agora
                    ],
                )
                .map_err(ErroStore::from)?;

            let item = if inseriu == 0 {
                conn.query_row(
                    &format!("SELECT {COLUNAS_SAIDA} FROM outbox WHERE turn_id = ?1 AND seq = ?2"),
                    params![&texto_turno, i64::from(seq)],
                    linha_saida,
                )
                .optional()
                .map_err(ErroStore::from)?
                .ok_or_else(|| {
                    ErroStore::Interno(
                        "inserção da saída ignorada, mas a original não foi encontrada".to_owned(),
                    )
                })?
            } else {
                item_de_saida_novo(
                    id_saida,
                    org,
                    NovaSaida {
                        conversation_id,
                        turn_id: turno_da_saida,
                        seq,
                        tipo,
                        conteudo_json,
                    },
                    agora,
                )
            };

            Ok((
                Mensagem {
                    id,
                    org_id: org,
                    conversation_id,
                    direcao,
                    papel,
                    origem,
                    autor_nome,
                    texto,
                    anexos_json,
                    id_externo,
                    turn_id,
                    criado_em: agora,
                },
                item,
            ))
        })
    }

    /// Registra a resposta do atendente humano pelo Element (MX-04, PRD do módulo Matrix §7 N2)
    /// numa única escrita.
    ///
    /// Grava na mesma transação: a mensagem (`direction out`, `role assistant`, `origin human`,
    /// `external_id = evento_id`, com o nome do autor); o item da fila de saída (`text`, seq 0, com
    /// o texto já formatado); a passagem de `open` para `transferred` (a versão sobe uma vez só);
    /// e a auditoria `human_takeover`. Se qualquer parte falhar, nada fica gravado: a escrita roda
    /// num savepoint do escritor, que volta inteiro.
    ///
    /// `texto_bruto` é o corpo que o atendente escreveu (vai para `messages.text`); `texto_enviado`
    /// é o que vai ao cliente, já formatado por quem chama (MX-18). `autor_id` é o id Matrix do
    /// atendente (vai para a auditoria, `actor_id`); `autor_nome` vai para `author_name`.
    ///
    /// Ordem das decisões: a conversa tem que existir na org; um `evento_id` já gravado nela devolve
    /// `Duplicada` antes de olhar a situação (reentrega não vira aviso de "conversa fechada"); só
    /// então uma conversa `closed` recusa a gravação. Conversa já `transferred` continua assim e
    /// não sobe a versão.
    ///
    /// A auditoria é gravada a cada resposta gravada, com `transferiu` dizendo se esta resposta
    /// fez a passagem. A mensagem e o item de saída levam o mesmo `turn_id`, que é a junção usada
    /// pelo aviso de falha de entrega (MX-19).
    ///
    /// A resposta **com arquivo** (MX-44) passa pelo mesmo caminho, por
    /// [`Mensagens::registrar_resposta_humana_com_arquivo`]: o que muda é o tipo do item de saída e
    /// a linha em `attachments`.
    pub fn registrar_resposta_humana(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nova: RespostaHumanaNova<'_>,
    ) -> Resultado<RespostaHumana> {
        let payload = serde_json::json!({ "texto": nova.texto_enviado }).to_string();
        self.gravar_resposta_humana(
            org,
            conversa,
            EventoHumano {
                evento_id: nova.evento_id,
                texto_bruto: nova.texto_bruto,
                autor_id: nova.autor_id,
                autor_nome: nova.autor_nome,
            },
            "text",
            payload,
            None,
        )
    }

    /// Registra a resposta do atendente humano **com arquivo** pelo Element (MX-44) numa única
    /// escrita.
    ///
    /// É a mesma operação de [`Mensagens::registrar_resposta_humana`] (mesma idempotência pelo id do
    /// evento, mesma passagem para `transferred`, mesma auditoria `human_takeover`) com duas
    /// diferenças: `messages.text` fica vazio (o atendente não digitou nada) e a mensagem leva o id
    /// do anexo em `attachments_json`; além do item de saída `text`, entra um item `file`, com o
    /// payload do HR-24 (`blob_ref`, `arquivo_id`, `nome`, `mime`, `tamanho_bytes`, `sha256`) mais a
    /// `legenda` que acompanha o arquivo no canal.
    ///
    /// O blob **já está** no disco da org quando esta função roda: quem baixou e gravou foi o
    /// módulo Matrix. Se o arquivo já existe com o mesmo hash, nenhum byte é regravado.
    pub fn registrar_resposta_humana_com_arquivo(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nova: RespostaHumanaComArquivo<'_>,
    ) -> Resultado<RespostaHumana> {
        let arquivo = nova.arquivo;
        let id_anexo = AttachmentId::generate();
        let payload = serde_json::json!({
            "blob_ref": arquivo.blob_path,
            "arquivo_id": id_anexo.to_string(),
            "nome": arquivo.nome,
            "mime": arquivo.mime,
            "tamanho_bytes": arquivo.tamanho_bytes,
            "sha256": arquivo.sha256,
            "legenda": arquivo.legenda,
        })
        .to_string();
        let anexo = NovoAnexoCanal {
            nome: arquivo.nome.to_owned(),
            mime: arquivo.mime.to_owned(),
            tamanho_bytes: arquivo.tamanho_bytes,
            sha256: arquivo.sha256.to_owned(),
            blob_path: arquivo.blob_path.to_owned(),
            texto_extraido: None,
            status_extracao: arquivo.status_extracao,
        };
        self.gravar_resposta_humana(
            org,
            conversa,
            EventoHumano {
                evento_id: nova.evento_id,
                texto_bruto: "",
                autor_id: nova.autor_id,
                autor_nome: nova.autor_nome,
            },
            "file",
            payload,
            Some((id_anexo, anexo)),
        )
    }

    /// O corpo comum das respostas humanas (MX-04 e MX-44): as duas diferem só no tipo do item de
    /// saída e no anexo.
    fn gravar_resposta_humana(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nova: EventoHumano<'_>,
        tipo_saida: &str,
        conteudo_json: String,
        anexo: Option<(AttachmentId, NovoAnexoCanal)>,
    ) -> Resultado<RespostaHumana> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let evento = nova.evento_id.to_owned();
        let bruto = nova.texto_bruto.to_owned();
        let ator = nova.autor_id.to_owned();
        let nome = nova.autor_nome.to_owned();
        let tipo_saida = tipo_saida.to_owned();
        let anexos_json = match &anexo {
            Some((id, _)) => serde_json::json!([id.to_string()]).to_string(),
            None => "[]".to_owned(),
        };
        let id = MessageId::generate();
        let id_saida = OutboxId::generate();
        let turno = TurnId::generate();
        self.0.escrita(move |conn| {
            let situacao: Option<String> = conn
                .query_row(
                    "SELECT status FROM conversations WHERE org_id = ?1 AND id = ?2",
                    params![&texto_org, &texto_conversa],
                    |linha| linha.get::<_, String>(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some(situacao) = situacao else {
                return Ok(RespostaHumana::ConversaInexistente);
            };
            let ja_gravada = conn
                .query_row(
                    "SELECT 1 FROM messages
                      WHERE org_id = ?1 AND conversation_id = ?2 AND external_id = ?3",
                    params![&texto_org, &texto_conversa, &evento],
                    |linha| linha.get::<_, i64>(0),
                )
                .optional()
                .map_err(ErroStore::from)?
                .is_some();
            if ja_gravada {
                return Ok(RespostaHumana::Duplicada);
            }
            let situacao = StatusConversa::de_texto(&situacao)?;
            if situacao == StatusConversa::Fechada {
                return Ok(RespostaHumana::ConversaFechada);
            }
            let transferiu_agora = situacao == StatusConversa::Aberta;

            conn.execute(
                "INSERT INTO messages
                    (id, org_id, conversation_id, direction, role, text, attachments_json,
                     external_id, turn_id, created_at, origin, author_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id.to_string(),
                    &texto_org,
                    &texto_conversa,
                    Direcao::Saida.como_texto(),
                    Papel::Assistente.como_texto(),
                    &bruto,
                    &anexos_json,
                    &evento,
                    turno.to_string(),
                    agora,
                    Origem::Humano.como_texto(),
                    &nome
                ],
            )
            .map_err(ErroStore::from)?;

            // A linha do anexo é o que o resto do Harness lê para saber o que acompanha a mensagem
            // (e o que a retenção precisa apagar junto com ela).
            if let Some((id_anexo, anexo)) = &anexo {
                conn.execute(
                    "INSERT INTO attachments
                        (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path,
                         extracted_text, extraction_status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        id_anexo.to_string(),
                        &texto_org,
                        id.to_string(),
                        &anexo.nome,
                        &anexo.mime,
                        anexo.tamanho_bytes as i64,
                        &anexo.sha256,
                        &anexo.blob_path,
                        anexo.texto_extraido.as_deref(),
                        anexo.status_extracao,
                        agora
                    ],
                )
                .map_err(ErroStore::from)?;
            }

            let nova_saida = NovaSaida {
                conversation_id: conversa,
                turn_id: turno,
                seq: 0,
                tipo: tipo_saida.clone(),
                conteudo_json: conteudo_json.clone(),
            };
            conn.execute(
                "INSERT INTO outbox
                    (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state,
                     attempts, last_error, created_at, sent_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, 'pending', 0, NULL, ?7, NULL)",
                params![
                    id_saida.to_string(),
                    &texto_org,
                    &texto_conversa,
                    turno.to_string(),
                    &tipo_saida,
                    &conteudo_json,
                    agora
                ],
            )
            .map_err(ErroStore::from)?;
            let saida = item_de_saida_novo(id_saida, org, nova_saida, agora);

            if transferiu_agora {
                conn.execute(
                    "UPDATE conversations
                        SET status = 'transferred', version = version + 1,
                            last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2 AND status = 'open'",
                    params![&texto_org, &texto_conversa, agora],
                )
                .map_err(ErroStore::from)?;
            } else {
                conn.execute(
                    "UPDATE conversations SET last_message_at = ?3, updated_at = ?3
                      WHERE org_id = ?1 AND id = ?2",
                    params![&texto_org, &texto_conversa, agora],
                )
                .map_err(ErroStore::from)?;
            }

            let detalhe = serde_json::json!({
                "transferiu": transferiu_agora,
                "mensagem_id": id.to_string(),
            });
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, 'matrix_user', ?3, 'human_takeover', 'conversation', ?4, ?5, ?6)",
                params![
                    AuditId::generate().to_string(),
                    &texto_org,
                    &ator,
                    &texto_conversa,
                    detalhe.to_string(),
                    agora
                ],
            )
            .map_err(ErroStore::from)?;

            Ok(RespostaHumana::Gravada(Box::new(GravacaoHumana {
                mensagem: Mensagem {
                    id,
                    org_id: org,
                    conversation_id: conversa,
                    direcao: Direcao::Saida,
                    papel: Papel::Assistente,
                    origem: Origem::Humano,
                    autor_nome: Some(nome),
                    texto: bruto,
                    anexos_json,
                    id_externo: Some(evento),
                    turn_id: Some(turno),
                    criado_em: agora,
                },
                saida,
                transferiu_agora,
            })))
        })
    }

    /// Lista a conversa em ordem de chegada (cursor pelo id, que é ordenado por tempo).
    pub fn listar(
        &self,
        org: OrgId,
        conversa: ConversationId,
        pagina: Pagina,
    ) -> Resultado<Vec<Mensagem>> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM messages
                  WHERE org_id = ?1 AND conversation_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, texto_conversa, apos, limite], linha)?;
            let mut mensagens = Vec::new();
            for linha in linhas {
                mensagens.push(linha?);
            }
            Ok(mensagens)
        })
    }

    /// Últimas `limite` mensagens da conversa, em ordem de chegada (para a janela de contexto).
    pub fn ultimas(
        &self,
        org: OrgId,
        conversa: ConversationId,
        limite: u32,
    ) -> Resultado<Vec<Mensagem>> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let limite = i64::from(limite.clamp(1, 1000));
        let mut mensagens = self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM messages
                  WHERE org_id = ?1 AND conversation_id = ?2
                  ORDER BY id DESC LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, texto_conversa, limite], linha)?;
            let mut mensagens = Vec::new();
            for linha in linhas {
                mensagens.push(linha?);
            }
            Ok(mensagens)
        })?;
        mensagens.reverse();
        Ok(mensagens)
    }

    /// Busca pelo id dentro da org (e da conversa).
    pub fn obter(
        &self,
        org: OrgId,
        conversa: ConversationId,
        id: MessageId,
    ) -> Resultado<Mensagem> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let texto_id = id.to_string();
        let encontrada = self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS} FROM messages
                      WHERE org_id = ?1 AND conversation_id = ?2 AND id = ?3"
                ),
                params![texto_org, texto_conversa, texto_id],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrada, "mensagem", &id.to_string())
    }

    /// A conversa tem mensagem gravada antes de `corte` (ms)? Usado pelo aviso de histórico não
    /// espelhado (MX-11, P8).
    pub fn teve_mensagens_antes(
        &self,
        org: OrgId,
        conversa: ConversationId,
        corte: i64,
    ) -> Resultado<bool> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM messages
                                 WHERE org_id = ?1 AND conversation_id = ?2 AND created_at < ?3)",
                params![texto_org, texto_conversa, corte],
                |linha| linha.get::<_, bool>(0),
            )
            .map_err(ErroStore::from)
        })
    }

    /// Última mensagem da tabela (maior `rowid`) com o id dela, ou `None` quando não há nenhuma.
    /// O espelho usa para achar o ponto seguro quando o cursor ficou à frente (R5).
    pub fn ultima_rowid(&self) -> Resultado<Option<(u64, MessageId)>> {
        self.0.leitura(|conn| {
            let linha: Option<(i64, String)> = conn
                .query_row(
                    "SELECT rowid, id FROM messages ORDER BY rowid DESC LIMIT 1",
                    [],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            match linha {
                None => Ok(None),
                Some((rowid, id)) => {
                    let id: MessageId = id.parse().map_err(|_| {
                        ErroStore::Dominio(poph_types::DomainError::Validacao {
                            campo: "messages.id",
                            motivo: "id de mensagem inválido no banco".to_owned(),
                        })
                    })?;
                    Ok(Some((u64::try_from(rowid).unwrap_or(0), id)))
                }
            }
        })
    }

    /// Quantas mensagens a conversa tem.
    pub fn contar(&self, org: OrgId, conversa: ConversationId) -> Resultado<u64> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT count(*) FROM messages WHERE org_id = ?1 AND conversation_id = ?2",
                params![texto_org, texto_conversa],
                |linha| linha.get::<_, i64>(0),
            )
            .map(|total| total.max(0) as u64)
            .map_err(ErroStore::from)
        })
    }

    /// Leitura incremental do espelhamento (PRD §7 N3): mensagens com `rowid` maior que `apos`,
    /// em ordem crescente de `rowid`, no máximo `limite` (teto [`LIMITE_DESDE`]).
    ///
    /// Só entram `user` e `assistant`: `tool` e `system` nunca vão para a sala. Roda no leitor, não
    /// no escritor, então não disputa com o turno. Para paginar até o fim, o chamador avança o
    /// cursor até o `rowid` da última linha devolvida. Linhas gravadas depois de uma página
    /// aparecem nas páginas seguintes, porque o `rowid` só cresce.
    pub fn desde(&self, apos: u64, limite: u32) -> Resultado<Vec<MensagemDesde>> {
        let apos = i64::try_from(apos).unwrap_or(i64::MAX);
        let limite = i64::from(limite.clamp(1, LIMITE_DESDE));
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT rowid, id, org_id, conversation_id, direction, role, origin, author_name,
                        text, attachments_json, external_id, created_at
                   FROM messages
                  WHERE rowid > ?1 AND role IN ('user', 'assistant')
                  ORDER BY rowid
                  LIMIT ?2",
            )?;
            let linhas = consulta.query_map(params![apos, limite], linha_desde)?;
            let mut mensagens = Vec::new();
            for linha in linhas {
                mensagens.push(linha?);
            }
            Ok(mensagens)
        })
    }
}

/// Teto de linhas por página da leitura incremental (MX-05).
pub const LIMITE_DESDE: u32 = 1000;

/// Mensagem vista pela leitura incremental (PRD §7 N3).
///
/// Traz o `rowid`, que é o cursor de `matrix_estado.messages_cursor`, e o `id` da mensagem, de
/// onde o espelhamento deriva o `txnId` (MX-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MensagemDesde {
    /// Posição na tabela: o cursor do espelhamento avança até o `rowid` da última linha lida.
    pub rowid: u64,
    /// Id da mensagem.
    pub id: MessageId,
    /// Org dona.
    pub org_id: OrgId,
    /// Conversa dona.
    pub conversation_id: ConversationId,
    /// Direção (`Entrada` ou `Saida`).
    pub direcao: Direcao,
    /// Papel (sempre `Usuario` ou `Assistente` nesta leitura).
    pub papel: Papel,
    /// Quem produziu.
    pub origem: Origem,
    /// Nome do atendente, quando a origem é humana.
    pub autor_nome: Option<String>,
    /// Texto.
    pub texto: String,
    /// Anexos em JSON (ids de `attachments`).
    pub anexos_json: String,
    /// Id da mensagem no canal externo.
    pub id_externo: Option<String>,
    /// Criação (ms).
    pub criado_em: i64,
}

/// Lê uma linha no formato de [`MensagemDesde`]. A ordem das colunas segue a consulta de
/// [`Mensagens::desde`].
fn linha_desde(linha: &Row<'_>) -> rusqlite::Result<MensagemDesde> {
    let direcao = match linha.get::<_, String>(4)?.as_str() {
        "in" => Direcao::Entrada,
        "out" => Direcao::Saida,
        outro => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                format!("direção desconhecida: `{outro}`").into(),
            ));
        }
    };
    let papel = Papel::de_texto(&linha.get::<_, String>(5)?).map_err(|erro| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(erro))
    })?;
    let origem = Origem::de_texto(&linha.get::<_, String>(6)?).map_err(|erro| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(erro))
    })?;
    Ok(MensagemDesde {
        rowid: u64::try_from(linha.get::<_, i64>(0)?).unwrap_or(0),
        id: id_da_coluna(linha, 1)?,
        org_id: id_da_coluna(linha, 2)?,
        conversation_id: id_da_coluna(linha, 3)?,
        direcao,
        papel,
        origem,
        autor_nome: linha.get(7)?,
        texto: linha.get(8)?,
        anexos_json: linha.get(9)?,
        id_externo: linha.get(10)?,
        criado_em: linha.get(11)?,
    })
}

/// Lê uma linha no formato de [`COLUNAS`] (e das consultas de `caixas.rs` com as mesmas colunas).
pub(crate) fn linha(linha: &Row<'_>) -> rusqlite::Result<Mensagem> {
    let direcao = match linha.get::<_, String>(3)?.as_str() {
        "in" => Direcao::Entrada,
        "out" => Direcao::Saida,
        outro => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                format!("direção desconhecida: `{outro}`").into(),
            ));
        }
    };
    let papel = Papel::de_texto(&linha.get::<_, String>(4)?).map_err(|erro| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(erro))
    })?;
    let origem = Origem::de_texto(&linha.get::<_, String>(10)?).map_err(|erro| {
        rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(erro))
    })?;
    Ok(Mensagem {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        conversation_id: id_da_coluna(linha, 2)?,
        direcao,
        papel,
        origem,
        autor_nome: linha.get(11)?,
        texto: linha.get(5)?,
        anexos_json: linha.get(6)?,
        id_externo: linha.get(7)?,
        turn_id: id_opcional::<TurnId>(linha, 8)?,
        criado_em: linha.get(9)?,
    })
}
