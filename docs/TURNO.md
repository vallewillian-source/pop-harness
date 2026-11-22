# Turno: motor mínimo e `poph chat` (HR-13)

> Marco da fase 1: a conversa funciona de ponta a ponta pela CLI. Este documento explica o desenho,
> as decisões, o que a próxima tarefa consome e o que **não** está coberto.
> Código: `crates/poph-agent/src/turno/` (motor) e `crates/poph/src/chat.rs` (CLI).

## 1. O caminho de um turno

```
mensagem do cliente
   │
   ├─ chat:  messages.gravar(entrada) ─────────────────────────────┐
   └─ integração: Fluxo::receber_mensagem (mensagem + inbox, atômico)
                            │
                    Fluxo::despachar → ator da conversa (HR-11)
                            │
                    ProcessadorTurno → Motor::executar
                            │
   1. conversa + agente (D-05: inativo não abre turno; D12: transferida não responde)
   2. janela de agrupamento (Agente.grouping_window_ms, teto de 8 s)
   3. grupo = cauda de mensagens do cliente ainda sem resposta
   4. turns.iniciar (state=running)
   5. vaga de turno (Limites: org, agente, instância) + vaga do provedor
   6. prompt = prefixo estável + bloco volátil + janela de contexto (HR-11)
   7. laço de ferramentas (limite de rodadas e de chamadas por mensagem, D-04)
   8. resposta = messages.gravar(saida) + outbox.enfileirar(turno, seq 0)
   9. turns.finalizar (tempos, tokens, modelo, motivo, erro)

   (HR-29) se o G1 do Jev decidiu transferir, o turno termina no passo 7½:
     7½. pergunta de risco → texto de orientação + mensagem automática → messages.gravar +
         outbox.enfileirar → conversas.transferir(status=transferred, auditoria) → turns.finalizar
         O modelo de texto NÃO é chamado; os turnos seguintes caem no passo 1 (D12).
                            │
        Fluxo::enviar_pendentes(transporte) → cliente
```

`Motor::executar` devolve `ResumoTurno` (turno gravado, estado, texto, mensagens atendidas, tempos,
tokens). `Err` só quando o armazenamento falha antes de abrir o turno; agente inativo, conversa
transferida, mensagem já respondida e erro do provedor terminam como `ResumoTurno` com `estado` e
`erro` — o processador precisa concluir a entrada nos dois casos.

## 2. Janela de agrupamento

- A espera é medida **a partir da chegada da mensagem** que disparou o turno: `janela − decorrido`,
  nunca maior que `grouping_window_ms` (0 usa 2 s) nem que o teto de 8 s do PRD §6.3.
- Depois da espera o motor lê a conversa e agrupa **todas** as mensagens seguidas do cliente que
  ainda não têm resposta. É por isso que um "oi" seguido de "queria um orçamento" dentro da janela
  vira um turno só, e uma mensagem que chega depois da resposta abre outro.
- A pausa é injetável (`Pausa`), então o teste prova a janela com o relógio controlado, sem dormir.
- O **G2 do Jev** (esperar mais / processar agora) governa a janela desde a HR-30: com ele ligado, o
  lote do Jev sai **antes** da espera (o tempo dele cabe na janela) e a decisão ajusta a espera —
  `esperar_mais` dobra a janela até o teto de 8 s, `processar_agora` a reduz para a confirmação curta
  de 250 ms, e o fallback mantém a janela fixa. Se o cliente mandar outra mensagem durante a espera, o
  lote é refeito. Detalhe e medição em `docs/JEV.md` §5-B; teste em `tests/g2_janela.rs`.

## 3. Prompt estável (PRD §6.1)

Ordem das mensagens:

1. `system`: o markdown do compositor (HR-09), **idêntico byte a byte** entre turnos do mesmo agente.
   Vem de `agent_instructions.markdown` e nunca é recomposto no turno (pode ter sido editado à mão).
2. `system`: bloco volátil delimitado por `<contexto_do_turno>`: hora (UTC), canal, id da conversa,
   estado do atendimento e resumo (quando a HR-14 gravar um).
3. histórico: as últimas mensagens, dentro do orçamento de tokens do agente (HR-11).

Quando o RAG (HR-17), os arquivos (HR-23) e o Jev (HR-28) entrarem, eles vão **depois** do prefixo,
no bloco volátil (ou como mensagem seguinte), nunca dentro dele.

