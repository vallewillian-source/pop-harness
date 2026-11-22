//! Ligação do módulo Matrix no `poph serve` (MX-20): junta as peças prontas e as sobe.
//!
//! Este arquivo é a **raiz de composição** do módulo dentro do servidor. Ele não decide nada do
//! que as peças já decidem — só monta cliente, ids, espaços e as três portas reais sobre o banco,
//! injeta os números do processo e põe os laços no ar:
//!
//! - **espelho das mensagens** ([`crate::matrix_espelho`], MX-14);
//! - **espelho do estado da conversa** ([`crate::matrix_estado`], MX-15);
//! - **receptor do Application Service** ([`crate::api::matrix_as`], MX-16), com a entrega da
//!   resposta do atendente ([`crate::matrix_resposta`], MX-18) injetada.
//!
//! ## O que vale saber
//!
//! - **Desligado é desligado.** `[matrix]` ausente ou `habilitado = false` → [`iniciar`] devolve
//!   `None`, nenhuma tarefa entra no runtime, nenhuma métrica `poph_matrix_*` é declarada e as
//!   rotas do Synapse respondem `404` (A8), porque o receptor não é injetado.
//! - **O módulo nunca segura o servidor.** Falha de configuração ou de conexão com o Synapse na
//!   subida não impede o `poph serve` de atender: o módulo fica **degradado**, o log leva um código
//!   curto e cada laço retenta no próprio ritmo. O turno e a saída do cliente não dependem daqui.
//! - **Ligar e desligar uma org não pede reinício.** Os dois laços releem `matrix_orgs` a cada
//!   ciclo/passagem (MX-08); desligar uma org para o trabalho dela até o ciclo seguinte.
//! - **Término ordenado.** No desligamento do processo os laços recebem o pedido de parada e
//!   terminam o que estão fazendo antes de sair; o cursor só anda depois do sucesso, então nada
//!   pendente se perde — a próxima subida continua de onde parou.
//! - **Nada de dado de cliente em log.** Só contagens, org, conversa e código curto.
//!
//! ## Como ligar
//!
//! O `poph serve` chama [`iniciar`] antes de servir e [`MatrixRodando::parar`] no desligamento (é o
//! que [`crate::servir_em`] faz). Os testes montam a mesma coisa com [`iniciar_com`], passando o
//! módulo já preparado (sem tocar no ambiente do processo).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use poph_matrix::config::{ErroMatrix, ModuloMatrix};
use poph_matrix::espacos::{EspacoGravado, OrigemEspaco, PortaEspacos};
use poph_matrix::salas::ErroPorta;
use poph_matrix::{ClienteMatrix, Espacos, IdsFantasmas, UsuariosVirtuais};
use poph_store::{ErroStore, OrigemEspaco as OrigemGravada, Store};
use poph_types::{ConversationId, DepartmentId, OrgId};
use tokio::sync::watch;

use crate::api::matrix_as::ReceptorAs;
use crate::estado::Estado;
use crate::matrix_espelho::{DestinoReal, EspelhoMatrix};
use crate::matrix_estado::{DestinoDoSynapse, EstadoDasConversas, PortaEstadoStore};
use crate::matrix_falhas_aviso::{
    AvisoDeFalhas, AvisoNoSynapse, PERIODO_PADRAO, PortaFalhasDoHarness,
};
use crate::matrix_resposta::RespostaDoAtendente;

/// Serviço de espaços por departamento (MX-12) montado sobre o repositório do banco.
///
/// É o tipo que a API administrativa (MX-22) usa para criar e ligar espaço: o mesmo que o laço de
/// estado (MX-15) usa por dentro, só com a porta real [`PortaEspacosStore`].
pub type EspacosDoModulo = poph_matrix::Espacos<PortaEspacosStore>;

/// Intervalo da sonda de conexão quando o Synapse está respondendo.
pub const PERIODO_DA_SONDA: Duration = Duration::from_secs(30);

