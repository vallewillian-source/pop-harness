# Entrada e saída — idempotência, fila durável e retomada (HR-12)

> Depende de HR-07 (armazenamento) e HR-11 (atores).
> Código: `crates/poph-agent/src/entrada_saida.rs`; persistência em
> `crates/poph-store/src/repos/caixas.rs` (`inbox`/`outbox`).
> Decisões que mandam aqui: PRD §6 passos 1 e 8, D11 (entrega pelo menos uma vez), D-05
> (agente inativo não abre turno), D-07 (2.000 msg/s por instância).

## 1. O desenho em uma olhada

```
webhook ──► Fluxo::receber_mensagem ──► [transação única] messages + inbox ──► ACK ao canal
                                              │
                                              ▼
                       Fluxo::despachar (ou ciclo) ──► ator da conversa (HR-11)
                                              │             │
                                              │             ▼
                                              │      turno (HR-13): LLM, ferramentas...
                                              │             │
                                              │             ├─► outbox.enfileirar (turn_id, seq)
                                              │             └─► Fluxo::concluir_entrada
                                              ▼
                        Fluxo::enviar_pendentes ──► TransporteSaida (Evolution/plugin/console)
                                     pending ──► sending ──► sent | failed
```

Duas filas duráveis, nenhum estado crítico só na memória:

| Onde | O que garante |
|---|---|
| `inbox` | a mensagem do cliente entra uma vez (chave única `(integration_id, external_message_id)`) e só sai da fila quando o turno conclui |
| `outbox` | a resposta sai uma vez por `(turn_id, seq)`; `sending` sobrevive à queda e é reenviada na subida (D11) |
| memória (`Fluxo`) | quem já está em voo, quem está sendo enviado agora e quando cada item pode ser retentado (backoff). Perdida no restart de propósito: o banco é a verdade |

Duas **reservas em memória** evitam trabalho duplicado quando dois ciclos rodam ao mesmo tempo no
mesmo processo: uma para o item de entrada (fica reservado do despacho até o turno concluir) e outra
para o item de saída (fica reservada só durante o envio). Elas não substituem o banco — quem garante
"uma vez" depois de uma queda é a `inbox`/`outbox`.

## 2. Estados

### Entrada (`inbox`)

| Coluna | Significado |
|---|---|
| `processed_at` nulo | ainda na fila; será despachado |
| `processed_at` preenchido | o turno concluiu — com sucesso ou com erro do turno |
| `attempts` / `last_error` | quantas vezes foi concluída e por quê, quando houve erro |

O item **não** é marcado como processado no despacho. Isso é o que faz o teste de `SIGKILL` passar:
se o processo morrer com o evento já entregue ao ator (mas ainda não processado), a `inbox` continua
pendente e a retomada o despacha de novo. O custo é "pelo menos uma vez" também na entrada.

### Saída (`outbox`)

| Estado | Significado | Quem escreve |
|---|---|---|
| `pending` | criada pelo turno, ainda não tentada | `enfileirar_saida` |
| `sending` | **já gravada antes de chamar o canal**; se o processo morrer aqui, a retomada reenvia | `iniciar_envio` |
| `sent` | o canal confirmou | `confirmar_envio` |
| `failed` | erro definitivo ou tentativas esgotadas | `falhar_envio` |

`attempts` conta **tentativas reais de envio** (não transições de estado). Falha transitória mantém o
item em `sending` com `last_error`, e o backoff em memória decide quando tentar de novo; se o processo
reiniciar, o item é retentado na hora (o backoff em memória se perde — é o comportamento desejado na
subida).

## 3. O que a HR-13 consome

```rust
// 1. Gravar a mensagem do cliente e o item da inbox numa transação só, e só então dar o ACK.
let gravado = fluxo.receber_mensagem(org, MensagemRecebida {
    integration_id, id_externo, conversa, texto, anexos_json,
})?;                            // RecebidoComMensagem { situacao, item, mensagem }
// gravado.situacao == Recebido::Duplicado → responder OK ao canal e não gerar turno

// 2. Despachar (o worker do servidor chama ciclo a cada intervalo, ou isto):
fluxo.despachar(org)?;          // RelatoDespacho { despachados, adiados, invalidos, desistentes }

// 3. No processador do ator (implementa poph_agent::atores::Processador):
fn processar(&self, tarefa: Tarefa) -> Futuro {
    // tarefa.entrada: Some(inbox_id) quando veio de integração, None no chat da CLI
    // ... turno: janela de contexto, Jev, RAG, LLM, ferramentas ...
    // resposta:
    fluxo.enfileirar_saida(org, NovaSaida { conversation_id, turn_id, seq, tipo, conteudo_json })?;
    // no fim, sempre (com ou sem erro):
    if let Some(entrada) = tarefa.entrada {
        fluxo.concluir_entrada(org, entrada, erro_do_turno)?;
    }
    Ok(())
}

// 4. Envio (transporte real entra na HR-25/HR-26; no CI é o TransporteDeTeste):
fluxo.enviar_pendentes(org, transporte.as_ref()).await?;

// 5. Na subida do processo, antes de aceitar trabalho novo:
fluxo.retomar(transporte.as_ref()).await?;   // reenvia o que ficou em sending e drena a inbox
```

