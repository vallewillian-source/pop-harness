# Changelog

Todas as mudanças relevantes deste projeto ficam neste arquivo.

O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e o projeto segue o
[Versionamento Semântico](https://semver.org/lang/pt-BR/). Na série `0.x`, qualquer versão pode trazer
mudança incompatível; ela vem sempre marcada como **Quebra** nas notas.

## [Não lançado]

### Adicionado

- Motor de atendimento multi-tenant em Rust: orgs, agentes com instruções estruturadas, departamentos,
  categorias, produtos com carrinho de orçamento e biblioteca de arquivos por agente.
- Base de conhecimento com busca híbrida (vetor e texto) e embedding local em ONNX (int8).
- Jev com oito gates de decisão rápida (G1 a G8), medidos contra conjuntos rotulados, e o G9 para iniciar fluxos.
- WhatsApp pela EvolutionAPI como canal nativo e contrato de canal em processo externo (JSON-lines por stdio).
- Fluxos escritos em Starlark, com revisões imutáveis, permissão por agente e Fichas por atendimento.
- Módulo Matrix desligável: sala por conversa, espaços por departamento, resposta do atendente no Element
  e pausa do agente.
- API administrativa em `/admin/v1` com OpenAPI 3.1, Console web embutido e CLI `poph`.
- Roteamento estático entre instâncias com encaminhamento assinado, backup e restauração de org,
  retenção do histórico, métricas Prometheus e `poph doctor`.
- Arquivos de comunidade: licença, guia de contribuição, código de conduta, política de segurança,
  modelos de issue e de pull request.
