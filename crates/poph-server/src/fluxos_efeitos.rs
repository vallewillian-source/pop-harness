//! Fila de efeitos do fluxo (FX-21): entrega o que o motor pediu e a transição já gravou.
//!
//! Roda como laço dentro do `poph serve` (ver [`iniciar`]), uma vez por segundo, e só com o motor
//! de fluxos ligado (`[fluxos] ativo`). Para cada efeito na fila (`fluxo_efeitos`, FX-20):
//!
//! | Efeito | O que faz |
//! |---|---|
//! | `mensagem_ao_cliente(texto)` | enfileira na `outbox` **da conversa**, com `turn_id` novo (precedente: resposta humana, MX-04). Espera o turno da conversa terminar: a mensagem fixa não pode chegar antes da resposta do agente. |
//! | `notificar_whatsapp(para, mensagem)` | manda **a outro número** pela integração WhatsApp do agente da Ficha (a mesma instância que atende a conversa). Não usa a `outbox`: a `outbox` é por conversa e o destino aqui é a equipe. |
//! | `encaminhar(departamento)` | transfere a conversa para o departamento pelo nome, com a operação do turno (`conversas().transferir`, auditoria `system`). A **Ficha continua aberta** e o agente deixa de responder (D12, a guarda do turno). |
//!
//! # Estados e tentativas
//!
//! `pendente → enviando → enviado | falhou`, com a tentativa gravada **antes** da chamada externa
//! (mesma regra da saída do WhatsApp, D11): queda no meio reenvia, e uma duplicata rara é aceita e
//! documentada (PRD, riscos).
//!
//! - **Falha transitória** (rede, 429, 5xx) volta para `pendente` com espera crescente
//!   ([`Backoff`]) — nunca repete no mesmo ciclo.
//! - **Falha definitiva** (4xx, payload inválido, departamento que não existe, conversa encerrada,
//!   Ficha sem conversa) vira `falhou` com o motivo. Depois de
//!   [`OpcoesEfeitosFluxo::max_tentativas`] tentativas, também.
//! - **Espera que não é falha** não consome tentativa: sem integração/chave da Evolution, o efeito
//!   fica `pendente` até haver; com turno em andamento na conversa, a mensagem espera.
//!
//! Falha de efeito **não** desfaz a transição do fluxo (PRD L11): ela vira evento na Ficha
//! (`efeito_falhou`) e o atendimento segue.
//!
//! # Formato do `payload_json`
//!
//! É o formato que o adaptador do turno (FX-22/FX-23) grava a partir de
//! [`poph_fluxos::EfeitoProduzido`] — e quem grava usa [`efeito_da_transicao`], aqui do lado, para
//! não existirem dois formatos:
//!
//! ```json
//! {"texto": "..."}                       // mensagem_ao_cliente
//! {"para": "5511999990000", "mensagem": "..."}  // notificar_whatsapp
//! {"departamento": "Comercial"}          // encaminhar
//! ```
//!
//! # O que não sai daqui
//!
//! Telefone (`peer`, `para`) não vai para log, métrica nem mensagem de erro: o log leva ids
//! internos, o tipo do efeito e o motivo já limpo pelo cliente da Evolution.
//!
//! [`Backoff`]: poph_agent::Backoff

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::Backoff;
use poph_evolution::{BlobsEmDisco, FonteDeMidia, WhatsAppEvolution};
use poph_fluxos::EfeitoProduzido;
use poph_store::{EfeitoDaTransicao, EfeitoDoFluxo, ErroStore, NovaSaida, Store};
use poph_types::{AtorEvento, IntegrationId, OrgId, TipoEventoFicha, TurnId};
use serde_json::Value;

use crate::estado::Estado;
use crate::evolution::CanalEvolution;
use crate::fluxos_metricas::MetricasFluxos;

/// Ajustes da entrega. Os padrões seguem a fila de saída do WhatsApp.
#[derive(Debug, Clone)]
pub struct OpcoesEfeitosFluxo {
    /// Efeitos lidos por org e por ciclo.
    pub lote: u32,
    /// Tentativas antes de desistir (vira `falhou`).
    pub max_tentativas: u32,
    /// Espera entre tentativas transitórias.
    pub backoff: Backoff,
    /// Intervalo entre ciclos do laço.
    pub intervalo: Duration,
    /// Espera de quem não está esperando falha: sem integração, ou turno em andamento.
    pub espera_sem_integracao: Duration,
}

