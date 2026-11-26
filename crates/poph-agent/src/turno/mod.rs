//! Motor do turno mínimo — o marco da fase 1 (PRD §6, HR-13).
//!
//! Um turno, do começo ao fim, com o que já existe nas tarefas anteriores:
//!
//! 1. a conversa e o agente vêm do banco; agente inativo (**D-05**) e conversa transferida/fechada
//!    (**D12**) não abrem turno — a mensagem fica registrada e o turno é gravado como `skipped`;
//! 2. **janela de agrupamento** (PRD §6.3): espera a janela do agente (teto configurável) e agrupa
//!    as mensagens seguidas do cliente que chegaram nesse intervalo num único turno;
//! 3. **prompt estável** (PRD §6.1): o markdown do compositor (HR-09) é o prefixo idêntico byte a
//!    byte; hora, canal, estado e resumo vêm depois, em bloco delimitado;
//! 4. **janela de contexto** (HR-11) com o orçamento de tokens do agente e o resumo rolante
//!    (HR-14), gerado em segundo plano depois da resposta;
//!    a **busca na base de conhecimento** (HR-17) começa antes da primeira chamada ao modelo e o
//!    resultado entra no bloco volátil, marcado como conteúdo não confiável;
//! 5. **laço de ferramentas** com limite de rodadas e de chamadas por mensagem, oferecendo só as
//!    ferramentas habilitadas do agente (**D-04**) — nesta tarefa o registro de produção está vazio
//!    (HR-17/18/24 registram as suas);
//! 6. **streaming de verdade**: cada pedaço de texto vai para o [`Observador`] conforme chega, e o
//!    turno pode ser cancelado pelo [`Cancelamento`];
//! 7. a resposta vai para a `outbox` (HR-12) e o turno fica registrado em `turns` com tempos por
//!    etapa, tokens, modelo, motivo do fim e erro.
//!
//! O Jev entra aqui na HR-28: antes da primeira chamada ao modelo, um **lote único** com as
//! perguntas dos gates ligados decide o que é escolha entre opções conhecidas. A decisão fica no
//! registro do turno (`turns.jev_json`) e no [`ResumoTurno`].
//!
//! Na HR-29 o **G1 passa a ter efeito**: decidida a transferência, o turno faz a segunda pergunta
//! (tipo de risco), grava a orientação aprovada seguida da mensagem automática, marca a conversa
//! como `transferred` e **não chama o modelo de texto**. Daí em diante a conversa não responde até
//! alguém reabrir (`poph conversation reopen`); as mensagens do cliente continuam gravadas.
//!
//! Na HR-30 o **G2 passa a ter efeito** sobre a janela de agrupamento: com ele ligado, o lote do Jev
//! sai na chegada da mensagem (dentro da janela) e a decisão ajusta a espera — `esperar_mais` estica
//! a janela até o teto do PRD §6.3, `processar_agora` a encurta para a confirmação curta, e o
//! fallback mantém a janela fixa do agente, o comportamento de antes.
//!
//! Na HR-32 o **G4 passa a ter efeito** sobre o departamento que recebe a transferência: a escolha do
//! Jev entre os departamentos ativos da empresa é validada pelo código (org, situação ativa, nome
//! sem ambiguidade) e, quando não serve, vale o departamento de transferência do agente — o
//! comportamento de hoje. Ver [`departamento`].
//!
//! Na HR-34 o **G6 passa a ter efeito** sobre a resposta antes do envio: o Jev revisa o texto, uma
//! reprovação custa **uma** regeneração com orientação fixa (derivada da opção escolhida) e a
//! segunda reprovação cai na resposta segura do agente, quando configurada. O cliente nunca fica
//! sem resposta por causa do gate.
//!
//! Na HR-35 o **G7 passa a ter efeito** conforme a ação configurada pelo agente: `registrar` só
//! anota a classificação, `responder_padrao` responde o texto neutro sem chamar o modelo e
//! `transferir` usa o caminho do G1. O texto de arquivo do cliente entra no prompt entre as marcas
//! de conteúdo não confiável, como dado e nunca instrução.
//!
//! O que ele **não** faz (e é de propósito): HTTP (HR-20) e transporte real de canal (HR-25/HR-26).

mod console;
mod departamento;
mod ferramentas;
mod fluxo;
mod fluxo_ferramentas;
mod pausa;
mod prompt;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use poph_core::llm::{
    Cancellation, FinishReason, LlmError, LlmEvent, LlmProvider, Message, Part, Request, ToolCall,
    Usage,
};
use poph_jev::{
    AcaoG7, AjusteGates, AvaliadorGates, EstadoJev, G1_TRANSFERIR, G3_NADA, G6_APROVADA,
    G7_RESPOSTA_PADRAO, G8_DESPEDIDA_PADRAO, G8_ENCERRADA, IdGate, MensagemJev, MotorJev,
    Orientacao, ResultadoJev, ResultadoOrientacao,
};
use poph_store::{
    Direction, Mensagem as MensagemGravada, NewAuditEntry, NewMessage, NewOutboxItem, Page, Role,
    Store, TurnFinalization, TurnState,
};
use poph_types::{
    AgentId, ConversationId, DomainError, JevGates, LlmConfig, MessageId, OrgId, TurnId,
};
use serde_json::json;

use crate::atores::{Futuro, Processador, Tarefa};
use crate::base_conhecimento::{BuscadorBase, ParametrosBusca, TrechoRecuperado};
use crate::config_quente::{AgenteQuente, ConfigQuente};
use crate::entrada_saida::{EventoEntrada, Fluxo};
use crate::erro::{ErroAgente, Resultado};
use crate::limites::Limitador;
use crate::metricas::{MetricasG9, MetricasTurno};
use crate::produtos::{BuscadorProdutos, ParametrosProdutos, ProdutoEncontrado};
use crate::provedores::{ProvedorResolvido, Provedores};
use crate::resumo::{self, OpcoesResumo, Resumidor};

pub use console::{TransporteConsole, texto_do_item};
pub use ferramentas::{
    ChamadaRegistrada, ContextoFerramenta, Desfecho, FerramentaInterna, FuturoFerramenta,
    MARCA_DE_CORTE, MOTIVO_ARGUMENTO_INVALIDO, MOTIVO_DESCONHECIDA, MOTIVO_ERRO_DE_EXECUCAO,
    MOTIVO_FORA_DO_TURNO, MOTIVO_LIMITE_DE_CHAMADAS, MOTIVO_NAO_HABILITADA, ProdutoUsado,
    RegistroChamadas, RegistroFerramentas, RegistroPadrao, SemFerramentas, TETO_RESUMO_FERRAMENTA,
    UsoFerramentas, cortar, oferecidas, registro_do_turno, resumo_de_argumentos,
};
pub use fluxo::{ABRE_FLUXOS, AjustesDoFluxo, FECHA_FLUXOS, NOME_FERRAMENTA_INICIAR};
pub use fluxo_ferramentas::{
    NOME_ABANDONAR, NOME_EXECUTAR_ACAO, eh_ferramenta_do_fluxo,
    registrar as registrar_ferramentas_do_fluxo,
    registrar_com as registrar_ferramentas_do_fluxo_com,
    registrar_encerramento as registrar_ferramenta_encerramento,
};
pub use pausa::{FuturoPausa, Pausa, PausaReal};
pub use prompt::{
    ABRE_BASE, ABRE_CONTEXTO, AVISO_BASE, FECHA_BASE, FECHA_CONTEXTO, Prompt, bloco_base,
    bloco_volatil, bloco_volatil_com_base, data_utc,
};

/// Quantas mensagens o turno lê do banco por vez (teto de varredura da janela).
const TETO_DE_LEITURA: u32 = 200;

/// Quantos itens da `inbox` a conclusão do turno varre por org.
const LOTE_CONCLUSAO: u32 = 500;

// ───────────────────── Anexos de imagem no prompt (HR-23b) ─────────────────────

/// Quantas imagens, no máximo, um turno manda ao modelo.
///
/// O teto é por turno (janela de contexto inteira), não por mensagem: o cliente que manda cinco
/// fotos numa mensagem faz o modelo ver as quatro primeiras e responde com o que tem. Sem teto, uma
/// conversa longa com fotos em todas as mensagens mandaria dezenas de imagens por chamada.
pub const MAX_IMAGENS_POR_TURNO: usize = 4;

/// Teto de bytes de **uma** imagem enviada ao modelo (5 MB).
///
/// O recebimento aceita 16 MB por arquivo (PRD §11); para o modelo, o mesmo arquivo vira base64
/// (~4/3 do tamanho) dentro do corpo do pedido. 5 MB por imagem mantém o pedido abaixo de ~7 MB por
/// imagem, com o conjunto de 4 imagens ainda dentro de um corpo razoável para o gateway.
pub const MAX_BYTES_POR_IMAGEM: u64 = 5 * 1024 * 1024;

/// Teto de bytes somados das imagens de um turno (8 MB).
pub const MAX_BYTES_DE_IMAGENS_POR_TURNO: u64 = 8 * 1024 * 1024;

/// Orçamento de imagens de um turno: quantas já entraram e quantos bytes somam.
#[derive(Debug, Default)]
struct OrcamentoImagens {
    quantidade: usize,
    bytes: u64,
}

impl OrcamentoImagens {
    /// A imagem de `tamanho` bytes ainda cabe no turno?
    fn cabe(&self, tamanho: u64) -> bool {
        tamanho > 0
            && tamanho <= MAX_BYTES_POR_IMAGEM
            && self.quantidade < MAX_IMAGENS_POR_TURNO
            && self.bytes.saturating_add(tamanho) <= MAX_BYTES_DE_IMAGENS_POR_TURNO
    }

    /// Registra a imagem que entrou.
    fn consumir(&mut self, tamanho: u64) {
        self.quantidade += 1;
        self.bytes = self.bytes.saturating_add(tamanho);
    }
}

// ───────────────────────────── Ajustes e resultados ─────────────────────────────

/// Janela curta de confirmação do G2 quando a mensagem está completa (HR-30).
///
/// Não é zero de propósito: duas mensagens digitadas juntas ("oi" e "quero um orçamento" no mesmo
/// segundo) ainda entram no mesmo turno. É curta o bastante para que "processar agora" signifique
/// de fato menos espera que a janela fixa.
pub const JANELA_CONFIRMACAO: Duration = Duration::from_millis(250);

/// Janela de agrupamento ajustada pelo G2 (HR-30), contada da chegada da mensagem que abriu o turno.
///
/// - `esperar_mais`: mais uma janela base, limitada ao teto do PRD §6.3;
/// - `processar_agora`: janela curta de confirmação ([`JANELA_CONFIRMACAO`]);
/// - sem decisão confiável (fallback), gate desligado ou Jev fora: a janela fixa do agente — o
///   comportamento de antes desta tarefa.
pub fn janela_com_g2(
    base: Duration,
    teto: Duration,
    confirmacao: Duration,
    decisao: Option<&str>,
) -> Duration {
    match decisao {
        Some(poph_jev::G2_ESPERAR) => base.saturating_mul(2).min(teto),
        Some(poph_jev::G2_PROCESSAR) => base.min(confirmacao),
        _ => base,
    }
}

/// Ajustes do motor do turno.
#[derive(Debug, Clone)]
pub struct OpcoesTurno {
    /// Janela de agrupamento quando o agente não define uma (`grouping_window_ms = 0`).
    pub janela_padrao: Duration,
    /// Teto da janela de agrupamento (PRD §6.3: teto proposto de 8 s).
    pub janela_maxima: Duration,
    /// Janela curta de confirmação quando o G2 diz "processar agora" (HR-30).
    pub janela_confirmacao: Duration,
    /// Quanto esperar por uma vaga de turno antes de desistir.
    pub espera_vaga: Duration,
    /// Teto de tempo de um turno inteiro (chamadas ao modelo incluídas).
    pub tempo_maximo_turno: Duration,
    /// Quantas mensagens ler do banco para montar a janela e o grupo.
    pub teto_leitura_mensagens: u32,
}

impl Default for OpcoesTurno {
    fn default() -> Self {
        Self {
            janela_padrao: Duration::from_secs(2),
            janela_maxima: Duration::from_secs(8),
            janela_confirmacao: JANELA_CONFIRMACAO,
            espera_vaga: Duration::from_secs(5),
            tempo_maximo_turno: Duration::from_secs(180),
            teto_leitura_mensagens: TETO_DE_LEITURA,
        }
    }
}

/// O que dispara um turno.
#[derive(Debug, Clone)]
pub struct Entrada {
    /// Org dona.
    pub org: OrgId,
    /// Conversa que recebeu a mensagem.
    pub conversa: ConversationId,
    /// Mensagem que disparou o turno (já gravada em `messages`).
    pub mensagem: MessageId,
    /// Cancelamento do turno (Ctrl-C no `chat`, timeout do chamador).
    pub cancelamento: Cancellation,
}

impl Entrada {
    /// Entrada sem cancelamento pedido.
    pub fn nova(org: OrgId, conversa: ConversationId, mensagem: MessageId) -> Self {
        Self {
            org,
            conversa,
            mensagem,
            cancelamento: Cancellation::novo(),
        }
    }
}

/// Evento que o turno avisa a quem está olhando (o `chat`, o Console, o log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventoTurno {
    /// O turno foi aberto no banco.
    Inicio {
        /// Turno aberto.
        turno: TurnId,
        /// Agente que atende.
        agente: AgentId,
        /// Conversa.
        conversa: ConversationId,
    },
    /// Pedaço de texto da resposta, na ordem em que chegou.
    Texto(String),
    /// Trechos da base de conhecimento que entraram no bloco volátil (HR-17).
    Base {
        /// Fontes, na ordem em que aparecem no prompt.
        fontes: Vec<String>,
    },
    /// O modelo pediu uma ferramenta (e o motor vai executá-la ou recusar).
    Ferramenta {
        /// Nome pedido pelo modelo.
        nome: String,
        /// Rodada do laço de ferramentas (0 = primeira chamada ao modelo).
        rodada: u32,
    },
    /// O G1 decidiu transferir: a conversa passou a `transferred` e o cliente recebeu a mensagem
    /// automática (com ou sem a orientação de risco) — HR-29.
    Transferencia {
        /// Departamento humano que recebe a conversa (o padrão do agente, quando configurado).
        departamento: Option<poph_types::DepartmentId>,
        /// Tipo de risco identificado (`ingestao_contato_produto`…), quando houve orientação.
        tipo_risco: Option<String>,
        /// `true` quando a orientação de primeiros cuidados saiu junto da mensagem automática.
        orientacao: bool,
    },
    /// O turno terminou (com texto ou vazio).
    Fim {
        /// Como terminou (`done`, `failed`, `skipped`).
        estado: String,
        /// Motivo do fim informado pelo provedor, quando houver.
        motivo: Option<String>,
    },
}

/// Quem acompanha o turno em tempo real.
pub trait Observador: Send {
    /// Recebe um evento. Não deve bloquear: roda dentro do turno.
    fn evento(&mut self, evento: EventoTurno);
}

/// Observador que ignora tudo (caminho da integração: a resposta vai pela `outbox`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ObservadorNulo;

impl Observador for ObservadorNulo {
    fn evento(&mut self, _evento: EventoTurno) {}
}

/// Tempos por etapa do turno (vão para `turns.timings_json`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tempos {
    /// Da chegada da mensagem que disparou até o turno começar a montar o prompt.
    pub fila_ms: u64,
    /// Montagem do prompt (janela de contexto, bloco volátil, ferramentas).
    pub montagem_ms: u64,
    /// Do início do turno até o primeiro pedaço de texto do modelo.
    pub primeiro_texto_ms: Option<u64>,
    /// Total do turno.
    pub total_ms: u64,
    /// Rodadas de ferramenta executadas.
    pub rodadas_ferramenta: u32,
}

/// Resultado de um turno, com o que foi gravado.
#[derive(Debug, Clone)]
pub struct ResumoTurno {
    /// Turno gravado em `turns`.
    pub turno: TurnId,
    /// Como terminou.
    pub estado: TurnState,
    /// Texto final da resposta (vazio quando não houve).
    pub texto: String,
    /// Mensagens do cliente que este turno atendeu (para concluir os itens da `inbox`).
    pub mensagens: Vec<MessageId>,
    /// Tokens de entrada somados.
    pub tokens_entrada: u32,
    /// Tokens de saída somados.
    pub tokens_saida: u32,
    /// Tempos por etapa.
    pub tempos: Tempos,
    /// Modelo usado.
    pub modelo: String,
    /// Motivo do fim informado pelo provedor.
    pub motivo_fim: Option<String>,
    /// Erro, quando não terminou bem.
    pub erro: Option<String>,
    /// `true` quando o turno não foi atendido por falta de capacidade ou por falha do modelo
    /// (provedor fora do ar, conexão, tempo máximo, falta de vaga): o item da `inbox` é adiado e
    /// tentado de novo, até o limite de tentativas (HR-26i e HR-26k).
    pub falha_modelo: bool,
    /// Decisões do Jev neste turno (HR-28). `None` quando o Jev está desligado ou não se aplica.
    pub jev: Option<ResultadoJev>,
}

impl ResumoTurno {
    /// `true` quando o turno respondeu alguma coisa.
    pub fn respondeu(&self) -> bool {
        self.estado == TurnState::Completed && !self.texto.trim().is_empty()
    }
}

