# Arquivos do POP Harness (`crates/poph-files`)

Recebimento de arquivos do cliente (HR-23): valida, lê documento e grava o blob da org. Sem HTTP,
sem LLM de verdade e sem a tabela `attachments` (essa é a HR-24). Nenhum `unsafe` neste crate.

## O que o crate faz

1. **Tamanho:** recusa acima de 16 MB (configurável, até 256 MB) antes de qualquer outra coisa.
   Nada é gravado.
2. **Tipo real:** decide pelo conteúdo (assinatura do arquivo), não pelo nome. O nome só entra
   para duas coisas: recusar extensão de programa/script e pegar arquivo enganoso.
3. **Aceitos:** JPEG, PNG, WebP e GIF (vão ao LLM como imagem); PDF, Word (`.docx`), Excel
   (`.xlsx` e `.xls`) e PowerPoint (`.pptx`) (viram texto).
4. **Recusados com mensagem:** programa executável, script, áudio, vídeo, Word e PowerPoint antigos
   (`.doc` e `.ppt`), compactado (zip, rar etc.), texto simples, formato desconhecido.
5. **Leitura isolada:** PDF e Office são lidos num processo separado (`poph-extrator`), com teto de
   memória e prazo de tempo (ver abaixo).
6. **Texto:** cortado em 20 mil caracteres, com aviso ao LLM quando corta.
7. **PDF sem texto** (escaneado): não inventa conteúdo; o LLM recebe a instrução de dizer ao cliente
   que não conseguiu ler o arquivo.
8. **Blob:** gravado em `<pasta de blobs>/<id da org>/<sha256>`, uma vez por conteúdo. Repetir o
   mesmo arquivo reaproveita o blob. O nome que o cliente mandou nunca vira caminho de disco.

## Como o texto chega ao LLM

Todo texto extraído sai em **bloco não confiável**:

```
<<ARQUIVO_NAO_CONFIAVEL>>
...texto do arquivo...
<<FIM_ARQUIVO_NAO_CONFIAVEL>>
```

Antes do bloco vai um aviso nosso (texto confiável) dizendo que aquilo é dado, não instrução.
O texto do arquivo não consegue fechar o bloco: a sequência `<<` dentro dele vira `‹‹`.
`ArquivoRecebido::para_llm()` devolve as partes na ordem: aviso, (aviso de corte, se houver), bloco.
Imagens saem como partes de imagem.

**Atenção:** o compositor do motor do turno (HR-13) precisa usar as mesmas marcas e tratar o bloco
como dado. Este crate não conhece o compositor (que ainda está no `poph-core`).

## Leitura isolada (por que processo e não thread)

- O processo principal chama `poph-extrator <formato> <limite de caracteres> <teto de memória>`,
  envia os bytes pela entrada padrão e lê uma linha JSON pela saída padrão.
- Dentro do filho, a primeira coisa é o teto de memória (RLIMIT_AS, padrão 512 MB) e desligar o
  núcleo de memória (core dump), para um abort não gravar o conteúdo do documento em disco.
- O pai controla o prazo (padrão 5 s) e mata o filho se passar. O ambiente do filho é vazio e o
  erro do filho não vai para log.
- Medido no spike (HR-02): um PDF de 149 KB chegava a 2,5 GB de memória. Com o teto de 512 MB, o
  filho é interrompido pelo sistema em ~0,4 s (build de release); o processo principal segue.
- Zip bomba (Word/Excel/PowerPoint): cada bloco XML descompactado passa de 32 MB ou o total passa de
  64 MB, a leitura é recusada antes de descompactar (testado com a fixture de 200 MB, em ~3 ms).

## Configuração

`ConfigArquivos::padrao(pasta_blobs)`:

| Campo | Padrão | Observação |
|---|---|---|
| `limite_bytes` | 16 MB | recusa acima disso, sem gravar |
| `limite_caracteres` | 20 000 | corte do texto com aviso |
| `tempo_extracao` | 5 s | passou disso, o leitor é morto |
| `memoria_extracao_bytes` | 512 MB | mínimo 128 MB |
| `caminho_extrator` | ao lado do executável, `poph-extrator` | o build precisa levar o programa junto |

A ligação com `dados.blobs` (seção de dados de `HarnessConfig`, em `poph-types`) é feita pelo crate `poph` (HR-06 e HR-24).

## Programa `poph-extrator`

Vem do mesmo crate (`src/bin/poph-extrator.rs`). Precisa estar **ao lado** do executável `poph`.
`cargo build --workspace --release` gera os dois. Se faltar, o recebimento de PDF e Office recusa com
mensagem clara ("programa de leitura de arquivos indisponível"); imagens não dependem dele.

## Erros (o que aparece para o cliente, em resumo)

| Situação | Erro | Mensagem |
|---|---|---|
| Acima de 16 MB | `Grande` | "o arquivo tem X MB e o limite é de 16 MB; não foi gravado" |
| Vazio | `Vazio` | "o arquivo está vazio" |
| Programa, script, áudio, `.doc`/`.ppt` | `TipoNaoAceito` | explica o que é e o que enviar |
| Nome `.pdf`, conteúdo `.exe` | `ExtensaoEnganosa` | "o nome termina em .pdf, mas o conteúdo é um programa executável" |
| PDF corrompido, zip bomba, planilha ilegível | `Leitura` | motivo curto, sem conteúdo do arquivo |
| Passou do prazo | `Tempo` | "passou de 5 s e foi interrompida" |
| Blob não gravado | `Gravacao` | falha de disco |

## Testes

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/pop-harness
cargo test -p poph-files                                  # 13 unitários + 26 de ponta a ponta
cargo clippy -p poph-files --all-targets -- -D warnings
cargo test --release -p poph-files --test recebimento     # opcional
```

Fixtures em `crates/poph-files/tests/fixtures/`: PDFs (texto, tabela, escaneado, quebrado, bomba
e injeção), `.docx` (inclui bomba de descompactação), `.pptx`, `.xlsx`, `.xls`, `.doc` e `.ppt`.
Os `.doc`/`.ppt`/`.xls`/injeção foram gerados com o LibreOffice; a bomba de PDF e a de Word vêm do
spike HR-02.

## Detalhes que valem saber

- **Office sem pasta `word/`:** o `infer` (detector de tipo) classifica docx e pptx mínimos como ZIP
  genérico. O crate reconhece Office pelo nome interno do pacote (`word/document.xml`,
  `xl/workbook.xml`, `ppt/slides/`), lido como texto nos bytes. Se o nome for falso, o leitor
  isolado recusa.
- **Planilha:** `.xls` e `.xlsx` são o mesmo grupo; trocar o nome entre os dois não é recusado.
- **Tabela de Word:** célula vira tabulação, linha vira quebra de linha.
- **Escape XML:** `&amp;`, `&lt;` e referências numéricas são resolvidas.

## O que não está no crate

- Gravação na tabela `attachments` e a ligação com o motor do turno: HR-24 e HR-13.
- Validação de conteúdo da imagem (só a assinatura é conferida; imagem corrompida chega ao LLM).
- Limpeza de blobs órfãos e de arquivos temporários deixados por queda no meio (retenção, HR-55).
