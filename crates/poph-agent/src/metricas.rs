//! Métricas da memória quente (PRD §13/§15), sem HTTP.
//!
//! Registra no [`Registro`] de `poph-types` os números que a HR-20 publica em `/metrics`:
//! atores vivos, despejos por ociosidade, acertos/erros do cache de configuração, memória do cache,
//! espera por vaga e o que está fila por org. Os nomes não colidem com o catálogo base.

use poph_types::{Counter, DomainResult, Gauge, Histogram, Registry};

/// Números da camada de memória e concorrência.
#[derive(Debug, Clone)]
pub struct MetricasMemoria {
    /// Conversas com ator vivo.
    pub atores_vivos: Gauge,
    /// Atores descartados por ociosidade.
    pub atores_despejados: Counter,
    /// Pedidos de ator recusados por o teto de atores vivos estar todo ocupado.
    pub atores_acima_do_teto: Counter,
    /// Eventos entregues ao ator.
    pub eventos_entregues: Counter,
    /// Eventos recusados porque a fila da conversa estava cheia.
    pub eventos_recusados: Counter,
    /// Leitura de config quente que veio do cache.
    pub config_acertos: Counter,
    /// Leitura de config quente que precisou reler o banco.
    pub config_erros: Counter,
    /// Config quente invalidada por mudança de versão do agente.
    pub config_invalidadas: Counter,
    /// Memória estimada ocupada pelo cache de config quente, em bytes.
    pub config_bytes: Gauge,
    /// Agentes com config quente carregada.
    pub agentes_carregados: Gauge,
    /// Tempo de espera por vaga de simultaneidade, em segundos.
    pub espera_vaga_segundos: Histogram,
}

impl MetricasMemoria {
    /// Registra o catálogo no registro dado.
    pub fn registrar(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            atores_vivos: registro.gauge(
                "poph_atores_vivos",
                "Conversas com ator vivo na memória do processo.",
            )?,
            atores_despejados: registro.counter(
                "poph_atores_despejados_total",
                "Atores descartados por ociosidade.",
            )?,
            atores_acima_do_teto: registro.counter(
                "poph_atores_acima_do_teto_total",
                "Pedidos que passaram do teto de atores por não haver ator ocioso para despejar.",
            )?,
            eventos_entregues: registro.counter(
                "poph_eventos_entregues_total",
                "Eventos entregues à fila do ator de uma conversa.",
            )?,
            eventos_recusados: registro.counter(
                "poph_eventos_recusados_total",
                "Eventos recusados porque a fila da conversa estava cheia.",
            )?,
            config_acertos: registro.counter(
                "poph_config_quente_acertos_total",
                "Leituras de config do agente atendidas pelo cache.",
            )?,
            config_erros: registro.counter(
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
            agentes_carregados: registro.gauge(
                "poph_config_agentes_total",
                "Agentes com config quente carregada (distinto de poph_agentes_carregados do catálogo base).",
            )?,
            espera_vaga_segundos: registro.histogram(
                "poph_espera_vaga_segundos",
                "Tempo de espera por vaga de simultaneidade, em segundos.",
                &[0.0001, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0],
            )?,
        })
    }

    /// Constrói um catálogo isolado (testes e usos que não publicam em `/metrics`).
    ///
    /// Os handles são `Arc`: continuam válidos depois que o `Registro` local sai de escopo.
    pub fn proprio() -> Self {
        let registro = Registry::new();
        Self::registrar(&registro).expect("catálogo de métricas sem conflito de nome")
    }
}