/// O que o turno leva para a conversa com o modelo: as mensagens montadas e o coletor de uso.
struct Preparo {
    /// Mensagens prontas (prefixo estável, bloco volátil e histórico).
    mensagens: Vec<Message>,
    /// Coletor do que as ferramentas usarem (vai para o registro do turno).
    uso: Arc<UsoFerramentas>,
    /// Coletor das chamadas de ferramenta (vai para `turns.tools_json`, HR-47b).
    chamadas: Arc<RegistroChamadas>,
    /// `true` quando o G3 classificou a mensagem como `nada`: base e catálogo ficam de fora (HR-31).
    sem_base_produto: bool,
    /// `true` quando o G5 já escolheu o produto do turno: `buscar_produto` não é oferecida (HR-33).
    sem_buscar_produto: bool,
    /// Há execução de fluxo ativa na Ficha: ação e abandono valem neste turno (FX-24).
    fluxo_em_andamento: bool,
    /// O catálogo de fluxos foi ao prompt (reserva do Jev): `fluxo_iniciar` é oferecida (FX-24).
    fluxo_reserva: bool,
}

impl Preparo {
    /// Tira da lista oferecida as ferramentas do fluxo que não valem neste turno (FX-24).
    fn retain_ferramentas_do_fluxo(&self, oferecidas: &mut Vec<poph_core::llm::Tool>) {
        oferecidas.retain(|ferramenta| match ferramenta.nome.as_str() {
            fluxo_ferramentas::NOME_INICIAR => self.fluxo_reserva,
            fluxo_ferramentas::NOME_EXECUTAR_ACAO | fluxo_ferramentas::NOME_ABANDONAR => {
                self.fluxo_em_andamento
            }
            _ => true,
        });
    }
}

/// O que o G5 precisa do turno para procurar os candidatos e perguntar (HR-33).
struct ContextoG5<'a> {
    /// Org dona do turno.
    org: OrgId,
    /// Conversa (o G5 manda o mesmo estado dos outros gates).
    conversa: &'a poph_store::Conversa,
    /// Configuração quente do agente.
    quente: &'a AgenteQuente,
    /// Mensagens recentes da conversa.
    ultimas: &'a [MensagemGravada],
    /// Mensagens do grupo que abriu o turno (a consulta da busca).
    grupo: &'a [MensagemGravada],
    /// O G3 classificou a mensagem como `nada` (o catálogo sai do turno).
    g3_nada: bool,
}

/// O que o G6 precisa do turno para revisar a resposta antes do envio (HR-34).
struct ContextoG6<'a> {
    /// Org dona do turno.
    org: OrgId,
    /// Configuração quente do agente.
    quente: &'a AgenteQuente,
    /// Mensagens recentes da conversa (o mesmo estado dos outros gates).
    ultimas: &'a [MensagemGravada],
    /// Conversa (canal e id).
    conversa: &'a poph_store::Conversa,
    /// Conversa final enviada ao modelo, como base da regeneração.
    mensagens: &'a [Message],
    /// Produtos que o turno consultou: a evidência de preço da revisão.
    produtos: Vec<ProdutoUsado>,
    /// Modelo do turno (a regeneração usa o mesmo).
    modelo: String,
    /// Cancelamento do turno.
    cancelamento: Cancellation,
}

/// O que a revisão do G6 fez com a resposta (vai para o `turns.jev_json`).
#[derive(Debug, Clone, Default)]
struct RevisaoG6 {
    /// Escolha da primeira revisão (`aprovada` ou um `revisar_*`), quando houve.
    escolha: Option<String>,
    /// Confiança declarada na primeira revisão.
    confianca: f64,
    /// Escolha da revisão da resposta regenerada, quando houve.
    segunda: Option<String>,
    /// `true` quando a resposta foi reescrita uma vez.
    regenerou: bool,
    /// `true` quando o texto enviado é a resposta segura do agente.
    segura: bool,
}

impl RevisaoG6 {
    /// `true` quando o gate chegou a julgar a resposta.
    fn houve(&self) -> bool {
        self.escolha.is_some()
    }

    /// O bloco que entra em `turns.jev_json`.
    fn para_json(&self) -> serde_json::Value {
        json!({
            "gate": IdGate::G6.como_texto(),
            "escolha": self.escolha,
            "confianca": self.confianca,
            "regenerou": self.regenerou,
            "segunda_escolha": self.segunda,
            "resposta_segura": self.segura,
        })
    }
}

/// Onde o turno acontece: org, conversa e turno aberto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Alvo {
    org: OrgId,
    conversa: ConversationId,
    turno: TurnId,
}

// ───────────────────────────── Motor ─────────────────────────────

/// Motor do turno. É `Clone` barato (compartilha o estado), então pode ser usado por vários atores.
#[derive(Clone)]
pub struct Motor {
    store: Arc<Store>,
    config: Arc<ConfigQuente>,
    limitador: Arc<Limitador>,
    provedor: Arc<dyn LlmProvider>,
    provedores: Option<Arc<Provedores>>,
    ferramentas: Arc<dyn RegistroFerramentas>,
    /// Busca na base de conhecimento (HR-17). Sem ela, o turno roda sem RAG.
    busca: Option<Arc<dyn BuscadorBase>>,
    /// Busca no catálogo de produtos (HR-33): com ela o G5 pré-consulta os candidatos do turno.
    produtos: Option<Arc<dyn BuscadorProdutos>>,
    opcoes: OpcoesTurno,
    metricas: MetricasTurno,
    pausa: Arc<dyn Pausa>,
    /// Modelo usado quando o agente não define um.
    modelo_padrao: String,
    /// Cancelamento do processo (desligamento ordenado, teste): vale para qualquer turno em voo.
    cancelamento: Option<Cancellation>,
    /// Resumo rolante das conversas (HR-14), gerado em segundo plano depois de cada turno.
    resumidor: Arc<Resumidor>,
    /// Jev (HR-28): decide os portões antes da primeira chamada ao modelo. Sem ele, o turno roda
    /// exatamente como antes.
    jev: Option<Arc<dyn AvaliadorGates>>,
    /// Transferência para humano (HR-29): textos da orientação, mensagem automática e limiares.
    handoff: Orientacao,
    /// Arquivos do cliente (HR-23b): pasta dos blobs e limites. Sem ela, o turno não lê imagem
    /// nenhuma do disco — o texto extraído continua indo ao modelo pelo banco.
    arquivos: Option<poph_files::ConfigArquivos>,
    /// Motor de fluxos (FX-23). Sem ele, o turno não toca em fluxo nenhum: `[fluxos] ativo = false`
    /// ou composição que não ligou o módulo.
    fluxos: Option<AjustesDoFluxo>,
    /// Contadores do G9 (FX-23). Desacoplados do catálogo do servidor (ver [`MetricasG9`]).
    fluxos_g9: MetricasG9,
}

impl Motor {
    /// Cria o motor com o registro de ferramentas de produção (vazio até a fase 2) e espera real.
    pub fn novo(
        store: Arc<Store>,
        config: ConfigQuente,
        limitador: Arc<Limitador>,
        provedor: Arc<dyn LlmProvider>,
        opcoes: OpcoesTurno,
        metricas: MetricasTurno,
    ) -> Self {
        let resumidor = Arc::new(Resumidor::novo(
            Arc::clone(&store),
            Arc::clone(&provedor),
            Arc::clone(&limitador),
            OpcoesResumo::default(),
        ));
        Self {
            store,
            config: Arc::new(config),
            limitador,
            provedor,
            provedores: None,
            ferramentas: Arc::new(SemFerramentas),
            busca: None,
            produtos: None,
            opcoes,
            metricas,
            pausa: Arc::new(PausaReal),
            modelo_padrao: LlmConfig::default().model,
            cancelamento: None,
            resumidor,
            jev: None,
            handoff: Orientacao::default(),
            arquivos: None,
            fluxos: None,
            fluxos_g9: MetricasG9::proprias(),
        }
    }

    /// Troca os ajustes do resumo rolante (desligar, mudar prazos; testes e medições).
    pub fn com_resumo(mut self, opcoes: OpcoesResumo) -> Self {
        self.resumidor = Arc::new(Resumidor::novo(
            Arc::clone(&self.store),
            Arc::clone(&self.provedor),
            Arc::clone(&self.limitador),
            opcoes,
        ));
        self
    }

    /// Espera até não haver resumo sendo gerado (teste e desligamento). `false` se o prazo estourar.
    pub async fn aguardar_resumos(&self, limite: Duration) -> bool {
        self.resumidor.aguardar_ocioso(limite).await
    }

    /// Liga um cancelamento que vale para todos os turnos (Ctrl-C do processo, desligamento).
    pub fn com_cancelamento(mut self, cancelamento: Cancellation) -> Self {
        self.cancelamento = Some(cancelamento);
        self
    }

    /// Troca o modelo padrão (vem de `[llm].modelo`; o agente pode sobrescrever).
    pub fn com_modelo_padrao(mut self, modelo: impl Into<String>) -> Self {
        let modelo = modelo.into();
        if !modelo.trim().is_empty() {
            self.modelo_padrao = modelo;
        }
        self
    }

    pub fn com_provedores(mut self, provedores: Arc<Provedores>) -> Self {
        self.provedores = Some(provedores);
        self
    }

    /// Liga o Jev (HR-28). Sem ele, o turno não consulta portão nenhum.
    pub fn com_jev(mut self, avaliador: Arc<dyn AvaliadorGates>) -> Self {
        self.jev = Some(avaliador);
        self
    }

    /// Define a orientação ao cliente antes da transferência (HR-29): textos aprovados, mensagem
    /// automática, limiar e prazo da segunda pergunta. Sem ela, valem os padrões da configuração.
    pub fn com_handoff(mut self, orientacao: Orientacao) -> Self {
        self.handoff = orientacao;
        self
    }

    /// Troca o registro de ferramentas (HR-17/HR-18/HR-19/HR-24; testes).
    pub fn com_ferramentas(mut self, registro: Arc<dyn RegistroFerramentas>) -> Self {
        self.ferramentas = registro;
        self
    }

    /// Liga a busca na base de conhecimento (HR-17). Sem ela, o turno não consulta RAG.
    pub fn com_busca(mut self, busca: Arc<dyn BuscadorBase>) -> Self {
        self.busca = Some(busca);
        self
    }

    /// Liga a busca no catálogo de produtos (HR-33). Sem ela, o G5 não tem candidatos e não pergunta.
    pub fn com_produtos(mut self, produtos: Arc<dyn BuscadorProdutos>) -> Self {
        self.produtos = Some(produtos);
        self
    }

    /// Diz onde estão os blobs dos arquivos do cliente (HR-23b). Sem isso, o turno só usa o texto
    /// extraído; imagem fica gravada e não chega ao modelo.
    pub fn com_arquivos(mut self, arquivos: poph_files::ConfigArquivos) -> Self {
        self.arquivos = Some(arquivos);
        self
    }

    /// Liga o motor de fluxos com os ajustes de `[fluxos]` (FX-23).
    ///
    /// Sem esta chamada, ou com `[fluxos] ativo = false`, o turno não lê fluxo, não pergunta o G9 e
    /// não põe bloco no prompt — a Ficha continua sendo criada e fechada pelo `poph-store`.
    pub fn com_fluxos(mut self, config: &poph_types::FlowsConfig) -> Self {
        self.fluxos = Some(AjustesDoFluxo::da_config(config));
        self
    }

    /// Compartilha o cache de revisões de quem já tem um (o servidor tem o da API de fluxos).
    pub fn com_cache_de_fluxos(mut self, cache: Arc<poph_fluxos::cache::CacheRevisoes>) -> Self {
        let Some(ajustes) = self.fluxos.take() else {
            return self;
        };
        self.fluxos = Some(ajustes.com_cache(cache));
        self
    }

    /// Troca os contadores do G9 (o servidor passa os que publica em `/metrics`).
    pub fn com_metricas_g9(mut self, metricas: MetricasG9) -> Self {
        self.fluxos_g9 = metricas;
        self
    }

    /// Troca a espera (testes provam a janela de agrupamento sem dormir).
    pub fn com_pausa(mut self, pausa: Arc<dyn Pausa>) -> Self {
        self.pausa = pausa;
        self
    }

    /// Config quente em uso (a ferramenta `consultar_base` recebe a mesma).
    pub fn config_quente(&self) -> &Arc<ConfigQuente> {
        &self.config
    }

    /// Store por trás do motor (a CLI precisa dele para o resto).
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Métricas do turno.
    pub fn metricas(&self) -> &MetricasTurno {
        &self.metricas
    }

    /// Ajustes em uso.
    pub fn opcoes(&self) -> &OpcoesTurno {
        &self.opcoes
    }

