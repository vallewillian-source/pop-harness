# POP Harness (Rust): visão geral

> Ponto de partida do projeto. Diz o que o Harness é, o que está dentro e fora, as decisões de base e
> o caminho de uma mensagem de ponta a ponta. O que falta para a `1.0` está em [`../ROADMAP.md`](../ROADMAP.md).
> O contrato dos canais está em [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md). Os identificadores `HR-xx`, `RL-xx`,
> `MX-xx`, `FX-xx` e `RP-xx` que aparecem em comentários e documentos são números de tarefa do histórico do
> projeto: o commit que fechou a tarefa os cita. A especificação original (PRD e lista de tarefas) não faz
> parte deste repositório; o que valia dela está aqui, em `DECISOES.md` e nos docs por assunto.

## 1. O que é

Motor de atendimento por IA, multi-tenant, em Rust: um binário (`poph`) que roda os agentes de várias
organizações na mesma instância. Tem três entradas: a CLI, a API administrativa (`/admin/v1`) e o
Console web (React, embutido no binário), usado pelo time interno para administrar e testar.

É independente do POP Atende: não usa o Postgres dele, não importa código dele e não substitui o motor
dele (a troca do motor LangGraph só será discutida depois de o Harness estar estável).

Prioridades, nesta ordem: (1) entregar a mensagem o mais rápido possível; (2) baixo uso de RAM;
(3) escala vertical (milhares de agentes) e horizontal (várias instâncias); (4) minimalismo (poucas
dependências, poucos processos, sem framework de agente).

## 2. O que está dentro

- Orgs, categorias de produto, agentes, departamentos (com departamento padrão), instruções
  estruturadas, base de conhecimento, produtos com carrinho de orçamento, biblioteca de arquivos do
  agente e integrações.
- Busca própria (RAG): embedding local, busca híbrida (vetor + texto) com corte de relevância.
  [`BASE_DE_CONHECIMENTO.md`](BASE_DE_CONHECIMENTO.md).
- Jev (decisões rápidas) em 9 pontos, G1 a G9 (o G9 inicia fluxos), cada um atrás de flag. [`JEV.md`](JEV.md).
- LLM atrás de uma interface; hoje um provedor, compatível com a API de chat da OpenAI, com o modelo configurável por agente.
- Canais como integrações com um contrato único: WhatsApp pela EvolutionAPI (módulo nativo) e plugin em
  processo externo. [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md), [`PLUGIN_PROTOCOLO.md`](PLUGIN_PROTOCOLO.md).
- Arquivos: imagens e PDF/Office (como texto) recebidos; arquivos da biblioteca do agente enviados.
  [`ARQUIVOS.md`](ARQUIVOS.md).
- Histórico local com retenção configurável, backup/restauração de org, roteamento estático entre
  instâncias. [`RETENCAO.md`](RETENCAO.md), [`BACKUP.md`](BACKUP.md), [`CLUSTER.md`](CLUSTER.md).
- Fichas e fluxos: toda conversa nasce com uma Ficha; fluxos são scripts em Starlark, da org, com revisões
  imutáveis e permissão por agente. [`FLUXOS_LINGUAGEM.md`](FLUXOS_LINGUAGEM.md).
- Matrix e atendimento humano: módulo desligável (`poph-matrix`) que espelha a conversa numa sala do
  Element, recebe a resposta do atendente e pausa o agente. [`MATRIX_AMBIENTE_LOCAL.md`](MATRIX_AMBIENTE_LOCAL.md),
  [`DADOS.md`](DADOS.md) §11.

## 3. O que está fora

Documentos PDF e templates; calendário; e-mail; widget de site; perfil persistente do cliente (a memória é
a janela da conversa com resumo rolante); transcrição de áudio e vídeo; OCR de PDF escaneado; plugins WASM;
regras de rota por intenção (D-06); editor visual de fluxos; retenção própria das Fichas. Veja também as
lacunas conhecidas em [`../ROADMAP.md`](../ROADMAP.md).

## 4. Decisões de base (D1 a D13)

Estas decisões vieram da especificação original e continuam valendo. As posteriores estão em
[`DECISOES.md`](DECISOES.md) (D-01 em diante, com prefixo hífen para não confundir).

