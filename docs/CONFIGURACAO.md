# POP Harness: configuração, logs e `poph doctor`

Entrega da HR-06. Padrões marcados como **proposta** foram escolhidos nesta etapa e podem ser
calibrados; os demais vêm do PRD (§7.4, §8.4, §10, §15) ou do POP Atende.

## Precedência

Do menos para o mais forte: **padrão < arquivo TOML < variável de ambiente**.

Arquivo, na ordem de procura:
1. `--config <arquivo>` (vale para qualquer comando);
2. variável `POPH_CONFIG`;
3. `poph.toml` na pasta atual, se existir.

Sem arquivo, valem os padrões e o ambiente. Um arquivo pedido e inexistente é erro.

## Erros de configuração

Cada erro diz de onde veio (o arquivo ou a variável) e qual é a chave:

- `arquivo.toml: chave \`servidor.portaa\` inválida: unknown field ...` (chave desconhecida)
- `arquivo.toml: erro de sintaxe no TOML: ...` (a mensagem traz linha e coluna)
- `POPH_SERVIDOR_PORTA: valor inválido (esperado um número inteiro)`
- `configuração inválida: campo servidor.porta inválido: ...` (regra de faixa ou formato)

## Campos do arquivo TOML

| Chave | Padrão | Regra | Origem |
|---|---|---|---|
| `servidor.endereco` | `127.0.0.1` | endereço IP | proposta |
| `servidor.porta` | `7480` | 1024 a 65535 | proposta |
| `dados.diretorio` | `data` | caminho não vazio | HR-04 (`.gitignore`) |
| `dados.harness_db` / `rag_db` / `blobs` | dentro de `diretorio` | opcional | PRD §7.1 |
| `dados.modelos` | `modelos` dentro de `diretorio` | pasta do modelo de embedding (`poph model install`) | HR-15 (D-09) |
| `log.nivel` | `info` | trace, debug, info, warn ou error | PRD §15 |
| `log.formato` | `legivel` | `legivel` ou `json` | PRD §15 |
| `retencao.dias` | `30` | `0` = não apagar | PRD §7.4 |
| `limites.turnos_simultaneos` | `200` | 1 a 10000 | D-07 |
| `limites.turnos_simultaneos_por_org` | `50` | 1 até o total | **proposta** |
| `limites.taxa_por_org_por_minuto` | `600` | 1 a 100000 | **proposta** |
| `llm.provedor` | `antigravity2api` | só este na V1 | PRD §8.4 |
| `llm.url_base` | `http://127.0.0.1:8045/v1` | http ou https | POP Atende (`ANTIGRAVITY_BASE_URL`) |
| `llm.modelo` | `gemini-3.8-flash-tiered` | não vazio | PRD D6 |
| `llm.timeout_segundos` | `150` | 1 a 600 | POP Atende (`ANTIGRAVITY_TIMEOUT_SECONDS`) |
| `llm.retentativas` | `2` | 0 a 5, só em erro transitório | POP Atende (`LLM_RATE_LIMIT_RETRIES`) |
| `llm.simultaneos` | `32` | 1 a 1000 | **proposta** |
| `llm.temperatura` | `0.0` | 0 a 2 | POP Atende (padrão do `LLMService`) |
| `llm.max_tokens_saida` | `4096` | 1 a 131072 | POP Atende (`AGENT_MAX_OUTPUT_TOKENS`) |
| `jev.ativo` | `false` | liga o Jev; só depois de medido (PRD §10) | PRD §10 |
| `jev.url_base` | `https://api.typesafe.ai` | http ou https | POP Atende (`TYPESAFE_BASE_URL`) |
| `jev.modelo` | `jev-latest` | não vazio | POP Atende (`JEV_MODEL`) |
| `jev.timeout_segundos` | `8` | 1 a 60 | PRD §8.5 |
| `jev.retentativas` | `1` | 0 a 3, só em 429 ou 5xx | PRD §8.5 |
| `jev.gates.g1` … `g8` | g3, g5 e g8 `true`; os outros `false` | liga cada gate; padrão de instalação desde 20/11/2026 | PRD §10, `docs/JEV.md` |
| `[evolution] url_base` | ausente | seção existe = integração WhatsApp configurada | POP Atende |
| `[[plugins.integracoes]] kind` / `programa` | nenhum | `kind` único e não vazio; `programa` é caminho local do plugin | HR-44c |
| `[[plugins.integracoes]] argumentos` | `[]` | argumentos do processo; **nunca** segredo (aparece em `ps`) | HR-44c |
| `[matrix] habilitado` | `false` | liga o módulo Matrix; seção ausente ou `false` = módulo morto (nada dele carrega) | MX-07 |
| `matrix.homeserver_url` | — | endereço base da API cliente do Synapse (`http://` ou `https://`); conferido só com o módulo ligado | MX-07 |
| `matrix.server_name` | — | domínio dos ids Matrix, como no `server_name` do Synapse (ex.: `matrix-dev.local`, `localhost:18008`) | MX-07 |
| `matrix.prefixo` | `ph` | prefixo de usuários virtuais, aliases e `sender_localpart`: `[a-z0-9]{1,12}` | MX-07 |
| `matrix.concorrencia` | `8` | salas espelhadas em paralelo; 1 a 64 | MX-07 |
| `matrix.intervalo_ms` | `500` | espera entre leituras do cursor; 50 a 60000 ms | MX-07 |
| `matrix.poder_agente` | `50` | power level do fantasma do agente na sala; 0 a 100 | MX-07 |
| `matrix.midia_retencao_dias` | `4` | dias de retenção da mídia no Synapse (fase 2); 1 a 365 | MX-07 |

