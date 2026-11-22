# POP Harness: runbooks operacionais

Passo a passo para o dia a dia do app já instalado: subir e parar, atualizar a versão, fazer backup e
restaurar uma organização, adicionar um plugin, calibrar um gate do Jev e operar um cluster de instâncias.

Cada runbook diz quando usar, o que precisa estar pronto, os comandos, o que conferir depois, o que
fazer se falhar e o que não fazer. Quando um passo não foi executado na prova desta página, ele está
marcado **não verificado**, com o motivo.

**Prova desta página:** 21/11/2026, sobre o commit `035adbb6`, com o binário compilado nesse dia. Tudo
rodou num ambiente descartável em `/tmp/hr58b-run`, sem provedor de LLM, sem WhatsApp real e sem modelo
de embedding. O comando `poph token global` **não** foi rodado.

A seção 6 (cluster e encaminhamento) foi provada depois, no mesmo dia, com duas instâncias reais no
binário do commit `eff9dd1f` (encaminhamento da HR-50b-2), em `/tmp/hr58b2-run-2`. Mesmas regras: sem
`poph token global`, sem WhatsApp real, sem LLM.

## Antes de começar

| O que | Onde fica |
|---|---|
| Programa | `target/debug/poph` (gerado por `cargo build -p poph`) |
| Configuração | o arquivo `poph.toml` indicado em `--config` (ou na variável `POPH_CONFIG`) |
| Bancos | `harness.db` (dados) e `rag.db` (busca), dentro da pasta indicada em `[dados] diretorio` |
| Anexos e modelo | pastas `blobs/` e `modelos/` dentro da pasta de dados |
| Logs | não há arquivo de log próprio: o servidor escreve no terminal (erro padrão). Em segundo plano, redirecione para um arquivo (ver runbook 1) |
| Segredos | só em variáveis de ambiente, nunca no `poph.toml`: `POPH_CHAVE_CIFRA` e `ANTIGRAVITY_API_KEY` (obrigatórias); `TYPESAFE_API_KEY` (só com o Jev ligado); `EVOLUTION_API_KEY` (só com WhatsApp) |

Exemplo de `poph.toml` usado na prova (a porta padrão é 7480; valores fora do padrão não são obrigatórios):

```toml
[servidor]
endereco = "127.0.0.1"
porta = 18961

[dados]
diretorio = "/tmp/hr58b-run/a/dados"

[log]
nivel = "info"
```

Regras gerais:

- Os comandos com `--offline` e `migrate` escrevem direto no banco. Rode-os com o servidor **parado**
  (`docs/BACKUP.md` e `docs/CONFIGURACAO.md` pedem isso; por causa da regra de um escritor por vez em
  `docs/DADOS.md`). Não testei esses comandos com o servidor no ar.
- Segredo não entra em arquivo, em log nem em resposta. Ao colar um comando, troque o `<...>` pelo valor
  guardado fora do repositório.

---

## 1. Subir e parar

**Quando usar:** primeira subida, depois de reiniciar a máquina ou ao colocar uma instância no ar.

**Pré-requisitos:**

- binário compilado (`cargo build -p poph`);
- `POPH_CHAVE_CIFRA` e `ANTIGRAVITY_API_KEY` definidas no ambiente da sessão;
- `poph.toml` escrito (ver o exemplo acima).

### Comandos

Conferir o ambiente e preparar os bancos (servidor parado):

```bash
cd pop-harness
export POPH_CHAVE_CIFRA='<chave de cifra, guardada fora do repositório>'
export ANTIGRAVITY_API_KEY='<chave do provedor de LLM>'
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml doctor
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml migrate
```

Subir no terminal (fica preso; Ctrl-C encerra):

```bash
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml serve
```

Subir em segundo plano, com o log num arquivo:

```bash
cd pop-harness
setsid nohup ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml serve \
  > /tmp/hr58b-run/a/serve.log 2>&1 < /dev/null &
echo $! > /tmp/hr58b-run/a/serve.pid
```

### O que conferir depois

```bash
curl -s http://127.0.0.1:18961/healthz
```

Resposta da prova:

```json
{"status":"ok","version":"0.1.0","uptime_seconds":2,"databases":{"harness":2,"rag":0}}
```

- `status: ok` com `HTTP 200` = o servidor está respondendo.
- `databases` mostra a versão de cada banco.
- O log mostra `servidor no ar endereco=127.0.0.1:18961`.
- Sem o modelo de embedding aparece o aviso `busca na base desligada: modelo de embedding indisponível`.
  Isso é esperado até rodar `poph model install`, que precisa de internet (**não verificado** aqui).