/// Espera máxima entre duas sondas enquanto o Synapse não responde.
pub const TETO_DA_SONDA: Duration = Duration::from_secs(60);

/// Prazo para os laços terminarem o que estão fazendo no desligamento.
///
/// Passado o prazo, as tarefas são abortadas: o cursor do espelho só anda depois do sucesso, então
/// abortar no meio custa uma repetição idempotente, nunca perda nem duplicidade.
pub const PRAZO_DE_PARADA: Duration = Duration::from_secs(10);

/// O que a raiz de composição precisa saber sobre a conexão com o Synapse.
///
/// É o "conectado" da API administrativa (MX-22): a sonda fala com o Synapse de tempos em tempos e
/// conta aqui. A ausência de conexão nunca desliga nada — só aparece no estado e no log.
#[derive(Debug)]
pub struct SinalDeConexao {
    conectado: AtomicBool,
}

impl SinalDeConexao {
    fn nova() -> Self {
        Self {
            conectado: AtomicBool::new(false),
        }
    }

    /// `true` enquanto a última sonda falou com o Synapse.
    pub fn conectado(&self) -> bool {
        self.conectado.load(Ordering::Relaxed)
    }

    /// `true` enquanto a última sonda **não** falou com o Synapse.
    pub fn degradado(&self) -> bool {
        !self.conectado()
    }
}

/// Ajustes do início do módulo: o que muda entre produção e teste.
///
/// Em produção valem os padrões ([`AjustesDeInicio::padrao`]). Os testes encurtam a sonda e o prazo
/// de parada para não esperar dezenas de segundos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AjustesDeInicio {
    /// Intervalo da sonda de conexão quando o Synapse responde.
    pub periodo_da_sonda: Duration,
    /// Espera máxima entre duas sondas enquanto o Synapse não responde.
    pub teto_da_sonda: Duration,
    /// Prazo para os laços terminarem no desligamento.
    pub prazo_de_parada: Duration,
}

impl Default for AjustesDeInicio {
    fn default() -> Self {
        Self::padrao()
    }
}

impl AjustesDeInicio {
    /// Os valores de produção.
    pub const fn padrao() -> Self {
        Self {
            periodo_da_sonda: PERIODO_DA_SONDA,
            teto_da_sonda: TETO_DA_SONDA,
            prazo_de_parada: PRAZO_DE_PARADA,
        }
    }
}

/// O módulo ligado: as tarefas no ar e o pedido de parada de cada uma.
pub struct MatrixRodando {
    espelho: Arc<EspelhoMatrix<DestinoReal>>,
    estado_das_conversas:
        Arc<EstadoDasConversas<DestinoDoSynapse<PortaEspacosStore>, PortaEstadoStore>>,
    /// Aviso na sala quando a resposta humana falha de vez (MX-19).
    falhas_aviso: Arc<AvisoDeFalhas>,
    conexao: Arc<SinalDeConexao>,
    parar_sonda: watch::Sender<bool>,
    tarefas: Vec<tokio::task::JoinHandle<()>>,
    prazo_de_parada: Duration,
}

impl MatrixRodando {
    /// Sinal de conexão com o Synapse (o "conectado" da API administrativa).
    pub fn conexao(&self) -> Arc<SinalDeConexao> {
        Arc::clone(&self.conexao)
    }

    /// `true` enquanto a última sonda falou com o Synapse.
    pub fn conectado(&self) -> bool {
        self.conexao.conectado()
    }

    /// Troca o prazo do desligamento ordenado (testes não esperam os 10 s do padrão).
    pub fn com_prazo_de_parada(mut self, prazo: Duration) -> Self {
        self.prazo_de_parada = prazo;
        self
    }

