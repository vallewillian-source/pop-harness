use poph_types::{Counter, DomainResult, Gauge, Histogram, Registry};

#[derive(Debug, Clone)]
pub struct MetricsMemory {
    pub actors_live: Gauge,
    pub actors_despejados: Counter,
    pub actors_above_cap: Counter,
    pub events_delivered: Counter,
    pub events_refused: Counter,
    pub config_hits: Counter,
    pub config_errors: Counter,
    pub config_invalidadas: Counter,
    pub config_bytes: Gauge,
    pub agents_load: Gauge,
    pub wait_slot_seconds: Histogram,
}

impl MetricsMemory {
    pub fn register(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            actors_live: registro.gauge(
                "poph_atores_vivos",
                "Conversas com ator vivo na memória do processo.",
            )?,
            actors_despejados: registro.counter(
                "poph_atores_despejados_total",
                "Atores descartados por ociosidade.",
            )?,
            actors_above_cap: registro.counter(
                "poph_atores_acima_do_teto_total",
                "Pedidos que passaram do teto de atores por não haver ator ocioso para despejar.",
            )?,
            events_delivered: registro.counter(
                "poph_eventos_entregues_total",
                "Eventos entregues à fila do ator de uma conversa.",
            )?,
            events_refused: registro.counter(
                "poph_eventos_recusados_total",
                "Eventos recusados porque a fila da conversa estava cheia.",
            )?,
            config_hits: registro.counter(
                "poph_config_quente_acertos_total",
                "Leituras de config do agente atendidas pelo cache.",
            )?,
            config_errors: registro.counter(
                "poph_config_quente_erros_total",
                "Leituras de config do agente que precisaram reler o banco.",
            )?,
            config_invalidadas: registro.counter(
                "poph_config_quente_invalidadas_total",
                "Config quente recarregada porque a versão do agente mudou.",
            )?,
            config_bytes: registro.gauge(
                "poph_config_quente_bytes",
                "Memória estimada ocupada pelo cache de config quente, em bytes.",
            )?,
            agents_load: registro.gauge(
                "poph_config_agentes_total",
                "Agentes com config quente carregada (distinto de poph_agentes_carregados do catálogo base).",
            )?,
            wait_slot_seconds: registro.histogram(
                "poph_espera_vaga_segundos",
                "Tempo de espera por vaga de simultaneidade, em segundos.",
                &[0.0001, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0],
            )?,
        })
    }

    pub fn own() -> Self {
        let registro = Registry::new();
        Self::register(&registro).expect("catálogo de métricas sem conflito de nome")
    }
}

#[derive(Debug, Clone)]
pub struct MetricsG9 {
    pub start: Counter,
    pub none: Counter,
    pub below_threshold: Counter,
    pub fallback: Counter,
}

impl MetricsG9 {
    pub fn register(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            start: registro.counter(
                "poph_fluxos_g9_iniciou_total",
                "Decisões do G9 que iniciaram um fluxo de atendimento.",
            )?,
            none: registro.counter(
                "poph_fluxos_g9_nenhum_total",
                "Decisões do G9 em que o Jev respondeu que nenhum fluxo se aplica.",
            )?,
            below_threshold: registro.counter(
                "poph_fluxos_g9_abaixo_do_limiar_total",
                "Decisões do G9 com confiança ou margem abaixo do limiar (nada inicia).",
            )?,
            fallback: registro.counter(
                "poph_fluxos_g9_fallback_total",
                "Turnos em que o G9 não decidiu e o catálogo foi oferecido ao modelo.",
            )?,
        })
    }

    pub fn own() -> Self {
        let registro = Registry::new();
        Self::register(&registro).expect("catálogo do G9 sem conflito de nome")
    }
}

#[derive(Debug, Clone)]
pub struct MetricsFlow {
    pub inbox_pending: Gauge,
    pub outbox_sending: Gauge,
    pub entries_new: Counter,
    pub entries_duplicated: Counter,
    pub entries_invalid: Counter,
    pub entries_deferred: Counter,
    pub entries_desistentes: Counter,
    pub entries_finished: Counter,
    pub outputs_sent: Counter,
    pub outputs_transient: Counter,
    pub outputs_definitive: Counter,
    pub outputs_exhausted: Counter,
    pub resends_resumption: Counter,
}

