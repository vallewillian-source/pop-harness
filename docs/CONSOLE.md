# Console web do POP Harness

Console administrativo do POP Harness: SPA em React + TypeScript, servida **pelo próprio binário
`poph serve`** na raiz `/`. Diferente do POP Atende, aqui não existe tela de cliente: é ferramenta
interna de gestão e operação (PRD §10, UX_SPEC em `console/UX_SPEC.md`).

Esta página descreve o que existe hoje (HR-38 a HR-44d) e como o Console entra no binário.

## Rodar

Pré-requisito: Node >= 22 e `pnpm` (o monorepo usa pnpm; o Console fica fora do workspace do
`apps/frontend`, com `package.json` próprio).

```bash
cd apps/pop-harness/console
pnpm install          # única etapa que exige rede
pnpm dev              # http://127.0.0.1:5199, com proxy para a API em 127.0.0.1:7480
```

Em desenvolvimento o Vite faz proxy de `/admin`, `/healthz` e `/metrics` para o `poph serve` local,
então a origem é a mesma do build embutido e não há CORS em lugar nenhum.

Para o Console dentro do binário:

```bash
cd apps/pop-harness/console && pnpm build          # gera console/dist
cd apps/pop-harness && cargo build -p poph         # o build.rs embute dist no binário
./target/debug/poph serve                          # Console em http://127.0.0.1:7480/
```

### Encadeamento do build

1. `pnpm build` roda `tsc --noEmit` e o `vite build`, gerando `console/dist` (HTML, JS, CSS e as
   fontes copiadas para `public/res`).
2. O `build.rs` do `crates/poph-server` percorre `console/dist` e gera uma tabela de
   `include_bytes!` em `OUT_DIR/console_embutido.rs`. **O build do Rust não chama Node.**
3. `cargo build` compila essa tabela dentro do binário; `poph serve` a entrega em `/`.

**Sem o bundle** — clone limpo, CI offline, build de quem só quer a API — o crate compila
normalmente e `/` devolve uma página mínima explicando como gerar o Console. A API
(`/admin/v1`), `/healthz`, `/metrics` e `/webhook` não dependem do Console em nenhum momento.

## Serviço das rotas

| Caminho | Quem responde |
|---|---|
| `/admin/v1/...` | API administrativa (JSON, sempre) |
| `/healthz`, `/metrics`, `/openapi.json`, `/admin/v1/openapi.json` | servidor |
| `/webhook/evolution/...` | entrada do WhatsApp (Evolution) |
| `/assets/...`, `/res/fonts/...` | arquivos do bundle — `Cache-Control: immutable` (têm hash no nome) |
| qualquer outra rota (`/`, `/login`, `/orgs`, `/orgs/<id>`) | `index.html` do Console, sem cache |

Regras que o servidor garante (provadas em `crates/poph-server/tests/console.rs`):

- as rotas do servidor nunca caem no Console — inclusive rota inexistente dentro de `/admin/v1`,
  que continua erro JSON `not_found` com o caminho;
- caminho que parece arquivo (`/assets/nao-existe.js`) e não existe devolve **404 JSON**, nunca
  HTML no lugar de um asset (senão o navegador erra o tipo do conteúdo);
- `GET` e `HEAD` recebem o Console; qualquer outro método no caminho do Console devolve o erro JSON
  de rota inexistente;
- a página leva `Content-Security-Policy` (sem script inline, `frame-ancestors 'none'`),
  `X-Content-Type-Options: nosniff` e `Referrer-Policy: no-referrer`.

## Autenticação e escopo

- Login por token administrativo (`pophg_…` global, `poph_…` de org) — não há cadastro de usuário.
- O token fica **em memória e no `sessionStorage` da aba**; fechar a aba encerra a sessão. Nada vai
  para `localStorage` permanente, e a tela de login diz isso.
- A validação do token é uma chamada real à API (`GET /admin/v1/orgs`). Erro de digitação mostra
  mensagem inline; instância fora do ar mostra aviso de conexão com o endereço.
- `401` em qualquer chamada de uma sessão aberta encerra a sessão e volta ao login com aviso de
  "sessão encerrada".
- Escopo vem do prefixo do token (`pophg_` global, `poph_` de org). Com token global, o rail tem
  seletor de organização; com token de org, o seletor fica travado na própria organização — a UI
  não oferece o que o servidor recusaria.
- `403` (token válido, org errada) **não** encerra a sessão: mostra o erro da chamada. Só `401`
  significa token inválido.

## Telas desta rodada (HR-38)

- `/login` — token, com aviso de armazenamento e erro claro.
- `/orgs` — lista de organizações com estado de carga (skeleton da tabela), vazio e erro com
  "tentar de novo".
- `/orgs/:org_id` — resumo da organização (slug, situação, versão, retenção, criação).
- Casca global: rail de navegação com seletor de organização, barra superior com trilha, ambiente,
  saúde da instância e sair; `?` abre a lista de atalhos.

Criar, editar e excluir organização já existe (HR-39a, abaixo). Estrutura (categorias e
departamentos) veio na HR-39b, a **lista de agentes com a aba Instruções** veio na HR-40 e a
**aba Base de conhecimento** na HR-41 (abaixo). Produtos, arquivos, roteamento, integrações, modelo
e Jev e chat de teste chegam nas tarefas seguintes (HR-42 a HR-49) e aparecem no rail e nas abas do
agente como "em breve".

## Organizações: criar, editar e excluir (HR-39a)

As operações de escrita usam o mesmo contrato da CLI e da API (`docs/API.md`); o Console não tem
caminho próprio para o banco. O que muda é só a forma de operar:

- **Criar** — `POST /admin/v1/orgs` com nome (obrigatório, até 120), slug opcional (derivado do nome
  quando vazio) e retenção opcional em dias. O estado vazio da lista traz o CTA "Criar primeira
  organização". Slug repetido volta como `409 conflict` e vira erro no campo `Slug`, não em banner
  genérico.
- **Editar** — `PATCH` com `If-Match: <versão lida>`. Sem o cabeçalho a API responde `428`; com
  versão velha, `412 version_conflict`. No `412` a tela **relê** a organização, passa a usar a versão
  atual e **mantém tudo o que o operador digitou**, com o aviso "outra pessoa salvou antes — revise e
  salve de novo". Nada do formulário é descartado.
