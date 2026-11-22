# Desenvolvimento

Como preparar o ambiente, compilar, testar e gerar pacotes do POP Harness. Para as regras de trabalho e o
mapa do código, leia o [`AGENTS.md`](AGENTS.md). Para contribuir, leia o [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Requisitos

| Item | Versão | Observação |
|---|---|---|
| Linux | x86_64 ou ARM64 | outros sistemas não foram testados |
| Rust | `1.100.0` | fixada em `rust-toolchain.toml`; o `rustup` instala sozinho. Alvos `x86_64-unknown-linux-gnu` e `aarch64-unknown-linux-gnu`, mais `clippy` e `rustfmt` |
| Compilador C | `gcc` ou `clang` | o SQLite embutido (`rusqlite`, `bundled`) e o TLS (`aws-lc-sys`) compilam código C |
| `cargo-deny`, `cargo-audit` | recentes | `cargo install cargo-deny cargo-audit --locked`; usados pelo `scripts/ci.sh` |
| Node e pnpm | Node 22.18 ou mais novo | só para o Console |

## Compilar

```bash
cargo build --workspace                 # desenvolvimento: target/debug/poph
cargo build --release --workspace       # release: target/release/poph e target/release/poph-extrator
```

O `poph` procura o `poph-extrator` na pasta do próprio executável, então os dois precisam ficar lado a lado.
Sem ele, o recebimento de PDF e Office recusa com mensagem clara; imagens não dependem dele.

### Console

O Console (`console/`, React, TypeScript e Vite) é embutido no binário: o `build.rs` do `poph-server` lê
`console/dist` se existir. Sem o bundle, o servidor compila e serve uma página mínima explicando como gerá-lo,
então o `cargo build` nunca depende do `pnpm`.

```bash
cd console
pnpm install          # precisa de rede
pnpm build            # gera console/dist
cd ..
cargo build --workspace
```

Para desenvolver com recarga automática, suba o servidor na porta padrão (7480) e rode `pnpm dev` em
`console/`: o Vite abre em `http://localhost:5199` e encaminha a API para a 7480. Se o servidor usar outra
porta, ajuste `console/vite.config.ts`.

O cliente TypeScript da API sai do documento OpenAPI do servidor. Depois de mudar uma rota:

```bash
bash console/gen-api.sh        # regenera console/src/api/schema.d.ts (não edite à mão)
```

## Testar

```bash
bash scripts/ci.sh                        # tudo, na ordem do CI
cargo test --locked -p poph-retention     # um crate
cargo test --workspace                    # todos os testes Rust
(cd console && pnpm run testar)           # testes do Console
(cd console && npx vitest run src/telas/Operacao.test.tsx)   # um teste de componente
```

O `scripts/ci.sh` roda: poda do cache, bundle do Console, checagem do cliente da API, testes do Console,
`cargo fmt --check`, `cargo clippy -D warnings`, build, testes Rust, teste de RAM do modelo, `cargo check`
para ARM64, `cargo deny` e `cargo audit`. Os passos do Console se declaram pulados quando
`console/node_modules` não existe. Termina com `CI LOCAL: OK` ou com o número de checagens que falharam.

Os testes que falam com serviços externos (modelo, Jev, EvolutionAPI, Synapse) usam servidores falsos em
processo: a suíte não precisa de rede nem de chaves. Os testes marcados com `#[ignore]` são medições ou
dependem de arquivos de fora do repositório e dizem no atributo o que precisam.

### Scripts de medição e aceite

| Script | Para que serve |
|---|---|
| `scripts/medir-memoria.sh`, `scripts/medir-memoria-roteiro.sh` | RSS e vazão do módulo Matrix com Synapse falso |
| `scripts/medir-resiliencia.sh` | queda do Harness e do Synapse no meio do espelhamento |
| `scripts/matrix-dev.sh` | sobe um Synapse de desenvolvimento com dois registros ([`docs/MATRIX_AMBIENTE_LOCAL.md`](docs/MATRIX_AMBIENTE_LOCAL.md)) |
| `scripts/plugin-eco.sh` | exercita o plugin de exemplo |
| `scripts/jev-relatorio.py`, `scripts/jev-falso.py` | relatório e servidor falso do Jev ([`docs/JEV.md`](docs/JEV.md)) |

## Pacote de distribuição

```bash
bash scripts/build-dist.sh [pasta_de_saida]
```

Compila em release com o `Cargo.lock`, reúne `poph` e `poph-extrator`, gera `SHA256SUMS` e um `.tar.gz` ao
lado da pasta. O script recusa sobrescrever uma saída existente. O pacote é **nativo** da arquitetura onde o
script roda: não há compilação cruzada. O ONNX Runtime entra estático no `poph`; o modelo de embedding não
vai no pacote (instale com `poph model install`). O pacote ainda depende das bibliotecas compartilhadas do
sistema listadas em `LDD-poph.txt`.

### ARM64

O build ARM64 é feito numa máquina ARM64 nativa. `scripts/Dockerfile.build-arm` define uma imagem Ubuntu
24.04 (glibc 2.39) com `gcc` 13, para que o binário rode em hosts Ubuntu 24.04. A imagem oficial `rust` é
Debian e serve só para copiar a toolchain fixada.

Em x86_64, o CI faz `cargo check --target aarch64-unknown-linux-gnu` apenas dos crates sem código C
(`poph-types`, `poph-files`, `poph-plugins`, `poph-plugin-eco` e `poph-fluxos`). Os demais precisam de
compilador C para ARM e só são verificados num build ARM64 de verdade.

## TLS

Os clientes HTTP (provedor de modelo, Jev, download do modelo de embedding) usam `rustls` com as raízes de
confiança do sistema. OpenSSL é proibido no `deny.toml`. Falha de TLS é definitiva e não é repetida, e não
existe opção de ignorar certificado. Para testar com uma CA própria, instale-a no sistema.

## Cache de compilação

Desde o Cargo 1.100, cada forma diferente de comando (`clippy --all-targets`, `check`, outro alvo) gera um
conjunto novo de artefatos em `target/` e o Cargo nunca apaga os antigos, então o diretório cresce. O
workspace reduz o `debuginfo` do perfil `dev` (`debug = "line-tables-only"`) e o `scripts/ci.sh` começa
podando o cache.

```bash
bash scripts/limpar-cache.sh --seco     # mostra o que apagaria
bash scripts/limpar-cache.sh            # poda unidades sem uso há 3 dias; aperta a retenção acima de 25 GB
bash scripts/limpar-cache.sh --dias 7   # retenção maior
bash scripts/limpar-cache.sh --tudo     # cargo clean
```

A poda se recusa a rodar quando há `cargo` ou `rustc` em execução, para não quebrar um build em andamento.
`--forcar` ignora essa guarda e serve só para prova controlada.
