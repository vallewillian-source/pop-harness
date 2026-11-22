# Agentes, instruções e compositor (HR-09)

> Entrega da **HR-09**. Cobre a entidade agente com os campos `V1`, as instruções estruturadas, o
> compositor (portado do POP Atende, o produto de origem do projeto) e a CLI `poph agent` / `poph instructions`.
> Banco e repositórios: [`DADOS.md`](DADOS.md). Decisões: [`DECISOES.md`](DECISOES.md).

## 1. O que existe agora

| Peça | Onde | O que faz |
|---|---|---|
| Tipos das instruções | `crates/poph-types/src/instrucoes.rs` | `StructuredInstructions` com os mesmos nomes, padrões e tolerâncias do POP Atende (HR-03 §9.1); lista fechada de ferramentas da V1; validação de escrita. |
| Compositor | `crates/poph-core/src/compositor.rs` | Porte fiel de `core/agent/instructions_composer.py`: mesma saída, byte a byte. |
| Repositórios | `crates/poph-store/src/repos/agentes.rs` e `instrucoes.rs` | Agente (campos `V1`), vínculo agente↔categorias, instruções com `If-Match`. |
| CLI | `crates/poph/src/agente.rs` | `poph agent ...` e `poph instructions ...` (texto e `--json`). |

O crate `poph-agent` (D-08) ainda não existe. O compositor ficou em `poph-core` porque é código puro
sobre `poph-types`: pode ir para `poph-agent` sem mudar uma linha quando a HR-13 abrir a crate.

## 2. Campos do agente (`V1`)

Os campos são os marcados `V1` no inventário de configuração do agente (histórico) e já estavam no esquema
da HR-04b. O que esta tarefa acrescenta é a regra e a porta de entrada:

- **Identidade/perfil:** `name`, `slug`, `description`, `status`, `agent_type`, `category`.
- **Departamento:** `default_department_id` (para onde vão as conversas novas) e o par de handoff
  (`human_handoff_enabled`, `human_handoff_department_id`, gate G1/G4).
- **LLM por agente:** `provider`, `model`, `temperature`, `max_output_tokens`, `max_context_tokens`.
  No POP Atende esses campos eram gravados e ignorados; no Harness valem (é a diferença registrada
  na HR-03).
- **Turno:** `grouping_window_ms` (janela de agrupamento), `max_tool_rounds`,
  `max_tool_calls_per_message`.
- **RAG:** `rag_top_k`, `rag_min_score` (por agente; no POP Atende era global).
- **Jev:** `jev_json` (flags e limiares que sobrescrevem a configuração).
- **Categorias de produto:** tabela de vínculo `agent_product_categories`.

## 3. Regras que mudam o comportamento

### D-05 — inativo quer dizer inativo

`Agente::pode_abrir_turno()` e `Agentes::obter_para_turno()` aplicam a decisão: só `status = active`
abre turno. Agente `inactive`/`archived` devolve `DomainError::AgenteInativo` (com id e status), não
um agente pronto. O agente inativo **continua listado** e editável (o Console precisa mostrá-lo);
apenas sai do caminho do turno.

### D-04 — ferramenta habilitada filtra de verdade

`StructuredInstructions::ferramentas_habilitadas()` é o toolset do agente. A lista é fechada
(`FERRAMENTAS_V1`): `consultar_base`, `buscar_produto`, `enviar_arquivo`, `transferir`,
`cart_add_item`, `cart_remove_item`, `cart_summary`, `cart_clear`.

`validar()` roda **na escrita** (CLI/API): nome fora da lista é recusado com a lista do motor na
mensagem. Não roda na leitura: JSON legado do POP Atende, com nomes como `search_knowledge_base`,
precisa ser lido e mapeado pelo importador (HR-52), não recusado.

### Texto editado à mão é preservado