impl MetricsFlow {
    pub fn register(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            inbox_pending: registro.gauge(
                "poph_fluxo_inbox_pendentes",
                "Itens da inbox ainda não processados, somados entre as orgs.",
            )?,
            outbox_sending: registro.gauge(
                "poph_fluxo_outbox_enviando",
                "Itens da outbox em sending (entregues ao canal, sem confirmação).",
            )?,
            entries_new: registro.counter(
                "poph_fluxo_entradas_novas_total",
                "Entregas novas gravadas na inbox.",
            )?,
            entries_duplicated: registro.counter(
                "poph_fluxo_entradas_duplicadas_total",
                "Entregas repetidas descartadas pela chave única da inbox.",
            )?,
            entries_invalid: registro.counter(
                "poph_fluxo_entradas_invalidas_total",
                "Itens da inbox cujo evento não pôde ser lido.",
            )?,
            entries_deferred: registro.counter(
                "poph_fluxo_entradas_adiadas_total",
                "Despachos adiados por fila cheia ou ator ocupado.",
            )?,
            entries_desistentes: registro.counter(
                "poph_fluxo_entradas_desistentes_total",
                "Itens da inbox registrados sem processar depois de esgotar as tentativas de despacho.",
            )?,
            entries_finished: registro.counter(
                "poph_fluxo_entradas_concluidas_total",
                "Itens da inbox concluídos pelo turno.",
            )?,
            outputs_sent: registro.counter(
                "poph_fluxo_saidas_enviadas_total",
                "Saídas entregues ao canal com confirmação.",
            )?,
            outputs_transient: registro.counter(
                "poph_fluxo_saidas_transitorias_total",
                "Tentativas de envio que falharam de forma transitória.",
            )?,
            outputs_definitive: registro.counter(
                "poph_fluxo_saidas_definitivas_total",
                "Tentativas de envio que falharam de forma definitiva.",
            )?,
            outputs_exhausted: registro.counter(
                "poph_fluxo_saidas_esgotadas_total",
                "Saídas que esgotaram as tentativas antes de um envio bem-sucedido.",
            )?,
            resends_resumption: registro.counter(
                "poph_fluxo_reenvios_retomada_total",
                "Saídas em sending reenviadas na retomada da subida.",
            )?,
        })
    }

    pub fn own() -> Self {
        let registro = Registry::new();
        Self::register(&registro).expect("catálogo de métricas de fluxo sem conflito de nome")
    }
}

#[derive(Debug, Clone)]
pub struct MetricsTurn {
    pub turns_started: Counter,
    pub turns_finished: Counter,
    pub turns_faulty: Counter,
    pub turns_ignored: Counter,
    pub responses_stored: Counter,
    pub tokens_entry: Counter,
    pub tokens_output: Counter,
    pub first_text_ms: Histogram,
    pub duration_turn_ms: Histogram,
    pub wait_grouping_ms: Histogram,
    pub window_esticadas: Counter,
    pub window_direct: Counter,
    pub passages_recovered: Counter,
    pub search_failures: Counter,
    pub failures_model_deferred: Counter,
    pub failures_model_definitive: Counter,
    pub failures_model_refused: Counter,
    pub searches_puladas_g3: Counter,
    pub products_chosen_g5: Counter,
    pub closed_g8: Counter,
    pub revisions_g6: Counter,
    pub reprovadas_g6: Counter,
    pub responses_safe_g6: Counter,
    pub g7_detectados: Counter,
    pub g7_responses_default: Counter,
    pub g7_transfer: Counter,
}

