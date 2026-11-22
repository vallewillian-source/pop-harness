# POP Harness (Rust) — Registro de decisões

> Registro das decisões tomadas **depois** da especificação original (PRD e lista de tarefas, removidos
> em 21/11/2026). As decisões de base D1 a D13 estão em [`VISAO_GERAL.md`](VISAO_GERAL.md) §4 e
> continuam fechadas; aqui ficam as decisões operacionais e as respostas do mantenedor que mudam o desenho.
> Cada decisão diz onde ela morde, para quem implementa não precisar procurar.

| ID | Data | Decisão | Manda em |
|---|---|---|---|
| D-01 | 19/11/2026 | Rust instalado só no usuário (`~/.rustup`, `~/.cargo`), sem sudo: rustup 1.29.3, rustc/cargo 1.100.0 stable, alvos `x86_64-unknown-linux-gnu` e `aarch64-unknown-linux-gnu`, clippy, rustfmt, cargo-deny 0.22.0, cargo-audit 0.23.0. | HR-00, HR-04 |
| D-02 | 19/11/2026 | Sem linker cruzado na estação: o alvo aarch64 fica só baixado e validado por `cargo check`. O binário ARM é compilado **direto na máquina ARM64 de testes** na HR-56, com autorização do mantenedor na hora — usar a máquina ARM64 de testes continua exigindo autorização. | HR-01 (ressalva ARM), HR-56 |
| D-03 | 19/11/2026 | O Console serve as fontes da SofaUI em `/res/fonts` como estático e define fonte, cor e fundo do `body`. A biblioteca **não** é alterada para caminho relativo. | HR-38, HR-37 |
| D-04 | 19/11/2026 · mantenedor | A lista de ferramentas habilitadas por agente **filtra de verdade** o que o agente pode usar. Deixa de ser campo decorativo como no POP Atende (HR-03 §7). | HR-09, HR-13, HR-19, HR-24 |
| D-05 | 19/11/2026 · mantenedor | Base de conhecimento com status inativo **sai do índice e da busca**, e agente com status inativo **para de responder** (não recebe turno novo). | HR-09, HR-16, HR-17, HR-26 |
| D-06 | 19/11/2026 · mantenedor | Regras de rota por intenção ficam **fora da V1**: sem tela e sem implementação. Podem constar como item futuro, sem gancho no código. | HR-04, HR-21, HR-37, HR-52 |
| D-07 | 19/11/2026 · mantenedor | **Metas numéricas da V1 fixadas como o spike HR-01 propôs (M1–M13)**, sem aperto: 12 KB por agente ocioso, 96 KB por agente indexado, 150 MB com 10 mil ociosos, 450 MB com 2 mil índices, LRU de 512 MB, latência interna ≤ 2 ms p50 / ≤ 5 ms p95, escrita ≥ 50 mil msg/s e p95 ≤ 5 ms a 2.000 msg/s, ≥ 20 mil leituras/s, ingestão 2.000 msg/s por instância, 200 turnos simultâneos configuráveis, alerta se o primeiro texto passar de 12 s p95, carga com 10 mil agentes + 500 conversas ativas em ARM64. | HR-11, HR-12, HR-16, HR-55, HR-56, HR-59 |
| D-08 | 19/11/2026 · mantenedor | **Corte de crates aprovado** com a quebra do `poph-core` em três: `poph-types`, `poph-store` (SQLite + memória), `poph-rag` (embedding + índice, único `unsafe`/FFI), `poph-agent` (LLM, Jev, compositor, turno), `poph-server` (axum, API, SSE, plugins, Evolution) e `poph` (binário + console embutido). | HR-04b, HR-06 a HR-13, HR-15 a HR-27, HR-56 |

## Efeito de cada decisão no desenho

### D-04 — Ferramentas habilitadas filtram
- `agent_instructions.structured_json.ferramentas` (nome, habilitada, quando usar) passa a ser
  **fonte de verdade** do toolset: o motor só oferece ao LLM as ferramentas marcadas como
  habilitadas para aquele agente. Ferramenta desabilitada não aparece na lista enviada ao provedor.
- Consequência para a HR-09: o campo precisa ser tipado e validado (nome de ferramenta conhecido,
  lista fechada do motor), não texto livre.
- Consequência para o turno (HR-13): o laço de ferramentas rejeita chamada de ferramenta não
  habilitada mesmo que o modelo invente o nome (erro de ferramenta, não execução).