O **Jev** entrou na HR-28 e **não** toca no prompt: ele roda em paralelo com a busca, decide os
portões e grava o resultado em `turns.jev_json` ([`JEV.md`](JEV.md)). O G1 tem efeito desde a HR-29
(transfere a conversa e encerra o turno antes do modelo), o G2 desde a HR-30 (governa a janela de
agrupamento) e o G4 desde a HR-32 (escolhe o departamento da transferência, validado pelo código);
falta o efeito do G3 (pular a busca, HR-31). Todos nascem desligados e só ligam depois de medidos.

## 4. Laço de ferramentas (D-04)

- Ao modelo só são oferecidas as ferramentas **habilitadas no agente** *e* implementadas no processo.
  Nesta tarefa o registro de produção (`SemFerramentas`) está vazio: `consultar_base` é HR-17,
  `buscar_produto`/carrinho HR-18/19, `enviar_arquivo` HR-24, `transferir` depende do Jev.
  Cada uma se registra em `RegistroFerramentas` sem tocar no laço.
- O registro de ferramentas **não depende da base de conhecimento** (HR-46f): `enviar_arquivo` e as
  quatro do carrinho usam só o banco e são registradas sempre; `consultar_base` e `buscar_produto`
  só entram quando o índice de embedding abriu. Sem `poph model install`, o pedido ao modelo segue
  com `enviar_arquivo` e o agente continua podendo enviar arquivo.
- Chamada de ferramenta desabilitada (ou inventada) vira **resultado de erro** para o modelo, não
  execução.
- `max_tool_rounds` limita as rodadas: na última passagem o pedido vai **sem ferramenta**, e é assim
  que o turno termina em texto. `max_tool_calls_per_message` limita as chamadas de uma resposta; as
  que passam recebem erro de limite.
- Os argumentos chegam inteiros antes de a ferramenta rodar (garantia da HR-10).

## 5. Streaming, cancelamento e erro

- Cada `EventoLlm::Texto` vai para o `Observador` na hora: o `chat` imprime o pedaço (texto) ou uma
  linha JSON (`--json`).
- Ctrl-C no `chat` pede `Cancelamento`; o provedor fecha o stream com `ErroLlm::Cancelada` e o turno
  é gravado como **falho** com o motivo. A resposta parcial **não** é enviada nem gravada como
  mensagem do agente: ela nunca chegou ao cliente, e guardá-la faria o próximo turno "achar" que o
  agente já tinha respondido aquilo.
- Erro do provedor: turno `failed` com o motivo e nada na `outbox`. No `poph serve`, a entrada é
  adiada e tentada de novo, com limite (ver a seção 11, "Falha do modelo"). Em `poph chat`, o turno
  termina aí, como antes.
- **Falta de vaga** (vaga da instância, da org ou do agente, ou vaga de chamada ao provedor): o
  turno é gravado como falho e a entrada entra na mesma escada da falha do modelo (seção 11, HR-26k).
  Não há mais repetição sem limite nesse caminho. A espera pela vaga dentro do turno é
  `espera_vaga` (5 s).
- `tempo_maximo_turno` (padrão 180 s) fecha o turno com erro em vez de deixar o ator preso.

## 6. Registro em `turns`

`timings_json` guarda as etapas e, junto, `modelo` e `motivo_fim` — `turns` não tem coluna própria
para esses dois, e a HR-04b fechou o esquema (migração nova exige autorização do mantenedor).

```json
{"fila_ms":2002,"montagem_ms":0,"primeiro_texto_ms":3719,"total_ms":4181,
 "rodadas_ferramenta":0,"modelo":"gemini-3.8-flash-tiered","motivo_fim":"concluido"}
```

- `fila_ms`: da chegada da mensagem que disparou o turno até o turno começar (inclui a janela).
- `montagem_ms`: montar prompt e ferramentas.
- `primeiro_texto_ms`: do início do turno ao primeiro pedaço de texto do modelo.
- `total_ms`: o turno inteiro (inclui as rodadas de ferramenta).
- `tokens_entrada`/`tokens_saida`: somados entre as chamadas do turno.
- `tools_json`: registro das ferramentas do turno. Guarda `{"nomes": [...], "chamadas": [...]}`:
  `nomes` é a lista publicada como `tools` (toda chamada que chegou ao laço, inclusive a recusada,
  como antes), e `chamadas` traz o detalhe por chamada (HR-47b, ver abaixo). `custo_micros` fica 0:
  a config ainda não tem tabela de preço por modelo (limite conhecido da HR-10).

### Registro das chamadas de ferramenta (HR-47b)

Cada chamada que o modelo faz entra em `chamadas`, na ordem em que aconteceu:

```json
{"nome":"consultar_base","rodada":0,"ordem":1,"estado":"executada","motivo":null,
 "duracao_ms":12,"argumentos":"{\"consulta\":\"prazo\"}","resultado":"Prazo de 5 dias úteis."}
```

- `estado`: `executada` (a ferramenta rodou, mesmo quando devolveu erro de conteúdo ao modelo) ou
  `recusada` (não rodou).
- `motivo` da recusa, só quando `recusada`: `nao_habilitada` (fora da lista do agente, D-04),
  `desconhecida` (nome que o processo não registra), `argumento_invalido`, `erro_de_execucao`,
  `limite_de_chamadas` (teto por mensagem) e `fora_do_turno` (G3 em `nada` ou catálogo já
  consultado no turno).
- `duracao_ms`: tempo da execução; zero nas recusas antes de rodar.
- `argumentos` e `resultado`: **resumos de texto**, cortados em 1024 caracteres com a marca
  `…[cortado]`. Nos argumentos, campo com nome que indique segredo (`token`, `senha`, `secret`,
  `chave`, `api_key`, `password`, `authorization`, `credencial`) tem o valor trocado por
  `[redigido]`. O resultado é o texto que o modelo recebeu, sem redação (é saída das nossas
  ferramentas).
- Turno que falhou no provedor depois de já ter chamado ferramenta grava as chamadas feitas; turno
  sem chamada grava `"chamadas": []`.

## 7. Temperature e teto de tokens por agente

A HR-03 encontrou os dois campos gravados e ignorados no POP Atende. Aqui eles valem: o turno põe
`temperatura` e `max_tokens` no `Pedido`, e o cliente do provedor prefere o que veio no pedido ao
padrão do bloco `[llm]`. É o único acréscimo feito em `poph-core/src/llm/` (dois campos opcionais em
`Pedido` e uma linha em `montar_corpo`), e nada muda para quem não os preenche.

## 8. O que a próxima tarefa consome

| Tarefa | O que usar |
|---|---|
| HR-14 (resumo rolante) | `Prompt`/`bloco_volatil` já recebe `resumo`; `contexto::Janela::folga()` diz quanto sobrou; gerar fora do caminho crítico e gravar com `Conversas::definir_resumo` |
| HR-17 (RAG) | registrar `consultar_base` em `RegistroFerramentas`; a busca entra em paralelo antes da 1ª chamada e o resultado vai no bloco volátil (marcado como conteúdo não confiável) |
| HR-18/19 (produtos, carrinho) | idem, por `RegistroFerramentas`; o `ContextoFerramenta` já traz org, conversa, agente e turno |
| HR-20 (API/SSE) | `Motor::executar` + `Observador` que publique os eventos; `turns` já guarda o "por quê"; `Motor::com_cancelamento` serve ao desligamento ordenado |
| HR-28 (Jev) | feito: `Motor::com_jev`, decisões em `turns.jev_json` e no `ResumoTurno`, em paralelo com a busca (ver `JEV.md`) |
| HR-29 (G1) | feito: `Motor::com_handoff`, `AvaliadorGates::orientar`, `Conversas::transferir` (status + auditoria numa transação) e o evento `EventoTurno::Transferencia` (ver `JEV.md` §5-A) |
| HR-30 (gate G2) | substituir a pausa fixa por "esperar mais/processar agora" |
| HR-55 (carga) | 500 conversas ativas: `Atores` + `Motor` já são `Clone` barato e particionados por org |

## 9. Limites conhecidos

- **Sem RAG, sem Jev, sem produtos, sem arquivos**: nesta tarefa o modelo responde só com as
  instruções e o histórico. O toolset de produção está vazio de propósito.
- **Resposta vazia**: se o modelo não devolver texto (por exemplo, só chamadas de ferramenta até o
  limite), o turno fecha `done` com texto vazio e **nada** vai para a `outbox`. O Console vai mostrar
  o turno sem resposta; não há retentativa.
- **Hora em UTC**, sem fuso local: não há crate de data na lista do PRD §5.3. O Jev (HR-30) é quem
  precisa da janela temporal em horário local.
- **`custo_micros` é 0** (sem tabela de preço na configuração — limite herdado da HR-10).
- **Mensagem agrupada**: os itens da `inbox` das mensagens que entraram no mesmo turno são
  concluídos juntos. A busca é `listar_pendentes(org, 500)` por turno; com fila maior que isso o
  item que sobrar é despachado de novo e cai em "mensagem já respondida" (turno `skipped`).
