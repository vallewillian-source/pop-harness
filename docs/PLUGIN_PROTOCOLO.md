# Protocolo de plugin de integração (HR-25)

> Contrato: PRD §8.3. Código: `crates/poph-types/src/integracao.rs` (tipos) e
> `crates/poph-plugins/` (supervisor). Plugin de fixtura: `crates/poph-plugins/tests/fixtures/plugin_fixtura.rs`.
> Testes: `crates/poph-plugins/tests/protocolo.rs`.

Este é o contrato que um canal do POP Harness implementa. Vale para os dois formatos da V1:

- **nativo** — módulo Rust que implementa o trait `Integration` direto no processo (o WhatsApp da
  HR-26 é o caso);
- **processo externo** — um programa separado que fala o protocolo deste documento com o Harness.

O Console (HR-38) desenha o formulário de configuração de qualquer integração a partir do **JSON
Schema do manifesto**, sem código por canal.

## 1. Por que JSON-lines sobre stdio (e não socket Unix)

O plugin é filho do Harness: quem abre é o supervisor, que recebe `stdin`/`stdout` do processo.

- **ciclo de vida amarrado ao processo**: o fim do plugin vira `EOF` na hora, sem timeout de
  conexão nem socket órfão para limpar depois de uma queda;
- **isolamento real por processo**: um plugin que trava, cai ou imprime lixo não compartilha
  memória, descritor nem laço de eventos com o servidor nem com as outras integrações;
- **uma linha = uma mensagem**: dá para ler o tráfego com `cat` e testar o plugin com `echo`;
- **sem porta nem permissão de sistema**: nada para liberar em firewall, nada para autenticar na
  entrada do socket (o dono do processo é o Harness, que já entregou os segredos no handshake).

O socket Unix fica para o dia em que duas instâncias do Harness precisarem dividir o mesmo plugin —
o que a V1 não faz (roteamento é estático por org, D4). O contrato não muda: as mensagens são as
mesmas.

## 2. Versão do protocolo

`PROTOCOLO_VERSAO` (hoje **1**) vai no handshake dos dois lados. O supervisor recusa versão
diferente com `protocolo incompatível: o host fala 1 e o plugin falou N` — não existe negociação
nem "tenta o melhor esforço".

Campo **opcional novo** não muda a versão: o leitor ignora o que não conhece (`serde` sem
`deny_unknown_fields`). Mudou obrigatoriedade, tipo ou removeu mensagem? Sobe a versão.

## 3. Handshake

Primeira linha que o host escreve na entrada do plugin:

```json
{"tipo":"handshake","protocolo":1,
 "integracao":"01a1f6a5-cd9b-7000-8000-000000000000",
 "org":"01a1f6a5-cd9b-7000-8000-000000000001",
 "kind":"whatsapp",
 "config":{"instancia":"pop-atende"},
 "segredos":{"apikey":"…"}}
```

- `integracao` e `org` são UUID v7 (mesmos ids do banco).
- `config` é o que o operador preencheu no formulário (validado contra o schema do manifesto pela
  API/Console, não pelo supervisor).
- `segredos` é o **único** lugar por onde um segredo chega ao plugin. Nunca vai no ambiente nem na
  linha de comando: `argv` aparece em `ps`/`/proc`, e logs de terceiros copiam o ambiente.
- O plugin **não herda** o ambiente do Harness. O processo recebe só `PATH`, `HOME`, `LANG`,
  `LC_ALL`, `TZ`, `TMPDIR` e o que for declarado em `OpcoesPlugin::com_ambiente` — as chaves do
  LLM, do Jev e da cifra ficam fora do alcance de um programa de terceiro.

A primeira linha do plugin de volta é a aceitação, com o manifesto:

```json
{"tipo":"handshake_ok","protocolo":1,
 "manifesto":{
   "nome":"WhatsApp","kind":"whatsapp","versao":"1.0.0","protocolo":1,
   "capacidades":{"text":true,"image_in":true,"file_in":true,"file_out":true,
                  "typing":true,"read_receipt":false},
   "limites":{"max_anexo_bytes":16777216,"max_texto_bytes":65536,
              "tipos_aceitos":["image/png","application/pdf"]},
   "configuracao":{"type":"object","properties":{"instancia":{"type":"string"}}},
   "segredos":{"type":"object","properties":{"apikey":{"type":"string"}}}
 }}
```

O manifesto precisa de `nome`, `kind`, `versao`, `protocolo` e `configuracao`; `capacidades` e
`limites` têm padrão. `configuracao` e `segredos` são **JSON Schema de objeto** (`type: "object"`) —
é o mínimo que o supervisor confere; o resto do schema é do Console.

Nada mais é aceito antes do `handshake_ok`: mensagem fora disso é contada como lixo.

## 4. Mensagens

Uma mensagem JSON por linha, nos dois sentidos. `\n` termina; `\r\n` também é aceito; linha vazia é
ignorada.

### Host → plugin

