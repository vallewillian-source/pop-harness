# Linguagem de fluxos do POP Harness (backend)

Versão 1.0 · 21/11/2026 · tarefa **FX-02** · especificação interna do módulo de Fluxos e Fichas
Status: **especificação do dialeto**, do esquema da IR e da lista de diagnósticos. A implementação segue este documento.

Este documento é a fonte para quem implementar o compilador (FX-10), o motor (FX-11 a FX-19) e a API (FX-27 a FX-34). Ele descreve **só o backend**: a UI do Console não muda nesta especificação.

## 0. Em uma página

1. **Um arquivo = um fluxo**, escrito em Starlark (subconjunto do Python), com declarações no nível do arquivo: `fluxo(...)`, `etapa(...)` e `decisao(...)`. `def` só para regras puras.
2. **O script descreve; o motor executa.** Nada "espera a resposta do cliente" dentro do script: o estado (etapa atual, campos preenchidos, histórico) fica no banco (`fluxo_execucoes`, `fluxo_execucao_etapas`) e o motor lê a IR para decidir o próximo passo.
3. **Duas avaliações, com limites diferentes:** a *carga* (uma vez, ao publicar ou ao abrir a revisão) coleta as declarações e monta a IR; a *regra* (a cada decisão) avalia só combinadores em Rust nativo ou uma `def` em Starlark, com prazo curto.
4. **A IR é a estrutura compilada em JSON** (`schema_versao = 1`), guardada junto do texto na revisão. A IR nunca fica na memória como JSON: o motor desserializa em structs e recompila as `def`.
5. **Cada configuração tem efeito escrito** na tabela "configurar → o que acontece no atendimento" de cada declaração (§4).
6. **Todo erro tem código, linha, coluna e sugestão em português** (§10). Publicar só acontece sem nenhum erro; avisos não bloqueiam.
7. Os três fluxos completos de exemplo ficam em [`tests/fluxos/exemplos/`](../tests/fluxos/exemplos/) e são validados a cada rodada (§11).

## 1. Escopo e relação com o resto do módulo

**Dentro deste documento:** o dialeto, o significado de cada declaração, os tipos dos dados de uma execução, as regras e combinadores, as variáveis de mensagem, o esquema da IR, os diagnósticos de forma (com o que bloqueia a publicação) e as diferenças planejadas em relação ao POP Atende.

**Fora daqui (cada um na sua tarefa):** como o Jev escolhe o fluxo (G9, FX-22), o motor de avanço de etapa e de efeitos (FX-11 a FX-21), as rotas (FX-27), as tabelas (FX-05/06), o diff entre revisões em português (FX-28), o formatador (FX-29) e a UI de edição (especificação futura).

**Decisões do mantenedor que este documento obedece** (21/11/2026):

| # | Decisão |
|---|---|
| D1 | Limites: fonte ≤ **32 KB** (recusa antes de compilar); heap **64 MB**, ticks **5.000.000** e profundidade **128** na carga; prazo **1 s** para compilar e **200 ms** para avaliar uma regra. `while` e `**` não existem no Starlark: o exemplo de abuso é `"x" * 1000000000`. |
| D2 | Os combinadores (`igual`, `e`, …) são IR declarativa avaliada em Rust nativo, **sem** passar pelo Starlark. Só as `def` rodam no Starlark, em runtime. |
| D3 | Globais: a lista é fechada e **não inclui** `fail`, `getattr`, `hasattr`, `dir` nem `type`. Não há relógio, rede, arquivo nem aleatório. |
| D4 | IR: `schema_versao = 1`, topo com `fluxo{id,nome,descricao}`, `gatilho{quando,nao_quando,exemplos}`, `inicio`, `etapas[]`, `decisoes[]`, `fins[]` e `defs[{nome,fonte}]`; ids em texto; regras serializáveis em JSON; mudança futura só **aditiva**. |
| D5 | Dois tipos de ator separados: **ator da ação** (`agente` \| `atendente`) e **ator do evento** (`agente` \| `atendente` \| `sistema` \| `jev` \| `admin`). A IR não tem campo único "ator" ambíguo. |
| D6 | Fim de fluxo **não** fecha a Ficha; só fecha com `fim(encerra_atendimento=True)`, cujo padrão é `False`. O G8 (despedida do cliente) não encerra atendimento. |
| D7 | Números medidos no FX-01 entram no documento: **92,8 kB** de RAM por fluxo de 40 nós, **p95 de 1,7 µs** por regra, **+5,03 MiB** de binário. |

**Números medidos** (spike do FX-01, `spike/fluxos/RELATORIO.md`): 92,8 kB por fluxo de 40 nós (módulo congelado + IR em texto); p95 de 1,7 µs para avaliar uma regra já carregada; +5,03 MiB de binário release. Com o teto de 60 nós (`fluxos.max_nos`), a estimativa por fluxo fica em torno de **140 kB**; a meta de RAM das revisões carregadas continua sendo a do PRD (§9, `cache_revisoes_mb = 16`).

## 2. Como o script é usado

O caso de uso manda duas coisas opostas: **seguir o fluxo exatamente** e **durar dias** (o cliente some e volta). Por isso o script é **declarativo**: ele descreve etapas, campos, ações, decisões e fins; quem guarda o estado é o banco.

```
script (.star)  ──compilar──►  IR (JSON, schema_versao 1)  ──desserializar──►  structs do motor
      │                              │                                              │
      └── revisão publicada ─────────┘                     execução (banco) ◄─────┘
                                                            etapa atual + dados + histórico
```

- **Na publicação** (`POST /flows/{id}/publish`) e em toda abertura de revisão, o motor compila o texto, confere que a IR gerada é **igual** à IR gravada (`ir_divergente` se não for) e só então usa.
- **Durante o atendimento**, cada ação do agente ou do atendente vira uma escrita no banco; cada decisão avalia uma regra; cada mudança de etapa grava dados e evento.
- **O script nunca é executado "no meio" da conversa**: não existe `esperar_resposta()`, laço de espera nem estado em memória. É por isso que uma queda do servidor não perde o fluxo.

## 3. O dialeto

### 3.1 O que é um arquivo de fluxo

- Convenção de extensão `.star`, UTF-8, até **32 KB** (32.768 bytes). Acima disso o arquivo é recusado antes de qualquer compilação (`FLX02`).
- **Um arquivo, um fluxo.** `fluxo(...)` aparece exatamente uma vez (`FLX10`).
- **No nível do arquivo só existem declarações:** chamadas a `fluxo`, `etapa` e `decisao`, mais as `def` de regra. Qualquer outro item — atribuição (`x = ...`), `for`, `if`, `class`, `import`, expressão solta (`x = "a"` ou `1 + 1`), comentário solto fora de `#` — é recusado (`FLX05`).
- **Texto livre com acento; identificador sem acento.** `id` é `[a-z][a-z0-9_]*`, de 2 a 64 caracteres (`FLX13`). Rótulos, instruções, mensagens e descrições aceitam acento e pontuação.

### 3.2 Permitido e proibido

| Construção | Situação | Por quê |
|---|---|---|
| Chamada de declaração no topo (`fluxo`, `etapa`, `decisao`) | **permitido** | é a forma do fluxo |
| `def nome(dados):` | **permitido**, só para regra pura, com um argumento chamado `dados` | regra avançada que o combinador não expressa |
| `if`, `for`, `return`, `and`, `or`, `not`, `in`, `len`, `.get(...)`, comparação, aritmética | **permitido dentro de `def`** | corpo de regra |
| `load(...)` | **proibido** | importaria código de fora do fluxo |
| `while` | **não existe no Starlark** | laço infinito seria a forma óbvia de abuso |
| `**` (expoente) | **não existe no Starlark** | idem |
| `lambda`, `class`, `try`/`except`, `raise`, `import` | **proibido** | não têm lugar num fluxo declarativo |
| f-string (`f"..."`) | **proibido** | a interpolação do fluxo é `{etapa.campo}` (§7); duas sintaxes confundiriam o autor |
| Recursão (direta ou indireta) | **proibida** | o limite de profundidade (128) a transforma em erro, não em laço infinito |
| `print`, `open`, `time`, `random`, `os`, `socket`, `eval`, `exec`, `compile`, `getattr`, `hasattr`, `dir`, `type`, `fail` | **inexistentes** | `FLX07`; não há relógio, rede, arquivo, aleatório nem introspecção |
| `struct`, `record`, `enum`, `set`, `json`, `breakpoint` | **inexistentes** | superfície desnecessária para descrever fluxo |

