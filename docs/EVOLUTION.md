# Cliente da EvolutionAPI (`poph-evolution`)

Primeiro pedaço do WhatsApp nativo (HR-26a). Fala com a EvolutionAPI pelas mesmas rotas e corpos
que o POP Atende usa (`core/integrations/evolution_api.py`, `core/integrations/evolution.py`).

## Operações cobertas

| Operação | Rota | Repete? |
|---|---|---|
| Criar instância | `POST /instance/create` | não |
| Parear (QR e código) | `GET /instance/connect/{instância}` (com `?number=` para código) | sim, 1 vez |
| Estado da conexão | `GET /instance/connectionState/{instância}` | sim, 1 vez |
| Listar instâncias | `GET /instance/fetchInstances` | sim, 1 vez |
| Desconectar | `DELETE /instance/logout/{instância}` | não |
| Apagar | `DELETE /instance/delete/{instância}` | não |
| Definir webhook | `POST /webhook/set/{instância}` | sim, 1 vez (configuração idempotente) |
| Enviar texto | `POST /message/sendText/{instância}` | **não** |
| Enviar arquivo | `POST /message/sendMedia/{instância}` | **não** |
| "Digitando" | `POST /chat/sendPresence/{instância}` | não |
| Baixar mídia recebida | `POST /chat/getBase64FromMediaMessage/{instância}` | **não** |

Autenticação: a chave vai só no cabeçalho `apikey`. Ela nunca aparece em erro, `Debug` ou log.

Implementado no crate: `ClienteEvolution` (as operações acima) e `WhatsAppEvolution` (a
integração no contrato `Integration`, com envio de texto, arquivo, presença e saúde).

## Id da mensagem enviada (MX-40)

A resposta do envio (`sendText` e `sendMedia`) traz o id que o WhatsApp deu à mensagem. O cliente lê
os formatos que a Evolution já usou, nesta ordem: `key.id`, `message.key.id`, `id` e `data.key.id` — o
primeiro que for texto não vazio. Não há id sintético: se nenhum vier, o resultado é "sem id".

- **`ClienteEvolution::enviar_texto` e `enviar_midia`** devolvem `Option<String>` com esse id.
- **`WhatsAppEvolution::enviar_com_recibo(ComandoSaida)`** é o caminho com recibo: entrega texto ou
  arquivo e devolve o id; presença devolve `None` (a Evolution não devolve id para "digitando").
- **O contrato genérico do canal não muda:** `Integration::enviar_texto`/`enviar_arquivo` seguem
  respondendo `Result<(), ErroIntegracao>` e descartam o id, porque o mesmo contrato serve a plugin
  em processo externo, que pode não devolver nenhum.
- **O que o Harness guarda:** a saída do WhatsApp (`crates/poph-server/src/saida_whatsapp.rs`) grava
  o id em `outbox.external_message_id` ao confirmar o envio (migração `0011`). Envio que falha, ou
  canal que não devolve id, deixa a coluna nula — e isso não é erro de envio.

Formato observado no falso dos testes (`{"key": {"id": "…"}}`) e nos registros do POP Atende; o
formato real de uma instância de produção só se confirma com um número de verdade.

## Configuração

- Endereço da Evolution (`url_base`, `http://` na rede local ou `https://`), nome da instância
  e prazo por tentativa (padrão 20 s).
- Chave: variável de ambiente `EVOLUTION_API_KEY`. Não vai em arquivo de configuração.
- Segredo do webhook: obrigatório. Vai no cabeçalho `X-Evolution-Webhook-Secret` que a
  Evolution repete em cada entrega.
- Contadores opcionais: `ConfigEvolution::com_metricas` liga
  `poph_chamadas_evolution_total` (uma por requisição HTTP) e `poph_erros_evolution_total`
  (uma por requisição que falhou). Recusas feitas antes de sair (arquivo acima do teto,
  configuração) não contam como chamada.

## Limites

- **Arquivo** (envio e download): 16 MB (`TETO_MIDIA`), conferido antes de subir ou decodificar.
  Mídia recusada no envio não sai; mídia acima do teto no download é recusada sem decodificar.
- **Resposta da Evolution**: 1 MB para respostas comuns; para download de mídia, o necessário para
  16 MB em base64. Passou disso, a leitura para e o erro é `RespostaGrande`.
- **Mídia expirada**: o download só é tratado como expirado com a resposta 400/404 que contém
  "message not found", "media not found", "media not available" ou "expired/expirad". Qualquer outro
  400 é erro comum.
- **Texto de erro do provedor**: limpo antes de virar mensagem. Número, JID (tudo com `@`),
  trecho com 7 dígitos ou mais e a própria chave saem; o texto corta em 120 caracteres.
  Limitação: texto livre sem número (por exemplo, um nome de contato que o provedor ecoe) não
  é detectado.
- **Repetição**: só leitura e configuração repetem, e só uma vez, em queda de rede, prazo, 429 ou
  5xx. Envio e download nunca repetem dentro do cliente. Quem decide tentar de novo mais tarde
  é a fila de saída, que usa a marcação de transitório do erro.

