# Provedores de modelo no POP Harness

Como apontar o Harness para um serviço de modelo que fale o protocolo da OpenAI e como conviver com
mais de um provedor. Todo caminho, campo e nome abaixo foi conferido no código em 24/11/2026
(commit `9b518e6`); a configuração de referência está em [`CONFIGURACAO.md`](CONFIGURACAO.md).

## 1. O contrato de um provedor

O contrato é o trait `LlmProvider`, em `crates/poph-core/src/llm/provedor.rs`. Ele tem um método:

```rust
fn stream<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<FluxoLlm, ErroLlm>>;
```

`FluxoLlm` é um stream de `Result<EventoLlm, ErroLlm>`. Os eventos, definidos em
`crates/poph-core/src/llm/tipos.rs`, são quatro: `Texto` (pedaço da resposta), `Ferramenta` (chamada
completa, entregue só quando os argumentos chegam inteiros), `Uso` (tokens de entrada e saída, uma
vez, antes do fim) e `Fim` (com o motivo: concluído, ferramentas, limite de tokens ou outro). Quem
consome pode parar quando quiser: soltar o stream cancela a chamada.

O motor do turno (`crates/poph-agent/src/turno/mod.rs`) e o resumo rolante
(`crates/poph-agent/src/resumo.rs`) dependem só desse trait, nunca do cliente concreto. Hoje existem
duas implementações: `OpenAiCompatibleClient` (`crates/poph-core/src/llm/openai.rs`, a real) e
`ProvedorFake` (`crates/poph-core/src/llm/fake.rs`, determinística, usada nos testes). Uma
implementação nova entra pelo mesmo contrato, sem mudar o motor.

## 2. O que o cliente já fala

O `OpenAiCompatibleClient` já implementa o protocolo compatível com OpenAI. Para um endpoint novo
que fale esse mesmo protocolo, **não é preciso escrever outro cliente**: basta configurar endereço,
nome da variável da chave e modelo.

**Envio** — `POST {url_base}/chat/completions`, corpo montado em `montar_corpo`
(`crates/poph-core/src/llm/openai.rs`):

- cabeçalhos `Authorization: Bearer <chave>` e `Content-Type: application/json`;
- corpo com `model`, `stream: true`, `temperature`, `max_tokens`, `messages` e, quando existem
  ferramentas, `tools`;
- mensagens nos papéis `system`, `user`, `assistant` (com `tool_calls`) e `tool` (com
  `tool_call_id`);
- imagem na mensagem do usuário como `data:<mime>;base64,...`; os mimes aceitos são `image/jpeg`,
  `image/png`, `image/gif` e `image/webp` (`MIMES_IMAGEM` em `provedor.rs`).

**Resposta** — stream SSE (eventos `data:` e o marcador `[DONE]`). O cliente lê
`choices[0].delta.content` como texto, monta `tool_calls` por índice (id, nome e argumentos em
pedaços), lê `usage` (`prompt_tokens` e `completion_tokens`) e traduz `finish_reason` (`stop`,
`tool_calls`/`function_call`, `length` ou outro) no evento `Fim`.

**Comportamento do cliente** — aceita `http://` e `https://` (TLS com rustls e as raízes do sistema);
retenta só em 429, 5xx ou falha de conexão **antes** do primeiro byte da resposta, com espera que
dobra a cada tentativa até 30 s; o timeout vale por tentativa e não é retentado; falha de TLS e erro
4xx são definitivos; a chave não aparece em log, erro ou `Debug` e é substituída por `[oculto]` se o
provedor a ecoar. As vagas de simultaneidade ficam presas até o stream terminar ou ser solto.

## 3. Duas formas de usar um endpoint novo

### (a) Um provedor para a instância inteira — bloco `[llm]`

```toml
[llm]
provedor = "openai_compativel"
url_base = "https://api.exemplo.com/v1"
chave_env = "MINHA_CHAVE_LLM"
modelo = "meu-modelo"
```

`llm.provedor` aceita `openai_compativel` e `antigravity2api` (`provedor_llm_aceito`, em
`crates/poph-types/src/config.rs`): os dois nomes usam o **mesmo** cliente, sem comportamento
diferente.
`url_base` deve começar com `http://` ou `https://`. `chave_env` é o **nome da variável de ambiente**
que guarda a chave — o valor nunca vai para o arquivo de configuração. Os outros campos do bloco
(`recusa_turno_modelo`, `timeout_segundos`, `retentativas`, `simultaneos`, `temperatura`,
`max_tokens_saida`) seguem a tabela de `CONFIGURACAO.md`.

Use esta forma quando todos os agentes da instância devem falar com o mesmo endpoint, ou para trocar
o gateway padrão. Não há escolha por agente: todos caem nesse cliente.

### (b) Vários provedores — blocos `[provedores.<nome>]`