- Console: `http://127.0.0.1:18961/` responde `HTTP 200`. Não abri a tela no navegador nesta prova.

### Parar sem corromper

- No terminal onde ele roda: **Ctrl-C**.
- Em segundo plano: `kill -INT $(cat /tmp/hr58b-run/a/serve.pid)`.

Confirmado na prova: o processo saiu em cerca de 1 s, o log mostrou `encerrando o servidor…`, a porta
foi liberada e `doctor` depois da parada confirmou os dois bancos íntegros.

Não use `kill -9`: não testei, e ele pula qualquer encerramento limpo. Use SIGINT.

### Produção

Não há instalação de produção do POP Harness neste repositório nem nesta máquina: **não verificado**.
O procedimento é o mesmo: o mesmo binário, com outro `poph.toml` (porta, pasta de dados) e as chaves
no ambiente do processo da instalação real. Não confunda com `/opt/pop-atende`, que é do POP Atende.

### Se falhar

- `doctor` mostra `variável ... ausente`: a chave não está no ambiente da sessão. Defina e rode de novo.
  O `doctor` não mostra o valor.
- `doctor` mostra `banco ... não é SQLite`: **não rode `migrate`**. O arquivo no caminho do banco não é
  do POP Harness; confira a pasta de dados antes de qualquer coisa.
- Porta ocupada: não testei a mensagem de erro. Confira com `ss -ltn | grep <porta>` e troque `porta`
  no `poph.toml`.

### Não fazer

- Não coloque segredo no `poph.toml`.
- Não apague a pasta de dados para "resetar" uma instalação em uso: ela tem as conversas, a configuração
  das orgs e os anexos.

---

## 2. Atualizar a versão

**Quando usar:** trocar o binário por uma versão nova do código.

**Pré-requisitos:** acesso à pasta de dados e ao binário atual; o servidor precisa estar parado.

### Ordem segura

1. **Pare o servidor** (ver runbook 1).
2. **Guarde o binário atual e uma cópia da pasta de dados**, com o servidor parado:

   ```bash
   cd pop-harness
   cp target/debug/poph /tmp/poph-versao-anterior
   cp -a /tmp/hr58b-run/a/dados /tmp/hr58b-run/a/dados.antes-da-atualizacao
   ```

   Troque `/tmp/hr58b-run/a/dados` pela pasta de dados real da instalação.

3. **Traga a versão nova do código** pelo fluxo de git da equipe.
4. **Compile:**

   ```bash
   cd pop-harness
   cargo build -p poph
   ```

   Na prova, com o cache já existente, levou cerca de 10 s e recompilou `poph-server` e `poph`. Do zero
   demora mais.

5. **Migre os bancos** (servidor parado):

   ```bash
   ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml migrate
   ```

   Rodar de novo não faz nada. Na prova, o banco de harness foi de versão 0 para 2 na primeira vez e
   a segunda execução respondeu `já estava na versão 2`. Saída `0` = bancos em dia.

6. **Confira:** `./target/debug/poph --config /tmp/hr58b-run/a/poph.toml doctor` sem erro.
7. **Suba** de novo (runbook 1) e confira `/healthz`.

### Se a subida falhar

1. Pare a versão nova (Ctrl-C ou SIGINT).
2. Volte o binário: `cp /tmp/poph-versao-anterior target/debug/poph`.
3. Se a migração já rodou, volte a pasta de dados. Com o servidor parado, mova a atual de lado e
   restaure a cópia (não apague nada):

   ```bash
   mv /tmp/hr58b-run/a/dados /tmp/hr58b-run/a/dados.depois-da-falha
   mv /tmp/hr58b-run/a/dados.antes-da-atualizacao /tmp/hr58b-run/a/dados
   ```

4. Suba a versão anterior e confira `/healthz`.

**Não verificado:** se o binário antigo abre um banco que a versão nova já migrou. Não testei. Por isso
a volta segura é a cópia do passo 2. As migrações só andam para frente (`docs/DADOS.md`: migração
publicada não se edita) e não há comando para desfazer uma migração.

### O que não fazer

- Não migre com o servidor no ar.
- Não confie na cópia da pasta de dados se ela foi feita com o servidor rodando: pode estar pela metade.
- Não pule a cópia: a volta depende dela, e as escritas feitas depois da cópia se perdem ao voltar.

---

## 3. Backup e restauração de uma organização

**Quando usar:** antes de mexer numa org grande, para levar uma org para outra instalação ou para
reconstruir uma org numa instalação nova.

### O que o arquivo leva

