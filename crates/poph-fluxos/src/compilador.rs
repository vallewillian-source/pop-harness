//! Compilador: script Starlark de um fluxo → estrutura compilada (IR `schema_versao = 1`).
//!
//! Implementado na FX-10. Avalia o script uma vez, no topo, para coletar as declarações
//! (`fluxo`, `gatilho`, `etapa`, `decisao`, `fim`, `def`), e devolve a IR com os ids em texto e as
//! regras em forma serializável. Os limites de execução (32 KB de fonte, prazo de 1 s para compilar)
//! valem aqui. Especificação: `docs/FLUXOS_LINGUAGEM.md`.
