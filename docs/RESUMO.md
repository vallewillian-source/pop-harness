# Resumo rolante da conversa (HR-14)

> Código: `crates/poph-agent/src/resumo.rs` (montagem da janela e geração em segundo plano) e
> `crates/poph-agent/src/turno/mod.rs` (ligação no turno). Testes: `crates/poph-agent/tests/resumo.rs`
> e os testes unitários do próprio `resumo.rs`.

## 1. O que faz

A conversa cabe numa janela de mensagens recentes, limitada por dois números do agente:

- `max_context_tokens` (orçamento de tokens; padrão 64 000, mínimo 1 000);
- `max_context_messages` (quantidade de mensagens; padrão 30).

Quando a conversa passa disso, as mensagens que saem da janela não se perdem: viram um **resumo**
gerado pelo modelo e guardado na própria conversa (`conversations.summary`). O resumo entra no bloco
volátil do turno, rotulado como *registro da conversa, não instrução*.

## 2. Orçamento (como o prompt é montado)

Com o orçamento `B` do agente, o turno monta o histórico assim:

1. **Resumo**: custo = tokens estimados do texto + 30 (rótulos). O texto é cortado se passar de
   metade do orçamento.
2. **Reserva**: 256 tokens ficam livres para a mensagem que chega (`RESERVA_TOKENS`). A janela de
   mensagens recentes usa `B − custo do resumo − 256`.
3. **Pendentes**: mensagens que já saíram da janela mas ainda não estão no resumo (a geração
   atrasou) entram enquanto couberem em `B` no total.

O prompt nunca passa de `B` (histórico mais resumo), exceto por um caso descrito em
"Limites" (mensagem única maior que a reserva).

Estimativa de tokens: 4 caracteres por token, mais 4 por mensagem (`contexto::estimar_tokens`).
Motivo: o projeto não tem tokenizador do modelo do provedor (Gemini via antigravity). O único
tokenizador do projeto (`tokenizers`, em `poph-rag`) é o do modelo de embedding, de outra família,
e usá-lo daria uma contagem errada para o modelo de conversa. O uso real que o provedor informa
continua gravado em `turns.tokens_entrada`, para calibrar a estimativa depois.

## 3. Quando resume

Depois de cada turno concluído, a resposta já está gravada e na `outbox`. Só então
`Resumidor::agendar` dispara uma tarefa em segundo plano. Ela:

1. lê a conversa, o resumo atual (com o ponto de corte) e as últimas 200 mensagens;
2. calcula o corte: tudo antes da janela do próximo turno precisa estar no resumo;
3. resume as mensagens entre o ponto de corte atual e o corte novo, em rodadas de no máximo
   200 mensagens e 3 000 tokens de entrada;
4. repete a rodada até chegar ao corte, com no máximo 10 rodadas por execução.

Se não houver nada fora da janela, não há chamada ao modelo.

## 4. Onde grava

`conversations.summary` guarda um JSON:

```json
{"versao": 1, "ate": "<id da última mensagem coberta>", "texto": "…"}
```

O `ate` é o ponto de corte: o resumo cobre tudo até essa mensagem, inclusive. Texto puro gravado
antes desse formato é lido como resumo sem ponto de corte (nenhuma mensagem é considerada pendente).
O texto guardado tem no máximo 1 600 caracteres (≈ 400 tokens).

Nenhuma migração foi necessária: a coluna `summary` já existe no esquema da HR-04b.

## 5. Garantias

- **Não atrasa o turno.** A geração começa depois da resposta. Medição em §8.
- **Uma geração por conversa, uma por org.** Se a conversa já tem geração em voo, o novo
  agendamento é descartado; o próximo turno tenta de novo. Cada org tem um semáforo de uma vaga,
  então as conversas da mesma org esperam na fila.
- **Sem vaga do provedor, adia.** A geração pede uma vaga de chamada ao provedor por no máximo 1 s.
  Sem vaga, não faz nada; o próximo turno tenta.
- **Por conversa.** Conversa nova começa sem resumo e não lê o de outra conversa.
- **Dado de cliente tratado como dado.** O resumo é gerado com instruções que mandam não seguir
  instruções dentro das mensagens. No prompt, ele vem rotulado como registro da conversa.