Outras assinaturas úteis: `fluxo.estatisticas(org)` (tamanhos de fila), `fluxo.orgs()`,
`fluxo.atualizar_medidores()`, `fluxo.iniciar_laco(transporte)` (laço de fundo a cada
`intervalo_ciclo`).

`Tarefa` (do `atores`) ganhou o campo `entrada: Option<InboxId>` e os construtores `Tarefa::nova`
(sem inbox) e `Tarefa::da_entrada`. É o único gancho que o despacho precisou na HR-11; nada mais do
ator mudou.

## 4. Retomada na subida

`retomar` faz, nesta ordem:

1. conta os itens em `sending` (são os que podem ter saído sem confirmação) e soma em
   `poph_fluxo_reenvios_retomada_total`;
2. despacha a `inbox` pendente de todas as orgs;
3. envia `pending` + `sending` de todas as orgs.

O reenvio de `sending` pode duplicar a mensagem no canal. É a decisão D11 (o WhatsApp não reconhece
reenvio; a alternativa seria perder a resposta).

## 5. Backoff

`Backoff { base, fator, teto }`, padrão 500 ms × 2 até 60 s. Vale para:

- despacho recusado pelo ator (fila cheia, ator encerrado);
- envio com falha transitória.

`max_tentativas` (padrão 8) decide quando desistir:

- **entrada**: o item fica gravado com `last_error` e `processed_at` preenchido (a mensagem do cliente
  está em `messages`, nada se perde, mas não é respondida) e conta em
  `poph_fluxo_entradas_desistentes_total`;
- **saída**: vai para `failed` com motivo legível e conta em `poph_fluxo_saidas_esgotadas_total`.

Payload inválido na `inbox` não entra em laço de retentativa: é registrado com o erro do parser e
conta em `poph_fluxo_entradas_invalidas_total`.

## 6. Métricas (sem HTTP; a HR-20 publica em `/metrics`)

| Nome | Tipo | O que mede |
|---|---|---|
| `poph_fluxo_inbox_pendentes` | medidor | itens não processados, somados entre as orgs |
| `poph_fluxo_outbox_enviando` | medidor | itens em `sending`, somados entre as orgs |
| `poph_fluxo_entradas_novas_total` | contador | entregas novas |
| `poph_fluxo_entradas_duplicadas_total` | contador | reentregas descartadas |
| `poph_fluxo_entradas_invalidas_total` | contador | payload ilegível |
| `poph_fluxo_entradas_adiadas_total` | contador | despachos adiados por backoff |
| `poph_fluxo_entradas_desistentes_total` | contador | despacho desistiu depois das tentativas |
| `poph_fluxo_entradas_concluidas_total` | contador | itens concluídos pelo turno |
| `poph_fluxo_saidas_enviadas_total` | contador | respostas confirmadas |
| `poph_fluxo_saidas_transitorias_total` | contador | falhas que serão retentadas |
| `poph_fluxo_saidas_definitivas_total` | contador | falhas que não retentam |
| `poph_fluxo_saidas_esgotadas_total` | contador | itens que viraram `failed` por tentativas |
| `poph_fluxo_reenvios_retomada_total` | contador | itens em `sending` na subida |

Os medidores são da instância (somados entre orgs). Contagem por org é a HR-21/HR-20, que tem
rótulos.

## 7. Como os testes provam

`crates/poph-agent/tests/entrada_saida.rs` — 14 testes, todos no `scripts/ci.sh`:

| Grupo exigido | Teste |
|---|---|
| mesma mensagem 2× = 1 processamento | `mesma_mensagem_entregue_duas_vezes_e_processada_uma_vez` (a 2ª entrega responde `Duplicado`, um único turno) |
| `SIGKILL` entre gravar e processar | `sigkill_entre_gravar_e_processar_nao_perde_a_mensagem` (processo filho real morto com `SIGKILL`; a retomada processa e a mensagem do cliente continua lá) |
| `SIGKILL` entre `sending` e `sent` | `sigkill_entre_sending_e_sent_reenvia_uma_vez` (reenvio 1 na retomada; segunda retomada não reenvia) |
| falha repetida → `failed` | `falha_repetida_vai_para_failed_com_motivo_e_contador` (3 tentativas reais, motivo legível) |
| backoff cresce e para no teto | `backoff_cresce_e_para_no_teto` + `item_em_backoff_nao_e_reenviado_antes_da_hora` (relógio injetado, sem dormir) |
| nenhuma leitura cruza org | `caixas_nao_cruzam_org` |

Extras: `falha_definitiva_nao_retenta`, `saida_idempotente_por_turno_e_sequencia`,
`fila_cheia_adia_e_nao_perde_o_evento` (fila de 1 evento: os adiados são processados depois),
`dois_despachos_concorrentes_nao_duplicam_o_evento` e `dois_envios_concorrentes_nao_duplicam_a_saida`
(as reservas em memória).

Há ainda `medir_ingestao`, marcado `#[ignore]`: mede a vazão e a latência do ACK da ingestão atômica.
Não roda no CI porque é medição, não checagem:

```
cargo test --release -p poph-agent --test entrada_saida -- --ignored --nocapture medir_ingestao
```

Os dois testes de `SIGKILL` rodam `este mesmo binário de teste` com `--exact <nome do filho>`: o pai
espera o filho escrever um marcador (o ponto controlado) e manda `SIGKILL`. Não há simulação.

## 8. O que não está coberto

- **Cifra de campos e segredos da integração** (HR-26): o payload da `inbox` é texto.
- **Encaminhamento entre instâncias** (PRD §12): aqui é sempre a instância local.
- **Transporte real**: só o trait. Evolution e plugins entram na fase 4.
- **Cancelamento de turno em andamento** (HR-13).
- **Retenção/limpeza da `inbox`** (HR-51): nada apaga item processado antigo.
- **Concorrência entre processos**: o desenho é um escritor por processo e um processo por banco. O
  envio de uma org dentro de um ciclo é sequencial (um canal lento atrasa os itens seguintes daquela
  passagem); orgs diferentes também são percorridas em ordem. Paralelizar o envio por org é da HR-20.
- **Reserva órfã**: se o turno aceitar o evento e nunca chamar `concluir_entrada` (defeito na HR-13),
  o item fica reservado até o processo reiniciar. Na subida, a retomada o pega de novo.
- **Carga longa** (HR-55): vazamento de memória das agendas em memória (`agenda_entrada`/
  `agenda_saida` só perdem entrada quando o item é concluído/enviado; item que nunca é retentado
  por estar concluído sai). Sem teste de horas.
- **ARM64**: `poph-agent` depende do `poph-store` (SQLite em C), então fica fora do `cargo check`
  aarch64 da estação (D-02/D-10); a verificação e a medição são a HR-56, na máquina ARM64 de testes.

## 4. Conversa fechada e a próxima mensagem (MX-06)

Fechar uma conversa (`POST …/conversations/{id}/close`) muda o status dela para `closed`. Ela deixa de
receber mensagem e de contar como atendimento ativo. Quando o cliente escreve de novo, a entrada
procura só conversas `open` ou `transferred` do mesmo canal e contato. Se não achar nenhuma, abre uma
conversa nova, com o agente da integração e o histórico vazio. A conversa fechada continua no banco,
com as mensagens, e não é reaberta.

Provas: `mensagem_depois_do_fechamento_abre_conversa_nova` em `crates/poph-server/tests/api.rs`. O
comportamento já existia (a busca em `caixas.rs` filtra `status IN ('open', 'transferred')`); a MX-06
só o documenta e o prova.

## 5. A resposta do atendente humano (MX-18)

Quando um atendente responde pelo Element, o evento chega pelas rotas do Application Service do
Matrix (`api/matrix_as.rs`) e é processado em `matrix_resposta.rs`. A resposta entra na `outbox` pela
mesma operação atômica da MX-04 (`Mensagens::registrar_resposta_humana`): mensagem (`direction out`,
`role assistant`, `origin human`, com o nome do autor e o id do evento do Matrix em `external_id`),
item na fila com `text` e `seq 0`, conversa `open` → `transferred` e auditoria `human_takeover`, tudo
numa escrita só. Daí em diante o caminho é o de qualquer resposta: o consumidor da saída pega o item
e entrega pela Evolution.

Três detalhes valem para quem mexer nisso:

- **A idempotência é o `event_id`.** O Synapse reenvia a transação quando não recebe 2xx; a segunda
  gravação devolve `Duplicada` e nada é enfileirado de novo. Reenvio nunca vira segundo envio.
