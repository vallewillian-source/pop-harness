# WhatsApp pela EvolutionAPI no POP Harness (HR-26)

Este documento responde as três verificações que a HR-26 pede — **(a)** reenvio do webhook em 5xx,
**(b)** autenticação do webhook e **(c)** a regra de mensagem longa — e descreve o que o módulo
nativo entrega, com a evidência no código do POP Atende.

- **Código:** `crates/poph-evolution` (`cliente.rs`, `webhook.rs`, `texto.rs`, `integracao.rs`).
- **Contrato:** o mesmo da HR-25 (`poph_types::Integration`), igual ao plugin em processo externo.
- **Testes:** 50 (9 de unidade, 17 da entrada, 23 do cliente/canal contra Evolution falso e 1 doc test).

---

## 1. As três verificações

### (a) O Evolution reenvia o webhook quando respondemos 5xx?

**Não foi medido com uma instância real** — depende da conta de teste, que é do mantenedor. O que o
repositório prova:

1. **O POP Atende assume que sim.** Ele mantém uma tabela de deduplicação de eventos com 24 h de
   validade (`core/server/routes/evolution_bridge.py:184`, `_WEBHOOK_DEDUPE_TTL_HOURS = 24`), chave
   `wev_<instância>_<id da mensagem>` e `INSERT … ON CONFLICT DO NOTHING`. Deduplicar por id só faz
   sentido se a mesma entrega pode chegar duas vezes.
2. **Ele responde 5xx em vários caminhos** (`evolution_bridge.py:999`, `1002`, `1015`, `1021` —
   falha/timeout ao chamar o agente).
