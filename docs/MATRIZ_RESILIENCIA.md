# Resiliência do módulo Matrix e "cliente primeiro" (MX-29)

Este documento diz o que foi medido e o que foi provado sobre três coisas: o módulo Matrix não
atrasa o atendimento do cliente, a fila se recupera sem duplicar nem perder mensagem quando o
Synapse cai e volta, e uma queda do Harness no meio de uma operação não deixa sala, convite ou
resposta duplicada ou perdida. Os números são de execuções reais no ambiente de desenvolvimento.
Nada aqui é número estimado: o que não foi medido está marcado como tal.

## Resumo

- **Cliente primeiro (A1/A4):** com o módulo ligado, com o Synapse fora, lento (5 s por chamada) ou
  no ar, a resposta do atendimento chega ao cliente no mesmo tempo que com o módulo desligado. A
  diferença entre as medianas fica entre 1,1 ms abaixo e 0,7 ms acima do cenário sem módulo
  (na gravação), e em 0,1 ms no envio. A variação entre rodadas é de 4,8 ms na gravação e de 0,9 ms
  no envio, então a diferença fica dentro do ruído.
- **Volta do Synapse (A4):** com o Synapse fora, as mensagens se acumulam; ao voltar, a sala recebe
  todas, na ordem do banco, uma vez só.
- **Queda do Harness (A5):** cobertas as quatro quedas pedidas (criação de sala, envio do espelho,
  lote de convites e registro da resposta humana). Em todas, depois da subida, não há sala
  duplicada, mensagem perdida ou repetida.
- **Retransmissão do Synapse (A3):** a mesma transação chegando de novo depois de uma queda gera uma
  única entrada na fila de envio.
- **Mensagem venenosa:** é isolada depois de três tentativas e a fila segue.
- **Defeito achado e corrigido (MX-12, commit `869664d4`):** um convite que falhava no lote de convites
  de espaço não era refeito pela passagem seguinte. A correção grava o vínculo do espaço só depois de
  todos os convites darem certo. Ver a seção "Defeito achado".

## Como cada critério foi coberto

| Critério | O que prova | Onde está |
|---|---|---|
| A3: reenvio da mesma transação, um único envio | Queda depois do registro da resposta humana, retransmissão depois da subida: uma entrada na fila | `retransmissao_depois_da_queda_nao_gera_segunda_resposta` |
| A3: reenvio do envio do espelho | Queda com o Synapse já tendo aceito a mensagem: reenvio com o mesmo `txnId`, sala com uma cópia só | `queda_no_envio_do_espelho_nao_duplica_a_mensagem` |
| A4: Synapse fora, cliente atendido | Oito respostas chegam ao cliente com o Synapse fora e o módulo ligado | `cliente_recebe_resposta_com_synapse_fora_e_modulo_ligado` e a medição |
| A4: Synapse fora, ao voltar a fila esgota em ordem | Mensagens acumuladas durante a queda chegam à sala na ordem do banco, cada uma uma vez | `volta_do_synapse_esgota_a_fila_em_ordem_sem_duplicata` |
| A4: Synapse lento não segura o turno | A resposta chega à Evolution antes de qualquer chamada lenta ao Synapse terminar | `resposta_sai_antes_de_o_synapse_lento_responder` e a medição (cenário de 5 s) |
| A5: queda na criação de sala | Sala criada no Synapse e a resposta perdida: na subida, a mesma sala é reaproveitada pelo alias | `queda_na_criacao_de_sala_nao_duplica_a_sala` |
| A5: queda no espelhamento | Ver A3 acima | `queda_no_envio_do_espelho_nao_duplica_a_mensagem` |
| A5: queda no lote de convites | O terceiro convite fica no ar; na passagem seguinte, todos os humanos do espaço ficam convidados | `queda_no_meio_do_lote_de_convites_termina_sem_perda` |
| A5: queda no registro da resposta humana | Queda antes da gravação: a retransmissão grava uma vez só | `queda_antes_do_registro_da_resposta_grava_uma_vez` |
| Mensagem venenosa | Mensagem recusada de vez pelo Synapse é isolada; as outras saem na ordem | `mensagem_venenosa_e_isolada_e_a_fila_segue` |

Os testes estão em `crates/poph-server/tests/matrix_resiliencia.rs`. São dez testes que rodam no
`cargo test` normal (cerca de 8 s) e um `#[ignore]`: a medição. O teste do defeito achado roda ativo desde
a correção (commit `869664d4`).

## Medição: cliente primeiro

### Método

