//! O fluxo do atendimento dentro do turno (FX-23).
//!
//! O ciclo que este módulo executa, **antes** de o modelo de texto ser chamado (PRD §6.3):
//!
//! 1. **Motor desligado** (`[fluxos] ativo = false`): nada aqui acontece. A Ficha continua sendo
//!    criada e fechada pelo `poph-store`, e o turno é o de sempre.
//! 2. **Execução ativa na Ficha**: o bloco do prompt da etapa atual ([`poph_fluxos::renderizar_bloco`])
//!    entra na parte **volátil** do prompt. Não se pergunta nada ao Jev: já há fluxo em andamento
//!    (PRD L6: uma execução ativa por Ficha).
//! 3. **Sem execução ativa e com fluxo liberado ao agente**: o estado enviado ao Jev leva os fluxos
//!    (nome e gatilho) e o gate G9 escolhe um. Escolha confiável → o **código** valida (fluxo da org,
//!    ativo, liberado ao agente da Ficha, revisão publicada) e chama
//!    [`poph_store::Store::execucoes`] (`iniciar_execucao`, FX-20), que grava a execução, a primeira
//!    etapa, o evento `fluxo_iniciado` com a decisão do Jev e os efeitos do `ao_entrar` numa escrita
//!    só. O prompt nasce com o bloco da etapa de entrada.
//! 4. **Reserva** (Jev fora do ar, gate desligado, resposta inutilizável ou escolha recusada na
//!    validação): nada inicia e o prompt recebe o catálogo (`id`, nome, `quando`) para o modelo
//!    escolher — a ferramenta `fluxo_iniciar` é da FX-24, que ainda não existe; sem ela o catálogo
//!    vai como **informação** e o turno é registrado como reserva no `turns.jev_json`.
//! 5. **`nenhum` do Jev ou confiança baixa**: nada inicia e o catálogo **não** vai ao prompt.
//!
//! Falha de banco ou de leitura dentro daqui nunca derruba o atendimento: o turno segue sem fluxo,
//! como antes da FX-23, e o motivo fica no log.
//!
//! O repositório (`poph-store`) não compila nem interpreta fluxo; o motor (`poph-fluxos`) não
//! conhece banco. A tradução entre os dois é [`crate::fluxos_transicao`].

use std::sync::Arc;

use poph_fluxos::Limites;
use poph_fluxos::cache::{CacheRevisoes, ChaveRevisao};
use poph_fluxos::ir::Ir;
use poph_fluxos::motor::{self, Contexto as ContextoDoMotor, Estado as EstadoDoMotor};
use poph_fluxos::prompt::renderizar_bloco;
use poph_fluxos::regras::{Avaliador, Dados, LimitesRegra};
use poph_jev::{FluxoJev, G9_NENHUM, IdGate, ResultadoJev};
use poph_store::{
    DecisaoDoJev, EfeitoDaTransicao, ErroStore, Execucao, Ficha, Fluxo, InicioDeExecucao,
    RevisaoDeFluxo, Status, Store,
};
use poph_types::{AgentId, AtorAcao, AtorEvento, ConversationId, DomainError, FluxosConfig, OrgId};
use serde_json::{Value, json};

use crate::fluxos_transicao::{efeito_da_transicao, estado_para_banco};

/// Nome da ferramenta que o modelo usa para iniciar um fluxo na reserva (FX-24).
pub const NOME_FERRAMENTA_INICIAR: &str = "fluxo_iniciar";

/// Cabeçalho do bloco dos fluxos no prompt (dado nosso, do script do cliente).
pub const ABRE_FLUXOS: &str = "<fluxos_de_atendimento>";
/// Fecha o bloco dos fluxos.
pub const FECHA_FLUXOS: &str = "</fluxos_de_atendimento>";

/// Nota que acompanha o bloco da etapa quando a transição enfileirou efeito para o cliente.
///
/// O efeito sai pela fila do servidor, **depois** do texto final do turno (FX-21). Dizer isso ao
/// modelo evita a mensagem duplicada: ele não precisa repetir o que o fluxo já vai mandar.
pub(crate) const NOTA_DE_EFEITO: &str =
    "O fluxo agendou uma mensagem própria, que sai depois da sua resposta: não a repita.";

/// Acrescenta a nota do efeito ao texto do bloco, quando houve efeito na transição.
pub(crate) fn aplicar_nota_de_efeito(texto: String, efeitos: usize) -> String {
    if efeitos == 0 {
        return texto;
    }
    format!("{texto}\n{NOTA_DE_EFEITO}")
}

/// Ajustes do motor de fluxos no turno, lidos de `[fluxos]` na composição.
///
/// Sem eles (`Motor::com_fluxos` não chamado), o turno não toca em fluxo nenhum: é o que mantém os
/// testes antigos idênticos byte a byte.
#[derive(Clone)]
pub struct AjustesDoFluxo {
    ativo: bool,
    limites: Limites,
    /// Teto de decisões encadeadas numa passagem (`fluxos.max_saltos_decisao`).
    saltos: usize,
    cache: Arc<CacheRevisoes>,
}

impl AjustesDoFluxo {
    /// Ajustes a partir da configuração `[fluxos]`: os tetos de compilação e o cache de revisões.
    pub fn da_config(config: &FluxosConfig) -> Self {
        Self {
            ativo: config.ativo,
            limites: Limites::da_config(config),
            saltos: usize::try_from(config.max_saltos_decisao)
                .unwrap_or(1)
                .max(1),
            cache: Arc::new(CacheRevisoes::novo(
                usize::try_from(config.cache_revisoes_mb).unwrap_or(usize::MAX),
            )),
        }
    }

    /// O motor de fluxos está ligado nesta instância?
    pub fn ativo(&self) -> bool {
        self.ativo
    }

    /// Teto de decisões encadeadas numa passagem (`fluxos.max_saltos_decisao`).
    pub(crate) fn saltos(&self) -> usize {
        self.saltos
    }