/// Números do gate G9 (FX-23): o que a escolha do Jev fez com o fluxo do atendimento.
///
/// São as quatro séries `poph_fluxos_g9_*` que a FX-26 declarou. Elas moram aqui, e não no catálogo
/// `poph_fluxos_*` do servidor, porque **quem decide é o turno**: o servidor só publica os mesmos
/// contadores no registro do `/metrics` (`Estado::metricas_fluxos`), sem declarar o nome duas vezes.
#[derive(Debug, Clone)]
pub struct MetricasG9 {
    /// Decisões do G9 que iniciaram um fluxo (`poph_fluxos_g9_iniciou_total`).
    pub iniciou: Counter,
    /// Decisões do G9 em que o Jev respondeu "nenhum" (`poph_fluxos_g9_nenhum_total`).
    pub nenhum: Counter,
    /// Decisões do G9 com confiança ou margem abaixo do limiar
    /// (`poph_fluxos_g9_abaixo_do_limiar_total`).
    pub abaixo_do_limiar: Counter,
    /// Turnos em que o G9 não decidiu (Jev fora, resposta inutilizável ou recusa da validação) e o
    /// prompt recebeu o catálogo (`poph_fluxos_g9_fallback_total`).
    pub fallback: Counter,
}

impl MetricasG9 {
    /// Registra o catálogo no registro dado. Repetir no mesmo registro é erro (o registro recusa).
    pub fn registrar(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            iniciou: registro.counter(
                "poph_fluxos_g9_iniciou_total",
                "Decisões do G9 que iniciaram um fluxo de atendimento.",
            )?,
            nenhum: registro.counter(
                "poph_fluxos_g9_nenhum_total",
                "Decisões do G9 em que o Jev respondeu que nenhum fluxo se aplica.",
            )?,
            abaixo_do_limiar: registro.counter(
                "poph_fluxos_g9_abaixo_do_limiar_total",
                "Decisões do G9 com confiança ou margem abaixo do limiar (nada inicia).",
            )?,
            fallback: registro.counter(
                "poph_fluxos_g9_fallback_total",
                "Turnos em que o G9 não decidiu e o catálogo foi oferecido ao modelo.",
            )?,
        })
    }

    /// Catálogo isolado (testes e usos que não publicam em `/metrics`).
    pub fn proprias() -> Self {
        let registro = Registry::new();
        Self::registrar(&registro).expect("catálogo do G9 sem conflito de nome")
    }
}

/// Números da entrada e da saída (HR-12), sem HTTP.
///
/// Os medidores são agregados da instância: o tamanho da `inbox` e o que está em `sending` são
/// somados entre as orgs a cada ciclo do fluxo. Os contadores dizem o que aconteceu com cada
/// mensagem, e é por eles que a operação percebe reentrega de webhook, evento inválido e falha de
/// envio (transitória, definitiva ou por tentativas esgotadas).
#[derive(Debug, Clone)]
pub struct MetricasFluxo {
    /// Itens da `inbox` ainda não processados, somados entre as orgs.
    pub inbox_pendentes: Gauge,
    /// Itens da `outbox` em `sending` (entregues ao canal, sem confirmação), somados entre as orgs.
    pub outbox_enviando: Gauge,
    /// Entregas novas gravadas na `inbox`.
    pub entradas_novas: Counter,
    /// Entregas repetidas descartadas pela chave única (webhook reenviado).
    pub entradas_duplicadas: Counter,
    /// Itens da `inbox` cujo evento não pôde ser lido (payload inválido).
    pub entradas_invalidas: Counter,
    /// Despachos adiados por fila cheia ou ator ocupado (voltam no próximo ciclo).
    pub entradas_adiadas: Counter,
    /// Itens da `inbox` que ficaram registrados sem processar depois de esgotar as tentativas.
    pub entradas_desistentes: Counter,
    /// Itens da `inbox` concluídos pelo turno.
    pub entradas_concluidas: Counter,
    /// Saídas entregues ao canal com confirmação.
    pub saidas_enviadas: Counter,
    /// Tentativas de envio que falharam de forma transitória (serão repetidas).
    pub saidas_transitorias: Counter,
    /// Tentativas de envio que falharam de forma definitiva.
    pub saidas_definitivas: Counter,
    /// Saídas que esgotaram as tentativas antes de envio bem-sucedido.
    pub saidas_esgotadas: Counter,
    /// Saídas em `sending` reenviadas na retomada da subida (D11).
    pub reenvios_retomada: Counter,
}

