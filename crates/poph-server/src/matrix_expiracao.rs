//! Expiração da mídia publicada nas salas (MX-46, fase 2).
//!
//! Depois de `matrix.midia_retencao_dias` (padrão 4, de 1 a 365), a mídia que o espelho publicou na
//! sala é apagada do repositório do Synapse e o evento é redigido com o bloco que o Element lê
//! (`com.<marca>.attachment_expired`, MX-46). O atendente vê o aviso "não está mais disponível" **no
//! lugar do evento**, desenhado pelo próprio frontend — é o mesmo mecanismo que a projeção do Python
//! usa hoje; não existe `m.notice` extra, e a sala não ganha uma mensagem por arquivo expirado.
//!
//! ## O que uma passagem faz
//!
//! 1. lê as orgs com o módulo ligado (`matrix_orgs`);
//! 2. em cada uma, lê de `matrix_midia` o que já venceu (`sent_at <= agora - dias`) e ainda tem etapa
//!    pendente, da mídia mais antiga para a mais nova, até [`LOTE_PADRAO`];
//! 3. para cada mídia: **redige** o evento na sala (como o fantasma do agente, moderador da sala) e
//!    **apaga** o arquivo pela API administrativa — cada etapa com o carimbo próprio no banco;
//! 4. o que falhou fica com espera crescente (`attempts`, `next_attempt_at`) e volta depois; a
//!    passagem seguinte atende os outros em vez de repetir o problemático.
//!
//! ## Regras
//!
//! - **Sem token de administração a rotina fica desligada**, com aviso claro no log na subida. Não é
//!   erro: o módulo sobe, espelha e responde, e só a expiração não roda. O token vive no tipo
//!   `Sensivel` e nunca aparece em log, erro, métrica ou teste.
//! - **Relógio injetável** ([`ExpiracaoMidia::com_relogio`]): os testes simulam o futuro sem dormir,
//!   e antes do prazo nada acontece.
//! - **Falha não vira laço apertado**: o item que falha sai da fila até `next_attempt_at`, e a espera
//!   dobra a cada falha até [`ESPERA_MAXIMA`] (o mesmo espírito do atraso do laço do espelho).
//! - **A retenção vale mais do que a arrumação**: se a redação falhar, o arquivo é apagado do mesmo
//!   jeito; a próxima passagem tenta só a redação.
//! - **Nada de dado de cliente em log**: só org, conversa, ids técnicos e o código curto do erro.
//!
//! ## Ligação
//!
//! Quem liga é o `matrix_inicio` (MX-20), junto com os outros laços: com o token presente, a tarefa
//! entra no runtime e `pedir_parada` a encerra no desligamento; sem o token, nada é criado e o log
//! diz o que falta.

use std::sync::Arc;
use std::time::Duration;

use poph_matrix::cliente::ClienteMatrix;
use poph_matrix::erro::{Classificacao, ErroMidia};
use poph_matrix::expiracao::{self, MidiaParaExpirar};
use poph_matrix::usuarios::IdsFantasmas;
use poph_store::{MidiaDaSala, Store};
use poph_types::{MatrixConfig, OrgId, Segredo};
use tokio::sync::watch;

use crate::matrix_metricas::MetricasMatrix;

/// Intervalo entre passagens. A retenção é medida em dias: uma passagem por hora basta para o
/// arquivo sair perto do prazo sem varrer o banco a cada minuto.
pub const PERIODO_PADRAO: Duration = Duration::from_secs(3_600);

/// Mídias que uma passagem atende por org (teto de trabalho e de memória).
pub const LOTE_PADRAO: u32 = 50;

/// Primeira espera de um item que falhou.
pub const ESPERA_BASE: Duration = Duration::from_secs(60);

/// Espera máxima de um item que insiste em falhar (ele volta de vez em quando, não some da fila).
pub const ESPERA_MAXIMA: Duration = Duration::from_secs(3_600);

/// Milissegundos em um dia (o prazo é configurado em dias).
const MS_POR_DIA: i64 = 86_400_000;

/// Fundo máximo do expoente da espera (evita estouro em item que falha há muito tempo).
const EXPOENTE_MAXIMO: u32 = 20;

/// Espera antes da próxima tentativa de um item que já falhou `tentativas` vezes.
///
/// Dobra a cada falha, com teto: um arquivo que não se consegue apagar continua sendo tentado de
/// vez em quando, em vez de sair da fila para sempre ou de ser chamado a cada passagem.
pub fn espera_apos(tentativas: u32, base: Duration, teto: Duration) -> Duration {
    let passo = tentativas.saturating_sub(1).min(EXPOENTE_MAXIMO);
    let fator = 1u32.checked_shl(passo).unwrap_or(u32::MAX);
    base.saturating_mul(fator).min(teto)
}