| Entra sempre | Entra só com `--com-conversas` | Não entra nunca |
|---|---|---|
| Configuração da org, produtos, categorias, departamentos | Conversas, mensagens e anexos (com os bytes) | Tokens de administração (são criados de novo) |
| Agentes e instruções | Turnos (o "porquê" de cada resposta) | Trilha de auditoria (`audit_log`) |
| Base de conhecimento | As duas filas (entrada e saída) | Segredo de integração, nem cifrado (sai só a marca de que existia) |
| Biblioteca de arquivos (com os bytes) | Carrinhos de orçamento | Caminho interno dos arquivos |
| Integrações (sem o segredo) | | |

Formato: JSON com `formato: "poph-backup-org"` e `versao_formato: 1`. Uma versão que a instalação não
conhece é recusada.

### Exportar pela CLI (servidor parado)

```bash
cd pop-harness
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml --offline backup export clinica-prova --out /tmp/hr58b-run/backup-clinica.json
```

- `clinica-prova` é o slug ou o id da org.
- `--com-conversas` inclui o histórico. Sem ele, o arquivo não leva conversas.
- Não sobrescreve arquivo existente. Para trocar, use `--sobrescrever`.
- `--json` dá saída estável para máquina.

Saída da prova:

```
Org exportada para /tmp/hr58b-run/backup-clinica.json (2 KB).
  agents: 1
Blobs no arquivo: 0 (um por sha256; o caminho é recalculado na importação).
Conversas não entraram (use --com-conversas para levá-las).
```

Conferir que nada indevido saiu:

```bash
grep -c blob_path /tmp/hr58b-run/backup-clinica.json
grep -c secrets_enc /tmp/hr58b-run/backup-clinica.json
```

Esperado: `0` e `0`. Na prova deu `0` e `0` nos dois arquivos (com e sem conversas).

### Restaurar pela CLI (instalação limpa)

```bash
cd pop-harness
./target/debug/poph --config /tmp/hr58b-run/b/poph.toml --offline backup import /tmp/hr58b-run/backup-clinica.json
```

- Numa instalação sem a org, a importação cria a org com o mesmo id e o mesmo nome.
- Com `--org <slug-ou-id>` a importação grava numa org que já existe.
- Rodar de novo com o mesmo arquivo, numa org que já tem essas linhas, é **recusado** sem `--sobrescrever`:
  a saída diz quantas linhas seriam atualizadas, em quais tabelas e até 5 exemplos, e nada é gravado.
  Com `--sobrescrever`, a importação segue e o relatório mostra `inseridas` e `atualizadas`.

Saída da prova na instalação limpa: `Org ... criada.`, `agents: 1 nova(s)`, e o aviso de índice pendente
(sem o modelo de embedding, a base e os produtos ficam sem índice até rodar os comandos abaixo).

### Pela API (equivalente à CLI)

- Exportar: `GET /admin/v1/orgs/{org}/backup` (sem conversas) ou `GET /admin/v1/orgs/{org}/backup?com_conversas=true`.
  Resposta `200`, `content-type: application/json`, `content-disposition: attachment` com nome do tipo
  `clinica-prova-backup-20261121-1639.json`, e `x-content-type-options: nosniff`.
- Importar: `POST /admin/v1/orgs/{org}/backup/import`, com o **texto do arquivo** como corpo, em
  `Content-Type: application/json`. Não é multipart. Limite de 64 MB; acima disso a resposta é `413`.
- Sem `?sobrescrever=true`, se a importação atualizaria linhas que já existem na org, a resposta é `409`
  com `code: "importacao_sobrescreve"`, `details.tabelas` (tabela → linhas), `details.linhas` e
  `details.exemplos` (até 5). Nada é gravado. Com `?sobrescrever=true`, a importação segue.

Exemplo:

```bash
TOKEN=$(cat <arquivo onde você guardou o token de admin>)
curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  --data-binary @/tmp/hr58b-run/backup-clinica.json \
  http://127.0.0.1:18961/admin/v1/orgs/<id-da-org>/backup/import
```

Na prova da recusa: importar o arquivo na própria org, sem autorização, respondeu `409` com
`importacao_sobrescreve` e nada mudou; com `?sobrescrever=true`, respondeu `200` e o agente voltou ao nome
do arquivo. Importar o mesmo arquivo numa outra org da mesma instalação respondeu `409` com a lista do
conflito e a frase de que nada foi gravado. Os códigos de erro completos estão em `docs/API.md`.

### O que conferir depois

- Liste as orgs e os agentes na instalação de destino e compare com a de origem.
- Se o relatório disser que o índice ficou pendente, instale o modelo (`poph model install`, precisa de
  internet) e rode, com o servidor parado:
  `./target/debug/poph --config /tmp/hr58b-run/b/poph.toml --org clinica-prova --offline kb reindex` e
  `./target/debug/poph --config /tmp/hr58b-run/b/poph.toml --org clinica-prova --offline product reindex`
  (troque o caminho do `poph.toml` e o slug pelos da sua instalação).
  Na prova, sem o modelo, o `kb reindex` só avisou que o modelo não está instalado. A reindexação com
  o modelo presente **não verificado** aqui.
