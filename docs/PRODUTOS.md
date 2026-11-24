# Products: catalog, hybrid search and `buscar_produto` (HR-18)

> How the agent stops answering prices "from memory": the catalog is indexed in `rag.db` (text only), the
> price comes from the record at the moment of the reply, and the model only receives a value through the tool.
> Code: `crates/poph-store/src/repos/produtos.rs` (record), `crates/poph-rag/src/indice.rs` (product index),
> `crates/poph-agent/src/produtos.rs` (service and tool), `crates/poph/src/produto.rs` (CLI) and
> `crates/poph-agent/src/turno/` (link with the turn).

## 1. Design

```
catalog (harness.db)                     product index (rag.db)
  product_categories  ─────┐                rag_produtos          (1 row per product/agent)
  products (price!)   ─────┤                rag_produtos_trechos  (int8 vector per chunk)
  agent_product_categories ┘                rag_produtos_fts      (FTS5 over the text)
           │                                          │
           │  name, aliases, description and tags only ┘  (price NEVER enters)
           ▼
  customer question
     ├─ query embedding
     ├─ exact scan of the agent's int8 index (cosine) ─┐
     └─ FTS5 (words, with prefix and without stop words) ┤
                                                         ▼
                                    rank-based merge (RRF, k = 60)
                                                         │
                              relevance cut (cosine) and the agent's top-k
                                                         │
                              ids found → **read the record** (price, unit, conditions)
                                                         │
                                            `buscar_produto` tool → model
```

- **Price outside the index.** The source hash is computed over the indexable text; changing price, unit,
  conditions, includes/excludes **does not change the hash** and re-embeds nothing. The reply always uses the
  current record (test `mudar_preco_aparece_na_resposta_seguinte_sem_reindexar`; measured in the CLI: price changed
  from R$ 250,50 to R$ 199,90 and the next reply already used the new value).
- **One result per product.** The search returns up to `top_k` products (the best chunk of each), not several
  chunks of the same product.
- **Words and accents.** Same FTS5 query as HR-17: Portuguese stop words are removed and words of 4+ letters become
  prefixes. This is why "cupins" finds "cupim" and "ratos" finds "ratazanas".
- **Determinism.** Ties in the RRF score are broken by similarity, then by `rowid`.

### Measurement (fixture `crates/poph-rag/tests/fixtures/produtos_rotulados.json`)

12 products from a fictional pest-control company and **20 queries** written the way a customer would write them —
13 by nickname ("barata", "ralo entupido", "caixa de agua") and 7 with a simple typo ("dedetizacao", "cupin", "denge",
"escorsiao", "perceveijo"). Real int8 model, hit in the top 3:

| Search | Top-3 hit |
|---|---|
| vector only | 19/20 (95.0 %) |
| hybrid (RRF + FTS with prefix) | **19/20 (95.0 %)** |

The only miss in both is `escorsiao` (two errors in the same word). The test
`hibrida_de_produtos_acerta_top3_pelo_menos_tanto_quanto_so_vetor` fails if the hybrid falls below the vector-only
result; without the model installed it is skipped.

## 2. Category rule (`agent_product_categories`)

It is the same as in POP Atende (`core/skills/tools/product_tools.py`):

- **Agent with linked categories:** sees only the **active** products of those categories. A product from another
  category does not enter its index, does not come back from the search and is not executed by the tool.
- **Agent without links:** sees all active products of the org.
- **Filter requested by the model** (`categoria`, name or id): narrows within what the agent can already see. An
  unlinked category is **refused** with an error ("não está vinculada a este agente", "not linked to this agent"),
  as in POP Atende — the model receives the error and can continue without the filter.

The link is written by `poph agent update --category-id …` (HR-09) and changes the agent version, which invalidates
the hot config (HR-11).

## 3. D-05 applied to products

A product that is `inactive` or `archived` leaves the index during sync: it does not come back from the search, is
not offered by the tool and is not read by `ativos_por_ids`. It stays listed (`poph product list --all`) and
editable. Test: `produto_inativo_ou_arquivado_sai_da_busca_e_da_ferramenta`.

## 4. Tool `buscar_produto` (D-04)

- Registered in `RegistroPadrao`; `chat` combines it with `consultar_base` (`registro_com`). The next tasks (cart
  HR-19, files HR-24) register theirs without touching the loop.
- It is only **offered** when the agent has it enabled in its instructions *and* the process knows how to run it.
- Arguments: `termo` (required), `categoria` (optional, name or id) and `limite` (1–10). **No price field.**
- Returns, per product: id, name, category, price (text and cents), unit, conditions, includes/excludes. The text
  comes inside `<produtos_encontrados>` (`<found_products>`, a tag emitted by the code), marked as **data**, not as
  an order (the description is written by people and may contain "ignore as instruções", "ignore the instructions").
