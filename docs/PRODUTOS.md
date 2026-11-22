# Produtos: catálogo, busca híbrida e `buscar_produto` (HR-18)

> Como o agente deixa de responder preço "de cabeça": o catálogo é indexado no `rag.db` (só o texto),
> o preço vem do registro na hora da resposta, e o modelo só recebe valor pela ferramenta. Código:
> `crates/poph-store/src/repos/produtos.rs` (registro), `crates/poph-rag/src/indice.rs` (índice de
> produtos), `crates/poph-agent/src/produtos.rs` (serviço e ferramenta), `crates/poph/src/produto.rs`
> (CLI) e `crates/poph-agent/src/turno/` (ligação com o turno).

## 1. Desenho

```
catálogo (harness.db)                    índice de produtos (rag.db)
  product_categories  ─────┐                rag_produtos          (1 linha por produto/agente)
  products (preço!)   ─────┤                rag_produtos_trechos  (vetor int8 por trecho)
  agent_product_categories ┘                rag_produtos_fts      (FTS5 do texto)
           │                                          │
           │  só nome, aliases, descrição e tags ─────┘  (preço NUNCA entra)
           ▼
  pergunta do cliente
     ├─ embedding da consulta
     ├─ varredura exata no índice int8 do agente (cosseno) ─┐
     └─ FTS5 (palavras, com prefixo e sem palavras vazias) ─┤
                                                            ▼
                                    combinação por rank (RRF, k = 60)
                                                            │
                              corte de relevância (cosseno) e top-k do agente
                                                            │
                              ids achados → **leitura do registro** (preço, unidade, condições)
                                                            │
                                            ferramenta `buscar_produto` → modelo
```

- **Preço fora do índice.** O hash de origem é calculado sobre o texto indexável; mudar preço,
  unidade, condições, inclui/exclui **não muda o hash** e não reembeda nada. A resposta sempre usa o
  registro atual (teste `mudar_preco_aparece_na_resposta_seguinte_sem_reindexar`; medido na CLI:
  preço alterado de R$ 250,50 para R$ 199,90 e a resposta seguinte já saiu com o valor novo).
- **Um resultado por produto.** A busca devolve até `top_k` produtos (o melhor trecho de cada um),
  não vários trechos do mesmo produto.
- **Palavras e acentos.** Mesma consulta do FTS5 da HR-17: palavras vazias do português saem e
  palavras de 4+ letras viram prefixo. É o que faz "cupins" achar "cupim" e "ratos" achar "ratazanas".
- **Determinismo.** Empate na nota RRF desempata por similaridade e depois por `rowid`.

### Medição (fixtura `crates/poph-rag/tests/fixtures/produtos_rotulados.json`)

12 produtos de uma empresa fictícia de controle de pragas e **20 consultas** escritas como cliente
escreveria — 13 por apelido ("barata", "ralo entupido", "caixa de agua") e 7 com um erro de
digitação simples ("dedetizacao", "cupin", "denge", "escorsiao", "perceveijo"). Modelo real int8,
acerto no top-3:

| Busca | Acerto top-3 |
|---|---|
| só vetor | 19/20 (95,0 %) |
| híbrida (RRF + FTS com prefixo) | **19/20 (95,0 %)** |

A única falha das duas é `escorsiao` (dois erros na mesma palavra). O teste
`hibrida_de_produtos_acerta_top3_pelo_menos_tanto_quanto_so_vetor` falha se a híbrida ficar abaixo da
vetorial; sem o modelo instalado ele é pulado.

## 2. Regra de categoria (`agent_product_categories`)

É a mesma do POP Atende (`core/skills/tools/product_tools.py`):

- **Agente com categorias vinculadas:** só enxerga os produtos **ativos** dessas categorias. Produto de
  outra categoria não entra no índice dele, não volta da busca e não é executado pela ferramenta.
- **Agente sem vínculo:** enxerga todos os produtos ativos da org.
- **Filtro pedido pelo modelo** (`categoria`, nome ou id): restringe dentro do que o agente já pode
  ver. Categoria não vinculada é **recusada** com erro ("não está vinculada a este agente"), como no
  POP Atende — o modelo recebe o erro e pode seguir sem filtro.

O vínculo é escrito por `poph agent update --category-id …` (HR-09) e muda a versão do agente, o que
invalida a config quente (HR-11).

## 3. D-05 aplicado a produto

Produto `inactive` ou `archived` sai do índice na sincronização: não volta da busca, não é oferecido
pela ferramenta e não é lido por `ativos_por_ids`. Continua listado (`poph product list --all`) e
editável. Teste: `produto_inativo_ou_arquivado_sai_da_busca_e_da_ferramenta`.

## 4. Ferramenta `buscar_produto` (D-04)

- Registrada em `RegistroPadrao`; o `chat` junta com `consultar_base` (`registro_com`). As próximas
  tarefas (carrinho HR-19, arquivos HR-24) registram as suas sem tocar no laço.
- Só é **oferecida** quando o agente a tem habilitada nas instruções *e* o processo sabe executá-la.
- Argumentos: `termo` (obrigatório), `categoria` (opcional, nome ou id) e `limite` (1–10). **Nenhum
  campo de preço.**
- Devolve, por produto: id, nome, categoria, preço (texto e centavos), unidade, condições,
  inclui/exclui — o texto vem dentro de `<produtos_encontrados>`, marcado como **dado**, não ordem
  (a descrição é escrita por gente e pode conter "ignore as instruções").
