# Plugin de exemplo (eco) e a ligação com as filas (HR-27)

> Contrato: `docs/PLUGIN_PROTOCOLO.md` (HR-25). Código: `crates/poph-plugin-eco/`
> (o plugin) e `crates/poph-agent/src/fila_integracao.rs` (a ligação com `inbox`/`outbox`).

Este documento responde três perguntas:

1. **como um plugin de verdade conversa com o Harness** — com um exemplo que roda;
2. **como a fila de saída e a fila de entrada se ligam ao canal** — o que faltava depois da HR-25;
3. **o que ainda não existe** — para não confundir o exemplo com o WhatsApp da HR-26.

## 1. O plugin de exemplo

`crates/poph-plugin-eco/` gera o binário **`plugin-eco`**, um programa pequeno que implementa o
protocolo JSON-lines sobre stdio:

| Recebe | Faz | Responde |
|---|---|---|
| `handshake` | confere a versão do protocolo | `handshake_ok` com o manifesto (`kind: "eco"`, `text: true`, `file_out: true`, `typing: true`), com o esquema de configuração (`modo`, `remetente` obrigatório) e um segredo (`token`, obrigatório) só para o exemplo mostrar o formulário gerado (HR-44c) |
| `comando` / `enviar_texto` | inverte o texto recebido | `resultado` ok **e** um `evento` de entrada com o texto invertido |
| `comando` / `enviar_arquivo` | nada (um plugin real subiria o arquivo) | `resultado` ok |
| `comando` / `definir_presenca` | anota no stderr | `resultado` ok |
| `saude` | conta as mensagens tratadas | `saude` ok com o detalhe |
| `encerrar` | sai com código 0 | — |

Duas decisões que valem para qualquer plugin novo:

- **`id_externo` estável.** O evento que o plugin devolve usa `eco-<turno:seq>` — a chave de
  idempotência do comando. Se o Harness reenviar a mesma mensagem depois de uma queda, o evento
  chega com o mesmo id e a `inbox` descarta a repetição (a entrega é "pelo menos uma vez").
- **linha fora do protocolo é lixo.** JSON inválido ou mensagem desconhecida vão para o stderr e o
  laço continua; o plugin nunca morre por causa de uma linha estranha.

## 2. Como rodar

```bash
apps/pop-harness/scripts/plugin-eco.sh
```

O script compila o plugin e a demonstração, roda tudo num banco temporário e apaga a pasta no fim.
Saída esperada (resumida):

```text
2. supervisor pronto: Eco (exemplo) v0.1.0 (kind=eco, protocolo=1)
3. item na fila de saída: `olá, mundo` (estado Pendente)
4. envio pelo plugin: enviados=1 falharam=0 → estado Enviado
5. inbox → mensagem (in/user): "odnum ,álo"
6. reentrega do mesmo comando: gravados=1 duplicados=1 (nada duplicado na conversa)
9. fila de saída no fim: pendentes=0 enviados=1
==> processos do plugin ainda vivos: 0
```

O que a demonstração faz, passo a passo:

1. abre um banco (`harness.db`) numa pasta temporária, com org, agente e a conversa do contato;
2. sobe o `plugin-eco` **como processo filho** pelo `Supervisor` e faz o handshake;
3. enfileira um item `text` na `outbox` (`{"texto": "olá, mundo"}`), como o turno faria;
4. entrega o item pelo `EnvioPeloCanal` — o mesmo transporte que o WhatsApp vai usar;
5. espera o evento de volta e mostra a mensagem gravada na conversa (`"odnum ,álo"`);
6. reenvia o mesmo comando e mostra que a `inbox` conta `duplicado` em vez de gravar de novo.

## 3. A ligação com as filas (`poph-agent/src/fila_integracao.rs`)

Ela mora no `poph-agent`, e não no `poph-plugins`, por dois motivos: quem conhece a `outbox`/`inbox`
e o `TransporteSaida` é o motor (`poph-agent`), e o `poph-plugins` continua sendo código puro (só o
protocolo e o supervisor), o que o mantém no check ARM da estação sem arrastar SQLite.

### Sai­da: `outbox` → canal

```rust
let transporte = EnvioPeloCanal::novo(store, canal);   // canal: Arc<dyn Integration>
fluxo.enviar_pendentes(org, &transporte).await?;        // pending/sending → sent | failed
```

`EnvioPeloCanal` implementa `TransporteSaida` (HR-12) e traduz o item da `outbox` no comando do
protocolo:

| Item da `outbox` | Vira | Campos usados do payload |
|---|---|---|
| `text` | `enviar_texto` | `texto` |
| `file` | `enviar_arquivo` | `blob_ref`, `nome`, `mime`, `tamanho_bytes`, `sha256`, `legenda` |
| `presence` | `definir_presenca` | `estado` (`digitando`/`pausado`) |

O destino (`peer`) vem do `external_peer` da conversa do item — o canal não aparece no payload. Erro
**transitório** do canal (`indisponível`, `tempo esgotado`, recusa marcada como transitória) faz o
item voltar para a fila como `sending` e ser retentado; erro definitivo (`conversa sem peer`,
payload inválido) marca `failed` com o motivo legível. O item só vira `sent` depois que o canal
confirma — é o que impede mensagem perdida em queda no meio do envio.

### Entrada: canal → `inbox`