| `tipo`      | Campos                          | Resposta do plugin                       |
|-------------|---------------------------------|------------------------------------------|
| `handshake` | `protocolo`, `integracao`, `org`, `kind`, `config`, `segredos` | `handshake_ok` |
| `comando`   | `id`, `comando`                 | `resultado` com o mesmo `id`             |
| `saude`     | `id`                            | `saude` com o mesmo `id`                 |
| `encerrar`  | —                               | nenhuma: o processo deve sair            |

`comando` é um dos três (`tipo` dentro de `comando`):

```json
{"tipo":"comando","id":7,"comando":{"tipo":"enviar_texto","idempotencia":"turno:0",
   "peer":"5511999999999","texto":"olá"}}

{"tipo":"comando","id":8,"comando":{"tipo":"enviar_arquivo","idempotencia":"turno:1",
   "peer":"5511999999999","nome":"manual.pdf","mime":"application/pdf",
   "tamanho_bytes":1234,"sha256":"…","blob_ref":"<org>/<sha256>","legenda":"segue o manual"}}

{"tipo":"comando","id":9,"comando":{"tipo":"definir_presenca","peer":"5511999999999",
   "estado":"digitando"}}      // digitando | pausado
```

`idempotencia` vem do `(turn_id, seq)` da `outbox` (D11): se o plugin souber deduplicar do lado do
canal, use essa chave. O mesmo arquivo pode ser reenviado depois de uma queda — a entrega é "pelo
menos uma vez".

### Plugin → host

| `tipo`         | Campos                                  | Quando |
|----------------|-----------------------------------------|--------|
| `handshake_ok` | `protocolo`, `manifesto`                | resposta ao handshake |
| `evento`       | `evento` (`InboundEvent`)               | chegou mensagem do cliente |
| `estado`       | `estado` (`conectado`/`desconectado`/`qr`/`erro`) | mudou o estado do canal |
| `resultado`    | `id`, `ok`, `erro` (quando `ok=false`)  | resposta a um `comando` |
| `saude`        | `id`, `saude` (`ok`, `detalhe`)         | resposta a `saude` |
| `erro`         | `codigo`, `mensagem`, `transitorio`     | problema não solicitado (vira evento de estado `erro`) |

Evento de entrada normalizado:

```json
{"tipo":"evento","evento":{
  "integracao":"01a1f6a5-…","id_externo":"wamid.ABC","peer":"5511999999999",
  "remetente":null,"texto":"quero um orçamento","anexos":[],"ocorrido_em":1795140000000,
  "metadados":{"reply_to":"wamid.XYZ"}}}
```

`id_externo` é a chave de idempotência da entrega (a `inbox` usa
`(integration_id, external_message_id)`); `peer` é o contato/sala; `ocorrido_em` é em ms.

Anexo, nas duas formas do PRD:

```json
{"fonte":"blob","nome":"manual.pdf","mime":"application/pdf",
 "sha256":"<64 hexa>","tamanho_bytes":1234}
{"fonte":"bytes","nome":"foto.jpg","mime":"image/jpeg","dados_base64":"…"}
```

O supervisor recusa evento sem `id_externo`/`peer`/`ocorrido_em`, com sha256 que não tem 64 hexa,
com anexo vazio — e conta em `eventos_invalidos`, sem entregar ao motor.

## 5. Limites, prazos e política de reinício

| Item | Padrão | O que acontece quando estoura |
|---|---|---|
| Tamanho de mensagem (dois sentidos) | 1 MiB | linha do plugin é descartada e contada em `lixo`; mensagem do host é recusada com `mensagem de N bytes passa do limite de M` (o plugin **não** é derrubado) |
| Handshake | 5 s | processo morto, falha contada, reinício agendado |
| Comando | 10 s | processo morto, falha contada, reinício agendado; quem chamou recebe "tempo esgotado" |
| Saúde | 3 s | idem |
| Falhas seguidas | 5 | a integração vira `Caida`: não reinicia mais sozinho |
| Espera entre reinícios | 500 ms × 2 até 60 s | — |
| Encerramento | 500 ms | depois disso o processo é morto (`SIGKILL`) |

Um comando que estoura o prazo derruba o processo: os outros comandos que estiverem em voo na
mesma integração falham como "indisponível" (transitório) e são retentados pela `outbox`. É o preço
de não deixar um processo travado segurando a fila.

O placar de falhas **não** zera quando o plugin sobe: subir não prova que ele está saudável (um
plugin que cai a cada comando sobe toda vez). Ele zera quando o plugin **responde** um comando ou
uma saúde. Assim um plugin que trava em toda entrega para de ser reiniciado depois do teto, em vez
de derrubar mensagens para sempre.

A fila de leitura é limitada (256 linhas): um plugin que imprime em laço enche o cano do sistema
operacional e fica bloqueado na escrita, em vez de encher a memória do Harness.

`Caida` é estado final para o supervisor: quem quiser tentar de novo cria um `Supervisor` novo (o
Console/CLI pode expor um "reconectar"), ou reinicia o processo do Harness.