impl Default for OpcoesEfeitosFluxo {
    fn default() -> Self {
        Self {
            lote: 100,
            max_tentativas: 8,
            backoff: Backoff::default(),
            intervalo: Duration::from_secs(1),
            espera_sem_integracao: Duration::from_secs(30),
        }
    }
}

/// O que um ciclo fez. Só contagens: nada de texto de cliente nem de telefone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoEfeitos {
    /// Efeitos entregues neste ciclo.
    pub enviados: u32,
    /// Efeitos que continuam esperando (falta integração, turno em andamento).
    pub adiados: u32,
    /// Falhas transitórias (voltam para a fila).
    pub transitorios: u32,
    /// Falhas definitivas (`falhou`).
    pub definitivos: u32,
    /// Efeitos que esgotaram as tentativas (`falhou`).
    pub esgotados: u32,
}

/// Um canal de WhatsApp configurado, para o `notificar_whatsapp`.
struct CanalNotificacao {
    org: OrgId,
    integracao: IntegrationId,
    /// `None` quando a chave da Evolution não está no ambiente.
    canal: Option<Arc<WhatsAppEvolution>>,
}

/// O entregador dos efeitos do fluxo.
pub struct EfeitosFluxo {
    store: Arc<Store>,
    canais: Vec<CanalNotificacao>,
    metricas: Option<Arc<MetricasFluxos>>,
    opcoes: OpcoesEfeitosFluxo,
}

/// O que fazer com um efeito, decidido antes de mexer em qualquer estado.
enum Plano {
    /// Pode tentar agora.
    Entregar(Entrega),
    /// Espera que não é falha: não conta tentativa.
    Esperar(&'static str),
    /// Falha definitiva sem tentar.
    Falhar(String),
}

/// A entrega concreta de um efeito.
enum Entrega {
    /// Texto fixo ao cliente, pela `outbox` da conversa.
    TextoAoCliente {
        conversa: poph_types::ConversationId,
        texto: String,
    },
    /// Mensagem a outro número, pela integração WhatsApp do agente.
    Whatsapp {
        canal: Arc<WhatsAppEvolution>,
        para: String,
        mensagem: String,
    },
    /// Transferência da conversa para um departamento pelo nome.
    Encaminhar {
        conversa: poph_types::ConversationId,
        departamento: poph_types::DepartmentId,
        nome: String,
    },
}

impl EfeitosFluxo {
    /// Entregador para as instâncias dadas. `pasta_blobs` é onde a biblioteca lê os arquivos (o
    /// `notificar_whatsapp` só manda texto, mas o canal precisa da fonte).
    pub fn novo(
        store: Arc<Store>,
        canais: &[CanalEvolution],
        pasta_blobs: PathBuf,
        metricas: Option<Arc<MetricasFluxos>>,
        opcoes: OpcoesEfeitosFluxo,
    ) -> Self {
        let fonte: Arc<dyn FonteDeMidia> = Arc::new(BlobsEmDisco::nova(pasta_blobs));
        let canais = canais
            .iter()
            .map(|instancia| CanalNotificacao {
                org: instancia.org,
                integracao: instancia.integracao,
                canal: instancia.cliente.as_ref().map(|cliente| {
                    Arc::new(WhatsAppEvolution::para_saida(
                        cliente.clone(),
                        instancia.integracao,
                        Arc::clone(&fonte),
                    ))
                }),
            })
            .collect();
        Self {
            store,
            canais,
            metricas,
            opcoes,
        }
    }

    fn canal_de(&self, org: OrgId, integracao: IntegrationId) -> Option<&CanalNotificacao> {
        self.canais
            .iter()
            .find(|canal| canal.org == org && canal.integracao == integracao)
    }

    fn contar(&self, contador: &dyn Fn(&MetricasFluxos) -> &poph_types::Contador) {
        if let Some(metricas) = self.metricas.as_ref() {
            contador(metricas).incrementar(1);
        }
    }

    /// Um ciclo: percorre as orgs com efeito na fila e tenta entregar o que venceu a espera.
    pub async fn ciclo(&self) -> Result<Vec<RelatoEfeitos>, ErroStore> {
        let orgs = self.store.efeitos().orgs_com_pendentes()?;
        let mut relatos = Vec::with_capacity(orgs.len());
        for org in orgs {
            relatos.push(self.ciclo_da_org(org).await?);
        }
        Ok(relatos)
    }