    /// Executa um turno inteiro e devolve o que foi gravado.
    ///
    /// `Err` é só para falha de armazenamento ou de estado que impede até abrir o turno. Agente
    /// inativo, conversa transferida, mensagem já respondida e erro do provedor terminam como
    /// [`ResumoTurno`] com `estado` e `erro` preenchidos — o chamador precisa concluir a entrada.
    pub async fn executar(
        &self,
        entrada: Entrada,
        observador: &mut dyn Observador,
    ) -> Resultado<ResumoTurno> {
        let inicio = Instant::now();
        let org = entrada.org;
        let conversa_id = entrada.conversa;
        let cancelamento = entrada.cancelamento.clone();

        let conversa = self.store.conversations().get(org, conversa_id)?;
        let agente_id = conversa.agent_id;
        if self
            .cancelamento
            .as_ref()
            .is_some_and(Cancellation::cancelado)
        {
            cancelamento.cancelar();
        }

        // D-05: agente inativo não abre turno. A mensagem continua gravada, o turno fica `skipped`.
        let quente = match self.config.carregar_verificando(org, agente_id) {
            Ok(quente) => quente,
            Err(ErroAgente::Dominio(DomainError::AgentInactive { status, .. })) => {
                let motivo =
                    format!("agente inativo ({status}): mensagem registrada e não respondida");
                return self.encerrar_ignorado(org, conversa_id, Some(agente_id), motivo, inicio);
            }
            Err(erro) => return Err(erro),
        };

        // D12: conversa transferida ou fechada não é respondida até ser reaberta.
        if conversa.status != poph_store::ConversationStatus::Open {
            let motivo = format!(
                "conversa {} ({}): o agente não responde até a conversa ser reaberta",
                conversa_id,
                conversa.status.as_text()
            );
            return self.encerrar_ignorado(org, conversa_id, Some(agente_id), motivo, inicio);
        }

        // HR-26h: conversa de canal (WhatsApp) só é respondida com a integração ativa. Inativa ou
        // arquivada, a mensagem fica gravada e o turno termina sem resposta, com o motivo.
        if let Some(integracao) = conversa.integration_id {
            let situacao = self.store.channel_integration(org, integracao)?;
            let ativa = situacao
                .as_ref()
                .is_some_and(|canal| canal.status == "active");
            if !ativa {
                let motivo = match situacao {
                    Some(canal) => format!(
                        "integração do canal ({}): mensagem registrada e não respondida",
                        canal.status
                    ),
                    None => {
                        "integração do canal não encontrada: mensagem registrada e não respondida"
                            .to_owned()
                    }
                };
                return self.encerrar_ignorado(org, conversa_id, Some(agente_id), motivo, inicio);
            }
        }

        let gatilho = match self
            .store
            .messages()
            .get(org, conversa_id, entrada.mensagem)
        {
            Ok(mensagem) => mensagem,
            Err(erro) if erro.is_not_found() => {
                let motivo = format!(
                    "mensagem {} não existe na conversa {}",
                    entrada.mensagem, conversa_id
                );
                return self.encerrar_ignorado(org, conversa_id, Some(agente_id), motivo, inicio);
            }
            Err(erro) => return Err(erro.into()),
        };

        // Janela de agrupamento: conta a partir da chegada da mensagem que disparou o turno.
        let janela = self.janela_do_agente(&quente.agente);
        let decorrido = || {
            Duration::from_millis(
                u64::try_from(self.store.now().saturating_sub(gatilho.created_at)).unwrap_or(0),
            )
        };

        // FX-23: o fluxo do atendimento é lido **antes** do lote do Jev, porque o G9 precisa dos
        // fluxos liberados no estado que vai junto das outras perguntas. Motor desligado, conversa
        // sem Ficha ou leitura que falhou devolvem `None`, e o turno segue sem fluxo.
        let fluxo_preparo = self
            .fluxos
            .as_ref()
            .and_then(|ajustes| fluxo::preparar(ajustes, &self.store, org, conversa_id, agente_id));
        // Só os fluxos de uma Ficha **sem** execução ativa entram no estado do G9 (PRD L6): com
        // execução em andamento, a pergunta não existe e o custo é zero.
        let fluxos_do_g9: Vec<poph_jev::FluxoJev> = fluxo_preparo
            .as_ref()
            .filter(|preparo| preparo.ativa.is_none())
            .map(|preparo| preparo.liberados.clone())
            .unwrap_or_default();
        let fluxos_do_g9 = (!fluxos_do_g9.is_empty()).then_some(fluxos_do_g9);
        let fluxos_do_g9 = fluxos_do_g9.as_deref();

        // HR-30: o G2 governa a janela, então a decisão dele precisa existir **antes** da espera.
        // Com o G2 ligado, o lote do Jev sai na chegada da mensagem e o tempo dele cabe dentro da
        // janela; o mesmo lote é reaproveitado pelos outros gates depois (G1/G3/G4) e só é refeito
        // se uma mensagem nova do cliente chegar durante a espera — aí o estado mudou.
        let mut jev_na_chegada: Option<ResultadoJev> = None;
        let mut marco_do_lote: Option<MessageId> = None;
        let mut espera = janela.saturating_sub(decorrido());
        if self.g2_ligado(&quente.agente) {
            let na_chegada = self.ultimas_mensagens(org, conversa_id)?;
            marco_do_lote = ultima_entrada_do_cliente(&na_chegada);
            if let Some(futuro) =
                self.iniciar_jev(org, &quente, &na_chegada, &conversa, fluxos_do_g9)
            {
                jev_na_chegada = Some(futuro.await);
            }
            let decisao = jev_na_chegada
                .as_ref()
                .and_then(|resultado| resultado.escolha_confiavel(IdGate::G2));
            let ajustada = janela_com_g2(
                janela,
                self.opcoes.janela_maxima,
                self.opcoes.janela_confirmacao,
                decisao,
            );
            match decisao {
                Some(poph_jev::G2_ESPERAR) if ajustada > janela => {
                    self.metricas.janelas_esticadas.increment(1);
                }
                Some(poph_jev::G2_PROCESSAR) if ajustada < janela => {
                    self.metricas.janelas_diretas.increment(1);
                }
                _ => {}
            }
            espera = ajustada.saturating_sub(decorrido());
        }
        self.pausa.esperar(espera).await;
        self.metricas
            .espera_agrupamento_ms
            .observe(espera.as_secs_f64() * 1000.0);

        // Depois da janela, agrupa as mensagens seguidas do cliente que ainda não foram respondidas.
        let ultimas = self.ultimas_mensagens(org, conversa_id)?;
        // O lote feito na chegada só vale para o resto do turno se o cliente não mandou nada novo
        // durante a espera: com mensagem nova, o estado que o Jev viu está velho.
        // As mensagens que este turno responde (o grupo) são conhecidas antes do lote: é delas que
        // sai o texto de arquivo do G7 (HR-35), que precisa entrar no estado enviado ao Jev.
        let (situacao, grupo) = analisar(&ultimas, entrada.mensagem);
        // HR-36: o G8 só encerra quando o turno tem **uma** mensagem do cliente. Com duas (uma dúvida
        // e um "obrigado, era só isso" na mesma janela), o Jev julga a última e a dúvida ficaria sem
        // resposta: nesse caso a conversa segue.
        let encerramento_possivel = grupo.len() == 1;
        let fila_ms = gatilho_fila(self.store.now(), gatilho.created_at);
        match situacao {
            Situacao::SemNovidade => {
                let motivo = "nenhuma mensagem nova para responder".to_owned();
                return self.encerrar_ignorado(org, conversa_id, Some(agente_id), motivo, inicio);
            }
            Situacao::JaRespondida => {
                let motivo = "mensagem já respondida por um turno anterior (reentrega ou retomada)"
                    .to_owned();
                return self.encerrar_ignorado(org, conversa_id, Some(agente_id), motivo, inicio);
            }
            Situacao::Processar => {}
        }
        let grupo_ids: Vec<MessageId> = grupo.iter().map(|mensagem| mensagem.id).collect();
        let jev_pronto =
            jev_na_chegada.filter(|_| ultima_entrada_do_cliente(&ultimas) == marco_do_lote);

        let turno = self
            .store
            .turns()
            .start(org, conversa_id, Some(agente_id))?;
        self.metricas.turnos_iniciados.increment(1);
        observador.evento(EventoTurno::Inicio {
            turno: turno.id,
            agente: agente_id,
            conversa: conversa_id,
        });

        // Vaga de turno (org/agente/instância). Sem vaga na espera, o turno falha e a entrada
        // continua pendente para a retomada; não é perda de mensagem.
        let vaga = match self
            .limitador
            .adquirir_turno(org, agente_id, self.opcoes.espera_vaga)
            .await
        {
            Ok(vaga) => vaga,
            Err(erro) => {
                return self.falhar(
                    org,
                    turno.id,
                    &quente.agente.modelo,
                    grupo_ids,
                    inicio,
                    fila_ms,
                    0,
                    None,
                    format!("sem vaga para o turno: {erro}"),
                    ClasseFalha::SemVaga,
                    None,
                    observador,
                );
            }
        };

        // Montagem do prompt: prefixo estável + bloco volátil + janela de contexto.
        let montagem_inicio = Instant::now();
        // A busca na base começa aqui e só é aguardada depois de o resto do prompt estar montado.
        // É o ponto em que o Jev (HR-28) entra em paralelo: as duas respostas são juntadas antes da
        // primeira chamada ao modelo, nunca dentro do prefixo estável.
        let busca_futuro = self.iniciar_busca(org, agente_id, &grupo, &quente);
        // O Jev entra no mesmo ponto, e as duas chamadas correm juntas: o lote de perguntas não
        // atrasa a resposta além do que a busca já atrasaria (PRD §9: busca e Jev concorrentes).
        // Com o G2 ligado, o lote já foi feito na chegada da mensagem (HR-30) e é este o momento de
        // reaproveitá-lo: só se a conversa mudou na espera é que ele é refeito.
        let jev_futuro = if jev_pronto.is_some() {
            None
        } else {
            self.iniciar_jev(org, &quente, &ultimas, &conversa, fluxos_do_g9)
        };
        let limite = quente
            .agente
            .max_context_messages
            .clamp(1, self.opcoes.teto_leitura_mensagens.max(1));
        // Janela de contexto com o resumo rolante (HR-14): o resumo ocupa parte do orçamento.
        let resumo_da_conversa = resumo::Resumo::decodificar(conversa.summary.as_deref());
        let montagem_historico = resumo::montar(
            &ultimas,
            &resumo_da_conversa,
            u64::from(quente.agente.max_context_tokens),
            limite,
        );
        // Janela de contexto com o que veio dos arquivos de cada mensagem do cliente:
        // - **texto extraído** (HR-35): o bloco vai **depois** da mensagem que trouxe o arquivo, com
        //   o aviso nosso (confiável) e o conteúdo entre as marcas do bloco não confiável, como a
        //   extração já o entregou (poph-files). Sem isso, "ignore as instruções anteriores" escrito
        //   num PDF chegaria ao modelo com a mesma voz do cliente.
        // - **imagem** (HR-23b): vai como parte multimodal, no mesmo lugar (depois da mensagem), lida
        //   do blob pelo hash. O aviso de conteúdo não confiável é textual e não se aplica à imagem,
        //   que o modelo vê como dado do cliente.
        let mut historico: Vec<Message> = Vec::new();
        let mut imagens = OrcamentoImagens::default();
        for mensagem in &montagem_historico.mensagens {
            let partes = if mensagem.role == Role::User {
                self.partes_de_arquivos(org, mensagem, &mut imagens)
            } else {
                Vec::new()
            };
            // Mensagem do cliente só com arquivo (texto vazio): com a parte do arquivo, a mensagem
            // de texto vazio não entra — o pedido teria conteúdo e um texto em branco ao lado.
            let texto_vazio = mensagem.role == Role::User && mensagem.texto.trim().is_empty();
            if let Some(item) = prompt::da_mensagem_gravada(mensagem)
                && !(texto_vazio && !partes.is_empty())
            {
                historico.push(item);
            }
            if !partes.is_empty() {
                historico.push(Message::Usuario(partes));
            }
        }
        let (resultado_busca, jev_pos_janela) = match (busca_futuro, jev_futuro) {
            (Some(busca), Some(jev)) => {
                // Os dois correm juntos. Quem chega primeiro não espera o outro: se o Jev volta
                // antes e classifica a mensagem como `nada` (HR-31), a busca em andamento é
                // abandonada — o turno não paga o tempo dela. Nos outros casos a busca é aguardada
                // como antes, e o custo continua sendo o maior dos dois, não a soma.
                match futures::future::select(busca, jev).await {
                    futures::future::Either::Left((resultado, jev_pendente)) => {
                        (Some(resultado), Some(jev_pendente.await))
                    }
                    futures::future::Either::Right((jev, busca_pendente)) => {
                        // HR-36: o G8 encerrado também dispensa a busca: a resposta não sai do modelo.
                        let dispensa_busca = jev.escolha_confiavel(IdGate::G3) == Some(G3_NADA)
                            || (encerramento_possivel
                                && jev.escolha_confiavel(IdGate::G8) == Some(G8_ENCERRADA));
                        if dispensa_busca {
                            (None, Some(jev))
                        } else {
                            (Some(busca_pendente.await), Some(jev))
                        }
                    }
                }
            }
            (Some(busca), None) => (Some(busca.await), None),
            (None, Some(jev)) => (None, Some(jev.await)),
            (None, None) => (None, None),
        };
        let mut jev = jev_pronto.or(jev_pos_janela);
        // HR-31: efeito do G3. Classificada a última mensagem como `nada` (saudação, agradecimento,
        // despedida), a base e o catálogo não servem para responder: os trechos em andamento são
        // descartados e as ferramentas de base/produto saem da lista deste turno. Abstenção, gate
        // desligado ou erro do Jev = `tudo`, isto é, o comportamento de sempre.
        let g3_nada = jev
            .as_ref()
            .and_then(|resultado| resultado.escolha_confiavel(IdGate::G3))
            == Some(G3_NADA);
        let mut trechos = match resultado_busca {
            Some(Ok(trechos)) => trechos,
            Some(Err(erro)) => {
                // Base fora do ar não derruba a conversa: o turno segue sem os trechos.
                tracing::warn!(erro = %erro, "busca na base falhou; turno segue sem trechos");
                self.metricas.busca_falhas.increment(1);
                Vec::new()
            }
            None => Vec::new(),
        };
        if g3_nada {
            if !trechos.is_empty() {
                tracing::debug!(
                    descartados = trechos.len(),
                    "G3 classificou a mensagem como nada; trechos descartados"
                );
            }
            trechos.clear();
            self.metricas.buscas_puladas_g3.increment(1);
        }
        if let Some(jev) = &jev
            && (jev.falhou() || !jev.decisoes.is_empty())
        {
            tracing::debug!(
                falha = ?jev.chamada.falha,
                decisoes = ?jev.resumo(),
                "Jev do turno"
            );
        }

        // HR-29: o G1 tem efeito. Decidida a transferência, o turno termina aqui: o cliente recebe a
        // orientação (quando há risco) e a mensagem automática, a conversa vira `transferred` e o
        // **modelo de texto não é chamado** — é o que faz o agente parar de responder até
        // `poph conversation reopen` (D12).
        if jev
            .as_ref()
            .and_then(|resultado| resultado.escolha_confiavel(IdGate::G1))
            == Some(G1_TRANSFERIR)
        {
            let resultado_jev = jev.expect("a transferência só existe com o resultado do Jev");
            return self
                .transferir(
                    Alvo {
                        org,
                        conversa: conversa_id,
                        turno: turno.id,
                    },
                    &quente,
                    &ultimas,
                    &conversa,
                    &resultado_jev,
                    IdGate::G1,
                    grupo_ids,
                    (
                        fila_ms,
                        u64::try_from(montagem_inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
                    ),
                    inicio,
                    observador,
                )
                .await;
        }
        // HR-35: efeito do G7. Classificada a última mensagem (ou o texto do arquivo) como
        // injeção ou abuso, vale a ação configurada pelo agente: `registrar` só anota e o turno
        // segue como hoje; `responder_padrao` manda o texto neutro **sem chamar o modelo**;
        // `transferir` usa o caminho do G1. Abstenção, confiança baixa, resposta inválida e Jev
        // fora = `normal`, isto é, o comportamento de sempre.
        let acao_g7 = poph_jev::acao_g7_do_agente(&quente.agente.jev_json);
        if jev
            .as_ref()
            .and_then(|resultado| resultado.escolha_confiavel(IdGate::G7))
            .is_some_and(poph_jev::g7_pede_acao)
        {
            let resultado_jev = jev.as_ref().expect("o G7 só decide com o resultado do Jev");
            self.metricas.g7_detectados.increment(1);
            tracing::warn!(
                conversa = %conversa_id,
                classificacao = jev
                    .as_ref()
                    .and_then(|resultado| resultado.escolha_confiavel(IdGate::G7))
                    .unwrap_or("—"),
                acao = acao_g7.como_texto(),
                "G7: mensagem classificada como injeção ou abuso"
            );
            match acao_g7 {
                AcaoG7::Registrar => {}
                AcaoG7::ResponderPadrao => {
                    return self.responder_g7(
                        Alvo {
                            org,
                            conversa: conversa_id,
                            turno: turno.id,
                        },
                        &quente,
                        resultado_jev,
                        grupo_ids,
                        (
                            fila_ms,
                            u64::try_from(montagem_inicio.elapsed().as_millis())
                                .unwrap_or(u64::MAX),
                        ),
                        inicio,
                        observador,
                    );
                }
                AcaoG7::Transferir => {
                    self.metricas.g7_transferencias.increment(1);
                    return self
                        .transferir(
                            Alvo {
                                org,
                                conversa: conversa_id,
                                turno: turno.id,
                            },
                            &quente,
                            &ultimas,
                            &conversa,
                            resultado_jev,
                            IdGate::G7,
                            grupo_ids,
                            (
                                fila_ms,
                                u64::try_from(montagem_inicio.elapsed().as_millis())
                                    .unwrap_or(u64::MAX),
                            ),
                            inicio,
                            observador,
                        )
                        .await;
                }
            }
        }
        // HR-36: efeito do G8. Decidido que o cliente encerrou a conversa, o turno termina aqui: a
        // despedida configurada (ou o padrão) sai e o **modelo de texto não é chamado**. Nada muda
        // no estado da conversa: a próxima mensagem do cliente entra num turno normal. Abstenção,
        // confiança baixa, resposta inválida e Jev fora = `continua`, isto é, o turno de sempre.
        if encerramento_possivel
            && jev
                .as_ref()
                .and_then(|resultado| resultado.escolha_confiavel(IdGate::G8))
                == Some(G8_ENCERRADA)
        {
            let resultado_jev = jev.expect("o encerramento só existe com o resultado do Jev");
            return self.encerrar_g8(
                Alvo {
                    org,
                    conversa: conversa_id,
                    turno: turno.id,
                },
                &quente,
                &resultado_jev,
                grupo_ids,
                (
                    fila_ms,
                    u64::try_from(montagem_inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
                ),
                inicio,
                observador,
            );
        }
        // HR-33: efeito do G5. Fora do lote de propósito — os candidatos são o resultado da busca
        // de produtos do turno, que só existe depois do estado inicial. O código valida o id
        // devolvido contra os candidatos **deste** turno; abstenção, confiança baixa, resposta
        // inválida e Jev fora do ar mantêm o comportamento de hoje.
        let produto_g5 = self
            .produto_do_g5(
                ContextoG5 {
                    org,
                    conversa: &conversa,
                    quente: &quente,
                    ultimas: &ultimas,
                    grupo: &grupo,
                    g3_nada,
                },
                &mut jev,
            )
            .await;
        if let Some(jev) = &jev
            && jev.decisao(IdGate::G5).is_some()
        {
            tracing::debug!(
                decisao = ?jev.decisao(IdGate::G5).map(|decisao| decisao.para_json()),
                "G5 do turno"
            );
        }

        // FX-23: o fluxo do atendimento, decidido **antes** da chamada do modelo. Com execução
        // ativa, o bloco da etapa entra no prompt; sem ela, a escolha confiável do G9 é validada e
        // a execução é aberta aqui; sem escolha, a reserva (catálogo) ou nada, conforme o motivo.
        // O G9 estar ligado para este agente separa "não foi perguntado" de "a chamada falhou" no
        // registro e nas métricas.
        let g9_ligado = self.jev.is_some() && self.gates_do_agente(&quente.agente).g9;
        let fluxo_do_turno = match self.fluxos.as_ref() {
            Some(ajustes) => fluxo::decidir(
                ajustes,
                &self.store,
                org,
                fluxo_preparo.as_ref(),
                jev.as_ref(),
                g9_ligado,
                // A instrução do catálogo só manda chamar a ferramenta quando ela existe **e**
                // está habilitada no agente (D-04): instruir o modelo a chamar o que não é
                // oferecido só produz recusa no laço.
                self.ferramentas
                    .ferramenta(fluxo::NOME_FERRAMENTA_INICIAR)
                    .is_some()
                    && quente.ferramenta_habilitada(fluxo::NOME_FERRAMENTA_INICIAR),
            ),
            None => Default::default(),
        };
        self.contar_g9(&fluxo_do_turno);

        if !trechos.is_empty() {
            self.metricas
                .trechos_recuperados
                .increment(trechos.len() as u64);
            observador.evento(EventoTurno::Base {
                fontes: trechos
                    .iter()
                    .map(|trecho| format!("{} ({})", trecho.titulo, trecho.grupo))
                    .collect(),
            });
        }
        let bloco = prompt::BlocoVolatil {
            agora_ms: self.store.now(),
            canal: &conversa.canal,
            status: status_do_atendimento(conversa.status),
            conversa: &conversa.id.to_string(),
            resumo: montagem_historico.resumo.as_deref(),
            trechos: &trechos,
            catalogo_de_produtos: !g3_nada
                && quente.ferramenta_habilitada(crate::produtos::NOME_FERRAMENTA),
            produto_escolhido: produto_g5.as_ref(),
            fluxos: fluxo_do_turno.texto.as_deref(),
        }
        .montar();
        let mensagens = Prompt::novo(quente.markdown.clone())
            .com_volatil(bloco)
            .mensagens(historico);
        let montagem_ms = u64::try_from(montagem_inicio.elapsed().as_millis()).unwrap_or(u64::MAX);

        // O que as ferramentas usarem neste turno (produtos, por enquanto) vai para o registro.
        // O produto escolhido pelo G5 entra aqui como usado: ele veio do catálogo neste turno.
        let uso = UsoFerramentas::nova();
        // HR-47b: cada chamada de ferramenta (executada ou recusada) entra aqui, com o motivo.
        let chamadas = RegistroChamadas::nova();
        if let Some(produto) = &produto_g5 {
            uso.registrar_produto(ProdutoUsado {
                produto: produto.produto,
                nome: produto.nome.clone(),
                preco_centavos: produto.preco_centavos,
                preco_texto: produto.preco_texto.clone(),
                unidade: produto.unidade.clone(),
                condicoes: produto.condicoes.clone(),
            });
            self.metricas.produtos_escolhidos_g5.increment(1);
        }
        let preparo = Preparo {
            mensagens,
            uso: Arc::clone(&uso),
            chamadas: Arc::clone(&chamadas),
            // HR-31: com o G3 em `nada`, base e catálogo não são oferecidos neste turno.
            sem_base_produto: g3_nada,
            // HR-33: com o G5 escolhendo o produto, o catálogo já foi consultado neste turno.
            sem_buscar_produto: produto_g5.is_some(),
            // FX-24: cada ferramenta do fluxo é oferecida no momento em que faz sentido.
            fluxo_em_andamento: fluxo_do_turno.em_andamento,
            fluxo_reserva: fluxo_do_turno.reserva,
        };
        let conversar = self.conversar(
            Alvo {
                org,
                conversa: conversa_id,
                turno: turno.id,
            },
            &quente,
            preparo,
            &cancelamento,
            observador,
            inicio,
        );
        let (mut resposta, conversa_final) =
            match tokio::time::timeout(self.opcoes.tempo_maximo_turno, conversar).await {
                Ok(Ok(par)) => par,
                Ok(Err(erro)) => {
                    let texto = format!("erro do provedor: {erro}");
                    return self.falhar(
                        org,
                        turno.id,
                        &quente.agente.modelo,
                        grupo_ids,
                        inicio,
                        fila_ms,
                        montagem_ms,
                        None,
                        texto,
                        classe_do_erro_llm(&erro),
                        Some(&chamadas),
                        observador,
                    );
                }
                Err(_) => {
                    let texto = format!(
                        "tempo máximo do turno esgotado ({} s)",
                        self.opcoes.tempo_maximo_turno.as_secs()
                    );
                    return self.falhar(
                        org,
                        turno.id,
                        &quente.agente.modelo,
                        grupo_ids,
                        inicio,
                        fila_ms,
                        montagem_ms,
                        None,
                        texto,
                        ClasseFalha::Modelo,
                        Some(&chamadas),
                        observador,
                    );
                }
            };
        drop(vaga);

        // HR-34: revisão do G6 antes do envio. O Jev julga o texto do modelo; uma reprovação custa
        // **uma** regeneração com orientação fixa; se a resposta nova também for reprovada (ou a
        // segunda revisão não responder), sai a resposta segura do agente, quando configurada.
        // Gate desligado, abstenção, confiança baixa, id fora do contrato e Jev fora = o texto do
        // modelo, como antes: o gate nunca deixa o cliente sem resposta.
        let revisao = self
            .revisar_g6(
                ContextoG6 {
                    org,
                    quente: &quente,
                    ultimas: &ultimas,
                    conversa: &conversa,
                    mensagens: &conversa_final,
                    produtos: uso.produtos(),
                    modelo: self.modelo_usado(&quente),
                    cancelamento: cancelamento.clone(),
                },
                &mut resposta,
                &mut jev,
            )
            .await;

        let tempos = Tempos {
            fila_ms,
            montagem_ms,
            primeiro_texto_ms: resposta.primeiro_texto_ms,
            total_ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
            rodadas_ferramenta: resposta.rodadas,
        };

        // Resposta: mensagem do agente + item da `outbox` (idempotente por turno + seq) **na mesma
        // transação** (HR-57): queda entre as duas escritas não deixa a resposta no histórico sem
        // envio.
        let texto = resposta.texto.trim().to_owned();
        if !texto.is_empty() {
            self.store.messages().write_with_output(
                org,
                NewMessage {
                    conversation_id: conversa_id,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    texto: texto.clone(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: Some(turno.id),
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                NewOutboxItem {
                    conversation_id: conversa_id,
                    turn_id: turno.id,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": texto }).to_string(),
                },
            )?;
            self.metricas.respostas_gravadas.increment(1);
        }
        self.store
            .conversations()
            .touch(org, conversa_id, self.store.now())?;

        let modelo = self.modelo_usado(&quente);
        // `turns.jev_json` é o painel "por quê" do turno: as decisões do Jev **e** o que o código
        // fez com a escolha do G9 (iniciou, já havia execução, reserva). Sem texto de cliente.
        let jev_json = match (&jev, &fluxo_do_turno.registro) {
            (Some(item), _) if !item.decisoes.is_empty() => {
                let mut registro = item.para_json();
                if revisao.houve() {
                    registro["revisao"] = revisao.para_json();
                }
                if let Some(fluxo) = &fluxo_do_turno.registro {
                    registro["fluxo"] = fluxo.clone();
                }
                Some(registro.to_string())
            }
            (_, Some(fluxo)) => Some(json!({ "fluxo": fluxo }).to_string()),
            _ => None,
        };
        let fim = TurnFinalization {
            state: Some(TurnState::Completed),
            jev_json,
            times_json: Some(tempos_json(&tempos, &modelo, resposta.motivo.as_ref())),
            passages_json: Some(utilizados_json(&trechos, &uso.produtos())),
            // HR-47b: `tools_json` guarda os nomes (contrato antigo) **e** o detalhe por chamada.
            tools_json: Some(registro_do_turno(&resposta.ferramentas, &chamadas)),
            input_tokens: Some(u32::try_from(resposta.uso.entrada).unwrap_or(u32::MAX)),
            output_tokens: Some(u32::try_from(resposta.uso.saida).unwrap_or(u32::MAX)),
            ..TurnFinalization::default()
        };
        self.store.turns().finalise(org, turno.id, fim)?;
        self.metricas.turnos_concluidos.increment(1);
        self.metricas.tokens_entrada.increment(resposta.uso.entrada);
        self.metricas.tokens_saida.increment(resposta.uso.saida);
        self.metricas
            .duracao_turno_ms
            .observe(tempos.total_ms as f64);
        if let Some(primeiro) = tempos.primeiro_texto_ms {
            self.metricas.primeiro_texto_ms.observe(primeiro as f64);
        }
        observador.evento(EventoTurno::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: resposta.motivo.as_ref().map(motivo_texto),
        });

        // Resumo rolante: agendado depois da resposta, em segundo plano. O turno não espera por ele.
        self.resumidor.agendar(org, conversa_id, modelo.clone());

        Ok(ResumoTurno {
            turno: turno.id,
            estado: TurnState::Completed,
            texto,
            mensagens: grupo_ids,
            tokens_entrada: u32::try_from(resposta.uso.entrada).unwrap_or(u32::MAX),
            tokens_saida: u32::try_from(resposta.uso.saida).unwrap_or(u32::MAX),
            tempos,
            modelo,
            motivo_fim: resposta.motivo.as_ref().map(motivo_texto),
            erro: None,
            falha_modelo: false,
            jev,
        })
    }

    /// Uma chamada ao modelo (ou várias, quando há ferramenta), com streaming.
    async fn conversar(
        &self,
        alvo: Alvo,
        quente: &AgenteQuente,
        preparo: Preparo,
        cancelamento: &Cancellation,
        observador: &mut dyn Observador,
        inicio: Instant,
    ) -> Result<(Resposta, Vec<Message>), LlmError> {
        let max_rodadas = quente.agente.max_tool_rounds.max(1);
        let max_por_mensagem = quente.agente.max_tools_per_message.max(1);
        let mut oferecidas =
            ferramentas::oferecidas(self.ferramentas.as_ref(), &quente.ferramentas);
        // HR-31: o G3 em `nada` tira do turno as ferramentas que consultam base e catálogo. O
        // modelo não pode chamar o que não é oferecido (o laço de ferramentas só executa as
        // oferecidas), então não há resposta inventada de política nem de preço.
        if preparo.sem_base_produto {
            oferecidas.retain(|ferramenta| !eh_base_ou_produto(&ferramenta.nome));
        } else if preparo.sem_buscar_produto {
            // HR-33: o G5 já consultou o catálogo e escolheu o produto deste turno — a ferramenta
            // não é oferecida, e o turno economiza a rodada inteira.
            oferecidas.retain(|ferramenta| ferramenta.nome != crate::produtos::NOME_FERRAMENTA);
        }
        // FX-24: as ferramentas do fluxo aparecem no momento em que a conversa está.
        // `fluxo_iniciar` só na reserva (PRD §6.3-4: com o Jev respondendo, quem decide é o
        // código); ação e abandono só com execução ativa — oferecer `fluxo_executar_acao` sem
        // fluxo em andamento convidaria o modelo a chamar uma ferramenta que só pode recusar.
        preparo.retain_ferramentas_do_fluxo(&mut oferecidas);
        // Biblioteca de arquivos do agente (HR-24): a lista de arquivos entra na descrição da
        // ferramenta `enviar_arquivo`, que é onde o modelo decide o que pode enviar.
        crate::biblioteca::anexar_biblioteca(
            &mut oferecidas,
            &self.store,
            alvo.org,
            quente.agente.id,
        );
        let modelo = self.modelo_usado(quente);
        let provedor_do_turno = self.provedor_do_turno(quente)?;

        let uso = &preparo.uso;
        let chamadas_do_turno = &preparo.chamadas;
        let mut conversa = preparo.mensagens;
        let mut resposta = Resposta::default();
        for rodada in 0..=max_rodadas {
            // Na última passagem o modelo recebe o pedido sem ferramenta: é assim que o turno
            // termina em texto quando o limite de rodadas foi atingido.
            let lista = if rodada == max_rodadas {
                Vec::new()
            } else {
                oferecidas.clone()
            };
            let mut pedido = Request::new(alvo.org, conversa.clone());
            pedido.ferramentas = lista;
            pedido.modelo = Some(modelo.clone());
            pedido.cancelamento = cancelamento.clone();
            pedido.temperatura = quente.agente.temperatura.map(|valor| valor as f32);
            pedido.max_tokens = quente
                .agente
                .max_tokens_saida
                .and_then(|valor| u32::try_from(valor).ok());

            let vaga_provedor = self
                .limitador
                .adquirir_provedor(provedor_do_turno.nome(), self.opcoes.espera_vaga)
                .await
                .map_err(erro_de_vaga_do_provedor)?;

            let mut fluxo = provedor_do_turno.cliente().stream(pedido).await?;
            let mut texto_da_rodada = String::new();
            let mut chamadas: Vec<ToolCall> = Vec::new();
            while let Some(evento) = self.proximo_evento(&mut fluxo, cancelamento).await? {
                match evento? {
                    LlmEvent::Texto(pedaco) => {
                        if resposta.primeiro_texto_ms.is_none() {
                            resposta.primeiro_texto_ms = Some(
                                u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
                            );
                        }
                        texto_da_rodada.push_str(&pedaco);
                        observador.evento(EventoTurno::Texto(pedaco));
                    }
                    LlmEvent::Tool(chamada) => chamadas.push(chamada),
                    LlmEvent::Usage(uso) => resposta.uso = somar_uso(resposta.uso, uso),
                    LlmEvent::Fim(motivo) => resposta.motivo = Some(motivo),
                }
            }
            drop(vaga_provedor);
            resposta.texto.push_str(&texto_da_rodada);

            if chamadas.is_empty() {
                break;
            }
            resposta.rodadas += 1;
            conversa.push(Message::Assistente {
                texto: texto_da_rodada,
                chamadas: chamadas.clone(),
            });
            for (indice, chamada) in chamadas.iter().enumerate() {
                if indice >= max_por_mensagem as usize {
                    let conteudo = format!(
                        "erro: limite de {max_por_mensagem} chamadas de ferramenta por \
                         mensagem excedido"
                    );
                    // HR-47b: a recusa por teto também entra no registro, para o painel do por quê
                    // não tratar como chamada silenciosa.
                    chamadas_do_turno.registrar(
                        &chamada.nome,
                        rodada,
                        Desfecho::Recusada(MOTIVO_LIMITE_DE_CHAMADAS),
                        Duration::ZERO,
                        &chamada.argumentos,
                        &conteudo,
                    );
                    conversa.push(Message::Resultado {
                        id: chamada.id.clone(),
                        conteudo,
                    });
                    continue;
                }
                observador.evento(EventoTurno::Ferramenta {
                    nome: chamada.nome.clone(),
                    rodada,
                });
                resposta.ferramentas.push(chamada.nome.clone());
                let inicio_chamada = Instant::now();
                // HR-31: mesmo que o modelo invente a chamada, base e catálogo não são executados
                // no turno em que o G3 disse `nada` (a ferramenta nem foi oferecida).
                let desfecho = if preparo.sem_base_produto && eh_base_ou_produto(&chamada.nome) {
                    DesfechoFerramenta::recusada(
                        MOTIVO_FORA_DO_TURNO,
                        format!(
                            "erro: a ferramenta `{}` não faz parte deste turno (a mensagem não \
                             exige consulta)",
                            chamada.nome
                        ),
                    )
                } else if chamada.nome == fluxo_ferramentas::NOME_INICIAR && !preparo.fluxo_reserva
                {
                    // FX-24: `fluxo_iniciar` só existe na reserva. Fora dela (Jev escolheu, já há
                    // execução, `nenhum`), quem abre fluxo é o código — a ferramenta não é
                    // oferecida e a chamada inventada não é executada.
                    DesfechoFerramenta::recusada(
                        MOTIVO_FORA_DO_TURNO,
                        "erro: iniciar fluxo não é sua decisão neste turno; siga a orientação do bloco <fluxo_em_andamento>"
                            .to_owned(),
                    )
                } else if preparo.sem_buscar_produto
                    && chamada.nome == crate::produtos::NOME_FERRAMENTA
                {
                    // HR-33: evitando consulta repetida ao catálogo no mesmo turno.
                    DesfechoFerramenta::recusada(
                        MOTIVO_FORA_DO_TURNO,
                        "erro: o catálogo já foi consultado e o produto do cliente está no bloco \
                         <produtos_encontrados> deste turno; não chame buscar_produto de novo"
                            .to_owned(),
                    )
                } else {
                    self.executar_ferramenta(alvo, quente, uso, chamada).await
                };
                let duracao = inicio_chamada.elapsed();
                chamadas_do_turno.registrar(
                    &chamada.nome,
                    rodada,
                    desfecho.para_registro(),
                    duracao,
                    &chamada.argumentos,
                    desfecho.texto(),
                );
                conversa.push(Message::Resultado {
                    id: chamada.id.clone(),
                    conteudo: desfecho.em_texto(),
                });
            }
        }
        // As mensagens finais vão junto porque a revisão do G6 (HR-34) reescreve a resposta a
        // partir da mesma conversa: o modelo vê o rascunho reprovado e a orientação da revisão.
        Ok((resposta, conversa))
    }

    /// Prepara a busca na base do agente (HR-17). `None` quando não há busca configurada ou quando
    /// o grupo não tem texto de cliente. O futuro é aguardado mais adiante, já em paralelo com a
    /// montagem do resto do prompt.
    fn iniciar_busca<'a>(
        &'a self,
        org: OrgId,
        agente: AgentId,
        grupo: &[MensagemGravada],
        quente: &AgenteQuente,
    ) -> Option<crate::base_conhecimento::FuturoBusca<'a>> {
        let base = self.busca.as_ref()?;
        let consulta = consulta_do_grupo(grupo);
        if consulta.is_empty() {
            return None;
        }
        Some(base.buscar(
            org,
            agente,
            consulta,
            ParametrosBusca {
                top_k: usize::try_from(quente.top_k).unwrap_or(3).max(1),
                corte: quente.corte as f32,
            },
        ))
    }

    /// Efeito do G5 (HR-33): pré-consulta o catálogo e pergunta ao Jev qual produto o cliente quer.
    ///
    /// Devolve o produto escolhido **validado contra os candidatos deste turno** — id fora da lista
    /// nunca passa, porque quem manda é o resultado da busca, não a confiança do modelo. Qualquer
    /// outro caminho (`None`) é o comportamento de hoje: os candidatos vão ao LLM, que pode chamar
    /// `buscar_produto` normalmente.
    async fn produto_do_g5(
        &self,
        contexto: ContextoG5<'_>,
        jev: &mut Option<ResultadoJev>,
    ) -> Option<ProdutoEncontrado> {
        let ContextoG5 {
            org,
            conversa,
            quente,
            ultimas,
            grupo,
            g3_nada,
        } = contexto;
        let avaliador = Arc::clone(self.jev.as_ref()?);
        // G3 em `nada` já tirou o catálogo do turno; sem a ferramenta habilitada o agente não fala
        // de preço; sem busca de produtos não há candidato.
        if g3_nada
            || !self.gates_do_agente(&quente.agente).g5
            || !quente.ferramenta_habilitada(crate::produtos::NOME_FERRAMENTA)
        {
            return None;
        }
        let produtos = self.produtos.as_ref()?;
        let consulta = consulta_do_grupo(grupo);
        if consulta.is_empty() {
            return None;
        }
        let parametros = ParametrosProdutos {
            top_k: usize::try_from(quente.top_k).unwrap_or(3).clamp(1, 10),
            corte: quente.corte as f32,
        };
        let candidatos = match produtos
            .buscar(org, quente.agente.id, consulta, None, parametros)
            .await
        {
            Ok(candidatos) => candidatos,
            Err(erro) => {
                // Catálogo fora do ar não derruba o turno: ele segue como sem o G5.
                tracing::warn!(
                    erro = %erro,
                    "G5: a busca de produtos falhou; turno segue como sem o gate"
                );
                return None;
            }
        };
        if candidatos.is_empty() {
            return None;
        }
        let do_estado: Vec<poph_jev::CandidatoProduto> =
            candidatos.iter().map(candidato_do_g5).collect();
        let gates = self.gates_do_agente(&quente.agente);
        let estado = self
            .estado_jev(org, quente, ultimas, conversa, gates, None)?
            .com_produtos_candidatos(do_estado);
        // Só o G5 nesta chamada: os outros gates já foram perguntados no lote, e repeti-los aqui
        // gastaria uma segunda rodada sem mudar nada.
        let resultado = avaliador
            .avaliar(estado, Some(poph_jev::AjusteGates::apenas(IdGate::G5)))
            .await;
        let escolha = resultado.escolha_confiavel(IdGate::G5).map(str::to_owned);
        // A pergunta do G5 é uma chamada própria: o custo dela entra no mesmo registro do turno.
        match jev {
            Some(atual) => {
                atual.decisoes.extend(resultado.decisoes);
                atual.brutas.extend(resultado.brutas);
                atual.chamada.perguntas += resultado.chamada.perguntas;
                atual.chamada.ms = atual.chamada.ms.saturating_add(resultado.chamada.ms);
                atual.chamada.tokens = atual
                    .chamada
                    .tokens
                    .saturating_add(resultado.chamada.tokens);
                atual.chamada.usage = atual.chamada.usage.somar(resultado.chamada.usage);
                if atual.chamada.falha.is_none() {
                    atual.chamada.falha = resultado.chamada.falha;
                }
            }
            None => *jev = Some(resultado),
        }
        let escolha = escolha?;
        // Validação pelo código, de novo: o id tem de ser um dos candidatos **deste** turno.
        candidatos
            .into_iter()
            .find(|produto| produto.produto.to_string() == escolha)
    }

    /// Efeito do G6 (HR-34): revisa a resposta antes do envio e, quando preciso, reescreve uma vez.
    ///
    /// O texto do modelo entra no estado (`resposta_candidata`), junto com os produtos que o turno
    /// consultou — a evidência de preço da revisão. Quem decide é o Jev; quem aplica é este código:
    /// a escolha `aprovada` mantém tudo, um `revisar_*` gasta **uma** regeneração e a segunda
    /// revisão decide se o texto novo sai ou se vale a resposta segura do agente.
    async fn revisar_g6(
        &self,
        contexto: ContextoG6<'_>,
        resposta: &mut Resposta,
        jev: &mut Option<ResultadoJev>,
    ) -> RevisaoG6 {
        let ContextoG6 {
            org,
            quente,
            ultimas,
            conversa,
            mensagens,
            produtos,
            modelo,
            cancelamento,
        } = contexto;
        let mut revisao = RevisaoG6::default();
        let Some(avaliador) = self.jev.as_ref().map(Arc::clone) else {
            return revisao;
        };
        if !self.gates_do_agente(&quente.agente).g6 {
            return revisao;
        }
        if resposta.texto.trim().is_empty() || mensagens.is_empty() {
            return revisao;
        }

        let Some(escolha) = self
            .revisar_texto(
                org,
                quente,
                ultimas,
                conversa,
                &produtos,
                &resposta.texto,
                avaliador.as_ref(),
                jev,
            )
            .await
        else {
            return revisao;
        };
        revisao.escolha = Some(escolha.clone());
        revisao.confianca = jev
            .as_ref()
            .and_then(|resultado| resultado.decisao(IdGate::G6))
            .map(|decisao| decisao.confianca)
            .unwrap_or(0.0);
        if escolha == G6_APROVADA {
            return revisao;
        }

        // Reprovada: **uma** regeneração, com a orientação fixa do motivo escolhido.
        self.metricas.reprovadas_g6.increment(1);
        let orientacao = poph_jev::g6_orientacao(&escolha);
        let regenerada = match self
            .regenerar(
                org,
                quente,
                mensagens,
                &resposta.texto,
                orientacao,
                &modelo,
                &cancelamento,
            )
            .await
        {
            Ok(regenerada) => regenerada,
            Err(erro) => {
                // A regeneração falhou: sai o texto original. Nunca fica sem resposta.
                tracing::warn!(
                    erro = %erro,
                    "G6: a regeneração falhou; o turno segue com a resposta original"
                );
                return revisao;
            }
        };
        resposta.uso = somar_uso(resposta.uso, regenerada.uso);
        if regenerada.motivo.is_some() {
            resposta.motivo = regenerada.motivo;
        }
        let novo = regenerada.texto.trim().to_owned();
        if novo.is_empty() {
            tracing::warn!("G6: a regeneração voltou vazia; o turno segue com a resposta original");
            return revisao;
        }
        resposta.texto = novo;
        revisao.regenerou = true;

        // Segunda revisão, da resposta nova. Só `aprovada` mantém o texto regenerado; qualquer
        // outro desfecho (reprovação, abstenção, Jev fora) usa a resposta segura, se configurada.
        let segunda = self
            .revisar_texto(
                org,
                quente,
                ultimas,
                conversa,
                &produtos,
                &resposta.texto,
                avaliador.as_ref(),
                jev,
            )
            .await;
        revisao.segunda = segunda.clone();
        if segunda.as_deref() == Some(G6_APROVADA) {
            return revisao;
        }
        if let Some(segura) = poph_jev::resposta_segura_do_agente(&quente.agente.jev_json) {
            resposta.texto = segura;
            revisao.segura = true;
            self.metricas.respostas_seguras_g6.increment(1);
        }
        revisao
    }

    /// Uma revisão do G6 sobre o texto dado: monta o estado, pergunta ao Jev e devolve a escolha
    /// confiável (ou `None` quando o gate absteve, a resposta veio fora do contrato ou o Jev falhou).
    ///
    /// O custo da chamada entra no mesmo `turns.jev_json` do turno, como no G5.
    #[allow(clippy::too_many_arguments)]
    async fn revisar_texto(
        &self,
        org: OrgId,
        quente: &AgenteQuente,
        ultimas: &[MensagemGravada],
        conversa: &poph_store::Conversa,
        produtos: &[ProdutoUsado],
        texto: &str,
        avaliador: &dyn AvaliadorGates,
        jev: &mut Option<ResultadoJev>,
    ) -> Option<String> {
        let gates = self.gates_do_agente(&quente.agente);
        let candidatos: Vec<poph_jev::CandidatoProduto> = produtos
            .iter()
            .map(|produto| poph_jev::CandidatoProduto {
                id: produto.produto.to_string(),
                nome: produto.nome.clone(),
                detalhe: format!(
                    "{} {} · {}",
                    produto.preco_texto, produto.unidade, produto.condicoes
                ),
            })
            .collect();
        let estado = self
            .estado_jev(org, quente, ultimas, conversa, gates, None)?
            .com_resposta_candidata(texto)
            .com_produtos_candidatos(candidatos);
        // Só o G6 nesta chamada: os outros gates já foram perguntados no lote do turno.
        let resultado = avaliador
            .avaliar(estado, Some(AjusteGates::apenas(IdGate::G6)))
            .await;
        let escolha = resultado.escolha_confiavel(IdGate::G6).map(str::to_owned);
        somar_resultado_jev(jev, resultado);
        self.metricas.revisoes_g6.increment(1);
        escolha
    }

    /// Uma regeneração da resposta, com o rascunho reprovado e a orientação da revisão.
    ///
    /// Sem ferramentas de propósito: a revisão corrige o texto, não reabre consulta. O pedido é
    /// "assistente (rascunho) + usuário (orientação)", como qualquer reescrita.
    #[allow(clippy::too_many_arguments)]
    async fn regenerar(
        &self,
        org: OrgId,
        quente: &AgenteQuente,
        mensagens: &[Message],
        rascunho: &str,
        orientacao: &str,
        modelo: &str,
        cancelamento: &Cancellation,
    ) -> Result<RespostaRegenerada, LlmError> {
        let mut conversa = mensagens.to_vec();
        conversa.push(Message::Assistente {
            texto: rascunho.to_owned(),
            chamadas: Vec::new(),
        });
        conversa.push(Message::Usuario(vec![Part::Texto(orientacao.to_owned())]));
        let mut pedido = Request::new(org, conversa);
        pedido.modelo = Some(modelo.to_owned());
        pedido.cancelamento = cancelamento.clone();
        pedido.temperatura = quente.agente.temperatura.map(|valor| valor as f32);
        pedido.max_tokens = quente
            .agente
            .max_tokens_saida
            .and_then(|valor| u32::try_from(valor).ok());
        let provedor_do_turno = self.provedor_do_turno(quente)?;
        let _vaga = self
            .limitador
            .adquirir_provedor(provedor_do_turno.nome(), self.opcoes.espera_vaga)
            .await
            .map_err(erro_de_vaga_do_provedor)?;
        let mut fluxo = provedor_do_turno.cliente().stream(pedido).await?;
        let mut regenerada = RespostaRegenerada::default();
        while let Some(evento) = self.proximo_evento(&mut fluxo, cancelamento).await? {
            match evento? {
                LlmEvent::Texto(pedaco) => regenerada.texto.push_str(&pedaco),
                // Sem ferramentas no pedido: uma chamada aqui é ignorada de propósito.
                LlmEvent::Tool(_) => {}
                LlmEvent::Usage(uso) => regenerada.uso = somar_uso(regenerada.uso, uso),
                LlmEvent::Fim(motivo) => regenerada.motivo = Some(motivo),
            }
        }
        Ok(regenerada)
    }

    /// Prepara o estado do Jev e devolve o futuro do lote único de perguntas (HR-28).
    ///
    /// `None` quando o Jev está desligado no motor, quando nenhum gate está ligado para o agente,
    /// quando não há mensagem para julgar ou quando o estado não se aplica a gate nenhum. Nada aqui
    /// pode falhar para o turno: no pior caso ele roda sem Jev, como antes da HR-28.
    fn iniciar_jev(
        &self,
        org: OrgId,
        quente: &AgenteQuente,
        ultimas: &[MensagemGravada],
        conversa: &poph_store::Conversa,
        fluxos: Option<&[poph_jev::FluxoJev]>,
    ) -> Option<futures::future::BoxFuture<'static, ResultadoJev>> {
        let avaliador = Arc::clone(self.jev.as_ref()?);
        let ajuste: Option<AjusteGates> = poph_jev::ajuste_do_agente(&quente.agente.jev_json);
        let gates: JevGates = avaliador.gates_efetivos(ajuste);
        if MotorJev::nenhum(gates) {
            return None;
        }
        let estado = self.estado_jev(org, quente, ultimas, conversa, gates, fluxos)?;
        Some(Box::pin(
            async move { avaliador.avaliar(estado, ajuste).await },
        ))
    }