### 3.3 A lista fechada de nomes que o autor alcança

Todo nome que um script pode usar, e nada além disso. O compilador registra **exatamente** esta lista no ambiente do script; um teste do FX-10 falha se a lista mudar sem esta tabela mudar junto.

| Categoria | Nomes | Para que serve |
|---|---|---|
| Declarações | `fluxo`, `etapa`, `decisao` | estrutura do fluxo |
| Valores de declaração | `gatilho`, `campo`, `acao`, `fim`, `funcao` | partes de uma declaração |
| Efeitos | `mensagem_ao_cliente`, `notificar_whatsapp`, `encaminhar` | o que acontece fora do fluxo |
| Combinadores | `igual`, `diferente`, `maior`, `menor`, `contem`, `vazio`, `e`, `ou`, `nao` | regra declarativa |
| Constantes | `True`, `False`, `None` | valores |
| Funções puras (só dentro de `def`) | `abs`, `all`, `any`, `bool`, `dict`, `enumerate`, `float`, `int`, `len`, `list`, `max`, `min`, `range`, `reversed`, `sorted`, `str`, `tuple`, `zip` | cálculo de regra |

**Não há relógio, rede, arquivo nem aleatório.** O script é determinístico: o mesmo texto produz a mesma IR, sempre (provado no FX-01: duas execuções e duas threads com o mesmo resultado).

### 3.4 Limites

| Limite | Valor | Onde vale | Configuração |
|---|---|---|---|
| Tamanho da fonte | 32.768 bytes | antes de compilar | `fluxos.max_fonte_bytes` (fixado em 32 KB nesta versão) |
| Heap da carga | 64 MB | compilar/publicar | interno |
| Ticks da carga | 5.000.000 | compilar/publicar | interno |
| Profundidade de chamada | 128 | carga e regra | interno |
| Prazo da carga | 1 s | compilar/publicar | interno |
| Ticks de uma regra | 10.000 | avaliar `def` | `fluxos.max_ticks_regra` |
| Heap de uma regra | 1 MiB | avaliar `def` | `fluxos.max_heap_regra` |
| Prazo de uma regra | 200 ms | avaliar `def` | interno |
| Saltos encadeados de decisão | 10 | executar | `fluxos.max_saltos_decisao` |
| Nós por fluxo (etapas + decisões + fins) | 60 | publicar | `fluxos.max_nos` |
| Dados de uma execução | 64 KB | executar | `fluxos.max_dados_bytes` |
| Fluxos liberados por agente | 20 | permissão/G9 | fixo nesta versão |

**Armadilha conhecida do Starlark (medida no FX-01):** o limite de heap é verificado *depois* da alocação. Uma alocação única e enorme (`"x" * 1000000000`) chega a ~2 GB de RSS e quem a corta é o prazo, não o heap; laços (concatenação, lista, recursão) são contidos em dezenas de ms e dezenas de MB. Por isso o requisito de o prazo ser curto e de a compilação na publicação rodar em trabalhador com `RLIMIT_AS` (PRD §11; o desenho é da FX-10).

### 3.5 Exemplo válido e exemplo inválido (dialeto)

Válido — o menor arquivo de fluxo possível:

```python
fluxo(
    id = "triagem_rapida",
    nome = "Triagem rápida",
    inicio = "abertura",
    gatilho = gatilho(
        quando = "O cliente quer falar com uma pessoa.",
        nao_quando = "O cliente aceita ser atendido pelo agente.",
        exemplos = ["quero falar com um atendente"],
    ),
)

etapa(
    id = "abertura",
    nome = "Abertura",
    acoes = [acao("encaminhar", "Falar com uma pessoa", vai_para = "humano")],
)

etapa(
    id = "humano",
    nome = "Aguardando o atendente",
    atores = ["atendente"],
    acoes = [acao("atendido", "Atendido", ator = "atendente", vai_para = fim("concluido"))],
)
```

Inválido — `while` não existe, `load` é proibido e atribuição no topo não é declaração:

```python
contador = 0                      # FLX05: no topo só pode haver declaração
load("outro.star", "ajuda")       # FLX03: `load` é proibido no dialeto
def gira(dados):
    while True:                   # FLX03: `while` não existe
        pass
```

## 4. As declarações

Convenções desta seção: **posicional** = argumento sem nome, na ordem mostrada; **nomeado** = `chave = valor`. Só os argumentos listados existem: qualquer outro dá `FLX12` (é assim que um `obrigatorioo` digitado errado aparece na hora, e não em silêncio).

### 4.1 `fluxo(...)` — o cabeçalho do fluxo

```python
fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(...),
)
```

| Argumento | Tipo | Obrigatório | Padrão | O que faz |
|---|---|---|---|---|
| `id` | texto (id) | sim | — | identifica o fluxo na organização; vira o `slug` e aparece no nome das ferramentas e na API. Não muda depois de publicado (trocar o `id` cria outro fluxo) |
| `nome` | texto | sim | — | nome legível, mostrado ao atendente e no catálogo do G9 |
| `descricao` | texto | não | vazio | uma frase para o catálogo; não vai para o cliente |
| `inicio` | texto (id de etapa) | sim | — | a etapa em que a execução começa |
| `gatilho` | `gatilho(...)` | sim | — | o texto que o Jev usa para decidir se o fluxo começa (§4.2) |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `id` | é a chave do fluxo na org; o agente só usa o fluxo depois de liberado, e o `id` é o que a permissão e o G9 carregam |
| `nome` | aparece no catálogo do Jev, no bloco do prompt e no Console |
| `descricao` | aparece no catálogo e nas listas; nunca é enviada ao cliente |
| `inicio` | define a primeira etapa; sem ela o arquivo não publica |
| `gatilho` | é o único texto que o Jev lê para decidir iniciar; sem ele o fluxo não entra no G9 |

Válido: o exemplo acima. Inválido:

```python
fluxo(
    id = "Agendar Visita",          # FLX13: id com espaço e maiúscula
    nome = "Agendar visita",
    # falta `inicio`               # FLX11: argumento obrigatório ausente
    gatilho = gatilho(...),
)
```

### 4.2 `gatilho(...)` — quando o fluxo deve começar

```python
gatilho(
    quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
    nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
    exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
)
```

| Argumento | Tipo | Obrigatório | Padrão | O que faz |
|---|---|---|---|---|
| `quando` | texto ≤ 400 caracteres (recomendado) | sim | — | critério positivo do G9: a situação em que o fluxo serve |
| `nao_quando` | texto ≤ 400 | não (aviso `FLX57` se faltar) | vazio | critério negativo; evita que o fluxo capture conversa parecida mas errada |
| `exemplos` | lista de 1 a 5 textos | não (aviso `FLX56` se faltar) | `[]` | 2 ou 3 falas típicas do cliente; melhora a escolha do Jev |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `quando` | entra nas opções do G9 como critério; é o que o Jev compara com a conversa |
| `nao_quando` | idem, como critério de exclusão |
| `exemplos` | entram no mesmo critério do G9, como amostras de fala |
| (qualquer campo) | sem fluxo liberado ao agente ou com o motor desligado, nada disso é lido: o G9 não pergunta |

Este texto **não é enviado ao cliente** e **não interpola variável** (`FLX26`).

Válido: o exemplo acima. Inválido:

```python
gatilho(
    quandoo = "O cliente quer agendar.",   # FLX12: argumento desconhecido
)                                          # FLX11: falta `quando`; FLX56 e FLX57 são avisos
```

### 4.3 `etapa(...)` — um passo do atendimento

```python
etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    atores = ["agente"],
    campos = [campo("endereco", "Endereço completo", obrigatorio = True)],
    ao_entrar = [mensagem_ao_cliente("Vou anotar alguns dados para a visita.")],
    acoes = [acao("confirmar", "Dados completos", exige = ["endereco"], vai_para = "confirmacao")],
)
```