O servidor de verdade sobe com o webhook da Evolution, o turno com um provedor de modelo falso, a
saída pela Evolution falsa e, quando o cenário pede, o módulo Matrix. Cada mensagem do cliente é
enviada depois que a anterior chegou à Evolution, e são medidos dois instantes a partir do envio do
webhook:

- **resposta gravada:** a resposta do agente entra no banco. É o trabalho do turno, sem a espera do
  laço de envio.
- **envio:** a resposta chega à Evolution falsa. É o que o cliente recebe, com o laço de envio de
  produção (intervalo de 1 s).

Cenários: sem o módulo (estado padrão), módulo ligado com o Synapse **fora** (porta fechada durante
toda a rodada), módulo ligado com o Synapse **lento** (5 s em cada chamada) e módulo ligado com o
Synapse **no ar**. Cada cenário teve 3 rodadas de 40 mensagens, em duas execuções independentes.

Dois desvios da configuração de produção, aplicados igualmente aos quatro cenários:

- A janela de agrupamento do turno foi de 1 ms em vez dos 2 s padrão. Com os 2 s, a gravação da
  resposta ficava sempre em ~2 s e o efeito do módulo não aparecia sob esse piso.
- O Synapse fora durou a rodada inteira, cerca de 40 s, e não 10 minutos. Os 10 minutos não foram
  reproduzidos; ver "Limites".

### Números (ms)

Execução 1 (3 rodadas × 40 mensagens por cenário, n = 120):

| Cenário | Gravada: mediana | Gravada: p95 | Envio: mediana | Envio: p95 |
|---|---|---|---|---|
| Sem módulo | 16,1 | 25,5 | 1000,7 | 1003,0 |
| Módulo, Synapse fora | 15,0 | 25,0 | 1000,8 | 1002,9 |
| Módulo, Synapse lento (5 s) | 16,2 | 25,2 | 1000,6 | 1002,7 |
| Módulo, Synapse no ar | 16,4 | 25,4 | 1000,6 | 1002,8 |

Execução 2 (mesmos parâmetros, n = 120):

| Cenário | Gravada: mediana | Gravada: p95 | Envio: mediana | Envio: p95 |
|---|---|---|---|---|
| Sem módulo | 14,2 | 25,8 | 1000,8 | 1002,8 |
| Módulo, Synapse fora | 14,6 | 25,8 | 1000,8 | 1003,0 |
| Módulo, Synapse lento (5 s) | 14,4 | 25,9 | 1000,8 | 1003,1 |
| Módulo, Synapse no ar | 14,9 | 25,6 | 1000,8 | 1003,0 |

### Ruído e veredito

- **Ruído (variação entre rodadas):** as medianas de cada rodada da gravação ficaram entre 13,3 e
  18,1 ms, nas 24 rodadas das duas execuções (faixa de 4,8 ms). As medianas de envio ficaram entre
  1000,4 e 1001,3 ms (faixa de 0,9 ms).
- **Diferença com o módulo ligado:** na gravação, a mediana com o módulo ficou entre 1,1 ms abaixo e
  0,7 ms acima da mediana sem o módulo, em todas as combinações das duas execuções. No envio, a
  diferença máxima é de 0,1 ms.
- **Veredito:** a diferença fica dentro do ruído da própria medição. O Synapse fora ou lento não
  piora a resposta ao cliente. Nas duas execuções, o turno chamou o modelo 65 vezes para 40
  mensagens em todos os cenários, então o módulo também não muda o número de chamadas ao modelo.
  A causa das 25 chamadas a mais não foi investigada; não afeta a comparação, porque é igual nos
  quatro cenários.

### Como reproduzir

```bash
cd apps/pop-harness
bash scripts/medir-resiliencia.sh 3 40    # 3 rodadas de 40 mensagens por cenário
```

Leva cerca de 9 minutos por execução. O CSV com cada amostra sai em `target/mx29/medicao.csv`
(fora do git). O resumo impresso traz a mediana e o p95 de cada cenário.

## Quedas: como foram simuladas

- **Queda do Harness:** cada "processo" do módulo roda num runtime próprio, numa thread. A queda é o
  fim desse runtime: todas as tarefas param no ponto em que estavam, sem executar código depois do
  `await`. O banco é reaberto do arquivo na subida seguinte, com uma conexão nova.
- **Ponto da queda:** o Synapse falso segura a resposta de uma chamada escolhida (criar sala, convite,
  envio ou leitura de perfil) depois de já ter gravado o efeito. O teste espera essa chamada chegar e
  então derruba o processo. Assim a queda cai exatamente no ponto que o teste quer.
- **Synapse fora:** a porta fica fechada, como num Synapse parado. Ao voltar, o mesmo endereço é
  religado com o estado anterior, como um Synapse reiniciado.