impl MetricasFluxo {
    /// Registra o catálogo no registro dado.
    pub fn registrar(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            inbox_pendentes: registro.gauge(
                "poph_fluxo_inbox_pendentes",
                "Itens da inbox ainda não processados, somados entre as orgs.",
            )?,
            outbox_enviando: registro.gauge(
                "poph_fluxo_outbox_enviando",
                "Itens da outbox em sending (entregues ao canal, sem confirmação).",
            )?,
            entradas_novas: registro.counter(
                "poph_fluxo_entradas_novas_total",
                "Entregas novas gravadas na inbox.",
            )?,
            entradas_duplicadas: registro.counter(
                "poph_fluxo_entradas_duplicadas_total",
                "Entregas repetidas descartadas pela chave única da inbox.",
            )?,
            entradas_invalidas: registro.counter(
                "poph_fluxo_entradas_invalidas_total",
                "Itens da inbox cujo evento não pôde ser lido.",
            )?,
            entradas_adiadas: registro.counter(
                "poph_fluxo_entradas_adiadas_total",
                "Despachos adiados por fila cheia ou ator ocupado.",
            )?,
            entradas_desistentes: registro.counter(
                "poph_fluxo_entradas_desistentes_total",
                "Itens da inbox registrados sem processar depois de esgotar as tentativas de despacho.",
            )?,
            entradas_concluidas: registro.counter(
                "poph_fluxo_entradas_concluidas_total",
                "Itens da inbox concluídos pelo turno.",
            )?,
            saidas_enviadas: registro.counter(
                "poph_fluxo_saidas_enviadas_total",
                "Saídas entregues ao canal com confirmação.",
            )?,
            saidas_transitorias: registro.counter(
                "poph_fluxo_saidas_transitorias_total",
                "Tentativas de envio que falharam de forma transitória.",
            )?,
            saidas_definitivas: registro.counter(
                "poph_fluxo_saidas_definitivas_total",
                "Tentativas de envio que falharam de forma definitiva.",
            )?,
            saidas_esgotadas: registro.counter(
                "poph_fluxo_saidas_esgotadas_total",
                "Saídas que esgotaram as tentativas antes de um envio bem-sucedido.",
            )?,
            reenvios_retomada: registro.counter(
                "poph_fluxo_reenvios_retomada_total",
                "Saídas em sending reenviadas na retomada da subida.",
            )?,
        })
    }

    /// Catálogo isolado (testes e usos que não publicam em `/metrics`).
    pub fn proprio() -> Self {
        let registro = Registry::new();
        Self::registrar(&registro).expect("catálogo de métricas de fluxo sem conflito de nome")
    }
}

