# AGENTS.md — POP Harness (Rust)

Mapa do código para quem (pessoa ou agente de IA) vai mexer neste app. Leia também o
[`CONTRIBUTING.md`](CONTRIBUTING.md) para o fluxo de contribuição. Para instalar, configurar e
rodar, leia o [`README.md`](README.md).

O POP Harness é independente do POP Atende. Nada no workspace importa código do POP Atende: as
menções a ele estão só em comentários e em nomes de teste.

## Antes de mexer

1. Leia o doc do assunto em `docs/` (lista na seção "Regras de negócio" abaixo). Os docs descrevem o
   desenho e os limites. Quando o código mudar o comportamento, atualize o doc do mesmo assunto.
2. Escopo e decisões de base: `docs/VISAO_GERAL.md` e `docs/DECISOES.md`. Estado das tarefas e do
   aceite: `ROADMAP.md`. Contrato dos
   canais: `docs/CONTRATO_CANAL.md`. Decisões de dados, contratos de API e arquitetura são do mantenedor.
   Quem implementa não as muda sozinho. Referências a "PRD §x", "D6" ou "HR-xx" em docs e comentários de código vêm da especificação removida em 21/11/2026; a equivalência está em `docs/VISAO_GERAL.md` §10; o commit que fechou cada tarefa a cita.
3. Não há deploy a partir deste app. Não existe script de deploy aqui: `scripts/build-dist.sh` só gera
   o pacote local. Envio para a máquina ARM64 de testes ou para produção exige autorização explícita do mantenedor.

## Regras que valem aqui

- **Segredo nunca sai.** Chaves vêm só do ambiente (`POPH_CHAVE_CIFRA`, `ANTIGRAVITY_API_KEY`,
  `TYPESAFE_API_KEY`, `EVOLUTION_API_KEY`, `POPH_CLUSTER_SECRET`). Nunca vão para arquivo, log, relatório
  ou resposta de API. O tipo `Sensivel` (em `crates/poph-types/src/segredo.rs`) imprime `[oculto]` em
  `Debug` e em `Display`. Use-o para qualquer valor sensível que passe por log ou erro.
- **Migração.** Uma migração nova é um arquivo `crates/poph-store/migrations/harness/NNNN_nome.sql`,
  registrado em `crates/poph-store/src/migracoes.rs` com o próximo número. Nunca edite uma migração
  já publicada. Quem aplica é o `poph migrate` **e também o servidor** ao abrir o banco. Confirmei
  isso: um banco novo, com o servidor subindo, já saiu na versão 2 sem `poph migrate`.
- **Token de org não faz ação global.** Criar org, criar ou revogar token global e recarregar o cluster
  exigem token global. Com token de org, a API responde `403 forbidden`. Sem token, `401`.
- **Contrato da API.** Não edite `console/src/api/schema.d.ts` à mão: ele sai de `poph openapi`, por
  `bash console/gen-api.sh`. O CI (`verificar-api.sh`) falha se o arquivo divergir do servidor.
- **Git.** Commit local segue a política do repositório. Push e deploy só com autorização do mantenedor.

## Crates (`crates/`)

Os nomes e as dependências saíram do `cargo metadata` deste workspace, não de memória.

