//! Estado compartilhado do servidor: banco, configuração e métricas.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use argon2::password_hash::PasswordHash;
use poph_agent::entry_output::Fluxo;
use poph_agent::turno::TurnEngine;
use poph_files::FileConfig;
use poph_flows::{Compiled, Ir, Limites, LoadFailure, RevisionCache, RevisionKey, compile};
use poph_store::Store;
use poph_types::{BaseMetrics, ConversationId, HarnessConfig, OrgId, Registry};

use crate::cipher::CipherKey;
use crate::cluster::ClusterVivo;
use crate::evolution::CanalEvolution;
use crate::fluxos_metricas::MetricasFluxos;
use crate::manifestos::CatalogoManifestos;

/// Um token global, já validado na subida (hash argon2 em formato PHC).
#[derive(Debug, Clone)]
pub struct TokenGlobal {
    /// Rótulo da configuração (vai para a auditoria).
    pub label: String,
    /// Hash PHC conferido na subida.
    pub hash: String,
}

/// Tudo o que os handlers precisam. É `Clone` (todos os campos são `Arc` ou cópia barata).
#[derive(Clone)]
pub struct Estado {
    /// Banco (escritor dedicado + leitores).
    pub store: Arc<Store>,
    /// Configuração carregada (já validada).
    pub config: Arc<HarnessConfig>,
    /// Registro de métricas do processo.
    pub registro: Arc<Registry>,
    /// Handles das métricas base (o `/metrics` atualiza os medidores).
    pub metricas: BaseMetrics,
    /// Tokens globais (só o hash).
    pub tokens_globais: Vec<TokenGlobal>,
    /// Quando o processo subiu (para `uptime` no `/healthz`).
    pub inicio: SystemTime,
    /// Instâncias da Evolution que entregam mensagens (HR-26b). Vazio = nenhum webhook aceito.
    pub canais_evolution: Arc<Vec<CanalEvolution>>,
    /// Fluxo de entrada e saída com o turno ligado (HR-26h). `None` = o servidor recebe mensagens
    /// mas não responde: elas ficam na fila até o turno ser ligado (ex.: chave do modelo ausente).
    pub turno: Option<Fluxo>,
    /// Intervalo entre duas leituras da fila de entrada, quando o turno está ligado.
    pub intervalo_turno: Duration,
    /// Manifestos dos plugins declarados em `[plugins].integracoes` (HR-44c), resolvidos sob
    /// demanda e guardados em memória.
    pub manifestos: Arc<CatalogoManifestos>,
    /// Chave de cifra dos segredos de integração (`POPH_CHAVE_CIFRA`). `None` = segredo não é
    /// gravado nem lido (a API responde `503 cifra_indisponivel`).
    pub chave_de_cifra: Option<Arc<CipherKey>>,
    /// Motor do turno (HR-22): o chat de teste do Console roda o mesmo motor da conversa, com o
    /// mesmo provedor, memória e ferramentas. `None` = sem chave do modelo: o chat de teste
    /// responde `503 turno_indisponivel` em vez de abrir um turno que não pode responder.
    pub motor: Option<Arc<TurnEngine>>,
    pub(crate) site_workers: Arc<crate::site_chat::SiteWorkers>,
    /// Configuração dos arquivos recebidos (HR-22/HR-23): limite de 16 MB, extração isolada.
    pub arquivos: Arc<FileConfig>,
    /// Conversas de teste com turno em voo (HR-22): a segunda aba recebe `409`, nunca um segundo
    /// turno na mesma conversa. O valor é o instante da reserva: uma reserva que passe da validade
    /// é descartada (HR-57), para uma conversa não ficar presa para sempre.
    pub testes_em_voo: Arc<Mutex<HashMap<ConversationId, Instant>>>,
    /// Quanto tempo uma reserva de turno de teste vale (HR-57).
    pub validade_da_reserva: Duration,
    /// Receptor do Application Service do Matrix (MX-16). `None` = módulo Matrix desligado nesta
    /// instância: as rotas `/_matrix/app/...` respondem `404`. Quem injeta é a raiz de composição
    /// (`poph serve`, MX-20), com `ModuloMatrix::preparar` + `AsReceiver::of_module`.
    pub matrix_as: Option<Arc<crate::api::matrix_as::AsReceiver>>,
    /// Métricas `poph_matrix_*` (MX-26), declaradas no registro só quando o módulo é preparado. Ver
    /// [`Estado::metricas_matrix`].
    matrix_metrics: Arc<Mutex<Option<Arc<crate::matrix_metrics::MetricsMatrix>>>>,
    /// Sinal de vida do módulo Matrix (MX-20): `None` = módulo desligado nesta instância. É a sonda
    /// de conexão que a API administrativa (MX-22) lê como "conectado".
    matrix_conexao: Option<Arc<crate::matrix_beginning::ConnectionSignal>>,
    /// Espaços por departamento do módulo Matrix (MX-12), montados pela raiz de composição (MX-20)
    /// para a API administrativa (MX-22) criar e ligar espaço com a mesma peça que o laço de estado
    /// usa. `None` = módulo desligado nesta instância: as rotas de espaço respondem `409`.
    matrix_espacos: Option<Arc<crate::matrix_beginning::ModuleSpaces>>,
    /// Cache das revisões compiladas de fluxo (FX-19), com o teto `fluxos.cache_revisoes_mb` (FX-26).
    /// Um só por processo: `Clone` compartilha o estado.
    pub cache_fluxos: Arc<RevisionCache>,
    /// Métricas `poph_fluxos_*` (FX-26). `None` = motor de fluxos desligado (`[fluxos] ativo = false`):
    /// nenhuma `poph_fluxos_*` aparece no `/metrics`.
    metricas_fluxos: Option<Arc<MetricasFluxos>>,
    /// Mapa de roteamento entre instâncias (HR-50a), trocado inteiro pela recarga.
    cluster: Arc<RwLock<ClusterVivo>>,
    /// Nonces de encaminhamento já recebidos (HR-50b): nonce → instante em que chegou. Janela
    /// anti-repetição, podada a cada inserção.
    nonces_vistos: Arc<Mutex<HashMap<String, Instant>>>,
}