O programa do plugin é caminho **local do servidor** (nunca vem de requisição). Com ele o servidor
descobre o manifesto de cada `kind` — subindo o plugin uma vez e guardando o resultado em memória —
e valida a configuração da integração. Sem a declaração, `config_json` de integração é recusado com
`422 manifest_indisponivel` e a rota do manifesto responde `404`.

## Variáveis de ambiente

Campos não secretos (sobrepõem o arquivo):

| Variável | Campo |
|---|---|
| `POPH_SERVIDOR_ENDERECO` | `servidor.endereco` |
| `POPH_SERVIDOR_PORTA` | `servidor.porta` |
| `POPH_DADOS_DIRETORIO` | `dados.diretorio` |
| `POPH_LOG_NIVEL` | `log.nivel` |
| `POPH_LOG_FORMATO` | `log.formato` |
| `POPH_RETENCAO_DIAS` | `retencao.dias` |
| `POPH_LLM_URL_BASE` | `llm.url_base` |
| `POPH_LLM_MODELO` | `llm.modelo` |
| `POPH_JEV_ATIVO` | `jev.ativo` (`true` ou `false`) |
| `POPH_JEV_URL_BASE` | `jev.url_base` |

Variável vazia não sobrepõe nada.

`POPH_CLUSTER` não é campo do `poph.toml`: aponta o caminho do `cluster.toml` do roteamento entre
instâncias (padrão: `cluster.toml` no mesmo diretório do `poph.toml`). Formato e regras em
`docs/CLUSTER.md`.

`RUST_LOG`, se definida, vence `log.nivel` no filtro de logs.

## Segredos (só no ambiente)

Nunca vão para arquivo, relatório, log nem resposta da API. O `poph doctor` mostra só se estão
definidos.

| Variável | Uso | Obrigatória |
|---|---|---|
| `POPH_CHAVE_CIFRA` | cifra dos segredos guardados (AES-256-GCM, a partir da HR-07) | sempre |
| `ANTIGRAVITY_API_KEY` | provedor de LLM | sempre |
| `TYPESAFE_API_KEY` | Jev | só com `jev.ativo = true` |
| `EVOLUTION_API_KEY` | Evolution (WhatsApp) | só com a seção `[evolution]` |
| `POPH_CLUSTER_SECRET` | segredo compartilhado entre as instâncias (HMAC do encaminhamento, HR-50b) | só com `cluster.toml` que declara instâncias |
| `POPH_MATRIX_AS_TOKEN` | token do Application Service do módulo Matrix, que o Harness apresenta ao Synapse | só com `[matrix] habilitado = true` |
| `POPH_MATRIX_HS_TOKEN` | token do Application Service que o Synapse apresenta ao Harness | só com `[matrix] habilitado = true` |
| `POPH_MATRIX_ADMIN_TOKEN` | API administrativa do Synapse (apagar mídia expirada) | **fase 2**; ainda não é lido |