    /// O que o Jev enxerga neste turno (conversa, política do agente e o que ele tem à mão).
    ///
    /// `None` quando não há mensagem para julgar. É a mesma montagem usada pelo lote (HR-28) e pela
    /// pergunta de risco da transferência (HR-29), para as duas verem a mesma conversa.
    fn estado_jev(
        &self,
        org: OrgId,
        quente: &AgenteQuente,
        ultimas: &[MensagemGravada],
        conversa: &poph_store::Conversa,
        gates: JevGates,
        fluxos: Option<&[poph_jev::FluxoJev]>,
    ) -> Option<EstadoJev> {
        let mensagens: Vec<MensagemJev> = ultimas.iter().filter_map(mensagem_do_jev).collect();
        if mensagens.is_empty() {
            return None;
        }
        let mut estado = EstadoJev::novo(conversa.canal.clone())
            .com_mensagens(mensagens)
            .com_handoff(
                poph_jev::situacoes_do_agente(&quente.agente.jev_json),
                quente.agente.handoff_active,
            )
            .com_capacidades(
                self.busca.is_some(),
                quente.ferramenta_habilitada(crate::produtos::NOME_FERRAMENTA),
            );
        if gates.g4 {
            estado = estado.com_departamentos(self.departamentos_ativos(org));
        }
        // FX-23: os fluxos liberados vão no estado **só** quando o G9 está ligado para o agente —
        // com o gate desligado o estado fica como era, e as medições dos outros gates não mudam.
        if gates.g9
            && let Some(fluxos) = fluxos
        {
            estado = estado.com_fluxos_liberados(fluxos.iter().cloned());
        }
        // G7 (HR-35): o texto extraído do arquivo do cliente vai junto com a mensagem, para o Jev
        // julgar injeção embutida no documento. Sem arquivo com texto, o campo não existe.
        if let Some(texto) = self.texto_de_arquivos(org, ultimas) {
            estado = estado.com_texto_de_arquivo(texto);
        }
        Some(estado)
    }