- **O texto da fila vai em Markdown**, `**[Nome]**:` seguido do corpo como o atendente escreveu. A
  conversão para o formato do WhatsApp (`para_whatsapp`, que também corta em 4.000 com `…`) acontece
  **uma vez**, no envio — converter aqui faria o envio converter de novo e `**negrito**` chegaria
  como itálico.
- **Falha nossa de banco vira 5xx**, nunca 4xx: o Synapse trata qualquer não-2xx como "Application
  Service fora do ar" e segura a fila por minutos. Sala desconhecida e conversa encerrada são `200`
  (na encerrada, com um aviso na sala para o atendente).

Provas: `crates/poph-server/tests/matrix_resposta.rs` — 9 testes de ponta a ponta com Synapse falso e
Evolution falsa (prefixo e envio único, `transferred`, reenvio sem segundo envio, isolamento entre
orgs, conversa encerrada, sala desconhecida, falha de banco com reenvio, mídia, cache do nome).

## 6. Nome do cliente na entrada (MX-32)

O `receber_do_canal` grava o nome do cliente (`pushName`, em `metadados["push_name"]` do evento) junto
com a mensagem, na mesma transação. A gravação do nome roda num savepoint próprio: se ela falhar, a
mensagem entra mesmo assim, e a falha aparece só como log com código curto. Mensagem repetida
(reentrega) não regrava o nome. Regras e colunas: `DADOS.md`, migração `0008`.

Provas: `crates/poph-store/tests/nome_cliente.rs` (entrega grava o nome, falha do nome não derruba a
mensagem, reentrega não reescreve) e `crates/poph-server/tests/webhook_evolution.rs` (criar com o nome,
último nome vence, limpeza, grupo e eco não gravam, falha isolada, canário de log).

## 7. O estado da conversa na sala (MX-15)

O laço de mensagens leva o texto para a sala; o de estado (`matrix_estado.rs`) leva o que acontece com a
conversa. A cada passagem ele lê as conversas alteradas (cursor de `updated_at` + id, `DADOS.md` §11.5) e
compara a situação com o que a sala já espelhou:

- **`open → transferred`**: aviso "atendimento humano assumiu" (com o departamento, quando há) e metadata
  reescrito. É o que o atendente vê quando assume a conversa pelo Element.
- **`transferred → open`**: aviso "o agente automático retomou o atendimento".
- **`* → closed`**: aviso "conversa encerrada" e metadata com `status = closed` — o único valor de situação
  que o Element distingue.
- **Mesma situação**: nenhum aviso e nenhuma escrita no Synapse. A conversa é reavaliada a cada alteração
  (mensagem nova sobe o `updated_at`), mas só escreve quando muda de situação.
- **Departamento novo** (transferência com outro departamento): a sala muda de espaço (`m.space.child` no
  novo, sai do antigo) sem aviso repetido.
- **Nome do cliente que chega depois** (MX-33): a sala é renomeada (`m.room.name`), o metadata é reescrito
  com o estado atual da conversa e o fantasma do cliente recebe o nome novo; **sem** `m.notice` — trocar
  nome não é evento de atendimento. O nome só é dado como espelhado (`matrix_rooms.client_name_espelhado`,
  `DADOS.md` §11.7) depois das três chamadas, então repetir a passagem não escreve nada.

O aviso sai pelo bot do Application Service quando ele já está na sala; como a MX-11 não convida o bot para
a sala de atendimento, o normal hoje é sair pelo fantasma do agente — o dono da sala. O `txnId` do aviso é
(conversa, instante da alteração, situação): repetir a passagem depois de uma falha não duplica o aviso, e
uma segunda transferência da mesma conversa tem aviso próprio.

Provas: `crates/poph-server/tests/matrix_estado.rs` — 23 testes com banco de verdade e Synapse falso
(transições, repetição sem escrita, corrida de situação, mudança de espaço, falha transitória sem avançar
cursor, duas conversas no mesmo milissegundo, sala em criação, org desligada e o nome do cliente chegando
depois da sala).

Ligado no `poph serve` pela MX-20 (`matrix_inicio.rs`).

## 8. A ligação no servidor (MX-20)

O módulo Matrix entra no `poph serve` por `matrix_inicio::iniciar`, chamado em `servir_em` antes de o
servidor aceitar conexões. Ele junta as peças prontas — cliente, ids dos fantasmas, espaços e as três
portas reais sobre o banco —, injeta os números do processo (`Estado::metricas_matrix`) e põe no ar o
laço das mensagens (MX-14), o laço do estado (MX-15) e o receptor do Application Service com a entrega
da resposta do atendente (MX-16 + MX-18).