Os nomes de `ANTIGRAVITY_API_KEY`, `TYPESAFE_API_KEY` e `EVOLUTION_API_KEY` são os mesmos do POP
Atende, para reaproveitar o mesmo ambiente. Valor vazio ou só com espaços conta como ausente.

O módulo Matrix tem uma regra própria: **desligado, nada dele é carregado**. Sem `[matrix]` no arquivo,
ou com `habilitado = false`, a configuração da seção não é conferida (pode ficar vazia), os dois tokens
não são lidos e nenhuma tarefa ou rota do módulo existe. Com `habilitado = true`, a seção é validada —
erro nomeia a chave e nunca repete o valor — e os dois tokens passam a ser obrigatórios: faltando um, a
subida recusa com o nome da variável, sem imprimir o valor. Os tokens vivem no tipo `Sensivel` e não
aparecem em `Debug`, `Display`, log, erro ou resposta (`crates/poph-matrix/src/config.rs`).

O cliente da API do Matrix (`crates/poph-matrix/src/cliente.rs`, MX-09) usa **prazo fixo de 10 s por
chamada** (`TEMPO_LIMITE_PADRAO`): a seção `[matrix]` não tem chave de prazo, e `concorrencia` e
`intervalo_ms` são do laço, não do cliente. `429` com `retry_after_ms` é esperado e repetido **uma
vez** (até 60 s de espera); acima disso o cliente devolve o erro com a espera pedida e quem espera é o
laço. Nenhuma outra repetição acontece dentro do cliente: o laço decide pelos erros transitórios
(rede, prazo, 429, 5xx) e definitivos (4xx, resposta fora do contrato).

### Registro do Application Service (`poph matrix registro`)

```bash
poph matrix registro --saida /etc/poph/registro.yaml --url https://harness.exemplo.com
poph matrix registro --verificar /etc/poph/registro.yaml
```

- Lê a seção `[matrix]` e os dois tokens do ambiente; grava o arquivo em **0600**, recusa sobrescrever
  sem `--forcar` e **nunca** imprime token.
- `--url` é o endereço por onde o **Synapse alcança o Harness** (o `homeserver_url` da seção é o
  Synapse). Sem `--url`, vale `POPH_MATRIX_URL_AS`.
- `--id` (padrão `poph-matrix`) vira o `id` do Application Service; na fase 3, cada instância tem o seu.
- `--verificar` compara o arquivo com a configuração e os tokens atuais e aponta o campo divergente
  (`url`, `sender_localpart`, `rate_limited`, `namespaces.*`, `as_token`, `hs_token`) sem repetir
  valor. O Synapse só lê registros na subida: depois de gerar, reinicie o Synapse do ambiente.

### Operar o módulo Matrix por org (`poph matrix estado|ligar|desligar|espaco`, MX-23)

```bash
poph --offline --org loja matrix estado [--json]
poph --offline --org loja matrix ligar [--json]
poph --offline --org loja matrix desligar [--json]
poph --offline --org loja matrix espaco [--json]
poph --offline --org loja matrix espaco --departamento <id> [--espaco <!id|#alias>] [--json]
poph --offline --org loja matrix espaco desligar --departamento <id> [--json]
```

- Como os outros comandos administrativos da CLI (`org`, `department`, `token`), exigem `--offline` e
  `--org` (slug ou id) e operam direto no banco. A CLI **não** usa token e **não** fala com o servidor.
  O `poph serve` pega a mudança no ciclo seguinte, sem reiniciar.
- `estado` mostra se o módulo está ligado, o corte do backfill, quantas mensagens faltam espelhar, as
  falhas isoladas e o último motivo. Não mostra se o Synapse está respondendo: essa sonda roda no
  servidor (na API, o campo `connected`).
