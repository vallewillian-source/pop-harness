//! Saída pelo WhatsApp (EvolutionAPI, HR-26c): entrega ao canal o que o agente deixou na `outbox`.
//!
//! Roda como um laço dentro do `poph serve` (ver [`iniciar`]), uma vez por segundo, e só quando há
//! instância da Evolution configurada. Para cada item pendente:
//!
//! - **Só mexe no que é de WhatsApp configurado.** A conversa do item precisa ter uma integração
//!   que esteja na configuração do servidor. Item de console, de outro canal ou de integração não
//!   configurada fica como está, sem falhar: outro dono pode estar esperando por ele.
//! - **Sem chave da Evolution, o item fica pendente**, não falha: quando a chave entrar, sai.
//! - **Estados** (mesma regra da fila de saída, D11): `pending → sending → sent`. A tentativa é
//!   gravada como `sending` antes da chamada, então queda no meio deixa o item para o reenvio.
//! - **Falha transitória** (rede, 429, 5xx) volta para a fila com espera crescente; **definitiva**
//!   (4xx, arquivo acima de 16 MB, mídia expirada) vira `failed` com o motivo. Depois de
//!   [`OpcoesSaidaWhatsApp::max_tentativas`] tentativas, vira `failed`.
//! - **Nunca repete dentro da mesma tentativa.** O cliente da Evolution não repete envio; a
//!   repetição só acontece entre tentativas, como a fila de saída já aceita.
//!
//! Por que não usar `Fluxo::enviar_pendentes` do `poph-agent`: ele pega todos os itens da org, e
//! um item de console marcado como falha pelo canal do WhatsApp seria perdido. E `Fluxo::ciclo`
//! também despacha a entrada, que aqui não tem turno para processar.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_agent::Backoff;
use poph_agent::entrada_saida::FalhaEnvio;
use poph_agent::fila_integracao::{chave_idempotencia, comando_do_item};
use poph_evolution::{BlobsEmDisco, FonteDeMidia, WhatsAppEvolution};
use poph_store::{ErroStore, ItemSaida, Store};
use poph_types::{IntegrationId, OrgId, OutboxId};

use crate::estado::Estado;
use crate::evolution::CanalEvolution;

/// Ajustes do envio. Os padrões são os da fila de saída do motor.
#[derive(Debug, Clone)]
pub struct OpcoesSaidaWhatsApp {
    /// Itens lidos por org e por ciclo.
    pub lote: u32,
    /// Tentativas antes de desistir (vira `failed`).
    pub max_tentativas: u32,
    /// Espera entre tentativas transitórias.
    pub backoff: Backoff,
    /// Intervalo entre ciclos do laço.
    pub intervalo: Duration,
}

impl Default for OpcoesSaidaWhatsApp {
    fn default() -> Self {
        Self {
            lote: 100,
            max_tentativas: 8,
            backoff: Backoff::default(),
            intervalo: Duration::from_secs(1),
        }
    }
}

/// O que um ciclo fez. Só contagens: nada de texto de cliente nem de telefone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoSaidaWhatsApp {
    /// Itens confirmados pela Evolution (`sent`).
    pub enviados: u32,
    /// Itens que ficaram esperando (backoff ou sem chave).
    pub adiados: u32,
    /// Falhas transitórias (voltam para a fila).
    pub transitorios: u32,
    /// Falhas definitivas (`failed`).
    pub definitivos: u32,
    /// Itens que esgotaram as tentativas (`failed`).
    pub esgotados: u32,
}

/// Um canal configurado: a org, a integração e o canal pronto para enviar, se houver chave.
struct CanalSaida {
    org: OrgId,
    integracao: IntegrationId,
    /// `None` quando a chave da Evolution não está no ambiente.
    ///
    /// É o canal concreto (não `dyn Integration`) porque esta saída precisa do id que a Evolution
    /// devolve no envio (MX-40): o contrato genérico do canal não carrega id.
    canal: Option<Arc<WhatsAppEvolution>>,
}

/// O consumidor da saída do WhatsApp.
pub struct SaidaWhatsApp {
    store: Arc<Store>,
    canais: Vec<CanalSaida>,
    opcoes: OpcoesSaidaWhatsApp,
    /// Quando cada item com falha transitória pode tentar de novo.
    agenda: Mutex<HashMap<OutboxId, Instant>>,
}