/// Validade padrão da reserva de turno de teste (HR-57).
///
/// A reserva é liberada no fim do stream (fim, erro ou desconexão) e o processo morto leva a memória
/// junto, então este prazo é só rede de segurança para uma reserva que escape dentro de um processo
/// vivo. Trinta minutos ficam muito acima do maior turno que os limites do provedor permitem.
pub const VALIDADE_DA_RESERVA: Duration = Duration::from_secs(30 * 60);

impl Estado {
    /// Monta o estado e registra o catálogo de métricas.
    ///
    /// Falha quando um hash de token global não é argon2 PHC: melhor não subir do que subir com
    /// um token que nunca vai autenticar sem dizer por quê.
    pub fn novo(store: Arc<Store>, config: HarnessConfig) -> Result<Self, String> {
        let mut tokens_globais = Vec::with_capacity(config.admin.tokens_globais.len());
        for token in &config.admin.tokens_globais {
            PasswordHash::new(&token.hash).map_err(|erro| {
                format!(
                    "token global `{}`: hash argon2 inválido ({}); gere outro com `poph token global`",
                    token.label, erro
                )
            })?;
            tokens_globais.push(TokenGlobal {
                label: token.label.clone(),
                hash: token.hash.clone(),
            });
        }
        if tokens_globais.is_empty() {
            tracing::warn!(
                "nenhum token global configurado ([admin].tokens_globais): só tokens de org \
                 autenticam na API"
            );
        }
        let manifestos = Arc::new(CatalogoManifestos::novo(&config.plugins));
        let arquivos = Arc::new(FileConfig::default(config.blobs()));
        let registro = Arc::new(Registry::new());
        let metricas = BaseMetrics::register(&registro)
            .map_err(|erro| format!("não consegui registrar as métricas: {erro}"))?;
        let cache_fluxos = Arc::new(RevisionCache::new(
            usize::try_from(config.flows.revision_cache_mb).unwrap_or(usize::MAX),
        ));
        let metricas_fluxos = if config.flows.enabled {
            let metricas = MetricasFluxos::registrar(&registro)
                .map_err(|erro| format!("não consegui registrar as métricas de fluxos: {erro}"))?;
            // FX-40: quem escreve `fluxo_execucoes` é o store, então ele recebe os handles das
            // séries já declaradas aqui — o nome não é declarado duas vezes.
            store.publish_execution_metrics(metricas.das_execucoes());
            Some(Arc::new(metricas))
        } else {
            None
        };
        Ok(Self {
            store,
            config: Arc::new(config),
            registro,
            metricas,
            tokens_globais,
            inicio: SystemTime::now(),
            canais_evolution: Arc::new(Vec::new()),
            turno: None,
            intervalo_turno: Duration::from_millis(1000),
            manifestos,
            chave_de_cifra: CipherKey::from_environment().map(Arc::new),
            motor: None,
            site_workers: Arc::default(),
            arquivos,
            testes_em_voo: Arc::new(Mutex::new(HashMap::new())),
            validade_da_reserva: VALIDADE_DA_RESERVA,
            // O módulo Matrix só existe quando a raiz de composição injeta o receptor (MX-20).
            matrix_as: None,
            matrix_metrics: Arc::new(Mutex::new(None)),
            matrix_conexao: None,
            matrix_espacos: None,
            // Modo instância única até a raiz de composição ler o `cluster.toml` (HR-50a).
            cluster: Arc::new(RwLock::new(ClusterVivo::instancia_unica(PathBuf::from(
                "cluster.toml",
            )))),
            nonces_vistos: Arc::new(Mutex::new(HashMap::new())),
            cache_fluxos,
            metricas_fluxos,
        })
    }