| # | Decisão |
|---|---|
| D1 | Rust; workspace Cargo na raiz do repositório, `console/` ao lado; binário `poph`. |
| D2 | Plugins de integração: nativo e processo externo (JSON-lines sobre stdio). WASM fica para depois. |
| D3 | Histórico de conversas e mensagens local, no SQLite, com retenção configurável por org (padrão 30 dias). |
| D4 | Roteamento estático por org entre instâncias. |
| D5 | Arquivos do cliente: imagens e PDF/Office como texto. Sem áudio. |
| D6 | LLM: antigravity2api, modelo `gemini-3.8-flash-tiered` por padrão, configurável por agente. |
| D7 | Embedding: mesmo modelo do POP Atende (`paraphrase-multilingual-MiniLM-L12-v2`), local. |
| D8 | O Jev decide o que é escolha entre opções conhecidas; o LLM só escreve texto. O Jev escolhe, ranqueia ou se abstém; o código valida e executa; falha do Jev cai no comportamento sem ele. |
| D9 | Console administrativo interno, com login por token (global ou de org). |
| D10 | Sem Redis, sem Postgres, sem framework de agente. |
| D11 | Mensagem ao cliente é entregue pelo menos uma vez: se o envio cair no meio, reenvia, aceitando duplicata. |
| D12 | Conversa transferida para humano deixa de ser respondida pelo agente até ser reaberta; as mensagens do cliente continuam gravadas. |
| D13 | O agente só envia arquivos da biblioteca do agente, nunca caminhos livres. |

## 5. Uma mensagem de ponta a ponta

Detalhe em [`TURNO.md`](TURNO.md) e [`ENTRADA_E_SAIDA.md`](ENTRADA_E_SAIDA.md).

1. **Entrada.** O canal (webhook da Evolution, plugin, chat do Console ou CLI) entrega a mensagem. Ela
   é gravada na `inbox` com chave única por canal e id externo (duplicata é descartada) e só então o
   webhook recebe OK. Se a org é de outra instância, o pedido é encaminhado à dona.
2. **Fila por conversa.** Um ator por conversa, um turno por vez. Conversa ociosa sai da memória.
3. **Janela de agrupamento.** Espera curta (padrão 2 s, teto 8 s) para juntar mensagens em sequência; o
   G2 do Jev pode encurtar ou alongar.
4. **Em paralelo:** uma chamada ao Jev com as perguntas de todos os gates ligados, e a busca na base e
   nos produtos. Se o G1 manda transferir, o turno termina sem chamar o LLM.
5. **Laço do agente.** Prompt com prefixo estável (para o cache do provedor) e bloco volátil depois;
   LLM com streaming; ferramentas habilitadas para o agente (`consultar_base`, `buscar_produto`,
   `enviar_arquivo`, carrinho). Texto de arquivo e trecho recuperado entram marcados como dado, nunca
   como instrução.
6. **Saída.** A resposta vai para a `outbox` (idempotente por turno e sequência); o canal envia pela
   mesma integração de onde veio a mensagem. Estados: `pending`, `sending`, `sent`, `failed`.
7. **Registro.** O turno grava decisões do Jev, trechos recuperados, ferramentas, tempos, tokens; o
   Console mostra isso no painel "por quê".

## 6. Dados e isolamento

Memória (estado quente, perdido no restart), `harness.db` (SQLite durável), `rag.db` (SQLite
reconstruível) e blobs em disco por org e hash. Toda tabela de negócio tem `org_id` e todo acesso filtra
por ele; índice vetorial e caches são particionados por org. Detalhe em [`DADOS.md`](DADOS.md).
Segredos ficam cifrados em repouso, só do ambiente, e nunca em log. Tokens de admin são guardados como
hash, com escopo global ou de org.

## 7. Console: regras de design

Valem para qualquer tela nova ([`CONSOLE.md`](CONSOLE.md), [`../console/UX_SPEC.md`](../console/UX_SPEC.md)):
sem cartões, e nunca cartão dentro de cartão (seções separadas por título e linha de 1 px); fundo
`#151719`; acento âmbar só em ação primária, linha selecionada e item ativo; status como ponto de 6 px
mais texto; slugs, ids, portas e comandos em fonte monoespaçada; foco visível e uso por teclado; estados
vazio, carregando e erro em toda tela. Rótulos em caixa alta e contadores no menu seguem o desenho aprovado do layout.