impl SaidaWhatsApp {
    /// Consumidor para as instâncias dadas. `pasta_blobs` é onde a biblioteca grava os arquivos.
    pub fn novo(
        store: Arc<Store>,
        canais: &[CanalEvolution],
        pasta_blobs: PathBuf,
        opcoes: OpcoesSaidaWhatsApp,
    ) -> Self {
        let fonte: Arc<dyn FonteDeMidia> = Arc::new(BlobsEmDisco::nova(pasta_blobs));
        let canais = canais
            .iter()
            .map(|instancia| CanalSaida {
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
            opcoes,
            agenda: Mutex::new(HashMap::new()),
        }
    }

    /// Orgs que têm alguma instância configurada (sem repetir).
    fn orgs(&self) -> Vec<OrgId> {
        let mut orgs: Vec<OrgId> = Vec::new();
        for canal in &self.canais {
            if !orgs.contains(&canal.org) {
                orgs.push(canal.org);
            }
        }
        orgs
    }

    fn canal_de(&self, org: OrgId, integracao: IntegrationId) -> Option<&CanalSaida> {
        self.canais
            .iter()
            .find(|canal| canal.org == org && canal.integracao == integracao)
    }

    fn esperando(&self, id: OutboxId) -> bool {
        let agora = Instant::now();
        self.agenda
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .get(&id)
            .is_some_and(|proximo| agora < *proximo)
    }

    fn agendar(&self, id: OutboxId, espera: Duration) {
        self.agenda
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .insert(id, Instant::now() + espera);
    }

    fn esquecer(&self, id: OutboxId) {
        self.agenda
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .remove(&id);
    }

    /// Um ciclo: percorre os itens pendentes das orgs configuradas e tenta enviar os seus.
    pub async fn ciclo(&self) -> Result<RelatoSaidaWhatsApp, ErroStore> {
        let mut relato = RelatoSaidaWhatsApp::default();
        for org in self.orgs() {
            let itens = self
                .store
                .caixa_saida()
                .listar_pendentes(org, self.opcoes.lote)?;
            for item in itens {
                let id = item.id;
                if let Err(erro) = self.tratar(org, item, &mut relato).await {
                    // Erro de banco num item não para os outros: o item segue pendente.
                    tracing::warn!(
                        target: "poph_server::saida_whatsapp",
                        saida = %id,
                        motivo = %erro,
                        "item de saída do WhatsApp não pôde ser atualizado; segue no próximo ciclo"
                    );
                }
            }
        }
        Ok(relato)
    }

    async fn tratar(
        &self,
        org: OrgId,
        item: ItemSaida,
        relato: &mut RelatoSaidaWhatsApp,
    ) -> Result<(), ErroStore> {
        let conversa = self.store.conversas().obter(org, item.conversation_id)?;
        // Destino no canal: é o identificador do cliente (o "peer") gravado na conversa.
        let peer_externo = conversa.peer_externo.clone();
        // Não é do WhatsApp configurado aqui: não é deste consumidor, e não se mexe nele.
        let Some(integracao) = conversa.integration_id else {
            return Ok(());
        };
        let Some(instancia) = self.canal_de(org, integracao) else {
            return Ok(());
        };
        let Some(canal) = instancia.canal.clone() else {
            // Sem chave da Evolution: fica pendente até a chave entrar.
            relato.adiados += 1;
            return Ok(());
        };

        let id = item.id;
        let conversa = item.conversation_id;
        let tipo = rotulo_do_tipo(&item.tipo);
        if self.esperando(id) {
            relato.adiados += 1;
            return Ok(());
        }
        if item.tentativas >= self.opcoes.max_tentativas {
            self.store
                .caixa_saida()
                .falhar_envio(org, id, MOTIVO_TENTATIVAS_ESGOTADAS, true)?;
            self.esquecer(id);
            relato.esgotados += 1;
            tracing::warn!(
                target: "poph_server::saida_whatsapp",
                saida = %id,
                conversa = %conversa,
                tipo,
                "envio do WhatsApp desistido: tentativas esgotadas"
            );
            return Ok(());
        }

        // A tentativa precisa estar gravada antes da chamada externa (D11).
        let tentativa = self.store.caixa_saida().iniciar_envio(org, id)?;
        let Some(peer) = peer_externo else {
            // Sem destino não há envio, e repetir não adianta (mesmo desfecho de antes, quando a
            // tradução do item é que reprovava a conversa sem peer).
            self.store.caixa_saida().falhar_envio(
                org,
                id,
                "a conversa não tem identificador do cliente no canal",
                true,
            )?;
            self.esquecer(id);
            relato.definitivos += 1;
            tracing::warn!(
                target: "poph_server::saida_whatsapp",
                saida = %id,
                conversa = %conversa,
                tipo,
                "item de saída do WhatsApp sem destino no canal: não será repetido"
            );
            return Ok(());
        };
        let comando = match comando_do_item(&item, &peer, &chave_idempotencia(&item)) {
            Ok(comando) => comando,
            Err(falha) => {
                // Payload inválido ou destino ausente: nada a enviar, e repetir não adianta.
                self.store
                    .caixa_saida()
                    .falhar_envio(org, id, &falha.motivo, true)?;
                self.esquecer(id);
                relato.definitivos += 1;
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    saida = %id,
                    conversa = %conversa,
                    tipo,
                    motivo = %falha.motivo,
                    "item de saída do WhatsApp não pôde ser montado: não será repetido"
                );
                return Ok(());
            }
        };
        match canal.enviar_com_recibo(comando).await {
            Ok(id_externo) => {
                // O id que a Evolution deu à mensagem vai para a fila (MX-40): é o que a fase 2 usa
                // para reconhecer o eco do próprio envio. Canal sem id grava sem id.
                self.store
                    .caixa_saida()
                    .confirmar_envio_com_id(org, id, id_externo.as_deref())?;
                self.esquecer(id);
                relato.enviados += 1;
                tracing::info!(
                    target: "poph_server::saida_whatsapp",
                    saida = %id,
                    conversa = %conversa,
                    tipo,
                    com_id = id_externo.is_some(),
                    "item de saída do WhatsApp enviado"
                );
            }
            Err(erro) if erro.is_transient() => {
                let falha = FalhaEnvio::transitoria(erro.to_string());
                // O motivo já vem limpo do cliente (sem telefone, JID ou chave). Vai para o banco
                // e, aqui, uma linha por tentativa; sem a chave da Evolution nem o corpo do evento.
                self.store
                    .caixa_saida()
                    .falhar_envio(org, id, &falha.motivo, false)?;
                self.agendar(id, self.opcoes.backoff.espera(tentativa));
                relato.transitorios += 1;
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    saida = %id,
                    conversa = %conversa,
                    tipo,
                    tentativa,
                    motivo = %falha.motivo,
                    "envio do WhatsApp adiado: falha transitória, nova tentativa depois"
                );
            }
            Err(erro) => {
                let falha = FalhaEnvio::definitiva(erro.to_string());
                self.store
                    .caixa_saida()
                    .falhar_envio(org, id, &falha.motivo, true)?;
                self.esquecer(id);
                relato.definitivos += 1;
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    saida = %id,
                    conversa = %conversa,
                    tipo,
                    tentativa,
                    motivo = %falha.motivo,
                    "envio do WhatsApp falhou de vez: não será repetido"
                );
            }
        }
        Ok(())
    }
}

