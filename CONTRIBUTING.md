# Como contribuir com o POP Harness

Obrigado por querer ajudar. Este guia diz como preparar o ambiente, o que rodar antes de abrir um pull
request e o que esperamos de uma contribuição. Se algo aqui estiver confuso ou desatualizado, isso já é uma
contribuição útil: abra uma issue ou corrija o texto.

Ao participar, você aceita o [Código de Conduta](CODE_OF_CONDUCT.md).

> **Idioma.** O código, os comentários e a documentação do projeto são em português do Brasil. Issues e
> pull requests podem ser escritos em português ou em inglês. Mantenha o idioma do código ao qual você está
> contribuindo.

## Antes de começar

| Quero... | Faça isto |
|---|---|
| Relatar um erro | Abra uma [issue de erro](https://github.com/vallewillian-source/pop-harness/issues/new/choose) com o passo a passo para reproduzir. |
| Sugerir uma melhoria | Abra uma issue de proposta **antes** de escrever código, para alinhar o desenho. |
| Tirar uma dúvida de uso | Use as [Discussions](https://github.com/vallewillian-source/pop-harness/discussions). Veja o [`SUPPORT.md`](SUPPORT.md). |
| Reportar falha de segurança | **Não** abra issue pública. Siga o [`SECURITY.md`](SECURITY.md). |
| Corrigir algo pequeno (erro de digitação, teste, documentação) | Abra o pull request direto. |
| Escolher uma tarefa | Procure as issues com `good first issue` e `help wanted`. Comente que vai pegar, para ninguém duplicar o trabalho. |

Mudanças que alteram **contrato de API, formato de arquivo, esquema de banco ou o desenho de um módulo**
precisam de uma issue aprovada por quem mantém o projeto antes do pull request. É para evitar que você gaste
tempo numa direção que não vamos aceitar.

## Preparar o ambiente

Você precisa de Linux, um compilador C (`gcc` ou `clang`) e o [rustup](https://rustup.rs). A versão do Rust
(`1.100.0`) está fixada em `rust-toolchain.toml` e o `rustup` a instala sozinho na primeira vez.

```bash
git clone https://github.com/<seu-usuario>/pop-harness.git
cd pop-harness
cargo build --workspace
```

Ferramentas de verificação (uma vez):

```bash
cargo install cargo-deny cargo-audit --locked
```

Para mexer no Console (React e TypeScript) você também precisa de Node 22.18 ou mais novo e do pnpm. Veja
[`DEVELOPMENT.md`](DEVELOPMENT.md).

## O que rodar antes de abrir o pull request

O comando único é:

```bash
bash scripts/ci.sh
```

Ele roda, em ordem: poda do cache de compilação, build do Console e checagem do cliente da API (pulados se
`console/node_modules` não existir), `cargo fmt --check`, `cargo clippy` com `-D warnings`, build, testes,
checagem para ARM64 dos crates sem código C, `cargo deny` e `cargo audit`. No fim mostra `CI LOCAL: OK` ou
o número de checagens que falharam.

Para iterar mais rápido, rode só o que você mexeu:

```bash
cargo test --locked -p poph-retention          # um crate
cargo fmt --all                                 # formatar
cargo clippy --workspace --all-targets -- -D warnings
```

Dois testes de contrato dependem de arquivos de fora deste repositório (o frontend do Element) e ficam
ignorados por padrão. Quem tem esses arquivos roda `cargo test -p poph-matrix --test contrato_frontend --
--ignored` com `POPH_ELEMENT_CONTRATO` apontando para o `atendimentoRoom.ts`.

## Regras do projeto

Estas regras vêm de decisões já tomadas e estão explicadas em [`AGENTS.md`](AGENTS.md) e
[`docs/DECISOES.md`](docs/DECISOES.md).

- **Segredo nunca sai.** Chaves vêm só do ambiente e nunca vão para arquivo, log, mensagem de erro,
  resposta de API ou teste com valor real. Use o tipo `Sensivel` para qualquer valor sensível que passe por
  log ou erro, e teste com um valor-canário.
- **Isolamento entre orgs.** Toda tabela de negócio tem `org_id` e todo acesso filtra por ele. Código novo
  que lê ou grava dado de org precisa de teste que prove que a org A não alcança a org B.
- **Camadas.** `poph-types` não depende de crate interno; crates de capacidade não importam `poph-server`
  nem `poph`; `poph-agent` não importa `poph-matrix`. A tabela está em `AGENTS.md`. O Cargo impede ciclos,
  mas não a direção das camadas: confira você mesmo ao adicionar uma dependência.
- **Migrações de banco.** Crie um arquivo novo em `crates/poph-store/migrations/harness/NNNN_nome.sql` e
  registre em `crates/poph-store/src/migracoes.rs` com o próximo número. **Nunca edite uma migração já
  publicada.** Mudanças são aditivas.
- **Contrato da API.** Não edite `console/src/api/schema.d.ts` à mão: ele sai de `poph openapi`. Depois de
  mudar uma rota, rode `bash console/gen-api.sh`. O CI falha se o arquivo divergir do servidor.
- **Sem `unsafe`.** O workspace usa `unsafe_code = "forbid"`.
- **Dependências novas.** Cada crate novo precisa de justificativa no pull request e passar no `cargo deny`
  (licenças permissivas, sem OpenSSL, só o registro do crates.io). Menos dependência é melhor.
- **O Jev escolhe, o código valida.** Falha do Jev não pode virar efeito. Veja `docs/JEV.md`.
- **Documentação junto com o código.** Se o seu pull request muda um comportamento, atualize o documento do
  mesmo assunto em `docs/` no mesmo commit.
- **Console.** Sem cartão dentro de cartão, estados de carregando, vazio e erro em toda tela, e botão de
  ação travado enquanto a requisição roda. Veja `console/UX_SPEC.md`.

## Estilo de código

- Rust: `cargo fmt` e `clippy -D warnings`. Erros tipados com `thiserror`; sem `unwrap` ou `expect` em
  caminho de produção fora de invariantes que o comentário explica.
- Nomes de domínio em português (`conversa`, `turno`, `org`), como no resto do código.
- Comentários explicam o **porquê**, não o quê. Comentário que repete o código sai.
- Testes ao lado do código que eles cobrem. Teste novo deve falhar sem a sua mudança.

## Mensagens de commit

Usamos [Conventional Commits](https://www.conventionalcommits.org/pt-br/v1.0.0/), com o crate ou a área no
escopo:

```
feat(poph-store): filtro por cliente na listagem de conversas
fix(poph-evolution): ignora evento de mensagem sem id externo
docs(cluster): explica a recarga do mapa sem reiniciar
```

Prefira commits pequenos e que compilam. Um pull request deve fazer uma coisa.

## Abrir o pull request

1. Faça um fork e crie uma branch a partir de `main` (`feat/nome-curto` ou `fix/nome-curto`).
2. Rode `bash scripts/ci.sh` e confirme que passou.
3. Abra o pull request preenchendo o modelo. Diga **o que muda na prática**, **como você testou** e o que
   **não** conferiu.
4. Responda às revisões na própria conversa. Quem mantém o projeto pode pedir mudanças ou explicar por que
   não vai aceitar; isso não é pessoal.

Respondemos conforme a disponibilidade de quem mantém o projeto. Se o pull request ficar parado por muito
tempo, pode cobrar na própria conversa.

### Origem e licença da contribuição

O projeto é distribuído sob a [AGPL-3.0-or-later](LICENSE). Ao enviar uma contribuição você declara que
tem o direito de enviá-la e que ela entra sob essa licença. Para isso usamos o
[Developer Certificate of Origin](https://developercertificate.org/): assine cada commit com
`git commit -s`, que acrescenta a linha `Signed-off-by: Seu Nome <voce@exemplo.com>`.

## Contribuições feitas com ajuda de IA

Aceitamos. Você continua responsável pelo que envia: leia, rode e entenda o que a ferramenta escreveu,
confira que ela não copiou código de licença incompatível e não inclua nada que você não possa
relicenciar sob a AGPL. O [`AGENTS.md`](AGENTS.md) descreve o mapa do código e as regras de trabalho para
agentes e pessoas.

## Reconhecimento

Quem contribui aparece no histórico do git e nas notas de cada versão em [`CHANGELOG.md`](CHANGELOG.md).