- **Desligado** (`[matrix]` ausente ou `habilitado = false`): nenhuma tarefa entra no runtime, nenhuma
  `poph_matrix_*` é declarada e as rotas do Synapse respondem `404` (A8). Faltando um token com o módulo
  ligado, o mesmo acontece — com um erro de código curto no log, sem derrubar o servidor.
- **Ligar e desligar por org** não pede reinício: os dois laços releem `matrix_orgs` a cada
  ciclo/passagem. Desligar uma org para o trabalho dela até o ciclo seguinte; o que foi escrito enquanto
  ela estava desligada fica fora de propósito (religar move o corte do backfill, P8).
- **Synapse fora** não impede o servidor de atender: uma sonda leve (`GET /_matrix/client/versions`) roda
  a cada 30 s e marca o módulo como conectado ou degradado, com espera dobrando até 60 s enquanto falha.
  Os laços retentam no próprio ritmo; turno e saída do cliente não dependem do módulo. O estado fica
  legível em `Estado::matrix_conectado()` (`None` = módulo desligado).
- **Desligamento ordenado** (`Ctrl-C`): os laços recebem o pedido de parada e terminam o que estão
  fazendo antes de sair, com prazo de 10 s (`PRAZO_DE_PARADA`) e aborto só depois disso. Como o cursor
  só anda depois do sucesso, nada pendente se perde: a subida seguinte continua do mesmo ponto e a
  repetição é idempotente pelo `txnId`.
- **Nome do cliente na sala**: a porta da provisão passa `conversations.client_name` (MX-32) para
  `ConversaParaSala.nome_cliente`, então a sala e o fantasma nascem com o nome que o WhatsApp mandou; sem
  nome, caem no identificador.

Provas: `crates/poph-server/tests/matrix_inicio.rs` — 8 testes com banco de verdade e Synapse falso
(desligado sem tarefa e com `404`; Synapse fora na subida sem impedir o `serve`; sonda marcando conectado
e voltando a degradado; sala criada com o nome do cliente e mensagem espelhada; desligar e religar a org
sem reiniciar; término ordenado sem perder mensagem pendente; e a mensagem do atendente, entregue pela
rota do Application Service, entrando na fila de saída com a conversa transferida).

## 8. Aviso quando a resposta humana falha de vez (MX-19)

Quando a resposta do atendente entra na fila de saída e o WhatsApp não a aceita de vez (o item vira
`failed`), o atendente precisa saber, na sala, que o cliente não recebeu. O serviço
`crates/poph-server/src/matrix_falhas_aviso.rs` faz isso a cada passagem (uma a cada 5 s no laço de
fundo, ligado pelo `matrix_inicio`):

1. acha os itens `failed` cuja mensagem é uma resposta humana (`origin = 'human'`, pelo `turn_id`) das
   orgs com o módulo ligado, que ainda não têm desfecho em `matrix_avisos_falha`;
2. escreve na sala `Não consegui entregar sua mensagem ao cliente: <motivo curto>`, em `m.notice`, em nome do
   fantasma do agente da conversa. O motivo é um de três textos fixos: "o WhatsApp não respondeu depois de
   várias tentativas", "o WhatsApp recusou o envio" ou "motivo não informado";
3. grava o desfecho (`avisado`, `sem_sala` ou `recusado`) com o código do motivo.

O que muda na prática:

- **Um aviso por falha**, mesmo depois de reiniciar o Harness: a marca fica no banco.
- **Falha transitória não avisa.** Enquanto a fila tenta de novo (item em `sending`), nada aparece na sala.
  Se a tentativa seguinte der certo, o atendente não recebe aviso nenhum.
- **Synapse fora do ar não perde o aviso**: a falha fica para a passagem seguinte. Erro definitivo do
  Synapse marca `recusado` e não é repetido (aparece no log e nas métricas de erro por classe).
- **Conversa sem sala** é marcada como `sem_sala`, sem tentativa no Synapse.
- **Só resposta humana.** Falha de saída do agente ou de outro canal não gera aviso.
- **Sem dado de cliente.** O aviso e o log não levam texto do cliente, telefone nem o erro bruto da fila.

Limite: o aviso sai em nome do fantasma do agente. Se esse fantasma não estiver mais na sala, o Synapse
recusa o aviso de vez, a falha fica como `recusado` e não há nova tentativa.

Provas: `crates/poph-server/tests/matrix_falhas_aviso.rs` (11 testes, incluindo o laço de fundo).