    /// Para o módulo com calma: pede a parada dos três laços e espera o que está em andamento
    /// terminar. O que passar do prazo é abortado (o cursor não andou, então a próxima subida
    /// continua do mesmo ponto).
    pub async fn parar(self) {
        self.espelho.pedir_parada();
        self.estado_das_conversas.pedir_parada();
        self.falhas_aviso.pedir_parada();
        self.parar_sonda.send_replace(true);
        for tarefa in self.tarefas {
            if tokio::time::timeout(self.prazo_de_parada, tarefa)
                .await
                .is_err()
            {
                tracing::warn!(
                    target: "poph_server::matrix_inicio",
                    motivo = "prazo_de_parada_estourado",
                    "laço do módulo Matrix não terminou no prazo; abortado"
                );
            }
        }
        tracing::info!(
            target: "poph_server::matrix_inicio",
            "módulo Matrix parado"
        );
    }
}

/// Liga o módulo Matrix desta instância e injeta o receptor do Application Service no [`Estado`].
///
/// Devolve `None` quando o módulo está desligado (`[matrix]` ausente ou `habilitado = false`) ou
/// quando a configuração/segredos não permitem subir — nos dois casos o servidor segue sem o
/// módulo: sem tarefas, sem métricas e com as rotas do Synapse em `404`.
pub fn iniciar(estado: &mut Estado) -> Option<MatrixRodando> {
    let modulo = match ModuloMatrix::preparar(estado.config.matrix.clone()) {
        Ok(Some(modulo)) => modulo,
        Ok(None) => {
            // Desligado de propósito: nem os tokens são lidos do ambiente.
            return None;
        }
        Err(erro) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = codigo_do_modulo(&erro),
                "módulo Matrix ligado na configuração, mas não subiu; o servidor segue sem ele"
            );
            return None;
        }
    };
    iniciar_com(estado, modulo)
}

/// Igual a [`iniciar`], com o módulo já preparado (é o que os testes usam, sem tocar no ambiente).
///
/// Devolve `None` quando alguma peça não pôde ser montada; a configuração já foi validada antes.
pub fn iniciar_com(estado: &mut Estado, modulo: ModuloMatrix) -> Option<MatrixRodando> {
    iniciar_com_ajustes(estado, modulo, AjustesDeInicio::padrao())
}

