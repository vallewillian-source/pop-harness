//! Motor de execução de fluxos: iniciar a execução, registrar campos, executar ação, avançar
//! pelas decisões, fim, abandono e geração dos efeitos.
//!
//! Implementado na FX-13 e FX-14. É uma função pura sobre a estrutura compilada e o estado que
//! vem do banco; quem grava a transição é a operação atômica do `poph-store` (FX-20). Efeitos
//! (mensagem ao cliente, notificação de WhatsApp, encaminhamento) saem por fila durável (FX-21).