| Argumento | Tipo | Obrigatório | Padrão | O que faz |
|---|---|---|---|---|
| `id` | texto (id) | sim | — | identifica a etapa no fluxo (e nas variáveis `{etapa.campo}`) |
| `nome` | texto | sim | — | nome curto que o agente e o atendente veem |
| `instrucao` | texto ≤ 1000 (recomendado) | não | vazio | o que o agente deve fazer nesta etapa; entra no bloco volátil do prompt |
| `atores` | lista de `"agente"` \| `"atendente"` | não | `["agente"]` | **quem pode executar ação** nesta etapa |
| `campos` | lista de `campo(...)` | não | `[]` | dados que a etapa coleta |
| `acoes` | lista de `acao(...)` | não (aviso `FLX51` se vazia) | `[]` | as saídas da etapa |
| `ao_entrar` | lista de efeitos | não | `[]` | o que dispara ao entrar na etapa, antes de qualquer campo dela ser preenchido |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `id` | é o valor que aparece em `{etapa.campo}` e para onde `vai_para`/`entao`/`senao` apontam |
| `nome` | aparece no prompt e no histórico da Ficha; o cliente não vê |
| `instrucao` | orienta o agente na etapa; é parte volátil do prompt (não entra no prefixo estável, para não quebrar o cache) |
| `atores` | define quem consegue agir; ação de ator não listado é recusada na validação (`FLX19`) |
| `campos` | cada campo vira uma coisa a perguntar; só é aceito valor do tipo certo (§5) |
| `acoes` | são as únicas saídas da etapa; pedir ação inexistente devolve erro com a lista de ações válidas |
| `ao_entrar` | dispara **ao entrar** na etapa — a mensagem sai mesmo antes de a etapa ser preenchida; variável do próprio passo aqui sai vazia (aviso `FLX63`) |

Válido: o exemplo acima. Inválido:

```python
etapa(
    id = "coleta",                        # FLX13 se o id tiver maiúscula ou espaço
    # falta `nome`                        # FLX11
    atores = ["cliente"],                 # FLX19: cliente não é ator; use agente ou atendente
    campos = [campo("cep", "CEP", tipo = "opcao")],   # FLX18: opcao sem opções
    acoes = [acao("concluir", "Concluir", exige = ["endereco"], vai_para = "fim_x")],  # FLX17 e FLX15
)
```

### 4.4 `campo(...)` — um dado que a etapa coleta

```python
campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True)
campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"])
```

| Argumento | Tipo | Obrigatório | Padrão | O que faz |
|---|---|---|---|---|
| `id` (posicional) | texto (id) | sim | — | nome do campo dentro da etapa; é o segundo pedaço de `{etapa.campo}` |
| `rotulo` (posicional) | texto | sim | — | o que o agente usa para perguntar e o atendente vê |
| `tipo` | `texto` \| `numero` \| `data` \| `booleano` \| `opcao` \| `anexo` | não | `texto` | valida o valor que o agente registra (§5) |
| `obrigatorio` | booleano | não | `False` | marca o campo como necessário para a etapa estar completa |
| `opcoes` | lista de textos (2+) | só quando `tipo = "opcao"` | `[]` | os valores aceitos; qualquer outro é recusado |
| `ajuda` | texto | não | vazio | dica para quem edita o fluxo (UI futura); o motor ignora |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `id` | é a chave do valor em `dados` e nas variáveis de mensagem |
| `rotulo` | vira a pergunta do agente; o cliente vê o que o agente escrever, não o rótulo cru |
| `tipo` | valor fora do tipo é **recusado com a lista de erros** (nada é gravado pela metade) |
| `obrigatorio` | aparece na conferência de campos; ação que `exige` o campo só passa com ele preenchido |
| `opcoes` | o agente só aceita um dos valores; `igual(...)` contra valor fora da lista vira aviso (`FLX54`) |
| `ajuda` | nenhum efeito no atendimento |

Válido: os dois exemplos acima. Inválido:

```python
campo("cep", "CEP")                                   # válido: tipo texto, opcional
campo("cep", "CEP", tipo = "telefone")                # FLX20: tipo desconhecido
campo("turno", "Turno", tipo = "opcao")               # FLX18: opcao precisa de 2 opções ou mais
campo("turno", "Turno", tipo = "opcao", opcoes = ["manhã", "manhã"])   # FLX18: opção repetida
campo("nome", "Nome", obrigatorio = "sim")            # FLX06: `obrigatorio` é booleano
campo("nome", "Nome", obrigatorioo = True)            # FLX12: argumento desconhecido
```

### 4.5 `acao(...)` — uma saída da etapa

```python
acao(
    "confirmar",
    "Dados completos",
    exige = ["endereco", "periodo"],
    vai_para = "confirmacao",
    ator = "agente",
    efeitos = [notificar_whatsapp(para = "5521999990000", mensagem = "Pedido {coleta.endereco}")],
)
```

| Argumento | Tipo | Obrigatório | Padrão | O que faz |
|---|---|---|---|---|
| `id` (posicional) | texto (id) | sim | — | nome da ação na etapa; é o que o agente chama em `fluxo_executar_acao` |
| `rotulo` (posicional) | texto | sim | — | descrição legível, usada pelo agente e pelo atendente |
| `vai_para` | id de etapa, id de decisão ou `fim(...)` | sim | — | para onde a execução vai quando a ação é executada |
| `exige` | lista de ids de campo **da própria etapa** | não | `[]` | campos que precisam estar preenchidos para a ação valer |
| `ator` | `"agente"` \| `"atendente"` | não | vazio = qualquer ator de `atores` | restringe quem pode executar esta ação |
| `efeitos` | lista de efeitos (§4.9) | não | `[]` | dispara mensagem, notificação ou encaminhamento junto da ação |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `id` | é o nome que o modelo precisa usar; nome inventado devolve erro com a lista de ações válidas (não há transição escondida) |
| `rotulo` | aparece na lista de ações que o agente recebe no prompt |
| `vai_para` | a execução muda de etapa/decisão na mesma escrita; `fim(...)` conclui a execução |
| `exige` | faltando campo, a ação é recusada e a resposta diz **exatamente quais faltam** |
| `ator` | ação restrita ao atendente não pode ser executada pelo agente, e vice-versa |
| `efeitos` | são disparados **depois** de a transição ser aceita; falha de efeito não desfaz a transição e vira evento |

Válido: o exemplo acima. Inválido:

```python
acao("confirmar", "Dados completos")                          # FLX11: falta `vai_para`
acao("confirmar", "Dados completos", vai_para = "coleta_x")   # FLX15: etapa inexistente
acao("confirmar", "Dados completos", vai_para = "confirmacao", exige = ["cep"])   # FLX17: campo não existe na etapa
acao("confirmar", "Dados completos", vai_para = "confirmacao", ator = "cliente")  # FLX19: ator inválido
acao("confirmar", "Dados completos", vai_para = "confirmacao", efeitos = [enviar_email("x")])  # FLX05: efeito desconhecido
```

### 4.6 `decisao(...)` — uma bifurcação

```python
decisao(
    id = "tipo_de_periodo",
    se = igual("coleta.periodo", "manhã"),
    entao = "visita_manha",
    senao = "visita_tarde",
)
```

| Argumento | Tipo | Obrigatório | Padrão | O que faz |
|---|---|---|---|---|
| `id` | texto (id) | sim | — | nome da decisão; é destino válido de `vai_para` e de outro ramo |
| `se` | combinador ou `funcao("nome")` | sim | — | a regra avaliada com os dados do momento |
| `entao` | id de etapa, id de decisão ou `fim(...)` | sim | — | destino quando a regra é verdadeira |
| `senao` | id de etapa, id de decisão ou `fim(...)` | sim | — | destino quando é falsa |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `se` | é avaliado a cada passagem, **com os dados já registrados**; não guarda resultado |
| `entao` / `senao` | o motor segue o ramo na mesma transação e grava o evento `decisao_resolvida` com o resultado |
| (encadeamento) | decisão que leva a outra decisão é seguida até **10 saltos**; acima disso a execução falha (`FLXR04`); ciclo entre decisões é recusado na publicação (`FLX23`) |
| (voltar a uma etapa) | decisão pode voltar a uma etapa já visitada — o histórico por etapa guarda cada visita |

Válido: o exemplo acima. Inválido:

```python
decisao(id = "classificar", se = "afeta_agora == sim", entao = "urgente", senao = "normal")   # FLX19: `se` não é combinador nem `funcao(...)`
decisao(id = "classificar", se = igual("relato.cep", "1"), entao = "urgente", senao = "normal")  # FLX22: campo não existe
decisao(id = "classificar", se = funcao("urgente"), entao = "urgente", senao = "normal")        # FLX19: `def` não existe
decisao(id = "classificar", se = e(igual("relato.afeta_agora", "sim")), entao = "urgente", senao = "normal")  # FLX21: `e` com 1 argumento
decisao(id = "a", se = funcao("grave"), entao = "b", senao = "b")   # válido
decisao(id = "b", se = funcao("grave"), entao = "a", senao = "normal")   # FLX23: ciclo a → b → a
```

