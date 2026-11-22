//! Formatador: forma canônica do script de fluxo (script ⇄ estrutura).
//!
//! Implementado na FX-16. Formata o script para uma forma única e estável. Comentários são
//! preservados quando o script é formatado; a estrutura não guarda comentários, então gerar o
//! script a partir da estrutura os perde (decisão registrada em `docs/FLUXOS_LINGUAGEM.md`).