| Crate | O que faz | Importa (dependências internas, normais) |
|---|---|---|
| `poph-types` | Tipos de domínio, IDs, erros, contratos, configuração (`config.rs`), `Sensivel` (`segredo.rs`) | nenhum (é a base) |
| `poph-core` | Cliente do provedor de LLM (módulo `llm/`, com o provedor antigravity) | `poph-types` |
| `poph-store` | Armazenamento SQLite (`harness.db` e `rag.db`): um escritor, leitores em WAL, migrações, repositórios por org, backup | `poph-types` |
| `poph-rag` | Embedding local (ONNX Runtime, int8), fila de lote com limite por org, instalação do modelo | `poph-types` |
| `poph-files` | Recebimento de arquivos: limite de tamanho, tipo pelo conteúdo, blob por hash, texto de PDF/Office extraído em processo isolado (binário `poph-extrator`) | `poph-types` |
| `poph-jev` | Cliente do Jev, framework de gates, lote de perguntas, cache de decisão | `poph-types` |
| `poph-fluxos` | Módulo de Fluxos e Fichas (FX-04: esqueleto, ainda sem lógica). Compilador de script Starlark → estrutura (IR), validador, regras, motor de execução, bloco de prompt, diferença entre revisões e formatador. Tudo puro, sem banco, sem rede: as tabelas ficam no `poph-store`, o turno liga o motor no `poph-agent` (FX-23) e a API fica no `poph-server`. Especificação da linguagem: `docs/FLUXOS_LINGUAGEM.md` | `poph-types` e a biblioteca Starlark (`starlark`, `blake3` com `pure`). Nenhum crate interno além de `poph-types` |
| `poph-plugins` | Supervisão de plugins em processo externo: protocolo JSON-lines sobre stdio, handshake, timeout, reinício com backoff | `poph-types` |
| `poph-evolution` | Cliente HTTP da EvolutionAPI (WhatsApp): envio, mídia recebida, manifesto do canal | `poph-types` |
| `poph-matrix` | Módulo Matrix (MX-07, MX-09, MX-10, MX-11, MX-12, MX-13, MX-17): configuração e segredos do Application Service; cliente da API do Matrix (salas, eventos, estado, perfil, registro de usuário virtual, `m.space.child`, resolução de alias, hierarquia do espaço); usuários virtuais e perfil (com `eh_fantasma`/`eh_bot` para o anti-eco); provisão da sala por conversa (alias determinístico, metadata do Element, `creating` → `ready`), com a persistência injetada pela porta `PortaSalas`; espaços por departamento (MX-12: resolução do departamento, criação do espaço pelo alias `#dep_<departamento>`, ligação a um espaço existente com conferência de permissão, `m.space.child`/`m.space.parent`/entrada restrita, convite em lote e troca de espaço na transferência), com a persistência injetada pela porta `PortaEspacos`; formato das mensagens espelhadas (MX-13); classificador dos eventos recebidos do Synapse (MX-17: resposta do atendente, mídia do atendente ou ruído, com motivo enumerado). As tabelas do módulo ficam no `poph-store` (`matrix_*`, MX-08) e quem implementa a porta é o `poph-server`. **`poph-agent` não importa este crate** | `poph-types` |
| `poph-agent` | Estado quente: cache de configuração do agente, ator por conversa, turno (`turno/`), janela de contexto, resumo, limites, base de conhecimento, produtos, carrinho | `poph-core`, `poph-files`, `poph-jev`, `poph-rag`, `poph-store`, `poph-types` |
| `poph-retention` | Varredura de retenção por org, blobs órfãos, simulação (`--dry-run`), auditoria | `poph-store`, `poph-types` |
| `poph-server` | API administrativa (axum), autenticação, OpenAPI, Console embutido, cluster, entrada e saída do turno, WhatsApp, cifra, receptor do Application Service do Matrix (`api/matrix_as.rs`, MX-16) a resposta do atendente ao cliente (`matrix_resposta.rs`, MX-18: implementa a entrega dos eventos e usa a operação atômica da MX-04), o aviso na sala quando essa resposta falha de vez (`matrix_falhas_aviso.rs`, MX-19) e a raiz de composição do módulo (`matrix_inicio.rs`, MX-20: liga, degrada sem derrubar o servidor, honra `matrix_orgs` a cada ciclo e para ordenado) | `poph-agent`, `poph-evolution`, `poph-files`, `poph-matrix`, `poph-plugins`, `poph-rag`, `poph-store`, `poph-types`. `poph-core` só em teste |
| `poph` | Binário único: CLI (clap) e `serve`. Um módulo por grupo de comandos (`org.rs`, `agente.rs`, `cluster.rs`, `servidor.rs`...) | quase todos os acima |
| `poph-plugin-eco` | Plugin de exemplo (texto invertido) e modelo para novos plugins | `poph-types`. Só em teste: `poph-agent`, `poph-plugins`, `poph-store` |

### Quem pode importar o quê

A regra é esta tabela. O Cargo impede ciclos, mas **não há checagem automática** da direção das
camadas: não existe nesta árvore um equivalente ao dependency-cruiser. Ao adicionar uma dependência
entre crates, confira contra a tabela. Duas regras que a tabela deixa claras:

- `poph-types` não depende de nenhum crate interno. Tudo parte dele.
- Crates de capacidade (`poph-store`, `poph-rag`, `poph-files`, `poph-jev`, `poph-plugins`,
  `poph-evolution`, `poph-matrix`, `poph-core`, `poph-fluxos`) não conhecem o servidor nem a CLI. Quem junta as peças é
  `poph-agent` (turno) e `poph-server` (HTTP).
- `poph-agent` **não** importa `poph-matrix`: o módulo Matrix é ligado pelo `poph-server` e lê o que o
  turno já gravou. Se um dia o turno precisar do módulo, isso volta ao mantenedor antes.