    /// Métricas `poph_fluxos_*` (FX-26). `None` quando o motor de fluxos está desligado.
    pub fn metricas_fluxos(&self) -> Option<Arc<MetricasFluxos>> {
        self.metricas_fluxos.clone()
    }

    /// Compila o script de um fluxo com os tetos da configuração `[fluxos]` (FX-26b): tamanho do
    /// script e nós por fluxo. É a porta de compilação do servidor; não usa o padrão do compilador.
    pub fn compilar_fluxo(&self, fonte: &str) -> Compiled {
        compile(fonte, &Limites::from_config(&self.config.flows))
    }

    /// Carrega uma revisão de fluxo pelo cache do processo (FX-19), com os tetos da configuração
    /// `[fluxos]` (FX-26b). Falha com o mesmo erro tipado de [`CacheRevisoes::carregar`].
    pub fn carregar_revisao_de_fluxo(
        &self,
        chave: &RevisionKey,
        ir_gravada: &Ir,
        fonte: &str,
    ) -> Result<Arc<Ir>, LoadFailure> {
        self.cache_fluxos.load(
            chave,
            ir_gravada,
            fonte,
            &Limites::from_config(&self.config.flows),
        )
    }

    /// Registra um nonce de encaminhamento recebido (HR-50b).
    ///
    /// `false` = o mesmo nonce já chegou dentro da janela: é repetição e o pedido tem de ser
    /// recusado. A memória guarda só a janela de
    /// [`crate::encaminhamento::JANELA_DE_REPLAY`]; ao passar do teto, é zerada com um aviso (o pior
    /// caso é uma repetição passar uma vez, nunca um pedido legítimo ser recusado para sempre).
    pub fn registrar_nonce(&self, nonce: &str) -> bool {
        let mut vistas = self
            .nonces_vistos
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner());
        let janela = crate::encaminhamento::JANELA_DE_REPLAY;
        vistas.retain(|_, quando| quando.elapsed() < janela);
        if vistas.len() >= crate::encaminhamento::MAXIMO_DE_NONCES {
            tracing::warn!(
                guardados = vistas.len(),
                "memória de nonces do encaminhamento cheia: zerada para continuar aceitando pedidos"
            );
            vistas.clear();
        }
        if vistas.contains_key(nonce) {
            return false;
        }
        vistas.insert(nonce.to_owned(), Instant::now());
        true
    }

    /// Liga o motor do turno ao servidor (HR-22). É o mesmo motor que o fluxo usa; o chat de teste
    /// roda um turno direto nele, com observador próprio.
    pub fn com_motor(mut self, motor: Arc<TurnEngine>) -> Self {
        self.motor = Some(motor);
        self
    }

    /// Troca a configuração dos arquivos recebidos (testes: extrator próprio).
    pub fn com_arquivos(mut self, arquivos: FileConfig) -> Self {
        self.arquivos = Arc::new(arquivos);
        self
    }

    /// Reserva a conversa para um turno de teste. `false` = já há turno em voo nela (HR-22).
    ///
    /// A reserva vale por [`Estado::validade_da_reserva`] (HR-57): uma reserva mais velha que isso
    /// é considerada vazada (o turno que a segurava morreu sem soltar) e é substituída, senão a
    /// conversa ficaria presa em `409 turno_em_andamento` para sempre.
    pub fn reservar_turno_de_teste(&self, conversa: ConversationId) -> bool {
        let mut reservas = self
            .testes_em_voo
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner());
        if let Some(inicio) = reservas.get(&conversa) {
            if inicio.elapsed() < self.validade_da_reserva {
                return false;
            }
            tracing::warn!(
                conversa = %conversa,
                validade_ms = u64::try_from(self.validade_da_reserva.as_millis()).unwrap_or(u64::MAX),
                "reserva de turno de teste vencida: a conversa volta a aceitar mensagem"
            );
        }
        reservas.insert(conversa, Instant::now());
        true
    }

    /// Solta a reserva da conversa (fim do stream, erro ou desconexão do cliente).
    pub fn liberar_turno_de_teste(&self, conversa: ConversationId) {
        self.testes_em_voo
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .remove(&conversa);
    }

    /// Troca a validade da reserva de turno de teste (teste do vencimento, sem esperar 30 min).
    pub fn com_validade_da_reserva(mut self, validade: Duration) -> Self {
        self.validade_da_reserva = validade;
        self
    }

    /// Define o mapa do cluster em uso (HR-50a). A raiz de composição chama na subida; os testes
    /// montam um mapa próprio sem tocar no ambiente do processo.
    pub fn com_cluster(mut self, cluster: ClusterVivo) -> Self {
        self.cluster = Arc::new(RwLock::new(cluster));
        self
    }

    /// Cópia do mapa em uso. A recarga troca o conteúdo inteiro; quem lê nunca vê meio mapa.
    pub fn cluster(&self) -> ClusterVivo {
        self.cluster
            .read()
            .unwrap_or_else(|veneno| veneno.into_inner())
            .clone()
    }

    /// Troca o mapa em uso (a recarga chama depois de ler e validar o arquivo novo).
    pub fn trocar_cluster(&self, novo: ClusterVivo) {
        *self
            .cluster
            .write()
            .unwrap_or_else(|veneno| veneno.into_inner()) = novo;
    }

    /// Põe as instâncias da Evolution resolvidas na subida (feito pela raiz de composição).
    pub fn com_canais_evolution(mut self, canais: Vec<CanalEvolution>) -> Self {
        self.canais_evolution = Arc::new(canais);
        self
    }

    /// Liga o receptor do Application Service do Matrix (MX-16). Sem ele, as rotas do Synapse
    /// respondem `404` — é o módulo desligado.
    pub fn com_matrix_as(mut self, receptor: Arc<crate::api::matrix_as::AsReceiver>) -> Self {
        self.matrix_as = Some(receptor);
        self
    }

    pub(crate) fn matrix_blob_folder(&self) -> &std::path::Path {
        &self.arquivos.blob_folder
    }

    /// Liga o receptor já montado na instância viva (MX-20): é o que a raiz de composição usa,
    /// porque ela precisa mexer no estado que vai ser servido, não numa cópia.
    pub fn set_matrix_as(&mut self, receptor: Arc<crate::api::matrix_as::AsReceiver>) {
        self.matrix_as = Some(receptor);
    }

    /// Guarda o sinal de vida do módulo (MX-20) para quem pergunta depois (`/admin/v1/...`).
    pub fn set_matrix_connection_signal(
        &mut self,
        sinal: Arc<crate::matrix_beginning::ConnectionSignal>,
    ) {
        self.matrix_conexao = Some(sinal);
    }

    /// Guarda o serviço de espaços por departamento para a API administrativa (MX-22).
    pub fn set_matrix_spaces(&mut self, espacos: Arc<crate::matrix_beginning::ModuleSpaces>) {
        self.matrix_espacos = Some(espacos);
    }

    /// Espaços por departamento do módulo Matrix, quando ele está montado nesta instância.
    ///
    /// `None` = módulo desligado (ou sem configuração/segredos para subir): a API administrativa
    /// responde `409` e não tenta falar com o Synapse.
    pub fn matrix_espacos(&self) -> Option<Arc<crate::matrix_beginning::ModuleSpaces>> {
        self.matrix_espacos.as_ref().map(Arc::clone)
    }

    /// O módulo Matrix está falando com o Synapse? `None` = módulo desligado nesta instância.
    ///
    /// É o "conectado" do estado do módulo: `Some(false)` significa degradado (o servidor segue
    /// atendendo; os laços retentam com espera crescente).
    pub fn matrix_conectado(&self) -> Option<bool> {
        self.matrix_conexao.as_ref().map(|sinal| sinal.connected())
    }

    /// Métricas `poph_matrix_*` do módulo Matrix (MX-26). Declara-as no registro do processo na
    /// primeira chamada e devolve o mesmo handle depois: quem prepara o módulo (MX-20) chama isto,
    /// e só então as métricas aparecem no `/metrics`.
    ///
    /// Devolve `None` quando o módulo não está ligado nesta instância (`[matrix]` ausente ou
    /// `habilitado = false`): assim nenhuma `poph_matrix_*` entra no `/metrics` de uma instância que
    /// não usa o módulo, mesmo que quem chama não confira a configuração antes.
    pub fn matrix_metrics(
        &self,
    ) -> poph_types::DomainResult<Option<Arc<crate::matrix_metrics::MetricsMatrix>>> {
        if !self.modulo_matrix_ligado() {
            return Ok(None);
        }
        let mut guarda = self
            .matrix_metrics
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner());
        if let Some(ja) = guarda.as_ref() {
            return Ok(Some(Arc::clone(ja)));
        }
        let metricas = Arc::new(crate::matrix_metrics::MetricsMatrix::register(
            &self.registro,
        )?);
        *guarda = Some(Arc::clone(&metricas));
        Ok(Some(metricas))
    }

    /// O módulo Matrix está ligado nesta instância? (`[matrix] habilitado = true`.)
    fn modulo_matrix_ligado(&self) -> bool {
        self.config
            .matrix
            .as_ref()
            .is_some_and(|matrix| matrix.enabled)
    }

    /// Define a chave de cifra dos segredos de integração (a raiz de composição passa a do
    /// ambiente; os testes passam uma própria, sem mexer em variável de processo).
    pub fn com_chave_de_cifra(mut self, chave: Option<CipherKey>) -> Self {
        self.chave_de_cifra = chave.map(Arc::new);
        self
    }

    /// Liga o turno no servidor (HR-26h): o fluxo montado pela raiz de composição e o intervalo de
    /// leitura da fila de entrada.
    pub fn com_turno(mut self, fluxo: Fluxo, intervalo: Duration) -> Self {
        self.turno = Some(fluxo);
        self.intervalo_turno = intervalo;
        self
    }

    /// Canal da instância, dentro da org. `None` quando a org ou a instância não está configurada.
    pub fn canal_evolution(&self, org: &str, instancia: &str) -> Option<&CanalEvolution> {
        let org: OrgId = org.parse().ok()?;
        self.canais_evolution
            .iter()
            .find(|canal| canal.org == org && canal.instancia == instancia)
    }

    /// Canal que atende uma integração da org (HR-44b: pareamento, estado e desconexão).
    ///
    /// `None` quando a integração não está declarada em `[evolution].instancias` para esta org —
    /// é o servidor dizendo que não sabe em qual instância da Evolution ela vive.
    pub fn canal_evolution_da_integracao(
        &self,
        org: OrgId,
        integracao: poph_types::IntegrationId,
    ) -> Option<&CanalEvolution> {
        self.canais_evolution
            .iter()
            .find(|canal| canal.org == org && canal.integracao == integracao)
    }
}

impl std::fmt::Debug for Estado {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Estado")
            .field("tokens_globais", &self.tokens_globais.len())
            .field("inicio", &self.inicio)
            .finish_non_exhaustive()
    }
}
