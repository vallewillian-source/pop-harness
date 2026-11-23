# POP Harness: backup da org em arquivo único (HR-53)

Exporta uma org inteira para **um arquivo** e reconstrói essa org a partir dele em outra instância.
É a base do "exportar/importar org" do Console (HR-48): a tela vai chamar a mesma lógica.

## Comandos

```
poph --offline backup export <org> --out <arquivo> [--com-conversas] [--sobrescrever] [--json]
poph --offline backup import <arquivo> [--org <destino>] [--sobrescrever] [--json]
```

- `<org>` é id (UUID v7) ou slug. Os dois comandos rodam **direto no banco** e exigem `--offline`
  (como `org`, `file` e `kb`): rode com o servidor parado.
- `--com-conversas` inclui conversas, mensagens, anexos, turnos, as duas filas (`inbox`/`outbox`) e
  os carrinhos. Sem o flag, o arquivo leva só a configuração, a base, o catálogo e a biblioteca.
- `export` **não sobrescreve** arquivo existente sem `--sobrescrever`. O arquivo é escrito num
  temporário ao lado e renomeado no fim: erro no meio não deixa backup pela metade.
- `import --org <destino>` grava dentro de uma org **já existente** (id ou slug), mantendo o nome e
  a configuração dela. Sem `--org`, vale a org de origem do arquivo — criada se não existir.
- `import` **recusa por padrão** quando atualizaria linha que já existe no destino: diz quantas
  linhas, em quais tabelas e até cinco exemplos (`tabela: slug`), não grava nada e sai com código
  `1`. `--sobrescrever` autoriza (o `--sobrescrever` do `export` é outro: lá é o arquivo de saída).
  Destino limpo entra sem o flag.
- Saída: `0` quando terminou (inclusive com o aviso de índice pendente); `1` em erro; `2` é uso
  incorreto da linha de comando. `--json` tem `versao_relatorio: 1`.

## Formato do arquivo

JSON, com a extensão que o operador quiser (sugestão: `.json`). Estrutura:

| Campo | O que é |
|---|---|
| `formato` | `"poph-backup-org"` — confere na importação |
| `versao_formato` | `2` — a versão 1 continua importável; versão desconhecida é recusada com mensagem clara |
| `exportado_em` | instante da exportação (ms desde a época Unix) |
| `org_origem` | `{id, slug, nome}` da org de onde o arquivo saiu |
| `com_conversas` | se o histórico entrou |
| `org` | a linha completa da tabela `orgs` |
| `tabelas` | tabela → lista de linhas (objetos coluna → valor) |
| `blobs` | lista de `{sha256, conteudo_base64}` — um por conteúdo, deduplicado por sha256 |

### O que entra

| Tabela | Sempre |
|---|---|
| `product_categories`, `departments`, `products` | sim |
| `agents`, `agent_instructions`, `agent_product_categories` | sim |
| `knowledge_groups`, `knowledge_items`, `agent_knowledge_groups` | sim |
| `file_library` (+ bytes do blob) | sim |
| `integrations` (sem o segredo) | sim |
| `fluxos`, `fluxo_revisoes` (fonte e IR), `agente_fluxos` | sim |
| `conversations`, `messages`, `attachments` (+ bytes dos blobs), `turns`, `inbox`, `outbox`, `carts`, `cart_items` | só com `--com-conversas` |
| `fichas`, `ficha_eventos`, `fluxo_execucoes`, `fluxo_execucao_etapas`, `fluxo_efeitos` | só com `--com-conversas` |

Revisões publicadas preservam seus números e são imutáveis na importação, mesmo quando a sobrescrita é autorizada.
Backups da versão 1 com conversas são importados com uma Ficha criada para cada conversa.

**Nome do cliente (`conversations.client_name`, MX-32).** A coluna viaja com `conversations` e entra no
mesmo `--com-conversas`, como `external_peer`: é dado pessoal do cliente, e o tratamento é o mesmo.
Backup feito antes da migração `0008` não tem a coluna; a importação aceita esse arquivo e deixa o
nome nulo nas conversas.