- `ligar` e `desligar` leem a versão da org (a mesma que o `ETag` da API) antes de gravar. Se ela mudar
  entre a leitura e a gravação, nada é gravado e a saída é `3`. Desligar preserva o corte do backfill e
  não apaga sala nem espaço.
- `espaco --departamento` cria o espaço do departamento; com `--espaco` liga um espaço que já existe no
  Element (id `!…` ou alias `#…`). Fala com o Synapse de `[matrix]`, então exige `POPH_MATRIX_AS_TOKEN` e
  `POPH_MATRIX_HS_TOKEN` no ambiente. Repetir não cria segundo espaço.
- `espaco desligar --departamento <id>` tira o espaço do departamento só do Harness. Nada sai do Element.
  Na próxima alteração de conversa desse departamento, a sala volta a ser posicionada no mesmo espaço.
- `espaco` sem `--departamento` lista os espaços da org.
- Saída: `0` ok; `1` erro (org ou departamento inexistente, configuração, Synapse ou banco); `3` a versão
  mudou entre a leitura e a gravação. `--json` tem formato estável, com `versao_relatorio` = 1.
- Ao contrário da API, o comando grava auditoria com ator `cli`.

## `poph doctor`

```
poph doctor            # saída legível
poph doctor --json     # saída estável para máquina (versao_relatorio = 1)
poph --config arq.toml doctor
```

Confere: binário e compilador usados; arquivo de configuração; configuração válida; pasta de
dados (existe e aceita escrita, ou pode ser criada); `harness.db` e `rag.db` (formato SQLite e
versão de esquema, lida na tabela `schema_migrations`, em modo somente leitura); variáveis
obrigatórias.

- Escreve só um arquivo de teste na pasta de dados, e apaga logo depois.
- Banco ausente é **aviso** (crie com `poph migrate`). Banco que existe e não é SQLite é **erro**.
- Código de saída: `0` sem erro (avisos não contam); `1` com erro; `2` quando roda sem comando.

## `poph migrate` (HR-07)

```
poph migrate            # texto: versão antes → depois e o que foi aplicado
poph migrate --json     # versao_relatorio = 1: harness, rag, ja_estava_em_dia
```

Cria e/ou atualiza `harness.db` e `rag.db` a partir da configuração. É **idempotente**: rodar de
novo não aplica nada e sai com `0`. Rode com o servidor parado — ele abre conexão de escrita
própria, e a regra do armazenamento é um escritor por vez. O servidor também migra sozinho ao
abrir o banco. O esquema, as tabelas e as decisões de armazenamento estão em `docs/DADOS.md`.

## `poph model install` (HR-15)

```
poph model install           # texto
poph model install --json    # versao_relatorio = 1: arquivos, bytes, sha256 e situacao
```

Baixa para `dados.modelos` o modelo de embedding (int8, 118 MB) e o tokenizador (9 MB). Cada
arquivo é baixado para um temporário, o sha256 é conferido e só então ele recebe o nome final:
hash errado nunca fica na pasta. Arquivo que já confere não é baixado de novo.

- É o **único** comando que baixa modelo. O turno nunca baixa em silêncio: se o modelo falta, o
  erro manda rodar `poph model install`.
- Carga do modelo: confere os sha256 na subida; se algum arquivo falta ou está diferente, o
  Harness não sobe e o erro diz o que fazer.
- Código de saída: `0` instalado; `1` com erro (no `--json`, o erro vem no campo `erro`).

## Logs

- Saída no erro padrão (stderr), para o JSON do `doctor` continuar válido no stdout.
- Contexto de turno: `contexto_turno(org, agente, conversa, turno)` no crate `poph`. Vale a partir da HR-13.
- Regra: nenhum conteúdo de cliente nem segredo. Valores sensíveis usam `Sensivel`, que se imprime
  como `[oculto]`.

## Testes

```
cd apps/pop-harness && bash scripts/ci.sh
```

Os testes do `poph` provam a precedência, os erros com origem e chave, o JSON válido, e que as
sentinelas de segredo não aparecem no stdout, no stderr nem em log `trace` em JSON.