/// Igual a [`iniciar_com`], com os ajustes de sonda e de desligamento escolhidos pelo chamador.
pub fn iniciar_com_ajustes(
    estado: &mut Estado,
    modulo: ModuloMatrix,
    ajustes: AjustesDeInicio,
) -> Option<MatrixRodando> {
    let store = Arc::clone(&estado.store);
    let metricas = estado.metricas_matrix().ok().flatten();

    let cliente = match ClienteMatrix::das_partes(&modulo.config, &modulo.segredos) {
        Ok(cliente) => cliente,
        Err(erro) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = codigo_da_chamada(&erro),
                "cliente do Matrix não foi montado; o módulo não sobe"
            );
            return None;
        }
    };
    let ids = match IdsFantasmas::da_config(&modulo.config) {
        Ok(ids) => ids,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = "fantasmas_invalidos",
                "ids dos usuários virtuais inválidos; o módulo não sobe"
            );
            return None;
        }
    };
    // Fantasmas da instância: o laço de estado renomeia o do cliente quando o nome chega depois da
    // criação da sala (MX-33), pelo mesmo serviço idempotente da MX-10.
    let usuarios = Arc::new(UsuariosVirtuais::novo(
        cliente.clone(),
        ids.clone(),
        poph_matrix::TETO_PADRAO_CACHE,
    ));

    // ───── espelho das mensagens (MX-14) ─────
    let mut destino = match DestinoReal::montar(Arc::clone(&store), cliente.clone(), &modulo.config)
    {
        Ok(destino) => destino,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = "espelho_nao_montado",
                "provisão de sala não foi montada; o módulo não sobe"
            );
            return None;
        }
    };
    if let Some(metricas) = &metricas {
        destino = destino.com_metricas(Arc::clone(metricas));
    }
    // ───── espaços do espelho (MX-12 + MX-31, item 2) ─────
    // Terceiro handle do mesmo serviço, só para a sala entrar no espaço **ao ser criada**: o laço
    // de estado pode ter visto a conversa antes de a sala existir, e sem isso a sala ficaria fora
    // do espaço até o cliente escrever de novo. Nenhuma decisão muda: quem move a sala quando o
    // departamento muda continua sendo o laço de estado (MX-15).
    match Espacos::da_config(
        cliente.clone(),
        PortaEspacosStore(Arc::clone(&store)),
        &modulo.config,
    ) {
        Ok(espacos_do_espelho) => destino = destino.com_espacos(espacos_do_espelho),
        Err(_) => {
            tracing::warn!(
                target: "poph_server::matrix_inicio",
                motivo = "espacos_do_espelho_nao_montados",
                "o espelho cria a sala sem espaço; o laço de estado posiciona na próxima alteração"
            );
        }
    }
    let mut espelho = match EspelhoMatrix::novo(Arc::clone(&store), destino, &modulo.config) {
        Ok(espelho) => espelho,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = "espelho_nao_montado",
                "laço do espelho não foi montado; o módulo não sobe"
            );
            return None;
        }
    };
    if let Some(metricas) = &metricas {
        espelho = espelho.com_metricas(Arc::clone(metricas));
    }

    // ───── espelho do estado (MX-15) ─────
    let espacos = match Espacos::da_config(
        cliente.clone(),
        PortaEspacosStore(Arc::clone(&store)),
        &modulo.config,
    ) {
        Ok(espacos) => espacos,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = "espacos_nao_montados",
                "espaços por departamento não foram montados; o módulo não sobe"
            );
            return None;
        }
    };
    // ───── espaços da API administrativa (MX-22) ─────
    // Segundo handle do mesmo serviço, só para a API criar e ligar espaço sob demanda: nenhuma
    // decisão muda (quem posiciona sala continua sendo o laço de estado, MX-15) e não há estado em
    // memória em `Espacos` — é cliente + ids + porta.
    match Espacos::da_config(
        cliente.clone(),
        PortaEspacosStore(Arc::clone(&store)),
        &modulo.config,
    ) {
        Ok(espacos_da_api) => estado.ligar_espacos_do_matrix(Arc::new(espacos_da_api)),
        Err(_) => {
            tracing::warn!(
                target: "poph_server::matrix_inicio",
                motivo = "espacos_da_api_nao_montados",
                "API administrativa do módulo fica sem os espaços; o resto do módulo segue"
            );
        }
    }

    // ───── aviso de falha da resposta humana (MX-19) ─────
    let mut falhas_aviso = AvisoDeFalhas::novo(
        Arc::new(PortaFalhasDoHarness::nova(Arc::clone(&store))),
        AvisoNoSynapse::novo(cliente.clone(), ids.clone()),
    );
    if let Some(metricas) = &metricas {
        falhas_aviso = falhas_aviso.com_metricas(Arc::clone(metricas));
    }

    let destino_estado = DestinoDoSynapse::novo(cliente.clone(), usuarios, espacos);
    let mut estado_das_conversas = match EstadoDasConversas::novo(
        PortaEstadoStore::nova(Arc::clone(&store)),
        destino_estado,
        &modulo.config,
    ) {
        Ok(laco) => laco,
        Err(_) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = "estado_nao_montado",
                "laço do estado da conversa não foi montado; o módulo não sobe"
            );
            return None;
        }
    };
    if let Some(metricas) = &metricas {
        estado_das_conversas = estado_das_conversas.com_metricas(Arc::clone(metricas));
    }

    // ───── receptor do Application Service (MX-16) com a resposta do atendente (MX-18) ─────
    let mut entrega = match RespostaDoAtendente::do_modulo(Arc::clone(&store), &modulo) {
        Ok(entrega) => entrega,
        Err(erro) => {
            tracing::error!(
                target: "poph_server::matrix_inicio",
                motivo = codigo_da_chamada(&erro),
                "entrega da resposta do atendente não foi montada; o módulo não sobe"
            );
            return None;
        }
    };
    if let Some(metricas) = &metricas {
        entrega = entrega.com_metricas(Arc::clone(metricas));
    }
    let receptor = ReceptorAs::do_modulo(modulo, Arc::new(entrega));
    estado.ligar_matrix_as(Arc::new(receptor));

    // ───── os laços no ar ─────
    let espelho = Arc::new(espelho);
    let estado_das_conversas = Arc::new(estado_das_conversas);
    let falhas_aviso = Arc::new(falhas_aviso);
    let conexao = Arc::new(SinalDeConexao::nova());
    // O sinal de vida também fica no `Estado`: é o "conectado" que a API administrativa lê.
    estado.ligar_sinal_do_matrix(Arc::clone(&conexao));
    let (parar_sonda, sinal_da_sonda) = watch::channel(false);
    let tarefas = vec![
        tokio::spawn(Arc::clone(&espelho).executar()),
        tokio::spawn(Arc::clone(&estado_das_conversas).executar()),
        tokio::spawn(Arc::clone(&falhas_aviso).executar(PERIODO_PADRAO)),
        tokio::spawn(sondar(
            cliente,
            Arc::clone(&conexao),
            ajustes.periodo_da_sonda,
            ajustes.teto_da_sonda,
            sinal_da_sonda,
        )),
    ];
    tracing::info!(
        target: "poph_server::matrix_inicio",
        servidor = %estado.config.matrix.as_ref().map(|matrix| matrix.server_name.as_str()).unwrap_or_default(),
        intervalo_ms = estado
            .config
            .matrix
            .as_ref()
            .map(|matrix| matrix.intervalo_ms)
            .unwrap_or_default(),
        "módulo Matrix ligado: espelho, estado e receptor do Application Service no ar"
    );
    Some(MatrixRodando {
        espelho,
        estado_das_conversas,
        falhas_aviso,
        conexao,
        parar_sonda,
        tarefas,
        prazo_de_parada: ajustes.prazo_de_parada,
    })
}