- Sem configuração explícita, vale o padrão por tipo de agente definido na HR-09 — a decisão não
  autoriza "tudo liberado".

### D-05 — Inativo quer dizer inativo
- Item/grupo de conhecimento inativo não entra em `rag.db` (ou sai na reindexação) e não volta na
  busca híbrida nem na ferramenta `consultar_base`.
- Agente inativo não abre turno novo: mensagem recebida da integração é registrada e descartada
  sem resposta (com registro em `audit_log`/`turns`), no mesmo padrão de conexão pausada.
- Conversa em andamento com agente que ficou inativo no meio não continua o turno.

### D-06 — Roteamento por intenção fora da V1
- Não se cria tabela de regras de intenção no `harness.db`; o encaminhamento é só transferência
  para departamento (G1/G4).
- Não aparece tela de regras no Console (HR-37/HR-38) nem rota na API (HR-21).
- O importador (HR-52) ignora essas regras ao trazer agente do POP Atende, sem falhar por isso.

### D-07 — Metas numéricas valem como critério de aceite
- A lista completa, com a medida que sustenta cada número, é `spike/RELATORIO_HR01.md` §9 (M1–M13);
  os valores da tabela acima são a versão curta e não substituem a tabela original.
- HR-55 (carga) e HR-59 (aceite) medem contra esses números; número não medido é "pendente", nunca
  "equivalente". As medidas em ARM64 são feitas na HR-56, na máquina ARM64 de testes.
- Metas de latência percebida (M12) monitoram o provedor: não são desculpa para o Harness gastar mais.

### D-08 — Corte de crates aprovado
- O workspace atual (HR-04 parte A) tem `poph-core` quase vazio. A quebra em `poph-store`,
  `poph-rag` e `poph-agent` entra **a partir da HR-04b/HR-06**, sem replay: `poph-core` deixa de
  existir quando o primeiro dos três trouxer código de verdade, e o nome nunca é reaproveitado.
- Regra de fronteira: `unsafe`/FFI só em `poph-rag` (embedding) e na parte de SQLite; `poph-agent`
  não depende de axum, para o turno ser testável pela CLI (HR-13) antes da API (HR-20).
- `exclude = ["spike"]` no workspace continua valendo; o spike não vira crate.

### D-09 — RAG local: `ort` + int8, trechos curtos e extração de arquivo isolada
- Runtime do embedding: `ort` (ONNX Runtime) 2.0.0-rc.15 linkado estaticamente, mesma família de
  modelo do POP Atende (MiniLM multilíngue, janela 128, tokenizador original do `sentence-transformers`).
- Quantização: int8 na V1, com fp32 como reserva. A troca exige reindexação (`poph rag reindex --model`).
- Chunker: ~350 caracteres (~100 tokens) com sobreposição de 80, respeitando parágrafo e frase — é o
  que faz o int8 passar do limiar de paridade (R@3 0,80 contra 0,45 com trechos de 1000 caracteres).
  O PRD §9 já autorizava trechos menores caso o truncamento em 128 tokens se confirmasse — confirmou:
  só 47% do texto de um trecho de 1000 caracteres é lido pelo embedding hoje.
- Consequência para HR-15/HR-16: gravar a versão do chunker e o sha256 do modelo em `rag.db`, e
  medir a paridade de verdade com o corpus de aceite (o int8 ficou em 0,90, no limite).
- Arquivos: PDF com `pdf-extract`, Office com `calamine` (xlsx/xls) e `zip`+`quick-xml` (docx/pptx).
  A extração **roda em processo isolado**, com limite de memória e timeout: um PDF de 149 KB levou
  2,5 GB de RSS nos dois leitores testados. PDF só de imagem devolve vazio e o turno responde
  "não consegui ler"; `.doc`/`.ppt` são recusados.

## Pendências de decisão (não bloqueiam o código)
- Uso da máquina ARM64 de testes para o teste ARM: pedir autorização na HR-56.
- ~~Token de fundo escuro do Console (`#151719` na SofaUI × `#121a24` na instrução antiga)~~ —
  resolvido na HR-37: `#151719` é o canônico e `#121a24` fica revogado (ver D-11).