- Integrações voltam sem o segredo. Digite o segredo de novo na aba Integrações do Console. **Não
  verificado** nesta prova; a regra está coberta pelos testes do repositório.

### Se falhar

- Arquivo cortado ou que não é JSON: a importação recusa com "não é um backup do POP Harness". Nada é
  gravado (confirmado). Copie o arquivo de novo.
- Versão de formato desconhecida: a mensagem diz qual versão a instalação entende (confirmado com a
  versão `99`). Use uma instalação com a versão que exportou o arquivo.
- Conflito: "o id do arquivo já existe nesta instância em outra org". Nada é gravado (confirmado).

### O que não fazer

- **Não autorize a sobrescrita (`--sobrescrever` ou `?sobrescrever=true`) para desfazer uma mudança.** Sem
  a autorização, a importação é recusada e mostra o que seria atualizado. Com ela, as linhas voltam ao
  conteúdo do arquivo: na prova, um agente que estava na versão 2 (com a configuração do Jev alterada)
  voltou para a versão 1 e perdeu a alteração feita depois do arquivo. Leia a lista da recusa antes de
  autorizar. Para voltar no tempo, restaure numa instalação limpa.
- **Não importe o mesmo arquivo em duas orgs da mesma instalação.** Dá `409`, porque os ids do arquivo
  já existem. Restaurar numa instalação limpa funciona. Esse é um limite conhecido, já registrado nas
  entregas de backup.
- Não exporte nem importe com o servidor no ar (ver regras gerais).
- Trate o arquivo como dado de cliente: com `--com-conversas` ele leva mensagens e anexos.

---

## 4. Adicionar um plugin

**Quando usar:** ligar uma integração que roda em processo externo (um programa separado, que o
servidor sobe e fala com ele).

**Como funciona:** o servidor sobe o programa e manda a primeira linha, o *handshake*, com o protocolo,
a org, o tipo (`kind`), a configuração e os segredos. O programa responde com o manifesto (nome, versão,
o que aceita e o formato da configuração). Se o programa não responder, o tipo aparece como
*indisponível*; o servidor não cai. O protocolo está em `docs/PLUGIN_PROTOCOLO.md`.

**Pré-requisitos:**

- o programa do plugin compilado, com caminho absoluto. Para testar, o plugin de exemplo:
  `cargo build -q -p poph-plugin-eco --bins` (é o que `scripts/plugin-eco.sh` faz). O binário fica em
  `target/debug/plugin-eco`.

### Passo 1: declarar no `poph.toml`

```toml
[[plugins.integracoes]]
kind = "eco"
programa = "target/debug/plugin-eco"
argumentos = []
```

- `kind` é único e não pode ser vazio.
- `programa` é caminho local do servidor. Nunca vem de uma requisição.
- `argumentos` não pode levar segredo: ficam visíveis na lista de processos (`ps`). Segredo chega só pelo
  handshake.

### Passo 2: reiniciar o servidor

O manifesto é lido na primeira consulta e guardado em memória (ver "Limites" abaixo). Reinicie o servidor
(runbook 1).

### Passo 3: conferir

Troque `<id-da-org>` pelo id da org (aparece em `poph --config <seu poph.toml> --offline org list`, com o servidor parado) e use o token de
admin no `Authorization`:

```bash
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:18961/admin/v1/orgs/<id-da-org>/manifests
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:18961/admin/v1/orgs/<id-da-org>/manifests/eco
```

Saída da prova, com o plugin de exemplo e um segundo tipo de propósito quebrado (`/bin/false`):

```json
{"items":[{"kind":"eco","nome":"Eco (exemplo)","situacao":"ok","versao":"0.1.0"},
          {"kind":"quebrado","mensagem":"o plugin deste tipo não respondeu ao handshake; verifique o programa declarado em `[plugins].integracoes`","situacao":"indisponivel"}]}
```

- `situacao: ok` = manifesto lido, com nome e versão.
- `situacao: indisponivel` = o programa não respondeu. Pelo detalhe (`.../manifests/quebrado`) a resposta
  é `503` com `manifesto_indisponivel`.
- Sem token: `401`.
- A tela de Operação (Saúde da instância) mostra a mesma lista. **Não verificado** nesta prova, porque o
  Console não foi aberto no navegador.

### Limites