- **`--conversa` de outro agente** é recusado; para trocar de agente abra conversa nova.
- **Transporte real** (Evolution/plugin) é fase 4. O `TransporteConsole` confirma itens de texto; e
  `file`/`presence` são confirmados sem imprimir, para não repetir envio para sempre.
- **`chat` não passa pela `inbox`**: ele grava a mensagem e dispara o turno direto (`Tarefa::nova`),
  porque um humano está esperando a resposta na mesma sessão. A idempotência da `inbox` é do canal
  externo.
- **Sanitização de texto**: o que o cliente escreve entra como conteúdo do usuário; não há
  marcação de "conteúdo não confiável" nesta fase (entra com o RAG/arquivos, HR-17/23).
- **aarch64**: só `cargo check` nos crates sem C (D-02/D-10); o binário ARM é a HR-56.

## 10. Demonstração (19/11/2026, gateway local `127.0.0.1:8045`, `gemini-3.8-flash-tiered`)

Agente criado pela CLI (`poph agent create`, `poph instructions create --file`), conversa de 5
mensagens pelo `poph chat`. Transcrição completa em `/tmp/hr13-demo-final/transcricao.txt` (fora do
repositório); os tempos vêm de `turns`:

| # | fila | montagem | 1º texto | total | tokens (in/out) |
|---|---|---|---|---|---|
| 1 | 2002 ms | 0 ms | 3719 ms | 4181 ms | 414 / 43 |
| 2 | 2003 ms | 0 ms | 4492 ms | 4854 ms | 471 / 48 |
| 3 | 2001 ms | 0 ms | 3744 ms | 4189 ms | 529 / 53 |
| 4 | 2003 ms | 0 ms | 5081 ms | 5712 ms | 592 / 78 |
| 5 | 2001 ms | 0 ms | 3704 ms | 4237 ms | 677 / 41 |

10 mensagens gravadas, 5 itens de saída `sent`, nenhuma entrada pendente. O `fila_ms` de ~2000 ms é
a janela de agrupamento (padrão de 2 s do agente).

## 11. Turno no `poph serve` (HR-26h)

O servidor responde as mensagens do WhatsApp. O caminho é o mesmo do `poph chat`, com a entrada
vindo da fila em vez do terminal:

1. O webhook da Evolution grava a mensagem do cliente na fila de entrada (`inbox`) e na conversa.
2. O laço de despacho (`crates/poph-server/src/entrada_turno.rs`) lê a fila pendente a cada
   `[turno] intervalo_despacho_ms` (padrão 1000 ms; aceita de 100 a 60000). Ele despacha **só a
   entrada**: não envia nada. A saída é do laço do WhatsApp (`saida_whatsapp.rs`).
3. O ator da conversa roda o turno. Um turno por vez por conversa é garantido pelo próprio ator.
4. A resposta vai para a conversa como `out` e para a fila de saída; o laço do WhatsApp entrega.

A montagem do turno (motor, modelo, base, produtos, Jev, transferência) é a mesma nos dois
comandos: `crates/poph/src/montagem.rs`. O `chat` mantém as métricas dele, fora do `/metrics`; o
`serve` publica as dele no `/metrics`.

### Quando não responde

A mensagem fica gravada e o item da fila é concluído **sem resposta**, com o motivo no turno
(estado `ignorado`). Não é marcado como falha. Casos:

- conversa transferida para humano, ou fechada (até alguém reabrir);
- agente inativo ou arquivado;
- integração do WhatsApp inativa ou arquivada, ou que não existe mais.

### Sem a chave do modelo

Sem `ANTIGRAVITY_API_KEY`, o `poph serve` sobe normalmente e continua recebendo as mensagens, mas
não roda turno. O aviso sai na subida. As mensagens ficam na fila (não se perdem) e são respondidas
quando o servidor sobe com a chave. Trocar a chave ou o intervalo exige reiniciar o servidor.

### Retomada na subida

- Mensagem pendente na fila (chegou enquanto o servidor estava parado, ou com o turno desligado)
  é despachada na primeira passagem do laço.
- Resposta que estava em envio quando o processo caiu (`sending`) é reenviada pelo laço do
  WhatsApp. O risco de duplicata na queda entre o envio e a confirmação segue o que já está
  aceito (D11).
- Mensagem que estava em turno quando o processo caiu continua na fila. Ao ser refeita, o turno
  confere o histórico: se já há resposta depois dela, não responde de novo.

### Falha do modelo: repetir depois, com limite (HR-26i)

Quando o modelo falha no meio de um turno (provedor fora do ar ou com erro depois das retentativas
do próprio cliente, conexão caída, ou tempo máximo do turno estourado), a mensagem **não** fica
sem resposta em silêncio. A escada é:

1. A tentativa falha. O turno fica `falhou` com o motivo, como antes.
2. Os itens da fila que esse turno atenderia (a mensagem e as do mesmo grupo) são **adiados**: ficam
   na fila, não vão para a saída, e contam mais uma tentativa. O instante da próxima tentativa é
   gravado no banco (coluna `next_attempt_at` de `inbox`), então o adiamento sobrevive a reinício.
3. Vencido o instante, o laço de despacho pega o item de novo e roda um turno novo.
4. Se a tentativa der certo, a resposta segue o caminho normal (conversa, `outbox`, WhatsApp).
5. Se o limite acabar, a entrada é **concluída sem resposta**, com o motivo gravado, e o turno fica
   como falha definitiva. Não volta mais para a fila.

Padrões, todos em `[turno]` do `poph.toml`:

| Chave | Padrão | Faixa | O que faz |
|---|---|---|---|
| `tentativas_falha_modelo` | 3 | 1 a 10 | Tentativas no total, contando a primeira. |
| `atraso_falha_modelo_ms` | 30000 | 1000 a 600000 | Espera antes da segunda tentativa. Cada espera seguinte é quatro vezes a anterior, até 10 min. |

Com os padrões, as esperas são 30 s (antes da 2ª tentativa) e 120 s (antes da 3ª). Com
`tentativas_falha_modelo = 1`, a primeira falha já é definitiva. Mudar exige reiniciar o servidor.

Regras que valem em toda repetição:

- **Não duplica resposta.** Antes de chamar o modelo de novo, o turno refaz a análise do grupo: se
  o agente já respondeu depois da mensagem (por exemplo, o turno anterior gravou a resposta e caiu
  antes de fechar a entrada), conclui sem chamar o modelo.
- **Não repete se já saiu algo.** Se o turno que falhou já tinha enfileirado saída para o cliente
  (hoje, um arquivo da biblioteca enviado antes da falha), a falha é definitiva e a entrada é
  concluída com o motivo. Repetir mandaria o arquivo de novo. O cliente recebe o arquivo e não
  recebe o texto.
- **Adiado não sai na saída.** Nada vai para a `outbox` por causa do adiamento.

Classificação (HR-26i e HR-26k):

- **Entra na escada:** falha do modelo (provedor fora do ar depois das retentativas, conexão caída,
  tempo máximo do turno estourado) e falta de vaga (do turno ou do provedor). Nenhuma chamada chega
  ao modelo na falta de vaga, então a repetição não duplica nada.
- **Não repete, avisa na hora (HR-26k):** recusa do provedor (`Recusado`: chave recusada (401),
  pedido recusado (400) e demais códigos que não são 429 nem 5xx). A entrada é concluída sem
  resposta na primeira tentativa, o turno fica `falhou` e sai um `WARN` com o motivo sanitizado.
- **Não repete:** erro de configuração local, pedido inválido e cancelamento.

A falta de vaga usa o mesmo `tentativas_falha_modelo` e `atraso_falha_modelo_ms`. Não há knob
próprio: é a mesma escada de capacidade, e um knob separado só criaria mais uma configuração para
o mesmo efeito.

Métricas no `/metrics`: `poph_turnos_falha_modelo_adiadas_total` (repetições agendadas, inclui falta
de vaga), `poph_turnos_falha_modelo_definitivas_total` (mensagens concluídas sem resposta: depois do
limite ou por recusa do provedor) e `poph_turnos_falha_modelo_recusadas_total` (só as recusas; parte
das definitivas).
Logs: uma linha `WARN` por adiamento (com a tentativa, de quantas e a espera), uma por desistência
(`tentativas esgotadas; mensagem concluída sem resposta`) e uma por recusa do provedor (`provedor
recusou o pedido: ... confira a chave e o modelo do agente`), todas com o motivo sanitizado.

### Registro e métricas

- Uma linha `INFO` por turno que termina, com org, conversa, turno, estado, duração e tokens. Turno
  sem resposta sai em `INFO` com o motivo. Turno que falhou sai em `WARN` com o motivo.
- Nenhuma linha leva texto do cliente ou da resposta, telefone, JID, chave ou segredo. O motivo
  corta em 160 caracteres e esconde números longos e identificadores com `@`.
- No `/metrics`: `poph_turnos_iniciados_total`, `poph_turnos_concluidos_total`,
  `poph_turnos_falhos_total`, `poph_turnos_ignorados_total`, as duas de falha do modelo (seção acima)
  e as medidas de duração.

### O que não faz (e limites)

