# Memória e vazão do módulo Matrix (MX-30)

Este documento responde ao critério **A13**: quanto de memória o módulo Matrix custa ao `poph serve`
e quantas mensagens ele espelha por segundo. Todos os números aqui foram medidos no ambiente de
desenvolvimento, com o binário de release e o servidor de verdade; nenhum é estimado. O que não deu
para medir está marcado como tal, com o motivo.

A medição também encontrou **um defeito real no caminho de criação de sala contra um Synapse de
verdade** — a sala nasce sem nome, sem tópico e sem poder configurado. Está na seção "Defeito
achado", com a evidência; não foi corrigido porque o arquivo é de outra tarefa.

## Resumo

- **Memória com 1.000 salas:** o módulo ligado custa **+6,5 MB** sobre o servidor sem o módulo
  (mediana de 27,1 MB contra 20,4 MB, em três rodadas). A meta proposta no PRD — 20 MB com 1.000
  salas — é atendida com folga de cerca de três vezes.
- **Custo fixo:** com uma sala só, o módulo custa **+2,5 MB**. O resto cresce devagar: **+3,7 MB**
  com 100 salas, **+6,5 MB** com 1.000 e **+10,3 MB** com 10.000 — ou seja, dez vezes mais salas
  custam 1,6 vez mais memória que mil salas. De 100 para 1.000 salas são ~3,1 kB por sala; de 1.000
  para 10.000, ~0,4 kB por sala.
- **Vazão:** **739 mensagens/s** no cenário de uma mensagem por sala (com a criação das 1.000 salas
  junto) e **2.648 a 2.811 mensagens/s** no cenário de dez mensagens por sala (criação amortizada).
  O gargalo é o **Synapse falso**, que chega a 93% de um núcleo; o Harness fica em 39–64%.
- **Synapse de desenvolvimento (20 salas):** 4,8 mensagens/s. Esse número **não mede vazão de
  espelhamento** — mede o custo de criar sala no Synapse de dev (≈5 salas/s), e ainda está
  contaminado pelo defeito abaixo.

## Critério A13

| O que o A13 pede | O que foi medido | Onde |
|---|---|---|
| RAM do `poph serve` com o módulo desligado | ~20,4 MB de RSS em regime (20.272–20.984 kB), de 1 a 10.000 salas | "Números: memória" |
| RAM com o módulo ligado, em 100, 1.000 e 10.000 salas | +3,7 MB / +6,5 MB / +10,3 MB | "Números: memória" |
| Meta de 20 MB com 1.000 salas | medida em 6,5 MB; folga de ~3× | "Números: memória" |
| Vazão de mensagens espelhadas por segundo | 739 msg/s (1 msg/sala, 1.000 salas) e 2.648–2.811 msg/s (10 msgs/sala) | "Números: vazão" |
| Medição contra o Synapse de verdade | 20 salas, 5 rodadas, 4,5–4,8 msg/s — número contaminado, ver a seção | "Synapse de desenvolvimento" |

## Método

Cada rodada é uma execução independente:

1. **Semeadura** (processo separado, pela API do `poph-store`, não por SQL à mão): cria a org, o
   agente e N conversas com M mensagens do cliente cada. Com o módulo ligado, a org é ligada no
   módulo **antes** das mensagens, então tudo o que veio depois é espelhado.
2. **`poph serve` de verdade** (`target/release/poph`) sobre aquele banco, com a configuração
   padrão de produção: concorrência 8, intervalo de 500 ms, log em `info`. Nada de laço acelerado.
3. **Synapse falso em processo separado** (`scripts/medicao-memoria/synapse_falso.py`, só biblioteca
   padrão), para que o consumo dele não entre no RSS do `poph serve` e para que a escala de 10.000
   salas caiba no tempo. Ele responde as mesmas rotas que o falso dos testes do módulo: registro de
   fantasma, perfil, `createRoom` com alias, convite, entrada, membros, estado, envio com dedupe por
   `txnId` e a sonda `/_matrix/client/versions`.