## Entrada de mensagens (webhook, HR-26b)

Rota pública do servidor, fora do `/admin/v1`:

```
POST /webhook/evolution/{org_id}/{instancia}
Cabeçalho: X-Evolution-Webhook-Secret: <segredo da instância>
```

**Configuração.** Cada instância liga uma org a uma integração WhatsApp do Harness, no arquivo de
configuração (sem segredo dentro):

```toml
[evolution]
url_base = "http://127.0.0.1:18081"

[[evolution.instancias]]
org_id = "<id da org>"
instancia = "loja"                       # nome da instância na Evolution
integracao_id = "<id da integração WhatsApp>"
segredo_env = "POPH_EVO_SEGREDO_LOJA"    # nome da variável que guarda o segredo
```

O segredo vem só do ambiente (`POPH_EVO_SEGREDO_LOJA=...`), lido na subida. A chave da Evolution
continua em `EVOLUTION_API_KEY`, usada para baixar mídia. Sem a variável do segredo, a instância
responde 401 até ela existir (o servidor avisa no log, com o nome da variável e não o valor).

**Respostas.** Pensadas para a Evolution, que reenvia 5xx e timeout (até 10 vezes em ~21 min) e
não reenvia 4xx:

| Situação | Resposta | Efeito |
|---|---|---|
| Mensagem nova gravada | 200, `situacao: nova` | inbox, conversa, mensagem e anexos gravados numa transação |
| Mesma mensagem de novo (reentrega) | 200, `situacao: duplicada` | devolve as mesmas ids; não grava nem baixa mídia |
| Estado de conexão (`open`, `close`, QR) | 200, `efeito: estado` | atualiza `connection_state` da integração |
| Estado `connecting` ou erro transitório | 200, `efeito: nenhum` | não muda nada |
| Evento sem efeito (ex.: `send.message`) | 200, `efeito: nenhum` | não grava |
| Estado de uma integração que não existe | 200, `efeito: integracao_inexistente` | não cria linha |
| Segredo ausente ou errado | 401, `webhook_nao_autorizado` | não lê o corpo nem grava |
| Instância ou org não configurada | 404 | — |
| Mensagem de integração que não existe ou está arquivada | 404 | não grava (sem agente dono não há conversa) |
| Corpo que não é um objeto JSON | 400 | — |
| Falha nossa ao gravar (banco ou blob) | 503, `indisponivel` | nada fica pela metade; a Evolution reenvia |

**Mensagens.** Texto e legenda viram o texto da mensagem. Imagem é baixada pela Evolution, gravada
no armazenamento de blobs (por hash, na pasta da org) e vira anexo. PDF e Office passam pelo mesmo
recebimento de arquivos da biblioteca (tipo real pelo conteúdo, leitura de texto com o extrator).
Mídia que falha (expirada, acima de 16 MB, tipo não aceito, arquivo ilegível) **não derruba a
mensagem**: ela entra com o texto que veio e um aviso no log. Áudio, vídeo, figurinha, contato e
localização não entram (decisão do mantenedor: arquivo = imagem + PDF/Office).

**Conversa.** Uma mensagem vai para a conversa aberta ou transferida do mesmo contato, nessa
integração. Se não houver, abre uma nova com o agente da integração e o departamento padrão dele.
Conversa transferida continua gravando, sem resposta do agente (D12).

**Idempotência.** A chave é a integração mais o id da mensagem no WhatsApp. Reentrega devolve o
mesmo item e não repete nada.

**Segurança.** O corpo do evento traz o `apikey` da instância. Ele não é gravado cru, não é logado e
não volta na resposta. Erros e logs saem só com ids internos e categorias: sem telefone, JID, chave
ou segredo. A comparação do segredo é em tempo constante.

**Limites desta etapa.**

- A conversa **não é respondida** ainda: o turno (chamada ao modelo e envio da resposta) é a HR-26c.
- Estado de integração arquivada não é checado no webhook de estado; a regra de "agente inativo para
  de responder" fica para o turno.
- Mensagem de integração **inativa** é gravada normalmente (sem perder o que o cliente mandou); o
  turno decide não responder.
- O horário gravado é o do recebimento, não o `messageTimestamp` do WhatsApp (que fica em
  `evento_json` da inbox).
- O texto extraído de PDF/Office fica gravado já envelopado como conteúdo não confiável. O turno
  deve usá-lo como está, sem envelopar de novo.
- Blob gravado antes da transação: se o banco falhar depois, sobra um arquivo sem linha de anexo
  (mesmo conteúdo, mesmo hash; é reaproveitado numa próxima entrega).
- PDF e Office pelo webhook **não foram testados ponta a ponta**: a leitura de texto usa o
  programa `poph-extrator`, que precisa estar ao lado do executável. A imagem foi testada.