## 6. Contadores (para `/metrics` e para o "por quê" do Console)

`linhas`, `lixo` (linha fora do protocolo ou acima do teto), `eventos`, `eventos_invalidos`,
`estados`, `comandos`, `saudes`, `handshakes`, `falhas` (seguidas), `reinicios`, `tempo_esgotado`,
`abortos`. São por integração — nunca somados entre orgs.

## 7. Exemplo mínimo de plugin

Programa que só faz o handshake e devolve as respostas obrigatórias (Rust, sem dependência além de
`serde_json`):

```rust
use std::io::{BufRead, Write};
use serde_json::json;

fn main() {
    let stdin = std::io::stdin();
    let mut linhas = stdin.lock().lines();
    // 1. handshake
    let _handshake = linhas.next().expect("handshake").expect("linha");
    println!("{}", json!({
        "tipo":"handshake_ok","protocolo":1,
        "manifesto":{"nome":"Eco","kind":"eco","versao":"1.0.0","protocolo":1,
                     "capacidades":{"text":true},
                     "configuracao":{"type":"object","properties":{}}}
    }));
    std::io::stdout().flush().ok();
    // 2. laço de pedidos
    for linha in linhas {
        let Ok(pedido) = serde_json::from_str::<serde_json::Value>(&linha.unwrap()) else { continue };
        match pedido["tipo"].as_str() {
            Some("comando") => println!("{}", json!({"tipo":"resultado","id":pedido["id"],"ok":true})),
            Some("saude") => println!("{}", json!({"tipo":"saude","id":pedido["id"],"saude":{"ok":true}})),
            Some("encerrar") => break,
            _ => {}
        }
        std::io::stdout().flush().ok();
    }
}
```

O plugin de verdade para testar o contrato é `tests/fixtures/plugin_fixtura.rs` (modos `normal`,
`trava_no_handshake`, `trava_no_comando`, `trava_no_primeiro_comando`, `cai_depois_do_handshake`,
`cai_no_primeiro_comando`, `cai_em_todo_comando`, `lixo`, `protocolo_errado`,
`manifesto_invalido`). Ele existe só para os testes: o pacote de distribuição (`scripts/build-dist.sh`)
não o inclui.

## 8. Como o Harness usa o contrato

```rust
use poph_plugins::{OpcoesPlugin, Supervisor};
use poph_types::Integration;              // o trait, igual para nativo e plugin

let opcoes = OpcoesPlugin::novo("/opt/poph/plugins/whatsapp", integracao_id, org_id)
    .com_kind("whatsapp")
    .com_config(config_do_operador)
    .com_segredo("apikey", segredo);      // só vai no handshake

let canal = Supervisor::iniciar(opcoes, receptor).await?;   // handshake já feito aqui
canal.enviar_texto(comando).await?;                          // por `Integration`
let saude = canal.saude().await?;
canal.encerrar().await;
```

O `receptor` implementa `poph_types::Receptor`: recebe `EventoIntegracao::Entrada(InboundEvent)` e
`EventoIntegracao::Estado(EventoEstado)`. A ligação pronta entre o contrato e as filas está em
`poph-agent/src/fila_integracao.rs` (HR-27): `EnvioPeloCanal` (item da `outbox` → `ComandoSaida`) e
`InboxDoCanal` (`InboundEvent` → `inbox`). O exemplo completo é `docs/PLUGIN_EXEMPLO.md`.

## 8.1 O manifesto no servidor (HR-44c)

O servidor não guarda manifesto: ele pergunta ao plugin. `[plugins].integracoes` declara
`kind` + `programa` (+ `argumentos`); na primeira leitura do manifesto (rota
`GET /admin/v1/orgs/{org_id}/manifests/{kind}` ou primeira configuração de integração daquele tipo)
o servidor sobe o processo pelo supervisor, faz o handshake, lê o `manifesto`, encerra o processo e
guarda o resultado em memória. Falha fica em cache por 30 s para não virar laço de processos.

Quem valida a configuração do operador contra os esquemas é a API (`docs/API.md`), com o subconjunto
de JSON Schema descrito lá: `type`, `required`, `enum`, `minLength`/`maxLength`,
`minimum`/`maximum` e `properties` aninhados.

## 9. O que ainda não existe

- **Plugin externo como canal no `poph serve`**: o servidor sobe o plugin só para ler o manifesto
  (`[plugins].integracoes`, `poph-server/src/manifestos.rs`). Receber e enviar mensagens de cliente por
  plugin externo dentro do servidor não está ligado; a ligação entre o contrato e as filas existe
  (`poph-agent/src/fila_integracao.rs`, `docs/PLUGIN_EXEMPLO.md`) e é exercitada em teste.
- **Re-cifragem dos segredos** (`integrations.secrets_enc`) quando `POPH_CHAVE_CIFRA` troca.
- **Canal observador da conversa** (Matrix): ver `docs/CONTRATO_CANAL.md` §7.

O canal WhatsApp nativo (HR-26) e o formulário do Console gerado a partir do manifesto (HR-44) existem.