### 4.7 `fim(...)` — o encerramento da execução

`fim(...)` não é uma declaração do topo: é um **valor** usado em `vai_para`, `entao` ou `senao`. Todos os fins usados no arquivo aparecem em `fins[]` na IR, na ordem da primeira aparição.

```python
fim("concluido", resultado = "Visita agendada")
fim("concluido", resultado = "Visita agendada", encerra_atendimento = True)
```

| Argumento | Tipo | Obrigatório | Padrão | O que faz |
|---|---|---|---|---|
| `id` (posicional) | texto (id) | sim | — | nome do desfecho; aparece no resultado da execução |
| `resultado` | texto | não | vazio | resumo gravado em `fluxo_execucoes.resultado` e no histórico da Ficha |
| `encerra_atendimento` | booleano | não | `False` | `True` **também** encerra o atendimento: fecha a conversa e a Ficha na mesma operação |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `id`/`resultado` | aparecem no histórico da Ficha e nas métricas; o cliente não vê |
| `encerra_atendimento = False` (padrão) | a execução fica `concluida`; a conversa **continua** com o agente, sem fluxo ativo — o cliente pode entrar em outro fluxo na mesma Ficha |
| `encerra_atendimento = True` | encerra o atendimento: fecha conversa e Ficha e registra `fechada_por = fluxo`; mensagem seguinte do cliente abre outra conversa e outra Ficha |
| (nenhum fim alcançável) | aviso na publicação (`FLX52` aponta a etapa sem saída); o fluxo nunca termina sozinho |

Válido: os exemplos acima. Inválido:

```python
fim("concluido", encerra = True)          # FLX12: argumento desconhecido
fim("concluido", encerra_atendimento = "sim")   # FLX06: precisa ser True/False
acao("ok", "Pronto", vai_para = fim("x", resultado = "a"))
acao("ok2", "Pronto", vai_para = fim("x", resultado = "b"))   # FLX16: mesmo `fim` com argumentos diferentes
```

### 4.8 Combinadores

Os combinadores montam a **IR declarativa** da regra. Eles não executam nada no Starlark: quem avalia é o motor em Rust nativo (D2). Isso é o que dá a latência medida (p95 1,7 µs) e o que deixa a UI futura editar a lógica sem entender de código.

| Combinador | Forma | Verdadeiro quando |
|---|---|---|
| `igual` | `igual("etapa.campo", "valor")` | o valor do campo, em texto canônico (§5), é igual ao valor — sem espaços nas pontas |
| `diferente` | `diferente("etapa.campo", "valor")` | o valor canônico é diferente |
| `maior` | `maior("etapa.campo", "10")` | o valor numérico é **maior**; campo que não é `numero` gera aviso `FLX60` e o teste é falso se o texto não for número |
| `menor` | `menor("etapa.campo", "10")` | idem, menor |
| `contem` | `contem("etapa.campo", "atras")` | o campo contém o trecho, **sem diferenciar maiúscula nem acento** |
| `vazio` | `vazio("etapa.campo")` | o campo não foi preenchido, é nulo ou só tem espaços |
| `e` | `e(a, b, ...)` | todas as condições verdadeiras (2 ou mais) |
| `ou` | `ou(a, b, ...)` | qualquer condição verdadeira (2 ou mais) |
| `nao` | `nao(a)` | inverte (exatamente 1 argumento) |
| `funcao` | `funcao("nome")` | chama a `def nome(dados)` do arquivo e usa o `True`/`False` devolvido |

Regras de combinação: `e`/`ou` podem receber outros combinadores (aninhamento livre, dentro do prazo de 200 ms); `funcao` pode entrar em qualquer ponto; o campo é sempre `etapa.campo` (`FLX21` se não for) e a etapa e o campo precisam existir (`FLX22`).

Válido:

```python
decisao(
    id = "prioridade",
    se = e(
        igual("relato.afeta_agora", "sim"),
        nao(vazio("relato.resumo")),
        ou(contem("relato.resumo", "parado"), funcao("problema_grave")),
    ),
    entao = "urgente",
    senao = "normal",
)
```

Inválido:

```python
igual("periodo", "manhã")                 # FLX21: falta a etapa (`coleta.periodo`)
igual("coleta.periodo")                   # FLX21: falta o valor
maior("coleta.periodo", "10")             # válido, com aviso FLX60 (campo não é numero)
nao(igual("coleta.periodo", "manhã"), igual("coleta.periodo", "tarde"))   # FLX21: `nao` recebe 1
igual("coleta.data", "2026-01-01")        # FLX22: campo não existe na etapa
```

### 4.9 Efeitos

Efeito é o que acontece **fora** do fluxo: falar com o cliente, avisar a equipe, mandar a conversa para outro departamento. Ficam em `ao_entrar` da etapa ou em `efeitos` da ação.

| Efeito | Forma | O que faz | Entrega |
|---|---|---|---|
| `mensagem_ao_cliente` | `mensagem_ao_cliente("texto")` | manda um texto fixo ao cliente, já interpolado | fila de saída da conversa (`outbox`), depois da resposta do turno; fora do turno, com `turn_id` novo |
| `notificar_whatsapp` | `notificar_whatsapp(para = "5521999990000", mensagem = "texto")` | manda mensagem para **outro número** (equipe) | fila própria do fluxo (`fluxo_efeitos`), porque a `outbox` é por conversa |
| `encaminhar` | `encaminhar("Logística")` | transfere a conversa para o departamento (nome na org) | mesma operação de transferência do turno; a Ficha **continua aberta** e o agente deixa de responder |

**Configurar → o que acontece no atendimento**

| Você configura | No atendimento |
|---|---|
| `mensagem_ao_cliente` em `ao_entrar` | o cliente recebe o texto ao entrar na etapa, **antes** de o agente responder |
| `mensagem_ao_cliente` em `efeitos` de ação | o texto sai junto com a transição que a ação provocou |
| `notificar_whatsapp` | a equipe recebe no número indicado; `para` é obrigatório e precisa de DDI (`FLX28`) |
| `encaminhar` | a conversa vai para o departamento; a pessoa que assume vê a Ficha aberta; o agente para de responder naquela conversa (D12) |
| efeito sem variável | aviso `FLX58` (a equipe recebe sempre o mesmo texto) |
| efeito com variável inexistente | aviso `FLX59` na publicação; no atendimento, o trecho sai vazio |
| falha de entrega | o efeito fica `falhou` com tentativas e espera crescente; **a transição não é desfeita** e o caso vira evento na Ficha |

Válido: o exemplo de `confirmar_entrega.star` (§11). Inválido:

```python
mensagem_ao_cliente()                                          # FLX11: falta o texto
notificar_whatsapp(para = "123", mensagem = "Pedido entregue") # FLX28: número sem DDI
notificar_whatsapp(para = "5521999990000", mensagem = "Pedido entregue")  # válido, com aviso FLX58
encaminhar()                                                   # FLX11: falta o departamento
enviar_email("x")                                              # FLX05: efeito desconhecido
mensagem_ao_cliente("Olá {cliente.nome}")                      # FLX25: `cliente.nome` não existe; use {cliente.peer}
```

## 5. Tipos de campo e os dados de uma execução

Os dados ficam em `fluxo_execucoes.dados_json`, no formato `{"etapa": {"campo": valor}}` (o mesmo formato que o POP Atende usa em `metadata_json`, que as regras já leem). `def` recebe exatamente esse dicionário.

| Tipo | Valor guardado | Texto canônico (usado por `igual`, `contem`, variável) | Observação |
|---|---|---|---|
| `texto` | string | a própria string, sem espaços nas pontas | texto do cliente é **dado**, nunca instrução |
| `numero` | número JSON | `123`, `12.5` (ponto decimal, sem zero à direita) | `maior`/`menor` só valem aqui |
| `data` | string ISO 8601 (`AAAA-MM-DD` ou `AAAA-MM-DDTHH:MM:SS-03:00`) | a própria string | comparação de `data` é por instante; `igual` compara o texto |
| `booleano` | `true` / `false` | `sim` / `nao` | no script, `igual(..., "sim")`, `"nao"`, `"true"`, `"false"`, `"1"` e `"0"` são aceitos e normalizados |
| `opcao` | string com o **id da opção** | o id da opção | `opcoes` é fechado; valor fora é recusado |
| `anexo` | objeto `{"nome", "tipo", "tamanho"}` | `nome` do arquivo | o conteúdo não passa pelo fluxo |

