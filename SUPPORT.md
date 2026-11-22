# Support

POP Harness is an open-source project maintained by a small team. This file says where each kind of
question should go to get answered faster.

| I need... | Go to |
|---|---|
| To understand how something works or how to configure it | Read [`docs/`](docs/) and the [`README.md`](README.md). Step-by-step runbooks are in [`docs/RUNBOOKS.md`](docs/RUNBOOKS.md). |
| To ask a usage question, exchange ideas or show what you built | [GitHub Discussions](https://github.com/vallewillian-source/pop-harness/discussions) |
| To report a bug | [New issue](https://github.com/vallewillian-source/pop-harness/issues/new/choose), bug template |
| To propose an improvement | [New issue](https://github.com/vallewillian-source/pop-harness/issues/new/choose), proposal template |
| To report a security flaw | Privately, through [`SECURITY.md`](SECURITY.md). Do **not** use a public issue. |
| To contribute code or documentation | [`CONTRIBUTING.md`](CONTRIBUTING.md) |

Questions and issues are welcome in English or Portuguese.

## Before opening a bug issue

1. Run `poph doctor` and see whether it points to the problem.
2. Search open and closed issues.
3. Reproduce on the latest version (`main` or the latest release).
4. Gather: the version (`poph --version`), the operating system, the **secret-free** part of `poph.toml`,
   the command and the full error output.

**Never paste** keys, tokens, the contents of `POPH_CHAVE_CIFRA`, phone numbers or real customer
conversations into an issue.

## What this project does not offer

There is no SLA, and no email or phone support for the open-source version. We answer issues and
discussions as maintainers have time.

Companies that need deployment, integration with their own systems, custom agents or support with a
deadline can hire [SofaCoding](https://sofacoding.com.br).