    /// Partes que os arquivos de uma mensagem do cliente viram no prompt (HR-23b).
    ///
    /// Ordem: texto extraído (aviso + bloco não confiável) e, depois, as imagens do anexo. Uma
    /// leitura de anexos por mensagem; mensagem sem anexo não consulta o banco.
    ///
    /// Falha de leitura nunca derruba o turno: o arquivo daquele anexo fica de fora e o resto segue.
    fn partes_de_arquivos(
        &self,
        org: OrgId,
        mensagem: &MensagemGravada,
        imagens: &mut OrcamentoImagens,
    ) -> Vec<Part> {
        let anexos_json = mensagem.attachments_json.trim();
        if anexos_json.is_empty() || anexos_json == "[]" {
            return Vec::new();
        }
        let anexos = match self.store.messages().attachments(org, mensagem.id) {
            Ok(anexos) => anexos,
            Err(erro) => {
                tracing::warn!(erro = %erro, "não consegui ler os anexos da mensagem");
                return Vec::new();
            }
        };
        let mut partes: Vec<Part> = Vec::new();
        let mut textos: Vec<String> = Vec::new();
        for anexo in &anexos {
            // Imagem vai como parte multimodal; o texto extraído (documento) vai como bloco.
            if poph_core::llm::IMAGE_MIMES.contains(&anexo.mime.as_str()) {
                if let Some(parte) = self.imagem_do_anexo(org, anexo, imagens) {
                    partes.push(parte);
                }
                continue;
            }
            if let Some(texto) = &anexo.extracted_text {
                let texto = texto.trim();
                if !texto.is_empty() {
                    textos.push(texto.to_owned());
                }
            }
        }
        if !textos.is_empty() {
            // Vários documentos da mesma mensagem viram um texto só, como o G7 já os lê.
            partes.insert(
                0,
                Part::Texto(poph_files::envelope::AVISO_NAO_CONFIAVEL.to_owned()),
            );
            partes.insert(1, Part::Texto(textos.join("\n\n")));
        }
        partes
    }

    /// Lê os bytes de uma imagem do blob da org e devolve a parte para o modelo.
    ///
    /// `None` (com registro no log) quando: não há pasta de blobs configurada, a imagem passa do
    /// teto do turno, o blob não está no disco ou está ilegível. Nunca falha o turno, e o log nunca
    /// traz bytes nem base64 — só o motivo.
    fn imagem_do_anexo(
        &self,
        org: OrgId,
        anexo: &poph_store::AttachmentRecord,
        imagens: &mut OrcamentoImagens,
    ) -> Option<Part> {
        let Some(arquivos) = self.arquivos.as_ref() else {
            tracing::debug!(
                anexo = %anexo.id,
                "sem configuração de arquivos no turno: imagem do anexo não vai ao modelo"
            );
            return None;
        };
        if !imagens.cabe(anexo.size_bytes) {
            tracing::warn!(
                anexo = %anexo.id,
                tamanho_bytes = anexo.size_bytes,
                "imagem omitida do turno: passa do teto de imagens por turno"
            );
            return None;
        }
        match poph_files::blob::ler(
            &arquivos.pasta_blobs,
            org,
            &anexo.sha256,
            MAX_BYTES_POR_IMAGEM,
        ) {
            Ok(bytes) if !bytes.is_empty() => {
                imagens.consumir(bytes.len() as u64);
                Some(Part::Imagem {
                    mime: anexo.mime.clone(),
                    bytes,
                })
            }
            Ok(_) => {
                tracing::warn!(anexo = %anexo.id, "imagem vazia no blob; turno segue sem ela");
                None
            }
            Err(erro) => {
                tracing::warn!(
                    anexo = %anexo.id,
                    erro = %erro,
                    "não consegui ler a imagem do anexo; turno segue sem ela"
                );
                None
            }
        }
    }

    /// Texto extraído dos arquivos das mensagens do cliente dadas, já envelopado.
    ///
    /// Na ordem em que os arquivos chegaram; vários arquivos viram um texto só, separado por linha
    /// em branco (o envelope de cada um continua fechado). Falha de leitura não derruba o turno: o
    /// texto daquele arquivo fica de fora e o turno segue.
    fn texto_de_arquivos(&self, org: OrgId, mensagens: &[MensagemGravada]) -> Option<String> {
        let mut textos: Vec<String> = Vec::new();
        for mensagem in mensagens {
            if mensagem.role != Role::User {
                continue;
            }
            // Sem anexo não há o que ler: a imensa maioria das mensagens cai fora aqui, sem query.
            let anexos = mensagem.attachments_json.trim();
            if anexos.is_empty() || anexos == "[]" {
                continue;
            }
            match self.store.messages().attachments(org, mensagem.id) {
                Ok(anexos) => {
                    for anexo in anexos {
                        if let Some(texto) = anexo.extracted_text {
                            let texto = texto.trim();
                            if !texto.is_empty() {
                                textos.push(texto.to_owned());
                            }
                        }
                    }
                }
                Err(erro) => {
                    tracing::warn!(erro = %erro, "não consegui ler os anexos da mensagem");
                }
            }
        }
        if textos.is_empty() {
            None
        } else {
            Some(textos.join("\n\n"))
        }
    }