impl MetricsTurn {
    pub fn register(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            turns_started: registro.counter(
                "poph_turnos_iniciados_total",
                "Turnos abertos no banco.",
            )?,
            turns_finished: registro.counter(
                "poph_turnos_concluidos_total",
                "Turnos concluídos com resposta gravada.",
            )?,
            turns_faulty: registro.counter(
                "poph_turnos_falhos_total",
                "Turnos que falharam (provedor, tempo máximo ou sem vaga).",
            )?,
            turns_ignored: registro.counter(
                "poph_turnos_ignorados_total",
                "Turnos ignorados (agente inativo, conversa transferida ou mensagem já respondida).",
            )?,
            responses_stored: registro.counter(
                "poph_turnos_respostas_total",
                "Respostas do agente gravadas em messages e na outbox.",
            )?,
            tokens_entry: registro.counter(
                "poph_turnos_tokens_entrada_total",
                "Tokens de entrada somados nos turnos.",
            )?,
            tokens_output: registro.counter(
                "poph_turnos_tokens_saida_total",
                "Tokens de saída somados nos turnos.",
            )?,
            first_text_ms: registro.histogram(
                "poph_turnos_primeiro_texto_ms",
                "Tempo até o primeiro pedaço de texto do modelo, em milissegundos.",
                &[100.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 12_000.0, 30_000.0],
            )?,
            duration_turn_ms: registro.histogram(
                "poph_turnos_duracao_ms",
                "Duração total do turno, em milissegundos.",
                &[50.0, 200.0, 500.0, 1_000.0, 3_000.0, 8_000.0, 20_000.0, 60_000.0],
            )?,
            wait_grouping_ms: registro.histogram(
                "poph_turnos_espera_agrupamento_ms",
                "Espera da janela de agrupamento antes de montar o turno, em milissegundos.",
                &[0.0, 100.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0],
            )?,
            window_esticadas: registro.counter(
                "poph_turnos_janelas_esticadas_total",
                "Janelas de agrupamento esticadas pelo G2 (esperar mais, HR-30).",
            )?,
            window_direct: registro.counter(
                "poph_turnos_janelas_diretas_total",
                "Janelas de agrupamento encurtadas pelo G2 (processar agora, HR-30).",
            )?,
            passages_recovered: registro.counter(
                "poph_turnos_trechos_recuperados_total",
                "Trechos da base de conhecimento injetados nos prompts.",
            )?,
            search_failures: registro.counter(
                "poph_turnos_busca_falhas_total",
                "Buscas na base de conhecimento que falharam (o turno seguiu sem trechos).",
            )?,
            failures_model_deferred: registro.counter(
                "poph_turnos_falha_modelo_adiadas_total",
                "Mensagens adiadas porque o modelo falhou; serão tentadas de novo.",
            )?,
            failures_model_definitive: registro.counter(
                "poph_turnos_falha_modelo_definitivas_total",
                "Mensagens concluídas sem resposta depois de esgotar as tentativas do modelo.",
            )?,
            failures_model_refused: registro.counter(
                "poph_turnos_falha_modelo_recusadas_total",
                "Mensagens concluídas sem repetir porque o provedor recusou o pedido (chave, modelo ou pedido).",
            )?,
            searches_puladas_g3: registro.counter(
                "poph_turnos_buscas_puladas_g3_total",
                "Turnos em que o G3 dispensou a base e o catálogo (mensagem classificada como nada).",
            )?,
            products_chosen_g5: registro.counter(
                "poph_turnos_produtos_escolhidos_g5_total",
                "Turnos em que o G5 escolheu o produto do cliente entre os candidatos da busca.",
            )?,
            closed_g8: registro.counter(
                "poph_turnos_encerrados_g8_total",
                "Turnos em que o G8 encerrou a conversa pelo cliente, sem chamar o modelo de texto.",
            )?,
            revisions_g6: registro.counter(
                "poph_turnos_revisoes_g6_total",
                "Respostas revisadas pelo G6 antes do envio (HR-34).",
            )?,
            reprovadas_g6: registro.counter(
                "poph_turnos_reprovadas_g6_total",
                "Respostas reprovadas pelo G6 que custaram uma regeneração ao modelo.",
            )?,
            responses_safe_g6: registro.counter(
                "poph_turnos_respostas_seguras_g6_total",
                "Respostas trocadas pela resposta segura do agente, depois de uma regeneração também reprovada.",
            )?,
            g7_detectados: registro.counter(
                "poph_turnos_g7_detectados_total",
                "Turnos em que o G7 classificou a mensagem ou o texto do arquivo como injeção ou abuso.",
            )?,
            g7_responses_default: registro.counter(
                "poph_turnos_g7_resposta_padrao_total",
                "Turnos em que o G7 respondeu o texto neutro sem chamar o modelo de texto.",
            )?,
            g7_transfer: registro.counter(
                "poph_turnos_g7_transferidos_total",
                "Turnos em que o G7 transferiu a conversa para humano pelo caminho do G1.",
            )?,
        })
    }

    pub fn own() -> Self {
        let registro = Registry::new();
        Self::register(&registro).expect("catálogo de métricas de turno sem conflito de nome")
    }
}