Regras gerais:

- Valor de tipo errado, opção fora da lista ou campo de outra etapa: **recusado com a lista de erros**, nada é gravado pela metade.
- Acima de **64 KB** de `dados_json`, a ação é recusada (`fluxos.max_dados_bytes`).
- Campo vazio é ausente: `vazio("etapa.campo")` cobre não preenchido, `null` e só espaços.
- `contem` ignora maiúscula/minúscula e acento (é busca em fala de cliente); `igual` **não** ignora (é comparação de valor).

## 6. Regras: como são avaliadas

| | Combinador | `def` |
|---|---|---|
| Onde a lógica está | na IR, como árvore JSON | no texto do script, em Starlark |
| Quem avalia | motor, em Rust nativo | Starlark, dentro do prazo e dos limites da regra |
| Custo medido | p95 1,7 µs | ~mesma ordem, com o custo de uma chamada |
| A UI futura edita | sim, é estrutura | não; fica como texto avançado |
| Determinismo | total (declarativo) | total (sem relógio/rede/arquivo/aleatório) |
| Erro na avaliação | não existe: campo ausente = vazio | `FLXR01`/`FLXR02`/`FLXR03`, execução `falha` |

**Contrato de uma `def` de regra:**

```python
def problema_grave(dados):
    resumo = dados.get("relato", {}).get("resumo", "").lower()
    return len(resumo) > 200
```

- Nome: id normal (`[a-z][a-z0-9_]*`), nunca igual a um nome reservado do dialeto (`FLX24`).
- Um único argumento, chamado `dados`.
- Devolve `True` ou `False`. Outro valor → `FLXR02` e execução `falha` (não "verdadeiro por acaso").
- Só lê `dados` e usa as funções puras da lista fechada (§3.3). Não escreve: é regra, não efeito.
- Pode conter `if`, `for`, `return`, `len`, `.get(...)`, comparação. Não pode recursão.
- `funcao("nome")` é a única forma de usar a `def` numa decisão; chamar `problema_grave(dados)` no corpo do script não é permitido (não há dados na carga).

**Ordem de avaliação de uma decisão:** o motor lê os dados atuais (snapshot da execução), avalia a árvore, segue `entao` ou `senao`. Cada passagem grava o evento `decisao_resolvida` com o resultado e o destino. Se o destino for outra decisão, repete — até 10 saltos (`FLXR04`). A execução **não** fica em memória entre turnos.

## 7. Variáveis nas mensagens

Só existem três formas, e só nos textos que são enviados:

| Variável | Vale | Exemplo de uso |
|---|---|---|
| `{etapa.campo}` | o valor de um campo já registrado, em texto canônico (§5) | `"Confirmando o pedido {identificacao.pedido}."` |
| `{ficha.id}` | o id da Ficha do atendimento | `"(ficha {ficha.id})"` para a equipe localizar |
| `{cliente.peer}` | o identificador do cliente no canal (no WhatsApp, o número) | `"Cliente {cliente.peer} pediu retorno."` |

Onde interpolam: `mensagem_ao_cliente(texto)` e `notificar_whatsapp(mensagem = ...)`. **Em qualquer outro texto** (`nome`, `rotulo`, `instrucao`, `quando`, `nao_quando`, `descricao`, `resultado`, `ajuda`), `{...}` é **erro** (`FLX26`): esses textos são lidos como estão pelo agente, pelo Jev e pelas telas.

Comportamento:

- Variável de etapa/campo que **não existe** no fluxo: aviso na publicação (`FLX59`); no atendimento, sai **vazio** (o PRD pede aviso em vez de silêncio, como era no POP Atende).
- Variável de campo **ainda não preenchido**: sai vazia, e isso é normal (o texto é montado no momento do envio).
- Variável do **próprio passo** em `ao_entrar`: sai vazia por definição, porque `ao_entrar` dispara antes de a etapa ser preenchida — aviso `FLX63`.
- Formato inválido (`{pedido}`, `{coleta}`, `{cliente.nome}`): erro `FLX25`.
- Texto de campo do cliente **nunca** é reinterpretado: se o cliente escrever `{ficha.id}`, isso entra no dado como texto e não é substituído.

## 8. A estrutura compilada (IR)

### 8.1 Regras do esquema

- `schema_versao = 1`, no topo. O motor lê todas as versões publicadas; o número só muda em mudança incompatível.
- **Mudança futura só aditiva:** campo novo é opcional, com padrão definido; nenhum campo existente muda de significado nem é reinterpretado. Campo desconhecido numa IR mais nova é ignorado com aviso no log, nunca erro.
- IDs em texto; regras serializáveis em JSON (nada de expressão em texto livre dentro da IR — o texto livre fica em `defs[].fonte`).
- **Uma IR, uma revisão:** a IR vive em `fluxo_revisoes.definicao_json` junto de `fonte` e `fonte_sha256`. A IR não é recalculada em execução: ela é gravada na publicação e recompilada só para conferir igualdade (`ir_divergente` se divergir).
- **A IR não carrega ator de evento.** O que existe é `atores_acao` (etapa) e `ator` (ação), que respondem "quem pode agir aqui" (D5). O ator do **evento** da Ficha (`agente`/`atendente`/`sistema`/`jev`/`admin`) é gravado em `ficha_eventos.ator_tipo` no momento do evento, não no fluxo.

### 8.2 O esquema, campo a campo

| Caminho | Tipo | Obrigatório | O que é |
|---|---|---|---|
| `schema_versao` | número | sim | `1` |
| `fluxo.id` | texto | sim | id do fluxo (slug na org) |
| `fluxo.nome` | texto | sim | nome legível |
| `fluxo.descricao` | texto | não | frase de catálogo |
| `gatilho.quando` | texto | sim | critério positivo do G9 |
| `gatilho.nao_quando` | texto | não | critério negativo |
| `gatilho.exemplos` | lista de textos | não | falas típicas |
| `inicio` | texto | sim | id da etapa de entrada |
| `etapas[].id` / `.nome` / `.instrucao` | texto | id e nome sim; instrucao não | — |
| `etapas[].atores_acao` | lista de `"agente"`/`"atendente"` | sim, padrão `["agente"]` | quem executa ação |
| `etapas[].campos[]` | lista de campo | não | ver abaixo |
| `campos[].id` / `.rotulo` | texto | sim | — |
| `campos[].tipo` | `texto`/`numero`/`data`/`booleano`/`opcao`/`anexo` | sim | — |
| `campos[].obrigatorio` | booleano | sim, padrão `false` | — |
| `campos[].opcoes` | lista de textos | não | só com `tipo = "opcao"` |
| `campos[].ajuda` | texto | não | dica de edição; sem efeito no atendimento |
| `etapas[].acoes[].id` / `.rotulo` | texto | sim | — |
| `acoes[].exige` | lista de textos | sim, padrão `[]` | campos da própria etapa |
| `acoes[].ator` | `"agente"`/`"atendente"`/`null` | sim, padrão `null` (qualquer ator da etapa) | — |
| `acoes[].vai_para` | destino (abaixo) | sim | para onde a ação leva |
| `acoes[].efeitos` | lista de efeito | sim, padrão `[]` | — |
| `etapas[].ao_entrar` | lista de efeito | sim, padrão `[]` | — |
| `decisoes[].id` | texto | sim | — |
| `decisoes[].se` | regra (abaixo) | sim | — |
| `decisoes[].entao` / `.senao` | destino | sim | — |
| `fins[].id` / `.resultado` | texto | id sim; resultado pode ser `null` | — |
| `fins[].encerra_atendimento` | booleano | sim, padrão `false` | — |
| `defs[].nome` | texto | sim | nome da `def` |
| `defs[].fonte` | texto | sim | o trecho do script, recompilado na carga |

Na IR **gerada pelo compilador**, todo campo desta tabela aparece: o que o script não declarou entra com o padrão (`[]`, `null`, `false`, `"agente"`). O exemplo da §8.3 sai do validador do spike, que ainda omite as chaves vazias — a partir da FX-10 a IR materializa os padrões, e é essa forma que vai para `definicao_json`.

**Destino** (`vai_para`, `entao`, `senao`, e o que `fim` vira dentro deles):