`agent_instructions.derived_from_json` guarda a distinção do POP Atende: `1` = texto gerado pelo
compositor (pode ser recomposto), `0` = prompt editado à mão (não é sobrescrito sem ordem).
`poph instructions create|update --file` recompõe; `--markdown-file` grava o texto como está.

## 4. O compositor

Porte de `compose_instructions`. Ordem fixa dos blocos:

1. **Identidade** — só se `identity.name` **ou** `identity.company`; cabeçalho
   `# {nome}, {papel} da {empresa}` seguido da descrição da empresa.
2. **Princípios** — numerados, cada um com a frase de contingência quando a ação é conhecida.
3. **Escopo** — `O que voce faz` / `O que voce NAO faz`. **Não** emite seção de handoff.
4. **Tom** — sempre emitido (idioma, tom, emojis, formato, restrições adicionais).
5. **Ferramentas** — só as habilitadas, com `— quando usar` quando houver.
6. **Instruções adicionais**.

O que ele **não** faz (de propósito): `flow_steps` (sem efeito no POP Atende), handoff, estilo fixo do
sistema, fichas, regras de data e contexto temporal — esses blocos são do motor do turno (PRD §6.1)
e entram depois do prefixo estável, nunca dentro dele.

### Paridade provada e como regerar

Fixturas em `crates/poph-core/tests/fixtures/compositor/`: `<nome>.json` (entrada) e
`<nome>.esperado.txt` (saída real do compositor do POP Atende). O teste
`crates/poph-core/tests/compositor_paridade.rs` compara byte a byte em 9 entradas e cobre os casos de
borda do §9.3 (identidade sem nome, empresa sem nome, ferramenta desabilitada, `flow_steps`).

Para regerar as referências quando o POP Atende mudar:

```bash
cd apps/backend
.venv/bin/python ../pop-harness/crates/poph-core/tests/fixtures/compositor/gerar_referencia.py
```

O script só importa `core.agent.instructions_composer` e chama a mesma função que o
`PATCH /agents/{id}/instructions` chama; nada do backend é alterado.

## 5. CLI

`--org` (UUID v7 ou slug) é global e obrigatório nos comandos de dados. Escrita exige `--if-match`
com a versão lida; `--json` devolve formato estável (`versao_relatorio: 1`).

```bash
# cria o agente (slug derivado do nome: "Bia Atendente" → bia-atendente)
poph --org demo agent create --name "Bia Atendente" --department <dep_id> --model gemini-3.8-flash-tiered

# grava as instruções estruturadas (o texto é recomposto pelo compositor)
poph --org demo instructions create bia-atendente --file instrucoes.json

# prompt editado à mão, sem recomposição
poph --org demo instructions update bia-atendente --if-match 2 --markdown-file prompt.md

# mostra o agente, as categorias e o texto composto
poph --org demo agent show bia-atendente

# inativa (D-05: deixa de abrir turno)
poph --org demo agent update bia-atendente --if-match 1 --status inactive

# vincula categorias de produto (substitui a lista)
poph --org demo agent update bia-atendente --if-match 2 --category-id <cat_id>
```

Códigos de saída: `0` sucesso, `1` erro (com mensagem no stderr), `2` uso incorreto (clap).

Enquanto o `poph org` não existir (HR-08), o exemplo `crates/poph/examples/semear_org.rs` cria uma
org e um departamento para exercitar a CLI:

```bash
cargo run -p poph --example semear_org -- demo "Org Demonstração"
```

## 6. O que fica para a HR-13

- Ler o agente pelo `obter_para_turno` (D-05), carregar as instruções e usar o markdown como prefixo
  estável — com o bloco volátil (RAG, Jev, contexto temporal, atendimento humano) depois.
- Regra de conversa em andamento com agente que ficou inativo no meio (D-05).
- Registrar em `audit_log`/`turns` a mensagem descartada de agente inativo.
- Ligar `ferramentas_habilitadas()` ao laço de ferramentas, rejeitando nome inventado pelo modelo.