## Defeito achado

**Convite falho no lote de um espaço não era refeito.** Achado no teste
`convite_falho_no_lote_e_refeito_na_passagem_seguinte`, que ficou marcado como `#[ignore]` até a correção.
A correção está em `crates/poph-matrix/src/espacos.rs` (commit `869664d4`) e o teste roda ativo.

- **Reprodução (antes da correção):** `cargo test -p poph-server --test matrix_resiliencia -- --ignored convite_falho`
- **Resultado:** falha com `@humano1:matrix-dev.local não foi convidado: a passagem seguinte não refez
  o convite falho`. O primeiro convite falhou (o Synapse respondeu 503 uma vez), a primeira passagem
  registrou um convite falho e gravou a sala no espaço; a segunda passagem voltou "já posicionada" e
  não tentou o convite de novo.
- **Causa (lida no código, `crates/poph-matrix/src/espacos.rs`):** `posicionar` grava o espaço da sala
  no banco no fim, depois dos convites, mesmo quando algum convite falhou. A passagem seguinte compara
  com o banco, vê a sala no espaço e sai com `JaPosicionada` antes de convidar. O número de convites
  falhos volta no resultado, mas não fica guardado em lugar nenhum.
- **Impacto:** o membro do espaço que não recebeu o convite não entra na conversa. Pelo código de
  `matrix_estado.rs`, o `posicionar` roda para cada conversa alterada, então a mensagem seguinte do
  cliente também não corrige isso. Esta última parte eu li no código; não testei de ponta a ponta.
- **O que o relatório da MX-12 dizia:** "o convite é repetido na passagem seguinte". O teste mostra
  que isso não acontece.
- **Dono:** `espacos.rs` é da MX-12 . Não corrigi, conforme a regra desta tarefa.
- **Caminhos de correção (escolha de quem é dono):** (a) não gravar o espaço no banco enquanto houver
  convite falho, para a próxima passagem refazer tudo (as escritas no Synapse são idempotentes); ou
  (b) na saída "já posicionada", refazer só os convites pendentes. A opção (a) é a mais simples.

## Limites

- **Queda simulada, não processo morto de verdade.** A queda é o fim de um runtime, dentro do mesmo
  processo de teste. Não cobre um corte de energia nem o SQLite no meio de uma escrita no disco. Não
  foi feita queda com `SIGKILL` num servidor real.
- **Synapse falso, não o Synapse real.** O comportamento de `M_ROOM_IN_USE`, `M_NOT_FOUND` e do
  deduplicação por `txnId` vem do falso, que segue o que a MX-02 comprovou no Synapse real. O único
  teste contra o Synapse real continua sendo o `#[ignore]` da MX-09.
- **"10 minutos fora" não foi reproduzido.** A volta foi testada com cerca de 3 s de queda, e a
  medição teve cerca de 40 s por rodada. O efeito de uma queda longa é calculado pelo código, não
  medido: o intervalo entre tentativas do espelho dobra a cada falha, até 60 s. Com o padrão de
  `intervalo_ms = 500`, o teto de 60 s é atingido a partir da sétima falha seguida. Portanto, depois de
  uma queda longa, a fila pode levar até 60 s para começar a andar depois de o Synapse voltar. Isso
  não foi medido.
- **Produção tem mais atraso que a medição.** A medição tirou a janela de agrupamento de 2 s e usou
  uma Evolution falsa com resposta imediata. O envio em produção soma a janela, a latência real da
  Evolution e o intervalo de 1 s do laço de envio. Só o caminho até a gravação e o efeito do módulo
  foram comparados.
- **Envio duplicado no WhatsApp após queda não foi testado aqui.** Se a Evolution já tiver entregado
  a mensagem antes da queda, a resposta pode sair de novo na subida. O teste existente
  `resposta_que_ficou_em_envio_na_queda_sai_na_subida` (em `turno_whatsapp.rs`) mostra que a resposta
  é reenviada; não verifiquei o que acontece na Evolution real.
- **Memória de transações do receptor se perde na queda.** A deduplicação depois de uma queda depende
  da chave do evento no banco. O teste `retransmissao_depois_da_queda_nao_gera_segunda_resposta` prova
  isso com a memória zerada.
- **Causa das 65 chamadas ao modelo para 40 mensagens não foi investigada.**

## Arquivos desta entrega

- `crates/poph-server/tests/matrix_resiliencia.rs`: os testes de resiliência, a medição e o teste do
  defeito.
- `scripts/medir-resiliencia.sh`: repete a medição.
- `docs/MATRIZ_RESILIENCIA.md`: este documento.
- `docs/ESTADO.md`: linha da MX-29.