- **Falha do modelo sem resposta no fim.** Esgotadas as tentativas, a mensagem é concluída **sem
  resposta** (não há mensagem de instabilidade ao cliente). A decisão foi repetir com limite, não
  avisar o cliente.
- **Cota por minuto da org não entra no turno.** O limitador tem a conta (`consumir_cota`), mas o
  caminho do turno não a chama hoje, então ela não limita nem adia nada. Se for ligada, entra na
  mesma escada.
- **Falta de vaga é provada no motor, não no servidor completo.** O teste de servidor com Evolution
  falsa não segura vaga ocupada; a escada de falta de vaga está provada em `poph-agent`.
- **Recusa do provedor que é transitória na prática.** Um 400 causado por uma falha momentânea do
  provedor conta como recusa e não é repetido. Hoje a classificação não distingue essas duas coisas.
- **Falha de TLS não é repetida, mas entra na escada.** O provedor marca TLS como não repetível; o
  turno ainda a trata como falha do modelo (decisão da HR-26i, sem mudança nesta rodada).
- **Turno interrompido pelo processo** fica com estado `rodando` no histórico. A mensagem é refeita
  na subida, mas o registro antigo não é fechado.
- **Intervalo e chaves** são lidos na subida: mudar exige reiniciar o servidor.
- **Provado com LLM falso** (`ProvedorFake` no teste, servidor falso de SSE no smoke) e Evolution
  falsa. Nenhum teste fala com o modelo real nem com número de WhatsApp.

### Como provar

```
cd apps/pop-harness
cargo test -p poph-server --test turno_whatsapp --locked
```

Sete casos: mensagem vira resposta uma vez (com contagem de chamadas e da chave só no cabeçalho);
reentrega não abre segundo turno; conversa transferida e integração inativa não respondem; sem chave
a mensagem fica na fila e é respondida na subida; resposta em envio na queda sai na subida; métricas
no `/metrics`; logs sem dado sensível (com captura que prova que a linha existe).

Falha do modelo (HR-26i), com provedor falso e Evolution falsa:

```
cargo test -p poph-agent --test turno --locked falha_do_modelo
cargo test -p poph-agent --test biblioteca_arquivos --locked falha_do_modelo
cargo test -p poph-server --test turno_whatsapp --locked falha_do_modelo
```

Falta de vaga e recusa do provedor (HR-26k), com provedor falso e Evolution falsa:

```
cargo test -p poph-agent --test turno --locked falta_de_vaga
cargo test -p poph-agent --test turno --locked provedor_que_recusa
cargo test -p poph-server --test turno_whatsapp --locked recusa_do_provedor
```

Provas: falta de vaga do turno em todas as tentativas (três turnos falhos, nenhuma chamada ao modelo,
nada na saída, nenhuma repetição depois do limite); falta de vaga na primeira e resposta na segunda
(uma chamada, uma resposta, uma saída); falta de vaga do provedor com a mesma escada (nenhuma chamada
sai enquanto a vaga está ocupada); recusa do provedor (401) com uma única chamada, sem adiamento, com
métrica e aviso no log sem dado sensível.

Provas: duas falhas e uma resposta (três chamadas ao modelo, uma saída, espera crescente antes de
cada tentativa, nada despachado antes do instante); três falhas (três chamadas, entrada concluída,
sem saída, sem nova tentativa depois); limite de uma tentativa; tempo máximo entra na mesma escada;
resposta já gravada não chama o modelo; adiado sai da fila junto com a mensagem nova do cliente;
arquivo já enviado não é repetido; pelo servidor, métricas e logs sem dado sensível.

## 12. Chat de teste do Console: SSE, anexos e cancelamento (HR-22)

A aba de chat de teste do Console (HR-46) conversa com o mesmo motor da produção: a rota
`POST /admin/v1/orgs/{org}/agents/{agente}/test-chat/{conversa}/messages` grava a mensagem do
cliente e roda um turno direto no `Motor`, com o mesmo provedor, Jev, base, produtos e ferramentas
do `poph serve`. Não é um motor paralelo de teste.

- **Conversa de teste:** conversa comum com `channel = "test"` e sem integração, criada por
  `POST …/test-chat`. Memória zerada é consequência de ser nova (`summary` nulo); o histórico
  completo (mensagens, anexos, turnos) volta por `GET …/test-chat/{conversa}`.
