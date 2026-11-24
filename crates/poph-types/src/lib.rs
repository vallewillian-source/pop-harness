//! Tipos de domínio do POP Harness, compartilhados por todos os crates.
//!
//! Aqui ficam identificadores tipados ([`ids`]), o erro raiz do domínio ([`error`]), a
//! configuração (tipos, padrões e validação, sem leitura de arquivo, [`config`]), o valor
//! que não pode vazar em log ([`segredo`]), o registro de métricas em memória ([`metricas`]), as
//! instruções estruturadas do agente ([`instrucoes`]), o contrato de integração com os canais
//! ([`integracao`], PRD §8.3) e os tipos do módulo de Fluxos e Fichas ([`fluxos`]).
//! Nenhuma regra de negócio: ela entra nos crates `poph-core` e `poph-server`.

pub mod config;
pub mod error;
pub mod fluxos;
pub mod ids;
pub mod instrucoes;
pub mod integracao;
pub mod metricas;
pub mod midia_fora_do_escopo;
pub mod segredo;

pub use config::{
    ClusterConfig, ClusterInstance, DadosConfig, EvolutionConfig, FluxosConfig, Formato, GatesJev,
    HandoffConfig, HarnessConfig, JevConfig, LimitesConfig, LlmConfig, LogConfig, MatrixConfig,
    PROVEDORES_LLM_ACEITOS, RetencaoConfig, ServidorConfig, provedor_llm_aceito,
};
pub use error::{DomainError, DomainResult};
pub use fluxos::{
    AtorAcao, AtorEvento, EstadoEfeito, FechadaPor, GatilhoFluxo, StatusEtapa, StatusExecucao,
    StatusFicha, TipoCampo, TipoEventoFicha,
};
pub use ids::*;
pub use instrucoes::{
    Escopo, FERRAMENTAS_V1, FerramentaConfig, HandoffEscopo, Identidade, PassoFluxo, Principio,
    StructuredInstructions, Tom,
};
pub use integracao::{
    AnexoEntrada, Capacidades, ComandoSaida, ErroComando, ErroIntegracao, EsquemaJson,
    EstadoIntegracao, EventoEstado, EventoIntegracao, FuturoComando, FuturoSaude, Handshake,
    InboundEvent, Integration, LimitesCanal, Manifesto, MensagemHost, MensagemPlugin,
    PROTOCOLO_VERSAO, PluginDeclarado, PluginsConfig, Presenca, Receptor, Saude, SendFile,
    SendText, SetPresence,
};
pub use metricas::{Contador, Histograma, Medidor, MetricasBase, Registro};
pub use midia_fora_do_escopo::MidiaForaDoEscopo;
pub use segredo::{Segredo, Sensivel};