- **Troca de plugin só aparece depois de reiniciar o servidor.** O manifesto fica em cache até o processo
  acabar. Esse comportamento está escrito no comentário de `crates/poph-server/src/manifestos.rs`. Não
  repeti o teste nesta prova.
- **Falha fica em cache por pouco tempo.** Um plugin consertado continua indisponível por até 30 s
  (número da entrega da lista de plugins; não remedido aqui).
- **Remover um plugin:** apagar o bloco e reiniciar. **Não verificado.**

### Se falhar

- O log do servidor mostra `não consegui ler o manifesto do plugin kind=... programa=... erro=handshake
  recusado`. A causa costuma ser: o caminho do programa não existe, o arquivo não é executável, ou o
  programa fecha antes de responder.
- Protocolo diferente: o servidor recusa a versão incompatível (`docs/PLUGIN_PROTOCOLO.md`).

### O que não fazer

- Não ponha segredo em `argumentos`.
- Não aponte para um programa que venha de fora (de uma requisição, de um arquivo enviado por cliente).
  O caminho é só da configuração.
- Não conte com a troca de um plugin sem reiniciar.

---

## 5. Calibrar um gate do Jev

**Quando usar:** decidir se um gate liga por padrão ou só em um agente, ou medir o efeito de uma mudança
antes de mexer no padrão.

**O que é o Jev:** o serviço que decide, por escolha entre opções conhecidas, coisas como transferir
para humano, esperar mais ou processar agora. Ele é separado do modelo de texto. O detalhe completo
está em `docs/JEV.md`.

### Os gates

| Gate | Pergunta que responde | Ligado por padrão |
|---|---|---|
| G1 | transferir para humano agora? | não |
| G2 | esperar mais ou processar agora? | não |
| G3 | precisa de base, produto ou nada? | **sim** |
| G4 | qual departamento recebe a transferência? | não |
| G5 | qual produto o cliente quer? | **sim** |
| G6 | a resposta pode ser enviada? | não |
| G7 | é injeção de prompt ou abuso? | não |
| G8 | a conversa foi encerrada pelo cliente? | **sim** |

O padrão está em código, na função `padrao()` de `crates/poph-types/src/config.rs`: G3, G5 e G8 ligados,
os outros desligados (confirmado lendo o código). Mudar o padrão é mudança de código e passa pela
revisão; não é só edição de configuração.

O interruptor geral é `[jev] ativo`, que vem desligado (`docs/CONFIGURACAO.md`: "liga o Jev"). A variável
`POPH_JEV_ATIVO` sobrepõe o arquivo. Com o interruptor desligado, o Jev não é consultado, mesmo com os
gates ligados. Esse efeito não foi testado nesta prova; a leitura vem da documentação.

### Ligar e desligar

**Na instalação inteira** (`poph.toml`):

```toml
[jev]
ativo = true
url_base = "https://api.typesafe.ai"

[jev.gates]
g1 = true
```

Com o Jev ligado, defina `TYPESAFE_API_KEY` no ambiente. **Não verificado** com o Jev real: a prova não
tinha a chave.

**Em um agente só** (é o único ajuste por agente):

```bash
cd pop-harness
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml --org clinica-prova agent show atendente-prova --json
./target/debug/poph --config /tmp/hr58b-run/a/poph.toml --org clinica-prova agent update atendente-prova \
  --if-match 1 --offline --jev-json '{"ativo": true, "gates": {"g1": true, "g4": false}}'
```

- `--if-match` é a versão lida no `agent show` (campo `version`). Se não bater, a alteração é recusada com
  `conflito de versão ... esperada 2, atual 1` (confirmado).
- `{"ativo": false}` desliga todos os gates desse agente. O que o agente não declarar herda o global.
- Retornar a `{}` herda tudo do global. **Não testei** essa volta.

**Não existe ajuste por org.** O que se liga é a instalação inteira ou cada agente. Para mudar uma org
inteira, ajuste cada agente dela.

### Medir antes de mudar o padrão

Os conjuntos rotulados estão em `tests/jev/g1.jsonl` a `tests/jev/g8.jsonl`. Com o Jev real:

```bash
cd pop-harness
TYPESAFE_API_KEY='<chave do Jev>' ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml jev eval g1 tests/jev/g1.jsonl
TYPESAFE_API_KEY='<chave do Jev>' ./target/debug/poph --config /tmp/hr58b-run/a/poph.toml jev eval g1 tests/jev/g1.jsonl --json > /tmp/g1-real.json
```

- Leia acertos, abstenções e erros. O que mais pesa é a **decisão confiante errada** (`docs/JEV.md`
  explica a interpretação e traz as medições de 20/11/2026 para cada gate).
- Ao mudar o padrão, meça com o Jev real antes.