```toml
[llm]
modelo = "gemini-3.8-flash-tiered"
chave_env = "ANTIGRAVITY_API_KEY"

[provedores.openai-forte]
url_base = "https://api.exemplo.com/v1"
chave_env = "CHAVE_DO_FORTE"
modelo = "gpt-de-exemplo"
simultaneos = 4
```

O nome do provedor (`openai-forte` no exemplo) só pode ter letras minúsculas, números, `-` e `_`, e
não pode ser `openai_compativel` nem `antigravity2api`: esses dois são nomes de protocolo,
reservados. Todos os campos do bloco são opcionais; o que for omitido herda de `[llm]`
(`ProvedorConfig::efetivo`, em `crates/poph-types/src/config.rs`). Se `url_base` ou `chave_env`
estiverem vazios nos dois níveis, a validação recusa com o nome do provedor na mensagem.

Cada agente escolhe o provedor pelo campo `provider`: `--provider <nome>` na CLI, `provider` na API
(gravado no banco como `provedor`). Vazio, `openai_compativel` ou `antigravity2api` caem no provedor
padrão de `[llm]` — foi assim que os agentes já gravados continuaram funcionando sem migração. Um
nome declarado em `[provedores]` usa aquele cliente, aquele modelo e o teto dele de chamadas
simultâneas (`simultaneos`).

Use esta forma quando precisar de mais de um provedor ao mesmo tempo, com chaves, modelos ou limites
diferentes por agente. O resumo rolante segue no provedor padrão (decisão tomada na PV-02); a escolha
por agente vale para o turno.

## 4. Regras que são do gateway, não do contrato

`llm.recusa_turno_modelo` (padrão `true`) nasceu de uma resposta HTTP 400 do `antigravity2api`:
"Requests ending with a model turn are not supported". Com `true`, `validar_pedido`
(`crates/poph-core/src/llm/provedor.rs`) recusa antes do envio um pedido cuja última mensagem é do
modelo, e o erro cita a opção. Num provedor que aceita esse formato, configure `false` em `[llm]` ou
no bloco do provedor: o cliente envia o pedido normalmente.

As demais checagens de `validar_pedido` valem para qualquer provedor: pedido sem mensagem,
ferramenta sem nome ou com nome repetido, tipo de imagem fora dos quatro aceitos e imagem vazia.

## 5. Como provar sem gastar chave

`crates/poph-core/tests/llm_servidor_falso.rs` sobe um servidor HTTP falso que fala o protocolo
completo. É o teste de conformidade a copiar e estender quando surgir um provedor novo. Ele cobre:
texto em stream com uso e fim; chamada de ferramenta de ida e volta com argumentos completos;
imagem como `data:`; retentativa em 429 e 5xx sem alterar o pedido; erro definitivo sem retentativa e
sem ecoar a chave; esgotamento de retentativas; timeout sem retentativa; cancelamento no meio do
stream com liberação das vagas; resposta cortada; e os dois casos da regra de turno do modelo (com a
regra ligada nada é enviado; desligada, o pedido chega ao servidor).

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p poph-core --locked --test llm_servidor_falso
```

Conferido em 24/11/2026: **11 passed; 0 failed**.

`crates/poph-core/tests/llm_real.rs` faz uma chamada real, está marcado como `#[ignore]` e exige
`ANTIGRAVITY_API_KEY` definida. Só rode com autorização para gastar a chave.

## 6. Erros comuns

- **Nome reservado ou inválido em `[provedores]`** — `openai_compativel` e `antigravity2api` são
  recusados ("é nome de protocolo e não pode ser usado como nome de provedor"); nome com maiúscula,
  espaço ou outro caractere é recusado ("use letras minúsculas, números, `-` e `_`").
- **Variável de chave ausente** — na subida aparece o aviso
  ``[aviso] provedor `nome` sem a variável X no ambiente: agentes que apontam para ele não atendem``;
  o provedor não entra no registro e os agentes dele falham com
  ``provedor `nome` não existe nesta instância; use vazio (o padrão do [llm]) ou um de [provedores]: ...``.
  Sem a chave do provedor **padrão**, `poph serve` mantém o turno desligado (as mensagens ficam na
  fila) e `poph chat` recusa a abertura.
- **`provider` desconhecido no agente** — recusado antes de gravar pela CLI (`--provider: ...`) e
  pela API (validação do campo `provider`), com a lista dos provedores disponíveis.
- **Endereço fora do protocolo** — `url_base` que não começa com `http://` ou `https://` é recusada
  pelo cliente ("o endereço do provedor deve começar com http:// ou https://").
- **`chave_env` inválida** — vazia, com espaço ou que não começa por letra/`_` é recusada na
  validação da configuração.
- **`poph doctor`** mostra uma linha `provedor <nome>` por provedor configurado (endereço,
  simultâneos e situação da variável de chave) e um aviso `provedor dos agentes` quando há agentes
  apontando para nome que não existe. É o primeiro lugar a olhar quando um agente não atende.