4. **RSS lido do processo** (`/proc/<pid>/status`: `VmRSS` e o pico `VmHWM`), amostrado uma vez por
   segundo durante 10 s **depois** de a fila esvaziar. O número da tabela é a mediana dessas 10
   amostras; o pico é o `VmHWM`. CPU vem dos ticks de `/proc/<pid>/stat` divididos pelo tempo de
   parede.
5. **Vazão** = envios únicos contados pelo falso (dedupe por `txnId`) divididos pelo tempo entre o
   servidor responder em `/healthz` e a última mensagem chegar ao falso.

Rodadas: três por cenário em 1, 100, 1.000 e 10.000 salas; duas no cenário de vazão; cinco no
Synapse de dev. Comandos no fim do documento.

### Desvios da produção, declarados

- **Synapse falso, não o Synapse real**, para a escala de 100 a 10.000 salas: o Synapse de
  desenvolvimento cria ~5 salas por segundo, então 10.000 salas levariam cerca de meia hora por
  rodada, num ambiente que outros agentes usam. A rodada real foi feita em escala pequena (20
  salas) e é reportada à parte.
- **Falso e servidor na mesma máquina**: os 93% de CPU do falso e a contenção entre os dois
  processos tornam a vazão medida um **piso**, não um teto.
- **Binário de release**, sem `perf` nem instrumentação de alocador: o número é o RSS total do
  processo, incluindo o cache de páginas do SQLite e o alocador — não é "memória do módulo" isolada.

## Números: memória

RSS em regime (mediana de 10 amostras por rodada; kB), `poph serve` de release:

| Salas | Módulo desligado (3 rodadas) | Módulo ligado (3 rodadas) | Acréscimo |
|---|---|---|---|
| 1 | 20.152 (20.084–20.468) | 22.720 (22.664–22.776) | **+2,5 MB** |
| 100 | 20.768 (20.624–20.788) | 24.526 (24.272–24.660) | **+3,7 MB** |
| 1.000 | 20.440 (20.272–20.984) | 27.092 (26.780–27.152) | **+6,5 MB** |
| 10.000 | 20.596 (20.404–20.628) | 31.186 (31.024–31.348, 2 rodadas) | **+10,3 MB** |

Pico (`VmHWM`) nas mesmas rodadas: 20.768 kB (desligado, 100 salas) a 31.348 kB (ligado, 10.000
salas). O pico ficou entre 0 e 24 kB acima do regime em todas as rodadas — o módulo não tem picos
de alocação no ciclo normal; o que existe é o patamar.

Leitura:

- **Custo fixo de 2,5 MB** (uma sala): cliente HTTP, ids dos fantasmas, a sonda de conexão, os três
  laços, o registro de métricas e os primeiros acessos ao banco.
- **Crescimento sublinear**: 100 → 1.000 salas acrescenta 2,8 MB (~3,1 kB por sala); 1.000 →
  10.000 acrescenta 3,8 MB (~0,4 kB por sala). É o comportamento esperado de cache com teto (os
  caches de fantasma têm teto de 1.024 entradas por processo) somado ao que o banco mantém em
  memória para o cursor e para a leitura incremental.
- **Não medi a divisão por componente** (quanto é cache do módulo, quanto é página do SQLite): não
  há instrumentação de alocador no projeto e criá-la está fora do escopo desta tarefa. O que está
  medido é o total, que é o número do critério.

## Números: vazão

| Cenário | Mensagens | Tempo de drenagem | Vazão | CPU do serve | CPU do falso |
|---|---|---|---|---|---|
| Ligado, 100 salas, 1 msg/sala | 100 | 0,25 s (0,25–0,25) | 403,2 msg/s (403–407) | 8% | 8–16% |
| Ligado, 1.000 salas, 1 msg/sala | 1.000 | 1,35 s (1,35–1,36) | **739,1 msg/s** (737,5–740,7) | 61% | 92–95% |
| Ligado, 10.000 salas, 1 msg/sala | 10.000 | 14,66 s (14,65–14,66) | **682,4 msg/s** (682,0–682,7) | 64% | 93–94% |
| Ligado, 1.000 salas, 10 msg/sala | 10.000 | 3,67 s (3,56–3,78) | **2.729 msg/s** (2.648–2.811) | 39% | 70–73% |

Como ler:

- Nos três primeiros cenários, **a maior parte do tempo é criação de sala**, não envio de mensagem:
  são N salas criadas, convite do fantasma do cliente, entrada e metadata. A criação de sala roda a
  ~700–740 salas/s contra o falso.
- Só o cenário de 10 mensagens por sala separa o envio da criação: 10.000 mensagens em 3,67 s. Dali
  sai a leitura de que **o envio de mensagem em si é mais rápido que o número da tabela**: descontando
  o ~1,35 s que a criação das 1.000 salas custou no cenário anterior, sobram ~2,3 s para 10.000
  mensagens (≈4.300 msg/s). Esse número é **derivado**, não medido diretamente — a medição direta
  exigiria semear as salas e só depois injetar mensagens, e o script atual faz as duas coisas na
  mesma semeadura.
- O **falso é o gargalo** a partir de 1.000 salas: 92–95% de um núcleo contra 61–64% do Harness. A
  vazão medida é, portanto, um piso do que o módulo sustenta.
- Na escala de 10.000 salas a vazão cai 8% em relação a 1.000 (739 → 682), com a mesma CPU do
  Harness: é o custo de manter 10.000 salas no banco e no laço, não uma degradação do envio.

## Ruído

- **Entre rodadas do mesmo cenário** (o que mais importa para comparar com/sem módulo): o RSS sem o
  módulo variou 712 kB (20.272–20.984 kB); com o módulo, 372 kB em 1.000 salas. O acréscimo medido
  (6,5 MB) é **nove vezes** essa variação.
- **Na vazão:** a faixa entre rodadas foi de 3,2 msg/s em 1.000 salas (0,4% da mediana) e de
  162 msg/s no cenário de 10 mensagens por sala (5,9% da mediana).
- **No tempo de drenagem:** ±0,11 s em 10 mensagens por sala; ±0,01 s nos cenários de uma mensagem.
- **Execuções**: o roteiro principal rodou uma vez inteiro (três rodadas em cada cenário de 100,
  1.000 e 10.000 salas, duas no de vazão). Depois disso, o cenário de uma sala (custo fixo) e o
  modo real foram rodados em execuções separadas — o real cinco vezes no total, com os resultados
  dentro da faixa de 4,5–4,8 msg/s.

## Duração real

O roteiro completo (100/1.000/10.000 salas, desligado e ligado, vazão e real) levou **7 min 27 s**
de parede (03:10:24 a 03:17:51 UTC, medidos pelos carimbos dos logs do servidor):

| Cenário | Semeadura | Drenagem | Amostragem | Total por rodada |
|---|---|---|---|---|
| Desligado, 100–1.000 salas | 0,2–0,7 s | — | 10 s | ~11 s |
| Desligado, 10.000 salas | 4,6–4,8 s | — | 10 s | ~15–16 s |
| Ligado, 100–1.000 salas, 1 msg | 0,2–0,7 s | 0,02–1,35 s | 10 s | ~11–12 s |
| Ligado, 10.000 salas | 4,8 s | 14,66 s | 10 s | ~30 s |
| Ligado, 1.000 salas, 10 msgs | 2,0–2,1 s | 3,56–3,78 s | 10 s | ~16 s |
| Real, 20 salas | 0,2 s | 4,18–4,40 s | 10 s | ~15 s |

A maior parte de cada rodada é a janela fixa de 10 s de amostragem do RSS, não o trabalho.

## Synapse de desenvolvimento (MX-01, porta 18008)

Cinco rodadas de 20 salas e 20 mensagens:

| Rodada | RSS de subida | RSS em regime | Drenagem | Vazão |
|---|---|---|---|---|
| 1 | 23.464 kB | 24.056 kB | 4,40 s | 4,5 msg/s |
| 2 | 23.672 kB | 24.116 kB | 4,19 s | 4,8 msg/s |
| 3 | 23.180 kB | 23.644 kB | 4,19 s | 4,8 msg/s |
| 4 | 22.852 kB | 23.316 kB | 4,19 s | 4,8 msg/s |
| 5 | 23.120 kB | 23.644 kB | 4,18 s | 4,8 msg/s |