**Id do envio (`outbox.external_message_id`, MX-40).** Viaja com `outbox` no `--com-conversas`, como
o resto da fila. Não é dado de cliente: é o id que o canal deu à mensagem enviada. Backup feito antes
da migração `0011` não tem a coluna; a importação aceita o arquivo e deixa o id nulo, sem efeito no
resto da fila (o item continua `sent`).

### O que **não** entra

- **`admin_tokens`**: credencial. O token se cria de novo com `poph token`.
- **`audit_log`**: histórico de infraestrutura, não faz parte do estado da org.
- **`integrations.secrets_enc`**: o segredo cifrado. Nem em claro, nem cifrado (ver abaixo).
- **`blob_path`**: caminho interno (`<org>/<sha256>`). O arquivo guarda `sha256` e os bytes; o
  caminho é recalculado para a org de destino na importação.
- **Tabelas do módulo Matrix (`matrix_orgs`, `matrix_rooms`, `matrix_spaces`, `matrix_estado`,
  `matrix_falhas`, `matrix_avisos_falha`)**: **não** viajam no backup. A sala pertence ao Synapse do ambiente, e um
  `room_id` copiado para outra instalação apontaria para uma sala que não existe lá (ou, pior, para
  a de outro ambiente). Na instância de destino o módulo começa desligado e, quando a org o ligar,
  cria as salas de novo a partir das conversas que existirem. O mesmo vale para a importação: um
  arquivo nunca traz essas tabelas, e o esquema delas não é tocado.

## Segredo

Do segredo de integração o arquivo leva só:

- `tem_segredo: true` — havia segredo gravado;
- `segredos_campos: ["token"]` — os **nomes** dos campos preenchidos, quando a chave de cifra do
  processo (`POPH_CHAVE_CIFRA`) consegue abrir o blob. Sem a chave (ou com uma chave diferente da
  que gravou), vem `null`: só a marca sobra.

O valor nunca é escrito, em nenhuma forma. Ao importar, a integração entra com os segredos **vazios**
e o relatório diz quantas precisam ser digitadas de novo (aba Integrações do Console). Reimportar o
mesmo arquivo **não apaga** o segredo que já foi digitado depois da primeira importação.

Na exportação, prove que o valor não saiu com um `grep` pelo segredo real no arquivo (o teste de CI
faz isso com uma org que tem segredo cifrado gravado).

## Importação

- **Atômica**: todas as linhas vão numa transação do escritor. Erro em qualquer ponto desfaz tudo —
  não fica estado pela metade. Os blobs são gravados antes (são content-addressed: regravar o mesmo
  hash não muda nada); se o banco recusar, eles ficam órfãos e a retenção apaga depois.
- **Localiza pela chave da tabela**: cada linha é procurada pelo `id` (ou pelas colunas da chave
  primária composta). Linha que não existe entra (`inseridas`); linha que já existe **atualiza**
  (`atualizadas`) com o valor do arquivo — e é por isso que a atualização precisa de autorização
  explícita (abaixo). Importar o mesmo arquivo duas vezes, com `--sobrescrever`, deixa o mesmo
  estado, sem duplicar.
- **Atualização exige autorização (HR-48e-3)**: sem `--sobrescrever`, a importação confere antes de
  gravar (só leitura) e, se alguma linha do arquivo já existe no destino, **recusa**: mostra total,
  tabelas e até cinco exemplos e não grava nada. Reimportar um arquivo antigo na mesma org
  reverteria o que mudou depois dele (versão do agente, instruções, Jev) — é o caso que a recusa
  protege. Numa org de destino (`--org`), a linha da org não entra na conta: ela não é tocada.
- **Não apaga**: o que existe no destino e não está no arquivo fica como está. Para reconstruir
  exatamente, importe numa instância limpa.
- **Ids preservados**: agentes, produtos, conversas, mensagens e turnos mantêm os ids, versões e
  carimbos. É o que mantém conversa → agente, mensagem → turno e anexo → mensagem ligados.
