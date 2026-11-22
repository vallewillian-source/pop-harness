# Biblioteca de arquivos do agente e `enviar_arquivo` (HR-24)

O agente só envia ao cliente um arquivo que esteja na **biblioteca dele** (decisão D13). O modelo
nunca escolhe caminho de disco nem conteúdo: escolhe um **id** de uma lista que ele vê no próprio
pedido. Quem grava o arquivo é o crate `poph-files` (HR-23); quem decide o que sai é o motor do
turno; quem entrega ao canal é a `outbox` (HR-12).

## Modelo

Tabela `file_library` (já existia no esquema da HR-07, sem migração nova):

| coluna | conteúdo |
|---|---|
| `id` | UUID v7 do arquivo |
| `org_id` | org dona |
| `agent_id` | agente dono (o arquivo só existe para ele) |
| `name` | nome mostrado ao cliente (até 200 caracteres) |
| `description` | texto que o modelo lê para decidir enviar (até 500) |
| `mime` | tipo **real**, pelo conteúdo |
| `size_bytes` | tamanho em bytes |
| `sha256` | hash do conteúdo (o blob é deduplicado por ele) |
| `blob_path` | caminho do blob **relativo** à pasta de blobs (`<org>/<sha256>`) |
| `created_at` | criação (ms) |

O conteúdo não fica no SQLite. O blob fica em `<dados.blobs>/<org>/<sha256>`, gravado pelo
`poph-files`, e é compartilhado por qualquer registro com o mesmo hash dentro da org.

Não há versão/`If-Match`: a tabela não é editável. Para trocar o conteúdo de um arquivo, exclua e
suba de novo (o blob não é duplicado se o conteúdo for o mesmo).

## Upload pela CLI

```
poph file upload <caminho> --agent <id|slug> [--name "Nome mostrado"] [--description "…"] [--json] \
  --org <slug|id> --offline
poph file list   --agent <id|slug> [--limit 50] [--json] --org … --offline
poph file show   <id> --agent <id|slug> [--json] --org … --offline
poph file delete <id> --agent <id|slug> [--json] --org … --offline
```

Regras da CLI:

- `--offline`, `--org` e `--agent` são obrigatórios (como `org`, `token`, `kb`).
- O upload passa pelo `poph-files`: limite de 16 MB, tipo real pelo conteúdo, extensão enganosa
  recusada, blob por hash. Arquivo recusado não deixa nada gravado.
- O arquivo é lido com teto (`limite + 1` bytes): um arquivo de vários GB nunca entra inteiro na
  memória para depois ser recusado.
- Erro de negócio: código 1 e mensagem em português no erro padrão. `--json` sai com
  `versao_relatorio: 1`.
- `delete` tira o registro; o blob fica no disco até a retenção (HR-51) apagar o que não tem dono.

PDF e Office exigem o `poph-extrator` ao lado do executável (ver `docs/ARQUIVOS.md`). Sem ele, o
upload de documento falha com mensagem clara; imagem continua funcionando.

## Ferramenta `enviar_arquivo`

- Nome: `enviar_arquivo`. Argumentos: **só** `id` (`additionalProperties: false`). Qualquer outra
  chave (caminho, nome, conteúdo) é recusada com erro legível.
- O id é conferido contra a biblioteca **do mesmo agente e da mesma org**. Id de outro agente, de
  outra org ou inexistente devolve `erro: não existe arquivo com esse id na sua biblioteca…` ao
  modelo — o turno continua e o modelo pode seguir sem o arquivo.
- Habilitação por agente (D-04): sem `enviar_arquivo` na lista de ferramentas do agente, ela não vai
  no pedido ao modelo.
- A **descrição** da ferramenta é completada, a cada turno, com a lista da biblioteca do agente:

  ```
  - <id> — <nome> (<mime>, <tamanho>): <descrição>
  ```

  É assim que o modelo decide o que enviar, sem precisar de outra chamada. A lista traz até 60
  arquivos (`LIMITE_LISTAGEM`); acima disso aparece um aviso de quantos ficaram de fora. Biblioteca
  vazia aparece como vazia.
- O resultado da ferramenta volta como **dado** (nome e mime sem `<` nem `>`), não como instrução.

## O que sai na `outbox`

Item `kind = 'file'` com `payload_json`:

```json
{
  "blob_ref": "<org>/<sha256>",
  "arquivo_id": "<uuid v7>",
  "nome": "catalogo.pdf",
  "mime": "application/pdf",
  "tamanho_bytes": 4096,
  "sha256": "<64 hex>"
}
```

- A resposta em texto do turno usa `seq = 0`; o arquivo usa `seq` derivada do id (faixa 1 a
  1.000.000.000), estável por arquivo.
- **Idempotência:** repetir a mesma chamada no mesmo turno cai no mesmo `(turn_id, seq)` e não
  duplica; a ferramenta responde "já estava enfileirado neste turno". Dois arquivos diferentes no
  mesmo turno ocupam itens separados.
- O estado segue `pending → sending → sent | failed` pela HR-12 (`D11`: pelo menos uma entrega).

## O que falta para o envio real

- **Transporte do canal** (HR-25/HR-26): hoje o item fica na `outbox`; o WhatsApp e o plugin entram
  com o mesmo contrato.
- **Console (HR-46)**: mostrar o arquivo enviado como anexo na conversa. Hoje o item vive na
  `outbox`; o transporte de console confirma o item sem imprimir nada (só texto é impresso).
- **Teto de arquivos por turno**: não há limite além do `max_rodadas_ferramenta` do agente — cada
  rodada pode enfileirar um ou mais arquivos.

## Testes

- `crates/poph-store/tests/biblioteca.rs` — repositório: criar/listar/contar/excluir, isolamento por
  org e por agente da mesma org, agente de outra org recusado, validação de campos.
- `crates/poph-agent/tests/biblioteca_arquivos.rs` — ferramenta e turno: id de outro agente/org
  recusado (inclusive com org forjada no contexto), id inexistente com erro legível e turno
  seguindo, `SendFile` na `outbox` com `blob_ref` e sem duplicar na repetição, agente sem a
  ferramenta não a recebe, agente com ela vê a descrição dos arquivos, biblioteca vazia.
- `crates/poph/tests/cli_arquivo.rs` — CLI de ponta a ponta: upload/lista/mostra/exclui, dedupe de
  blob por hash, arquivo acima de 16 MB recusado, extensão enganosa recusada, erros com código 1,
  `--offline` obrigatório, biblioteca por agente.