- **SSE:** o turno roda numa tarefa e cada evento do `Observador` vira um evento
  `text/event-stream` com `id` sequencial: `mensagem` (o que foi gravado, antes de abrir o turno),
  `inicio`, `texto` (deltas na ordem), `base`, `ferramenta`, `transferencia` e `fim` (ou `erro`).
  O `Fim` do motor não é repassado: o `fim` do SSE é montado do `ResumoTurno`, com turno, tokens,
  erros e tempos. Contrato em `docs/API.md`.
- **Um turno por conversa:** a reserva é em memória, por instância (`Estado::reservar_turno_de_teste`).
  Com um turno em voo, a outra conexão recebe `409 turno_em_andamento` antes de qualquer gravação —
  a mensagem da segunda aba não entra no turno que já está rodando.
- **Desconexão:** o fluxo do SSE carrega um guarda com o cancelamento do turno e a reserva. Quando
  o cliente fecha a conexão, o cancelamento é acionado, o turno termina `failed` com o motivo
  (`Cancelada`) e **nada** vai para a `outbox`: resposta parcial nunca chegou ao cliente. A
  reserva é liberada no mesmo instante, então a conversa aceita a próxima mensagem.
- **Saída:** o texto já saiu pelo SSE; ao terminar, o servidor confirma a `outbox` do console com
  `TransporteConsole::silencioso`, para o item não ficar pendente na fila.
- **Anexos:** o corpo é `multipart/form-data` (`text` + `file` repetível) e cada arquivo passa pelo
  `poph_files::receber` da HR-23 (16 MB, tipo pelo conteúdo, extração isolada em `poph-extrator`).
  O texto extraído entra no prompt pelo caminho da HR-35, entre as marcas de conteúdo não
  confiável, e a imagem vai como parte multimodal (§13). Limite de corpo da rota: 24 MB.
- **Sem a chave do modelo** não há motor no estado: a rota responde `503 turno_indisponivel` antes
  de gravar a mensagem.

## 13. Imagem do cliente no prompt (HR-23b)

O turno lê os anexos de cada mensagem do cliente no histórico. Até a HR-22 só o texto extraído
(`attachments.extracted_text`) entrava no prompt: imagem era gravada, aparecia no histórico e **não**
chegava ao modelo — mensagem só com imagem gerava pedido sem conteúdo. Agora:

- **Imagem vai como parte multimodal.** Anexo cujo MIME real é `image/jpeg`, `image/png`,
  `image/gif` ou `image/webp` vira `Parte::Imagem { mime, bytes }` na mesma posição em que o texto
  do arquivo entrava: numa mensagem de usuário **depois** da mensagem do cliente que trouxe o
  arquivo. Os bytes vêm do blob (`data/blobs/<org>/<sha256>`) pelo hash gravado em `attachments`;
  nada é reextraído.
- **Documento continua como dado, nunca instrução.** O texto extraído do PDF/Office mantém o aviso
  `AVISO_NAO_CONFIAVEL` e as marcas do bloco não confiável (HR-35). Uma mensagem com imagem e
  documento leva as duas coisas: o texto marcado e a imagem.
- **Mensagem só com arquivo.** Se o texto da mensagem é vazio e há anexo, a mensagem de texto em
  branco não entra no pedido: o conteúdo é a parte do arquivo. Antes, o pedido ia com um texto vazio.
- **Tetos do turno** (constantes em `crates/poph-agent/src/turno/mod.rs`), contados na janela de
  contexto inteira, não por mensagem:
  - `MAX_IMAGENS_POR_TURNO` = **4** imagens;
  - `MAX_BYTES_POR_IMAGEM` = **5 MB** por imagem (o recebimento aceita 16 MB; no pedido a imagem
    vira base64, ~4/3 do tamanho);
  - `MAX_BYTES_DE_IMAGENS_POR_TURNO` = **8 MB** somados.
  O que passa do teto é **omitido** com `tracing::warn!` (id do anexo e tamanho, nunca bytes nem
  base64) e o turno segue.
- **Degradação.** Sem configuração de arquivos no motor (`Motor::com_arquivos`), sem blob no disco,
  hash inválido ou blob acima do limite de leitura: o anexo fica de fora, com registro no log, e o
  turno **nunca** falha por causa da imagem. O texto do documento, se houver, continua indo.
- **Quem liga a configuração:** a raiz de composição (`crates/poph/src/montagem.rs`) passa
  `ConfigArquivos::padrao(configuracao.blobs())` ao motor. É a mesma pasta que o recebimento e o
  chat de teste usam para gravar o blob.