    /// Nomes dos departamentos ativos da org: os candidatos do G4 no estado enviado ao Jev.
    fn departamentos_ativos(&self, org: OrgId) -> Vec<String> {
        self.departamentos_ativos_com_id(org)
            .into_iter()
            .map(|candidato| candidato.nome)
            .collect()
    }

    /// Departamentos ativos da org com id e nome.
    ///
    /// Duas leituras, de propósito: o **estado** manda só os nomes ao Jev, e a **decisão** é
    /// validada contra esta mesma lista lida no momento da transferência (HR-32) — departamento
    /// apagado ou desativado no meio do caminho é recusado, e a conversa vai para o padrão.
    fn departamentos_ativos_com_id(&self, org: OrgId) -> Vec<departamento::Candidato> {
        match self.store.departments().list(org, Page::with_limit(200)) {
            Ok(departamentos) => departamentos
                .iter()
                .filter(|departamento| departamento.status == poph_store::Status::Active)
                .map(departamento::Candidato::de)
                .collect(),
            Err(erro) => {
                tracing::warn!(erro = %erro, "não consegui listar departamentos para o G4");
                Vec::new()
            }
        }
    }

    /// Próximo evento do stream, interrompido também pelo cancelamento do processo.
    async fn proximo_evento(
        &self,
        fluxo: &mut poph_core::llm::LlmStream,
        cancelamento: &Cancellation,
    ) -> Result<Option<Result<LlmEvent, LlmError>>, LlmError> {
        match &self.cancelamento {
            Some(compartilhado) => tokio::select! {
                evento = fluxo.next() => Ok(evento),
                _ = compartilhado.aguardar() => {
                    cancelamento.cancelar();
                    Err(LlmError::Cancelled)
                }
            },
            None => Ok(fluxo.next().await),
        }
    }

    /// Executa (ou recusa) uma chamada de ferramenta do modelo.
    async fn executar_ferramenta(
        &self,
        alvo: Alvo,
        quente: &AgenteQuente,
        uso: &Arc<UsoFerramentas>,
        chamada: &ToolCall,
    ) -> DesfechoFerramenta {
        // D-04: ferramenta desabilitada nunca é executada, mesmo que o modelo invente o nome.
        if !quente.ferramenta_habilitada(&chamada.nome) {
            return DesfechoFerramenta::recusada(
                MOTIVO_NAO_HABILITADA,
                format!(
                    "erro: a ferramenta `{}` não está habilitada para este agente",
                    chamada.nome
                ),
            );
        }
        let Some(ferramenta) = self.ferramentas.ferramenta(&chamada.nome) else {
            return DesfechoFerramenta::recusada(
                MOTIVO_DESCONHECIDA,
                format!(
                    "erro: a ferramenta `{}` não está disponível neste motor",
                    chamada.nome
                ),
            );
        };
        let contexto = ContextoFerramenta {
            org: alvo.org,
            conversa: alvo.conversa,
            agente: quente.agente.id,
            turno: alvo.turno,
            uso: Arc::clone(uso),
        };
        match ferramenta
            .executar(contexto, chamada.argumentos.clone())
            .await
        {
            Ok(texto) => DesfechoFerramenta::Executada(texto),
            Err(erro) => DesfechoFerramenta::recusada(
                motivo_do_erro_da_ferramenta(&erro),
                format!("erro: {erro}"),
            ),
        }
    }

    // ───────────────────────── Consultas e encerramentos ─────────────────────────

    /// Janela de agrupamento do agente: a dele, limitada ao teto global e ao padrão quando zero.
    pub fn janela_do_agente(&self, agente: &poph_store::Agente) -> Duration {
        let do_agente = Duration::from_millis(u64::from(agente.grouping_window_ms));
        let janela = if do_agente.is_zero() {
            self.opcoes.janela_padrao
        } else {
            do_agente
        };
        janela.min(self.opcoes.janela_maxima)
    }

    /// Últimas mensagens da conversa (a mesma leitura usada pelo agrupamento e pelo Jev).
    fn ultimas_mensagens(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Resultado<Vec<MensagemGravada>> {
        Ok(self.store.messages().last_set(
            org,
            conversa,
            self.opcoes.teto_leitura_mensagens.max(1),
        )?)
    }

    /// Conta o desfecho do fluxo do turno nas séries `poph_fluxos_g9_*` (FX-26/FX-23).
    ///
    /// Cada turno cai em **uma** série. "Reserva" (gate desligado ou acima do teto de 20 fluxos)
    /// não conta como falha: o Jev não errou, ele não foi perguntado.
    fn contar_g9(&self, desfecho: &fluxo::DesfechoDoFluxo) {
        let Some(registro) = desfecho.registro.as_ref() else {
            return;
        };
        match registro
            .get("resultado")
            .and_then(serde_json::Value::as_str)
        {
            Some("iniciou" | "iniciou_sem_bloco") => self.fluxos_g9.iniciou.increment(1),
            Some("nenhum") => self.fluxos_g9.nenhum.increment(1),
            Some("abaixo_do_limiar") => self.fluxos_g9.abaixo_do_limiar.increment(1),
            Some("fallback" | "falhou" | "nao_iniciou") => self.fluxos_g9.fallback.increment(1),
            _ => {}
        }
    }

    /// Gates efetivos do agente: configuração global ajustada pelo `jev_json` dele.
    fn gates_do_agente(&self, agente: &poph_store::Agente) -> JevGates {
        let Some(avaliador) = self.jev.as_ref() else {
            return JevGates::default();
        };
        avaliador.gates_efetivos(poph_jev::ajuste_do_agente(&agente.jev_json))
    }

    /// `true` quando o G2 está ligado para este agente — é ele quem governa a janela (HR-30).
    fn g2_ligado(&self, agente: &poph_store::Agente) -> bool {
        self.jev.is_some() && self.gates_do_agente(agente).g2
    }

    /// Modelo do pedido: o do agente ou, quando vazio, o da configuração.
    fn modelo_usado(&self, quente: &AgenteQuente) -> String {
        let modelo = quente.agente.modelo.trim();
        if !modelo.is_empty() {
            return modelo.to_owned();
        }
        match self.provedor_do_turno(quente) {
            Ok(provedor) => provedor.modelo().to_owned(),
            Err(_) => self.modelo_padrao.clone(),
        }
    }

    fn provedor_do_turno(&self, quente: &AgenteQuente) -> Result<ProvedorResolvido, LlmError> {
        let Some(provedores) = &self.provedores else {
            return Ok(ProvedorResolvido::novo(
                quente.agente.provedor.clone(),
                Arc::clone(&self.provedor),
                self.modelo_padrao.clone(),
            ));
        };
        provedores.resolver(&quente.agente.provedor)
    }

    /// Grava o turno como `skipped` e devolve o resumo (agente inativo, conversa fechada, sem
    /// mensagem nova).
    /// Efeito do G1: transfere a conversa para humano e entrega ao cliente a orientação (HR-29).
    ///
    /// Sequência, toda ela fail-safe:
    /// 1. pergunta o tipo de risco (segunda pergunta do G1) — erro, prazo ou resposta ambígua só
    ///    suprimem a orientação, nunca a transferência;
    /// 2. grava **uma** mensagem do agente com a orientação aprovada (quando há) seguida da
    ///    mensagem automática, e enfileira o mesmo texto na `outbox`;
    /// 3. resolve o departamento de destino (HR-32): a escolha do G4 é validada contra os
    ///    departamentos ativos da org agora, e sem escolha válida vale o padrão do agente;
    /// 4. marca a conversa como `transferred` com auditoria (`actor_kind = 'system'`) na mesma
    ///    transação;
    /// 5. fecha o turno como concluído, com as decisões do Jev, o departamento resolvido e a
    ///    classificação de risco no registro.
    ///
    /// Nenhuma chamada ao modelo de texto acontece aqui: é o que garante o "não responde mais".
    #[allow(clippy::too_many_arguments)]
    async fn transferir(
        &self,
        alvo: Alvo,
        quente: &AgenteQuente,
        ultimas: &[MensagemGravada],
        conversa: &poph_store::Conversa,
        jev: &ResultadoJev,
        origem: IdGate,
        grupo_ids: Vec<MessageId>,
        (fila_ms, montagem_ms): (u64, u64),
        inicio: Instant,
        observador: &mut dyn Observador,
    ) -> Resultado<ResumoTurno> {
        let org = alvo.org;
        let conversa_id = alvo.conversa;
        let turno = alvo.turno;

        // 1. Orientação mínima (segunda pergunta do G1). Nunca segura a transferência.
        let risco = self.classificar_risco(org, quente, ultimas, conversa).await;
        let tipo = self.handoff.tipo_confiavel(&risco);
        let texto = self.handoff.compor(tipo.as_deref());

        // 2. Mensagem do agente + item da `outbox` (idempotente por turno + seq, como a resposta),
        //    escritos juntos (HR-57). A configuração recusa mensagem automática vazia; se alguém
        //    montar a orientação em código sem texto, a transferência acontece sem mensagem em vez
        //    de mandar vazio.
        if texto.trim().is_empty() {
            tracing::warn!(
                conversa = %conversa_id,
                "transferência sem mensagem automática configurada: cliente não recebe texto"
            );
        } else {
            self.store.messages().write_with_output(
                org,
                NewMessage {
                    conversation_id: conversa_id,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    texto: texto.clone(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: Some(turno),
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                NewOutboxItem {
                    conversation_id: conversa_id,
                    turn_id: turno,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": texto }).to_string(),
                },
            )?;
            self.metricas.respostas_gravadas.increment(1);
        }

        // 3. Departamento de destino (HR-32). A escolha do G4 é validada contra os departamentos
        //    ativos da org **lidos agora** — outra org, apagado ou inativo é recusado com motivo — e,
        //    sem escolha válida, vale o departamento de transferência do agente (o de hoje).
        let escolha_do_g4 = jev.escolha_confiavel(IdGate::G4);
        let motivo_da_abstencao = jev
            .decisao(IdGate::G4)
            .and_then(|decisao| decisao.motivo.clone());
        let candidatos = if escolha_do_g4.is_some() {
            self.departamentos_ativos_com_id(org)
        } else {
            Vec::new()
        };
        let destino = departamento::resolver(
            escolha_do_g4,
            &candidatos,
            quente.agente.handoff_department,
            motivo_da_abstencao.as_deref(),
        );
        if let Some(motivo) = &destino.motivo {
            tracing::info!(
                conversa = %conversa_id,
                escolha_do_jev = ?escolha_do_g4,
                motivo = %motivo,
                "departamento do G4 não aplicado; usando o padrão do agente"
            );
        }

        // 4. Conversa transferida, com auditoria na mesma transação.
        self.store
            .conversations()
            .touch(org, conversa_id, self.store.now())?;
        let departamento = destino.departamento;
        let detalhe = json!({
            "gate": origem.como_texto(),
            "decisao": jev.escolha(origem),
            "confianca": jev.decisao(origem).map(|decisao| decisao.confianca),
            // A decisão do G1 vai sempre: a transferência do G7 (HR-35) usa este mesmo caminho.
            "g1_decisao": jev.escolha(IdGate::G1),
            "risco": tipo,
            "orientacao": tipo.is_some(),
            "departamento": departamento.map(|dep| dep.to_string()),
            "departamento_origem": destino.origem.como_texto(),
            "departamento_escolhido_pelo_jev": escolha_do_g4,
            "departamento_motivo": destino.motivo,
        })
        .to_string();
        self.store.conversations().transfer(
            org,
            conversa_id,
            departamento,
            Some("turno".to_owned()),
            detalhe,
        )?;

        // 5. Registro do turno.
        let tempos = Tempos {
            fila_ms,
            montagem_ms,
            primeiro_texto_ms: None,
            total_ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
            rodadas_ferramenta: 0,
        };
        let modelo = self.modelo_usado(quente);
        let mut registro = jev.para_json();
        registro["risco"] = risco.para_json();
        // Efeito do G4, já validado pelo código: o painel "por quê" (HR-47) mostra para onde a
        // conversa foi e, quando não foi a escolha do Jev, por quê.
        registro["transferencia"] = json!({
            // Gate que pediu a transferência: G1 (pedido do cliente) ou G7 (injeção/abuso).
            "gate": origem.como_texto(),
            "departamento": departamento.map(|dep| dep.to_string()),
            "origem": destino.origem.como_texto(),
            "escolha_do_jev": escolha_do_g4,
            "motivo": destino.motivo,
        });
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Completed),
                jev_json: Some(registro.to_string()),
                times_json: Some(tempos_json(&tempos, &modelo, None)),
                ..TurnFinalization::default()
            },
        )?;
        self.metricas.turnos_concluidos.increment(1);
        self.metricas
            .duracao_turno_ms
            .observe(tempos.total_ms as f64);
        tracing::info!(
            conversa = %conversa_id,
            departamento = ?departamento,
            departamento_origem = destino.origem.como_texto(),
            orientacao = tipo.is_some(),
            "conversa transferida para humano (G1)"
        );
        observador.evento(EventoTurno::Transferencia {
            departamento,
            tipo_risco: tipo.clone(),
            orientacao: tipo.is_some(),
        });
        observador.evento(EventoTurno::Texto(texto.clone()));
        observador.evento(EventoTurno::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: Some("transferida para humano".to_owned()),
        });