- Sem resultado, devolve instrução explícita de **não inventar preço** ("Não invente preço: diga que
  não encontrou e ofereça falar com a equipe").
- O que a ferramenta usou fica no coletor do turno (`UsoFerramentas`), que o turno grava em
  `turns.retrieved_json`.

### Registro do turno

`turns.retrieved_json` passa a ser uma lista com discriminante `tipo`:

```json
[
  {"tipo": "trecho",  "item": "…", "titulo": "Prazo de agendamento", "grupo": "Políticas", "texto": "…", "similaridade": 0.72, "relevancia": 0.032},
  {"tipo": "produto", "produto": "…", "nome": "Dedetização de baratas", "preco_centavos": 32000, "preco_texto": "R$ 320,00", "unidade": "por visita", "condicoes": "Garantia de 6 meses"}
]
```

Sem uso nenhum continua `[]`. Sem migração: a coluna já existia (HR-04b) e a HR-17 grava a mesma
estrutura, agora com `tipo`.

**Limite:** o registro guarda o que a **ferramenta devolveu**, não o que o modelo citou. Com o corte
de relevância configurado no agente, só os produtos acima do corte entram (medido: com corte 0,35 uma
pergunta sobre escorpiões — fora do catálogo — gravou `[]`).

## 5. Regra do preço no bloco volátil

Quando o agente tem `buscar_produto` habilitada, o bloco volátil do turno ganha uma linha:

> `preço: só informe valor que vier da ferramenta buscar_produto neste turno; se ela não devolveu o produto, diga que vai confirmar com a equipe e não estime valor.`

Motivo medido: sem essa linha, o modelo respondeu de cabeça para uma pergunta indireta ("Vocês fazem
desratização? Qual o valor?") e inventou R$ 380,00 e "retorno após 15 dias" — nada disso existia no
catálogo. Com a linha, as mesmas perguntas passaram a chamar a ferramenta e a responder com o valor do
registro (3 de 3 execuções). O bloco é volátil: o prefixo estável continua idêntico byte a byte.

## 6. CLI `poph product`

```
poph --offline --org <org> product create --name … --category <id|nome> --price <centavos> [--price-text …]
        [--alias …]… [--tag …]… [--description …] [--unit …] [--includes …] [--excludes …] [--conditions …] [--slug …]
poph --offline --org <org> product list [--limit N] [--all]
poph --offline --org <org> product show <id|slug>
poph --offline --org <org> product update <id|slug> --if-match <versão> [--name …] [--price …] [--status active|inactive|archived] …
poph --offline --org <org> product delete <id|slug> --force
poph --offline --org <org> product reindex [--agent <id|slug>]
```

- `--price` é sempre **centavos** (`32000` = R$ 320,00). `--price-text` é o que o cliente vê; vazio, a
  resposta formata a partir dos centavos (`25050` → "R$ 250,50").
- `--category` aceita id (UUID v7) ou nome exato, sem diferenciar maiúsculas.
- `--if-match` obrigatório nas alterações; `If-Match` velho devolve conflito de versão.
- Toda escrita reindexa os agentes da org na hora (comparação por hash: só reembeda o que mudou). Sem
  o modelo instalado, a escrita vale e o comando avisa: rode `poph product reindex` depois.
- `delete` exige `--force` (a exclusão apaga o produto do catálogo; itens de carrinho ficam com
  `product_id` nulo, por decisão do esquema).

## 7. O que a HR-19 (carrinho) e a HR-42 (Console) consomem

- **HR-19:** `Produtos::ativos_por_ids` e `Produtos::obter` para conferir o produto e o preço na hora
  de adicionar ao carrinho (`cart_items.unit_price_cents` guarda o preço praticado). A ferramenta
  `buscar_produto` devolve o `id` para o modelo usar nas ferramentas de carrinho.
- **HR-42 (Console):** `poph product list --json` (catálogo e situação), `poph category list --json`
  (categorias) e `turns.retrieved_json` (o "por quê": produto, preço aplicado e similaridade). O
  estado do índice de um agente sai de `product reindex --json` (novos/alterados/removidos).
- **HR-31 (Fichas/fluxo de orçamento)** usa a mesma ferramenta; nada muda no contrato.

## 8. O que não está coberto

- **Carrinho** (HR-19) e o preço por quantidade; hoje o preço é por unidade, como no POP Atende.
- **Promoção/desconto por cliente**: não existe; o preço é o do registro.
- **Custo por turno** continua 0 (sem tabela de preços de modelo, herdado da HR-10).
- **A busca de produtos não tem cache em memória**: lê os vetores do `rag.db` a cada consulta (o
  catálogo é pequeno; a HR-55 mede carga). A base de conhecimento continua com o LRU da HR-16.
- **Reindexação é por org** nas escritas da CLI (uma consulta por agente, sem reembedar o que não
  mudou). Org com milhares de agentes é a HR-55.
- **O modelo pode responder sem chamar a ferramenta**: a linha do bloco volátil reduz muito, mas não
  proíbe. Quem vai decidir "preço exige consulta" é o Jev (HR-28 em diante); hoje o motor não força a
  chamada.
- **Produto sem preço** (`price_cents = 0` e texto vazio) sai como "R$ 0,00" — é decisão de cadastro:
  quem quer "sob consulta" escreve isso em `--price-text` ou em `--conditions`.