### D-10 — TLS no cliente do provedor e alcance da checagem ARM na estação
- O cliente do provedor (HR-10) foi aceito **sem TLS**: o gateway antigravity roda em `http://127.0.0.1`,
  e `https` é recusado com erro explícito. Isso vale só até lá: **antes da HR-26** (Evolution) e da
  **HR-28** (Jev, https), o cliente passa a usar `rustls` — as duas integrações falam https com o mundo.
  **Feito na HR-10b** (ver D-16): os dois clientes já falam https. A HR-26 (Evolution) usa o mesmo
  caminho; quem abrir a HR-26 não precisa mais habilitar TLS.
- **Checagem `cargo check --target aarch64` na estação:** só vale para crates de código puro. Tudo que
  compila C (SQLite embutido do `rusqlite`, `ring`/`aws-lc` do TLS, `onig` do tokenizador) falha por falta
  de compilador C cruzado — não é defeito do nosso código. A verificação ARM de verdade é a HR-56, na máquina ARM64 de testes,
  com os mesmos comandos. **Decisão do mantenedor (19/11/2026, 19:49):** não instalar o compilador cruzado de
  ARM na estação; a checagem ARM completa fica para a HR-56, na máquina ARM64 de testes. O CI local não gateia por ela.
- **Limite de chamadas simultâneas ao LLM por org:** usa as chaves que a config já tem (`limites`), sem
  chave nova; a ligação com o turno fica na HR-13.

### D-11 — Console: fundo `#151719`, fontes em `/res/fonts` e `body` no hospedeiro
Confirmado na HR-37 (aceita em 19/11/2026). O fundo escuro canônico do Console é o token `--pop-bg`
(`#151719`) da SofaUI/DESIGN.md; a menção a `#121a24` numa instrução antiga fica **revogada**. O tema
claro correspondente é `#F5F6F7`. O binário hospedeiro serve `IBM Plex Sans`/`IBM Plex Mono`
estaticamente em `/res/fonts/` e define fonte, cor e fundo no seletor `body`; a biblioteca não é
alterada para caminho relativo (HR-05 → HR-38). Rota por intenção continua fora da interface (D-06).

### D-12 — TLS entrou na HR-15 (download do modelo), não só na HR-26/28
O D-10 previa TLS só antes da HR-26/28. O `poph model install` precisa baixar por https, então o
`poph-rag` já usa `rustls`. O `ort` foi configurado com `tls-rustls` em vez da feature padrão
`tls-native`, que puxava OpenSSL (banido no `deny.toml`); OpenSSL não entra no binário, conferido com
`ldd`. Sobra uma exceção **estreita** no `deny.toml` para `webpki-roots` (CDLA-Permissive-2.0), que é
só dependência de build do download do ONNX Runtime e não vai para o binário. O cliente do LLM
continuou sem TLS até a **HR-10b**, que o habilitou nos dois clientes (D-16).

### D-13 — Lote do embedding: agrupa na fila, roda um trecho por vez no modelo
A quantização int8 dinâmica calcula a escala sobre o tensor do lote inteiro: o mesmo texto dentro de
um lote saía com cosseno 0,991 contra ele sozinho (0,991 < 0,9999 exigido na HR-15). Com um trecho por
vez, o cosseno volta a 1,0000000 e não há custo medido (12,9–13,8 ms por trecho em lote de 32 contra
14,6 ms no spike). A fila continua agrupando (32 trechos / 10 ms), o modelo é chamado por trecho.
Consequência para HR-16/HR-17: consulta e documento usam o mesmo caminho, então os vetores comparam.

### D-14 — Teto de RAM do modelo de embedding na V1: 470 MB
Medido em release, processo isolado, 3 rodadas: 458 MB de RSS depois da carga (+447 MB sobre 11 MB de
base), contra 438 MB no spike da HR-02 (+4,6%). A diferença não foi investigada e não bloqueia; adoto
**470 MB** como teto de referência na V1 e mantenho a medição no relatório da HR-15. Revisar se a
HR-55 (carga) ou a HR-59 mostra pressão de memória na máquina ARM64 de testes de 12 GB.

### D-15 — Corte de relevância do RAG começa em 0 (paridade), revisto na HR-59
A HR-17 entrega `rag_min_score` configurável por agente, com padrão **0** — o mesmo comportamento do
POP Atende hoje. Com 0, o top-k volta mesmo sem relação: na demonstração da HR-17 vieram 3 trechos
onde só 1 era relevante. Mantenho 0 por dois motivos: (a) paridade declarada com o POP Atende é
critério do PRD, e mudar o padrão muda o que o cliente final recebe sem medição na base real; (b) o
corpus da HR-17 é sintético (24 itens), e um corte calibrado nele pode barrar trecho bom na base da
Exemplo. A calibração do padrão é item da HR-59, com a base real; até lá o corte se ajusta por agente.