/// O que uma passagem fez. Só contagens: nada de texto de cliente, nome de arquivo ou token.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelatoExpiracao {
    /// Mídias lidas da fila.
    pub vencidas: u32,
    /// Eventos redigidos nesta passagem.
    pub redigidas: u32,
    /// Arquivos apagados do Synapse nesta passagem.
    pub apagadas: u32,
    /// Arquivos que já não existiam no Synapse (contam como apagados, sem chamada nova).
    pub ja_apagadas: u32,
    /// Mídias que ficaram para depois (alguma etapa falhou).
    pub adiadas: u32,
    /// Mídias que já estavam completas e não voltam mais.
    pub concluidas: u32,
    /// Falhas de leitura ou de gravação no banco nesta passagem.
    pub erros_de_banco: u32,
}

/// O serviço de expiração da mídia das salas.
pub struct ExpiracaoMidia {
    store: Arc<Store>,
    cliente: ClienteMatrix,
    /// Token da API administrativa do Synapse (nunca em log).
    token_admin: Segredo,
    ids: IdsFantasmas,
    /// Prazo de retenção, em dias, como está em `matrix.midia_retencao_dias`.
    retencao_dias: u32,
    lote: u32,
    periodo: Duration,
    espera_base: Duration,
    espera_maxima: Duration,
    metricas: Arc<MetricasMatrix>,
    /// Relógio em milissegundos (testes simulam o futuro).
    agora_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    parar: watch::Sender<bool>,
}

/// `Debug` manual de propósito: o token aparece como `[oculto]` (comportamento do `Sensivel`), e
/// um `{:?}` no serviço nunca vira vazamento em log ou relatório.
impl std::fmt::Debug for ExpiracaoMidia {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpiracaoMidia")
            .field("cliente", &self.cliente)
            .field("token_admin", &self.token_admin)
            .field("retencao_dias", &self.retencao_dias)
            .field("lote", &self.lote)
            .field("periodo", &self.periodo)
            .finish_non_exhaustive()
    }
}

impl ExpiracaoMidia {
    /// Monta o serviço a partir da seção `[matrix]` já validada e do token de administração.
    ///
    /// O token **não** vem da configuração: quem o leu do ambiente passa aqui, e a ausência dele é
    /// tratada antes (a rotina nem é criada).
    pub fn novo(
        store: Arc<Store>,
        cliente: ClienteMatrix,
        token_admin: Segredo,
        config: &MatrixConfig,
    ) -> Result<Self, poph_matrix::usuarios::ErroFantasmas> {
        let ids = IdsFantasmas::da_config(config)?;
        let (parar, _) = watch::channel(false);
        Ok(Self {
            store,
            cliente,
            token_admin,
            ids,
            retencao_dias: config.midia_retencao_dias.max(1),
            lote: LOTE_PADRAO,
            periodo: PERIODO_PADRAO,
            espera_base: ESPERA_BASE,
            espera_maxima: ESPERA_MAXIMA,
            metricas: MetricasMatrix::descartaveis(),
            agora_ms: Arc::new(crate::matrix_espelho::relogio_do_sistema),
            parar,
        })
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }

    /// Troca o relógio (testes simulam o futuro sem dormir).
    pub fn com_relogio(mut self, agora_ms: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.agora_ms = agora_ms;
        self
    }

    /// Mídias por org e por passagem (zero vale um).
    pub fn com_lote(mut self, lote: u32) -> Self {
        self.lote = lote.max(1);
        self
    }

    /// Intervalo entre passagens do laço de fundo.
    pub fn com_periodo(mut self, periodo: Duration) -> Self {
        self.periodo = periodo;
        self
    }

    /// Troca a espera da falha (testes com base curta provam o crescimento sem esperar minutos).
    pub fn com_espera(mut self, base: Duration, teto: Duration) -> Self {
        self.espera_base = base;
        self.espera_maxima = teto;
        self
    }

    /// Pede a parada do laço de fundo. Quem está no meio de uma mídia termina as duas chamadas dela.
    pub fn pedir_parada(&self) {
        self.parar.send_replace(true);
    }