/// Sonda de conexão com o Synapse: marca o estado e retenta com espera crescente enquanto falha.
///
/// Ela não segura nada: se o Synapse estiver fora, o módulo fica degradado e o servidor atende
/// normalmente. Cada sonda é uma requisição leve (`/_matrix/client/versions`).
async fn sondar(
    cliente: ClienteMatrix,
    conexao: Arc<SinalDeConexao>,
    periodo: Duration,
    teto: Duration,
    mut parar: watch::Receiver<bool>,
) {
    let mut falhas = 0u32;
    loop {
        if *parar.borrow() {
            break;
        }
        let resultado = cliente.conferir_conexao().await;
        let espera = match resultado {
            Ok(()) => {
                if conexao.degradado() {
                    tracing::info!(
                        target: "poph_server::matrix_inicio",
                        "Synapse respondeu: módulo Matrix conectado"
                    );
                }
                conexao.conectado.store(true, Ordering::Relaxed);
                falhas = 0;
                periodo
            }
            Err(erro) => {
                if conexao.conectado() {
                    tracing::warn!(
                        target: "poph_server::matrix_inicio",
                        motivo = codigo_da_chamada(&erro),
                        "Synapse parou de responder: módulo Matrix degradado, sem derrubar o servidor"
                    );
                }
                conexao.conectado.store(false, Ordering::Relaxed);
                falhas = falhas.saturating_add(1);
                let fator = 1u32.checked_shl(falhas.min(10)).unwrap_or(u32::MAX);
                periodo.saturating_mul(fator).min(teto)
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(espera) => {}
            _ = parar.changed() => break,
        }
    }
}

// ───────────────────────────── portas reais ─────────────────────────────

/// Porta dos espaços por departamento (MX-12) sobre o repositório `matrix` do banco.
///
/// Fica aqui, e não em `poph-matrix`, pela mesma razão da [`PortaSalasStore`]: `poph-matrix` é
/// lógica do módulo e não conhece o banco; quem junta as duas pontas é o servidor.
pub struct PortaEspacosStore(pub Arc<Store>);

impl PortaEspacos for PortaEspacosStore {
    fn espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
    ) -> impl std::future::Future<Output = Result<Option<EspacoGravado>, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            let espaco = store
                .matrix()
                .espaco(org, departamento)
                .map_err(erro_porta)?;
            Ok(espaco.map(|espaco| EspacoGravado {
                room_id: espaco.room_id,
                alias: espaco.alias,
                origem: origem_da_gravada(espaco.origem),
            }))
        }
    }

    fn garantir_espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
        room_id: &str,
        alias: &str,
        origem: OrigemEspaco,
    ) -> impl std::future::Future<Output = Result<EspacoGravado, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        let room_id = room_id.to_owned();
        let alias = alias.to_owned();
        async move {
            let gravado = store
                .matrix()
                .garantir_espaco(org, departamento, &room_id, &alias, origem_gravada(origem))
                .map_err(erro_porta)?;
            Ok(EspacoGravado {
                room_id: gravado.room_id,
                alias: gravado.alias,
                origem: origem_da_gravada(gravado.origem),
            })
        }
    }

    fn espaco_da_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl std::future::Future<Output = Result<Option<String>, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            let sala = store.matrix().sala(org, conversa).map_err(erro_porta)?;
            Ok(sala.and_then(|sala| sala.espaco))
        }
    }

    fn definir_espaco_da_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        espaco: Option<&str>,
    ) -> impl std::future::Future<Output = Result<(), ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        let espaco = espaco.map(str::to_owned);
        async move {
            store
                .matrix()
                .definir_espaco_da_sala(org, conversa, espaco.as_deref())
                .map(|_| ())
                .map_err(erro_porta)
        }
    }
}