/// Números do turno (HR-13), sem HTTP.
///
/// O primeiro texto é o número que o PRD §8 vigia de perto: ele é do provedor, não do Harness, e
/// por isso é publicado separado da duração total do turno (que inclui rodadas de ferramenta).
#[derive(Debug, Clone)]
pub struct MetricasTurno {
    /// Turnos abertos no banco.
    pub turnos_iniciados: Counter,
    /// Turnos concluídos.
    pub turnos_concluidos: Counter,
    /// Turnos que falharam (provedor, tempo máximo, sem vaga).
    pub turnos_falhos: Counter,
    /// Turnos ignorados (agente inativo, conversa transferida, mensagem já respondida).
    pub turnos_ignorados: Counter,
    /// Respostas gravadas em `messages` e na `outbox`.
    pub respostas_gravadas: Counter,
    /// Tokens de entrada somados.
    pub tokens_entrada: Counter,
    /// Tokens de saída somados.
    pub tokens_saida: Counter,
    /// Tempo até o primeiro pedaço de texto do modelo, em milissegundos.
    pub primeiro_texto_ms: Histogram,
    /// Duração total do turno, em milissegundos.
    pub duracao_turno_ms: Histogram,
    /// Espera da janela de agrupamento, em milissegundos.
    pub espera_agrupamento_ms: Histogram,
    /// Janelas de agrupamento esticadas pelo G2 ("esperar mais", HR-30).
    pub janelas_esticadas: Counter,
    /// Janelas de agrupamento encurtadas pelo G2 ("processar agora", HR-30).
    pub janelas_diretas: Counter,
    /// Trechos da base de conhecimento injetados nos prompts (HR-17).
    pub trechos_recuperados: Counter,
    /// Buscas na base que falharam (o turno segue sem os trechos).
    pub busca_falhas: Counter,
    /// Mensagens adiadas porque o modelo falhou: vão ser tentadas de novo.
    pub falhas_modelo_adiadas: Counter,
    /// Mensagens concluídas sem resposta porque o modelo falhou em todas as tentativas. Inclui as
    /// recusas do provedor (ver `falhas_modelo_recusadas`).
    pub falhas_modelo_definitivas: Counter,
    /// Mensagens concluídas sem repetir porque o provedor recusou o pedido (chave, modelo ou pedido).
    /// Parte de `falhas_modelo_definitivas`.
    pub falhas_modelo_recusadas: Counter,
    /// Turnos em que o G3 dispensou a base e o catálogo (mensagem classificada como `nada`).
    pub buscas_puladas_g3: Counter,
    /// Turnos em que o G5 escolheu o produto entre os candidatos da busca (HR-33). Nesses turnos a
    /// ferramenta `buscar_produto` não é oferecida: o catálogo já foi consultado.
    pub produtos_escolhidos_g5: Counter,
    /// Conversas encerradas pelo cliente com o G8 (HR-36): o turno termina sem chamar o modelo de
    /// texto e grava só a despedida configurada.
    pub encerradas_g8: Counter,
    /// Respostas revisadas pelo G6 antes do envio (HR-34). Uma por turno com o gate ligado.
    pub revisoes_g6: Counter,
    /// Respostas reprovadas pelo G6: cada uma custou **uma** regeneração ao modelo de texto.
    pub reprovadas_g6: Counter,
    /// Respostas substituídas pela resposta segura do agente, porque a regeneração também foi
    /// reprovada (ou a segunda revisão não respondeu).
    pub respostas_seguras_g6: Counter,
    /// Turnos em que o G7 classificou a mensagem (ou o texto do arquivo) como injeção ou abuso
    /// (HR-35), qualquer que seja a ação configurada.
    pub g7_detectados: Counter,
    /// Turnos em que o G7 respondeu o texto neutro, sem chamar o modelo de texto.
    pub g7_respostas_padrao: Counter,
    /// Turnos em que o G7 transferiu a conversa para humano pelo caminho do G1.
    pub g7_transferencias: Counter,
}