    /// Laço de fundo: uma passagem a cada [`ExpiracaoMidia::com_periodo`] até a parada.
    ///
    /// Erro de uma passagem não derruba o laço: a próxima tenta de novo.
    pub async fn executar(self: Arc<Self>) {
        let mut parar = self.parar.subscribe();
        loop {
            if *parar.borrow() {
                break;
            }
            let relato = self.passar().await;
            tracing::debug!(
                target: "poph_server::matrix_expiracao",
                vencidas = relato.vencidas,
                redigidas = relato.redigidas,
                apagadas = relato.apagadas,
                adiadas = relato.adiadas,
                "passagem da expiração da mídia"
            );
            tokio::select! {
                _ = tokio::time::sleep(self.periodo) => {}
                _ = parar.changed() => break,
            }
        }
    }

    /// Uma passagem: redige e apaga o que já venceu, adia o que falhou.
    pub async fn passar(&self) -> RelatoExpiracao {
        let mut relato = RelatoExpiracao::default();
        let agora = (self.agora_ms)();
        let corte = agora - i64::from(self.retencao_dias) * MS_POR_DIA;

        let orgs = match self.store.matrix().orgs_ligadas() {
            Ok(orgs) => orgs,
            Err(_) => {
                tracing::warn!(
                    target: "poph_server::matrix_expiracao",
                    motivo = "banco_indisponivel",
                    "não consegui listar as orgs do módulo; a expiração tenta na próxima passagem"
                );
                relato.erros_de_banco += 1;
                return relato;
            }
        };

        let mut orgs_com_tratamento: Vec<OrgId> = Vec::new();
        for estado in orgs.iter().filter(|estado| estado.ligado) {
            let org = estado.org_id;
            let midias = match self
                .store
                .matrix_midia()
                .pendentes(org, corte, agora, self.lote)
            {
                Ok(midias) => midias,
                Err(_) => {
                    relato.erros_de_banco += 1;
                    tracing::warn!(
                        target: "poph_server::matrix_expiracao",
                        org = %org,
                        motivo = "banco_indisponivel",
                        "não consegui ler a fila da expiração; tenta na próxima passagem"
                    );
                    continue;
                }
            };
            relato.vencidas += u32::try_from(midias.len()).unwrap_or(u32::MAX);
            for midia in &midias {
                self.tratar(midia, agora, &mut relato).await;
            }
            orgs_com_tratamento.push(org);
        }

        // O número publicado é o que **sobrou** depois da passagem: é o que diz se a fila está
        // andando. Uma leitura por org, sem contar duas vezes o que já foi expirado agora.
        let mut pendentes = 0u32;
        for org in orgs_com_tratamento {
            match self.store.matrix_midia().pendentes_da_org(org) {
                Ok(total) => pendentes = pendentes.saturating_add(total),
                Err(_) => relato.erros_de_banco += 1,
            }
        }
        self.metricas
            .midias_pendentes_expiracao
            .definir(i64::from(pendentes));
        relato
    }

