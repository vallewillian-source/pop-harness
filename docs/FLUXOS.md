# Fluxos e Fichas — medidas

> Este arquivo nasceu na FX-36 (medição de RAM, vazão e regressão) e traz **só** a seção
> "Medidas". As outras seções — visão, ciclo da Ficha, execução, efeitos, operação e limites — são
> da FX-38. Nada aqui é promessa de desenho: são números medidos, com o comando que os reproduz.

## Medidas

**Máquina:** Fedora 44, x86_64, AMD Ryzen 5 5600GT (6 núcleos, 12 threads), 32 GB de RAM.
**Data:** 23/11/2026, entre 19:15 e 19:25 (UTC-03:00).
**Estação compartilhada:** outros builds rodavam ao mesmo tempo (`load average` entre 5 e 7 nos
momentos das medidas). Por isso toda comparação antes/depois foi feita **intercalada**, na mesma
condição de carga: os números absolutos de hoje são menores que os da tabela de `DADOS.md`
(estação ociosa quando foram tomados), mas a comparação entre as duas versões não é afetada.

Todas as medidas usam binários `release`, `PATH` com `~/.cargo/bin` e `CARGO_INCREMENTAL=0`, em
`apps/pop-harness`.

### 1. RAM do processo com revisões carregadas no cache

Comando (o teto do cache vem de `POPH_RAM_TETO_MB` e cada valor roda num processo próprio, porque o
alocador não devolve ao sistema o que uma fase liberou):

```text
POPH_RAM_TETO_MB=4  cargo test --release -p poph-fluxos --test ram -- --ignored --nocapture
POPH_RAM_TETO_MB=16 cargo test --release -p poph-fluxos --test ram -- --ignored --nocapture
```

Revisões de 8 etapas com instruções de 200 caracteres: peso estimado pelo cache
(`peso_da_ir` = JSON × 2 + 256 B) **7.592 B** por revisão.

| Revisões carregadas | Teto de 4 MB (cache enche e expulsa) | Teto padrão de 16 MB (as 1.000 cabem) |
|---|---|---|
| 0 | 9.092 KB | 8.924 KB |
| 100 | 9.856 KB (+764 KB · **7.823 B por revisão**) | 9.688 KB (+764 KB · **7.823 B por revisão**) |
| 1.000 | 13.060 KB (+3.968 KB · 4.063 B por revisão) | 15.956 KB (+7.032 KB · **7.201 B por revisão**) |

O que o cache diz ter guardado em cada caso:

| Teto | Entradas | Expulsões | Não cacheadas | Bytes guardados |
|---|---|---|---|---|
| 4 MB | 551 | 449 | 0 | 4.187.604 B |
| 16 MB | 1.000 | 0 | 0 | 7.599.572 B |

Leitura: o RSS acompanha as revisões que **cabem** no teto e para de crescer quando o teto é
atingido (4 MB guardados → ~4 MB de RSS a mais, com 449 revisões já expulsas). Com o teto padrão,
1.000 revisões custam ~7 MB de RSS e ainda sobram ~9 MB de folga dentro dos 16 MB de
`fluxos.cache_revisoes_mb`.

**Contra a meta [proposta]:** a FX-01 propôs **≤ 300 KB por fluxo carregado**. A revisão medida
custa **7,2 a 7,8 KB** reais — ~2,6% da meta. Limite: o fluxo medido tem 8 etapas e a meta fala de
40 nós; extrapolando linearmente em número de nós (o que **não** foi medido), 40 nós ficariam em
~36–39 KB, ainda muito abaixo da meta.

### 2. Vazão de escrita de mensagens antes e depois da Ficha na conversa

Comando:

```text
cargo test --release -p poph-store --test vazao -- --ignored --nocapture
```