- Não testado com número real: o cenário de conversa de verdade depende da conta de teste do
  WhatsApp. Coberto por testes com Evolution falsa e com os formatos de corpo capturados da
  Evolution 2.3.7 no ambiente isolado.

## Saída: da fila de saída até o WhatsApp (HR-26c)

Quem escreve a resposta é o turno (ainda não roda no `poph serve`). Ele grava a mensagem do agente
como `out` na conversa e enfileira um item `text` (ou `file`, para arquivo da biblioteca) na fila
de saída. O consumidor fica em `crates/poph-server/src/saida_whatsapp.rs`, roda como um laço de
1 s dentro do `poph serve` (só quando há instância da Evolution configurada) e segue estas regras:

- **Só pega o que é do WhatsApp configurado.** Item cuja conversa é de console, de outro canal ou
  de integração que não está em `[[evolution.instancias]]` fica como está. Não é marcado como falha.
- **Sem chave da Evolution** (`EVOLUTION_API_KEY` ausente no ambiente), o item fica `pending` e não
  gasta tentativa. A chave é lida na subida do servidor: depois de pôr a chave, é preciso reiniciar
  o `poph serve` para os itens saírem.
- **Texto acima de 4000 caracteres** sai cortado, terminando em `…`, numa mensagem só. Não é
  dividido (regra do POP Atende, `WHATSAPP_TEXT_LIMIT = 4000`).
- **Arquivo** sai como mídia, com o conteúdo lido do blob da biblioteca. Arquivo acima de 16 MB é
  recusado **antes** de ser lido e vira `failed` com o motivo; o texto do mesmo turno segue.
- **Não repete dentro da tentativa.** Envio de texto e de mídia fazem uma requisição por tentativa.
- **Falha transitória** (rede, 429, 5xx) volta para a fila com espera crescente (500 ms, dobrando,
  teto de 60 s). Depois de 8 tentativas, vira `failed` com "tentativas de envio esgotadas".
- **Falha definitiva** (4xx, mídia expirada, arquivo acima do teto) vai para `failed` na hora e
  não repete. O motivo gravado já vem sem telefone, JID nem chave.
- **Repetição entre tentativas aceita.** Se a Evolution receber e a resposta se perder, a próxima
  tentativa pode duplicar a mensagem no WhatsApp do cliente. É o risco aceito na fila (D11); a
  Evolution não tem chave de idempotência para evitar isso.
- **Reinício do servidor**: itens em `sending` voltam a ser tentados na subida (são pendentes).

Limites desta entrega:

- O id da mensagem que a Evolution devolve ao enviar não é gravado. O trait do canal devolve só
  sucesso ou erro. Por isso a linha `out` da conversa não recebe o id externo; ela é gravada no
  momento do turno, sem id externo.
- O consumidor não confere se a integração está arquivada no banco; só a configuração do servidor.
- O `poph chat` confirma só os itens das conversas do console (corrigido na HR-26f). Os de WhatsApp
  ficam para o laço de saída.
- A ordem entre mensagens da mesma conversa pode inverter quando uma delas está esperando nova
  tentativa.

## Turno no servidor (HR-26h)

A mensagem que entra pelo webhook vira turno, e a resposta sai pela fila de saída acima. O laço que
lê a fila de entrada e o que acontece quando o turno não responde (transferida, integração inativa,
sem chave do modelo) estão em [`TURNO.md`](TURNO.md), seção 11.

Um defeito da entrada (HR-26b) foi corrigido nesta entrega: a fila de entrada guardava o evento do
canal, sem a conversa e a mensagem que o turno lê. Agora o item guarda os dois ids, e o evento do
canal fica junto, em `canal`.

Configuração do intervalo: `[turno] intervalo_despacho_ms` (padrão 1000).

Quando o modelo falha no meio do turno, a mensagem do cliente é tentada de novo, até 3 vezes
(`[turno] tentativas_falha_modelo`), com espera de 30 s e depois 120 s (`atraso_falha_modelo_ms`). Se
todas falharem, a mensagem é concluída sem resposta e nada sai pelo WhatsApp. Detalhes em
[`TURNO.md`](TURNO.md), seção 11.

## O que ainda não faz

- **Teste com número real**: não feito. A conta de teste do WhatsApp segue pendente; o aceite
  usa servidor falso local e a Evolution isolada.
- **Número real e modelo real**: as duas pontas (entrada e saída) foram provadas com Evolution
  falsa e modelo falso. A prova com número de WhatsApp de verdade segue pendente.
- **Criar instância e parear pela API do Harness**: as operações existem no cliente, mas não há
  rota administrativa que as chame.

## Como rodar os testes

```
cd apps/pop-harness
cargo test -p poph-evolution --locked
cargo test -p poph-server --test saida_whatsapp --locked
cargo test -p poph-server --test webhook_evolution --locked
cargo test -p poph-server --test turno_whatsapp --locked
```

Os testes usam um servidor falso em `127.0.0.1` com roteiro de respostas. Nenhum deles fala com a
Evolution de verdade nem com número de WhatsApp.