    /// As duas etapas de uma mídia. Cada uma é independente: a que falha não impede a outra.
    async fn tratar(&self, midia: &MidiaDaSala, agora: i64, relato: &mut RelatoExpiracao) {
        let mut falha: Option<ErroMidia> = None;

        if midia.falta_redigir() {
            match self.redigir(midia).await {
                Ok(()) => {
                    match self
                        .store
                        .matrix_midia()
                        .marcar_redigida(midia.org_id, midia.id, agora)
                    {
                        Ok(()) => {
                            relato.redigidas += 1;
                            self.metricas.midias_redigidas.incrementar(1);
                            tracing::info!(
                                target: "poph_server::matrix_expiracao",
                                org = %midia.org_id,
                                conversa = %midia.conversa,
                                "evento de mídia redigido na sala depois do prazo de retenção"
                            );
                        }
                        Err(_) => relato.erros_de_banco += 1,
                    }
                }
                Err(erro) => {
                    self.contar_erro(&erro);
                    tracing::warn!(
                        target: "poph_server::matrix_expiracao",
                        org = %midia.org_id,
                        conversa = %midia.conversa,
                        motivo = expiracao::codigo_do_erro(&erro),
                        classe = %erro.classificacao(),
                        "não consegui redigir o evento da mídia; o arquivo é apagado do mesmo jeito"
                    );
                    falha = Some(erro);
                }
            }
        }

        if midia.falta_apagar() {
            match expiracao::apagar(&self.cliente, &self.token_admin, &midia.mxc).await {
                Ok(apagou) => {
                    match self
                        .store
                        .matrix_midia()
                        .marcar_apagada(midia.org_id, midia.id, agora)
                    {
                        Ok(()) => {
                            relato.apagadas += 1;
                            if !apagou {
                                relato.ja_apagadas += 1;
                            }
                            self.metricas.midias_apagadas.incrementar(1);
                            tracing::info!(
                                target: "poph_server::matrix_expiracao",
                                org = %midia.org_id,
                                conversa = %midia.conversa,
                                ja_apagada = !apagou,
                                "arquivo de mídia apagado do Synapse depois do prazo de retenção"
                            );
                        }
                        Err(_) => relato.erros_de_banco += 1,
                    }
                }
                Err(erro) => {
                    self.contar_erro(&erro);
                    tracing::warn!(
                        target: "poph_server::matrix_expiracao",
                        org = %midia.org_id,
                        conversa = %midia.conversa,
                        motivo = expiracao::codigo_do_erro(&erro),
                        classe = %erro.classificacao(),
                        "não consegui apagar o arquivo da mídia; tenta de novo depois"
                    );
                    falha = falha.or(Some(erro));
                }
            }
        }

        match falha {
            Some(erro) => {
                let tentativas = midia.tentativas.saturating_add(1);
                let espera = espera_apos(tentativas, self.espera_base, self.espera_maxima);
                let proxima =
                    agora.saturating_add(i64::try_from(espera.as_millis()).unwrap_or(i64::MAX));
                if self
                    .store
                    .matrix_midia()
                    .adiar(
                        midia.org_id,
                        midia.id,
                        proxima,
                        &expiracao::codigo_do_erro(&erro),
                        agora,
                    )
                    .is_err()
                {
                    relato.erros_de_banco += 1;
                }
                relato.adiadas += 1;
                self.metricas.midias_expiracao_adiadas.incrementar(1);
            }
            None => {
                if !midia.falta_redigir() && !midia.falta_apagar() {
                    relato.concluidas += 1;
                }
            }
        }
    }

    /// Redige o evento da mídia na sala, como o fantasma do agente da conversa.
    async fn redigir(&self, midia: &MidiaDaSala) -> Result<(), ErroMidia> {
        let fantasma = self.ids.agente(&midia.agente).map_err(|_| {
            ErroMidia::Endereco(
                "o agente da conversa não vira usuário virtual nesta instalação".to_owned(),
            )
        })?;
        let id = midia.id.to_string();
        let para = MidiaParaExpirar {
            id: &id,
            nome: &midia.nome,
            mime: &midia.mime,
            categoria: midia.categoria.como_texto(),
            tamanho_bytes: midia.tamanho_bytes,
            enviada_em: midia.enviada_em,
        };
        expiracao::redigir(&self.cliente, &fantasma, &midia.sala, &midia.evento, &para)
            .await
            .map(|_| ())
            .map_err(ErroMidia::Chamada)
    }

    /// Conta a falha por classe (e o `429`), como o restante do módulo.
    fn contar_erro(&self, erro: &ErroMidia) {
        match erro.classificacao() {
            Classificacao::Transitoria => self.metricas.erros_transitorios.incrementar(1),
            Classificacao::Definitiva => self.metricas.erros_definitivos.incrementar(1),
        }
        if let ErroMidia::Chamada(poph_matrix::erro::ErroChamada::Resposta {
            status: 429, ..
        }) = erro
        {
            self.metricas.respostas_429.incrementar(1);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn a_espera_dobra_ate_o_teto() {
        let base = Duration::from_secs(60);
        let teto = Duration::from_secs(3_600);
        assert_eq!(espera_apos(1, base, teto), Duration::from_secs(60));
        assert_eq!(espera_apos(2, base, teto), Duration::from_secs(120));
        assert_eq!(espera_apos(3, base, teto), Duration::from_secs(240));
        assert_eq!(espera_apos(6, base, teto), Duration::from_secs(1_920));
        // No teto, e não passa dele nem com muita falha.
        assert_eq!(espera_apos(7, base, teto), teto);
        assert_eq!(espera_apos(1_000, base, teto), teto);
        // Sem falha não há espera (não é chamado nesse caso, mas a função não estoura).
        assert_eq!(espera_apos(0, base, teto), base);
    }

    #[test]
    fn o_prazo_conta_do_dia_de_retencao() {
        // Só para fixar a conta do corte usada em `passar`.
        let agora = 10 * MS_POR_DIA;
        assert_eq!(agora - i64::from(4) * MS_POR_DIA, 6 * MS_POR_DIA);
    }
}