- With no result, it returns an explicit instruction **not to invent a price**: "Não invente preço: diga que não
  encontrou e ofereça falar com a equipe" (emitted by the code in Portuguese; in English: "Do not invent a price: say
  you did not find it and offer to talk to the team").
- What the tool used goes into the turn collector (`UsoFerramentas`), which the turn saves in `turns.retrieved_json`.

### Turn record

`turns.retrieved_json` becomes a list with a `tipo` discriminator:

```json
[
  {"tipo": "trecho",  "item": "…", "titulo": "Prazo de agendamento", "grupo": "Políticas", "texto": "…", "similaridade": 0.72, "relevancia": 0.032},
  {"tipo": "produto", "produto": "…", "nome": "Dedetização de baratas", "preco_centavos": 32000, "preco_texto": "R$ 320,00", "unidade": "por visita", "condicoes": "Garantia de 6 meses"}
]
```

With no use at all it stays `[]`. No migration: the column already existed (HR-04b) and HR-17 writes the same
structure, now with `tipo`.

**Limit:** the record stores what the **tool returned**, not what the model cited. With the relevance cut configured on
the agent, only products above the cut enter (measured: with a cut of 0.35, a question about scorpions — outside the
catalog — recorded `[]`).

## 5. Price rule in the volatile block

When the agent has `buscar_produto` enabled, the turn's volatile block gets one line:

> `preço: só informe valor que vier da ferramenta buscar_produto neste turno; se ela não devolveu o produto, diga que vai confirmar com a equipe e não estime valor.`

(Emitted by the code in Portuguese. In English: "price: only state a value that comes from the buscar_produto tool in
this turn; if it did not return the product, say you will confirm with the team and do not estimate a value.")

Measured reason: without this line, the model answered from memory to an indirect question ("Vocês fazem
desratização? Qual o valor?", "Do you do rat control? What's the price?") and invented R$ 380,00 and "retorno após 15
dias" ("return after 15 days") — none of it existed in the catalog. With the line, the same questions started calling
the tool and answering with the value from the record (3 of 3 runs). The block is volatile: the stable prefix stays
identical byte for byte.

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

- `--price` is always in **cents** (`32000` = R$ 320,00). `--price-text` is what the customer sees; if empty, the reply
  is formatted from the cents (`25050` → "R$ 250,50").
- `--category` accepts an id (UUID v7) or an exact name, case-insensitive.
- `--if-match` is required for changes; an outdated `If-Match` returns a version conflict.
- Every write reindexes the org's agents right away (hash comparison: only what changed is re-embedded). Without the
  model installed, the write still takes effect and the command warns you: run `poph product reindex` afterwards.
- `delete` requires `--force` (deletion removes the product from the catalog; cart items keep `product_id` null, by
  the schema's decision).

## 7. What HR-19 (cart) and HR-42 (Console) consume

- **HR-19:** `Produtos::ativos_por_ids` and `Produtos::obter` to check the product and the price when adding to the
  cart (`cart_items.unit_price_cents` stores the price charged). The `buscar_produto` tool returns the `id` for the
  model to use in the cart tools.
- **HR-42 (Console):** `poph product list --json` (catalog and status), `poph category list --json` (categories) and
  `turns.retrieved_json` (the "why": product, applied price and similarity). The state of an agent's index comes from
  `product reindex --json` (new/changed/removed).
- **HR-31 (Cases/budget flow)** uses the same tool; nothing changes in the contract.

## 8. What is not covered

- **Cart** (HR-19) and price per quantity; today the price is per unit, as in POP Atende.
- **Promotion/discount per customer:** does not exist; the price is the one in the record.
- **Cost per turn** stays at 0 (no model price table, inherited from HR-10).
- **The product search has no in-memory cache:** it reads the vectors from `rag.db` on every query (the catalog is
  small; HR-55 measures load). The knowledge base still uses the LRU from HR-16.
- **Reindexing is per org** on CLI writes (one query per agent, without re-embedding what did not change). An org with
  thousands of agents is HR-55.
- **The model can reply without calling the tool:** the line in the volatile block greatly reduces this, but does not
  forbid it. Jev (HR-28 onwards) is the one to decide "price requires a lookup"; today the engine does not force the
  call.
- **Product without a price** (`price_cents = 0` and empty text) is shown as "R$ 0,00" — this is a catalog decision:
  whoever wants "sob consulta" ("price on request") writes that in `--price-text` or in `--conditions`.