```json
{"tipo": "etapa",    "id": "confirmacao"}
{"tipo": "decisao",  "id": "classificar"}
{"tipo": "fim",      "id": "concluido", "resultado": "Visita agendada", "encerra_atendimento": true}
```

**Regra** (`se`): um nó por combinador. `campo` é sempre `"etapa.campo"` em texto.

```json
{"tipo": "igual",     "campo": "relato.afeta_agora", "valor": "sim"}
{"tipo": "diferente", "campo": "coleta.periodo",     "valor": "manhã"}
{"tipo": "maior",     "campo": "pedido.valor",       "valor": "1000"}
{"tipo": "menor",     "campo": "pedido.valor",       "valor": "10"}
{"tipo": "contem",    "campo": "relato.resumo",      "valor": "atras"}
{"tipo": "vazio",     "campo": "relato.resumo"}
{"tipo": "e",         "args": [ ... ]}
{"tipo": "ou",        "args": [ ... ]}
{"tipo": "nao",       "args": [ ... ]}
{"tipo": "funcao",    "nome": "problema_grave"}
```

**Efeito:**

```json
{"tipo": "mensagem_ao_cliente", "texto": "Vou confirmar a entrega do pedido {identificacao.pedido} com você."}
{"tipo": "notificar_whatsapp",  "para": "5521999990000", "mensagem": "Pedido {identificacao.pedido} entregue (ficha {ficha.id})."}
{"tipo": "encaminhar",          "departamento": "Logística"}
```

`encaminhar` guarda o **nome** do departamento; o id do departamento no momento da publicação vai para `fluxo_revisoes.resolucoes_json` (o mesmo mecanismo previsto no PRD §7.1). Departamento que não existe na org é aviso na publicação e efeito `falhou` no atendimento, com evento.

### 8.3 Exemplo real: IR de `agendar_visita.star`

Gerada pelo validador do spike sobre o arquivo de `tests/fluxos/exemplos/` (`spike/fluxos/evidencias/fx02-ir/agendar_visita.json`):

```json
{
  "decisoes": [],
  "defs": [],
  "etapas": [
    {
      "acoes": [
        {
          "exige": [
            "endereco",
            "periodo"
          ],
          "id": "confirmar",
          "rotulo": "Dados completos",
          "vai_para": {
            "id": "confirmacao",
            "tipo": "etapa"
          }
        },
        {
          "id": "desistir",
          "rotulo": "Cliente desistiu",
          "vai_para": {
            "id": "desistencia",
            "resultado": "Cliente desistiu",
            "tipo": "fim"
          }
        }
      ],
      "campos": [
        {
          "id": "endereco",
          "obrigatorio": true,
          "rotulo": "Endereço completo",
          "tipo": "texto"
        },
        {
          "id": "periodo",
          "obrigatorio": true,
          "opcoes": [
            "manhã",
            "tarde"
          ],
          "rotulo": "Período",
          "tipo": "opcao"
        }
      ],
      "id": "coleta",
      "instrucao": "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
      "nome": "Coletar dados"
    },
    {
      "acoes": [
        {
          "id": "ok",
          "rotulo": "Visita combinada",
          "vai_para": {
            "id": "concluida",
            "resultado": "Visita agendada",
            "tipo": "fim"
          }
        }
      ],
      "id": "confirmacao",
      "instrucao": "Repita o endereço e o período combinados e peça a confirmação do cliente.",
      "nome": "Confirmar visita"
    }
  ],
  "fins": [
    {
      "encerra_atendimento": false,
      "id": "desistencia",
      "resultado": "Cliente desistiu"
    },
    {
      "encerra_atendimento": false,
      "id": "concluida",
      "resultado": "Visita agendada"
    }
  ],
  "fluxo": {
    "descricao": "Marca uma visita técnica no endereço do cliente.",
    "id": "agendar_visita",
    "nome": "Agendar visita"
  },
  "gatilho": {
    "exemplos": [
      "queria agendar uma visita",
      "pode vir aqui amanhã?"
    ],
    "nao_quando": "Só quer saber preço ou prazo, sem pedir visita.",
    "quando": "O cliente quer marcar, remarcar ou cancelar uma visita técnica."
  },
  "inicio": "coleta",
  "schema_versao": 1
}
```

### 8.4 Exemplo real: decisão com combinador e `def`

Trecho da IR de `triagem_suporte.star`:

```json
{
  "decisoes": [
    {
      "entao": {
        "id": "urgente",
        "tipo": "etapa"
      },
      "id": "classificar",
      "se": {
        "args": [
          {
            "campo": "relato.afeta_agora",
            "tipo": "igual",
            "valor": "sim"
          },
          {
            "nome": "problema_grave",
            "tipo": "funcao"
          }
        ],
        "tipo": "e"
      },
      "senao": {
        "id": "normal",
        "tipo": "etapa"
      }
    }
  ],
  "defs": [
    {
      "nome": "problema_grave",
      "fonte": "def problema_grave(dados):\n    # Regra avançada: recebe os dados do fluxo e devolve True ou False.\n    # Só lê o que está na ficha; não tem relógio, rede, arquivo nem aleatório.\n    resumo = dados.get(\"relato\", {}).get(\"resumo\", \"\").lower()\n    for palavra in [\"parado\", \"não abre\", \"nao abre\", \"perdi\", \"urgente\"]:\n        if palavra in resumo:\n            return True\n    return len(resumo) > 200"
    }
  ]
}
```

Repare em duas coisas: (1) `se` é uma árvore de combinadores com a `funcao` como folha — a UI futura edita a árvore e ignora o texto da `def`; (2) `defs[].fonte` guarda o texto exato, para o motor recompilar e conferir a IR.

## 9. Forma canônica e ida e volta

Duas operações diferentes, de propósito:

| Operação | Entrada | Saída | Comentários |
|---|---|---|---|
| **Formatar** (`POST /flows/format`, CLI `poph flow formatar`) | texto do script | o mesmo script, formatado | **preservados** |
| **Gerar** (UI futura, a partir da IR) | IR | script | **perdidos** (a IR não guarda comentário) |

Regras da forma canônica (para o formatador, FX-29):

- Ordem das declarações preservada: `fluxo`, `etapa`/`decisao` na ordem do arquivo, `def` no fim.
- `campo`/`acao`/`fim` com os dois (um) primeiros argumentos posicionais; o resto nomeado.
- `True`/`False`, aspas duplas, 4 espaços de indentação, uma linha em branco entre declarações, vírgula final mantida em listas de várias linhas.
- Acentos preservados exatamente.
- Ida e volta (A2 do PRD): `texto → IR → texto → IR` produz **a mesma IR**.

## 10. Diagnósticos

Lista **fechada**: todo diagnóstico tem código; código novo entra aqui primeiro. Publicar exige **zero erro**; aviso não bloqueia. A mensagem é em português, com linha e coluna do trecho (o formato exato da resposta está em `API.md`, quando a rota existir — FX-27).

### 10.1 Erros (bloqueiam a publicação)