    /// Um ciclo numa org.
    pub async fn ciclo_da_org(&self, org: OrgId) -> Result<RelatoEfeitos, ErroStore> {
        let mut relato = RelatoEfeitos::default();
        let efeitos = self
            .store
            .efeitos()
            .listar_pendentes(org, self.opcoes.lote)?;
        for efeito in efeitos {
            let id = efeito.id;
            if let Err(erro) = self.tratar(org, efeito, &mut relato).await {
                // Erro de banco num efeito não para os outros: ele segue na fila.
                tracing::warn!(
                    target: "poph_server::fluxos_efeitos",
                    efeito = %id,
                    motivo = %erro,
                    "efeito de fluxo não pôde ser atualizado; segue no próximo ciclo"
                );
            }
        }
        Ok(relato)
    }

    async fn tratar(
        &self,
        org: OrgId,
        efeito: EfeitoDoFluxo,
        relato: &mut RelatoEfeitos,
    ) -> Result<(), ErroStore> {
        let id = efeito.id;
        // "Entrou na fila": a primeira vez que o laço vê o efeito, ele ainda está `pendente`, sem
        // tentativa e sem espera marcada. Depois disso a espera é sempre gravada, então não conta
        // duas vezes.
        if efeito.estado == poph_types::EstadoEfeito::Pendente
            && efeito.tentativas == 0
            && efeito.proxima_tentativa.is_none()
        {
            self.contar(&|metricas| &metricas.efeitos_pendentes);
        }
        if efeito.tentativas >= self.opcoes.max_tentativas {
            self.falhar(org, &efeito, MOTIVO_TENTATIVAS_ESGOTADAS, true, relato)
                .await?;
            return Ok(());
        }

        let plano = self.plano(org, &efeito)?;
        match plano {
            Plano::Esperar(motivo) => {
                let espera = self.espera_sem_integracao();
                self.store
                    .efeitos()
                    .adiar(org, id, self.store.agora() + espera)?;
                relato.adiados += 1;
                tracing::debug!(
                    target: "poph_server::fluxos_efeitos",
                    efeito = %id,
                    tipo = %efeito.tipo,
                    motivo,
                    espera_ms = u64::try_from(espera).unwrap_or(u64::MAX),
                    "efeito de fluxo esperando: nada a fazer agora"
                );
            }
            Plano::Falhar(motivo) => {
                self.falhar(org, &efeito, &motivo, false, relato).await?;
            }
            Plano::Entregar(entrega) => {
                // A tentativa precisa estar gravada antes da chamada externa (D11).
                let tentativa = self.store.efeitos().iniciar_envio(org, id)?;
                match self.entregar(org, &efeito, entrega).await {
                    Ok(departamento) => {
                        self.store.efeitos().confirmar_envio(org, id)?;
                        self.contar(&|metricas| &metricas.efeitos_enviados);
                        self.registrar_evento(
                            org,
                            &efeito,
                            TipoEventoFicha::EfeitoEnviado,
                            "enviado",
                            tentativa,
                        );
                        // O encaminhamento também é um fato do atendimento, não só um efeito
                        // entregue: a linha do tempo ganha o evento que o atendente lê.
                        if let Some(departamento) = departamento {
                            self.registrar_evento(
                                org,
                                &efeito,
                                TipoEventoFicha::AtendimentoTransferido,
                                &departamento,
                                tentativa,
                            );
                        }
                        relato.enviados += 1;
                        tracing::info!(
                            target: "poph_server::fluxos_efeitos",
                            efeito = %id,
                            tipo = %efeito.tipo,
                            tentativa,
                            "efeito de fluxo entregue"
                        );
                    }
                    Err(EntregaFalhou::Transitoria(motivo)) => {
                        let espera = self.opcoes.backoff.espera(tentativa);
                        self.store.efeitos().falhar_envio(
                            org,
                            id,
                            &motivo,
                            false,
                            Some(self.store.agora() + espera_ms(espera)),
                        )?;
                        relato.transitorios += 1;
                        tracing::warn!(
                            target: "poph_server::fluxos_efeitos",
                            efeito = %id,
                            tipo = %efeito.tipo,
                            tentativa,
                            motivo = %motivo,
                            "efeito de fluxo adiado: falha transitória, nova tentativa depois"
                        );
                    }
                    Err(EntregaFalhou::Definitiva(motivo)) => {
                        self.falhar(org, &efeito, &motivo, false, relato).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Decide o que fazer com o efeito **sem** mexer em estado.
    fn plano(&self, org: OrgId, efeito: &EfeitoDoFluxo) -> Result<Plano, ErroStore> {
        let payload = match serde_json::from_str::<Value>(&efeito.payload_json) {
            Ok(payload) => payload,
            Err(erro) => {
                return Ok(Plano::Falhar(format!(
                    "o efeito está gravado com JSON inválido: {erro}"
                )));
            }
        };
        let texto = |campo: &str| {
            payload
                .get(campo)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|valor| !valor.is_empty())
                .map(str::to_owned)
        };
        match efeito.tipo.as_str() {
            "mensagem_ao_cliente" => {
                let Some(texto) = texto("texto") else {
                    return Ok(Plano::Falhar(
                        "mensagem ao cliente sem `texto` no efeito".to_owned(),
                    ));
                };
                let ficha = self.store.fichas().obter(org, efeito.ficha_id)?;
                let Some(conversa) = ficha.conversation_id else {
                    return Ok(Plano::Falhar(
                        "a Ficha do efeito não tem conversa (apagada pela retenção)".to_owned(),
                    ));
                };
                let atual = self.store.conversas().obter(org, conversa)?;
                if atual.status != poph_store::StatusConversa::Aberta {
                    return Ok(Plano::Falhar(format!(
                        "a conversa está {} e não recebe mensagem do fluxo",
                        situacao_em_portugues(atual.status)
                    )));
                }
                // PRD §6.8: depois da resposta do turno. Enquanto o turno roda, a mensagem espera.
                if self.store.turnos().em_andamento(org, conversa)? {
                    return Ok(Plano::Esperar(
                        "o turno da conversa ainda está em andamento",
                    ));
                }
                Ok(Plano::Entregar(Entrega::TextoAoCliente { conversa, texto }))
            }
            "notificar_whatsapp" => {
                let (Some(para), Some(mensagem)) = (texto("para"), texto("mensagem")) else {
                    return Ok(Plano::Falhar(
                        "notificação sem `para` ou sem `mensagem` no efeito".to_owned(),
                    ));
                };
                let ficha = self.store.fichas().obter(org, efeito.ficha_id)?;
                let Some(conversa) = ficha.conversation_id else {
                    return Ok(Plano::Falhar(
                        "a Ficha do efeito não tem conversa: não há integração para notificar"
                            .to_owned(),
                    ));
                };
                let atual = self.store.conversas().obter(org, conversa)?;
                let Some(integracao) = atual.integration_id else {
                    return Ok(Plano::Esperar(
                        "a conversa não tem integração de WhatsApp (console ou CLI)",
                    ));
                };
                let Some(canal) = self.canal_de(org, integracao) else {
                    return Ok(Plano::Esperar(
                        "a integração da conversa não está configurada nesta instância",
                    ));
                };
                let Some(canal) = canal.canal.as_ref().map(Arc::clone) else {
                    return Ok(Plano::Esperar("a chave da Evolution não está no ambiente"));
                };
                Ok(Plano::Entregar(Entrega::Whatsapp {
                    canal,
                    para,
                    mensagem,
                }))
            }
            "encaminhar" => {
                let Some(nome) = texto("departamento") else {
                    return Ok(Plano::Falhar(
                        "encaminhamento sem `departamento` no efeito".to_owned(),
                    ));
                };
                let ficha = self.store.fichas().obter(org, efeito.ficha_id)?;
                let Some(conversa) = ficha.conversation_id else {
                    return Ok(Plano::Falhar(
                        "a Ficha do efeito não tem conversa: não há o que encaminhar".to_owned(),
                    ));
                };
                let Some(departamento) = self.store.departamentos().por_nome(org, &nome)? else {
                    return Ok(Plano::Falhar(format!(
                        "o departamento `{nome}` não existe na organização"
                    )));
                };
                Ok(Plano::Entregar(Entrega::Encaminhar {
                    conversa,
                    departamento: departamento.id,
                    nome: departamento.nome,
                }))
            }
            outro => Ok(Plano::Falhar(format!(
                "tipo de efeito desconhecido na fila: `{outro}`"
            ))),
        }
    }

    /// Faz a entrega. Não toca no estado da fila: quem confirma ou falha é [`Self::tratar`].
    async fn entregar(
        &self,
        org: OrgId,
        efeito: &EfeitoDoFluxo,
        entrega: Entrega,
    ) -> Result<Option<String>, EntregaFalhou> {
        match entrega {
            Entrega::TextoAoCliente { conversa, texto } => {
                // `turn_id` novo por tentativa, como a resposta humana (MX-04): o efeito não nasce
                // dentro de um turno. O preço é que uma queda entre enfileirar e confirmar deixa
                // duas mensagens iguais na `outbox` — a duplicata rara e aceita do PRD.
                let nova = NovaSaida {
                    conversation_id: conversa,
                    turn_id: TurnId::generate(),
                    seq: 0,
                    tipo: "text".to_owned(),
                    conteudo_json: serde_json::json!({ "texto": texto }).to_string(),
                };
                self.store
                    .caixa_saida()
                    .enfileirar(org, nova)
                    .map_err(|erro| {
                        EntregaFalhou::Transitoria(format!(
                            "não consegui enfileirar a mensagem ao cliente: {erro}"
                        ))
                    })?;
                Ok(None)
            }
            Entrega::Whatsapp {
                canal,
                para,
                mensagem,
            } => {
                let comando = poph_types::SendText {
                    idempotencia: format!("fluxo-efeito-{}", efeito.id),
                    peer: para,
                    texto: mensagem,
                };
                match canal.enviar_texto_recibo(comando).await {
                    Ok(_) => Ok(None),
                    // O motivo do cliente já vem limpo (sem telefone, JID ou chave).
                    Err(erro) if erro.eh_transitorio() => {
                        Err(EntregaFalhou::Transitoria(erro.to_string()))
                    }
                    Err(erro) => Err(EntregaFalhou::Definitiva(erro.to_string())),
                }
            }
            Entrega::Encaminhar {
                conversa,
                departamento,
                nome,
            } => {
                let detalhe = serde_json::json!({
                    "origem": "fluxo",
                    "departamento": nome,
                    "efeito": efeito.id.to_string(),
                })
                .to_string();
                self.store
                    .conversas()
                    .transferir(
                        org,
                        conversa,
                        Some(departamento),
                        Some("fluxo".to_owned()),
                        detalhe,
                    )
                    .map_err(|erro| {
                        EntregaFalhou::Transitoria(format!(
                            "não consegui transferir a conversa: {erro}"
                        ))
                    })?;
                Ok(Some(nome))
            }
        }
    }

    /// Marca o efeito como `falhou` e registra o evento na Ficha.
    async fn falhar(
        &self,
        org: OrgId,
        efeito: &EfeitoDoFluxo,
        motivo: &str,
        esgotado: bool,
        relato: &mut RelatoEfeitos,
    ) -> Result<(), ErroStore> {
        self.store
            .efeitos()
            .falhar_envio(org, efeito.id, motivo, true, None)?;
        self.contar(&|metricas| &metricas.efeitos_falhas);
        if esgotado {
            relato.esgotados += 1;
        } else {
            relato.definitivos += 1;
        }
        self.registrar_evento(
            org,
            efeito,
            TipoEventoFicha::EfeitoFalhou,
            motivo,
            efeito.tentativas,
        );
        tracing::warn!(
            target: "poph_server::fluxos_efeitos",
            efeito = %efeito.id,
            tipo = %efeito.tipo,
            tentativas = efeito.tentativas,
            motivo,
            "efeito de fluxo falhou de vez: a transição continua valendo"
        );
        Ok(())
    }

    /// Registra `efeito_enviado`/`efeito_falhou` na Ficha, uma vez por desfecho.
    ///
    /// A chave de idempotência é o próprio desfecho (`efeito:<id>:enviado`), então confirmar duas
    /// vezes não duplica a linha do tempo. Ficha fechada recusa evento novo: o efeito já está
    /// resolvido na fila e o aviso fica só no log.
    fn registrar_evento(
        &self,
        org: OrgId,
        efeito: &EfeitoDoFluxo,
        tipo: TipoEventoFicha,
        detalhe: &str,
        tentativa: u32,
    ) {
        let chave = format!("efeito:{}:{}", efeito.id, tipo.como_texto());
        let payload = serde_json::json!({
            "efeito": efeito.id.to_string(),
            "chave": efeito.chave,
            "tipo": efeito.tipo,
            "tentativa": tentativa,
            "detalhe": detalhe,
        })
        .to_string();
        let novo = poph_store::NovoEventoFicha {
            execucao_id: Some(efeito.execucao_id),
            tipo,
            ator_tipo: AtorEvento::Sistema,
            ator_ref: Some("fluxo"),
            payload_json: payload,
            chave_idempotencia: Some(&chave),
        };
        if let Err(erro) = self
            .store
            .fichas()
            .registrar_evento(org, efeito.ficha_id, novo)
        {
            tracing::warn!(
                target: "poph_server::fluxos_efeitos",
                efeito = %efeito.id,
                tipo = tipo.como_texto(),
                motivo = %erro,
                "não consegui registrar o desfecho do efeito na Ficha"
            );
        }
    }

    fn espera_sem_integracao(&self) -> i64 {
        espera_ms(self.opcoes.espera_sem_integracao)
    }
}

/// Por que a entrega não aconteceu.
enum EntregaFalhou {
    /// Vale tentar de novo.
    Transitoria(String),
    /// Não adianta repetir.
    Definitiva(String),
}

/// Texto gravado no `ultimo_erro` quando o efeito esgota as tentativas.
pub const MOTIVO_TENTATIVAS_ESGOTADAS: &str = "tentativas de entrega do efeito esgotadas";

/// Situação da conversa em português, para o motivo que fica gravado e vai ao log.
fn situacao_em_portugues(status: poph_store::StatusConversa) -> &'static str {
    match status {
        poph_store::StatusConversa::Aberta => "aberta",
        poph_store::StatusConversa::Transferida => "transferida para um atendente",
        poph_store::StatusConversa::Fechada => "encerrada",
    }
}

/// Milissegundos de uma espera, para a coluna de tempo.
fn espera_ms(espera: Duration) -> i64 {
    i64::try_from(espera.as_millis()).unwrap_or(i64::MAX)
}

/// Traduz o efeito do motor no que a fila grava (FX-20) — o **único** lugar do formato.
///
/// O adaptador do turno (FX-22/FX-23) usa esta função para montar [`EfeitoDaTransicao`], e o
/// entregador lê o mesmo formato em [`EfeitosFluxo::plano`]: não existem dois jeitos de escrever o
/// payload.
pub fn efeito_da_transicao(produzido: &EfeitoProduzido) -> EfeitoDaTransicao {
    let payload = match &produzido.efeito {
        poph_fluxos::ir::Efeito::MensagemAoCliente { texto } => {
            serde_json::json!({ "texto": texto })
        }
        poph_fluxos::ir::Efeito::NotificarWhatsapp { para, mensagem } => {
            serde_json::json!({ "para": para, "mensagem": mensagem })
        }
        poph_fluxos::ir::Efeito::Encaminhar { departamento } => {
            serde_json::json!({ "departamento": departamento })
        }
    };
    EfeitoDaTransicao {
        chave: produzido.chave.clone(),
        tipo: produzido.tipo().to_owned(),
        payload_json: payload.to_string(),
    }
}

/// Sobe o laço da fila de efeitos. Sem o motor de fluxos ligado, não sobe nada.
pub fn iniciar(estado: &Estado) -> Option<tokio::task::JoinHandle<()>> {
    if !estado.config.fluxos.ativo {
        return None;
    }
    let efeitos = Arc::new(EfeitosFluxo::novo(
        Arc::clone(&estado.store),
        &estado.canais_evolution,
        estado.config.blobs(),
        estado.metricas_fluxos(),
        OpcoesEfeitosFluxo::default(),
    ));
    let intervalo = efeitos.opcoes.intervalo;
    Some(tokio::spawn(async move {
        loop {
            if let Err(erro) = efeitos.ciclo().await {
                tracing::warn!(
                    target: "poph_server::fluxos_efeitos",
                    motivo = %erro,
                    "ciclo da fila de efeitos falhou; segue no próximo"
                );
            }
            tokio::time::sleep(intervalo).await;
        }
    }))
}