        Ok(ResumoTurno {
            turno,
            estado: TurnState::Completed,
            texto,
            mensagens: grupo_ids,
            tokens_entrada: 0,
            tokens_saida: 0,
            tempos,
            modelo,
            motivo_fim: Some("transferida para humano".to_owned()),
            erro: None,
            falha_modelo: false,
            jev: Some(jev.clone()),
        })
    }

    /// Efeito do G7 com a ação `responder_padrao` (HR-35): manda o texto neutro do agente (ou o
    /// padrão do gate) e conclui o turno **sem** chamar o modelo de texto.
    ///
    /// O texto configurado em `jev_json.g7_resposta` pode ser vazio? Não: vazio é tratado como não
    /// configurado, e vale o [`G7_RESPOSTA_PADRAO`], porque aqui o silêncio seria deixar o cliente
    /// sem resposta — diferente da despedida do G8, em que vazio é uma escolha legítima.
    #[allow(clippy::too_many_arguments)]
    fn responder_g7(
        &self,
        alvo: Alvo,
        quente: &AgenteQuente,
        jev: &ResultadoJev,
        grupo_ids: Vec<MessageId>,
        (fila_ms, montagem_ms): (u64, u64),
        inicio: Instant,
        observador: &mut dyn Observador,
    ) -> Resultado<ResumoTurno> {
        let org = alvo.org;
        let conversa_id = alvo.conversa;
        let turno = alvo.turno;

        let texto = poph_jev::resposta_g7_do_agente(&quente.agente.jev_json)
            .unwrap_or_else(|| G7_RESPOSTA_PADRAO.to_owned());
        self.store.messages().write_with_output(
            org,
            NewMessage {
                conversation_id: conversa_id,
                direction: Direction::Outbound,
                role: Role::Assistant,
                texto: texto.clone(),
                attachments_json: "[]".to_owned(),
                external_id: None,
                turn_id: Some(turno),
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            NewOutboxItem {
                conversation_id: conversa_id,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                content_json: json!({ "texto": texto }).to_string(),
            },
        )?;
        self.metricas.respostas_gravadas.increment(1);
        self.store
            .conversations()
            .touch(org, conversa_id, self.store.now())?;

        let tempos = Tempos {
            fila_ms,
            montagem_ms,
            primeiro_texto_ms: None,
            total_ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
            rodadas_ferramenta: 0,
        };
        let modelo = self.modelo_usado(quente);
        let motivo = "mensagem classificada como injeção ou abuso (G7)".to_owned();
        let mut registro = jev.para_json();
        registro["classificacao"] = json!({
            "gate": IdGate::G7.como_texto(),
            "escolha": jev.escolha(IdGate::G7),
            "confianca": jev.decisao(IdGate::G7).map(|decisao| decisao.confianca),
            "acao": AcaoG7::ResponderPadrao.como_texto(),
        });
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Completed),
                jev_json: Some(registro.to_string()),
                times_json: Some(tempos_json(&tempos, &modelo, None)),
                ..TurnFinalization::default()
            },
        )?;
        self.metricas.turnos_concluidos.increment(1);
        self.metricas.g7_respostas_padrao.increment(1);
        self.metricas
            .duracao_turno_ms
            .observe(tempos.total_ms as f64);
        tracing::warn!(
            conversa = %conversa_id,
            "G7: resposta neutra enviada sem chamar o modelo de texto"
        );
        observador.evento(EventoTurno::Texto(texto.clone()));
        observador.evento(EventoTurno::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: Some(motivo.clone()),
        });

        Ok(ResumoTurno {
            turno,
            estado: TurnState::Completed,
            texto,
            mensagens: grupo_ids,
            tokens_entrada: 0,
            tokens_saida: 0,
            tempos,
            modelo,
            motivo_fim: Some(motivo),
            erro: None,
            falha_modelo: false,
            jev: Some(jev.clone()),
        })
    }

    /// Efeito do G8 (HR-36): a conversa foi encerrada pelo cliente.
    ///
    /// Grava a despedida (a do agente em `jev_json.despedida`, ou o padrão; texto vazio = nada
    /// sai), enfileira na `outbox` como qualquer resposta e fecha o turno com o registro do Jev.
    /// Nenhuma chamada ao modelo de texto acontece aqui.
    #[allow(clippy::too_many_arguments)]
    fn encerrar_g8(
        &self,
        alvo: Alvo,
        quente: &AgenteQuente,
        jev: &ResultadoJev,
        grupo_ids: Vec<MessageId>,
        (fila_ms, montagem_ms): (u64, u64),
        inicio: Instant,
        observador: &mut dyn Observador,
    ) -> Resultado<ResumoTurno> {
        let org = alvo.org;
        let conversa_id = alvo.conversa;
        let turno = alvo.turno;

        let texto = poph_jev::despedida_do_agente(&quente.agente.jev_json)
            .unwrap_or_else(|| G8_DESPEDIDA_PADRAO.to_owned())
            .trim()
            .to_owned();
        if !texto.is_empty() {
            self.store.messages().write_with_output(
                org,
                NewMessage {
                    conversation_id: conversa_id,
                    direction: Direction::Outbound,
                    role: Role::Assistant,
                    texto: texto.clone(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: Some(turno),
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
                NewOutboxItem {
                    conversation_id: conversa_id,
                    turn_id: turno,
                    seq: 0,
                    tipo: "text".to_owned(),
                    content_json: json!({ "texto": texto }).to_string(),
                },
            )?;
            self.metricas.respostas_gravadas.increment(1);
        }
        self.store
            .conversations()
            .touch(org, conversa_id, self.store.now())?;

        let tempos = Tempos {
            fila_ms,
            montagem_ms,
            primeiro_texto_ms: None,
            total_ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
            rodadas_ferramenta: 0,
        };
        let modelo = self.modelo_usado(quente);
        let motivo = "conversa encerrada pelo cliente (G8)".to_owned();
        let mut registro = jev.para_json();
        registro["encerramento"] = json!({
            "gate": IdGate::G8.como_texto(),
            "confianca": jev.decisao(IdGate::G8).map(|decisao| decisao.confianca),
            "despedida": !texto.is_empty(),
        });
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Completed),
                jev_json: Some(registro.to_string()),
                times_json: Some(tempos_json(&tempos, &modelo, None)),
                ..TurnFinalization::default()
            },
        )?;
        self.metricas.turnos_concluidos.increment(1);
        self.metricas.encerradas_g8.increment(1);
        self.metricas
            .duracao_turno_ms
            .observe(tempos.total_ms as f64);
        tracing::info!(
            conversa = %conversa_id,
            despedida = !texto.is_empty(),
            "conversa encerrada pelo cliente (G8)"
        );
        if !texto.is_empty() {
            observador.evento(EventoTurno::Texto(texto.clone()));
        }
        observador.evento(EventoTurno::Fim {
            estado: TurnState::Completed.as_text().to_owned(),
            motivo: Some(motivo.clone()),
        });

        Ok(ResumoTurno {
            turno,
            estado: TurnState::Completed,
            texto,
            mensagens: grupo_ids,
            tokens_entrada: 0,
            tokens_saida: 0,
            tempos,
            modelo,
            motivo_fim: Some(motivo),
            erro: None,
            falha_modelo: false,
            jev: Some(jev.clone()),
        })
    }

    /// Segunda pergunta do G1 (HR-29): o tipo de risco imediato, para a orientação ao cliente.
    ///
    /// Fail-safe por desenho: orientação desligada, Jev desligado, prazo estourado ou erro do
    /// provedor devolvem [`ResultadoOrientacao`] sem tipo, e a transferência segue só com a
    /// mensagem automática (PRD §5.4).
    async fn classificar_risco(
        &self,
        org: OrgId,
        quente: &AgenteQuente,
        ultimas: &[MensagemGravada],
        conversa: &poph_store::Conversa,
    ) -> ResultadoOrientacao {
        if !self.handoff.ativo {
            return ResultadoOrientacao::nao_consultado("orientação desligada na configuração");
        }
        let Some(avaliador) = self.jev.as_ref() else {
            return ResultadoOrientacao::nao_consultado("Jev desligado no motor");
        };
        let ajuste: Option<AjusteGates> = poph_jev::ajuste_do_agente(&quente.agente.jev_json);
        let gates: JevGates = avaliador.gates_efetivos(ajuste);
        let Some(estado) = self.estado_jev(org, quente, ultimas, conversa, gates, None) else {
            return ResultadoOrientacao::nao_consultado("sem mensagem para julgar");
        };
        let resultado = avaliador.orientar(estado, self.handoff.prazo).await;
        if resultado.falhou() {
            tracing::warn!(
                falha = ?resultado.falha,
                "pergunta de risco da transferência falhou; segue só com a mensagem automática"
            );
        }
        resultado
    }

    fn encerrar_ignorado(
        &self,
        org: OrgId,
        conversa: ConversationId,
        agente: Option<AgentId>,
        motivo: String,
        inicio: Instant,
    ) -> Resultado<ResumoTurno> {
        let turno = self.store.turns().start(org, conversa, agente)?;
        let tempos = Tempos {
            total_ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
            ..Tempos::default()
        };
        self.store.turns().finalise(
            org,
            turno.id,
            TurnFinalization {
                state: Some(TurnState::Skipped),
                times_json: Some(tempos_json(&tempos, "", None)),
                error: Some(motivo.clone()),
                ..TurnFinalization::default()
            },
        )?;
        self.metricas.turnos_ignorados.increment(1);
        if let Some(agente) = agente {
            let _ = self.store.audit().register(
                Some(org),
                NewAuditEntry {
                    ator_tipo: "system".to_owned(),
                    actor_id: Some("turno".to_owned()),
                    acao: "skip".to_owned(),
                    entity: "turn".to_owned(),
                    entity_id: Some(turno.id.to_string()),
                    detail_json: json!({
                        "motivo": motivo,
                        "conversa": conversa.to_string(),
                        "agente": agente.to_string(),
                    })
                    .to_string(),
                },
            );
        }
        Ok(ResumoTurno {
            turno: turno.id,
            estado: TurnState::Skipped,
            texto: String::new(),
            mensagens: Vec::new(),
            tokens_entrada: 0,
            tokens_saida: 0,
            tempos,
            modelo: String::new(),
            motivo_fim: None,
            erro: Some(motivo),
            falha_modelo: false,
            jev: None,
        })
    }

    /// Grava o turno como `failed` e devolve o resumo.
    #[allow(clippy::too_many_arguments)]
    fn falhar(
        &self,
        org: OrgId,
        turno: TurnId,
        modelo: &str,
        mensagens: Vec<MessageId>,
        inicio: Instant,
        fila_ms: u64,
        montagem_ms: u64,
        primeiro_texto_ms: Option<u64>,
        erro: String,
        classe: ClasseFalha,
        chamadas: Option<&RegistroChamadas>,
        observador: &mut dyn Observador,
    ) -> Resultado<ResumoTurno> {
        let tempos = Tempos {
            fila_ms,
            montagem_ms,
            primeiro_texto_ms,
            total_ms: u64::try_from(inicio.elapsed().as_millis()).unwrap_or(u64::MAX),
            rodadas_ferramenta: 0,
        };
        self.store.turns().finalise(
            org,
            turno,
            TurnFinalization {
                state: Some(TurnState::Failed),
                times_json: Some(tempos_json(&tempos, modelo, None)),
                // HR-47b: o que já tinha rodado antes da falha não se perde. Nomes ficam vazios
                // porque o contrato `tools` só lista chamadas de turno concluído.
                tools_json: chamadas.map(|registro| registro_do_turno(&[], registro)),
                error: Some(erro.clone()),
                ..TurnFinalization::default()
            },
        )?;
        self.metricas.turnos_falhos.increment(1);
        if classe == ClasseFalha::ModeloRecusado {
            // HR-26k: o provedor recusou o pedido (chave, modelo ou formato). Não se resolve
            // repetindo; a operação precisa corrigir a configuração. Aviso na hora.
            self.metricas.falhas_modelo_recusadas.increment(1);
            self.metricas.falhas_modelo_definitivas.increment(1);
            tracing::warn!(
                target: "poph_agent::turno",
                org = %org,
                turno = %turno,
                modelo,
                motivo = %motivo_para_log(&erro),
                "provedor recusou o pedido: mensagem concluída sem repetir; confira a chave e o modelo do agente"
            );
        }
        observador.evento(EventoTurno::Fim {
            estado: TurnState::Failed.as_text().to_owned(),
            motivo: Some(erro.clone()),
        });
        Ok(ResumoTurno {
            turno,
            estado: TurnState::Failed,
            texto: String::new(),
            mensagens,
            tokens_entrada: 0,
            tokens_saida: 0,
            tempos,
            modelo: modelo.to_owned(),
            motivo_fim: None,
            erro: Some(erro),
            // Falta de vaga (do turno ou do provedor) entra na mesma escada que a falha do modelo.
            falha_modelo: matches!(classe, ClasseFalha::Modelo | ClasseFalha::SemVaga),
            jev: None,
        })
    }
}

/// Mensagem gravada no formato que o Jev lê (cliente/agente).
///
/// Mensagem de ferramenta e de sistema ficam fora: o Jev decide sobre a conversa, não sobre o
/// resultado de ferramenta.
fn mensagem_do_jev(mensagem: &MensagemGravada) -> Option<MensagemJev> {
    match mensagem.direction {
        poph_store::Direction::Inbound => Some(MensagemJev::cliente(mensagem.texto.clone())),
        poph_store::Direction::Outbound => Some(MensagemJev::agente(mensagem.texto.clone())),
    }
    .filter(|_| mensagem.role != Role::Tool && mensagem.role != Role::System)
}

// ───────────────────────────── Processador de ator ─────────────────────────────

/// Liga o ator da conversa (HR-11/HR-12) ao motor do turno: processa o evento, conclui os itens da
/// `inbox` que o turno atendeu (o próprio e os do grupo) e deixa a resposta na `outbox`.
///
/// O `Fluxo` é ligado depois, com [`ProcessadorTurno::ligar_fluxo`], porque ele precisa dos atores,
/// que precisam deste processador. Quem monta a instância (servidor, CLI, teste) faz:
/// processador → atores → fluxo → `ligar_fluxo(fluxo)`.
pub struct ProcessadorTurno {
    motor: Arc<Motor>,
    fluxo: Arc<std::sync::OnceLock<Fluxo>>,
}