| Código | Quando aparece | Mensagem (modelo) |
|---|---|---|
| `FLX01` | sintaxe inválida; o parser aponta linha e coluna | "Erro de sintaxe na linha 12, coluna 5: falta fechar a lista `[`." |
| `FLX02` | fonte acima de 32 KB | "O script tem 41 KB e o limite é 32 KB. Tire textos longos das instruções ou divida o fluxo em dois." |
| `FLX03` | construção fora do dialeto: `load`, `while`, `lambda`, `class`, `try`, `import`, `raise`, f-string, `**` | "`load` não faz parte da linguagem de fluxos. Um fluxo não importa outro arquivo." |
| `FLX04` | limite estourado na carga (ticks, heap, profundidade, prazo) | "O script executou demais ao ser carregado (prazo de 1 s). No topo do arquivo deve haver só declarações; confira se alguma `def` tem laço longo." |
| `FLX05` | item no topo que não é declaração; declaração desconhecida; declaração dentro de `def` | "No topo do arquivo só pode haver `fluxo(...)`, `etapa(...)` e `decisao(...)`; tire a atribuição da linha 3." |
| `FLX06` | valor com tipo errado num argumento (lista, texto, booleano) | "`obrigatorio` do campo `coleta.cep` precisa ser `True` ou `False`, não texto." |
| `FLX07` | nome inexistente no dialeto (`open`, `time`, `print`, `getattr`…) | "`time` não existe na linguagem de fluxos: o script não tem relógio. Use os campos do fluxo para decidir." |
| `FLX10` | falta `fluxo(...)` ou há mais de um | "O arquivo não declara `fluxo(...)`. Um arquivo é um fluxo: comece por `fluxo(id = ..., nome = ..., inicio = ...)`." |
| `FLX11` | argumento obrigatório ausente | "`fluxo` não recebeu `inicio`: indique o id da etapa em que o atendimento começa." |
| `FLX12` | argumento desconhecido (pega erro de digitação) | "`campo` não tem o argumento `obligatorio`. Use `obrigatorio`." |
| `FLX13` | id fora do formato (2 a 64, `[a-z][a-z0-9_]*`) | "O id `Agendar Visita` não é válido: use minúsculas, números e `_`, por exemplo `agendar_visita`." |
| `FLX14` | id repetido (etapa, decisão, campo na etapa, ação na etapa) | "O id `coleta` está declarado duas vezes. Cada etapa, decisão, campo e ação tem id único no seu escopo." |
| `FLX15` | destino inexistente em `inicio`, `vai_para`, `entao`, `senao` | "A ação `coleta.confirmar` vai para `confirmacao_final`, que não existe. Use o id de uma etapa ou de uma decisão." |
| `FLX16` | o mesmo `fim` declarado com argumentos diferentes | "O fim `concluido` aparece com `resultado` diferente em dois lugares; use o mesmo `fim(...)` nas duas ações." |
| `FLX17` | `exige` cita campo que não existe na etapa | "A ação `coleta.confirmar` exige `cep`, que não é campo da etapa `coleta`." |
| `FLX18` | problema nas `opcoes`: `opcao` com menos de 2, opção repetida, `opcoes` em campo que não é `opcao` | "O campo `coleta.periodo` é `opcao` e precisa de pelo menos duas `opcoes`." |
| `FLX19` | regra ou ator inválido: `se` não é combinador nem `funcao`; `funcao` cita `def` inexistente; ator fora de `atores` da etapa | "A ação `coleta.confirmar` é do ator `atendente`, que não está em `atores` da etapa `coleta`." |
| `FLX20` | tipo de campo desconhecido | "O campo `coleta.cep` tem tipo `telefone`; use texto, numero, data, booleano, opcao ou anexo." |
| `FLX21` | combinador malformado: campo fora de `etapa.campo`, aridade errada (`e`/`ou` < 2, `nao` ≠ 1), combinador desconhecido | "`igual` precisa de `etapa.campo` e um valor: `igual(\"coleta.periodo\", \"manhã\")`." |
| `FLX22` | combinador cita etapa ou campo inexistente | "A regra compara `relato.cep`, mas a etapa `relato` não tem o campo `cep`." |
| `FLX23` | ciclo entre decisões (inclusive decisão que aponta para si mesma) | "As decisões formam um ciclo: `a → b → a`. Um fluxo não pode voltar a uma decisão já avaliada." |
| `FLX24` | `def` fora do formato: não é `def nome(dados)`, ou usa nome reservado do dialeto | "`def problema_grave(dados, extra)` precisa receber um único argumento chamado `dados`." |
| `FLX25` | variável com formato inválido | "`{cliente.nome}` não existe. As variáveis são `{etapa.campo}`, `{ficha.id}` e `{cliente.peer}`." |
| `FLX26` | `{...}` em texto que não interpola | "A `instrucao` da etapa `coleta` tem `{...}`, mas instrução não interpola: o agente lê o texto como está. Use variável só em `mensagem_ao_cliente` e `notificar_whatsapp`." |
| `FLX27` | acima de 60 nós (etapas + decisões + fins) | "O fluxo tem 72 nós e o limite é 60. Junte etapas parecidas ou divida o fluxo." |
| `FLX28` | `notificar_whatsapp` com número inválido | "O número `123` não serve: use dígitos com DDI, por exemplo `5521999990000`." |

### 10.2 Avisos (não bloqueiam)

| Código | Quando aparece | Mensagem (modelo) |
|---|---|---|
| `FLX51` | etapa sem nenhuma ação | "A etapa `confirmacao` não tem ação: o atendimento não sai dela. Acrescente uma `acao(...)` com `vai_para`." |
| `FLX52` | etapa ou decisão inalcançável a partir de `inicio` | "A etapa `confirmacao` não é alcançável a partir de `inicio = \"coleta\"`. Confira os `vai_para`." |
| `FLX53` | campo que nenhuma ação exige e nenhuma regra lê | "O campo `coleta.periodo` não é exigido por nenhuma ação nem lido por nenhuma regra: ninguém usa o valor." |
| `FLX54` | `igual`/`diferente` comparando com valor fora das `opcoes` | "A decisão `classificar` compara `relato.afeta_agora` com `talvez`, que não é uma das opções (sim, nao): o teste nunca casa." |
| `FLX55` | condição que sempre dá o mesmo resultado (`igual` e `diferente` do mesmo campo/valor na mesma conjunção; mesmo teste repetido numa disjunção) | "A decisão `classificar` não pode ser verdadeira e falsa ao mesmo tempo para `relato.afeta_agora`." |
| `FLX56` | gatilho sem `exemplos` | "O gatilho não tem `exemplos`: com 2 ou 3 falas típicas o Jev acerta mais." |
| `FLX57` | gatilho sem `nao_quando` | "O gatilho não tem `nao_quando`: diga em que situação o fluxo NÃO deve começar." |
| `FLX58` | `notificar_whatsapp` sem nenhuma variável | "A notificação para a equipe não usa variável: ela vai receber sempre o mesmo texto, sem saber de qual atendimento se trata." |
| `FLX59` | variável de etapa/campo que não existe no fluxo | "A mensagem usa `{identificacao.numero_pedido}`, mas a etapa `identificacao` não tem esse campo: no atendimento sai vazio." |
| `FLX60` | `maior`/`menor` em campo que não é `numero` | "A regra compara `coleta.periodo` com `maior`, mas o campo é texto: mude o tipo para `numero` ou use `igual`." |
| `FLX61` | `def` definida e não usada | "A função `problema_grave` não é usada por nenhuma decisão: chame com `funcao(\"problema_grave\")` ou apague." |
| `FLX62` | ator listado na etapa sem nenhuma ação dele | "A etapa `triagem` permite o `atendente`, mas nenhuma ação é dele: na prática ele não tem o que fazer aqui." |
| `FLX63` | `ao_entrar` usa variável do próprio passo | "`ao_entrar` da etapa `confirmacao` usa `{confirmacao.recebido}`, que só é preenchido dentro da própria etapa: a mensagem sai com o texto vazio." |
| `FLX64` | texto acima do recomendado (`quando`/`nao_quando` > 400; `instrucao` > 1000) | "A `instrucao` da etapa `coleta` tem 1.240 caracteres; o recomendado é até 1000, porque isso entra no prompt a cada turno." |

### 10.3 Falhas de execução (não são diagnóstico de publicação)

| Código | O que aconteceu | Efeito |
|---|---|---|
| `FLXR01` | a `def` lançou erro (chave ausente sem `.get`, divisão por zero, tipo inesperado) | execução `falha`, com o erro no evento; a Ficha continua aberta |
| `FLXR02` | a `def` devolveu algo que não é `True`/`False` | execução `falha`; nunca "verdadeiro por acaso" |
| `FLXR03` | limite da regra estourado (ticks, heap, prazo de 200 ms) | execução `falha` |
| `FLXR04` | mais de 10 saltos de decisão na mesma passagem | execução `falha` com o caminho percorrido |
| `FLXR05` | `dados_json` ilegível ou valor de tipo inválido gravado na execução | execução `falha`; nada é sobrescrito |
| `FLXR06` | IR gravada diverge da recompilação do texto (`ir_divergente`) | execução `falha` **fechada**: o motor não adivinha qual das duas vale |

Falha de **efeito** (mensagem, notificação, encaminhamento) não é falha de execução: fica `falhou`, com tentativas e evento na Ficha, e a transição continua valendo (PRD L11).

## 11. Os três fluxos de exemplo

| Arquivo | O que ensina |
|---|---|
| [`tests/fluxos/exemplos/agendar_visita.star`](../tests/fluxos/exemplos/agendar_visita.star) | a forma mínima: cabeçalho, gatilho, duas etapas, dois fins |
| [`tests/fluxos/exemplos/triagem_suporte.star`](../tests/fluxos/exemplos/triagem_suporte.star) | decisão nas duas formas: combinador (`e`/`igual`) e `def`; três fins |
| [`tests/fluxos/exemplos/confirmar_entrega.star`](../tests/fluxos/exemplos/confirmar_entrega.star) | efeitos: mensagem ao cliente com variável, `notificar_whatsapp` para a equipe com `{ficha.id}`, `encaminhar` para outro departamento e um fim que encerra o atendimento |