fn erro_porta(_: ErroStore) -> ErroPorta {
    // Sem o texto do erro do banco: ele pode carregar valor de cliente.
    ErroPorta::nova("armazenamento do módulo Matrix indisponível")
}

fn origem_da_gravada(origem: OrigemGravada) -> OrigemEspaco {
    match origem {
        OrigemGravada::Automatico => OrigemEspaco::Automatico,
        OrigemGravada::Ligado => OrigemEspaco::Ligado,
    }
}

fn origem_gravada(origem: OrigemEspaco) -> OrigemGravada {
    match origem {
        OrigemEspaco::Automatico => OrigemGravada::Automatico,
        OrigemEspaco::Ligado => OrigemGravada::Ligado,
    }
}

// ───────────────────────────── códigos curtos de log ─────────────────────────────

/// Código curto do erro de preparação (nunca o valor da configuração nem o token).
fn codigo_do_modulo(erro: &ErroMatrix) -> &'static str {
    match erro {
        ErroMatrix::Configuracao(_) => "configuracao_invalida",
        ErroMatrix::TokenAusente { .. } => "token_ausente",
    }
}

/// Código curto de um erro de chamada, sem o texto que o Synapse mandou.
fn codigo_da_chamada(erro: &poph_matrix::ErroChamada) -> &'static str {
    use poph_matrix::ErroChamada;
    match erro {
        ErroChamada::Configuracao(_) => "configuracao_invalida",
        ErroChamada::Rede { .. } => "rede",
        ErroChamada::Prazo { .. } => "prazo",
        ErroChamada::Resposta { .. } => "resposta_do_synapse",
        ErroChamada::RespostaInvalida { .. } => "resposta_invalida",
    }
}