    /// Compartilha o cache de revisões de quem já tem um (o servidor tem o da API).
    pub fn com_cache(mut self, cache: Arc<CacheRevisoes>) -> Self {
        self.cache = cache;
        self
    }
}

/// O que o turno leu antes do lote do Jev sobre o fluxo deste atendimento.
///
/// É a **fotografia** do começo do turno: a validação definitiva acontece na escrita
/// (`iniciar_execucao`), que confere tudo de novo dentro da transação.
pub(crate) struct Preparo {
    /// Ficha do atendimento (uma por conversa, invariante do `poph-store`).
    pub ficha: Ficha,
    /// Execução ativa no instante da leitura.
    pub ativa: Option<Execucao>,
    /// Fluxos liberados ao agente, ativos e com revisão publicada, com o gatilho lido da IR.
    pub liberados: Vec<FluxoJev>,
}

/// O que fazer com o fluxo neste turno, já decidido.
#[derive(Default)]
pub(crate) struct DesfechoDoFluxo {
    /// Texto que entra no bloco volátil do prompt (bloco da etapa atual ou catálogo da reserva).
    pub texto: Option<String>,
    /// Registro para `turns.jev_json` (o painel "por quê").
    pub registro: Option<Value>,
    /// Há execução ativa na Ficha neste turno: as ferramentas de ação e de abandono valem (FX-24).
    pub em_andamento: bool,
    /// O catálogo foi ao prompt (reserva do Jev): `fluxo_iniciar` pode ser oferecida (FX-24).
    pub reserva: bool,
}

impl DesfechoDoFluxo {
    /// Registro de quem não teve nada a decidir.
    fn sem_fluxo() -> Self {
        Self::default()
    }

    /// Desfecho com execução em andamento (o bloco da etapa, ou o catálogo da recusa).
    fn em_andamento(mut self) -> Self {
        self.em_andamento = true;
        self
    }

    /// Desfecho de reserva: o catálogo vai ao prompt e o modelo pode abrir o fluxo (FX-24).
    fn reserva(mut self) -> Self {
        self.reserva = true;
        self
    }
}

/// Lê a Ficha, a execução ativa e os fluxos liberados do agente.
///
/// `None` quando o motor está desligado, quando a conversa não tem Ficha ou quando a leitura falha
/// (o turno segue sem fluxo). Os fluxos liberados só são lidos quando não há execução ativa: com uma
/// em andamento, nada mais é oferecido neste turno.
pub(crate) fn preparar(
    ajustes: &AjustesDoFluxo,
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    agente: AgentId,
) -> Option<Preparo> {
    if !ajustes.ativo {
        return None;
    }
    let ficha = match store.fichas().por_conversa(org, conversa) {
        Ok(Some(ficha)) => ficha,
        Ok(None) => {
            tracing::debug!(
                conversa = %conversa,
                "a conversa não tem Ficha: o turno segue sem fluxo neste turno"
            );
            return None;
        }
        Err(erro) => {
            tracing::warn!(erro = %erro, "não consegui ler a Ficha da conversa");
            return None;
        }
    };
    let ativa = match store.execucoes().ativa_da_ficha(org, ficha.id) {
        Ok(ativa) => ativa,
        Err(erro) => {
            tracing::warn!(erro = %erro, ficha = %ficha.id, "não consegui ler a execução ativa");
            return None;
        }
    };
    if ativa.is_some() {
        return Some(Preparo {
            ficha,
            ativa,
            liberados: Vec::new(),
        });
    }
    let fluxos = match store.fluxos().listar_do_agente(org, agente) {
        Ok(fluxos) => fluxos,
        Err(erro) => {
            tracing::warn!(erro = %erro, agente = %agente, "não consegui listar os fluxos do agente");
            return None;
        }
    };
    let mut liberados = Vec::new();
    for fluxo in fluxos {
        // Fluxo inativo não inicia execução nova (PRD §6.2) e fluxo sem publicação não tem etapas.
        if fluxo.status != Status::Ativo || fluxo.revisao_atual.is_none() {
            continue;
        }
        let Some(ir) = ir_vigente(ajustes, store, org, &fluxo) else {
            continue;
        };
        liberados.push(fluxo_do_jev(&fluxo, &ir));
    }
    Some(Preparo {
        ficha,
        ativa: None,
        liberados,
    })
}

/// O gatilho do fluxo como o G9 o lê (FX-22): id = `slug`, nome e os três textos do script.
fn fluxo_do_jev(fluxo: &Fluxo, ir: &Ir) -> FluxoJev {
    FluxoJev {
        id: fluxo.slug.clone(),
        nome: fluxo.nome.clone(),
        quando: ir.gatilho.quando.clone(),
        nao_quando: ir.gatilho.nao_quando.clone().unwrap_or_default(),
        exemplos: ir.gatilho.exemplos.clone(),
    }
}

/// A IR da revisão publicada que vale hoje para o fluxo, pelo cache de revisões (FX-19).
///
/// A carga recompila o texto e confere que ele produz a IR gravada: revisão que não confere **não**
/// entra (falha fechada, como no turno do PRD). Falha aqui só tira o fluxo das opções do G9.
fn ir_vigente(
    ajustes: &AjustesDoFluxo,
    store: &Store,
    org: OrgId,
    fluxo: &Fluxo,
) -> Option<Arc<Ir>> {
    let revisao = match store.revisoes().atual(org, fluxo.id) {
        Ok(Some(revisao)) => revisao,
        Ok(None) => return None,
        Err(erro) => {
            tracing::warn!(erro = %erro, fluxo = %fluxo.id, "não consegui ler a revisão atual");
            return None;
        }
    };
    carregar_ir(ajustes, fluxo, &revisao)
}