**Cuidado com o Jev falso.** `scripts/jev-falso.py` serve para testar só o encanamento, não a qualidade.
Na prova, rodei `jev eval g1` contra ele: 135 casos, 113 acertos e 22 erros, 0 falhas de chamada. Esses
números **não** servem para decidir nada. Com `g5` o falso não respondeu nenhuma pergunta (ele só cobre
alguns gates), e a saída mostrou abstenção em todos os casos. Isso é limite do falso, não do comando.

**Não verificado nesta prova:** a medição com o Jev real (sem chave) e o efeito do gate dentro de um
turno (exige o provedor de LLM).

### O que não fazer

- Não ligue um gate por padrão sem medir com o Jev real.
- Não use o Jev falso para decidir.
- Não espere ajuste por org: é por instalação ou por agente.

---

## 6. Cluster e encaminhamento entre instâncias

**Quando usar:** quando há mais de uma instância do POP Harness, cada uma com algumas organizações. Também
quando uma organização foi atendida pela instância errada, quando é preciso mudar o mapa de quem vive onde,
ou quando um pedido volta com `421` ou `503`.

**Como funciona:** todas as instâncias leem o mesmo arquivo de mapa (`cluster.toml`), e cada organização
tem uma única dona. Se um pedido chega na instância que não é a dona, ela o repassa à dona, assinado, e a
dona responde como se tivesse recebido direto. O formato do arquivo, as regras e a lista de códigos de erro
estão em `docs/CLUSTER.md` e `docs/API.md`. Aqui fica só a operação.

**Pré-requisitos:**

- um `cluster.toml` em cada instância, com o mesmo conteúdo, exceto o `instance_id` de cada uma. Ele fica
  ao lado do `poph.toml` (ou no caminho indicado por `POPH_CLUSTER`);
- o segredo do cluster (`POPH_CLUSTER_SECRET`) definido no ambiente de **todas** as instâncias, com o
  mesmo valor, antes de subir. Ele nunca vai no arquivo;
- o mesmo token global em todas as instâncias (`[admin] tokens_globais`), para que a recarga e a lista
  consolidada funcionem em qualquer uma;
- as instâncias alcançam umas às outras pelas URLs do mapa.

### Passo 1: conferir o mapa antes de subir

```bash
export POPH_CLUSTER_SECRET='<segredo do cluster, guardado fora do repositório>'
BIN=target/debug/poph
$BIN --config <poph.toml-desta-instancia> cluster check
$BIN --config <poph.toml-desta-instancia> cluster show
```

Troque `<poph.toml-desta-instancia>` pelo caminho absoluto do `poph.toml` da instância onde você está.
Os endereços `127.0.0.1:18901` e `:18902` dos exemplos são os da prova desta seção; use os da sua instalação.

Saída real de `cluster check` com o mapa válido:

```text
Mapa do cluster válido: /tmp/hr58b2-run-2/a/cluster.toml
  2 instância(s)
  fingerprint: a222897d055e72f43f624231b84a35c9
  segredo compartilhado: definido (POPH_CLUSTER_SECRET)
```

O que conferir no `cluster show`: a "instância local" é a desta máquina, e cada org aparece na linha da
instância que deve ser a dona. Sem o `cluster.toml`, o `show` diz `modo: instância única`, e isso é o
esperado numa instalação sem cluster.

Cada instância mostra duas impressões digitais. A "fingerprint" inclui a instância local, então ela é
diferente em cada uma; para comparar o mapa entre instâncias, use a "fingerprint do mapa (sem a instância
local)", ou o `cluster check --remoto` do passo 3. Esta é a mesma impressão que aparece no `/healthz` como
`cluster_map_fingerprint`.

Se o `check` reprovar, ele lista cada problema pela chave do arquivo. Exemplo real: `orgs.<id>: a instância
"c" não existe em instances`. Corrija e rode de novo; ele sai com código 1 enquanto houver problema.

### Passo 2: trocar o mapa sem reiniciar

1. Edite o `cluster.toml` da instância.
2. Recarregue essa instância (token global; token de organização não serve):

```bash
curl -s -X POST -H "Authorization: Bearer <token-global>" http://127.0.0.1:18901/admin/v1/cluster/reload
```

3. Faça o mesmo na outra instância.

A resposta traz `mudou`, `fingerprint_antes`, `fingerprint_depois`, `orgs_que_trocaram` e as instâncias
adicionadas ou removidas. Enquanto as duas instâncias não tiverem o mesmo mapa, um pedido entre elas é
recusado com `421 mapa_divergente`; nenhum pedido é processado com mapa errado.