- **Provedor que não aceite imagem:** não há declaração de capacidade por provedor hoje — o
  `validar_pedido` do `poph-core` recusa MIME fora da lista e imagem vazia antes do envio, e os
  provedores configurados (antigravity) recebem imagem. Se um provedor passar a recusar, o caminho
  atual é a falha do turno (retomada do HR-26i), não um envio sem a parte: a degradação por
  capacidade fica para quando existir essa configuração.
- **Como provar:** `cargo test -p poph-agent --test imagem_anexo` (5 testes: só imagem, imagem +
  documento, blob ausente, teto, e turno sem configuração de arquivos).

## 14. Queda e recuperação (HR-57)

O que acontece quando o processo morre (`kill -9`, falta de energia, OOM) no meio do atendimento.
A regra é: **a mensagem do cliente não se perde, o turno não fica preso em "em andamento", a
conversa volta a atender e a resposta não sai duas vezes.**

- **Garantias que já vinham da fila.** O item da `inbox` só é concluído quando o turno termina
  (`Fluxo::concluir_entrada`); queda no meio deixa o item pendente e a subida o despacha de novo
  (pelo menos uma vez). Se a resposta já estava gravada depois da mensagem do cliente, o turno
  refaz a análise (`analisar`) e conclui como `skipped` sem chamar o modelo — é o que impede
  resposta duplicada (HR-12).
- **Turno interrompido não fica `running`.** Na subida, `entrada_turno::recuperar_turnos_interrompidos`
  fecha como `failed` todo turno que ficou `running` e começou **até** o instante da subida, com o
  erro `interrompido: o processo caiu no meio do atendimento; a mensagem foi mantida`
  (`Turnos::falhar_orfaos`, leitura/escrita aditiva, sem migração). Turno aberto nesta instância
  depois da subida nunca é tocado. A varredura roda em `servir_em`, antes de qualquer atendimento.
- **Resposta e fila de saída na mesma transação.** A resposta do agente e o item da `outbox` são
  gravados por `Mensagens::gravar_com_saida` numa escrita só (o mesmo vale para a mensagem de
  transferência, a do G7 e a do G8). Antes eram duas escritas: morte entre elas deixava a resposta
  no histórico sem nunca ser enviada ao cliente.
- **Saída presa em `sending`.** Item de WhatsApp em `sending` é reenviado pelo laço de saída (D11 —
  pode duplicar no canal, é o preço da entrega pelo menos uma vez). Item de conversa do console
  (sem integração) é confirmado na subida por `enviar_pendentes_do_console` com o transporte
  silencioso: o texto já foi mostrado ao operador pelo stream e o item não pode ficar preso.
- **Trava de "um turno por conversa" (chat de teste).** A reserva é em memória, por instância, com
  validade de 30 minutos (`Estado::VALIDADE_DA_RESERVA`). Uma queda leva a memória junto, então a
  conversa já aceita mensagem na primeira requisição depois da subida; a validade existe para uma
  reserva que escape dentro de um processo vivo (bug, cancelamento que não passou pelo `Drop`).
- **Falha do provedor é estado controlado, não turno pendurado.** Provedor fora do ar, 5xx, tempo
  esgotado ou stream cortado: o turno termina `failed` com o motivo gravado, sem resposta inventada
  e sem texto parcial virando resposta. No WhatsApp, a falha do modelo entra na escada de repetição
  do HR-26i (com limite) e, se já havia saída enfileirada, não repete, para não duplicar.
- **Falha do Jev não derruba o turno.** O lote que falha vira fallback registrado em
  `turns.jev_json.chamada.falha` e o turno segue (`cargo test -p poph-agent --test jev_turno`).
- **Limite conhecido:** mensagem do chat de teste do Console (`channel = "test"`) **não** está na
  `inbox`. Morta antes de o turno terminar, ela fica gravada e sem resposta automática depois da
  subida (o operador reenvia). O caminho de canal (WhatsApp) retoma e responde uma vez.
- **Como provar:**
  - `cargo test -p poph-server --test resiliencia_queda` — turno órfão fechado na subida, conversa
    volta a atender, provedor indisponível, stream cortado, reserva vencida e saída do console presa
    em `sending`;
  - `cargo test -p poph-server --test turno_whatsapp` — `turno_interrompido_por_queda_e_retomado_uma_vez_na_subida`
    (uma resposta, um envio) e `resposta_que_ficou_em_envio_na_queda_sai_na_subida`;
  - `cargo test -p poph-store --test armazenamento` — `turno_orfaos_fecha_so_os_comecados_antes_da_subida`
    e `resposta_e_item_de_saida_entram_na_mesma_escrita`;
  - roteiro com o binário real, `kill -9` em seis pontos, em `/tmp/hr57-run/queda.py`.