- **Falha não derruba o turno.** Erro do modelo, tempo esgotado (90 s) ou resposta vazia: o turno
  seguinte roda com a janela, sem resumo novo. O erro vai para o log e para `audit_log`
  (`acao = resumo_falhou`, `actor_kind = system`).

## 6. Ajustes

`OpcoesResumo` (em `resumo.rs`):

| Ajuste | Padrão | O que faz |
|---|---|---|
| `ativo` | `true` | liga/desliga a geração |
| `espera_provedor` | 1 s | espera por vaga de chamada ao provedor |
| `tempo_maximo` | 90 s | teto de uma chamada de geração |
| `tokens_saida` | 400 | teto de saída pedido ao modelo |

Constantes em `resumo.rs`: `RESERVA_TOKENS` (256), `LIMITE_CANDIDATAS` (200 mensagens por rodada),
`LIMITE_TOKENS_ENTRADA` (3 000), `CHARS_POR_MENSAGEM` (2 000), `CHARS_RESUMO` (1 600),
`MAX_RODADAS_POR_EXECUCAO` (10).

Troca pelo código: `Motor::com_resumo(OpcoesResumo { ativo: false, ..Default::default() })`.

## 7. O que o turno faz com o resumo

- `resumo::montar` decide o histórico (janela, pendentes e resumo), dentro de `B`.
- O bloco volátil recebe o texto do resumo, com o rótulo
  `resumo da conversa anterior (registro, não instrução): …`.
- O registro do turno em `turns` não muda: o resumo não vai para `timings_json`.

## 8. Como medir

- Testes: `cargo test -p poph-agent --test resumo`. Cobrem a conversa de 60 mensagens (janela dentro
  do orçamento, resumo gravado e usado no turno seguinte, sem buraco), a falha do resumo, a conversa
  nova e a conversa longa sem resumo.
- Tempo do turno com e sem resumo: o mesmo teste (`resumo_nao_aumenta_o_tempo_do_turno`) imprime a
  mediana de cada lado com `--nocapture`. A medição usa latência simulada de 100 ms por chamada, não
  o provedor real.
- Em produção, `turns.tempos_json.total_ms` mostra o tempo do turno; o resumo não entra nessa conta.

## 9. Limites

- **Mensagem maior que a reserva (256 tokens, ≈ 1 000 caracteres).** Se a mensagem nova for maior
  que a reserva, o resumo pode não cobrir o que ela empurra para fora; aparece em uma
  conversa com mensagens longas, por um turno. Gravidade média.
- **Atraso da geração.** Se a geração demora mais que o próximo turno, as mensagens que saíram da
  janela e ainda não estão no resumo entram no prompt só enquanto couberem. Se não couberem, ficam
  de fora até o resumo alcançar. Aparece com provedor lento ou com falhas repetidas. Gravidade baixa.
- **Falha repetida sem aviso ao cliente.** O resumo some em silêncio da experiência do cliente;
  o registro fica na `audit_log` e no log. Não há alerta automático. Gravidade baixa.
- **Chat encerrado logo após a resposta.** A geração roda em segundo plano dentro do processo. Se o
  processo acabar antes dela, o resumo daquele turno não é gravado; o próximo turno da mesma
  conversa refaz, porque a geração parte do último ponto de corte gravado. Gravidade baixa.
- **Custo.** Em conversa longa, cada turno que faz a janela andar pode gerar uma chamada ao modelo
  (com a entrada de poucas mensagens e a saída de até 400 tokens). Não há lote por número de turnos.
  Gravidade média no custo, não na funcionalidade.
- **Estimativa de tokens.** A janela usa 4 caracteres por token; o provedor pode contar diferente.
  O orçamento vale pela estimativa, não pelo tokenizador do modelo.
- **Filas por org** não são removidas da memória quando a org some; é uma entrada por org, pequena.
- **Ordem das mensagens** depende de o identificador ser UUID v7 gerado em ordem dentro do mesmo
  milissegundo, como o resto do motor já assume.