/// Carrega a IR de uma revisão pelo cache, conferindo-a contra o texto publicado.
fn carregar_ir(
    ajustes: &AjustesDoFluxo,
    fluxo: &Fluxo,
    revisao: &RevisaoDeFluxo,
) -> Option<Arc<Ir>> {
    let gravada = match Ir::de_json(&revisao.definicao_json) {
        Ok(ir) => ir,
        Err(erro) => {
            tracing::error!(
                erro = %erro,
                revisao = %revisao.id,
                "a IR gravada da revisão não é JSON legível"
            );
            return None;
        }
    };
    let chave = ChaveRevisao::com_hash(
        fluxo.slug.clone(),
        u32::try_from(revisao.numero).unwrap_or(u32::MAX),
        revisao.fonte_sha256.clone(),
    );
    match ajustes
        .cache
        .carregar(&chave, &gravada, &revisao.fonte, &ajustes.limites)
    {
        Ok(ir) => Some(ir),
        Err(falha) => {
            tracing::error!(
                falha = falha.codigo(),
                revisao = %revisao.id,
                "a revisão publicada não confere com o texto guardado"
            );
            None
        }
    }
}

/// O estado do motor reconstruído a partir da linha da execução e da IR da revisão dela.
///
/// O banco guarda o nó pelo id (FX-20); a IR diz se ele é etapa, decisão ou fim.
fn estado_da_execucao(
    ir: &Ir,
    execucao: &Execucao,
    atores_que_agiram: Vec<AtorAcao>,
) -> Option<EstadoDoMotor> {
    let dados: Dados = match serde_json::from_str(&execucao.dados_json) {
        Ok(dados) => dados,
        Err(erro) => {
            tracing::warn!(erro = %erro, execucao = %execucao.id, "dados da execução não leem");
            return None;
        }
    };
    let no_atual = if ir.etapas.iter().any(|etapa| etapa.id == execucao.no_atual) {
        poph_fluxos::ir::Destino::Etapa {
            id: execucao.no_atual.clone(),
        }
    } else if ir
        .decisoes
        .iter()
        .any(|decisao| decisao.id == execucao.no_atual)
    {
        poph_fluxos::ir::Destino::Decisao {
            id: execucao.no_atual.clone(),
        }
    } else if let Some(fim) = ir.fins.iter().find(|fim| fim.id == execucao.no_atual) {
        poph_fluxos::ir::Destino::Fim {
            id: fim.id.clone(),
            resultado: fim.resultado.clone(),
            encerra_atendimento: fim.encerra_atendimento,
        }
    } else {
        tracing::warn!(
            no = %execucao.no_atual,
            execucao = %execucao.id,
            "o nó da execução não existe na IR da revisão"
        );
        return None;
    };
    Some(EstadoDoMotor {
        fluxo_id: ir.fluxo.id.clone(),
        status: execucao.status,
        no_atual,
        dados,
        atores_que_agiram,
        resultado: execucao.resultado.clone(),
        motivo: execucao.motivo_fim.clone(),
        chaves: Default::default(),
    })
}

/// O bloco do prompt de uma execução: rende a etapa atual, os campos e as ações (FX-18).
///
/// `None` quando a IR da revisão dela não carrega ou os dados não leem — o prompt fica sem bloco em
/// vez de mentir sobre a etapa.
pub(crate) fn bloco_da_execucao(
    ajustes: &AjustesDoFluxo,
    store: &Store,
    org: OrgId,
    execucao: &Execucao,
) -> Option<(String, Arc<Ir>)> {
    let revisao = match store.revisoes().obter_por_id(org, execucao.revisao_id) {
        Ok(revisao) => revisao,
        Err(erro) => {
            tracing::warn!(erro = %erro, revisao = %execucao.revisao_id, "revisão da execução não lê");
            return None;
        }
    };
    let fluxo = match store.fluxos().obter(org, execucao.fluxo_id) {
        Ok(fluxo) => fluxo,
        Err(erro) => {
            tracing::warn!(erro = %erro, fluxo = %execucao.fluxo_id, "fluxo da execução não lê");
            return None;
        }
    };
    let ir = carregar_ir(ajustes, &fluxo, &revisao)?;
    // O bloco do prompt diz quem falta agir pela IR e pelos dados (`atores_exigidos` + `dados`):
    // a coluna não guarda os atores da etapa e a lista vazia é honesta para ele.
    let estado = estado_da_execucao(&ir, execucao, Vec::new())?;
    Some((renderizar_bloco(&ir, &estado), ir))
}

/// O que a ferramenta do agente precisa para agir numa execução (FX-24).
pub(crate) struct CargaDaAcao {
    /// IR da revisão que a execução segue.
    pub ir: Arc<Ir>,
    /// Avaliador das regras daquela revisão (as `def` do script, quando existem).
    pub avaliador: Avaliador,
    /// Estado da execução como está no banco, com os atores que já agiram na etapa.
    pub estado: EstadoDoMotor,
}

/// Lê a IR, o avaliador e o estado de uma execução para o motor agir (`fluxo_executar_acao`).
///
/// Diferente do bloco do prompt, aqui a lista de atores que já agiram **é** a do banco: é ela que o
/// critério `auto` usa para decidir se a etapa avança depois da ação (FX-13). Sem isso, o agente
/// agindo depois do atendente nunca completaria a etapa.
pub(crate) fn carga_para_agir(
    ajustes: &AjustesDoFluxo,
    store: &Store,
    org: OrgId,
    execucao: &Execucao,
) -> Option<CargaDaAcao> {
    let revisao = match store.revisoes().obter_por_id(org, execucao.revisao_id) {
        Ok(revisao) => revisao,
        Err(erro) => {
            tracing::warn!(erro = %erro, revisao = %execucao.revisao_id, "revisão da execução não lê");
            return None;
        }
    };
    let fluxo = match store.fluxos().obter(org, execucao.fluxo_id) {
        Ok(fluxo) => fluxo,
        Err(erro) => {
            tracing::warn!(erro = %erro, fluxo = %execucao.fluxo_id, "fluxo da execução não lê");
            return None;
        }
    };
    let ir = carregar_ir(ajustes, &fluxo, &revisao)?;
    let avaliador = avaliador_da_revisao(&revisao, &ir, &ajustes.limites)?;
    let atores = match store.execucoes().etapas(org, execucao.id) {
        Ok(etapas) => etapas
            .iter()
            .rev()
            .find(|etapa| etapa.no_id == execucao.no_atual)
            .map(|etapa| etapa.atores_que_agiram.clone())
            .unwrap_or_default(),
        Err(erro) => {
            tracing::warn!(erro = %erro, execucao = %execucao.id, "etapas da execução não leem");
            return None;
        }
    };
    let estado = estado_da_execucao(&ir, execucao, atores)?;
    Some(CargaDaAcao {
        ir,
        avaliador,
        estado,
    })
}