### D-16 — TLS nos clientes do provedor e do Jev (HR-10b) e o que o check ARM deixou de cobrir
Feito na HR-10b (20/11/2026). Os dois clientes HTTP (`poph-core` para o provedor, `poph-jev` para o
Jev) passam a falar `https` com a feature `rustls` do `reqwest` — a mesma do download do modelo no
`poph-rag`, com as **raízes de confiança do sistema** (rustls-platform-verifier) e sem OpenSSL, que
segue banido no `deny.toml`. `http://` continua aceito (gateway local).
- **Falha de TLS é definitiva**: certificado não confiável, nome do servidor diferente do certificado
  ou handshake recusado viram erro próprio (`ErroLlm::Tls` / `ErroJev::Tls`), sem retentativa, com a
  mensagem da causa (ex.: `invalid peer certificate: UnknownIssuer`). Não existe opção pública de
  "ignorar certificado"; o teste usa uma raiz extra, nunca validação desligada.
- **O que o `cargo check --target aarch64` da estação deixou de cobrir:** `poph-core` e `poph-jev`
  saíram do passo, porque o TLS traz `aws-lc-sys`, que compila C — o build cruzado falha na estação
  por falta de `aarch64-linux-gnu-gcc` (`error occurred in cc-rs: failed to find tool
  "aarch64-linux-gnu-gcc"`), o mesmo motivo do D-02/D-10. Continuam no check: `poph-types`,
  `poph-files`, `poph-plugins` e `poph-plugin-eco`. A prova ARM de verdade desses dois crates (e do
  binário inteiro) é o build na máquina ARM64 de testes, na HR-56; o CI local não gateia por ela.

### D-17 — Tarefas suspensas (21/11/2026, mantenedor)
HR-29c (nova rodada de ajuste e medição do G1), HR-52 (importador de agente do POP Atende), HR-55
(teste de carga) e HR-59 (aceite ponta a ponta) foram suspensas para priorizar novos desenvolvimentos
antes dos testes finais. A documentação foi mantida e nenhum trabalho em execução foi interrompido.
Estado em `ESTADO.md`. Enquanto a HR-55 estiver suspensa, as metas de D-07 não têm medição em escala.

### D-18 — Colunas da tabela de organizações do Console (21/11/2026, mantenedor)
Nó/Porta, Agentes e WhatsApp, previstas no desenho aprovado, ficam **fora** por enquanto: dependem de
a API devolver esses dados na listagem de orgs. Nenhuma tarefa foi aberta.

### D-19 — Gates do Jev por padrão (20/11/2026, mantenedor)
Ligados por padrão: G3, G5 e G8. Desligados: G1, G2, G4, G6 e G7. Risco à saúde **não** vira
transferência incondicional: o G1 depende da política configurada pela empresa e fica desligado até a
rodada de ajuste e nova medição (HR-29c, suspensa). Não há ajuste de gate por organização: vale o global
(`poph.toml`) ou o por agente. Detalhe em `JEV.md`.

### D-20 — Falha do modelo, texto longo e backup (20 e 21/11/2026, mantenedor)
- Falha do modelo no meio do turno: a mensagem é repetida depois, com limite de tentativas
  (`tentativas_falha_modelo`, padrão 3); não avisa o cliente e não deixa sem resposta. `TURNO.md`.
- Resposta acima de 4000 caracteres no WhatsApp é cortada com `…` numa mensagem só, nunca dividida.
  `EVOLUTION.md`.
- Importar backup que sobrescreveria dados já existentes na org é recusado por padrão e exige
  confirmação explícita. `BACKUP.md`.
- WhatsApp: a HR-26 fecha sem a conta de teste; o teste com número real fica como pendência, não como
  bloqueio.

### D-21 — Matrix como módulo do Harness (21/11/2026, mantenedor) — direção, sem especificação
Levar o Matrix para dentro do Harness como módulo desligável, em crate separado, com poucas mudanças no
núcleo, e criar um segundo papel de integração (observador da conversa). Nada especificado nem
implementado; o levantamento do que falta está em `CONTRATO_CANAL.md` §7.