## Regras de negócio e onde estão

| Assunto | Código | Doc |
|---|---|---|
| Turno: da mensagem à resposta | `poph-agent/src/turno/`, `poph-server/src/entrada_turno.rs`, `saida_whatsapp.rs` | `docs/TURNO.md`, `docs/ENTRADA_E_SAIDA.md` |
| Concorrência: um ator por conversa, filas, limites | `poph-agent/src/atores.rs`, `limites.rs`, `config_quente.rs` | `docs/MEMORIA_E_CONCORRENCIA.md` |
| Contexto e resumo rolante | `poph-agent/src/contexto.rs`, `resumo.rs` | `docs/RESUMO.md` |
| Base de conhecimento e busca híbrida | `poph-agent/src/base_conhecimento.rs`, `poph-rag` | `docs/BASE_DE_CONHECIMENTO.md` |
| Produtos e `buscar_produto` | `poph-agent/src/produtos.rs`, `carrinho.rs` | `docs/PRODUTOS.md` |
| Arquivos e biblioteca do agente | `poph-files`, `poph-agent/src/biblioteca.rs` | `docs/ARQUIVOS.md`, `docs/BIBLIOTECA_ARQUIVOS.md` |
| Jev: gates e decisões rápidas | `poph-jev/` (`gates.rs`, `motor.rs`, `cache.rs`) | `docs/JEV.md` |
| Autenticação: token global e de org | `poph-server/src/auth.rs` | `docs/API.md` (seção Autenticação) |
| Cifra dos segredos guardados | `poph-server/src/cifra.rs` | `docs/CONFIGURACAO.md` |
| Cluster: mapa, impressão digital, recarga | `poph-server/src/cluster.rs`, `poph/src/cluster.rs`, `poph-types/src/config.rs` | `docs/CLUSTER.md` |
| Encaminhamento entre instâncias | `poph-server/src/encaminhamento.rs` | `docs/CLUSTER.md` (seção Encaminhamento) |
| Retenção do histórico | `poph-retention/` | `docs/RETENCAO.md` |
| Backup de uma org | `poph-store/src/backup.rs`, `poph-server/src/api/backup.rs` | `docs/BACKUP.md` |
| Plugins de integração | `poph-plugins/`, `poph-plugin-eco/` | `docs/PLUGIN_PROTOCOLO.md`, `docs/PLUGIN_EXEMPLO.md` |
| WhatsApp (EvolutionAPI) | `poph-evolution/`, `poph-server/src/evolution.rs` | `docs/EVOLUTION.md` |
| Contrato de canal e eventos do turno | `poph-types/src/integracao.rs`, `poph-agent/src/turno/mod.rs` | `docs/CONTRATO_CANAL.md` |
| Receptor do Application Service do Matrix (rotas que o Synapse chama) | `poph-server/src/api/matrix_as.rs` | `docs/API.md` (seção do Application Service) |
| Resposta do atendente no Element → cliente no WhatsApp | `poph-server/src/matrix_resposta.rs` (sobre `poph-matrix/src/eventos.rs` e a operação da MX-04 em `poph-store`) | `docs/ENTRADA_E_SAIDA.md` §5 |
| Aviso na sala quando a resposta do atendente falha de vez no WhatsApp | `poph-server/src/matrix_falhas_aviso.rs` (`AvisoDeFalhas`, ligado pelo `matrix_inicio`; tabela `matrix_avisos_falha`, migração `0009`) | `docs/ENTRADA_E_SAIDA.md` §8, `docs/DADOS.md` §11.6 |
| Espaços por departamento (o que o atendente vê organizado no Element) | `poph-matrix/src/espacos.rs` (porta `PortaEspacos`, implementada pelo `poph-server` sobre `store.matrix()`) | `docs/DADOS.md` §11.4 |
| Ligação do módulo Matrix no servidor (sobe, degrada, liga/desliga por org, para ordenado) | `poph-server/src/matrix_inicio.rs` (raiz de composição; `PortaEspacosStore` sobre `store.matrix()`) | `docs/ENTRADA_E_SAIDA.md` §8, `docs/API.md` |
| Visão geral, decisões de base, estado | (sem código) | `docs/VISAO_GERAL.md`, `docs/DECISOES.md`, `ROADMAP.md` |
| Fluxos e Fichas (linguagem, compilador, motor; FX-04: só o esqueleto) | `crates/poph-fluxos/` (esqueleto; a lógica entra nas FX-10 a FX-18) | `docs/FLUXOS_LINGUAGEM.md` (linguagem, IR e diagnósticos, FX-02) |
| Bancos, esquema e migrações | `poph-store/` (`migracoes.rs`, `migrations/harness/`, `escritor.rs`) | `docs/DADOS.md` |
| Agentes e instruções | `poph-agent/src/compositor.rs`, `poph-server/src/api/instrucoes.rs` | `docs/AGENTES.md` |