**Recarga não move dados.** Se uma organização passa da instância `b` para a `a`, os dados continuam na `b`
até você fazer exportar e importar (runbook 3). Até lá, a nova dona não tem os dados e a organização aparece
vazia ou inexistente para quem chama. Esse caso específico não foi testado.

**Casos ruins, provados nesta prova:**

- **Arquivo com erro de digitação:** a recarga responde `400` com `cluster_invalido` e a lista dos problemas.
  O mapa em uso continua o anterior. Confira com `curl -s http://127.0.0.1:18901/healthz`: o campo
  `cluster_map_fingerprint` não muda. Corrija o arquivo e recarregue.
- **Arquivo apagado** (ou `POPH_CLUSTER` apontando para um caminho errado): a recarga responde `200` com
  `modo_depois: instancia_unica`, e a instância passa a tratar **todas** as organizações como suas. Isso é
  perigoso, porque ela pode responder por uma organização que é de outra instância. Restaure o arquivo e
  recarregue; o modo volta para `cluster` e a impressão do mapa é a de antes.

### Passo 3: conferir as instâncias entre si

```bash
$BIN --config <poph.toml-desta-instancia> cluster check --remoto
```

O comando consulta o `/healthz` de cada outra instância (prazo de 3 s cada, em paralelo). Com as duas no ar e
o mesmo mapa, a saída é esta, e o código de saída é 0:

```text
consulta remota (mapa local: 9a6b810b853d4649e51e27f2c5056959):
  b (http://127.0.0.1:18902): ok (mapa 9a6b810b853d4649e51e27f2c5056959)
  todas as instâncias respondem com o mesmo mapa
```

Os três problemas possíveis, com código de saída 1:

- **`inacessível`**: a outra instância não respondeu (fora do ar, porta ou URL errada). Provado com a `b`
  parada: `inacessível — error sending request for url (…/healthz)`.
- **`mapa divergente — ela usa <impressão>`**: o mapa da outra é diferente. Provado editando e recarregando
  só a `a`.
- **`identidade divergente`**: a outra se declara com outro `instance_id`. Não verificado nesta prova.

Sem `--remoto`, o comando só valida o arquivo local. Com `--json`, o resultado vem em formato para script.
O comando não confere a URL da própria instância.

### Passo 4: ver as organizações de todas as instâncias

```bash
curl -s -H "Authorization: Bearer <token-global>" "http://127.0.0.1:18901/admin/v1/orgs?cluster=true"
```

Cada organização traz `instancia`, e o campo `instancias` traz `situacao` por instância: `local`, `ok` ou
`inacessivel`. Quando uma instância está fora do ar, ela aparece como `inacessivel` com `orgs: 0` e uma
`mensagem`, e a lista sai com `200` com as organizações das outras. Cada instância vizinha tem prazo de 2 s.
Sem o parâmetro, a resposta mostra só as organizações desta instância. Com um token de organização, o
parâmetro é ignorado.

Provado com a `b` parada: a lista saiu com `200` em cerca de 0,4 s, com a `b` marcada como `inacessivel`.

### Passo 5: diagnosticar um erro do encaminhamento

| Resposta | O que significa | O que fazer |
|---|---|---|
| `421 mapa_divergente` | o mapa de quem encaminhou é diferente do desta instância. `details` traz `fingerprint_local` e `fingerprint_recebido` | rode `cluster check --remoto` na instância que atendeu; alinhe o `cluster.toml` das duas e recarregue as duas |
| `421 rota_em_laco` com `details.instancia_dona` | a organização não vive nesta instância, e o pedido já tinha sido encaminhado | o mapa desta instância está errado: confira com `cluster show` aqui e corrija o arquivo |
| `421 rota_em_laco` com `details.hop` | um pedido já encaminhado chegou pedindo mais um encaminhamento | sinal de mapa desencontrado; faça como acima |
| `503 instancia_fora` | a dona não respondeu (conexão recusada ou prazo). `details` traz `instancia` e `url` | suba a dona (runbook 1) e confira o `/healthz` dela. Não há nova tentativa automática: quem chamou precisa repetir |
| `401 assinatura_invalida` | assinatura não confere, carimbo fora dos 60 s, instância não declarada no mapa, ou nonce repetido | confira se `POPH_CLUSTER_SECRET` é o mesmo nas duas e reinicie se trocou; confira se o relógio das duas máquinas está certo |

Provados nesta prova: `421 mapa_divergente`, `421 rota_em_laco` (com `instancia_dona` e com `hop`), `401` por
carimbo antigo, por assinatura adulterada e por instância desconhecida, `503 instancia_fora`. A diferença de
segredo entre instâncias **não** foi testada.

**Roteiro: uma organização foi atendida pela instância errada**