impl MetricasTurno {
    /// Registra o catálogo no registro dado.
    pub fn registrar(registro: &Registry) -> DomainResult<Self> {
        Ok(Self {
            turnos_iniciados: registro.counter(
                "poph_turnos_iniciados_total",
                "Turnos abertos no banco.",
            )?,
            turnos_concluidos: registro.counter(
                "poph_turnos_concluidos_total",
                "Turnos concluídos com resposta gravada.",
            )?,
            turnos_falhos: registro.counter(
                "poph_turnos_falhos_total",
                "Turnos que falharam (provedor, tempo máximo ou sem vaga).",
            )?,
            turnos_ignorados: registro.counter(
                "poph_turnos_ignorados_total",
                "Turnos ignorados (agente inativo, conversa transferida ou mensagem já respondida).",
            )?,
            respostas_gravadas: registro.counter(
                "poph_turnos_respostas_total",
                "Respostas do agente gravadas em messages e na outbox.",
            )?,
            tokens_entrada: registro.counter(
                "poph_turnos_tokens_entrada_total",
                "Tokens de entrada somados nos turnos.",
            )?,
            tokens_saida: registro.counter(
                "poph_turnos_tokens_saida_total",
                "Tokens de saída somados nos turnos.",
            )?,
            primeiro_texto_ms: registro.histogram(
                "poph_turnos_primeiro_texto_ms",
                "Tempo até o primeiro pedaço de texto do modelo, em milissegundos.",
                &[100.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 12_000.0, 30_000.0],
            )?,
            duracao_turno_ms: registro.histogram(
                "poph_turnos_duracao_ms",
                "Duração total do turno, em milissegundos.",
                &[50.0, 200.0, 500.0, 1_000.0, 3_000.0, 8_000.0, 20_000.0, 60_000.0],
            )?,
            espera_agrupamento_ms: registro.histogram(
                "poph_turnos_espera_agrupamento_ms",
                "Espera da janela de agrupamento antes de montar o turno, em milissegundos.",
                &[0.0, 100.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0],
            )?,
            janelas_esticadas: registro.counter(
                "poph_turnos_janelas_esticadas_total",
                "Janelas de agrupamento esticadas pelo G2 (esperar mais, HR-30).",
            )?,
            janelas_diretas: registro.counter(
                "poph_turnos_janelas_diretas_total",
                "Janelas de agrupamento encurtadas pelo G2 (processar agora, HR-30).",
            )?,
            trechos_recuperados: registro.counter(
                "poph_turnos_trechos_recuperados_total",
                "Trechos da base de conhecimento injetados nos prompts.",
            )?,
            busca_falhas: registro.counter(
                "poph_turnos_busca_falhas_total",
                "Buscas na base de conhecimento que falharam (o turno seguiu sem trechos).",
            )?,
            falhas_modelo_adiadas: registro.counter(
                "poph_turnos_falha_modelo_adiadas_total",
                "Mensagens adiadas porque o modelo falhou; serão tentadas de novo.",
            )?,
            falhas_modelo_definitivas: registro.counter(
                "poph_turnos_falha_modelo_definitivas_total",
                "Mensagens concluídas sem resposta depois de esgotar as tentativas do modelo.",
            )?,
            falhas_modelo_recusadas: registro.counter(
                "poph_turnos_falha_modelo_recusadas_total",
                "Mensagens concluídas sem repetir porque o provedor recusou o pedido (chave, modelo ou pedido).",
            )?,
            buscas_puladas_g3: registro.counter(
                "poph_turnos_buscas_puladas_g3_total",
                "Turnos em que o G3 dispensou a base e o catálogo (mensagem classificada como nada).",
            )?,
            produtos_escolhidos_g5: registro.counter(
                "poph_turnos_produtos_escolhidos_g5_total",
                "Turnos em que o G5 escolheu o produto do cliente entre os candidatos da busca.",
            )?,
            encerradas_g8: registro.counter(
                "poph_turnos_encerrados_g8_total",
                "Turnos em que o G8 encerrou a conversa pelo cliente, sem chamar o modelo de texto.",
            )?,
            revisoes_g6: registro.counter(
                "poph_turnos_revisoes_g6_total",
                "Respostas revisadas pelo G6 antes do envio (HR-34).",
            )?,
            reprovadas_g6: registro.counter(
                "poph_turnos_reprovadas_g6_total",
                "Respostas reprovadas pelo G6 que custaram uma regeneração ao modelo.",
            )?,
            respostas_seguras_g6: registro.counter(
                "poph_turnos_respostas_seguras_g6_total",
                "Respostas trocadas pela resposta segura do agente, depois de uma regeneração também reprovada.",
            )?,
            g7_detectados: registro.counter(
                "poph_turnos_g7_detectados_total",
                "Turnos em que o G7 classificou a mensagem ou o texto do arquivo como injeção ou abuso.",
            )?,
            g7_respostas_padrao: registro.counter(
                "poph_turnos_g7_resposta_padrao_total",
                "Turnos em que o G7 respondeu o texto neutro sem chamar o modelo de texto.",
            )?,
            g7_transferencias: registro.counter(
                "poph_turnos_g7_transferidos_total",
                "Turnos em que o G7 transferiu a conversa para humano pelo caminho do G1.",
            )?,
        })
    }

    /// Catálogo isolado (testes e usos que não publicam em `/metrics`).
    pub fn proprio() -> Self {
        let registro = Registry::new();
        Self::registrar(&registro).expect("catálogo de métricas de turno sem conflito de nome")
    }
}