"Antes" é uma cópia isolada da árvore (worktree próprio em `/tmp`, alvo de build próprio) com **uma
única** diferença: a criação da Ficha removida de `Conversas::criar`. É o caminho de escrita de
antes da FX-07, montado à mão porque aquele estado não está no histórico deste repositório. As
rodadas foram intercaladas (antes, depois, antes, depois…), três de cada lado.

| Cenário | Antes (sem Ficha) | Depois (com Ficha) | Diferença |
|---|---|---|---|
| uma thread, sem lote | 19.232 · 22.460 · 23.349 msg/s | 23.243 · 22.355 · 19.986 msg/s | +0,8% (médias), faixas sobrepostas |
| 32 concorrentes, lote de 512 | 61.923 · 65.447 · 66.832 msg/s (média 64.834) | 62.991 · 64.639 · 57.075 msg/s (média 61.568) | **−5,0%** |
| 256 concorrentes, lote de 512 | 71.577 · 70.779 · 67.051 msg/s (média 69.802) | 67.707 · 61.271 · 65.527 msg/s (média 64.835) | **−7,1%** |
| chegada constante de 2.000 msg/s | p50 54–76 µs, p95 87–111 µs | p50 59–69 µs, p95 96–115 µs | p95 +4 a +9 µs |

**Regressão acima de 10%: nenhuma.** A maior diferença é −7,1% com 256 escritores, dentro da
dispersão das três rodadas e com as faixas sobrepostas (a rodada mais lenta de um lado é pior que a
mais rápida do outro). As duas versões ficam **acima** da meta de D-07 (≥ 50 mil msg/s) e **abaixo**
do p95 de 5 ms na chegada de 2.000 msg/s.

O que a Ficha custa de verdade é na **criação de conversa**, que é o caminho raro (uma vez por
atendimento, não por mensagem). Medida isolada, 200 amostras por rodada, três rodadas de cada lado,
no mesmo par de árvores:

| `Conversas::criar` | Rodadas (p50 / p95) | Vazão |
|---|---|---|
| Antes, sem Ficha | 84 / 156 µs · 81 / 125 µs · 79 / 126 µs | 10.160 a 11.465 conversas/s |
| Depois, com Ficha | 281 / 402 µs · 292 / 432 µs · 281 / 410 µs | 3.262 a 3.382 conversas/s |

Ou seja: **+~200 µs por conversa** (p50 ~80 → ~285 µs, cerca de 3,5×), pagos uma vez por
atendimento. Mensagem não consulta nem escreve Ficha, e é isso que a tabela acima mostra.

### 3. Latência da transição atômica (`aplicar_transicao`)

Comando:

```text
cargo test --release -p poph-store --test transicao_latencia -- --ignored --nocapture
```

Duas rodadas; cada uma com banco próprio e as amostras em série (uma thread). A transição é uma
escrita só no escritor do `poph-store`.

| Cenário | p50 | p95 | p99 | Vazão em série |
|---|---|---|---|---|
| na etapa (estado + 2 eventos + 1 efeito) | 211 · 219 µs | 248 · 344 µs | 346 · 428 µs | 4.244 a 4.614/s |
| avançando de etapa (estado + 3 eventos + linha da etapa nova) | 297 · 301 µs | 454 · 491 µs | 486 · 537 µs | 2.933 a 3.027/s |
| `fim` que encerra o atendimento (fecha conversa **e** Ficha) | 406 · 427 µs | 503 · 684 µs | 567 · 800 µs | 2.312 a 2.543/s |

Leitura: a transição mais cara (a que encerra o atendimento e fecha conversa e Ficha na mesma
escrita) fica em **~0,4 ms p50 e ~0,7 ms p95** — dentro do p95 de 5 ms que D-07 fixa para escrita, e
muito abaixo do custo de uma chamada ao modelo de texto. Não há meta específica para a transição no
PRD; estes são os primeiros números.

### 4. Custo do G9 no lote, com 20 fluxos

Comando:

```text
cargo test --release -p poph-jev --test g9_medicao -- --ignored --nocapture
```