**Esses números não medem a vazão do módulo.** Nas cinco rodadas, as 20 salas falharam na primeira
tentativa e só entraram na passagem seguinte do laço; o tempo de 4,2 s é o custo de criar sala no
Synapse de dev **mais** uma volta do laço. A memória da rodada real (23,3–24,1 MB em regime) é
coerente com a das rodadas locais na mesma escala, mas o ambiente é compartilhado com outros
agentes, então não serve como medida de referência. A causa da falha está abaixo.

## Defeito achado (não corrigido)

**A sala criada contra um Synapse de verdade nasce sem nome, sem tópico e sem o poder configurado.**

O que acontece: o `createRoom` pedido pelo módulo manda `power_level_content_override` com o fantasma
do agente no nível de `matrix.poder_agente`, cujo padrão é **50** (`crates/poph-types/src/config.rs`,
`poder_agente: 50`). No Synapse real, quem cria a sala precisa de nível **100** para escrever o
próprio `m.room.power_levels`: com 50, a requisição volta **403** — depois de a sala e o alias já
existirem. A passagem seguinte acha o alias (`M_ROOM_IN_USE` → resolve o alias → segue), então o
Harness "recupera", mas a sala **fica sem `m.room.name` e sem `m.room.topic`**, e sem o poder que o
módulo queria aplicar.

Evidência (capturada por um proxy de diagnóstico entre o `poph serve` e o Synapse de dev, que
registra método, caminho, status e `errcode` — sem token):

```
POST /_matrix/client/v3/createRoom user=@ph_ag_… status=403 errcode=M_FORBIDDEN
  erro=You don't have permission to post that to the room. user_level (50) < send_level (100)
POST /_matrix/client/v3/createRoom user=@ph_ag_… status=400 errcode=M_ROOM_IN_USE
  erro=Room alias already taken
```

Estado da sala criada nessa rodada, lido pela API do Matrix como o fantasma do agente (que é o
criador): `m.room.name` → `404 M_NOT_FOUND`, `m.room.topic` → `404`, `m.room.power_levels` → `404`,
enquanto o `com.sofacoding.atendimento.metadata` está presente (200). A sala existe, os dois membros
estão dentro, e ela está **sem nome**.

Contraprova, com o mesmo corpo e só o poder trocado:

| `power_level_content_override` do criador | `createRoom` | `m.room.name` depois |
|---|---|---|
| 50 | **403** `M_FORBIDDEN` (`user_level (50) < send_level (100)`) | `404 M_NOT_FOUND` |
| 100 | **200** | `{"name":"Nome Visivel"}` |

Impacto no produto: é o critério **A1** ("nome do cliente e do agente corretos" na sala) falhando no
Synapse de verdade — a sala aparece no Element com o identificador técnico, não com o nome do
cliente. Além disso, cada conversa paga uma chamada condenada e uma volta do laço antes de a sala
ficar utilizável. Os testes locais não pegam isso porque o Synapse falso não valida poder.

Não corrigi: o arquivo é da MX-11 e a regra desta tarefa é parar e devolver com a
evidência. A correção provável é não rebaixar o **criador** abaixo de 100 e aplicar o poder do
fantasma do agente só depois da criação (ou usar `events`/`users_default` em vez de sobrescrever o
criador). Também vale um teste no Synapse real, já que o falso não reproduz.

### Como reproduzir o defeito (contra o Synapse de dev, sem o Harness)

Com o `$TOKEN` do Application Service do Harness e `$BASE` do Synapse (`http://127.0.0.1:18008`):

1. `POST /_matrix/client/v3/register?kind=user` com
   `{"type":"m.login.application_service","username":"ph_ag_pl50","inhibit_login":true}`.
2. Criar a sala **com o poder que o módulo manda hoje**, como o fantasma recém-registrado:
   `POST /_matrix/client/v3/createRoom?user_id=@ph_ag_pl50:matrix-dev.local` com
   `{"room_alias_name":"ph_c_pl50","name":"Nome Visivel","topic":"Atendimento",
   "creation_content":{"type":"com.sofacoding.atendimento"},
   "power_level_content_override":{"users":{"@ph_ag_pl50:matrix-dev.local":50}},
   "preset":"private_chat"}` → **403** `M_FORBIDDEN` (`user_level (50) < send_level (100)`).