/// Texto gravado no `last_error` quando o item esgota as tentativas e vai para `failed`. Quem lê
/// o motivo da falha (o aviso da MX-19) compara com esta constante, não com o texto solto.
pub const MOTIVO_TENTATIVAS_ESGOTADAS: &str = "tentativas de envio esgotadas";

/// Nome do tipo de item para o log (sem o conteúdo).
fn rotulo_do_tipo(tipo: &str) -> &'static str {
    match tipo {
        "text" => "texto",
        "file" => "mídia",
        "presence" => "presença",
        _ => "outro",
    }
}

/// Sobe o laço de envio quando há instância da Evolution configurada. Sem instância, não sobe nada.
pub fn iniciar(estado: &Estado) -> Option<tokio::task::JoinHandle<()>> {
    if estado.canais_evolution.is_empty() {
        return None;
    }
    let saida = Arc::new(SaidaWhatsApp::novo(
        Arc::clone(&estado.store),
        &estado.canais_evolution,
        estado.config.blobs(),
        OpcoesSaidaWhatsApp::default(),
    ));
    let intervalo = saida.opcoes.intervalo;
    Some(tokio::spawn(async move {
        loop {
            if let Err(erro) = saida.ciclo().await {
                tracing::warn!(
                    target: "poph_server::saida_whatsapp",
                    motivo = %erro,
                    "ciclo de saída do WhatsApp falhou; segue no próximo"
                );
            }
            tokio::time::sleep(intervalo).await;
        }
    }))
}