/// Avaliador das regras de uma revisão: sem `def` no script, sem função nenhuma a oferecer.
fn avaliador_da_revisao(revisao: &RevisaoDeFluxo, ir: &Ir, limites: &Limites) -> Option<Avaliador> {
    if ir.defs.is_empty() {
        return Some(Avaliador::sem_funcoes());
    }
    match Avaliador::novo_com(&revisao.fonte, limites, LimitesRegra::default()) {
        Ok(avaliador) => Some(avaliador),
        Err(erro) => {
            tracing::error!(
                erro = %erro,
                revisao = %revisao.id,
                "as regras da revisão não carregam"
            );
            None
        }
    }
}

/// O que a tentativa de iniciar um fluxo produziu.
pub(crate) enum Inicio {
    /// Execução aberta (ou já aberta pela mesma chave), com quantos efeitos a entrada enfileirou.
    Aberta {
        /// Execução gravada.
        execucao: Box<Execucao>,
        /// Efeitos do `ao_entrar` da etapa de entrada (0 quando não há).
        efeitos: usize,
    },
    /// A Ficha já tinha execução ativa (corrida com o atendente, por exemplo).
    JaAtiva,
    /// A validação recusou, com o motivo pronto para o registro.
    Recusado(String),
}

/// Quem pediu a abertura: o G9 do Jev (turno) ou o agente pela ferramenta `fluxo_iniciar` (FX-24).
///
/// Muda só a assinatura do evento `fluxo_iniciado` — a validação é a **mesma** nos dois caminhos,
/// que é o que o bloco da FX-24 cobra ("valida como o G9").
pub(crate) enum QuemInicia<'a> {
    /// Escolha confiável do gate G9, com a decisão dele no evento.
    Jev(DecisaoDoJev<'a>),
    /// O modelo, pela ferramenta de reserva (sem decisão de Jev).
    Agente,
}

impl QuemInicia<'_> {
    /// Ator do evento `fluxo_iniciado` e a referência dele (o gate, quando foi o Jev).
    fn ator(&self) -> (AtorEvento, Option<&'static str>) {
        match self {
            Self::Jev(_) => (AtorEvento::Jev, Some("g9")),
            Self::Agente => (AtorEvento::Agente, None),
        }
    }

    /// A decisão do Jev que vai no evento, quando houve.
    fn decisao(&self) -> Option<DecisaoDoJev<'_>> {
        match self {
            Self::Jev(decisao) => Some(decisao.clone()),
            Self::Agente => None,
        }
    }
}

/// Valida a escolha e abre a execução na Ficha.
///
/// A ordem é a do PRD §6.3-2: fluxo da org, ativo, liberado ao agente **da Ficha**, com revisão
/// publicada e IR que confere. A escrita (`iniciar_execucao`) repete as conferências dentro da
/// transação — aqui elas existem para o motivo ficar claro antes de qualquer gravação.
///
/// `chave_idempotencia` só é usada quando quem abre é o agente; o G9 abre sem ela e a garantia
/// contra duplicar é a execução ativa única por Ficha (PRD L6).
pub(crate) fn iniciar(
    ajustes: &AjustesDoFluxo,
    store: &Store,
    org: OrgId,
    ficha: &Ficha,
    slug: &str,
    quem: QuemInicia<'_>,
    chave_idempotencia: Option<&str>,
) -> Result<Inicio, ErroStore> {
    let fluxo = match store.fluxos().por_slug(org, slug) {
        Ok(Some(fluxo)) => fluxo,
        Ok(None) => {
            return Ok(Inicio::Recusado(format!(
                "o fluxo `{slug}` não existe na organização"
            )));
        }
        Err(erro) => return Err(erro),
    };
    if fluxo.status != Status::Ativo {
        return Ok(Inicio::Recusado(format!(
            "o fluxo `{slug}` está inativo e não inicia execução nova"
        )));
    }
    let liberados = store.fluxos().listar_do_agente(org, ficha.agent_id)?;
    if !liberados.iter().any(|liberado| liberado.id == fluxo.id) {
        return Ok(Inicio::Recusado(format!(
            "o fluxo `{slug}` não está liberado para o agente deste atendimento"
        )));
    }
    let revisao = match store.revisoes().atual(org, fluxo.id)? {
        Some(revisao) => revisao,
        None => {
            return Ok(Inicio::Recusado(format!(
                "o fluxo `{slug}` não tem revisão publicada"
            )));
        }
    };
    let Some(ir) = carregar_ir(ajustes, &fluxo, &revisao) else {
        return Ok(Inicio::Recusado(format!(
            "a revisão publicada do fluxo `{slug}` não carrega"
        )));
    };
    let estado = match motor::iniciar(&ir) {
        Ok(estado) => estado,
        Err(erro) => {
            return Ok(Inicio::Recusado(format!(
                "o fluxo `{slug}` não tem etapa inicial válida: {erro}"
            )));
        }
    };
    // Efeitos do `ao_entrar` da etapa de entrada, com `{ficha.id}` e `{cliente.peer}` resolvidos.
    let avaliador = Avaliador::sem_funcoes();
    let texto_da_ficha = ficha.id.to_string();
    let mut contexto = ContextoDoMotor::novo(&avaliador)
        .com_ficha(&texto_da_ficha)
        .com_limite_de_saltos(ajustes.saltos);
    if let Some(peer) = ficha.peer_externo.as_deref() {
        contexto = contexto.com_cliente(peer);
    }
    // O tipo é explícito: o braço de erro diverge (`return`) e a inferência sem âncora escolheria
    // `!` ao ver o `.len()` logo abaixo (rust-lang/rust#156047).
    let efeitos: Vec<EfeitoDaTransicao> =
        match motor::efeitos_de_entrada(&ir, estado.no_atual.id(), &estado.dados, &contexto) {
            Ok(efeitos) => efeitos.iter().map(efeito_da_transicao).collect(),
            Err(erro) => {
                return Ok(Inicio::Recusado(format!(
                    "os efeitos da etapa de entrada do fluxo `{slug}` não resolvem: {erro}"
                )));
            }
        };
    // Quem decide é o código; a assinatura do evento diz se a escolha veio do Jev (G9) ou do
    // modelo (ferramenta de reserva, FX-24). Quem executa é este código nos dois casos (FX-20).
    let (ator, ator_ref) = quem.ator();
    let inicio_efeitos = efeitos.len();
    let inicio = InicioDeExecucao {
        ficha: ficha.id,
        fluxo: fluxo.id,
        revisao: revisao.id,
        ator,
        ator_ref,
        decisao_jev: quem.decisao(),
        estado_inicial: estado_para_banco(&estado),
        efeitos_de_entrada: efeitos,
        // Sem chave (G9), a garantia contra duplicar é a execução ativa única por Ficha (PRD L6),
        // que a escrita confere dentro da transação.
        chave_idempotencia,
    };
    let efeitos_na_entrada = inicio_efeitos;
    match store.execucoes().iniciar_execucao(org, inicio) {
        Ok(gravada) => Ok(Inicio::Aberta {
            execucao: Box::new(gravada.execucao),
            efeitos: efeitos_na_entrada,
        }),
        Err(ErroStore::Dominio(DomainError::ExecucaoJaAtiva { .. })) => Ok(Inicio::JaAtiva),
        Err(ErroStore::Dominio(outro)) => Ok(Inicio::Recusado(outro.to_string())),
        Err(erro) => Err(erro),
    }
}