## 8. Riscos que continuam abertos

| Risco | Situação |
|---|---|
| Paridade de comportamento com o motor LangGraph | Não medida com agente real. O POP Bench pode servir de base. |
| RAM e vazão com milhares de agentes | Metas fixadas em D-07; o teste de carga (HR-55) está suspenso, então nada foi confirmado em escala. |
| Build ARM64 | Validado na máquina ARM64 de testes num commit de 20/11; builds posteriores não foram reconstruídos lá. |
| Um escritor SQLite por instância | Medido no spike; sem teste de carga completo. |
| `ort` ainda em release candidate | Versão fixada; a troca fica isolada pela interface do embedder. |
| WhatsApp real | Provado só com Evolution falsa e ambiente isolado; teste com número real pendente (não bloqueia). |

## 9. Onde está cada assunto

| Assunto | Documento |
|---|---|
| O que falta para a `1.0` | [`../ROADMAP.md`](../ROADMAP.md) |
| Canais e contrato de integração | [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md) |
| Decisões posteriores | [`DECISOES.md`](DECISOES.md) |
| Configuração e variáveis | [`CONFIGURACAO.md`](CONFIGURACAO.md) |
| Dados, esquema, concorrência | [`DADOS.md`](DADOS.md), [`MEMORIA_E_CONCORRENCIA.md`](MEMORIA_E_CONCORRENCIA.md) |
| API administrativa | [`API.md`](API.md) |
| Operação | [`RUNBOOKS.md`](RUNBOOKS.md), [`../DEVELOPMENT.md`](../DEVELOPMENT.md) |
| Mapa do código e regras de trabalho | [`../AGENTS.md`](../AGENTS.md) |

## 10. Referências antigas ("PRD §x", "D6", "HR-xx")

Comentários no código Rust e alguns docs ainda citam seções do PRD removido. Equivalência:

| Citação | Onde está agora |
|---|---|
| §3 (escopo) | seções 2 e 3 deste documento |
| §4 (decisões D1–D13) | seção 4 deste documento |
| §5.2, §5.3, §5.4 (crates, dependências) | [`../AGENTS.md`](../AGENTS.md), [`../DEVELOPMENT.md`](../DEVELOPMENT.md), `DECISOES.md` D-08 |
| §6, §6.1, §6.3 (turno, prompt, janela) | [`TURNO.md`](TURNO.md) |
| §7, §7.1, §7.2, §7.3 (dados) | [`DADOS.md`](DADOS.md) |
| §7.4 (retenção) | [`RETENCAO.md`](RETENCAO.md) |
| §8.1 (API) | [`API.md`](API.md) |
| §8.3 (contrato de integração) | [`CONTRATO_CANAL.md`](CONTRATO_CANAL.md) |
| §8.4 (provedor de LLM) | [`CONFIGURACAO.md`](CONFIGURACAO.md) (`llm.*`) |
| §8.5, §10 (Jev) | [`JEV.md`](JEV.md) |
| §9 (RAG) | [`BASE_DE_CONHECIMENTO.md`](BASE_DE_CONHECIMENTO.md), `DECISOES.md` D-09 |
| §11 (arquivos) | [`ARQUIVOS.md`](ARQUIVOS.md) |
| §12 (cluster) | [`CLUSTER.md`](CLUSTER.md) |
| §13 (Console) | [`CONSOLE.md`](CONSOLE.md) e a seção 7 deste documento |
| §14, §15 (segurança, observabilidade) | seção 6 deste documento, [`CONFIGURACAO.md`](CONFIGURACAO.md), [`RUNBOOKS.md`](RUNBOOKS.md) |
| §16 (riscos) | seção 8 deste documento |
| §17 (critério de aceite) | [`../ROADMAP.md`](../ROADMAP.md) |
| HR-xx, RL-xx, MX-xx, FX-xx, RP-xx | commit que fechou a tarefa (`git log --grep`) |

Os mesmos comentários citam `PARIDADE_AGENTE.md` (o que o POP Harness herdou do POP Atende), `ESTADO.md`
(lista de tarefas interna) e `spike/` (protótipos de medição). Nenhum dos três é publicado; o conteúdo que
importa para quem usa o projeto está nos documentos desta pasta.