- **Excluir** — diálogo de alta fricção: o operador digita o slug para habilitar o botão. O
  `DELETE` é tentado sem `force` primeiro; se a API recusar por conteúdo (`409 conflict`), o motivo
  devolvido aparece como veio, com o código, e a confirmação extra ("apagar também agentes,
  categorias, departamentos e tokens") fica destacada. Marcada a confirmação, a exclusão repete com
  `?force=true`.
- **Estados** — carga (skeleton), vazio (com CTA), erro com "tentar de novo" e erro por campo. O
  formulário não fecha em erro: o que foi digitado fica.
- **Acessibilidade** — todo campo tem rótulo (`SofaField`), o foco volta para o botão que abriu o
  modal/diálogo ao fechar, e o caminho `Tab`/`Enter`/`Esc` foi exercitado nos testes de componente.
- **Escopo** — token de org não recebe botão de criar, nem de editar/excluir outra organização: a
  tela não oferece o que o servidor recusaria (`403`).

### Testes de componente

Infraestrutura nova nesta entrega: **Vitest + Testing Library + jsdom**, em `console/`.

```bash
cd apps/pop-harness/console
pnpm run testar              # ou `bash scripts/testar.sh`, que é o que o CI chama
```

Cobrem os formulários de criar e editar (incluindo o caminho do `412` com formulário preservado), a
confirmação de exclusão (recusa por vínculo com motivo real e caminho do `force`) e a lista de
organizações com carga, vazio e erro. O teste roda contra um `fetch` falso — sem rede, sem servidor.

`scripts/ci.sh` executa `console/scripts/testar.sh`, que **se declara pulado** quando
`console/node_modules` não existe, igual ao build do bundle e à checagem do cliente OpenAPI: o CI
offline de um clone limpo continua verde.


## Departamentos e categorias: tela e escrita (HR-39b)

Rota `/orgs/:id/structure`, com o item **Estrutura** do rail ligado à organização aberta e um atalho
no painel da organização. São duas colunas independentes: **Departamentos** (criar, editar, excluir)
e **Categorias de produtos** (criar, editar). Cada coluna tem carga, vazio com CTA e erro com
"tentar de novo" próprios; uma falha numa não derruba a outra.

- **Criar e editar** usam o mesmo contrato da CLI (`poph department` e `poph category`, que gravam no
  mesmo banco) e o `If-Match` da versão lida. Nome obrigatório (1 a 120), único na org sem diferenciar
  maiúsculas. Nome repetido volta da API e aparece **no campo Nome**, com o motivo real. Editar
  permite a situação (ativo, inativo, arquivado).
- **`412` ao editar**: a tela relê o registro, passa a usar a versão atual e mantém o que foi
  digitado, com o aviso "outra pessoa salvou este departamento antes de você".
- **Excluir departamento** — diálogo com a consequência escrita antes do botão: a tela lista os
  agentes que usam o departamento (como padrão ou para transferência) e diz que eles ficam sem ele.
  O botão só habilita com o nome exato digitado. O `DELETE` é enviado **sem** `force`: a API não
  pede confirmação para isso e não há o que confirmar além do nome.
- **Categoria: criar e editar** pela tela; a exclusão tem diálogo próprio (seção abaixo).
- **Token de org** vê e edita a estrutura da própria org (o servidor aceita). A tela não esconde nada
  disso: só a organização em si (criar/excluir) fica restrita ao token global.

### Exclusão de categoria (HR-39b, rodada 2)

Antes, a API apagava os produtos da categoria sem aviso (medido: `DELETE` sem parâmetro respondia
`204` e os produtos sumiam). A API passou a seguir a regra da CLI e da organização:

- **Sem `force`, com produtos ou agentes vinculados**: `409` com `code: conflict` e a mensagem
  "a categoria tem N produto(s) e M agente(s) vinculados; … Para confirmar, repita com force=true". O
  `details` traz `produtos` e `agentes`. **Nada é apagado.**
- **Sem `force`, sem vínculo**: `204`, como antes.
- **Com `force=true`**: apaga a categoria, os produtos dela (a chave estrangeira é `ON DELETE
  CASCADE`) e tira a categoria dos agentes. `204`.

Na tela, o diálogo "Excluir categoria?":

1. Ao abrir, conta os produtos (de qualquer situação, ativos, inativos ou arquivados) e os agentes
   que usam a categoria, e escreve a consequência com os nomes. Sem vínculo, diz que só o cadastro
   sai e não pede `force`.
2. O botão só libera com o nome exato da categoria digitado.
3. A primeira tentativa vai **sem** `force`. Se a API recusar, o motivo real aparece no diálogo com o
   código (`conflict`) e a confirmação extra "Apagar também os N produtos desta categoria" é liberada.
4. Marcada a confirmação, a exclusão repete com `force=true`. Foco volta ao botão "Excluir" da linha;
   `Esc` fecha.

A contagem usa a listagem de produtos com `all=true` e pagina só com `cursor`. **A listagem de
produtos recusa `limit` com `400`** (defeito do servidor, ver Limites): por isso a tela não manda
`limit` nessa rota, e a página tem o tamanho padrão do servidor.

Teste de servidor: `categoria_com_vinculos_so_sai_com_force` (`crates/poph-server/tests/api.rs`).
Mutação "apagar sem checar `force`" reprova o teste. Testes de componente em
`DialogoExcluirCategoria.test.tsx`, incluindo a paginação em duas páginas e a recusa de `limit`.

Contagem de agentes na coluna de departamentos vem de uma leitura da lista de agentes; se ela falhar,
a coluna mostra `agentes: —` e a lista de departamentos segue normal.

## Agentes e aba Instruções (HR-40)

Rotas novas: `/orgs/:org_id/agents` (lista) e `/orgs/:org_id/agents/:agent_id/instructions`
(aba 1 do agente). `/orgs/:org_id/agents/:agent_id` sem aba redireciona para Instruções. O rail
habilita "Agentes" quando há organização aberta e a trilha passa a mostrar
`Organizações / <org> / Agentes / Instruções`.

**Lista de agentes** — `GET …/agents`. Só seleção e leitura: não há criar/editar/excluir agente
nesta rodada, e o estado vazio diz onde criar hoje (`poph agent create` ou a API). Situação,
departamento padrão (nome resolvido por `GET …/departments`) e versão aparecem na tabela.

**Aba Instruções** — formulário estruturado à esquerda e prévia do markdown à direita:

- **Leitura:** `GET …/agents/{id}/instructions/current`. `404` significa que o agente ainda não tem
  instruções: a tela abre com os padrões do contrato e já preenche o nome do assistente com o nome
  do agente. Base de conhecimento e catálogo vêm ligados por padrão; as demais ferramentas, não.
- **Escrita:** `POST …/instructions` quando não existe (a versão esperada é `0`), `PATCH
  …/instructions/current` com `If-Match: <versão lida>` quando existe. O corpo manda **só
  `structured`**; o servidor compõe o markdown e devolve `markdown` + `version`, que a tela adota.
- **Prévia:** é o `markdown` que o servidor devolve — o Console nunca remonta o texto. "Atualizar
  prévia" relê o servidor; com edição pendente, a prévia mostra a última versão salva e um aviso
  explica isso.
- **Conflito de versão:** `428` (sem `If-Match`) e `412 version_conflict` são tratados. No `412` a
  tela relê as instruções, passa a usar a versão atual e **mantém o rascunho** com o aviso "outra
  pessoa salvou as instruções antes de você (agora na versão N)".
- **Alterações não salvas:** trocar de tela por qualquer link interno (rail, trilha, lista de
  agentes), mudar a organização no seletor do rail, fechar ou recarregar a página com edição
  pendente abre o aviso com **Salvar e sair**, **Descartar alterações** e **Cancelar**. "Desfazer"
  no cabeçalho volta o formulário ao último estado salvo.
- **Erros:** validação local (nome vazio, princípio sem regra) volta no próprio campo sem chamar a
  API; `400 validation_error` da API aparece na seção de ferramentas quando o motivo é ferramenta
  fora da V1, e em banner nos demais casos.
- **Teclado:** `Tab`/`Shift+Tab` percorrem os campos, `Enter` em botão aciona, `Esc` fecha o
  diálogo (com o foco de volta ao disparador) e `Ctrl+S`/`Cmd+S` salva quando há edição pendente.

**Paridade com a CLI** (mesmo banco, mesma config): o que a tela grava, `poph instructions show
--offline --org <org> <agente>` lê; o que `poph instructions update --offline --if-match <v> --file
<json>` grava, a tela mostra na próxima leitura. Medido nos dois sentidos no ambiente de teste
(`/tmp/hr40`, `poph serve` em porta alta, banco temporário):

| sentido | comando | resultado |
|---|---|---|
| tela → CLI | `PATCH …/instructions/current` com `If-Match: 4` e o corpo `{structured}` que a tela envia | `200`, versão 5; `poph instructions show` imprime "Beatriz Tela" e `extra_instructions` "Gravado pela TELA…" na versão 5 |
| CLI → tela | `poph instructions update --if-match 5 --file cli3.json` | `GET …/instructions/current` devolve versão 6, "Beatriz CLI" e o markdown recomposto — é o que a aba exibe |
| `428` | `PATCH` sem `If-Match` | `{"code":"precondition_required"}` |
| `412` | `PATCH` com `If-Match: 1` (versão velha) | `{"code":"version_conflict","details":{"current_version":6,"expected_version":1}}`, e o registro continua na versão 6 |

A tela trata os dois conflitos sem perder o rascunho; o teste de componente
`no 412 relê as instruções, adota a versão nova e mantém o que foi digitado` cobre esse caminho
(o caso de teste não fala com o servidor: usa `fetch` falso).

**Testes** — 7 de componente da aba (`AgenteInstrucoes.test.tsx`: carga, salvamento com `If-Match`,
`412` sem perder o formulário, validação local, criação, erro de validação da API e o aviso de
alterações não salvas) e 3 da lista (`Agentes.test.tsx`: lista com departamento, vazio e erro com
"tentar de novo"). O `preparo.ts` dos testes ganhou um no-op de `scrollIntoView`, que o jsdom não
tem e o `SofaTabSwitcher` usa.

## Base de conhecimento do agente (HR-41)

Rota `/orgs/:org_id/agents/:agent_id/knowledge-base`, na aba 2 do agente. O rail leva à lista de
agentes; a trilha mostra `Organizações / <org> / Agentes / <nome do agente> / Base de conhecimento` (HR-49b).

Duas colunas:

- **Grupos da base** (`GET/POST …/knowledge`, `GET/PATCH/DELETE …/knowledge/{id}`): lista os grupos da
  organização, marca quais **este agente** usa (o vínculo vem no detalhe do grupo, `agents`) e deixa
  vincular/desvincular (`POST`/`DELETE …/agents/{agent_id}/knowledge/{group_id}`). Criar e editar
  usam o mesmo contrato da CLI (`poph kb group`).
- **Trechos** do grupo aberto (`GET/POST …/knowledge/{group_id}/items` e `…/items/{item_id}`): título
  (até 200), conteúdo (**até 1000 caracteres**, contador ao vivo), tags e ordem. O limite é o do
  servidor (`LIMITE_CONTEUDO_ITEM`), não uma escolha da tela.

**Concorrência:** `PATCH` de grupo e de item manda `If-Match` da versão lida. Sem o cabeçalho a API
responde `428`; com versão velha, `412 version_conflict`. No `412` a tela relê o registro, passa a
usar a versão atual e **mantém o que foi digitado**, com o aviso "outra pessoa salvou …; revise e
salve de novo" — o mesmo comportamento das outras abas.

**Exclusão:**

- **Grupo** — a API exige `force=true` **sempre** (sem ele responde `409`, mesmo com o grupo vazio) e
  apaga os itens e os vínculos junto. O diálogo conta os trechos, lista os agentes que usam o grupo e
  só libera o botão com o nome exato digitado; a confirmação é o que substitui o `force` na tela.
- **Trecho** — confirmação pelo título exato, com `DELETE` simples.

**Painel do índice — o que a API permite saber.** Não existe rota de estado do índice na API
administrativa (nem contagem de trechos indexados, dimensão ou versão do chunker). O painel não
inventa estado:

| Situação | O que a tela mostra |
|---|---|
| Ainda sem gravação nem reindexação nesta carga | "Índice não verificado", com o caminho `poph kb status --offline` para consultar o estado real |
| Depois de gravar grupo/trecho/vínculo | "Índice atualizado" se o servidor não devolveu `index_warning`; "Reindexação necessária" com o aviso quando devolveu |
| Depois de "Reindexar agente" | "Índice atualizado" com o relatório (novos, alterados, inalterados, removidos, embeddings, trechos) |

**Reindexação é síncrona.** `POST …/agents/{id}/knowledge/reindex` só responde quando a
sincronização termina; a tela avisa que pode demorar (a primeira carrega o modelo de embedding) e
não fecha a aba. Sem o modelo instalado a API responde `503 embedding_unavailable` e a tela diz que
o índice não mudou e que é preciso instalar o modelo — nada é mascarado como sucesso.

**Paridade com a CLI** (mesmo banco, mesma config, ambiente de teste `/tmp/hr41`, `poph serve` em
127.0.0.1:18271):

| sentido | comando | resultado |
|---|---|---|
| tela → CLI (grupo) | `POST …/knowledge` com o corpo que o formulário envia | `201`; `poph kb group list --offline` mostra "Políticas da loja", mesma descrição e versão 1 |
| CLI → tela (grupo) | `poph kb group create --name "Perguntas frequentes"` | `GET …/knowledge` devolve os dois grupos, com os mesmos ids |
| tela → CLI (trecho) | `POST …/knowledge/{g}/items` | `201`; `poph kb item list --offline` mostra o trecho com tags e posição |
| CLI → tela (trecho) | `poph kb item create --group … --title "Formas de pagamento"` | `GET …/knowledge/{g2}/items` devolve o trecho criado pela CLI |
| CLI → tela (edição) | `poph kb item update … --if-match 1 --title "… (editado pela CLI)"` | `GET` do item devolve versão 2 e a posição nova — é o que a tela exibe |
| `428` | `PATCH` de grupo e de item sem `If-Match` | `{"code":"precondition_required"}` nos dois |
| `412` | `PATCH` com `If-Match` de versão velha (7 no grupo, 9 no item) | `{"code":"version_conflict","details":{"current_version":1,"expected_version":…}}`, registro intacto |
| exclusão de grupo | `DELETE` sem `force` → `409`; com `force=true` → `200` e o `GET` depois dá `404` | a tela sempre manda `force=true`, depois da confirmação |

**Exclusão de grupo feita no navegador, na mesma instância de teste:** o diálogo contou o trecho do
grupo, a confirmação por nome liberou o botão, e a exclusão levou o grupo junto com o trecho. Logo
depois, `GET …/knowledge/{g}`, `…/{g}/items` e `…/{g}/items/{i}` responderam `404 not_found` e
`poph kb group list --offline` listou só o grupo que sobrou — o mesmo resultado da linha de comando.
Nada mais na base mudou (o outro grupo e o trecho dele continuaram intactos).

**Testes** — 15 de componente em `AgenteConhecimento.test.tsx`: carga com o grupo vinculado aberto,
criação de grupo, criação de trecho (corpo, contador e tags), validação local sem chamar a API, nome
repetido no campo, `412` no grupo e no trecho sem perder o rascunho, exclusão de grupo com a
contagem e `force=true`, desvinculação, reindexação com relatório, o `503` de modelo ausente, os
dois vazios (grupo sem trechos e org sem grupos) e o erro de carga com "tentar de novo".
Total do Console: 49 testes.

O helper `testes/modal.ts` passou a esperar o **estado** (diálogo aberto e foco dentro dele) em vez
de 30 ms fixos: sob carga, o foco inicial do `FloatingFocusManager` atrasava e a digitação se perdia
num campo ainda não focado — o teste `DialogoExclusao` reprovava de vez em quando sem haver defeito
no produto (medido: 1 falha em 8 execuções da suíte antes; 0 em 6 execuções com 12 processos de CPU
ocupando a máquina depois). O `console.rs` do servidor ganhou um contador na pasta temporária pelo
mesmo motivo: dois testes que abriam banco no mesmo instante dividiam a pasta e um derrubava o outro
com "database is locked".

O teto de espera das buscas assíncronas (`findBy*`/`waitFor`) subiu de 1 s para 5 s em
`testes/preparo.ts`: as telas encadeiam buscas (agente → organização → grupos → detalhe do grupo →
trechos) e, com a suíte inteira em paralelo e a máquina ocupada, 1 s estourava às vezes — foi o que
o CI de outro agente viu uma vez em `AgenteConhecimento.test.tsx`. Eu não consegui reproduzir: 15
execuções seguidas da suíte passaram (3 ociosas, 5 com 12 núcleos ocupados, 6 com 24 processos),
antes e depois da mudança. O teto maior só custa tempo quando algo realmente não aparece.

## Catálogo de produtos (HR-42)

Rota `/orgs/:org_id/products`, no rail (item "Produtos", sob "Gestão") e na trilha
(`Organizações / <org> / Produtos`). Também há um cartão "Produtos" no painel da organização. A tela
fica no contexto da organização, e não dentro do agente (ver Desvios).

**O que a tela faz**

- **Tabela** com todos os produtos (de qualquer situação): produto e identificador, outros nomes,
  categoria, preço como o cliente vê e unidade, situação e a coluna **"Agentes que oferecem"**.
- **Filtros** por nome (inclusive outro nome e identificador, sem diferenciar acento e maiúscula),
  situação (todas, ativos, inativos, arquivados) e categoria. Os filtros rodam no navegador sobre a
  lista inteira já carregada.
- **Criar e editar** pelo mesmo formulário (`ModalProduto`). Criar não envia situação, porque a API
  recusa `status` na criação. Preço se digita em reais (`450` ou `450,00`) e vai em centavos.
  "Preço como o cliente vê" é opcional e começa vazio; a regra de envio está em "Preço como o cliente
  vê" logo abaixo.
- **Excluir** pelo diálogo de confirmação (`DialogoExcluirProduto`), que só libera o botão com o
  nome exato digitado. Antes de confirmar, diz quantos agentes deixam de oferecer o produto e que
  itens de carrinho que o citam perdem a referência a ele. A exclusão é em dois passos (HR-42g),
  descritos abaixo.

**Exclusão em dois passos (HR-42g).** A tela não sabe antes se o produto está em carrinho (a API de
administração não tem leitura de carrinho), então a primeira tentativa vai **sem `force`**:

1. Sem carrinho: a API apaga, a tela fecha o diálogo, atualiza a lista e mostra o aviso do índice
   quando ele vier. Não aparece nenhuma confirmação extra.
2. Com carrinho: a API recusa com `409 conflict` e **não apaga nada**. A tela mantém o diálogo aberto,
   mostra o motivo como a API devolveu e, em linguagem comum, a contagem que veio no `409`: "Este
   produto está em N itens de M carrinhos. Excluir deixa esses itens como 'produto removido do
   catálogo', com o preço que já estava gravado." Só então aparece a confirmação extra **"Excluir
   mesmo assim: N itens de carrinho ficam como 'produto removido do catálogo'"**. Marcada e com o
   nome digitado, o botão "Excluir produto" envia `?force=true` e conclui.

O desenho segue o da categoria (HR-39b): a confirmação extra fica no próprio diálogo, com o nome
ainda digitado, e não há um segundo botão. Desvio registrado nos Desvios abaixo. Erro que não seja
`409` (por exemplo `404`) aparece como veio e o diálogo não fecha; `Esc` fecha sem excluir.

**Preço como o cliente vê (HR-42d).** A API devolve sempre o texto calculado (`R$ 8.900,00`), e não
diz se alguém digitou um texto próprio. Por isso a tela não preenche o campo com o valor devolvido:

- o campo **começa vazio**; embaixo, a linha "Como o cliente vê: …" mostra o texto que vale hoje (o
  gravado, se houver, ou o calculado pelo valor digitado em centavos, sem chamada extra ao servidor);
- o `price_text` **só vai no corpo** quando (a) o operador digitou no campo (vai o texto digitado; vazio
  volta a calcular), ou (b) o preço em centavos mudou e o campo não foi mexido: vai vazio, para o
  cliente não continuar vendo o valor antigo;
- nos outros casos o campo não vai no corpo e o servidor mantém o texto gravado;
- o botão "Voltar a calcular o texto pelo valor" aparece quando há texto próprio e manda vazio.

Desvio do pedido: a regra (b) não estava no pedido, que previa não enviar nada quando só o preço
muda. Sem ela, um produto com texto próprio gravado (por exemplo, "R$ 320,00 por visita") continuaria
mostrando o valor antigo ao cliente depois da troca de preço. O custo é que a troca de preço apaga o
texto próprio; a tela avisa isso no campo antes de salvar.

**Rotas usadas** (contrato de `docs/API.md`, sem mudança):

| Ação | Rota |
|---|---|
| Listar (todas as situações) | `GET /admin/v1/orgs/{org}/products?all=true`, paginando só com `cursor` |
| Criar | `POST /admin/v1/orgs/{org}/products` |
| Ler (depois de um `412`) | `GET /admin/v1/orgs/{org}/products/{id}` |
| Alterar | `PATCH /admin/v1/orgs/{org}/products/{id}` com `If-Match` |
| Excluir | `DELETE /admin/v1/orgs/{org}/products/{id}` (sem `force`, ver Limites) |
| Categorias (filtro e formulário) | `GET /admin/v1/orgs/{org}/categories` |
| Agentes (coluna e diálogo) | `GET …/agents` e `GET …/agents/{id}` (categorias de cada um) |

**Concorrência:** o `PATCH` manda `If-Match` com a versão lida. Sem o cabeçalho a API responde `428`;
com versão velha, `412 version_conflict`. No `412` a tela relê o produto, passa a usar a versão atual
e **mantém tudo o que foi digitado**, com o aviso "Outra pessoa salvou este produto antes de você
(agora na versão N)…". Salvar de novo grava com a versão nova.

**Índice de busca:** a criação, a alteração e a exclusão devolvem `index_warning` quando o índice de
produtos não atualizou (por exemplo, sem o modelo de embedding instalado). Nesse caso a tela mostra
um aviso no topo com o texto da API; a gravação está salva, só a busca dos agentes fica para depois da
reindexação.

**Coluna "Agentes que oferecem":** segue a regra do servidor (`visiveis_do_agente`). Um agente sem
nenhuma categoria vinculada enxerga o catálogo inteiro; com categorias, só enxerga produtos delas. Por
isso o agente sem vínculo entra na conta de todos os produtos. Se a leitura dos agentes falhar, a
coluna mostra "—" e o catálogo segue normal.

**Permissão:** token de org vê e altera só a própria organização. Medido: `GET` de produtos da própria
org `200`, `POST` na própria org `201`, e produtos ou um produto de outra org `403`.

**Prova de paridade com a CLI (HR-42), nos dois sentidos, com servidor no ar e banco temporário:**

```bash
# tela → CLI: a tela criou "Paridade pela tela" (categoria Ortodontia, R$ 180,00)
poph product list --offline --org clinica-hr42 --all     # mostra o produto com R$ 180,00
poph product show --offline --org clinica-hr42 paridade-pela-tela   # versão 1, situação ativo
# CLI → tela: a CLI alterou o mesmo produto
poph product update --offline --org clinica-hr42 --if-match 1 \
    --name "Paridade pela CLI" --price 25000 --status inactive paridade-pela-tela
# A tela, relida, mostra "Paridade pela CLI", "R$ 250,00", "Inativo", versão 2.
```

Roteiro de tela no Chrome, contra o servidor de teste: 22 de 22 passos de negócio ok (login, filtros,
criar, aviso do índice, `412` com nome preservado e salvamento seguinte na versão 3, situação
Inativo, `Esc` devolvendo o foco ao botão Editar, exclusão com nome exato e `404` depois, rótulos
de todos os campos e validação de nome vazio). A checagem de console acusou só os `412` provocados
de propósito e um `404` de `favicon.ico`, que o servidor não serve (também visto na HR-39b).

**Testes de componente:** `ModalProduto.test.tsx` (corpo da criação com centavos e listas, sem
`status` e sem `slug` vazio, validação sem chamar a API, erro de campo da API no campo, `412`
preservando o formulário e salvando com a versão nova), `DialogoExcluirProduto.test.tsx` (botão só
com o nome exato, contagem de agentes, `DELETE` sem `force` sem carrinho, motivo real da recusa,
`409` com a contagem sem apagar e a confirmação extra que envia `force=true`, `Esc` no segundo
passo) e
`Produtos.test.tsx` (listagem sem `limit`, contagem de agentes, filtros, estados vazio, sem resultado
e de erro).

## Arquivos e Roteamento do agente (HR-43)

As duas abas ficam dentro do agente, com as rotas `/orgs/:org_id/agents/:agent_id/files` e
`/orgs/:org_id/agents/:agent_id/routing`, na mesma barra de abas de Instruções e Base de conhecimento.
Trilha: `Organizações / <org> / Agentes / Arquivos` (ou `Roteamento`).

**Arquivos (biblioteca que o agente pode enviar ao cliente)**

- Tabela com nome e descrição, tipo, tamanho, início do SHA-256 e a ação de remover.
- **Enviar arquivo**: modal com o campo de arquivo, nome mostrado ao cliente (até 200 caracteres) e
  descrição para o agente (até 500). Tipos aceitos: PDF, Word (`.docx`), Excel (`.xlsx`/`.xls`),
  PowerPoint (`.pptx`) e imagem (JPEG, PNG, WebP, GIF).
- **Limite de 16 MB por arquivo**: quem passa do limite é recusado **ao escolher o arquivo**, com o
  tamanho em que passou. A API não é chamada. Se o servidor recusar (tipo errado, arquivo vazio,
  corpo acima do limite), o motivo que ele devolve aparece no campo do arquivo, como veio.
- **Remover**: confirmação digitando o nome do arquivo. A exclusão não tem desfazer; para voltar,
  é preciso enviar de novo.
- Sem `If-Match`: a biblioteca não tem versão (docs/BIBLIOTECA_ARQUIVOS.md).

**Roteamento (para onde a conversa vai e quando passa a humano)**

- **Departamento padrão**: onde a conversa começa. Lista os departamentos da organização
  (`Estrutura`); departamento inativo aparece com "(inativo)".
- **Transferir para atendente humano** (liga/desliga) e **departamento humano de destino**
  (obrigatório com a transferência ligada, validado antes da chamada).
- **Motivos de transferência**: até 15, com até 300 caracteres cada. Motivos vazios não são gravados.
  Ficam em `jev_json.situacoes_de_handoff` do agente, que é o que o motor lê hoje (ver Desvios).
- **Gravação com `If-Match`** da versão do agente. A tela manda só o que mudou; ao mudar os motivos,
  manda o `jev_json` inteiro, com as outras chaves do agente (despedida, gates) como foram lidas.
- **412**: a tela relê o agente, passa a usar a versão atual e **mantém o que foi digitado**. Como o
  envio é só do que o operador mudou, uma mudança feita por outra pessoa em outro campo não é
  desfeita. Mesma mensagem das outras abas.
- **428 e demais erros**: mostrados com código e motivo reais da API.
- Botões "Descartar alterações" e "Salvar roteamento"; `Ctrl+S` grava. Sair com alteração pendente
  passa pela guarda de alterações não salvas.

**Rotas usadas (`docs/API.md`, sem mudança de contrato)**

| Operação | Rota |
|---|---|
| listar arquivos | `GET /admin/v1/orgs/{org}/agents/{agent}/files?limit=200` |
| enviar arquivo | `POST /admin/v1/orgs/{org}/agents/{agent}/files?filename=…&name=…&description=…` (corpo binário, `application/octet-stream`) |
| remover arquivo | `DELETE /admin/v1/orgs/{org}/agents/{agent}/files/{file}` |
| ler agente (roteamento) | `GET /admin/v1/orgs/{org}/agents/{agent}` |
| gravar roteamento | `PATCH /admin/v1/orgs/{org}/agents/{agent}` com `If-Match` |
| departamentos | `GET /admin/v1/orgs/{org}/departments?limit=200` |

**Paridade com a CLI (medida nos dois sentidos, mesma base `/tmp/hr43`)**

- Arquivos: o envio com o corpo da tela aparece em `poph file list --agent beatriz --offline`; o
  `poph file upload` aparece na API que a tela lê; a remoção feita pela API some da CLI, e a removida
  pela CLI (`poph file delete`) some da API. A CLI recusa acima de 16 MB e não grava nada.
- Roteamento: o `PATCH` com o corpo da tela aparece em `poph agent show --json` (departamento padrão,
  transferência, departamento humano e `jev.situacoes_de_handoff`, com a despedida preservada);
  `poph agent update --if-match <v> --department … --handoff false --jev-json '…'` aparece na API
  que a tela lê. Com versão velha, a CLI recusa com o mesmo conflito (esperada 1, atual 8).

**Ponta a ponta no Chrome, contra o servidor de teste (porta 18291):** envio pela tela (aparece na
lista), 16 MB + 1 byte recusado na tela antes do envio, remoção com nome digitado (conferida na API),
departamento e transferência gravados pela tela, um `412` provocado por alteração feita pela API antes
do salvamento (aviso de versão, departamento e motivo digitados mantidos) e gravação seguinte na
versão atual. A despedida configurada no agente continua no `jev_json` depois de gravar os motivos.

**Testes de componente:** `AgenteArquivos.test.tsx` (lista e `limit=200`, estado vazio, envio com
nome e descrição na query e o próprio `File` no corpo, recusa de 16 MB antes da API com o tamanho,
motivo do servidor no campo do arquivo, remoção com nome exato). `AgenteRoteamento.test.tsx`
(carga do roteamento, `PATCH` só com o que mudou e `If-Match`, `jev_json` com as outras chaves,
`412` com releitura e rascunho preservado, departamento humano obrigatório sem chamar a API, 300
caracteres e 15 motivos, código e motivo reais da API, botão desabilitado sem alteração, estado
vazio de departamentos). Os 67 testes anteriores seguem verdes.

## Decisões desta entrega (HR-43)

- **Motivos de transferência em `jev_json.situacoes_de_handoff`**, não em `scope.handoff.reasons`
  das instruções (ver Desvios).
- Limite de 15 motivos e 300 caracteres aplicado **na tela**; a API aceita mais (medido: 16 motivos
  e um de 301 caracteres foram gravados).
- Departamento padrão e departamento humano **não voltam a "nenhum"** depois de definidos: a API trata
  `null` como "não mexer" (medido: "nada a alterar") e aceita `""` só como erro.

## Desvios desta entrega

| Spec | O que foi feito | Por quê |
|---|---|---|
| Motivos de transferência em `scope.handoff.reasons` das instruções (UX_SPEC §3.x, aba Roteamento) | Gravados em `jev_json.situacoes_de_handoff` do agente | O motor lê `situacoes_de_handoff` (`poph_jev::situacoes_do_agente`, chamada no turno do `poph-agent`). `scope.handoff.reasons` existe no contrato, mas o motor não o lê; gravar lá não mudaria nada no atendimento. |
| Aba Arquivos com extração de texto e coluna de SHA-256 completa (UX_SPEC) | Tipo, tamanho e início do SHA-256 | A API não devolve extração de texto; o SHA-256 completo fica no detalhe. |
| Excluir arquivo com confirmação genérica | Confirmação pelo nome do arquivo, como o resto do Console | Mesma trava das outras exclusões; a biblioteca não tem `force`. |

## Limites conhecidos (HR-43)

- **Listas sem paginação** (média, acima de 200 registros): arquivos e departamentos leem só 200.
- **Motivos sem limite na API** (média): a API grava qualquer quantidade e qualquer tamanho em
  `jev_json`; o limite de 15 × 300 só existe na tela. Quem grava pela CLI ou pela API direta
  (`poph agent update --jev-json`) pode passar do limite que o Jev documenta.
- **Não dá para voltar a "nenhum" departamento** (média): depois de definidos, departamento padrão e
  departamento humano só trocam de valor. A tela não oferece a opção vazia.
- **Exclusão de arquivo sem desfazer** (média, por desenho): o registro sai; o conteúdo fica no disco
  até a retenção apagar o que não tem dono.
- **Tamanho exibido pela CLI** (baixa, fora do Console): `poph file upload` recusa 16 MB + 1 byte com
  "16.000000953674316 MB"; a mensagem está certa, a precisão não.
- **Departamento que sai muda o agente sem mudar a versão** (baixa, herdado): a exclusão de
  departamento desfaz o vínculo do agente sem subir a versão dele, então um formulário de agente aberto
  antes não recebe `412`.
- **Tamanho do bundle** (baixa): o build do Vite passou de 500 kB (aviso, não erro).
- **Sem teste de navegador automatizado no CI** (baixa, já registrado): a prova de tela foi manual
  (Chrome, roteiro acima), e o CI roda os testes de componente.

## Integrações do agente (HR-44, HR-44d e HR-44e)

A aba fica dentro do agente, na rota `/orgs/:org_id/agents/:agent_id/integrations`, na mesma barra
de abas das outras. Trilha: `Organizações / <org> / Agentes / Integrações`.

**O que a tela faz**

- **Lista** as integrações do agente com tipo (`kind`), situação, estado de conexão, se há segredo
  gravado, quantos campos de configuração estão gravados, versão e as ações de editar e excluir.
- **Cria** com tipo (obrigatório, até 60 caracteres na tela), situação e, quando o tipo declara
  plugin no servidor, o **formulário gerado do manifesto** (seção abaixo). Sem manifesto para o
  tipo, o corpo é só `{ "kind": …, "status": … }` e nenhuma configuração é gravada.
- **Edita** a situação (`active`/`inactive`/`archived`), a configuração e os segredos com `If-Match`
  da versão lida. O tipo é somente leitura depois de criado (a API recusa troca de `kind`).
- **Exclui** com confirmação digitando o tipo. A API não pede `force` aqui, e a tela não manda.
- **Segredos**: a resposta traz só `tem_segredo` e os **nomes** dos campos preenchidos
  (`segredos_preenchidos`). O valor nunca sai do servidor; a tela mostra "gravado (não exibido)" ou
  "sem segredo" e não tem campo para lê-lo.

### Formulário do plugin (HR-44e)

**O formulário vem do plugin, não do Console.** Ao escolher (ou abrir) um tipo, a tela lê
`GET /admin/v1/orgs/{org}/manifests/{kind}` (HR-44c) e desenha `FormularioEsquema` com a
`configuracao` e os `segredos` que o manifesto declara — nenhuma integração tem formulário escrito à
mão. O mesmo componente já desenhava o esquema real do `poph-plugin-eco`; agora ele recebe a fonte
de verdade do servidor.

- **Enumeração do tipo:** enquanto o operador digita, a leitura espera 400 ms (não sobe o plugin a
  cada tecla); na edição é imediata. Enquanto lê, o botão de salvar avisa e não envia.
- **`200`:** desenha os campos (texto, número, booleano, `enum`, obrigatório, `default`, `title` e
  `description`) e envia `config_json` no `POST`/`PATCH` com os tipos do esquema (número como número,
  booleano como booleano; campo vazio fica fora do objeto).
- **`404`:** este tipo não declara plugin no servidor. A tela avisa ("Este tipo não tem formulário")
  e cria só com tipo e situação — **nunca** grava configuração sem esquema (a API responderia `422`).
- **`503 manifesto_indisponivel`:** o plugin está declarado e não respondeu ao handshake. A tela
  explica, mostra o código e **não deixa salvar**, porque o servidor recusaria a criação do mesmo
  jeito. Falha de transporte e manifesto sem esquema de objeto caem no mesmo bloqueio.
- **Segredo:** campo `password`, começa **sempre vazio**, só vai no corpo quando o operador digita.
  Em branco mantém o que está gravado; digitar troca; o campo avisa "já gravado: deixe em branco para
  manter, digite para trocar". Segredo não obrigatório pode ser removido por uma caixa ("Remover este
  segredo ao salvar"), que manda `""`. `segredos_preenchidos: null` (chave de cifra trocada) vira
  "gravado, mas a chave atual não consegue lê-lo" — nunca "vazio".
- **Configuração já gravada:** o formulário parte do `configuracao` devolvido pela integração,
  sobreposto aos `default` do esquema. Ao salvar, chaves que o formulário não desenha (objeto
  aninhado, `additionalProperties`) são preservadas; as desenhadas em branco saem.
- **Erro de campo:** `400 validation_error` com `details.field` (`config_json.remetente`,
  `secrets.token`, aninhado com ponto) cai no campo certo, com o motivo da API; o resto vai para o
  aviso geral, com `code`/`message` reais.
- **`412`:** relê a integração, adota a versão atual e **mantém a configuração e os segredos
  digitados nesta sessão**, com a mesma mensagem das outras abas. O salvamento seguinte usa a versão
  nova.

### WhatsApp: pareamento, estado e desconexão (HR-44d)

Na integração de `kind = whatsapp`, o painel da tela usa as três rotas de admin da HR-44b. Antes
desta rodada a aba declarava que a API não as tinha; agora ela opera de verdade.

- **Conectar / gerar QR** (`POST …/whatsapp/qrcode`, `If-Match` da versão lida): desenha o QR do
  `qrcode.base64` (data URL) e mostra a rodada (`qrcode.count`) e o `pairing_code` quando vier.
  Aparelho já conectado (`state: open`, `qrcode: null`) → aviso "Aparelho conectado", sem QR.
- **Número opcional**: campo "Número do WhatsApp (opcional)", só dígitos com DDI (10 a 15). Em
  branco, o pareamento é pelo QR (sem corpo na chamada); com o número, o corpo é `{"number": …}` e o
  WhatsApp também devolve o código de 8 caracteres. Número fora do formato é recusado na tela, sem
  chamar a API.
- **Verificar estado** (`GET …/whatsapp/state`): mostra o `connection_state` ao vivo e atualiza a
  linha da lista. Em `awaiting_scan`, a tela consulta sozinha **a cada 4 s, no máximo 45 vezes
  (~3 min)**, parando ao conectar, ao dar erro, no botão "Parar consulta automática" e ao sair da
  tela.
- **Desconectar aparelho** (`POST …/whatsapp/disconnect`, `If-Match`): abre confirmação dizendo que o
  aparelho para de receber e enviar e que **a integração continua cadastrada**. O botão fica
  desabilitado quando o estado já é `disconnected`.
- **Sem contagem regressiva**: `expira_em` é sempre `null` (a Evolution v2 não informa validade). A
  tela diz que o QR muda sozinho a cada ~20 s e oferece "Gerar outro QR" quando o QR está na tela.
- **Segredo e QR não vazam**: a tela nunca lê nem exibe segredo, e o `base64` do QR não vai para o
  console do navegador (conferido na captura automatizada).

**Rotas usadas**

| Operação | Rota |
|---|---|
| listar | `GET /admin/v1/orgs/{org}/agents/{agent}/integrations?limit=200` |
| criar | `POST /admin/v1/orgs/{org}/agents/{agent}/integrations` (corpo `{kind, status}`) |
| reler (após `412`) | `GET /admin/v1/orgs/{org}/agents/{agent}/integrations/{id}` |
| alterar situação | `PATCH /admin/v1/orgs/{org}/agents/{agent}/integrations/{id}` com `If-Match` |
| manifesto do plugin | `GET /admin/v1/orgs/{org}/manifests/{kind}` |
| excluir | `DELETE /admin/v1/orgs/{org}/agents/{agent}/integrations/{id}` |
| gerar QR | `POST …/integrations/{id}/whatsapp/qrcode` com `If-Match` (corpo opcional `{number}`) |
| estado ao vivo | `GET …/integrations/{id}/whatsapp/state` |
| desconectar | `POST …/integrations/{id}/whatsapp/disconnect` com `If-Match` |

**Conflito de versão:** as escritas de configuração (`PATCH`) e as duas de WhatsApp exigem
`If-Match`; sem ele `428 precondition_required`, versão velha `412 version_conflict` com
`details.current_version`. No `412` a tela relê a integração, passa a usar a versão atual e mantém o
que o operador escolheu (situação ou painel), com a mesma mensagem das outras abas. As duas rotas de
WhatsApp **não sobem a versão**: `connection_state` é estado de execução, não edição.

**Erros do painel** aparecem com a mensagem, a dica de operação e o código reais, entre eles:
`409 whatsapp_nao_configurado` (integração sem instância da Evolution declarada na configuração do
servidor), `503 evolution_indisponivel` (sem chave ou serviço fora), `409 integracao_nao_e_whatsapp`,
`502 evolution_recusou` e `502 evolution_sem_qr`.

**O que a API ainda NÃO oferece:** nada que esta aba consuma. A configuração por formulário gerado
do manifesto saiu do papel na HR-44c (rota do manifesto e `config_json`/`secrets` aceitos) e está na
tela desde a HR-44e. Tipo sem plugin declarado continua sem formulário — é o caso do `whatsapp`, que
opera pela Evolution e é criado só com tipo e situação.

**Provas por HTTP do formulário do plugin (HR-44e)** — servidor real em `127.0.0.1:18361`, banco
`/tmp/hr44e/e2e`, `[plugins].integracoes` apontando para o `plugin-eco` real, token em arquivo 0600
(nunca impresso), corpos iguais aos que a tela envia:

```
GET  …/manifests/eco                            → 200 {nome:"Eco (exemplo)", versao:"0.1.0",
                                                    configuracao:{required:["remetente"],enum:"inverter"},
                                                    segredos:{required:["token"]}}
GET  …/manifests/whatsapp                       → 404 not_found (tipo sem plugin declarado)
POST integração sem remetente                   → 400 {field:"config_json.remetente",reason:"obrigatório"}
POST integração com campo "cor"                 → 400 {field:"config_json.cor",reason:"não está no esquema"}
POST integração válida                          → 201 v1 {tem_segredo:true, segredos_preenchidos:["token"],
                                                    configuracao:{modo,remetente}}
GET  da integração                              → 200 com configuracao e nomes dos segredos; 0 ocorrências do valor
PATCH sem If-Match                              → 428 precondition_required
PATCH If-Match: 9                               → 412 {current_version:1, expected_version:9}
PATCH If-Match: 1 (config + segredo novos)      → 200 v2
PATCH removendo segredo obrigatório             → 400 {field:"secrets.token",reason:"obrigatório"}
POST whatsapp com config_json                   → 422 manifest_indisponivel (continua sem formulário)
GET  /orgs/{org}/agents/{agente}/integrations   → 200 text/html (SPA)
segredo no log do servidor / na base            → 0 e 0; coluna secrets_enc com 58 bytes de ciphertext
```

**Na tela (navegador real, servidor acima):** login com token, aba Integrações, "Nova integração",
tipo `eco` → o formulário aparece com "Configuração do plugin “Eco (exemplo)” (versão 0.1.0)", o modo
`inverter`, "Nome do remetente *" (obrigatório) e "Token do canal *" como campo de senha **vazio**
(captura mostrada no chat da entrega; o roteiro de navegador é manual, fora do CI).

**Paridade com a CLI: não se aplica.** `docs/API.md` registra integrações como **exceção**: a CLI
não tem comando de integração. A prova é por HTTP, com o mesmo corpo que a tela envia.

**Provas por HTTP (servidor de teste em 127.0.0.1:18331, banco `/tmp/hr44d`, Evolution falsa em
18198)**

```
POST …/whatsapp/qrcode sem If-Match              → 428 precondition_required
POST …/whatsapp/qrcode If-Match: 1               → 200 {state:"connecting", connection_state:"awaiting_scan",
                                                       qrcode:{base64:"data:image/png;base64,…",code,count:1},
                                                       expira_em:null}
GET  …/whatsapp/state                            → 200 {state:"close", connection_state:"disconnected"}
(celular escaneou: controle da Evolution falsa)  → aberto
GET  …/whatsapp/state                            → 200 {state:"open", connection_state:"connected"}
POST …/whatsapp/disconnect If-Match: 0           → 412 version_conflict {current_version:1, expected_version:0}
POST …/whatsapp/disconnect If-Match: 1           → 200 {connection_state:"disconnected", disconnected:true}
GET  …/whatsapp/state                            → 200 {state:"close", connection_state:"disconnected"}
POST …/whatsapp/qrcode (integração não declarada) → 409 whatsapp_nao_configurado
GET  …/whatsapp/state (integração inexistente)    → 404 not_found
grep da chave da Evolution e do número no log do servidor → 0 ocorrências
```

**Ponta a ponta no Chrome (Playwright, Console servido da pasta `console/dist` em 18341, API em
18331, Evolution falsa em 18198; token lido de arquivo, nunca impresso)**

- Abre no deep link com o painel; sem contagem regressiva de QR.
- Pareamento por número: QR desenhado do `base64` (`naturalWidth` 210), código `ABCD-1234` e rodada.
- O celular "escaneia" (controle da falsa) → "Verificar estado" mostra "Aparelho conectado" e o QR
  sai da tela.
- "Desconectar aparelho" abre a confirmação; confirmar mostra o aviso, a lista passa a
  "Desconectado" e a integração continua cadastrada. A Evolution falsa registrou o logout.
- 409 ao vivo: a integração sem instância declarada mostra o motivo e a dica, sem quebrar a aba.
- Nenhuma mensagem do console do navegador contém o `base64` do QR.

Capturas: `/tmp/hr44d/shots/{qr,conectado,desconectar,desconectado,409}.png`.

**Testes de componente (8 novos; 101 antes, 109 depois)**

`telas/AgenteIntegracoes.test.tsx` passou a cobrir, além dos 6 da HR-44: QR desenhado do `base64`
com `If-Match` e sem corpo; pareamento por número com recusa de formato inválido; "aparelho
conectado" sem QR; `409 whatsapp_nao_configurado` e `503 evolution_indisponivel` explicados sem
quebrar a aba; "Verificar estado" atualizando a tela; desconexão com confirmação, `If-Match` e foco
devolvido ao disparador; `412` do desconectar relendo a versão e mantendo o painel usável.

**Testes de componente da HR-44e (8 novos).** Ainda em `AgenteIntegracoes.test.tsx`: o formulário do
manifesto desenhado com obrigatório e segredo mascarado; tipo sem manifesto avisando e criando só com
tipo e situação; `config_json` com os tipos do esquema e segredo em branco **fora** do corpo; segredo
digitado enviado e obrigatório cobrado antes da API; `400` da API no campo que ela aponta; `412`
relendo a versão e mantendo configuração e segredo digitados; plugin que não respondeu ao handshake
explicado sem deixar salvar. A suíte do Console fechou em **147 testes verdes** (inclui os 28 da aba
Modelo e Jev, HR-45, que entraram em paralelo).

## Decisões desta entrega (HR-44, HR-44d e HR-44e)

- **O formulário é do plugin, não do Console.** Qualquer plugin que fale o protocolo ganha tela sem
  uma linha de código por integração; tipo sem manifesto continua sem formulário.
- **`503` bloqueia o salvamento; `404` não.** Plugin declarado que não respondeu é falha de operação
  (gravar daria `422`); tipo sem plugin é um caso normal (criar só com tipo e situação).
- **Leitura do manifesto com espera de 400 ms ao digitar o tipo.** Sem isso o servidor subiria o
  plugin a cada tecla; na edição (tipo já definido) a leitura é imediata.
- **Configuração parte do que está gravado.** Chaves que o formulário não desenha (objeto aninhado,
  `additionalProperties`) são preservadas ao salvar; só as desenhadas podem ser apagadas, e campo em
  branco sai do objeto.

- **Estado de conexão não é editável na mão.** Quem grava é o canal (webhook) e as rotas de WhatsApp;
  permitir editar `connection_state` faria a tela registrar uma conexão que não existe.
- **Sem contagem regressiva.** A Evolution não informa validade do QR; uma contagem seria invenção.
  No lugar: "Gerar outro QR" e o aviso de que o código gira sozinho.
- **Laço curto só durante `awaiting_scan`** (4 s, até 45 consultas), com parada explícita. Fora
  disso, o estado é lido quando o operador pede ("Verificar estado") ou ao carregar a aba.
- **Desconectar pede confirmação simples, não digitação do nome**: parar o pareamento é reversível
  (gerar outro QR) e não apaga dados; a fricção alta fica para exclusão.
- **Confirmação de exclusão pelo tipo** (não pelo id): o diálogo padrão pede um texto curto e
  legível; quando há duas integrações do mesmo tipo, o diálogo mostra o id para diferenciar.
- **Formulário por esquema entregue como peça testada**, fora da tela, porque falta a fonte do
  esquema (rota de manifesto) — ver acima.
- **Número do WhatsApp é opcional e validado na tela**; o servidor repassa o número à Evolution sem
  validar formato, então a trava é da tela.

## Desvios da UX_SPEC (HR-44 e HR-44d)

| Spec | O que foi feito | Por quê |
|---|---|---|
| Painel de status com número em E.164, latência de webhook e "Ver Logs" | Estado de conexão, instância, id da integração e o campo opcional de número para parear | A API não guarda número do aparelho conectado, latência de webhook nem expõe logs; inventar os campos seria pior que dizer o que falta. |
| Botões "Pausar Atendimento" e "Desconectar Instância" | "Pausar" é a situação (`active`/`inactive`/`archived`); "Desconectar aparelho" é a rota nova | As duas ações agora existem, com nomes que dizem o efeito. |
| Modal de QR com contador de 30 s e código por dígitos | QR com rodada, código de pareamento (quando há número) e botão "Gerar outro QR", sem contador | A Evolution não informa validade; contador de 30 s seria número inventado. |
| Formulário dinâmico do manifesto na tela | Formulário gerado do manifesto do plugin, entregue na HR-44e | O formulário é o do plugin (JSON Schema), sem código por integração; tipo sem plugin declarado continua sem formulário, e a tela diz isso em vez de inventar campos. |

## Limites conhecidos (HR-44 e HR-44d)

- **QR sem validade informada** (média, ao parear): a tela não sabe quando o código vence; quem
  demorar precisa clicar em "Gerar outro QR". O `count` é o único sinal de troca.
- **Instância tem de estar declarada na configuração** (média, ao operar): não há como criar a
  associação integração ↔ instância pela API/Console; sem ela, o painel explica o `409` e não pareia.
- **Laço de estado só com a aba aberta** (baixa): fechou a aba, o estado para de ser consultado; o
  servidor grava o estado que o webhook ou a última consulta trouxeram.
- **`GET …/whatsapp/state` grava no banco** (baixa): a leitura atualiza `connection_state` sem subir
  versão nem auditar — é o desenho da rota, não da tela.
- **Uma instância por integração** (baixa, configuração incomum): se a mesma integração aparecer em
  duas instâncias, a primeira responde.
- **Estado fora do vocabulário aparece como veio** (baixa): a Evolution devolvendo algo diferente de
  `open`/`connecting`/`close` vira `desconhecido` no servidor e a tela não quebra.
- **Formulário por esquema é gerado do manifesto** (média, ao estrear um plugin): a primeira leitura
  de cada tipo sobe o plugin pelo handshake (e a falha fica 30 s em cache); trocar o binário do
  plugin exige reiniciar o servidor. Tipo não declarado em `[plugins].integracoes` não tem
  formulário e só pode ser criado com tipo e situação.
- **Validação é subconjunto do JSON Schema** (média, ao cadastrar): `pattern`, `oneOf`, `allOf` e
  `format` não são checados (na tela nem no servidor); campo fora do esquema é recusado, salvo
  `additionalProperties: true`. Objeto aninhado e lista não são desenhados pelo formulário — são
  preservados como vieram, mas só se editam pela API.
- **`PATCH` de configuração substitui o objeto inteiro** (média, ao operar por API): a tela manda o
  objeto completo (gravado + campos editáveis); uma alteração parcial por fora precisa mandar tudo.
- **Segredo não é verificável pela tela** (baixa, ao operar): a API guarda, mas não testa a
  credencial; saber se ela vale é do plugin no handshake. Remover um segredo obrigatório é recusado
  pelo servidor.
- **Chave de cifra trocada** (alta, ao rotacionar `POPH_CHAVE_CIFRA`): os segredos antigos ficam
  ilegíveis; a tela mostra "gravado, mas a chave atual não consegue lê-lo" e é preciso digitar de
  novo. Sem `POPH_CHAVE_CIFRA`, gravar segredo responde `503 cifra_indisponivel` e nada é gravado em
  claro.
- **Um painel por integração do tipo `whatsapp`** (baixa): com muitas integrações do mesmo tipo a
  página fica longa; a lista da HR-43 também não pagina acima de 200.
- **Lista sem paginação acima de 200** (média): pede `limit=200` e ignora o cursor, como as outras.
- **Exclusão sem desfazer** (média, por desenho): a trava é digitar o tipo; o que for apagado não
  volta.
- **Tipo não normalizado** (baixa): `whatsapp`, `WhatsApp` e `whatsapp ` são tipos diferentes para o
  servidor; a tela tira espaços das pontas, mas não muda a caixa.
- **Estado de conexão pode ficar velho** (baixa): a tela atualiza ao carregar e quando o operador
  pede; fora do laço de `awaiting_scan`, não há consulta automática.
- **Tamanho do bundle** (baixa): 535 kB no build (aviso do Vite, não erro).
- **Sem captura de navegador no CI** (baixa): a prova de tela é o roteiro automatizado acima
  (Playwright local, fora do CI); o CI roda os testes de componente.


## Modelo e Jev do agente (HR-45)

**O que a tela faz.** A aba "Modelo e Jev" (`/orgs/:org/agents/:agent/model-jev`) deixou de ser
"em breve". Ela lê e grava, no mesmo agente das outras abas:

- **Modelo:** provedor, modelo, temperatura (0 a 2), máximo de tokens de saída e máximo de tokens de
  contexto. Temperatura e máximo de saída vazios mostram "herda o padrão do servidor" enquanto o agente
  não tem valor.
- **Atendimento e ferramentas:** janela de agrupamento (0 a 8000 ms), rodadas de ferramenta por turno e
  ferramentas por mensagem.
- **Busca na base (RAG):** top-k (1 a 50) e corte mínimo de relevância (0 a 1).
- **Jev:** o interruptor "Desligar o Jev neste agente" (`ativo: false`) e os oito gates (G1 a G8). Cada
  gate tem três estados: **herdar** (sem chave no agente, vale o padrão de fábrica), **ligado** e
  **desligado**. Cada linha diz em uma frase o que o gate decide e como ele está hoje no agente.
- **Chaves preservadas:** `situacoes_de_handoff`, `despedida`, `resposta_segura`, `g7_acao` e `g7_resposta`
  aparecem como "definida" ou "não definida" e voltam intactas ao salvar. Qualquer chave desconhecida
  também volta intacta.

**Rotas.** Nenhuma rota nova. Leitura: `GET …/agents/{agent_id}`. Gravação: `PATCH …/agents/{agent_id}`
com `If-Match` da versão do agente. O contrato (`docs/API.md`, OpenAPI, `schema.d.ts`) não mudou.

**Como grava.**

- Só vai no corpo o que o operador mudou em relação ao que ele viu. Campo não tocado não é enviado.
- `jev_json` é substituído inteiro pela API. Por isso a tela parte do `jev_json` **atual** (relido depois
  de um `412`) e mexe só nos gates e no interruptor. Gate que muda é gravado dentro de `gates` (formato
  de `docs/JEV.md`), e a cópia dele na raiz do `jev_json` sai, para não voltar a valer por trás da tela.
- Gate voltando a "herdar" tira a chave dele. Se `gates` ficar vazio, a chave sai. As outras chaves ficam.
- Validação de faixa acontece na tela, antes de chamar a API. Erro de campo que a API devolve aparece no
  campo; os demais aparecem com código e mensagem reais.

**`412` (versão desatualizada).** A tela relê o agente, passa a usar a versão atual e o `jev_json` atual,
e **mantém o que foi digitado**. Ela também mantém a base que o operador viu, então o próximo salvamento
manda só a diferença. Não desfaz a mudança de quem salvou antes em outros campos.

**Efeito no turno.** A alteração vale no **próximo turno**, sem reiniciar. O turno confere a versão do
agente antes de cada mensagem (`carregar_verificando`); quando a versão mudou, relê o agente e usa os
valores novos. Provado: teste `mudanca_de_versao_invalida_o_cache` (`poph-agent`) e a versão mudando a
cada gravação na prova de ponta a ponta. **Não provado:** um turno real com o modelo de texto.

**Padrão de fábrica.** O texto "herda o padrão de fábrica" usa o padrão do código: G3, G5 e G8 ligados;
G1, G2, G4, G6 e G7 desligados (`GatesJev::padrao()`). A configuração do servidor (`poph.toml`) pode ter
mudado isso, e a tela não consegue ler essa configuração.

**Paridade com a CLI (provada nos dois sentidos).**

- Tela → CLI: depois de gravar modelo `gemini-412-teste`, temperatura 0,4, top-k 5 e os gates G2 e G8, a
  CLI `poph agent show <agente> --json` mostrou os mesmos valores, versão 8, e o `jev_json` com as chaves
  preservadas.
- CLI → tela: `poph agent update <agente> --if-match 8 --rag-top-k 4 --max-tool-calls 6 --jev-json '…'`
  (versão 9). A API e a tela recarregada mostraram top-k 4, ferramentas 6, temperatura 0,4, G6 ligado e o
  interruptor desligado. Com versão velha, a CLI recusou com conflito (esperada 7, atual 9).

**Desvios da UX_SPEC (aba 7).**

- **Limiares de confiança e margem por gate não aparecem na tela.** A UX_SPEC traz números (por exemplo,
  G6 0,80 e G8 0,80, e corte padrão do RAG 0,70). O código usa limiares fixos (G1, G4, G5 e G7 em 0,7; G6
  em 0,5; G8 em 0,8) e não há rota que os exponha. O corte padrão do RAG é 0 (`docs/DECISOES.md`). A tela
  não mostra nenhum desses números, para não apresentar como valor atual o que não é.
- **"Provedor" é campo de texto livre.** A UX_SPEC fala em "antigravity2api local". A API aceita qualquer
  texto no agente; a configuração global só aceita `antigravity2api`. A tela mostra a dica, sem bloquear.
- **Faixas.** As faixas da tela são as que o banco impõe (restrições do esquema de `agents`): janela
  0 a 8000 ms, top-k 1 a 50, corte 0 a 1, rodadas de ferramenta 0 a 20, ferramentas por mensagem 0 a 100 e
  contexto a partir de 1000. Temperatura 0 a 2 segue a regra da configuração global (o banco não impõe).
  Máximo de tokens de saída só exige 1 ou mais: escolha desta tela, a UX_SPEC não traz faixa.

**Limites conhecidos (HR-45).**

- **Média, API (mensagem de erro errada):** quando um valor sai da faixa do banco (janela 9000 ms, top-k 51,
  corte 1,5, contexto abaixo de 1000), a API responde 400 com "campo `slug` inválido: já existe um registro
  com esse valor nesta org". A causa é `poph-store` (`da_escrita`): qualquer violação de restrição vira
  "índice único repetido" no campo fixo `slug`. O valor é recusado (correto), só a mensagem engana. A tela
  não chega a enviar esses valores, mas quem usa a API ou a CLI vê a mensagem errada. Correção: mapear só
  `UNIQUE` para conflito e tratar `CHECK` como validação do campo certo. Fora do terreno do Console.
- **Baixa, API:** temperatura não tem restrição no banco: a API aceitou 3,0 (confirmado). A tela limita a
  0 a 2; quem usa a API ou a CLI passa por cima.
- **Média, API:** temperatura e máximo de saída não voltam a "herda o padrão do servidor" depois de
  definidos (a gravação usa `COALESCE`, e não há como mandar `null`). A tela avisa e não finge apagar. É o
  mesmo desenho do departamento padrão, já aceito pelo mantenedor em 20/11.
- **Média, configuração:** "herda o padrão de fábrica" usa o padrão do código. Se o `poph.toml` do servidor
  mudou os gates, a tela não percebe.
- **Média, configuração:** o interruptor global `[jev].ativo` e a configuração global dos gates não aparecem
  na tela. Um agente com gate ligado não roda se o Jev global estiver desligado, e a tela não diz isso.
- **Baixa, após 412:** os campos que a outra pessoa mudou continuam mostrando o valor antigo até recarregar
  a página. O salvamento seguinte não os desfaz.
- **Baixa:** `despedida`, `resposta_segura`, `g7_acao` e `g7_resposta` só aparecem como definida ou não
  definida. Editar exige tarefa curta.
- **Baixa:** o efeito de cada gate no turno real (com o modelo de texto) não foi medido aqui; a prova é da
  gravação e da releitura, não da resposta do atendente.

## Chat de teste do agente (HR-46)

**O que a tela faz.** A aba "Chat de teste" (`/orgs/:org/agents/:agent/test-chat`) deixou de ser "em
breve" (menu e barra de abas). Ela conversa com o agente pelo mesmo turno do WhatsApp, numa conversa de
teste com memória zerada (`channel = "test"`, sem integração):

- **Lista e nova conversa.** Lista à esquerda, mais recentes primeiro, com id curto, situação (Ativa ou
  Transferida), data e contagem de mensagens. "Nova conversa" cria uma conversa com memória zerada e abre
  na hora. "Carregar mais conversas" aparece quando há próxima página.
- **Histórico.** Balões do cliente (à direita) e do agente (à esquerda), com os anexos de cada mensagem
  (nome, tamanho e se há texto extraído).
- **Envio.** Texto, arquivos ou os dois. Enter envia; Shift+Enter quebra a linha. Anexos aceitos: imagem
  (JPG, PNG, GIF, WebP), PDF e Office (DOC, DOCX, XLS, XLSX, PPT, PPTX), até 16 MB por arquivo. Arquivo
  acima disso é recusado na tela, sem chamar a API. Vazio (sem texto e sem arquivo) também não envia.
- **Streaming.** O texto do agente aparece conforme chega (cada `texto` acrescenta um pedaço, na ordem).
  Ferramentas usadas aparecem como "Usando a ferramenta …", a base consultada como "Base consultada: N
  trecho(s)" e a transferência como aviso no balão. No `fim`, a tela relê o histórico e mostra "Turno
  concluído" com tokens de entrada e saída e o tempo total. `failed` mostra o motivo; `skipped` avisa
  que a IA não respondeu.
- **Cancelar.** Enquanto o turno roda, o botão "Cancelar turno em andamento" aborta a conexão. O servidor
  cancela pela queda da conexão (não há rota própria). A tela avisa "Turno cancelado", diz que a mensagem
  ficou registrada e que a resposta não foi gerada, e volta a aceitar mensagem.
- **Transferida.** Conversa com status `transferred` mostra o aviso "Atendimento transferido para uma
  pessoa", bloqueia o envio e oferece "Reabrir atendimento da IA nesta conversa".
- **Trocar de conversa.** Clicar em outra conversa no meio de um turno aborta o turno da anterior (o
  servidor cancela) e limpa o rascunho e os anexos. Nada do turno antigo entra na conversa nova: as
  respostas de um turno só são aplicadas se a conversa ainda é a da tela.

**Rotas.** Nenhuma rota nova. Usa as cinco de `docs/API.md` ("Chat de teste do Console (HR-22)"):
criar, listar, histórico, enviar (`multipart/form-data` → `text/event-stream`) e reabrir.

**Como lê o fluxo.** O envio é um `POST` com corpo `FormData` (campo `text` quando há texto e `file` por
arquivo). A resposta é lida com `fetch` e `response.body`, não com `EventSource` (que só faz `GET`). O
leitor quebra o fluxo em quadros `event:`/`data:` e entrega cada evento na ordem. Um quadro que chega
partido entre dois pedaços espera a linha em branco que o fecha. `ping` e comentários são ignorados.

| Evento | O que a tela faz |
| --- | --- |
| `mensagem` | confirma o que foi gravado (nada a mais na tela) |
| `inicio` | abre o turno (nada a mais na tela) |
| `texto` | acrescenta `delta` ao balão do agente |
| `base` | mostra a quantidade de trechos da base |
| `ferramenta` | mostra "Usando a ferramenta <nome>" |
| `transferencia` | avisa no balão e, depois do `fim`, a conversa aparece como transferida |
| `fim` | guarda estado, tokens e tempo; `failed` e `skipped` viram aviso |
| `erro` | mostra a mensagem no balão do agente |

**Erros e o que a tela mostra.**

- `409 turno_em_andamento`: "Esta conversa já tem uma resposta em andamento". O texto e os arquivos
  voltam ao campo, porque nada foi gravado.
- `503 turno_indisponivel`: "O turno não está ligado neste servidor" (falta a chave do modelo). Nada é
  gravado; o texto volta ao campo.
- `400 validation_error` com `details.field` = `file` ou `text`: a mensagem da API aparece no campo
  certo; o texto e os arquivos voltam ao campo, porque nada foi gravado.
- Falha de rede ou outro erro antes do turno: aviso com a mensagem real, e o texto volta ao campo.
- `412` no reabrir: "A conversa mudou em outro lugar". A tela relê a conversa (com a versão nova), e o
  próximo "Reabrir" manda essa versão no `If-Match`. O que foi digitado continua na tela.
- `401` em qualquer chamada: volta ao login, como nas outras abas.

**Como reabre.** "Reabrir atendimento da IA nesta conversa" manda `POST …/reopen` com `If-Match` da
versão da conversa lida no histórico. Sem o cabeçalho a API responde `428`; a tela sempre manda.

**Arquivos que o cliente mandou (HR-46c).** Cada anexo do cliente no histórico tem botões com o nome do
arquivo no rótulo (por exemplo "Ver laudo.png" e "Baixar laudo.png"):

- **Ver** (só imagem: JPG, PNG, GIF, WebP) abre uma prévia numa janela centralizada sobre a conversa, sem sair dela (HR-49b: era gaveta lateral). A imagem cabe no modal, até 80% da largura e da altura da janela. Mostra
  "Carregando…", a imagem ou o erro com "Tentar de novo". `Esc` fecha e o foco volta ao botão.
- **Baixar** (imagem, PDF e Office) baixa o arquivo com o nome do cabeçalho `Content-Disposition`
  (`filename*` em UTF-8; sem ele, o nome do histórico).
- **De onde vem.** `GET …/test-chat/{conversa}/messages/{mensagem}/attachments/{anexo}/content` (HR-46b),
  chamada pelo mesmo cliente HTTP das outras telas, com `Authorization: Bearer`. Não se usa `<img src>`
  nem `<a href>`, porque essas tags não mandam o cabeçalho. Os bytes chegam como `Blob`.
- **Erros.** 401, 403 e 404 aparecem na linha do anexo (ou na prévia) com a mensagem da API e "Tentar de
  novo". A conversa continua na tela. Nenhum conteúdo do arquivo entra em console, log ou mensagem de erro.
- **Liberação.** A URL `blob:` de um download é revogada ao trocar de conversa, ao sair da tela e um
  minuto depois. A prévia usa `data:` (ver limites), então não há URL a revogar.
- **Decisões.** PDF e Office só têm "Baixar": abrir em outra aba depende de bloqueador de pop-up e do
  visualizador do navegador. Trocar de conversa fecha a prévia aberta e não deixa resposta de download
  antiga aparecer na conversa nova.

**Arquivos.** `console/src/api/arquivoTeste.ts` (caminho, nome do cabeçalho e prévia),
`console/src/componentes/PreviaArquivoTeste.tsx` (a prévia) e `console/src/telas/AgenteChatTeste.tsx`
(botões e liberação). Em `console/src/api/cliente.ts`, `pedirBytes` faz o `fetch` com o token e devolve
os bytes.

**Provas da HR-46c.**

- Testes do Console: `api/arquivoTeste.test.ts` (9) e 8 casos novos em `telas/AgenteChatTeste.test.tsx`
  (ver, baixar com nome do cabeçalho, 403 e 404 com "Tentar de novo", `Esc` e foco, resposta que chega
  depois de fechar a prévia, troca de conversa soltando a URL, sair da tela soltando a URL).
- Navegador (Chrome do sistema, Playwright, servidor `poph serve` real, banco de teste): 19 de 19
  conferências. Entrada pelo token; "Ver" só na imagem; prévia com a imagem de 240 px; `Esc` fecha e
  devolve o foco; 404 interceptado mostra o motivo e "Tentar de novo" sem derrubar a conversa; download
  da imagem e do PDF com sha256 igual ao original; duas URLs `blob:` criadas e revogadas ao trocar de
  conversa; console sem base64 de imagem, texto de PDF nem token.
- HTTP (curl, mesmo servidor): o `/content` devolveu os mesmos bytes do envio (sha256 conferido nos dois
  lados), com `inline` para imagem e PDF; sem token, 401; id de anexo válido e inexistente, 404.

**Limites da HR-46c.**

- **Média, prévia de imagem:** usa `data:` porque a política de conteúdo do Console libera
  `img-src 'self' data:` e não `blob:`. Usar `blob:` exige mudar essa política no servidor
  (`crates/poph-server/src/console.rs`), fora desta frente.
- **Média, PDF e Office:** só baixam; não há visualização dentro da tela.
- **Baixa, arquivo grande:** o download carrega o arquivo inteiro na memória (até 16 MB), sem retomada.
- **Baixa, 404 da prova:** o 404 do navegador foi interceptado no teste (o arquivo não foi removido do
  disco). O 404 de arquivo removido do disco foi provado na HR-46b.
- **Baixa, avisos do navegador:** quando uma resposta dá 404, o navegador escreve um aviso genérico no
  console ("Failed to load resource"). Ele não traz o conteúdo do arquivo.

**Arquivos que o agente enviou (HR-46e).** O turno do agente mostra os arquivos que ele enviou ao
cliente, com o nome, o tamanho e o estado do envio:

- **Onde aparece.** Na resposta do agente, logo abaixo do texto, na lista "Arquivos enviados pelo
  agente". Quando o turno não deixou texto (falhou, ou terminou sem resposta), a lista aparece na linha
  "Turno #N" do registro.
- **Estado do envio.** "enviado" (saiu), "na fila para envio" (`pending` ou `sending`) e "falhou ao
  enviar" (`failed`, em vermelho). O estado não bloqueia o download: um arquivo que falhou ainda pode ser
  baixado.
- **Ver e Baixar.** Mesmos botões do arquivo do cliente (HR-46c): "Ver <nome>" para imagem (prévia sobre
  a conversa, com `Esc` e foco devolvido) e "Baixar <nome>" para imagem, PDF e Office, com o nome do
  cabeçalho `Content-Disposition`.
- **De onde vem.** Cada item de `files_out` do turno (HR-46d) traz o `arquivo_id`. O download sai por
  `GET …/agents/{agent}/files/{arquivo_id}/content` (HR-46b), com `Authorization: Bearer` pelo mesmo cliente
  HTTP, e não por `<img src>` nem `<a href>`.
- **Quando aparece.** A lista vem da leitura do histórico. Quando o turno termina (`fim`), a tela relê a
  conversa, então os arquivos do turno que acabou aparecem logo depois. Enquanto o turno roda, a resposta
  mostra só o texto.
- **Erros.** Os mesmos do HR-46c: 401, 403 e 404 aparecem na linha do arquivo com "Tentar de novo". Um 404
  acontece quando o arquivo saiu da biblioteca ou o conteúdo sumiu do disco; a conversa continua na tela e
  nada do conteúdo ou do caminho aparece em console ou mensagem.

**Arquivos.** `console/src/api/arquivoTeste.ts` (origem do arquivo do cliente e do agente, rótulo do
estado e download), `console/src/telas/AgenteChatTeste.tsx` (lista do agente no turno e no turno sem
resposta). Os tipos de `files_out` estão em `console/src/api/testeChat.ts`.

**Provas da HR-46e.**

- Testes do Console: `api/arquivoTeste.test.ts` ganhou casos de caminho, origem e rótulo do estado; e
  `telas/AgenteChatTeste.test.tsx` ganhou 6 casos do arquivo do agente (baixar com o token e o nome do
  cabeçalho; imagem em prévia com "enviada pelo agente"; envio que falhou rotulado num turno sem texto;
  turno sem arquivos sem lista; 404 com "Tentar de novo" sem derrubar a conversa; troca de conversa
  soltando a URL do download).
- Mutação: trocar o `arquivo_id` pelo id do item da fila reprova 6 testes; arquivo restaurado e conferido
  com `cmp`.

**Limites da HR-46e.**

- **Média, conversa longa:** o histórico devolve no máximo 50 turnos. Turno fora dessa janela não aparece,
  e os arquivos dele também não.
- **Baixa, motivo da falha:** "falhou ao enviar" não traz o motivo técnico, porque o item da fila não grava
  essa causa.
- **Baixa, arquivo grande:** o download carrega o arquivo inteiro na memória (até 16 MB), como no anexo do
  cliente.

**O que a tela não faz (e por quê).**
- **Prévia da última mensagem na lista.** A listagem não devolve o texto da última mensagem; a lista mostra
  o id curto, a situação, a data e a contagem.
- **Painel do porquê, "Ver por quê" por turno e o atalho "Chat de teste ↗" no cabeçalho do agente.**
  Entregues na HR-47 (seção própria, mais abaixo).
- **Atalhos de teclado além de Enter e do foco nos botões** (por exemplo Alt+N) não entraram.

**Decisões desta entrega.**

- Enter envia e Shift+Enter quebra a linha: é o padrão de um chat, e o botão Enviar continua acessível
  pelo Tab.
- Trocar de conversa no meio do turno **cancela** o turno da conversa anterior. A alternativa (deixar
  rodando em segundo plano) deixaria o operador sem como cancelar e mostraria eventos de uma conversa em
  outra tela. Como o servidor cancela pela queda da conexão, nada fica pendente.
- Conversa transferida bloqueia o envio na tela (como manda a UX_SPEC §3.6). O servidor, que já recusa
  resposta da IA nessa situação, não é alterado.
- A lista carrega a primeira página de 50 conversas; as demais entram com "Carregar mais".

**Arquivos.** `console/src/api/testeChat.ts` (contrato da aba, leitor do fluxo e `pedirFluxo`),
`console/src/telas/AgenteChatTeste.tsx` (tela), `console/src/estilos/chat-teste.css`. Ajustes pontuais:
`console/src/api/cliente.ts` (`pedirFluxo` e a tradução de erro compartilhada com `pedir`),
`console/src/App.tsx` (rota), `console/src/componentes/AbasDoAgente.tsx` (aba), `navegacao.ts` (item do
menu), `Trilha.tsx` (migalha). Testes: `api/testeChat.test.ts` (6) e `telas/AgenteChatTeste.test.tsx`
(14); os de navegação e de abas (`navegacao.test.tsx`, `Agentes.test.tsx`) foram ajustados para a aba e o
item novos.

**Provas.**

- Testes do Console: 252 no total, todos verdes (`pnpm run testar`, 24 arquivos); 20 são novos (14 da tela e 6 do leitor do fluxo).
- HTTP contra `poph serve` real (banco `/tmp/hr46`, provedor OpenAI-compatível falso com SSE): criar (201),
  texto com PDF em streaming (`mensagem`, `inicio`, `texto` ×3, `fim` com `done`; o texto do PDF chega ao
  prompt marcado como não confiável), histórico com o anexo `procedimento.pdf` (`extraction_status` ok),
  segunda aba em voo (409 `turno_em_andamento`), queda da conexão (turno `failed`, nenhuma resposta na
  fila), reabrir (`If-Match` velho = 412, atual = 200, sem = 428), mensagem seguinte responde, token de
  outra org (403 no histórico, envio e reabrir).
- Navegador: entrada pela tela de login, aba "Chat de teste", lista com as conversas, "Nova conversa"
  (conversa criada e aberta com a mensagem de vazio) e a tela renderizada. O envio pela tela ficou
  pendente de aprovação do operador (a ferramenta de navegador pede aprovação para enviar), então o
  streaming pela tela não foi visto no navegador; o mesmo caminho foi provado por HTTP acima.

**Limites conhecidos.**

- **Média, transferência real:** a transferência não foi disparada no servidor de teste (depende de o G1
  decidir, e o provedor falso não produz isso). Transferida e reabrir foram provados com resposta
  simulada nos testes de componente e, no servidor real, só o reabrir (HTTP, `If-Match`).
- **Média, streaming pela tela:** o caminho visual do streaming e do cancelamento foi provado com fluxo
  simulado nos testes de componente; no navegador, só até a criação da conversa (ver "Provas").
- **Baixa, lista:** sem prévia da última mensagem e sem busca; primeira página de 50.
- **Baixa, campo de arquivo:** o seletor nativo do navegador mostra o texto em inglês ou português
  conforme o idioma do sistema ("Escolher arquivos", "Nenhum arquivo escolhido").
- **Baixa, mensagem de cancelamento:** a tela diz que a resposta não foi gerada; o servidor não diz o
  motivo ao cliente (é a queda da conexão), então o aviso não inclui o motivo técnico.

## Painel do por quê por turno (HR-47)

**O que a tela faz.** Em cada resposta do agente no chat de teste aparece o botão "Ver por quê (turno N)".
Turno que não deixou resposta (falhou, foi cancelado ou não respondeu) aparece no registro como uma linha
própria, com o estado e o erro gravado, e também tem o botão. O botão abre um painel lateral sobre a
conversa (`SofaDrawer` do design system): foco preso, `Esc` fecha, e o foco volta ao botão que abriu.

O painel mostra, nesta ordem:

- **Resumo do turno:** número, estado, início, duração, tokens do modelo (entrada e saída) e custo.
- **Erro registrado,** quando o turno gravou um.
- **Decisões do Jev por gate:** a chamada (perguntas, quantas vieram do cache, tempo, tokens, falha) e,
  por gate, a escolha que valeu, a escolha que o Jev deu, a origem, a confiança, a margem sobre a segunda
  opção, o motivo quando caiu no padrão, e o tempo e os tokens do lote. Revisão do G6 quando houve.
- **Base e catálogo usados:** trechos com título, grupo, similaridade e relevância (o texto fica em
  "Ver o texto do trecho"); produtos com o preço que o cliente viu.
- **Ferramentas chamadas:** uma linha por chamada, na ordem em que aconteceram. Cada chamada mostra o nome,
  a rodada e a posição (`rodada 1, chamada 2`), e um rótulo de estado: **Executada** (verde) ou **Recusada, não
  rodou** (laranja). A recusa traz o motivo em português (não habilitada neste agente; ferramenta desconhecida;
  argumento inválido; falha ao executar; passou do limite de chamadas nesta mensagem; fora do escopo deste
  turno). Depois, duração, argumentos e o resultado que voltou ao modelo; numa recusa, esse resultado é o erro
  que o modelo recebeu, com o rótulo "Erro devolvido ao modelo".
- **Tempos por etapa:** fila, montagem do contexto, até o primeiro texto e total (barras proporcionais ao
  total), rodadas de ferramenta, modelo e motivo do fim.

**De onde vem.** Só da leitura da conversa que a aba já faz (`turns[]` de `GET …/test-chat/{conversa}`).
Nenhuma rota nova. As chamadas de ferramenta vêm de `tool_calls` (registro da HR-47b, em `docs/API.md`); a
lista de nomes em `tools` continua a mesma e só serve de reserva para turno gravado antes da HR-47b. Ao abrir
o painel, a conversa é relida; se a leitura falhar, o painel mostra o erro com "Tentar de novo" e mantém o que
já estava na tela.

**Não gravado hoje, e a tela diz isso em vez de inventar.** O texto da pergunta enviada ao Jev e as
probabilidades de cada opção; a estrutura completa do retorno de cada ferramenta (fica só o texto que voltou
ao modelo, cortado em 1.024 caracteres); o tempo da busca na base separado do resto da montagem; a indicação
de corte por trecho; o link para a fonte oficial; e o custo (o turno não calcula custo: `cost_micros` sai 0).

**Decisões desta entrega.**

- Zero não é medição. Custo 0 vira "não calculado neste turno", e tokens 0 nos dois lados viram "não
  registrado" (é o padrão de quem não chegou a usar o modelo).
- Gate que caiu no padrão diz "Valeu o padrão do gate; a resposta do Jev não foi usada". O Jev pode ter
  respondido, mas abaixo do limiar ou com uma opção que o gate não reconhece; o motivo aparece ao lado.
- O tempo e os tokens de gate são do lote inteiro. Gates decididos no mesmo lote mostram o mesmo valor, e
  a tela avisa isso.
- O painel é modal. Enquanto está aberto, a conversa fica visível atrás, mas não recebe cliques. O turno em
  curso não para: o texto continua chegando e aparece quando o painel fechar.
- Turno sem resposta entra no registro pelo horário de início, entre as mensagens. A mensagem do cliente
  não guarda o número do turno, então o encaixe é pelo horário.
- "Turno N" é a posição na leitura, que traz no máximo 50 turnos (os mais antigos primeiro). Conversa com
  mais de 50 turnos não mostra os seguintes; é o limite já existente do histórico.
- A rodada aparece a partir de 1 ("rodada 1" é a primeira resposta do modelo), porque a API conta a partir
  de 0. Duração zero aparece como "menos de 1 ms": o turno mediu e arredondou para baixo, não é "não medido".
- Turno gravado antes da HR-47b (tem `tools` e não tem `tool_calls`) mostra só os nomes, com a nota de que
  estado, motivo, duração, argumentos e resultado não foram gravados. Turno sem chamada e sem `tools` mostra
  "Nenhuma ferramenta foi chamada neste turno".
- Recusada usa o tom de atenção, o rótulo "Recusada, não rodou" e a marca lateral laranja: não depende só da
  cor. Executada usa o tom de sucesso.

**Provas.**

- Testes de camada de leitura (`api/porQue.test.ts`, 21), de componente do painel
  (`componentes/PainelPorQue.test.tsx`, 17) e da página com o "por quê" (`telas/AgenteChatTestePorQue.test.tsx`,
  5). Suíte do Console: 322 testes em 28 arquivos, todos verdes (`console/scripts/testar.sh`).
- Mutações, com o arquivo restaurado e conferido com `cmp`: fazer aparecer "0 ms" quando o tempo não foi
  gravado reprova 1 teste; deixar de mostrar o turno sem resposta reprova 1 teste. Na HR-47c, mostrar uma
  recusada como "Executada" reprova 1 teste de componente; trocar a tradução de `nao_habilitada` reprova 2 testes
  de componente e 1 de leitura.
- Ponta a ponta na HR-47c, com `poph serve` real (banco próprio, token reaproveitado de rodada anterior, provedor
  OpenAI-compatível falso pedindo `enviar_arquivo`, que estava habilitada, e `cart_add_item`, que não estava):
  o histórico trouxe as duas chamadas com estado, motivo e os segredos redigidos. No navegador, 22 de 22
  conferências: enviar_arquivo como Executada, cart_add_item como Recusada com o motivo traduzido, argumentos com
  `[redigido]`, nenhum segredo no texto do painel nem no console, `Esc` fecha e devolve o foco. Captura do painel
  mostrada no chat.
- Ponta a ponta com `poph serve` real, base de conhecimento indexada pelo modelo de embedding instalado,
  produto e categoria ligados ao agente, LLM e Jev falsos com escolhas fixas. O turno concluído mostrou
  Jev (G3 respondido pelo Jev; G5 pelo padrão, porque a resposta não bateu com os candidatos), trecho "Prazo de entrega" (grupo Atendimento, similaridade 56%), produto
  "Dedetização de baratas" (R$ 450,00), as duas ferramentas e os tempos. O turno cancelado mostrou o erro
  e a linha própria no registro.
- Navegador (Chrome do sistema, Playwright): 14 conferências em tela, incluindo `Esc` com foco de volta
  ao botão e nenhum conteúdo sensível no console do navegador. Capturas em `/tmp/hr47/shots/`.

**Limites conhecidos.**

- **Média, dados não gravados:** probabilidades, texto da pergunta do Jev, estrutura completa do retorno de
  cada ferramenta, tempo da busca separado, corte por trecho, fonte oficial e custo. A tela declara cada um.
- **Baixa, texto cortado:** argumentos e resultado chegam cortados em 1.024 caracteres; um JSON longo aparece
  com `…[cortado]` no fim.
- **Baixa, redação por nome de campo:** um segredo dentro de um valor de campo com nome comum (ex.: uma frase
  com "meu token é…") não é redigido na gravação; o painel mostra o texto como veio. Vale para argumentos, que
  vêm do modelo.
- **Baixa, motivo de argumento inválido:** a classificação vem da mensagem de erro da ferramenta; mudar esse
  texto no servidor muda a classificação mostrada.
- **Média, Jev:** a prova usou um Jev falso com escolhas fixas. O Jev real não foi chamado.
- **Baixa, custo:** `cost_micros` é sempre 0 hoje; a tela mostra "não calculado" até o turno calcular custo.
- **Baixa, similaridade:** o cosseno aparece em porcentagem e pode ser lido como probabilidade.
- **Baixa, limite de 50 turnos** na leitura (ver acima).
- **Baixa, modal:** a conversa fica inerte enquanto o painel está aberto.

## Saúde da instância (HR-48a, HR-48c)

**O que a tela faz.** Entrada "Saúde da instância" no menu (grupo SISTEMA), com a organização aberta na URL
(`/orgs/:org/operacao`). A tela responde, em linguagem de operador, se o servidor está no ar, em que versão, há
quanto tempo, como estão as filas e quanto de recurso ele usa, e quais plugins a instância declara. São quatro
seções independentes: uma falha em uma não derruba as outras.

- **Servidor:** situação (no ar com os bancos respondendo, ou no ar com um banco sem resposta), versão do
  servidor, tempo no ar, e a versão de esquema do banco de conversas e da base de busca. Versão `0` aparece como
  "sem versão gravada ainda": o servidor manda `0` quando o arquivo ainda não foi migrado (ou não pôde ser lido).
- **Filas e recursos, agora:** turnos esperando na fila, mensagens esperando envio, conversas com atendimento em
  curso, agentes carregados na memória, memória usada pelo processo e tamanho do banco de conversas e da base de
  busca.
- **Filas e recursos, desde a última subida:** chamadas ao modelo, ao Jev e à Evolution, com o número de erros de
  cada uma, e tokens consumidos. São contadores do processo atual: zeram quando o servidor reinicia.
- **Plugins declarados (HR-48c):** lista os plugins que a instância declara em `[plugins].integracoes`, com a
  situação, o nome e a versão de cada um.
  - **No ar:** selo "No ar", nome, tipo e versão, e o botão "Ver detalhes do plugin <tipo>". O detalhe mostra nome,
    tipo, versão e a quantidade de campos de configuração e de segredo. Abre pela linha da lista (clique ou Enter)
    e fica abaixo dela.
  - **Indisponível:** selo "Indisponível" em texto (não só cor), o tipo e a mensagem do servidor. Não tem ação de
    detalhe, porque a consulta daquele tipo também falharia. Os outros plugins seguem na lista.
  - **Lista vazia:** explica que os plugins se declaram em `[plugins].integracoes` da configuração. Não é erro.

**De onde vem (rotas que já existem, nenhuma nova):**

| O que a tela mostra | Rota | Token |
| --- | --- | --- |
| Servidor (situação, versão, tempo no ar, esquemas) | `GET /healthz` | não precisa |
| Filas e recursos | `GET /metrics` (texto Prometheus; a tela lê só os nomes que explica) | não precisa |
| Lista de plugins declarados (HR-48c) | `GET /admin/v1/orgs/{org}/manifests` (HR-48b) | sim (token da org ou global) |
| Detalhe de um plugin | `GET /admin/v1/orgs/{org}/manifests/{kind}` | sim (token da org ou global) |

O `/healthz` responde `503` com o mesmo corpo quando um banco não responde; a tela trata os dois casos como
"no ar", com o aviso do banco. O `/metrics` não traz histogramas para a tela: a latência por etapa do turno não
aparece aqui. A lista não lê segredo, caminho de programa, argumentos nem configuração: só nome, versão e situação.

**Estados.** Cada seção tem carga (esqueleto), erro com a frase do que aconteceu e "Tentar de novo", e vazio
(métrica que a instância não publicou aparece como "não informado", nunca como zero). Erro numa atualização
depois de já ter dados mantém os últimos dados e mostra o aviso "A última atualização falhou". Na lista de
plugins, a primeira consulta mostra "Consultando plugins…", porque o servidor sobe cada plugin no primeiro
pedido. Mensagens:

- **401:** a sessão não vale mais; entrar de novo.
- **403:** o token não alcança esta organização.
- **404** (no servidor): não encontrado. No detalhe de plugin, o `404` quer dizer "este tipo não tem plugin
  declarado".
- **500 ou sem conexão:** a instância não respondeu; "Tentar de novo".
- **503 no detalhe de plugin:** o plugin está declarado, mas não respondeu ao teste de conexão.

**Atualização.** Atualiza sozinha a cada 15 s, só com a aba visível (sem laço de requisições). A lista de plugins
entra nesse ciclo: depois da primeira consulta, ela é leitura de memória. O botão "Atualizar" faz o mesmo na hora e
desabilita enquanto uma consulta está em curso. O texto "Atualizado às HH:MM" avisa quando foi a última atualização
bem-sucedida.

**O que a API ainda NÃO oferece:**

- **Instalar e remover plugin:** nenhuma rota. A tela não tem ação para isso.
- **Forçar nova leitura do manifesto:** nenhuma rota; reiniciar o servidor é o caminho (ver limites).
- **Latência por etapa do turno:** é um histograma no `/metrics`; a tela ainda não desenha.

**Testes de componente.** `console/src/api/operacao.test.ts` (leitura do `/metrics` em texto, `200` e `503` do
`/healthz`, erro de conexão, `500`; e a lista de plugins: ordem, plugin indisponível com e sem mensagem, item sem
tipo descartado, lista vazia, `403`) e `console/src/telas/Operacao.test.tsx` (saúde OK e degradada, erro de uma
seção sem derrubar a outra, "Tentar de novo" refazendo a consulta, métricas vazias sem zero inventado, botão
Atualizar, atualização automática a cada 15 s só com a aba visível, lista com um no ar e um indisponível sem botão
de detalhe, lista vazia, carregando, `403` na lista com "Tentar de novo" que recupera, detalhe aberto pela linha,
detalhe com `503`). O fetch falso (`ServidorFalso`) ganhou resposta em texto puro para o `/metrics`.

**Provas (HR-48a).**

- `npx vitest run` no Console: 349 testes verdes em 30 arquivos (eram 322).
- `tsc --noEmit` limpo; `console/scripts/build.sh` gerou o bundle e `cargo build -p poph` o embutiu.
- Servidor real (`poph serve`, porta 18811, banco em `/tmp/hr48a-run/dados`, token reaproveitado de
  `/tmp/hr46b` em arquivo 0600, sem gerar credencial nova). Pelo `curl`:
  - `GET /healthz` → `200`, `{"status":"ok","version":"0.1.0",…,"databases":{"harness":2,"rag":0}}`;
  - `GET …/manifests/eco` → `200` (plugin-eco real, declarado em `[plugins].integracoes`);
  - `GET …/manifests/whatsapp` → `404 not_found`;
  - `GET …/manifests/quebrado` → `503 manifesto_indisponivel` (programa `/bin/false` declarado de propósito);
  - `GET …/manifests/eco` sem token → `401`.
- Navegador (Chrome do sistema, Playwright, roteiro `/tmp/hr48a-run/e2e.mjs`): **20 de 20** conferências. O menu
  leva à tela; saúde "No ar, com os bancos respondendo"; versão `0.1.0`; base de busca "sem versão gravada ainda"
  (o `rag.db` não foi migrado nesta instalação); métricas sem "não informado"; "Atualizar" refaz a consulta;
  plugin `eco` com nome e versão; `whatsapp` com o `404` explicado; `quebrado` com o `503` explicado e
  "Tentar de novo"; campo em branco avisa sem chamar a API. Nenhuma resposta fora do esperado: as únicas não-200
  foram o `404` do `whatsapp` e o `503` do `quebrado`. O token não aparece no console do navegador.
  Capturas: `/tmp/hr48a-run/shots/01-operacao-saude-metricas.png`, `02-operacao-plugin-eco.png`,
  `03-operacao-plugin-quebrado.png`.

**Provas (HR-48c).**

- `npx vitest run` no Console: **356 testes verdes em 30 arquivos**. Desses, 7 são da lista de plugins na tela e
  5 da camada de API. Os 5 testes antigos de "digitar o tipo" saíram junto com o campo, porque a lista é o caminho
  agora; a consulta por tipo continua, acionada pela linha.
- `tsc --noEmit` limpo; `console/scripts/build.sh` gerou o bundle (15 arquivos) e `cargo build -p poph` o embutiu.
- Mutação: tratar a situação `indisponivel` como `ok` na camada de API reprova 3 testes (2 de API e 1 de tela).
  Arquivo restaurado e conferido com `cmp`.
- Servidor real (`poph serve`, porta 18830, banco em `/tmp/hr48c-run/dados`, com `eco` e `quebrado` declarados).
  Token: par reaproveitado da rodada HR-48a, sem `poph token global`. Pelo `curl`:
  - `GET …/manifests` (com token) → `200`, `{"items":[{"kind":"eco","nome":"Eco (exemplo)","situacao":"ok","versao":"0.1.0"},{"kind":"quebrado","mensagem":"o plugin deste tipo não respondeu ao handshake; …","situacao":"indisponivel"}]}`;
  - `GET …/manifests` sem token → `401`;
  - `GET …/manifests/quebrado` → `503 manifesto_indisponivel`.
  - Na resposta da lista: nenhuma ocorrência de caminho de programa, `secrets`, `config_json`, `argumentos` ou
    blob. Nos registros do servidor: nenhuma ocorrência do token nem do hash.
- Navegador (Chrome do sistema, Playwright, roteiro `/tmp/hr48c-run/e2e.mjs`): **24 de 24** conferências. Lista
  com os dois itens; selo "No ar" e "Indisponível" em texto; botão de detalhe só no plugin no ar; detalhe aberto
  pela linha e pelo teclado (Enter), com o botão marcado; "Atualizar" refaz a lista; `403` interceptado mostra
  a explicação e "Tentar de novo" recupera a lista; sem token e sem caminho de programa no console do navegador;
  nenhum 5xx. Capturas: `/tmp/hr48c-run/shots/01-lista-plugins.png`, `02-detalhe-eco.png`,
  `03-apos-tentar-de-novo.png`.

**Limites conhecidos.**

- **Média, primeira consulta:** a primeira listagem sobe cada plugin declarado (handshake com prazo). Com muitos
  plugins, a primeira resposta demora a soma dos handshakes; a tela avisa "Consultando plugins…". Depois, é cache.
- **Média, plugin atualizado:** o manifesto fica em cache pela vida do processo. Plugin trocado com o servidor no ar
  só aparece depois de reiniciar o servidor.
- **Baixa, falha em cache por 30 s:** um plugin consertado continua "Indisponível" por até 30 s na lista e no
  detalhe.
- **Baixa, plugin indisponível sem nome:** a lista mostra só o tipo, porque o manifesto não pôde ser lido.
- **Baixa, seleção:** o botão do plugin aberto recebe `aria-pressed`, mas não tem destaque visual próprio; o
  detalhe abre logo abaixo da lista.
- **Baixa, lista sem paginação:** é configuração da instância; se alguém declarar dezenas de plugins, a lista vem
  inteira.
- **Baixa, prova:** a prova usou o plugin de exemplo (`eco`) e um programa que não responde (`/bin/false`); um plugin
  real de WhatsApp (Evolution) não foi consultado nesta prova. Os cliques foram por automação, não por uma pessoa.
- **Baixa, contadores:** zeram quando o servidor reinicia; a tela diz isso no texto da página.
- **Baixa, leitura de `/metrics`:** a tela reconhece só os nomes listados na tela; uma métrica nova no servidor
  não aparece até a tela ser ajustada.
- **Baixa, atualização:** só com a aba visível; ao voltar para a aba, a próxima atualização espera o intervalo
  de 15 s, sem atualizar na hora.
- **Baixa, console do navegador:** aparecem 404 de recurso que o próprio navegador pede (o ícone do site,
  `/favicon.ico`, já listado em "Favicon ausente").

## Ajustes de design da revisão HR-49 (HR-49b)

A revisão de design com render real (1280 e 1024 px) gerou cinco correções. Todas estão no Console
atual; nenhuma muda contrato nem rota.

- **Nome do agente na trilha.** `Organizações / <org> / Agentes / <nome do agente> / <aba>`. O nome
  não é link. A aba do agente já carrega o agente e publica o nome; a trilha só usa o nome se o id bater
  com o da URL. Sem nome publicado, mostra "Agente".
- **Alvo de clique de 24 px.** Links da trilha e atalhos do painel da organização ganharam área de
  clique de 24 px de altura, sem mudar o texto.
- **Prévia de imagem em janela.** `SofaModal` de tamanho `lg` (760 px) no lugar da gaveta de 520 px.
  Foco preso, `Esc` fecha e devolve o foco ao botão "Ver <arquivo>". Vale para o anexo do cliente e para
  o arquivo do agente.
- **Argumentos e resultado indentados.** Quando o texto é JSON de objeto ou lista válido, sai com 2
  espaços. Cada valor sai como foi gravado (`10.50` continua `10.50`). Texto cortado com `…[cortado]`,
  texto livre, JSON quebrado e número com 16 ou mais dígitos saem como vieram.
- **Chat de teste em 1024 px.** Até 1100 px a coluna de conversas vai para `minmax(180px, 220px)`
  (era 280 px fixos), e as mensagens ganham largura. Até 1320 px a trilha mostra só o agente e a aba,
  e, até 1100 px, o campo de busca da barra superior fica com 150 px. Assim o nome do agente não vira pedaços.

**Provas.** Roteiro de navegador em 1280 e 1024 px (27 conferências, todas verdes): trilha, alvos de
clique, prévia (imagem de 1600×1000 limitada pelo modal), `Esc` e foco, argumentos indentados e texto
cortado. Testes de componente novos: trilha com e sem o nome, formatação do painel (válido, cortado,
não-JSON) e prévia como diálogo.

**Limites.**

- **Média, largura da prévia:** o modal `lg` tem 760 px. Numa janela de 1280 px a imagem fica em
  ~718 px de largura, e o limite de 80% só pesa em janelas mais estreitas. Se quiser imagem maior,
  o tamanho `xl` da SofaUI resolve (existe no componente; a largura dele não foi medida).
- **Média, barra superior até 1320 px:** a trilha mostra só o agente e a aba. Organização e
  "Agentes" somem nessa faixa, porque a trilha inteira cortaria os nomes; o contexto completo volta
  acima de 1320 px. Se preferir a trilha inteira em 1280 px, com nomes cortados, é só mudar esse limite.
- **Baixa, PDF e Office:** continuam só para baixar, como antes.

## Exportar e importar a organização (HR-48e-2)

**O que a tela faz.** Na tela de operação (`/orgs/:org/operacao`), a seção "Exportar e importar a organização"
baixa o arquivo da organização e reconstrói uma organização a partir de um arquivo. É a mesma regra da linha de
comando (`poph backup`): mesmo formato (`poph-backup-org`, versão 1) e as mesmas recusas.

- **Exportar.** Botão "Baixar arquivo da organização". A caixa "Incluir conversas" vem desmarcada: sem ela, o
  arquivo traz configuração, catálogo, base de conhecimento e biblioteca; com ela, leva também mensagens,
  turnos, anexos e as filas de entrada e saída. O download usa o nome que o servidor manda no cabeçalho
  (`<slug>-backup-AAAAMMDD-HHMM.json`).
- **Importar.** Escolha um arquivo `.json` (até 64 MB). A tela mostra o nome e o tamanho. Arquivo acima de
  64 MB é recusado na própria tela e nada sai do navegador. O botão "Importar arquivo nesta organização…" abre
  uma confirmação: a importação não apaga o que já existe na organização. Só depois de "Importar agora" o
  texto do arquivo é enviado. `Esc` ou "Cancelar" fecham a confirmação sem enviar nada.
- **Recusa por sobrescrita (HR-48e-4).** Se a importação atualizaria linhas que já existem na organização, o
  servidor recusa antes de gravar (`409 importacao_sobrescreve`) e a tela mostra "Esta importação vai
  sobrescrever o que já existe": o total de linhas, a contagem por tabela (nome de operador) e até 5 exemplos
  ("Agentes: bia"), com a frase de que **nada foi gravado**. O botão "Sobrescrever e importar" é a autorização
  explícita: só ele reenvia o arquivo com `?sobrescrever=true`, e a recusa some quando a importação passa. "Cancelar"
  tira a recusa da tela sem enviar nada. Nenhuma chamada é feita sozinha: a recusa só aparece depois da
  confirmação da importação, e o reenvio só depois do clique no botão. Conflito de nome (`409 conflict`) **não**
  mostra esse botão: não tem como ser autorizado.
- **Resultado da importação.** Mostra se o conteúdo foi gravado, o total de linhas (novas e atualizadas), uma
  tabela por tabela com as linhas que mudaram (nome de operador, não o técnico) e dois avisos quando couberem:
  - **Segredos para digitar de novo:** integração que chegou sem o segredo. O arquivo nunca leva segredo; a
    mensagem manda digitar o segredo de novo na aba Integrações do agente.
  - **Busca pronta** ou **Importado, índice pendente:** se o modelo de embedding estiver instalado, a base de
    conhecimento e os produtos ficam buscáveis na hora. Sem o modelo, o conteúdo entra, o índice fica pendente
    e a mensagem aponta a reindexação pela linha de comando. Índice pendente **não** é erro.

**De onde vem (rotas que já existem, nenhuma nova):**

| O que a tela faz | Rota | Observação |
| --- | --- | --- |
| Baixar o arquivo | `GET /admin/v1/orgs/{org}/backup` (sem query) ou `?com_conversas=true` | anexo; nome pelo `Content-Disposition` |
| Enviar o arquivo | `POST /admin/v1/orgs/{org}/backup/import` | corpo cru `application/json` com o texto do arquivo; a org do caminho é o destino |

Todas as chamadas levam o token da sessão. O envio é um texto JSON, não multipart.

**Erros, pelo código (nunca pela mensagem da API):**

| Código | O que o operador vê | Tentar de novo? |
| --- | --- | --- |
| `400 invalid_backup` | "Arquivo inválido": não é um backup válido desta instalação; com versões diferentes, diz as duas | não |
| `409 conflict` (importação) | "Importação recusada", com até 5 conflitos e "e mais N", e "Nada foi gravado" | não |
| `409 importacao_sobrescreve` (importação) | "Esta importação vai sobrescrever o que já existe": tabelas, total e exemplos; botão "Sobrescrever e importar" | pelo botão de autorizar |
| `409 backup_incompleto` (exportação) | um arquivo da biblioteca não está no disco; a exportação não sai pela metade | não |
| `413 payload_too_large` | "Arquivo grande demais": passa de 64 MB; nada foi gravado | não |
| `403` | o token não alcança esta organização | sim |
| `404` | a organização não existe mais | não |
| `401` | sessão encerrada (volta ao login) | não |
| `5xx` ou sem conexão | erro do servidor ou servidor fora do ar | sim |

Os conflitos são o que o servidor lista: o texto do servidor tem crase de Markdown, que a tela tira, e o nome
técnico da tabela sai com o nome de operador ("`product_categories`" vira "Categorias de produto"). Quando
não há conflito, a mensagem não se repete sozinha.

**Estados e atualização.** A seção não faz nada sozinha: exportar e importar são ações explícitas. A
atualização de 15 s da tela de operação não baixa nem envia arquivo, e a seção não consulta a API ao abrir.
Enquanto exporta ou importa, o botão fica em carga e o texto diz que não é para fechar a tela (na importação).

**Acessibilidade.** Campos com rótulo e dica; a confirmação é um diálogo com foco preso, `Esc` cancela e devolve
o foco. Carga e resultado são anunciados (região de status); erro é anunciado como alerta e não depende de cor.

**Provas.**

- Testes de componente (fetch falso, `console/src/componentes/ExportarImportarOrganizacao.test.tsx`, 21 testes; a recusa de sobrescrita (HR-48e-4) entra nesses 21):
  exportar sem e com conversas (query e nome do cabeçalho); download de `backup_incompleto` sem "Tentar de novo";
  erro de servidor na exportação com "Tentar de novo" que refaz; nada disparado ao montar; importação com
  sucesso (texto do arquivo no corpo, `content-type: application/json`, relatório, tabela, segredo e índice
  pronto); índice pendente sem tratar como erro; `409` com a lista e "nada foi gravado"; `400` com as versões;
  `400` sem o texto cru da API; `413`; `403` com "Tentar de novo" que envia de novo; arquivo acima de 64 MB
  recusado antes do envio; `Esc` e "Cancelar" sem envio; a confirmação explica que não apaga.
- Testes de funções puras (`console/src/api/backup.test.ts`, 17 testes): caminhos (query só com conversas),
  teto de 64 MB, nomes de tabela, textos de conflito e a tradução de cada código de erro.
- `console/src/telas/Operacao.test.tsx`: a seção aparece na tela de operação e a atualização automática não
  baixa nem envia nada.
- Servidor real (`poph serve`, portas 18860 e 18861/18862, bancos em `/tmp/hr48e2-run`, com o token do par
  reaproveitado de `/tmp/hr48c-run`; `poph token global` não foi rodado). Roteiro de navegador
  `/tmp/hr48e2-run/roteiro/e2e.mjs`: **28 de 28** conferências. Cobre:
  - exportação sem conversas (formato, versão, origem; sem tabelas de histórico) e com conversas (conversas,
    mensagens, turnos e filas);
  - no arquivo exportado: zero ocorrências de caminho de blob, de bloco de blob, de segredo cifrado e do valor
    de segredo plantado, e zero do token administrativo;
  - conflito: importação do arquivo da origem no destino vazio da mesma instância → "Importação recusada", lista
    e "nada foi gravado"; contagens do destino iguais antes e depois;
  - arquivo cortado no meio → "Arquivo inválido", sem texto cru, destino sem nada;
  - 1024 px: sem rolagem horizontal;
  - instância limpa: importação com sucesso (16 linhas novas, agentes, produtos e departamentos no destino),
    índice pendente com aviso (modelo ausente nesta instância) e a repetição com "0 novas" e contagens iguais;
  - arquivo com integração de segredo gravado → aviso "digite de novo" com a aba Integrações;
  - origem sem alteração; nenhuma resposta 5xx; token fora do console do navegador.
  - Capturas em `/tmp/hr48e2-run/shots/`: `01-exportar-1280.png`, `02-conflito-1280.png`, `03-invalido-1280.png`,
    `04-conflito-1024.png`, `sonda-segredo-1024-viewport.png` (os dois avisos em 1024 px).
- Conjunto de dados da prova: a origem foi criada na instância A importando o arquivo de prova da CLI
  (`/tmp/poph-cli-backup-origem-1115409/backup.json`, com conversas e integração de segredo). Esse import é a
  própria rota de importação; nenhum dado foi escrito direto no banco.

**Limites.**

- **Média, memória na exportação:** o arquivo inteiro é montado na memória do servidor e depois no navegador.
  Organização com biblioteca grande demora e pesa; não há download por partes.
- **Média, memória na importação:** o arquivo (até 64 MB) é lido inteiro no navegador e enviado como texto. Os
  blobs vão em base64, então o envio é ~33% maior que os bytes do arquivo.
- **Média, conflito recusa tudo:** um único recurso com o mesmo nome no destino impede a importação inteira.
  Não há mesclagem parcial.
- **Média, mesmo arquivo em duas organizações da mesma instalação:** a importação é recusada (409), porque os
  ids do arquivo já existem. Restaurar numa instalação limpa funciona.
- **Baixa, índice pendente:** a tela só avisa. Reindexar exige a linha de comando (`poph kb reindex` e
  `poph product reindex`); não há botão para isso.
- **Baixa, arquivo não é cifrado:** o conteúdo da biblioteca e dos anexos vai legível no arquivo. Guardar com
  permissão restrita.
- **Baixa, o arquivo não é conferido antes do envio:** a tela não lê a organização de origem do arquivo antes da
  confirmação; o que diz o arquivo aparece só depois do envio (no relatório ou no erro).
- **Baixa, download:** a URL temporária do arquivo é revogada 1 s depois do clique. Num navegador muito lento
  para iniciar o download, isso pode falhar; não foi visto na prova.
- **Baixa, prova com modelo:** nesta rodada o servidor de prova não tinha o modelo de embedding, então o estado
  "Busca pronta" foi provado só pelo teste de componente, não num servidor real.
- **Baixa, cliques:** todos foram por automação, não por uma pessoa.
- **Prova da recusa de sobrescrita (HR-48e-4):** servidor real na porta 18903, banco em `/tmp/hr48e4-run`, com o
  par de token de `/tmp/hr46b`. Roteiro `/tmp/hr48e4-run/roteiro/prova.mjs`: **28 de 28** conferências, em 1280 e
  1024 px. Exportação sem e com conversas pelo botão; o arquivo antigo reenviado depois de alterar o nome de um
  agente → recusa com a tabela (4 linhas), exemplos e "Nada foi gravado", e o nome continuou alterado; "Sobrescrever
  e importar" → relatório e o nome voltou ao do arquivo. Nenhuma resposta 5xx. Capturas em
  `/tmp/hr48e4-run/shots/` (`01-exportar-1280.png`, `02-recusa-1280.png`, `02-recusa-1024.png`,
  `03-relatorio-1280.png`, `03-relatorio-1024.png`).

## Cliente do OpenAPI

`console/src/api/schema.d.ts` é **gerado**, nunca editado à mão:

```bash
cd apps/pop-harness && bash console/gen-api.sh     # do `poph openapi`
bash console/gen-api.sh --url http://127.0.0.1:7480
```

`console/scripts/verificar-api.sh` regenera em pasta temporária e compara com o arquivo versionado
— é a checagem "cliente em dia" do `scripts/ci.sh`. Com `console/node_modules` instalado ela roda
**offline**; sem ele, se declara pulada para o CI de clone limpo não depender de rede.

## Decisões desta entrega

1. **pnpm**, como o resto do monorepo, com `console/package.json` próprio (fora do workspace do
   `apps/frontend`, que tem o seu).
2. **SofaUI por pasta local** (`file:../../frontend/packages/sofa-ui`): o Console consome o `dist`
   do pacote que já é versionado no monorepo; nenhuma cópia de componente no Console.
3. **Fontes copiadas** para `console/public/res/fonts` (12 arquivos WOFF2, 232 KB). O CSS da
   SofaUI usa caminho absoluto `/res/fonts/...` (HR-05) e a biblioteca não foi alterada; copiar
   deixa o bundle e o binário autossuficientes, sem depender de `apps/frontend` em tempo de build.
4. **Embutir sem dependência nova**: `build.rs` + `include_bytes!` em vez de `rust-embed`, porque o
   build do Rust continua sem Node e sem rede.
5. **Encadeamento explícito**, não automático: `cargo build` não chama `pnpm`. Quem quiser o
   Console no binário roda o build do Console antes — o CI faz isso quando `node_modules` existe.
6. **Teste de componente com Vitest + Testing Library + jsdom** (HR-39a), tudo em `devDependencies`
   do `console/package.json` e no `pnpm-lock.yaml` versionado. Nada de pacote global do sistema:
   `pnpm install` em `console/` é a única etapa que pede rede.

## Desvios da UX_SPEC (`console/UX_SPEC.md`)

| Spec | O que foi feito | Por quê |
|---|---|---|
| Rotas `/v1/...` e `POST /v1/auth/verify` | `/admin/v1/...`, validação por `GET /admin/v1/orgs` | O contrato real do servidor é `/admin/v1` (PRD §8.1) e não existe rota de verificação de token; o contrato não muda nesta rodada. |
| Campo "URL do endpoint" editável no login | Endereço da instância só exibido | O Console é servido pelo próprio binário: a origem é sempre a da instância que o entregou. |
| Seletor de escopo (Global / por org) no login | Escopo derivado do prefixo do token | O servidor decide o escopo pelo token; pedir ao operador seria pedir uma escolha que ele não controla. |
| `401` **e** `403` derrubam para o login | Só `401` derruba; `403` mostra o erro da chamada | `403` é token válido sem escopo para aquele recurso — derrubar a sessão esconderia a causa. |
| Botão "+ Criar Primeira Organização" no vazio | Implementado na HR-39a, junto com o CRUD de organização | Era dependência da tela de gestão; agora que ela existe, o vazio leva à criação pelo mesmo formulário. |
| Barra lateral de 240 px com rótulos longos | 232 px desde a HR-61 (a barra superior de 48 px atravessa a tela e o menu começa abaixo dela), com itens futuros mostrando "em breve" na segunda linha | Rótulo truncado ("Saúde da in…") é ruído; a segunda linha preserva a largura, e a medida de 232 px é a do desenho aprovado (D1/D2). |
| Categoria com `slug` e contagem de produtos na coluna | Só nome e descrição | A API de categoria não tem `slug` nem contagem de produtos; a tela não inventa o que o contrato não devolve. |
| Departamento com `ID` legível (`dep_general`) | Id da API (UUID) não é mostrado na lista | O contrato devolve UUID; o identificador legível não existe no servidor. |
| Exclusão de categoria no diálogo | Existe, com contagem de produtos e agentes e confirmação com `force` (HR-39b, rodada 2) | Na primeira versão a API apagava os produtos sem confirmação; a API passou a exigir `force`, como a CLI e a organização. |
| Exclusão de produto com itens de carrinho: "Excluir mesmo assim" como botão (pedido da HR-42g) | Confirmação extra (caixa de marcar) no mesmo diálogo, que aparece depois do `409` com a contagem | Segue o desenho da exclusão de categoria (HR-39b). Um segundo botão de exclusão criaria outro caminho destrutivo; aqui o nome digitado continua valendo. |
| Prévia do prompt "em tempo real" | A prévia é o markdown da última versão salva, com "Atualizar prévia" | O servidor só compõe o markdown quando grava (`POST`/`PATCH`); não existe rota de prévia. Remontar o texto na tela contrariaria a regra de que a prévia vem do servidor. |
| `VersionConflictModal` com diff lado a lado, "copiar rascunho" e "sobrescrever forçado" | Banner no formulário com a releitura e a versão atual; o rascunho fica e o operador salva de novo | O contrato não tem sobrescrita forçada (`PATCH` exige `If-Match`; `POST` com registro existente é `412`), e não há diff a mostrar: o estruturado salvo não é exibido lado a lado. |
| Abas do agente todas funcionais, com `Alt+1`…`Alt+7` | "Instruções" (HR-40), "Base de conhecimento" (HR-41), "Arquivos" e "Roteamento" (HR-43) e "Integrações" (HR-44) habilitadas; Modelo/Jev aparece desabilitada | As demais são as entregas HR-45 em diante; atalho para aba que não existe prometeria o que não há. |
| Aba "Produtos" dentro do agente (`#products`) | Tela de produtos no contexto da organização (`/orgs/:id/products`), com a coluna "Agentes que oferecem" em leitura | O catálogo é da organização: produto não tem vínculo com agente (o agente liga categorias). Na aba do agente, a mesma lista apareceria repetida para cada agente. Vale a decisão do mantenedor, que pediu o contexto da organização. |
| Cabeçalho do agente com "Chat de Teste ↗" | Sem o botão | Chat de teste é HR-47. |
| `PATCH /v1/orgs/:org/agents/:id/instructions` | `PATCH /admin/v1/orgs/:org/agents/:id/instructions/current` | Contrato real (`docs/API.md`). |
| Base: métricas do índice (chunks indexados, 384d int8, versão do chunker) | Painel com o estado que a API permite saber: "não verificado", aviso de `index_warning`, relatório do reindex | A API administrativa não expõe rota de estado nem contagem do índice; `poph kb status` lê isso **offline**. Mostrar números inventados seria pior que dizer o que falta. |
| Reindexar devolve `202` com polling a cada 2s | `POST` síncrono com aviso de espera e spinner; o relatório aparece ao concluir | O contrato real responde só quando a sincronização termina (`docs/API.md`). |
| Rotas `/v1/orgs/:id/knowledge-bases` e `…/items` | `/admin/v1/orgs/:id/knowledge` e `…/knowledge/{group}/items` | Contrato real do servidor. |
| Contador "1000ch" no editor | Contador `N / 1000 caracteres` ao vivo, validado antes e depois | O limite existe no servidor (`LIMITE_CONTEUDO_ITEM`), então o número é real. |

## Limites conhecidos

- **Exclusão de produto em carrinho é recusada sem `force`** (resolvido na HR-42f e na HR-42g): a API
  recusa com `409` quando há itens de carrinho e só apaga com `?force=true`; a tela manda `force` depois
  da confirmação extra. Medido por HTTP: `409` com `itens_de_carrinho: 2` e `carrinhos: 1`, `GET` depois
  com `200`; com `force=true`, `200` e `GET` com `404`. Produto sem carrinho sai sem `force`, como antes.
  A CLI continua exigindo `--force` sempre (divergência consciente, registrada na HR-42f).
- **Texto próprio do preço não se distingue do calculado na leitura** (média, contrato): a API devolve
  só o texto que vale; a tela trata a troca de preço como "volta a calcular" para não deixar o valor
  antigo. Um texto próprio que o operador queira manter precisa ser digitado de novo depois da troca.
- **Preço automático com separador de milhar** (resolvido na HR-42b): o servidor formata
  `R$ 8.900,00` e `R$ 2.450,00`. Só o texto digitado pelo operador sai como foi digitado.
- **Carrinho que cita o produto excluído** (baixa): o item do carrinho perde a referência ao produto e
  mantém o preço gravado (medido no banco de teste na HR-42g: `product_id` vazio, preço `45000` mantido).
  A tela não lê carrinho pela API de administração, então a contagem vem só da recusa `409`.
  O efeito no orçamento que o cliente vê (nome "produto removido do catálogo") não foi testado no turno.
- **Índice de busca não atualiza sem o modelo de embedding** (média, neste ambiente): toda criação,
  alteração e exclusão devolve `index_warning`, e a tela avisa. A busca dos agentes só muda depois da
  reindexação. Testado aqui com o modelo ausente; com o modelo instalado, não medi.
- **Filtros sobre a lista inteira no navegador** (média, acima de centenas de produtos): a tela baixa
  todas as páginas do catálogo antes de filtrar. Com muitos produtos, a primeira carga demora mais.
- **Contagem de agentes lê até 200 agentes** (baixa): e conta agentes de qualquer situação, inclusive
  inativos, porque a tela não lê a situação do agente para essa coluna.
- **Sem vínculo editável de produto com agente** (baixa, por desenho): a tela só mostra quais agentes
  enxergam o produto. Mudar isso é alterar a categoria vinculada ao agente, que é tarefa da aba do
  agente (HR-43 a HR-46) e da CLI.

- **Departamentos e categorias: lista sem paginação** (média, acima de 200 registros): a tela pede
  `limit=200` e ignora o cursor, como na lista de organizações. A contagem de agentes também lê só
  os primeiros 200.
- **Listagem de produtos recusa `limit`** (média, defeito do servidor, fora desta entrega): `GET
  /products` com `limit` responde `400 invalid_request` (a leitura de `limit` junto de `all` no
  `ConsultaProdutos` de `crates/poph-server/src/api/produtos.rs` falha). A tela contorna paginando
  só com `cursor`. O consumidor da API e a CLI não foram verificados contra isso; o correto é corrigir
  a rota e cobri-la com teste.
- **Exclusão de categoria não atualiza a versão do agente** (baixa): quem perde a categoria continua
  com a mesma versão; um formulário de agente aberto antes não recebe `412`. Mesmo efeito do
  departamento, medido na rodada 2 (`versao` 2 antes e depois).
- **Contagem da exclusão lê um agente por vez** (baixa): a tela lê o detalhe de cada agente da org para
  saber se ele usa a categoria. Com muitos agentes, o diálogo demora mais para abrir.
- **Token de org não edita a própria organização pela tela**, embora a API permita (`docs/API.md`:
  PATCH "global; org na própria"; medido: a alteração passa com token de org). Por isso o texto da
  HR-39a que diz que o servidor responde 403 nesse caso está errado (baixa, herdado da HR-39a).
- **Exclusão de departamento não atualiza a versão do agente** (baixa): o agente que perde o
  departamento continua com a mesma versão; um formulário de agente aberto antes não recebe 412.
- **Favicon ausente** (baixa): o servidor não serve `/favicon.ico`; o navegador registra 404 no
  console. Não afeta a tela.
- **Sem i18n**: textos em português do Brasil, fixos no código.
- **Operação parcial** (HR-48a, HR-48c, HR-48e-2): a tela de saúde, a lista de plugins e exportar/importar a
  organização existem; cluster, migração e instalar ou remover plugin continuam fora. O que falta está nas seções
  "Saúde da instância (HR-48a, HR-48c)" e "Exportar e importar a organização (HR-48e-2)".
- **Bundle é cópia em tempo de build**: trocar o Console exige `pnpm build` e recompilar o binário;
  não há recarga a quente no servidor de produção.
- **`node_modules` não é versionado**: um clone limpo precisa de rede uma vez para instalar.
- **Lista de organizações sem paginação na tela** (média, acima de 200 orgs): a tela pede
  `limit=200` e ignora o cursor; passando disso, o excedente não aparece até a tela de busca chegar.
- **Retenção é um campo livre em dias** (baixa): o Console grava `config.retention_days` e não
  conhece limites de plano; fora da faixa aceita pelo servidor, o erro aparece no próprio campo.
- **Exclusão não tem "desfazer"** (média, por desenho): a confirmação por slug é a única trava; o
  que for apagado não volta.
- **Teste de componente não cobre o `poph serve` de verdade** (baixa): a prova de integração é por
  curl/script contra o servidor no ar; o teste de componente usa `fetch` falso.
- **Voltar do navegador não dispara o aviso** (média, ao navegar): a guarda cobre links internos,
  o seletor de organização, fechar/recarregar a página e o `Ctrl+S`. O botão voltar do navegador
  troca de rota sem passar pela guarda; o rascunho se perde. `BrowserRouter` não tem `useBlocker`
  (é dos data routers) e migrar o roteador não era desta tarefa.
- **A prévia não acompanha o rascunho** (média, ao editar): ela mostra o texto salvo até o próximo
  salvamento; o aviso no painel diz isso. Ver o markdown do que está na tela exigiria uma rota de
  composição sem gravação, que não existe.
- **Prévia nunca foi editada à mão** (baixa): a tela manda só `structured`. O contrato aceita
  `markdown` puro (`derived_from_json = false`), mas essa forma não tem editor no Console — quem
  precisa dela usa a CLI (`--markdown-file`). Ao salvar pela tela, um markdown manual é substituído
  pelo composto.
- **Lista de agentes sem paginação** (média, acima de 200 agentes): `limit=200`, sem cursor.
- **Sem CRUD de agente** (média, por escopo): criar, editar e excluir agente continua na CLI/API
  até a tarefa própria; a lista só seleciona.
- **Validação local é mínima** (baixa): exige nome do assistente e regra em todo princípio; o resto
  é validado pelo servidor (ferramentas da V1) e volta na tela como erro da seção.
- **`flow_steps` e `scope.handoff` são preservados, sem editor** (baixa): o formulário não expõe
  passos de fluxo nem motivos de transferência; o que veio do servidor volta intacto ao salvar.
- **Ferramenta fora da V1 aparece como aviso e precisa ser desligada** (baixa): se um registro
  antigo trouxer nome desconhecido, a tela mostra o aviso e o servidor recusa a gravação enquanto
  ela estiver ligada.
- **Padrão de ferramentas do registro novo** (baixa): `consultar_base` e `buscar_produto` vêm
  ligadas; o operador desliga o que não usar. Não há "modelo de agente" para escolher.
- **Estado do índice só a partir de escrita ou reindexação** (média, ao operar a base): a API não
  tem rota de status, então uma carga nova da tela mostra "não verificado" mesmo com o índice em
  dia. O estado real fica em `poph kb status --offline` (CLI). Uma rota `GET …/knowledge/status`
  resolveria; hoje não existe.
- **Reindexar bloqueia a aba até a resposta** (média, catálogo grande): o `POST` é síncrono; com
  muito conteúdo o navegador espera o servidor. Não há fila nem acompanhamento por polling, porque
  o contrato não oferece.
- **Grupos: uma leitura de detalhe por grupo** (baixa): para marcar o vínculo com o agente, a tela
  lê cada grupo (`GET …/knowledge/{id}`). Com muitos grupos isso vira N+1 requisições; o contrato
  não devolve os agentes na listagem.
- **Contagem de trechos do grupo só ao abrir/excluir** (baixa): a lista de grupos não mostra a
  contagem; ela existe no diálogo de exclusão (pagina até o fim) e no cabeçalho da coluna de trechos.
- **Ordem do trecho é número livre** (baixa): a tela aceita inteiro (inclusive negativo) e o
  servidor não valida faixa; empate resolve por id.
- **Sem campo de "fonte"** (baixa): o item criado pela tela é sempre `source_type = manual`; não há
  editor para conteúdo importado de arquivo.
- **Índice local sem modelo de embedding** (média, no ambiente atual): sem o modelo instalado, toda
  gravação que tenta sincronizar devolve `index_warning` e `reindexar` responde `503
  embedding_unavailable`. A tela explica, mas busca e reindexação não funcionam até instalar o modelo.