/// Texto do bloco do prompt para os fluxos deste turno.
///
/// `fluxos` são os liberados ao agente; `ferramenta` diz se `fluxo_iniciar` está registrada nesta
/// instância (FX-24). Sem a ferramenta, o catálogo vai como **informação** — instruir o modelo a
/// chamar ferramenta inexistente só produz erro no laço.
pub(crate) fn bloco_do_catalogo(fluxos: &[FluxoJev], ferramenta: bool) -> Option<String> {
    if fluxos.is_empty() {
        return None;
    }
    let mut bloco = String::with_capacity(120 + fluxos.len() * 200);
    bloco.push_str(ABRE_FLUXOS);
    bloco.push('\n');
    if ferramenta {
        bloco.push_str(&format!(
            "Fluxos de atendimento liberados para você. Se a mensagem do cliente pedir um deles, \
             chame {NOME_FERRAMENTA_INICIAR} com o id do fluxo (um só) e só então responda.\n"
        ));
    } else {
        bloco.push_str(
            "Fluxos de atendimento disponíveis para esta empresa (informação; quem inicia um fluxo \
             é o sistema):\n",
        );
    }
    for fluxo in fluxos {
        let quando = fluxo.quando.trim();
        if quando.is_empty() {
            bloco.push_str(&format!("- {}: {}\n", fluxo.id, fluxo.nome));
        } else {
            bloco.push_str(&format!(
                "- {}: {} — quando: {}\n",
                fluxo.id,
                fluxo.nome,
                uma_linha(quando)
            ));
        }
    }
    bloco.push_str(FECHA_FLUXOS);
    Some(bloco)
}