Estado de um turno com base disponível e uma resposta do agente, para o lote ter mais de uma
pergunta (G3 e G8 também perguntam). Texto de gatilho de cada fluxo no máximo que a pergunta aceita
(400 caracteres), que é o pior caso. O tempo medido é o do Harness **antes** da chamada — montar o
estado, montar as perguntas e serializar o pedido (`JevFalso`, sem rede). Duas rodadas.

| Lote | Perguntas | p50 | p95 | Pedido serializado | Estado | Perguntas |
|---|---|---|---|---|---|---|
| sem fluxo liberado (G9 inaplicável) | 2 | 30 · 32 µs | 56 · 76 µs | 2.521 B | 444 B | 2.077 B |
| 20 fluxos, G9 ligado | 3 | 181 · 183 µs | 250 · 266 µs | **42.057 B** | **19.399 B** | **22.658 B** |
| 20 fluxos, G9 desligado pela flag | 2 | 88 µs | 113 · 115 µs | 21.476 B | 19.399 B | 2.077 B |

Leitura:

- o G9 no topo (20 fluxos com textos longos) acrescenta **uma pergunta**, **~22,7 KB de critérios** e
  **~19,4 KB de estado** ao pedido — o pedido vai de ~2,5 KB para ~42 KB, e o custo de montagem e
  serialização vai de ~30 µs para ~180 µs;
- o estado leva os 20 fluxos **mesmo com a flag do gate desligada** (21,5 KB de pedido): desligar o
  G9 economiza a pergunta, não o estado;
- o tamanho do pedido é o que vira token de entrada no Jev; quanto isso custa em dinheiro e latência
  é a medição com o Jev real (FX-37), que depende de autorização.

### Comparação com as metas e com D-07

| Meta | Número | Resultado |
|---|---|---|
| FX-01 [proposta]: RAM por fluxo carregado ≤ 300 KB | 300 KB | **atendida**: 7,2–7,8 KB por revisão de 8 etapas |
| D-07: escrita de mensagens ≥ 50 mil msg/s | 50.000 | **atendida**: 57.075–66.832 msg/s com 32 e 256 escritores |
| D-07: p95 da escrita ≤ 5 ms com 2.000 msg/s | 5 ms | **atendida**: 96–115 µs |
| `fluxos.cache_revisoes_mb` = 16 MB (PRD §9) | 16 MB | **respeitada**: 1.000 revisões cabem em 7,6 MB guardados; acima do teto, para de crescer |
| PRD §11: nenhuma chamada de rede dentro da transação | — | **confirmada**: 0,4–0,7 ms p95 na transição que fecha o atendimento |
| CI e resto de D-07 (12 KB por agente ocioso, 150 MB com 10 mil agentes, ARM64) | — | **não medido aqui** — continua pendente, como antes desta tarefa |

Nenhuma meta nova virou oficial com esta medição: os números acima são medição de conformidade com
as metas propostas, não calibração delas. A HR-55 (teste de carga em escala) continua suspensa e
**não** foi reaberta.

### O que não foi medido

- **G9 com o Jev real** (FX-37): tempo de resposta do provedor, acerto da escolha e custo em tokens.
  A FX-36 mede só o que o Harness gasta antes de chamar.
- **Turno completo**: tempo de ponta a ponta de uma mensagem do cliente até a resposta com uma etapa
  de fluxo, e o custo do bloco de etapa no prompt (FX-18/FX-23 já medem o bloco em caracteres).
- **Entrega de efeitos** (FX-21): vazão da fila e tempo de `mensagem_ao_cliente` no WhatsApp real.
- **ARM64 e carga em escala**: exigem a VM3 e a HR-55, com autorização.
- **Fluxo de 40 nós**: a medida de RAM usou um fluxo de 8 etapas; a comparação com a meta da FX-01
  para 40 nós é extrapolação linear, não medida.