Os três são carregados pelo validador do spike e terminam sem nenhum diagnóstico (`spike/fluxos/evidencias/fx02-exemplos.txt`); a IR de cada um está em `spike/fluxos/evidencias/fx02-ir/`.

Os **40 diagnósticos de forma** (`FLX01`–`FLX28`, `FLX51`–`FLX64`) têm, cada um, um arquivo mutado que o dispara: `spike/fluxos/evidencias/fx02-negativos.txt`. As falhas de execução (`FLXR01`–`FLXR06`) são do motor (FX-11) e não são cobertas por esse validador.

## 12. O que muda em relação ao POP Atende

| Antes (POP Atende) | Agora (Harness) | Efeito prático |
|---|---|---|
| Fluxo do agente / template da org com associação | Fluxo da **org**, permissão por agente | trocar de agente não perde fluxo; liberar ou negar é uma lista |
| Ficha só nasce com o fluxo | Ficha nasce com a **conversa** | todo atendimento tem registro; fluxo é sub-item |
| `next_step: "closed"` fecha a Ficha; estado `closing_pending` | `fim(...)` conclui só a **execução**; encerrar é explícito (`encerra_atendimento = True`) | cliente pode ter vários fluxos no mesmo atendimento |
| Várias Fichas ativas por conversa (até 3 no prompt) | **uma execução ativa** por Ficha | prompt menor, sem ambiguidade |
| `completion` `auto`/`any`/`all` | só o comportamento automático (o único usado de fato) | menos opção, mesmo efeito do canônico |
| Atores `client`, `provider`, `operator`, `system` | `agente` e `atendente` (e `sistema` só como ator de evento) | cliente nunca é ator; provedor externo fora |
| Auto-transição por busca em largura, executando ações intermediárias | **removida**: erro com as ações válidas | o agente só avança o que pediu |
| Condicional em `pop_expr` (texto) ou regra simples | combinadores declarativos **ou** `def` | a UI edita o declarativo; o avançado fica como código |
| Notificação e encaminhamento como nós "satélite" ligados por arestas | **efeitos** dentro da ação ou do `ao_entrar` | fica claro o que cada ação dispara |
| Variável inexistente vira vazio, sem aviso | vazio **e** aviso na publicação (`FLX59`) | menos mensagem torta chegando ao cliente |
| Etapa inicial marcada por `is_entry` | `inicio = "id"` no `fluxo(...)` | não há como esquecer de marcar a entrada |
| Expressão livre em `def` com acesso a helpers do produto | `def` pura, um argumento `dados`, sem relógio/rede/arquivo/aleatório | regra determinística e testável |
| Executor/atribuição, calendário, PDF, prestadores | **fora** | escopo menor; voltam por especificação própria |

## 13. Pendências de contrato (para o mantenedor decidir)

São escolhas que este documento fecha de um jeito, mas que são do dono do contrato. Se alguma mudar, muda a IR e o compilador antes da FX-10; nenhuma bloqueia a FX-04.

| # | Assunto | O que adotei | Por quê / o que muda se for outra coisa |
|---|---|---|---|
| P1 | Onde ficam `inicio` e `gatilho` na IR | **hoisted**: no script eles são argumentos de `fluxo(...)`; na IR viram chaves de topo (`inicio`, `gatilho`) | segue a D4 e mantém o exemplo do PRD §5.1 válido. Se preferir tudo dentro de `fluxo{}`, muda o esquema e o compilador |
| P2 | Nomes dos atores na IR | `etapas[].atores_acao` e `acoes[].ator` | evita o campo "ator" ambíguo da D5; o ator de evento não entra na IR |
| P3 | Ordem e forma de `fins[]` | ordem da primeira aparição; sem `tipo` dentro do array (o discriminante só existe dentro de `vai_para`) | deixa `fins[]` como catálogo, não como destino |
| P4 | Como uma decisão usa uma `def` | `funcao("nome")`, com a `def` fora da regra | a chamada direta `problema_grave(dados)` não existe na carga (não há dados); `funcao` é serializável em JSON |
| P5 | `senao` obrigatório na decisão | **sim**, sempre | decisão sem `senao` deixa o atendimento sem destino; o motor não inventa um |
| P6 | Ciclo entre decisões | **erro de publicação** (`FLX23`), mais rígido que o PRD §6.4 (que trata como falha de execução) | pega o engano antes de um cliente chegar nele; a falha `FLXR04` continua para o encadeamento acima de 10 |
| P7 | Comparação de texto | `igual` exato (só tira espaço nas pontas, diferencia maiúscula); `contem` ignora maiúscula **e** acento | `contem` é para fala de cliente; `igual` é para valor de opção. Se quiser tudo insensível, muda a semântica e o motor |
| P8 | Id mínimo de 2 caracteres | **2 a 64** (`ok` é id válido) | 3 barraria nomes naturais como `ok`, `ir`, `sim` |
| P9 | Interpolação só em mensagem | `{...}` só em `mensagem_ao_cliente` e `notificar_whatsapp`; nos outros textos é **erro** (`FLX26`) | o agente lê `instrucao` como texto; interpolar ali daria a impressão de que o prompt é montado por variável |
| P10 | Forma de chamada | `mensagem_ao_cliente`/`encaminhar` com o texto posicional; `notificar_whatsapp` com `para`/`mensagem` nomeados | segue o exemplo do PRD; se preferir tudo nomeado, é só o compilador aceitar as duas formas na leitura |
| P11 | `fim.resultado` | texto livre, gravado na execução | se virar enum fechado (para métrica), precisa de lista de resultados por fluxo |
| P12 | `campo.ajuda` | argumento novo, sem efeito no atendimento | é dica para a UI futura; se não quiser campo novo na IR, sai agora (antes de publicar) |
| P13 | Comentários | `formatar` preserva, gerar a partir da IR perde | a IR é estrutura, não guarda comentário; documentado para o autor não se surpreender |
| P14 | Fonte máxima | 32 KB (D1) | o PRD §9 ainda traz `65536` em `max_fonte_bytes`; o PRD precisa mudar para 32.768 |
| P15 | Variáveis na lista fechada | só `{etapa.campo}`, `{ficha.id}`, `{cliente.peer}` | se a operação pedir, ex.: `{ficha.aberta_em}`, entra como campo novo (mudança aditiva) |

### 13.1 O que o PRD precisa mudar (avisar o mantenedor, não editar o PRD)

1. §9: `fluxos.max_fonte_bytes` passa a **32768** (era 65536), para bater com a decisão do mantenedor e com o §3.4 deste documento.
2. §5: `while` e `**` não existem no Starlark; o exemplo de abuso é `"x" * 1000000000` (o spike da FX-01 já registrou isso).
3. §5: meta de latência por regra pode cair de 200 µs para **20 µs p95** (medido 1,7 µs); a de RAM fica em **92,8 kB** por fluxo de 40 nós.
4. §6.4: ciclo entre decisões vira **erro de publicação** (`FLX23`), não falha de execução.
5. §6.6-3: a lista de avisos do PRD está coberta por `FLX51`–`FLX64`; nenhuma ficou de fora.
6. §6.8: as variáveis de mensagem passam a valer só em `mensagem_ao_cliente` e `notificar_whatsapp` (`FLX26` nos outros textos).
7. §7.3: os nomes de rota se fecham na **FX-27** (o texto diz FX-25). O mesmo erro já foi apontado pelo mantenedor.

## 14. O que este documento não decidiu e não testou

- **Não há código de produto**: a IR, os diagnósticos e a semântica aqui descritos são implementados na FX-10 em diante. O validador usado aqui é o do spike (descartável, fora do workspace) e serve só para provar que os exemplos não inventam sintaxe.
- **Não testei no motor real**: avaliação de regra com dados reais, eventos, filas de efeito e concorrência são das FX-11 a FX-21.
- **A IR não foi versionada contra um caso antigo** (não há versão anterior): a regra aditiva está escrita, não exercitada.
- **`ir_divergente`** depende de o compilador ser determinístico — provado no spike (duas execuções, duas threads), a confirmar na FX-10.
