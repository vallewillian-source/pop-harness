# POP Harness: retenção do histórico (HR-51)

Apaga o histórico que passou do prazo de cada org. Padrão: 30 dias (PRD §7.4). `0` = nunca apagar.

## Comandos

```
poph retention run [--dry-run] [--json]     # todas as orgs
poph --org <slug|id> retention run [...]    # só uma org
poph retention show [--json]                # dias de retenção de cada org
poph --org <slug|id> retention show         # só uma org
```

- `run` apaga. `run --dry-run` só mostra o que sairia; banco e pasta de arquivos ficam como estavam.
- O prazo vem de `retencao.dias` em `orgs.config_json`. Sem a chave, vale `retencao.dias` da
  configuração da instância (padrão 30).
- Valor inválido (negativo, texto, decimal, acima de 36.500) é erro. A org é pulada inteira, e a
  mensagem diz "Nada foi apagado nessa org". Nenhuma org é apagada "por padrão".
- Saída: `0` quando nenhuma org deu erro; `1` quando alguma deu; `2` é uso incorreto.
- `--json` tem `versao_relatorio: 1`. Em `run`, o campo `banco.ids_conversas` traz as conversas
  afetadas (o que a simulação mostra para conferir antes de apagar).

## O que sai (só o vencido, da org varrida)

| Tabela | Critério |
|---|---|
| `messages` (e `attachments` delas) | `created_at` anterior ao corte, mesmo se a conversa estiver aberta |
| `outbox` | `state = 'sent'` e enviada antes do corte |
| `inbox` | `processed_at` preenchido e anterior ao corte |
| `turns` | `state` diferente de `running`, encerrado antes do corte, sem saída ainda não enviada e sem mensagem apontando para ele |
| `conversations` | criada antes do corte e **vazia**: sem mensagem, turno, saída nem carrinho |
| blobs em `<blobs>/<org>/` | sha256 sem nenhum anexo vivo nem arquivo da biblioteca; `.tmp` abandonados |

A conversa só sai depois que o corte apagou as mensagens dela. Uma conversa criada antes do
corte com mensagem recente fica inteira.

## O que nunca sai

Base de conhecimento (grupos e itens), produtos e categorias, biblioteca de arquivos do agente,
orgs, agentes, instruções, integrações, tokens, departamentos, carrinho e `audit_log`.

Do módulo de Fluxos e Fichas, também nada sai (FX-34):

- **Fichas** (`fichas`) — a Ficha é o registro do atendimento e sobrevive à conversa (PRD L12). Se a
  varredura apaga a conversa, a Ficha fica e só perde o vínculo: `conversation_id` vai a nulo
  (`ON DELETE SET NULL`). O `status` dela não muda; quem fecha a Ficha é o encerramento do
  atendimento, não a retenção.
- **Execuções, etapas, eventos e efeitos** (`fluxo_execucoes`, `fluxo_execucao_etapas`,
  `ficha_eventos`, `fluxo_efeitos`) — é o que aconteceu no atendimento; ficam todos, inclusive os de
  uma Ficha cuja conversa já saiu.
- **Fluxos, revisões publicadas e permissão por agente** (`fluxos`, `fluxo_revisoes`,
  `agente_fluxos`) — revisão publicada é imutável e só sai com o fluxo inteiro (exclusão com
  `force`) ou com a org.

Não existe retenção de Ficha: enquanto a decisão de produto não mudar, Ficha não expira.

Pendente nunca é apagado: entrada não processada, saída `pending`, `sending` ou `failed`, e turno
`running`.

## Como funciona por dentro

- O relógio é recebido como parâmetro (`Opcoes::agora_ms`). Os testes simulam o futuro sem dormir.
- A regra de apagar fica no `poph-store` (repositório `retencao()`). O crate `poph-retention`
  decide o corte, lê a config e junta os arquivos.
- Ordem por org: lê a retenção → apaga no banco (uma transação por org) → lista os blobs sem dono
  → apaga os blobs → grava a auditoria.
- Se o banco falhar, nenhum arquivo é apagado.
- Com `--dry-run`, o banco faz as mesmas contas dentro de um savepoint e desfaz no fim. As contagens
  são as mesmas da execução real.
- Blobs: só entra arquivo cujo nome tem 64 hexadecimais (o sha256) ou termina em `.tmp`. Pasta e
  link simbólico nunca entram. Arquivo modificado na última hora fica (carência fixa), para não
  apagar um blob que outro pedido acabou de gravar e ainda não registrou.
- Auditoria: uma linha `audit_log` por org varrida, com `action = retention`. Traz as contagens
  (`mensagens`, `anexos`, `turnos`, `inbox`, `outbox`, `conversas`, `arquivos_sem_dono`,
  `arquivos_apagados`), o corte e `simulacao` (`true` no dry-run). Não é gravada para org pulada.

## Limites conhecidos

- **Varredura sem índice por data.** A consulta de mensagens vencidas lê a tabela inteira da org.
  Aparece com muito histórico (milhões de mensagens). Gravidade média; medir no teste de carga.
- **Conferência de conversa vazia sem índice em `outbox(conversation_id)`.** Mesma causa; com muitas
  conversas candidatas, a varredura fica lenta. Gravidade média; resolver com índice numa migração
  futura.
- **Auditoria depois dos arquivos.** Se a linha de auditoria falhar depois de os blobs saírem, o
  relatório marca erro, mas os arquivos já foram apagados. Próxima varredura não repete o apagamento
  do banco. Gravidade baixa.
- **Queda no meio da remoção de blobs.** O que sobrou vira órfão e sai na próxima varredura. Nenhum
  dado vivo se perde. Gravidade baixa.
- **Carência de 1 hora fixa**, sem configuração. Gravidade baixa.
- **Não roda sozinha.** O worker em `poph serve` e o agendamento ficam para a integração; hoje só
  roda por `poph retention run`. Rode com o servidor parado ou sem carga, como o `poph migrate`.
- **Mensagem apagada em conversa aberta não volta.** É a regra de retenção, não defeito.