Ao mexer no Jev: ele escolhe, ranqueia ou se abstém. O backend valida e executa. Falha do Jev não pode
virar efeito. Esse princípio está em `docs/JEV.md`.

## Console (`console/`)

React, TypeScript, Vite. Ele é embutido no binário: `console/dist` entra pelo `build.rs` do
`poph-server`. Não é um pacote do monorepo `apps/frontend`.

- `src/api/`: camada de acesso à API. `cliente.ts` cuida do transporte, do token e do formato de erro
  `{ code, message, details }`. Cada área tem o próprio arquivo (`agentes.ts`, `operacao.ts`...). Os
  tipos vêm de `schema.d.ts`, gerado. Quando o contrato deixa o corpo de uma resposta como `unknown`
  (caso de `operacao.ts`), os tipos ficam escritos à mão nessa camada. Não mexa no schema gerado.
- `src/telas/`: uma tela por arquivo. Cada uma tem o teste de componente ao lado (`Tela.test.tsx`).
- `src/componentes/`: diálogos, modais, menu lateral, topo. `navegacao.ts` tem a lista do menu.
- `src/estilos/`: CSS por tela.
- Estados obrigatórios: carregando, vazio e erro. Botão de ação fica travado enquanto a requisição
  roda, para não haver duplo envio.
- Token: fica só em memória e em `sessionStorage`. `401` encerra a sessão. `403` não encerra: mostra o
  erro da chamada.
- Visual: `@sofacoding/sofa-ui` (39 arquivos usam). Fundo `#151719`, que é o que vale para o Console
  segundo `console/UX_SPEC.md`. O `#121a24` do `DESIGN.md` da raiz foi revogado para o Console. Evite
  card dentro de card: é uma preferência do mantenedor para este Console.
- Adicionar tela: entrada em `navegacao.ts`, rota em `App.tsx`, e teste de componente ao lado.

## Testes e CI

Comando único, da raiz do app:

```bash
cd apps/pop-harness
bash scripts/ci.sh
```

Ordem: poda do cache (`limpar-cache.sh`), build do bundle do Console, checagem do cliente da API,
testes do Console (`console/scripts/testar.sh`), formatação (`cargo fmt --check`), clippy com
`-D warnings`, build, testes Rust (`cargo test --workspace`), teste de RAM do modelo, checagem aarch64
dos crates sem C, `cargo deny` e `cargo audit`. Nesta máquina passou inteiro (`CI LOCAL: OK`), em cerca
de seis minutos com o cache quente.

Pulos sem Console instalado: os três passos do Console (bundle, cliente da API e testes) se declaram
pulados quando `console/node_modules` não existe, e saem com 0. Isso está no script. Não testei esse
caminho nesta máquina, porque o `node_modules` existe.

Comandos menores:

- Um crate: `cargo test --locked -p poph-retention` (executado).
- Um teste de componente: `cd console && npx vitest run src/telas/Operacao.test.tsx`.
- Todos os testes do Console: `cd console && pnpm run testar`.
- Medição de RAM do modelo, que é `#[ignore]`: `cargo test --locked -p poph-rag --test ram -- --ignored --nocapture`.

ARM: a estação não tem compilador cruzado para aarch64, então o `rusqlite` (em C) não compila lá. A
checagem ARM da estação cobre só `cargo check` nos crates sem C. O binário ARM é compilado na máquina ARM64 de testes, e
isso só com autorização do mantenedor. Veja `DEVELOPMENT.md`.

Como provar uma mudança: rode o teste do crate mexido, mais o `ci.sh` antes de entregar. Um teste que
fica verde sem o código alterado não prova nada: confirme que ele falha quando o comportamento quebra.

## Limites conhecidos

- A tela de Operação não mostra o mapa de cluster em uso, porque não existe rota de leitura. Ela só
  mostra o resultado das recargas feitas na sessão.
- Sem checagem automática das regras de dependência entre crates (veja acima).
- O modelo de embedding não foi baixado nesta máquina, que não tem acesso à internet. A busca na base e
  em produtos fica desligada até `poph model install` rodar com rede.