/// O texto do cliente nunca abre linha nova dentro do bloco: quebra vira espaço.
fn uma_linha(texto: &str) -> String {
    texto.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Por que o G9 não iniciou nada, olhando o que o Jev respondeu.
///
/// Distingue as quatro séries da FX-26: `nenhum` (abstenção explícita), abaixo do limiar (o Jev
/// respondeu outra coisa sem confiança), falha da chamada e "não perguntado" (gate desligado, acima
/// do teto de 20 fluxos).
pub(crate) enum MotivoSemInicio {
    /// O Jev respondeu `nenhum`.
    Nenhum,
    /// O Jev respondeu um fluxo abaixo do limiar de confiança ou margem.
    AbaixoDoLimiar,
    /// A chamada do Jev falhou.
    Falhou,
    /// O gate não foi perguntado neste turno.
    NaoPerguntado,
}

/// Classifica a ausência de decisão confiável do G9.
///
/// `g9_ligado` separa as duas causas de "sem resposta": o gate estava fora (desligado, acima do teto
/// de 20 fluxos) ou o lote falhou. Sem essa distinção, uma falha do Jev na pergunta do G5 contaria
/// como falha do G9.
pub(crate) fn motivo_sem_inicio(jev: Option<&ResultadoJev>, g9_ligado: bool) -> MotivoSemInicio {
    let Some(jev) = jev else {
        return MotivoSemInicio::NaoPerguntado;
    };
    match jev.resposta(IdGate::G9) {
        Some(bruta) if bruta.escolha.as_deref() == Some(G9_NENHUM) => MotivoSemInicio::Nenhum,
        Some(_) => MotivoSemInicio::AbaixoDoLimiar,
        None if g9_ligado && jev.falhou() => MotivoSemInicio::Falhou,
        None => MotivoSemInicio::NaoPerguntado,
    }
}

/// Monta o registro do painel "por quê" (`turns.jev_json`), sem texto de cliente.
fn registro(resultado: &str) -> Value {
    json!({ "resultado": resultado })
}

/// Registro de um fluxo iniciado neste turno.
fn registro_iniciado(execucao: &Execucao, ir: &Ir) -> Value {
    json!({
        "resultado": "iniciou",
        "fluxo": ir.fluxo.id,
        "execucao": execucao.id.to_string(),
        "etapa": execucao.no_atual,
    })
}

/// Decide o fluxo do turno: bloco da execução ativa, início pela escolha do G9 ou reserva.
///
/// Tudo aqui é síncrono (leitura e escrita no banco): o turno já está no seu ator, com uma conversa
/// por vez. Nenhum erro sobe: o pior caso é o turno seguir sem fluxo.
pub(crate) fn decidir(
    ajustes: &AjustesDoFluxo,
    store: &Store,
    org: OrgId,
    preparo: Option<&Preparo>,
    jev: Option<&ResultadoJev>,
    g9_ligado: bool,
    ferramenta_de_inicio: bool,
) -> DesfechoDoFluxo {
    let Some(preparo) = preparo else {
        return DesfechoDoFluxo::sem_fluxo();
    };
    // 1. Execução ativa: o bloco dela entra no prompt e o Jev não é perguntado.
    if let Some(ativa) = preparo.ativa.as_ref() {
        return match bloco_da_execucao(ajustes, store, org, ativa) {
            Some((bloco, ir)) => DesfechoDoFluxo {
                texto: Some(bloco),
                registro: Some(json!({
                    "resultado": "execucao_ativa",
                    "fluxo": ir.fluxo.id,
                    "execucao": ativa.id.to_string(),
                    "etapa": ativa.no_atual,
                })),
                ..DesfechoDoFluxo::default()
            }
            .em_andamento(),
            None => DesfechoDoFluxo {
                registro: Some(json!({
                    "resultado": "execucao_ativa_sem_bloco",
                    "execucao": ativa.id.to_string(),
                })),
                ..DesfechoDoFluxo::default()
            }
            .em_andamento(),
        };
    }
    if preparo.liberados.is_empty() {
        return DesfechoDoFluxo::sem_fluxo();
    }

    // 2. Escolha confiável do G9: o código valida e inicia.
    if let Some(slug) = jev.and_then(|resultado| resultado.escolha_confiavel(IdGate::G9)) {
        let confianca = jev
            .and_then(|resultado| resultado.decisao(IdGate::G9))
            .map(|decisao| decisao.confianca);
        let decisao = DecisaoDoJev {
            escolha: slug,
            confianca,
        };
        return match iniciar(
            ajustes,
            store,
            org,
            &preparo.ficha,
            slug,
            QuemInicia::Jev(decisao),
            None,
        ) {
            Ok(Inicio::Aberta { execucao, efeitos }) => {
                let bloco = bloco_da_execucao(ajustes, store, org, &execucao);
                match bloco {
                    Some((texto, ir)) => DesfechoDoFluxo {
                        texto: Some(aplicar_nota_de_efeito(texto, efeitos)),
                        registro: Some(registro_iniciado(&execucao, &ir)),
                        ..DesfechoDoFluxo::default()
                    }
                    .em_andamento(),
                    None => DesfechoDoFluxo {
                        registro: Some(json!({
                            "resultado": "iniciou_sem_bloco",
                            "fluxo": slug,
                            "execucao": execucao.id.to_string(),
                        })),
                        ..DesfechoDoFluxo::default()
                    }
                    .em_andamento(),
                }
            }
            Ok(Inicio::JaAtiva) => {
                // Corrida com o atendente (ou com o turno anterior): a execução que está lá é a que
                // vale. O bloco dela entra no prompt, sem iniciar uma segunda.
                tracing::info!(
                    ficha = %preparo.ficha.id,
                    fluxo = slug,
                    "o G9 escolheu um fluxo, mas a Ficha já tinha execução ativa"
                );
                match store.execucoes().ativa_da_ficha(org, preparo.ficha.id) {
                    Ok(Some(ativa)) => {
                        let bloco = bloco_da_execucao(ajustes, store, org, &ativa);
                        DesfechoDoFluxo {
                            texto: bloco.map(|(texto, _)| texto),
                            registro: Some(json!({
                                "resultado": "execucao_ativa",
                                "fluxo": slug,
                                "execucao": ativa.id.to_string(),
                                "motivo": "a Ficha já tinha execução ativa quando o Jev escolheu",
                            })),
                            ..DesfechoDoFluxo::default()
                        }
                        .em_andamento()
                    }
                    _ => DesfechoDoFluxo {
                        registro: Some(json!({
                            "resultado": "nao_iniciou",
                            "fluxo": slug,
                            "motivo": "a Ficha já tinha execução ativa quando o Jev escolheu",
                        })),
                        ..DesfechoDoFluxo::default()
                    }
                    .em_andamento(),
                }
            }
            Ok(Inicio::Recusado(motivo)) => {
                tracing::warn!(
                    ficha = %preparo.ficha.id,
                    fluxo = slug,
                    motivo,
                    "o G9 escolheu um fluxo que a validação recusou; o catálogo vai ao prompt"
                );
                DesfechoDoFluxo {
                    texto: bloco_do_catalogo(&preparo.liberados, ferramenta_de_inicio),
                    registro: Some(json!({
                        "resultado": "nao_iniciou",
                        "fluxo": slug,
                        "motivo": motivo,
                    })),
                    ..DesfechoDoFluxo::default()
                }
                .reserva()
            }
            Err(erro) => {
                tracing::warn!(erro = %erro, fluxo = slug, "erro ao iniciar o fluxo escolhido pelo G9");
                DesfechoDoFluxo {
                    texto: bloco_do_catalogo(&preparo.liberados, ferramenta_de_inicio),
                    registro: Some(json!({
                        "resultado": "falhou",
                        "fluxo": slug,
                        "motivo": "erro de armazenamento ao iniciar a execução",
                    })),
                    ..DesfechoDoFluxo::default()
                }
                .reserva()
            }
        };
    }

    // 3. Sem escolha confiável: `nenhum` e limiar baixo não oferecem catálogo; falha e gate não
    //    perguntado caem na reserva do PRD §6.3-4.
    match motivo_sem_inicio(jev, g9_ligado) {
        MotivoSemInicio::Nenhum => DesfechoDoFluxo {
            registro: Some(registro("nenhum")),
            ..DesfechoDoFluxo::default()
        },
        MotivoSemInicio::AbaixoDoLimiar => DesfechoDoFluxo {
            registro: Some(registro("abaixo_do_limiar")),
            ..DesfechoDoFluxo::default()
        },
        MotivoSemInicio::Falhou => DesfechoDoFluxo {
            texto: bloco_do_catalogo(&preparo.liberados, ferramenta_de_inicio),
            registro: Some(registro("fallback")),
            ..DesfechoDoFluxo::default()
        }
        .reserva(),
        MotivoSemInicio::NaoPerguntado => DesfechoDoFluxo {
            texto: bloco_do_catalogo(&preparo.liberados, ferramenta_de_inicio),
            registro: Some(registro("reserva")),
            ..DesfechoDoFluxo::default()
        }
        .reserva(),
    }
}

#[cfg(test)]
mod testes {
    use std::sync::Arc;

    use poph_fluxos::{Limites, compilar};
    use poph_store::{NovaConversa, NovaOrg, NovoAgente, NovoFluxo, PublicacaoDeRevisao, Store};
    use poph_types::{AgentId, AtorEvento, FluxosConfig, OrgId};

    use super::{AjustesDoFluxo, Inicio, QuemInicia, iniciar};

    const SCRIPT: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(quando = "O cliente quer marcar visita."),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    acoes = [acao("seguir", "Seguir", vai_para = fim("pronto"))],
)
"#;

    /// Banco temporário com org, agente, conversa (que já nasce com Ficha) e um fluxo publicado.
    struct Cenario {
        pasta: std::path::PathBuf,
        store: Arc<Store>,
        org: OrgId,
        agente: AgentId,
        ficha: poph_store::Ficha,
        fluxo: poph_types::FluxoId,
    }

    impl Cenario {
        fn novo(rotulo: &str) -> Self {
            let pasta = std::env::temp_dir().join(format!(
                "poph-fx23-un-{rotulo}-{}-{}",
                std::process::id(),
                AgentId::generate()
            ));
            let _ = std::fs::remove_dir_all(&pasta);
            std::fs::create_dir_all(&pasta).expect("pasta do teste");
            let store = Arc::new(
                Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
                    .expect("abre o banco"),
            );
            let org = store
                .orgaos()
                .criar(NovaOrg {
                    nome: format!("Org {rotulo}"),
                    slug: format!("org-{rotulo}-{}", AgentId::generate()),
                    config_json: "{}".to_owned(),
                })
                .expect("cria a org")
                .id;
            let agente = store
                .agentes()
                .criar(
                    org,
                    NovoAgente {
                        nome: "Bia".to_owned(),
                        slug: format!("bia-{rotulo}"),
                        ..NovoAgente::default()
                    },
                )
                .expect("cria o agente")
                .id;
            store
                .instrucoes()
                .salvar(org, agente, 0, "{}", "# Bia", true)
                .expect("grava as instruções");
            let conversa = store
                .conversas()
                .criar(
                    org,
                    NovaConversa {
                        agent_id: agente,
                        integration_id: None,
                        canal: "cli".to_owned(),
                        peer_externo: Some("cliente-ficticio".to_owned()),
                        departamento_alvo: None,
                    },
                )
                .expect("abre a conversa")
                .id;
            let ficha = store
                .fichas()
                .por_conversa(org, conversa)
                .expect("lê a Ficha")
                .expect("a conversa nasce com Ficha");
            let fluxo = store
                .fluxos()
                .criar(
                    org,
                    NovoFluxo {
                        slug: "agendar_visita".to_owned(),
                        nome: "Agendar visita".to_owned(),
                        descricao: String::new(),
                        rascunho_fonte: SCRIPT.to_owned(),
                    },
                )
                .expect("cria o fluxo");
            let compilado = compilar(SCRIPT, &Limites::default());
            let ir = compilado.ir().expect("o script do teste compila");
            store
                .revisoes()
                .publicar_revisao(
                    org,
                    fluxo.id,
                    PublicacaoDeRevisao {
                        esperada: fluxo.versao,
                        fonte: SCRIPT.to_owned(),
                        definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                        schema_versao: Some(ir.schema_versao),
                        resolucoes_json: "{}".to_owned(),
                        ator_tipo: "system".to_owned(),
                        ator_id: None,
                    },
                )
                .expect("publica a revisão");
            Self {
                pasta,
                store,
                org,
                agente,
                ficha,
                fluxo: fluxo.id,
            }
        }

        fn liberar(&self) {
            self.store
                .fluxos()
                .liberar(self.org, self.agente, self.fluxo)
                .expect("libera o fluxo");
        }

        fn ajustes(&self) -> AjustesDoFluxo {
            AjustesDoFluxo::da_config(&FluxosConfig::default())
        }
    }

    impl Drop for Cenario {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.pasta);
        }
    }

    fn decisao<'a>(escolha: &'a str) -> poph_store::DecisaoDoJev<'a> {
        poph_store::DecisaoDoJev {
            escolha,
            confianca: Some(0.95),
        }
    }

    /// O caminho feliz: abre a execução com a etapa de entrada e o evento assinado pelo Jev.
    #[test]
    fn abre_a_execucao_com_a_decisao_do_jev() {
        let cenario = Cenario::novo("abre");
        cenario.liberar();
        let ajustes = cenario.ajustes();
        let inicio = iniciar(
            &ajustes,
            &cenario.store,
            cenario.org,
            &cenario.ficha,
            "agendar_visita",
            QuemInicia::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Inicio::Aberta { execucao, .. } = inicio else {
            panic!("a execução tinha de abrir");
        };
        assert_eq!(execucao.no_atual, "coleta");
        let eventos = cenario
            .store
            .fichas()
            .eventos(cenario.org, cenario.ficha.id, None, 20)
            .expect("lê os eventos");
        let iniciado = eventos
            .iter()
            .find(|evento| evento.tipo == poph_types::TipoEventoFicha::FluxoIniciado)
            .expect("evento fluxo_iniciado");
        assert_eq!(iniciado.ator_tipo, AtorEvento::Jev);
        assert!(
            iniciado
                .payload_json
                .contains("\"escolha\":\"agendar_visita\"")
        );
    }

    /// Fluxo inativo: recusado antes de qualquer escrita (o caso que pode acontecer no meio do
    /// turno, entre a leitura do preparo e a decisão do Jev).
    #[test]
    fn fluxo_inativo_e_recusado_sem_gravar() {
        let cenario = Cenario::novo("inativo");
        cenario.liberar();
        let atual = cenario
            .store
            .fluxos()
            .obter(cenario.org, cenario.fluxo)
            .expect("lê o fluxo");
        cenario
            .store
            .fluxos()
            .atualizar(
                cenario.org,
                cenario.fluxo,
                atual.versao,
                poph_store::MudancaFluxo {
                    status: Some(poph_store::Status::Inativo),
                    ..poph_store::MudancaFluxo::default()
                },
            )
            .expect("desativa");
        let ajustes = cenario.ajustes();
        let inicio = iniciar(
            &ajustes,
            &cenario.store,
            cenario.org,
            &cenario.ficha,
            "agendar_visita",
            QuemInicia::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Inicio::Recusado(motivo) = inicio else {
            panic!("fluxo inativo tinha de ser recusado");
        };
        // Frase desta validação (a do `poph-store` é outra: "o fluxo está inativo: ...").
        assert!(motivo.contains("e não inicia execução nova"), "{motivo}");
        assert!(
            cenario
                .store
                .execucoes()
                .da_ficha(cenario.org, cenario.ficha.id, 10)
                .expect("lê as execuções")
                .is_empty()
        );
    }

    /// Fluxo não liberado ao agente da Ficha: recusado.
    #[test]
    fn fluxo_nao_liberado_e_recusado() {
        let cenario = Cenario::novo("sem-permissao");
        let ajustes = cenario.ajustes();
        let inicio = iniciar(
            &ajustes,
            &cenario.store,
            cenario.org,
            &cenario.ficha,
            "agendar_visita",
            QuemInicia::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Inicio::Recusado(motivo) = inicio else {
            panic!("fluxo não liberado tinha de ser recusado");
        };
        assert!(
            motivo.contains("para o agente deste atendimento"),
            "{motivo}"
        );
    }

    /// Slug que não existe na organização (fluxo de outra org, ou inventado pelo Jev): recusado.
    #[test]
    fn slug_inexistente_e_recusado() {
        let cenario = Cenario::novo("inexistente");
        cenario.liberar();
        let ajustes = cenario.ajustes();
        let inicio = iniciar(
            &ajustes,
            &cenario.store,
            cenario.org,
            &cenario.ficha,
            "cancelar_tudo",
            QuemInicia::Jev(decisao("cancelar_tudo")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Inicio::Recusado(motivo) = inicio else {
            panic!("slug inexistente tinha de ser recusado");
        };
        assert!(motivo.contains("não existe na organização"), "{motivo}");
    }

    /// Fluxo sem revisão publicada: recusado (não há etapa para abrir).
    #[test]
    fn fluxo_sem_revisao_publicada_e_recusado() {
        let cenario = Cenario::novo("sem-revisao");
        let sem_revisao = cenario
            .store
            .fluxos()
            .criar(
                cenario.org,
                NovoFluxo {
                    slug: "sem_revisao".to_owned(),
                    nome: "Sem revisão".to_owned(),
                    descricao: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo");
        cenario
            .store
            .fluxos()
            .liberar(cenario.org, cenario.agente, sem_revisao.id)
            .expect("libera");
        let ajustes = cenario.ajustes();
        let inicio = iniciar(
            &ajustes,
            &cenario.store,
            cenario.org,
            &cenario.ficha,
            "sem_revisao",
            QuemInicia::Jev(decisao("sem_revisao")),
            None,
        )
        .expect("sem erro de armazenamento");
        let Inicio::Recusado(motivo) = inicio else {
            panic!("fluxo sem revisão tinha de ser recusado");
        };
        assert!(motivo.contains("não tem revisão publicada"), "{motivo}");
    }

    /// Ficha que já tem execução ativa (corrida com o atendente): `JaAtiva`, sem segunda execução.
    #[test]
    fn ficha_com_execucao_ativa_devolve_ja_ativa() {
        let cenario = Cenario::novo("ja-ativa");
        cenario.liberar();
        let ajustes = cenario.ajustes();
        let primeiro = iniciar(
            &ajustes,
            &cenario.store,
            cenario.org,
            &cenario.ficha,
            "agendar_visita",
            QuemInicia::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        assert!(matches!(primeiro, Inicio::Aberta { .. }));
        let segundo = iniciar(
            &ajustes,
            &cenario.store,
            cenario.org,
            &cenario.ficha,
            "agendar_visita",
            QuemInicia::Jev(decisao("agendar_visita")),
            None,
        )
        .expect("sem erro de armazenamento");
        assert!(matches!(segundo, Inicio::JaAtiva));
        assert_eq!(
            cenario
                .store
                .execucoes()
                .da_ficha(cenario.org, cenario.ficha.id, 10)
                .expect("lê as execuções")
                .len(),
            1,
            "uma execução ativa por Ficha"
        );
    }
}