3. **E, apesar disso, a reentrega seria descartada**: a reivindicação do evento acontece **antes** de
   qualquer processamento (`evolution_bridge.py:4701`, comentário "claim the webhook event BEFORE any
   processing") e não existe devolução da reivindicação quando o processamento falha. Ou seja: se a
   Evolution reenviar depois de um 5xx, o POP Atende responde `duplicate_message_claimed` e a
   mensagem do cliente **se perde**.

**O que o Harness faz (decisão, não herança).** A `inbox` da HR-12 grava a mensagem e só então o
webhook recebe `200`; o item só ganha `processed_at` quando o turno conclui
(`crates/poph-agent/src/entrada_saida.rs`, documentação do módulo). A chave única
`(integration_id, external_message_id)` faz a segunda entrega ser inofensiva **sem** impedir que uma
entrega que falhou seja processada depois. Portanto: reenvio do Evolution é **bem-vindo** aqui, e o
efeito é exatamente um turno por mensagem.

**Como fechar a medição quando houver conta de teste:** subir a instância, apontar o webhook para um
endpoint que responde `500` de propósito e contar as entregas do mesmo `key.id` no log da Evolution
(ou em um arquivo do endpoint falso). Se ela não reenviar, nada muda no Harness — a mensagem continua
sendo processada porque a retomada da `inbox` despacha item pendente na subida.

### (b) Como autenticar o webhook

- **Cabeçalho:** `X-Evolution-Webhook-Secret`, conferido contra o segredo configurado na integração.
- **Recusa:** segredo errado ou ausente → a entrega é recusada; sem segredo configurado, **nenhuma**
  entrega é aceita (`ErroWebhook::SemSegredoConfigurado`).
- **Comparação em tempo constante** (`webhook::conferir_segredo`): o conteúdo não vaza pelo tempo de
  resposta.
- **Do lado da Evolution**, o segredo viaja no próprio `webhook/set` (`headers`), como o POP Atende já
  faz — é o que `ClienteEvolution::definir_webhook` monta ao iniciar o canal.
- Evidência no POP Atende: `core/server/routes/evolution_bridge.py:4577-4579`
  (`X-Evolution-Webhook-Secret`, `403 Invalid Evolution webhook secret`).

### (c) Como o POP Atende trata mensagem longa — e a regra reproduzida

**Ele não divide: ele corta.** `apps/backend/core/utils/markdown_to_whatsapp.py`:

- `WHATSAPP_TEXT_LIMIT = 4000`;
- converte Markdown para o que o WhatsApp renderiza (negrito `*x*`, itálico `_x_`, riscado `~x~`,
  título em negrito, link `texto (url)`, lista `• item`, tabela achatada em `coluna: valor`);
- e, no fim, `text[: LIMITE - 1].rstrip() + "…"`.

A tarefa pedia "como divide mensagens longas e envia em sequência". **Isso não existe no POP Atende
hoje**: um texto acima de 4000 caracteres chega ao cliente truncado, não em duas mensagens. O
Harness reproduz o comportamento (`texto::para_whatsapp` + `texto::cortar`), porque paridade de canal
vale mais do que a suposição da tarefa — e porque dividir em N mensagens muda o que o cliente vê.

**Consequência que fica registrada:** um turno que produza resposta acima de 4000 caracteres perde o
final. Dividir em sequência (com o `(turn_id, seq)` da `outbox` já existente) é uma mudança de
comportamento visível ao cliente: fica para decisão do mantenedor, não para esta entrega.

---

## 2. O que o módulo entrega

| Peça | O que faz | Onde |
|---|---|---|
| `ClienteEvolution` | criar instância, parear (QR), estado, desconectar, apagar, `webhook/set`, `sendText`, `sendMedia`, `getBase64FromMediaMessage`, `sendPresence` | `cliente.rs` |
| `webhook::interpretar` | `messages.upsert` → `InboundEvent` (texto, imagem, PDF/Office) e `connection.update`/`qrcode.updated` → `EventoEstado` | `webhook.rs` |
| `texto::para_whatsapp` | Markdown → formato do WhatsApp + corte em 4000 | `texto.rs` |
| `WhatsAppEvolution` | o canal como `Integration`: enviar texto, enviar arquivo da biblioteca, presença, saúde, estado | `integracao.rs` |

Regras que vieram do POP Atende e por quê:

- **`fromMe = true` é descartado** — é o eco da nossa própria mensagem (`evolution.py:16`).
- **Idempotência pelo `key.id` do WhatsApp** — reentrega não gera segundo turno.
- **Só imagem e documento (PDF/Office) entram como anexo.** Áudio, vídeo, figurinha, contato e
  localização são ignorados **com motivo**; e documento de MIME fora da lista é recusado. É a decisão
  do mantenedor (arquivo = imagem + PDF/Office em texto, sem áudio).
- **Grupo vs. pessoa:** conversa individual usa o número (só dígitos, que é o que o `sendText`
  aceita); grupo usa o jid completo (`…@g.us`) e quem escreveu vai em `remetente`.
- **Mídia:** teto de **16 MB**, conferido **antes** de decodificar o base64 (um arquivo enorme não é
  carregado na memória para depois ser recusado); a Evolution guarda a mídia por **4 dias**, e passou
  disso ela responde "Message not found" → `ErroEvolution::MidiaExpirada`, que é erro **definitivo**
  (não é retentado).
- **Retentativa:** uma, só em queda de rede/prazo/429/5xx; 400/401/404 não são repetidos.
- **A chave (`apikey`) só no cabeçalho** e nunca em erro, `Debug` ou log — há teste que falha se ela
  aparecer na mensagem de erro.
- **Mídia que falha não derruba o texto:** se o download estourar o prazo ou a mídia tiver expirado, o
  evento chega ao turno **sem o anexo**, com o texto do cliente preservado.

## 3. Como rodar os testes

```bash
cd apps/pop-harness
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p poph-evolution            # 50 testes, nenhum usa rede externa
```

O servidor falso dos testes é um `TcpListener` em porta efêmera com roteiro combinado
(`crates/poph-evolution/tests/evolution.rs`): ele registra o que recebeu (método, trajeto,
cabeçalhos, corpo) e responde o que o teste mandar. Não há chave real, não há Evolution de verdade.

## 4. O que falta para produção

1. **Conta de teste real** (item do mantenedor): criar instância, ler o QR, mandar e receber texto,
   imagem e PDF numa instância de verdade. É o único passo do aceite da HR-26 que não fecha aqui.
2. **Registro de manifesto no servidor**: hoje a API recusa configurar integração (`422
   manifest_indisponivel`) até existir uma fonte confiável do manifesto. O manifesto do WhatsApp já
   está escrito em código (`integracao.rs`), com `apikey` e `webhook_secret` marcados como
   `format: password` + `x-secret: true`; quem hospeda esse registro liga a config da integração.
3. **Rota de webhook no servidor** (`poph serve`): o canal já sabe autenticar e ler o corpo, mas
   nenhuma rota HTTP o chama ainda — ela precisa do registro de integração e do motor de turno no
   servidor, que hoje não roda turnos.
4. **Persistência da mídia:** o canal entrega o conteúdo e o fluxo decide; gravar o blob é do
   `poph-files` (HR-23), que já existe — o encaixe é na rota da webhook, junto do item 3.

## 5. Notas de campo

- **`sendMedia` aceita base64** (é o que o POP Atende usa quando não há URL pública temporária). Para
  arquivo grande isso é caro: 16 MB viram ~21 MB de JSON. O caminho com URL pública (link temporário)
  não foi implementado porque o Harness ainda não serve arquivos por HTTP.
- **`getBase64FromMediaMessage` exige o objeto `data` do webhook**, não só o id: com o id sozinho a
  Evolution procura no banco dela e responde "Message not found" para mídia que ela não guardou
  (`evolution.py:272`).
- **O `webhook/set` é por instância, com `byEvents: false`** — igual ao POP Atende
  (`evolution_api.py:214`), e com a lista `MESSAGES_UPSERT`, `CONNECTION_UPDATE`, `QRCODE_UPDATED`.
- **`WEBHOOK_GLOBAL_ENABLED` fica `false`** nos dois compose do POP Atende
  (`infra/docker-compose.dev.yml:107`, `infra/docker-compose.oci.yml:197`): a configuração é sempre por
  instância, nunca global.