- **Conflito de chave natural**: se o destino já tem um agente/produto com o mesmo **slug** (ou
  categoria/departamento/grupo com o mesmo nome, ou a org com o mesmo slug), com **id diferente**,
  a importação é recusada **antes de gravar**, com a lista do que conflitou. Não há "merge"
  silencioso. Importar o mesmo conteúdo (mesmos ids) é atualização, não conflito.
- **Coluna desconhecida**: se o arquivo traz uma coluna que esta versão do esquema não tem, a
  importação é recusada com o nome da tabela e da coluna (arquivo de versão mais nova).
- **Arquivo antigo**: um arquivo gerado antes da coluna `origin` (migração `0003`) não traz `origin`
  nem `author_name`, e continua importável. As mensagens entram como `agent`, sem autor.

### Blobs

O conteúdo de cada arquivo da biblioteca e de cada anexo vai no arquivo em base64, com o sha256.
Na importação o sha256 é conferido antes de gravar (arquivo alterado ou truncado é recusado), o blob
é gravado por `poph-files` na pasta da **org de destino** e o `blob_path` das linhas é refeito.

## Reindexação

Depois de importar, a base de conhecimento e o catálogo da org são reindexados sem passo manual
(`sincronizar_org` de cada um), então `poph kb search` e a ferramenta `buscar_produto` já funcionam.

- Sem o modelo de embedding instalado (`poph model install`), a importação **termina com aviso** e
  o índice fica pendente: rode `poph kb reindex` e `poph product reindex` quando o modelo estiver
  disponível. Nada é gravado como índice vazio silencioso.
- O relatório traz `indice.base_conhecimento` e `indice.produtos` com agentes/trechos; o aviso, se
  houver, vai em `aviso` (e no erro padrão, na saída de texto).

## Validação e erros

| Situação | Resposta |
|---|---|
| arquivo truncado ou que não é JSON | `o arquivo … não é um backup do POP Harness (JSON inválido: …)` |
| `formato` diferente | "este arquivo não é um backup do POP Harness" |
| `versao_formato` desconhecida | "versão de formato desconhecida: N; esta instalação entende a M" |
| blob com conteúdo que não bate com o sha256 | "o conteúdo de um arquivo do backup não confere com o sha256 gravado" |
| coluna fora do esquema | "o arquivo traz a coluna `x` em `tabela`…" |
| chave natural repetida com outro id | lista dos conflitos, e nada é gravado (nem com `--sobrescrever`) |
| linhas que já existem no destino, sem `--sobrescrever` | total, tabelas e até 5 exemplos, e nada é gravado |
| blob ausente no disco na exportação | erro com o nome do arquivo e o sha256 (sem caminho) |

## Limites

- **Sem compactação**: o arquivo é JSON com os blobs em base64 (~4/3 do tamanho dos bytes). Um
  arquivo de 16 MB vira ~21 MB no backup. Pela API, o teto do corpo é 64 MB.
- **Arquivo inteiro na memória**: exportar e importar carregam o arquivo e os blobs na memória.
  Para orgs com muitos arquivos grandes, isso pesa; não há envio em pedaços.
- **Sem segredo**: a integração volta sem credencial; quem importou precisa digitar o segredo de
  novo. Rotacionar `POPH_CHAVE_CIFRA` antes de exportar só tira os nomes dos campos (`null`), nunca
  o valor.
- **Histórico**: `--com-conversas` leva todo o histórico vivo da org (a retenção já apaga o vencido
  antes, se estiver configurada).
- **Concorrência**: é um comando offline. Com o servidor escrevendo ao mesmo tempo, o retrato pode
  não refletir o último instante de cada tabela (a leitura é de uma conexão só, mas não trava o
  servidor). Rode com o servidor parado.
- **Importação numa org que já tem dados**: reimportar o mesmo arquivo atualiza **se** vier com
  `--sobrescrever` (sem ele, recusa com a lista do que seria sobrescrito); um arquivo diferente com
  os mesmos slugs e outro id é recusado sempre. Não há modo "mesclar".
- **HTTP**: as mesmas duas operações existem na API administrativa (`GET …/backup` e
  `POST …/backup/import`, com `?sobrescrever=true` para autorizar a atualização) — ver
  `docs/API.md`. O Console usa essas rotas, não a CLI.