```rust
let resolvedor = Arc::new(ConversaDoCanal::novo(store, agente, "whatsapp", Some(integracao)));
let entrada = InboxDoCanal::iniciar(fluxo, org, resolvedor);  // implementa Receptor
```

`InboxDoCanal` implementa `Receptor` (HR-25): o `Receptor::receber` só **enfileira** (o laço de
leitura do canal não pode esperar o banco) e uma tarefa própria grava pelo caminho da HR-12 —
`Fluxo::receber_mensagem`, que grava a mensagem e o item da `inbox` na **mesma transação**, com
idempotência por `(integração, id externo)`. Reentrega vira `duplicado`, não outra mensagem.

`ConversaDoCanal` decide a conversa: procura uma conversa da org com o mesmo canal e o mesmo
`external_peer` (prefere a aberta); se não houver, abre uma nova com o agente configurado. Sem
isso, um evento de contato novo não teria onde cair.

Contadores (`InboxDoCanal::contadores`) — por integração, nunca somados entre orgs:

| Contador | Significado |
|---|---|
| `gravados` | evento novo virou mensagem |
| `duplicados` | a `inbox` recusou por id externo repetido |
| `recusados` | a fila interna (1024) estava cheia — contrapressão |
| `falhas` | não deu para resolver a conversa ou gravar |
| `estados` | avisos de estado do canal (conectado, QR, erro) |
| `com_anexo` | evento trouxe anexo (ver limite 3 abaixo) |

## 4. Como escrever um plugin novo a partir deste

1. Copie a estrutura de `crates/poph-plugin-eco/src/main.rs`: laço de linhas, `MensagemHost` na
   entrada, `MensagemPlugin` na saída.
2. Responda o `handshake` com um manifesto verdadeiro (capacidades e esquema de configuração) e
   recuse versão de protocolo diferente.
3. Confirme **todo** comando com `resultado` e o mesmo `id` — é o que faz o item da `outbox` virar
   `sent`.
4. Entregue o que chega do cliente como `evento`, com `id_externo` **estável** (o id do canal).
   Reentrega é normal: quem deduplica é a `inbox`.
5. Nunca escreva segredo em `stderr`, em arquivo ou no evento: o segredo chega só no handshake.
6. Se o plugin cair, ele será reiniciado com backoff; não guarde estado que não sobreviva a isso.
7. Registre-o no Harness com `OpcoesPlugin::novo(caminho, integracao_id, org)` — programa,
   argumentos e ambiente declarados; segredo só por `com_segredo`.

Um plugin em outra linguagem segue o mesmo JSON de `docs/PLUGIN_PROTOCOLO.md`; o exemplo em Rust usa
os tipos de `poph-types` de propósito, para que uma mudança de contrato quebre a compilação em vez
de quebrar em produção.

## 5. O que a suíte prova

`crates/poph-plugin-eco/tests/conversa.rs` (5 testes) e `tests/queda.rs` (1 teste), com o
`plugin-eco` como processo filho de verdade:

- **conversa completa:** item na `outbox` → supervisor → plugin → evento → `inbox` → mensagem na
  conversa, com o texto invertido conferido;
- **idempotência:** reenviar o mesmo comando não cria segunda mensagem (`duplicados = 1`);
- **arquivo:** o item `file` da HR-24 vira `enviar_arquivo` e o item vira `sent`;
- **conversa sem contato:** falha definitiva com motivo legível, o canal nem é chamado;
- **dois contatos:** cada `peer` abre a sua conversa, sem misturar;
- **payload inválido:** falha definitiva, sem chamar o plugin;
- **`kill -9` no plugin:** o supervisor não cai, reinicia o plugin (`reinicios ≥ 1`), o item volta a
  ser enviado, nenhuma mensagem se perde e nenhuma duplica.

## 6. O que ainda não existe (limites conhecidos)

1. **Não é o WhatsApp.** A HR-26 traz o módulo nativo e a resposta ao webhook. Aqui o caminho é o
   das filas, provado com o plugin — o mesmo `EnvioPeloCanal` e o mesmo `InboxDoCanal` servem.
2. **O servidor ainda não sobe integrações.** `poph serve` não carrega plugins; quem liga hoje é o
   processo que monta o `Fluxo` (a demonstração e os testes). Ligar no servidor é a integração da
   HR-26/HR-38.
3. **Anexo recebido não vira linha em `attachments`.** Enquanto a HR-26 não registrar o blob e a
   extração, o anexo fica **descrito** no campo de anexos da mensagem, como o canal entregou —
   nada se perde, mas o turno ainda não lê arquivo do cliente por esse caminho.
4. **Busca de conversa por contato não é indexada.** O resolvedor lembra no cache a conversa de cada
   contato (conferida no banco a cada uso, só vale conversa aberta), então o caso normal custa uma
   leitura por id. Quando o contato não está no cache — primeiro contato, conversa transferida ou
   cache cheio (teto de 4.096 contatos) —, a busca varre até 50 páginas (10 mil conversas), porque o
   repositório de conversas ainda não tem busca por `peer`. Aparece em org com histórico muito
   grande; o certo é a HR-26 expor essa busca no repositório.
5. **Fila de entrada limitada a 1024 eventos**: com o banco travado e o canal entregando sem parar,
   o excedente é recusado e contado (`recusados`) — o evento não é gravado.
6. **O plugin de exemplo não entra no pacote de distribuição**: o `poph` não depende de
   `poph-plugin-eco`, e o `scripts/build-dist.sh` só empacota `poph` e `poph-extrator`.