impl ProcessadorTurno {
    /// Cria o processador. Ligue o fluxo antes do primeiro evento.
    pub fn novo(motor: Arc<Motor>) -> Self {
        Self {
            motor,
            fluxo: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Liga o fluxo de entrada e saída (o mesmo que a subida retoma e que o envio usa).
    pub fn ligar_fluxo(&self, fluxo: Fluxo) {
        let _ = self.fluxo.set(fluxo);
    }

    /// Motor por trás (teste e servidor).
    pub fn motor(&self) -> &Arc<Motor> {
        &self.motor
    }
}

impl Processador for ProcessadorTurno {
    fn processar(&self, tarefa: Tarefa) -> Futuro {
        let motor = Arc::clone(&self.motor);
        let fluxo = self.fluxo.get().cloned();
        Box::pin(async move {
            let Some(fluxo) = fluxo else {
                return Err(ErroAgente::Interno(
                    "processador do turno sem fluxo ligado (ligar_fluxo não foi chamado)"
                        .to_owned(),
                ));
            };
            let entrada = Entrada::nova(tarefa.org, tarefa.conversa, tarefa.mensagem);
            let resumo = motor.executar(entrada, &mut ObservadorNulo).await;
            registrar_fim_do_turno(&tarefa, &resumo);
            let (erro, mensagens, falha_modelo, turno) = match &resumo {
                Ok(resumo) => (
                    resumo.erro.clone(),
                    resumo.mensagens.clone(),
                    resumo.falha_modelo,
                    Some(resumo.turno),
                ),
                Err(erro) => (Some(erro.to_string()), Vec::new(), false, None),
            };
            if let (true, Some(turno)) = (falha_modelo, turno) {
                // Falha do modelo ou falta de vaga (HR-26i e HR-26k): adiada com limite, ou
                // concluída se esgotou.
                tratar_falha_modelo(
                    motor.metricas(),
                    &fluxo,
                    &tarefa,
                    turno,
                    &mensagens,
                    erro.as_deref().unwrap_or("falha do modelo"),
                )?;
            } else {
                concluir_entradas(
                    &fluxo,
                    tarefa.org,
                    tarefa.conversa,
                    tarefa.entrada,
                    &mensagens,
                    erro.as_deref(),
                )?;
            }
            Ok(())
        })
    }
}

/// Uma linha de log por turno que termina no servidor (HR-26h). Só ids, estado, tempos, tokens e
/// o motivo sem números longos nem `@`: nada de texto do cliente, da resposta ou de telefone.
fn registrar_fim_do_turno(tarefa: &Tarefa, resumo: &Resultado<ResumoTurno>) {
    match resumo {
        Ok(resumo) => {
            let estado = resumo.estado.as_text();
            match resumo.estado {
                TurnState::Completed => tracing::info!(
                    target: "poph_agent::turno",
                    org = %tarefa.org,
                    conversa = %tarefa.conversa,
                    turno = %resumo.turno,
                    estado,
                    duracao_ms = resumo.tempos.total_ms,
                    tokens_entrada = resumo.tokens_entrada,
                    tokens_saida = resumo.tokens_saida,
                    "turno concluído"
                ),
                TurnState::Skipped => tracing::info!(
                    target: "poph_agent::turno",
                    org = %tarefa.org,
                    conversa = %tarefa.conversa,
                    turno = %resumo.turno,
                    estado,
                    motivo = %motivo_para_log(resumo.erro.as_deref().unwrap_or("")),
                    "turno sem resposta: a mensagem ficou registrada"
                ),
                _ => tracing::warn!(
                    target: "poph_agent::turno",
                    org = %tarefa.org,
                    conversa = %tarefa.conversa,
                    turno = %resumo.turno,
                    estado,
                    duracao_ms = resumo.tempos.total_ms,
                    motivo = %motivo_para_log(resumo.erro.as_deref().unwrap_or("")),
                    "turno falhou"
                ),
            }
        }
        Err(erro) => tracing::warn!(
            target: "poph_agent::turno",
            org = %tarefa.org,
            conversa = %tarefa.conversa,
            motivo = %motivo_para_log(&erro.to_string()),
            "turno não pôde começar"
        ),
    }
}

/// Motivo seguro para o log: corta em 160 caracteres e esconde palavras com `@` (identificador de
/// WhatsApp) ou com 7 dígitos ou mais (telefone, número de cliente).
fn motivo_para_log(texto: &str) -> String {
    let limpo = texto
        .split_whitespace()
        .map(|palavra| {
            let digitos = palavra.chars().filter(char::is_ascii_digit).count();
            if palavra.contains('@') || digitos >= 7 {
                "[oculto]"
            } else {
                palavra
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    limpo.chars().take(160).collect()
}

/// Conclui os itens da `inbox` que o turno atendeu: o do evento e os das mensagens agrupadas.
fn concluir_entradas(
    fluxo: &Fluxo,
    org: OrgId,
    conversa: ConversationId,
    propria: Option<poph_types::InboxId>,
    mensagens: &[MessageId],
    erro: Option<&str>,
) -> Resultado<()> {
    let pendentes = fluxo.store().inbox().list_opens(org, LOTE_CONCLUSAO)?;
    let mut concluidos = Vec::new();
    for item in pendentes {
        let da_propria = Some(item.id) == propria;
        let do_grupo = mensagens.iter().any(|mensagem| {
            EventoEntrada::de_json(&item.event_json)
                .map(|evento| evento.conversa == conversa && evento.mensagem == *mensagem)
                .unwrap_or(false)
        });
        if da_propria || do_grupo {
            concluidos.push(item.id);
        }
    }
    for id in concluidos {
        fluxo.concluir_entrada(org, id, erro)?;
    }
    Ok(())
}

/// Por que o turno terminou sem resposta. Decide o destino do item da `inbox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClasseFalha {
    /// Falha que não se resolve repetindo (configuração local, pedido inválido, cancelamento).
    /// O item é concluído sem resposta.
    Definitiva,
    /// O provedor recusou o pedido (chave recusada, modelo inexistente, pedido rejeitado): não é
    /// repetido. O item é concluído sem resposta e a operação é avisada na hora (HR-26k).
    ModeloRecusado,
    /// Falta de vaga, do turno (instância, org, agente) ou do provedor (limite de simultâneos).
    /// Nenhuma chamada saiu, então entra na escada da falha do modelo (HR-26k).
    SemVaga,
    /// O modelo não respondeu (provedor fora do ar, conexão, tempo máximo): adiado, com limite.
    Modelo,
}

/// Classe da falha de um erro do provedor. Erro de configuração ou de pedido não se resolve
/// sozinho, e cancelamento é decisão de quem chamou: esses não são repetidos. Recusa do provedor
/// (chave, modelo, pedido) também não se resolve repetindo.
fn classe_do_erro_llm(erro: &LlmError) -> ClasseFalha {
    match erro {
        LlmError::Configuracao(_) | LlmError::InvalidRequest(_) | LlmError::Cancelled => {
            ClasseFalha::Definitiva
        }
        LlmError::Rejected { .. } => ClasseFalha::ModeloRecusado,
        _ => ClasseFalha::Modelo,
    }
}

/// Falta de vaga de chamada ao provedor (HR-26k). Nenhuma chamada chegou a sair, então vira falha
/// de indisponibilidade (entra na escada). Os outros erros do limitador seguem como configuração.
fn erro_de_vaga_do_provedor(erro: ErroAgente) -> LlmError {
    match erro {
        ErroAgente::SemVagaProvedor {
            provedor,
            espera_ms,
        } => LlmError::Unavailable {
            tentativas: 0,
            ultimo: format!("sem vaga de chamada ao provedor `{provedor}` em {espera_ms} ms"),
        },
        outro => LlmError::Configuracao(outro.to_string()),
    }
}

/// Itens da `inbox` que este turno atende: o próprio e os do grupo (mesma regra de concluir).
fn itens_do_turno(
    fluxo: &Fluxo,
    org: OrgId,
    conversa: ConversationId,
    propria: Option<poph_types::InboxId>,
    mensagens: &[MessageId],
) -> Resultado<Vec<poph_types::InboxId>> {
    let abertos = fluxo.store().inbox().list_opens(org, LOTE_CONCLUSAO)?;
    Ok(abertos
        .into_iter()
        .filter(|item| {
            Some(item.id) == propria
                || mensagens.iter().any(|mensagem| {
                    EventoEntrada::de_json(&item.event_json)
                        .map(|evento| evento.conversa == conversa && evento.mensagem == *mensagem)
                        .unwrap_or(false)
                })
        })
        .map(|item| item.id)
        .collect())
}

/// Falha do modelo no turno (HR-26i): repete depois, com limite, sem duplicar nada que já saiu.
///
/// - Se o turno já enfileirou saída para o cliente (ex.: um arquivo da biblioteca antes da falha),
///   não repete: repetir mandaria de novo. Conclui sem resposta, com o motivo.
/// - Se ainda há tentativa, os itens ficam na `inbox` com o instante da próxima (espera crescente).
/// - Se o limite acabou, os itens são concluídos com o motivo e o turno fica como falha definitiva.
///
/// A repetição também não duplica resposta: antes de chamar o modelo, o turno refaz a análise do
/// grupo, e se o agente já respondeu depois da mensagem, conclui sem chamar o modelo.
fn tratar_falha_modelo(
    metricas: &MetricasTurno,
    fluxo: &Fluxo,
    tarefa: &Tarefa,
    turno: TurnId,
    mensagens: &[MessageId],
    erro: &str,
) -> Resultado<()> {
    let org = tarefa.org;
    let motivo = motivo_para_log(erro);
    let ids = itens_do_turno(fluxo, org, tarefa.conversa, tarefa.entrada, mensagens)?;
    let ja_enfileirou = fluxo.store().outbox().count_turn(org, turno)? > 0;
    let politica = fluxo.opcoes();
    let maximo = politica.tentativas_falha_modelo.max(1);

    if ids.is_empty() {
        tracing::warn!(
            target: "poph_agent::turno",
            org = %org,
            conversa = %tarefa.conversa,
            turno = %turno,
            motivo = %motivo,
            "falha do modelo sem item na fila para repetir"
        );
        return Ok(());
    }
    if ja_enfileirou {
        for id in &ids {
            fluxo.concluir_entrada(org, *id, Some(motivo.as_str()))?;
        }
        metricas.falhas_modelo_definitivas.increment(1);
        tracing::warn!(
            target: "poph_agent::turno",
            org = %org,
            conversa = %tarefa.conversa,
            turno = %turno,
            motivo = %motivo,
            "falha do modelo depois de saída já enfileirada: não repetido para não duplicar"
        );
        return Ok(());
    }

    let tentativas = fluxo
        .store()
        .inbox()
        .attempts(org, &ids)?
        .into_iter()
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    if tentativas >= maximo {
        for id in &ids {
            fluxo.concluir_entrada(org, *id, Some(motivo.as_str()))?;
        }
        metricas.falhas_modelo_definitivas.increment(1);
        tracing::warn!(
            target: "poph_agent::turno",
            org = %org,
            conversa = %tarefa.conversa,
            turno = %turno,
            tentativas,
            motivo = %motivo,
            "falha do modelo: tentativas esgotadas; mensagem concluída sem resposta"
        );
        return Ok(());
    }

    let espera = politica.atraso_falha_modelo.espera(tentativas);
    let proxima_em = fluxo
        .store()
        .now()
        .saturating_add(i64::try_from(espera.as_millis()).unwrap_or(i64::MAX));
    fluxo.adiar_entradas(org, &ids, proxima_em, &motivo)?;
    metricas.falhas_modelo_adiadas.increment(1);
    tracing::warn!(
        target: "poph_agent::turno",
        org = %org,
        conversa = %tarefa.conversa,
        turno = %turno,
        tentativa = tentativas,
        de = maximo,
        espera_s = espera.as_secs(),
        motivo = %motivo,
        "falha do modelo: mensagem adiada para nova tentativa"
    );
    Ok(())
}

// ───────────────────────────── Análise do grupo ─────────────────────────────

/// O que fazer com a mensagem que disparou o turno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Situacao {
    /// Há mensagens do cliente ainda sem resposta: viram um turno.
    Processar,
    /// A conversa não tem mensagem nenhuma.
    SemNovidade,
    /// A mensagem que disparou o turno já tem resposta depois dela.
    JaRespondida,
}

/// Separa o grupo de mensagens do cliente que ainda não foram respondidas.
///
/// O grupo é a cauda de mensagens de entrada (`user`) depois da última fala do agente. Se a
/// mensagem que disparou o turno não estiver nessa cauda, ela já foi respondida — é o caso da
/// reentrega de webhook e da retomada (HR-12).
fn analisar(ultimas: &[MensagemGravada], gatilho: MessageId) -> (Situacao, Vec<MensagemGravada>) {
    if ultimas.is_empty() {
        return (Situacao::SemNovidade, Vec::new());
    }
    let mut grupo = Vec::new();
    for mensagem in ultimas.iter().rev() {
        if mensagem.role == Role::User && mensagem.direction == Direction::Inbound {
            grupo.push(mensagem.clone());
        } else {
            break;
        }
    }
    if grupo.is_empty() {
        return (Situacao::JaRespondida, Vec::new());
    }
    grupo.reverse();
    if grupo.iter().any(|mensagem| mensagem.id == gatilho) {
        (Situacao::Processar, grupo)
    } else {
        (Situacao::JaRespondida, Vec::new())
    }
}

// ───────────────────────────── Auxiliares ─────────────────────────────

/// Como uma chamada de ferramenta terminou, com o texto que vai para o modelo.
///
/// Existe para o turno gravar a diferença entre **executada** e **recusada** (HR-47b), sem mudar o
/// que o modelo recebe: nos dois casos ele lê o mesmo `Mensagem::Resultado`.
#[derive(Debug)]
enum DesfechoFerramenta {
    /// Rodou e devolveu este texto.
    Executada(String),
    /// Recusada antes (ou durante) a execução, por este motivo.
    Recusada {
        /// Um de `MOTIVO_*` de [`ferramentas`].
        motivo: &'static str,
        /// Texto de erro devolvido ao modelo.
        texto: String,
    },
}

impl DesfechoFerramenta {
    /// Recusa com motivo.
    fn recusada(motivo: &'static str, texto: String) -> Self {
        Self::Recusada { motivo, texto }
    }

    /// Texto devolvido ao modelo.
    fn texto(&self) -> &str {
        match self {
            Self::Executada(texto) => texto,
            Self::Recusada { texto, .. } => texto,
        }
    }

    /// O desfecho no vocabulário do registro.
    fn para_registro(&self) -> Desfecho {
        match self {
            Self::Executada(_) => Desfecho::Executada,
            Self::Recusada { motivo, .. } => Desfecho::Recusada(motivo),
        }
    }

    /// Texto devolvido ao modelo, consumindo o desfecho.
    fn em_texto(self) -> String {
        match self {
            Self::Executada(texto) => texto,
            Self::Recusada { texto, .. } => texto,
        }
    }
}

/// Motivo de recusa de uma ferramenta que devolveu `Err` ao modelo.
///
/// Sem tipo próprio de erro no contrato das ferramentas, a classificação vem da convenção que todas
/// seguem: erro de argumento começa por "erro: o argumento…"/"erro: os argumentos…" ou diz que o id
/// "não é um id … válido". O resto é falha de execução.
fn motivo_do_erro_da_ferramenta(erro: &str) -> &'static str {
    let procura = erro.to_lowercase();
    if procura.contains("argumento") || procura.contains("não é um id") {
        MOTIVO_ARGUMENTO_INVALIDO
    } else {
        MOTIVO_ERRO_DE_EXECUCAO
    }
}

/// Resposta acumulada das chamadas ao modelo.
#[derive(Debug, Default)]
struct Resposta {
    texto: String,
    uso: Usage,
    motivo: Option<FinishReason>,
    primeiro_texto_ms: Option<u64>,
    rodadas: u32,
    ferramentas: Vec<String>,
}

/// Resposta reescrita pelo G6 (HR-34), com o que ela custou.
#[derive(Debug, Default)]
struct RespostaRegenerada {
    /// Texto novo do modelo.
    texto: String,
    /// Tokens da chamada de regeneração.
    uso: Usage,
    /// Motivo do fim informado pelo provedor.
    motivo: Option<FinishReason>,
}

/// Soma o resultado de uma pergunta avulsa (G6) ao registro do turno.
///
/// A chamada da revisão é fora do lote: perguntas, tempo e tokens entram no mesmo `jev_json`, como
/// no G5. Sem resultado anterior, o da revisão passa a ser o registro.
fn somar_resultado_jev(jev: &mut Option<ResultadoJev>, resultado: ResultadoJev) {
    match jev {
        Some(atual) => {
            atual.decisoes.extend(resultado.decisoes);
            atual.brutas.extend(resultado.brutas);
            atual.chamada.perguntas += resultado.chamada.perguntas;
            atual.chamada.decididas_por_cache += resultado.chamada.decididas_por_cache;
            atual.chamada.ms = atual.chamada.ms.saturating_add(resultado.chamada.ms);
            atual.chamada.tokens = atual
                .chamada
                .tokens
                .saturating_add(resultado.chamada.tokens);
            atual.chamada.usage = atual.chamada.usage.somar(resultado.chamada.usage);
            if atual.chamada.falha.is_none() {
                atual.chamada.falha = resultado.chamada.falha;
            }
        }
        None => *jev = Some(resultado),
    }
}

fn somar_uso(total: Usage, outro: Usage) -> Usage {
    Usage {
        entrada: total.entrada.saturating_add(outro.entrada),
        saida: total.saida.saturating_add(outro.saida),
    }
}

fn gatilho_fila(agora: i64, chegada: i64) -> u64 {
    u64::try_from(agora.saturating_sub(chegada)).unwrap_or(0)
}

/// Última mensagem do cliente gravada na conversa.
///
/// É o que o Jev julga e o que identifica o estado da janela: se este id mudou durante a espera, o
/// lote feito na chegada ficou velho e precisa ser refeito (HR-30).
fn ultima_entrada_do_cliente(ultimas: &[MensagemGravada]) -> Option<MessageId> {
    ultimas
        .iter()
        .rev()
        .find(|mensagem| mensagem.role == Role::User && mensagem.direction == Direction::Inbound)
        .map(|mensagem| mensagem.id)
}

fn status_do_atendimento(status: poph_store::ConversationStatus) -> &'static str {
    match status {
        poph_store::ConversationStatus::Open => "aberta",
        poph_store::ConversationStatus::Transferred => "transferida para humano",
        poph_store::ConversationStatus::Closed => "fechada",
    }
}

fn motivo_texto(motivo: &FinishReason) -> String {
    match motivo {
        FinishReason::Concluido => "concluido".to_owned(),
        FinishReason::ChamadasFerramenta => "chamadas_ferramenta".to_owned(),
        FinishReason::LimiteDeTokens => "limite_de_tokens".to_owned(),
        FinishReason::Outro(texto) => texto.clone(),
    }
}

/// Teto do texto que vira consulta da base: o modelo truncaria em 128 tokens de qualquer forma, e
/// três mensagens seguidas já bastam para dizer o assunto.
const CONSULTA_MAX_CARACTERES: usize = 2000;

/// Texto da busca: as mensagens do cliente que entraram no turno, na ordem, até o teto.
/// Converte o produto da busca no candidato que o Jev julga (HR-33).
///
/// O `id` é a opção enviada e a chave validada de volta; o `detalhe` leva o que ajuda o Jev a
/// reconhecer o produto na fala do cliente (categoria, preço, unidade), sem despejar o cadastro
/// inteiro no pedido.
fn candidato_do_g5(produto: &ProdutoEncontrado) -> poph_jev::CandidatoProduto {
    let mut partes: Vec<String> = Vec::new();
    if !produto.categoria.trim().is_empty() {
        partes.push(format!("categoria: {}", produto.categoria.trim()));
    }
    if !produto.preco_texto.trim().is_empty() {
        partes.push(format!("preço: {}", produto.preco_texto.trim()));
    }
    if !produto.unidade.trim().is_empty() {
        partes.push(format!("unidade: {}", produto.unidade.trim()));
    }
    poph_jev::CandidatoProduto {
        id: produto.produto.to_string(),
        nome: produto.nome.clone(),
        detalhe: partes.join("; "),
    }
}

fn consulta_do_grupo(grupo: &[MensagemGravada]) -> String {
    let mut texto = String::new();
    for mensagem in grupo {
        if mensagem.role != Role::User {
            continue;
        }
        let fala = mensagem.texto.trim();
        if fala.is_empty() {
            continue;
        }
        if !texto.is_empty() {
            texto.push('\n');
        }
        texto.push_str(fala);
        if texto.chars().count() >= CONSULTA_MAX_CARACTERES {
            break;
        }
    }
    texto.chars().take(CONSULTA_MAX_CARACTERES).collect()
}

/// JSON do que a resposta usou (vai para `turns.retrieved_json`, que o Console mostra).
///
/// Uma lista só, com discriminante `tipo`: `trecho` (base de conhecimento, HR-17) e `produto`
/// (catálogo, HR-18 — com o preço aplicado). Sem uso nenhum, é `[]`, como antes.
fn utilizados_json(trechos: &[TrechoRecuperado], produtos: &[ProdutoUsado]) -> String {
    let mut itens: Vec<serde_json::Value> = Vec::with_capacity(trechos.len() + produtos.len());
    for trecho in trechos {
        let mut valor = serde_json::to_value(trecho).unwrap_or(serde_json::Value::Null);
        if let Some(objeto) = valor.as_object_mut() {
            objeto.insert("tipo".to_owned(), json!("trecho"));
        }
        itens.push(valor);
    }
    for produto in produtos {
        let mut valor = serde_json::to_value(produto).unwrap_or(serde_json::Value::Null);
        if let Some(objeto) = valor.as_object_mut() {
            objeto.insert("tipo".to_owned(), json!("produto"));
        }
        itens.push(valor);
    }
    serde_json::to_string(&itens).unwrap_or_else(|_| "[]".to_owned())
}

/// `true` para as ferramentas que consultam base de conhecimento ou catálogo (HR-31).
fn eh_base_ou_produto(nome: &str) -> bool {
    nome == crate::base_conhecimento::NOME_FERRAMENTA || nome == crate::produtos::NOME_FERRAMENTA
}

/// JSON dos tempos por etapa. `modelo` e `motivo_fim` viajam aqui porque `turns` não tem coluna
/// própria para eles e a HR-04b fechou o esquema (migração nova exige autorização do mantenedor).
fn tempos_json(tempos: &Tempos, modelo: &str, motivo: Option<&FinishReason>) -> String {
    json!({
        "fila_ms": tempos.fila_ms,
        "montagem_ms": tempos.montagem_ms,
        "primeiro_texto_ms": tempos.primeiro_texto_ms,
        "total_ms": tempos.total_ms,
        "rodadas_ferramenta": tempos.rodadas_ferramenta,
        "modelo": modelo,
        "motivo_fim": motivo.map(motivo_texto),
    })
    .to_string()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn mensagem(papel: Role, direcao: Direction, id: MessageId) -> MensagemGravada {
        MensagemGravada {
            id,
            org_id: OrgId::generate(),
            conversation_id: ConversationId::generate(),
            direction: direcao,
            role: papel,
            origin: poph_store::Origin::Agente,
            author_name: None,
            texto: String::new(),
            attachments_json: "[]".to_owned(),
            external_id: None,
            turn_id: None,
            created_at: 0,
        }
    }

    #[test]
    fn grupo_e_a_cauda_de_mensagens_do_cliente() {
        let (a, b, c) = (
            MessageId::generate(),
            MessageId::generate(),
            MessageId::generate(),
        );
        let ultimas = vec![
            mensagem(Role::User, Direction::Inbound, a),
            mensagem(Role::Assistant, Direction::Outbound, b),
            mensagem(Role::User, Direction::Inbound, c),
        ];
        let (situacao, grupo) = analisar(&ultimas, c);
        assert_eq!(situacao, Situacao::Processar);
        assert_eq!(grupo.len(), 1);
        assert_eq!(grupo[0].id, c);
    }

    #[test]
    fn mensagem_ja_respondida_nao_abre_turno() {
        let (a, b) = (MessageId::generate(), MessageId::generate());
        let ultimas = vec![
            mensagem(Role::User, Direction::Inbound, a),
            mensagem(Role::Assistant, Direction::Outbound, b),
        ];
        let (situacao, grupo) = analisar(&ultimas, a);
        assert_eq!(situacao, Situacao::JaRespondida);
        assert!(grupo.is_empty());
    }

    #[test]
    fn conversa_vazia_nao_tem_novidade() {
        let (situacao, grupo) = analisar(&[], MessageId::generate());
        assert_eq!(situacao, Situacao::SemNovidade);
        assert!(grupo.is_empty());
    }
}