3. Repetir com `100` no lugar de `50` → **200**, e
   `GET /_matrix/client/v3/rooms/{room_id}/state/m.room.name?user_id=@ph_ag_pl50:matrix-dev.local`
   devolve `{"name":"Nome Visivel"}`. Com 50, a mesma leitura devolve `404 M_NOT_FOUND` — a sala
   existe e não tem nome.

O mesmo acontece pelo caminho do Harness: suba o `poph serve` com `[matrix]` apontando para o Synapse
de dev e o banco de uma rodada real; no log aparecem as falhas `motivo=sala_rejeitada` na primeira
passagem e o espelhamento completa na seguinte, com as salas sem `m.room.name`.

## O que NÃO foi medido

- **RAM com o Synapse real em escala**: 10.000 salas a ~5 salas/s dariam cerca de meia hora por
  rodada, num ambiente que outros agentes usam. O número de 10.000 salas é do Synapse falso.
- **Vazão de envio isolada da criação de sala**: exigiria semear as salas, esperar o módulo
  estabilizar e só então injetar mensagens. O script atual semeia as duas coisas juntas; o número de
  ~4.300 msg/s que aparece na leitura é derivado, não medido.
- **A divisão do acréscimo de memória por componente** (cache do módulo × página do SQLite ×
  alocador): sem instrumentação de alocador.
- **Comportamento depois de horas de operação** (fragmentação, crescimento do WAL): a rodada mais
  longa foi de 30 s.
- **O estado visual no Element** (A1 de verdade, com o nome na tela): é a MX-31, de outro dono.
- **Duas instâncias do Harness** disputando a mesma máquina: fase 3.

## Limites conhecidos

- **O RSS é do processo inteiro**, não do módulo: inclui o cache de páginas do SQLite e o que o
  alocador não devolve ao sistema. Serve para comparar com/sem módulo na mesma máquina, não como
  promessa absoluta em outro hardware.
- **O falso é mais rápido que qualquer Synapse real**; a vazão de 739–2.811 msg/s é o que o módulo
  faz quando o outro lado acompanha, e o teto real será do Synapse.
- **A rodada real ficou contaminada** pelo defeito do poder (uma volta de laço a mais em todas as
  salas), então 4,8 msg/s é um piso pessimista, não uma medida limpa.
- **A janela de amostragem de 10 s** pega o regime, não o pico de uma rajada; o `VmHWM` cobre o pico
  de alocação e foi igual ao regime.
- **O mesmo SSD e a mesma CPU** hospedam o servidor, o falso e o banco: a vazão absoluta varia com a
  carga da máquina (as cinco rodadas reais mostram a variação).
- **O script não apaga os bancos** de `target/mx30/`: são descartáveis e ficam fora do git.

## Como reproduzir

```bash
cd apps/pop-harness
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release --locked -p poph          # o script usa target/release/poph

bash scripts/medir-memoria.sh --modo desligado --salas 1000 --rodada 1
bash scripts/medir-memoria.sh --modo ligado   --salas 1000 --mensagens 1  --rodada 1
bash scripts/medir-memoria.sh --modo ligado   --salas 1000 --mensagens 10 --rodada 1   # vazão
bash scripts/medir-memoria.sh --modo real     --salas 20   --rodada 1                  # Synapse de dev

bash scripts/medir-memoria-roteiro.sh           # a matriz completa (7–8 min)
bash scripts/medir-memoria-roteiro.sh rapido    # 100 salas, checagem rápida
```

Cada rodada grava uma linha em `target/mx30/memoria.csv` (fora do git), com RSS de subida, regime,
pico, tempo de drenagem, vazão, CPU dos dois processos, salas criadas, envios únicos, convites,
requisições e o tempo de semeadura. Os logs de cada rodada ficam em `target/mx30/logs/`.

O modo `real` lê as credenciais do Synapse de desenvolvimento de `~/.cache/poph-matrix/credenciais.env`
(geradas por `scripts/matrix-dev.sh`) e não imprime token nenhum: os tokens vão por variável de
ambiente e nunca para a linha de comando nem para o CSV.