1. Na instância que atendeu, rode `cluster show`: a organização aparece na linha da instância que deveria ser
   a dona?
2. Rode `cluster check --remoto`: a outra instância tem o mesmo mapa?
3. Se não, corrija o `cluster.toml` da instância errada, recarregue essa instância e depois a outra.
4. Se a resposta foi `503`, suba a instância dona.
5. Confira pela instância que atendeu: a organização deve vir igual à da dona.

### O que ainda NÃO existe

- Mover uma organização de instância automaticamente. Hoje é exportar e importar à mão (runbook 3).
- Nova tentativa ou troca automática de instância quando a dona falha no meio de um pedido.
- A tela do Console usando `?cluster=true`. A tela de operação ainda não mostra as instâncias.
- Encaminhamento com token global. O `Authorization` original segue junto, e quem valida é a dona.

### O que não fazer

- Não edite o mapa de uma instância e espere que as outras acompanhem. Recarregue as duas.
- Não apague o `cluster.toml` para "desfazer" uma mudança. A instância vira instância única e passa a
  responder por todas as organizações.
- Não troque `POPH_CLUSTER_SECRET` com o servidor no ar. A troca só vale depois de reiniciar, e até lá as
  instâncias não assinam nem conferem com o mesmo segredo.
- Não mude uma organização de instância sem antes mover os dados dela.
- Não coloque o segredo no `cluster.toml`, no `poph.toml`, em log ou em mensagem.

### Não verificado nesta prova

- Instâncias em máquinas diferentes e com HTTPS: a prova usou duas portas HTTP na mesma máquina.
- `identidade divergente` no `cluster check --remoto`.
- Recarga com token de organização (deve dar `403`): não testado nesta prova. O `401` sem token foi testado.
- Segredo diferente entre instâncias, e relógio fora da janela de 60 s: não testados.
- Chat ao vivo (SSE) encaminhado: coberto pelo teste automatizado da HR-50b-2, não repetido aqui.
- Mensagens reais do WhatsApp chegando na instância errada: não verificado, porque não há Evolution nesta
  prova.
- Mover uma organização com dados entre instâncias: não testado.

---

## Registro desta prova

| Runbook | Executado nesta prova | Não verificado (motivo) |
|---|---|---|
| 1. Subir e parar | `doctor` (sem e com as chaves de teste), `migrate` (duas vezes), `serve` em segundo plano, `/healthz`, `HTTP 200` no Console, parada com SIGINT, `doctor` depois da parada | produção (não há); porta ocupada (não testei); `kill -9` (não testei) |
| 2. Atualizar | `cargo build -p poph`, `migrate` (duas vezes, com a volta a "nada a fazer") | versão nova de verdade (o código era o mesmo); volta com banco já migrado (não testei) |
| 3. Backup | `backup export` com e sem conversas, export sem `--sobrescrever` (recusado), `backup import` numa instalação limpa e de novo (idempotente), recusa de arquivo cortado e de versão 99, conflito com outra org; HTTP: `GET backup` (200, cabeçalhos), `POST import` (200 e 409) | segredo de integração (coberto pelos testes do repositório); reindexação com modelo (sem modelo na prova); 413; 401 e 403 do backup; org com conversas (a org de prova não tinha conversa) |
| 4. Plugin | declarar, subir, lista e detalhe (`ok`, `indisponivel`, 503), `401` sem token | tela de Operação no navegador; troca de plugin (só pelo texto do código); remoção |
| 5. Gates do Jev | `agent show`, `agent update` com `--if-match` (sucesso e conflito), `jev eval g1` e `g5` com o Jev falso | Jev real; efeito no turno; volta a `{}`; medição de qualidade |
| 6. Cluster e encaminhamento | `cluster show`, `cluster check` (válido e com erro), `cluster check --remoto` (saudável, instância parada, mapa divergente), recarga com arquivo válido, inválido e apagado, `?cluster=true` com e sem parâmetro, encaminhamento de org de outra instância (200), `421 mapa_divergente`, `421 rota_em_laco`, `401` por carimbo, assinatura e instância desconhecida, `503 instancia_fora` | recarga com token de org (403); identidade divergente; segredo diferente; HTTPS e máquinas separadas; SSE e WhatsApp reais; mover org com dados |

Contagem: cerca de 65 chamadas entre CLI, HTTP e build nos runbooks 1 a 5; cerca de 60 comandos e chamadas HTTP no runbook 6. Não rodei `poph token global`.

Parte dos comandos rodou com o binário de 13:29 e parte depois de um `cargo build -p poph` que
recompilou `poph-server` e `poph`. A conferência do backup por HTTP e as chamadas de agente foram feitas
com o binário recompilado.
