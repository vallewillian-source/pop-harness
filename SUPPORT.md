# Suporte

O POP Harness é um projeto de código aberto, mantido por uma equipe pequena. Este arquivo diz onde cada
tipo de pergunta deve ir para ser respondido mais rápido.

| Preciso de... | Vá para |
|---|---|
| Entender como algo funciona ou como configurar | Leia [`docs/`](docs/) e o [`README.md`](README.md). Os runbooks passo a passo estão em [`docs/RUNBOOKS.md`](docs/RUNBOOKS.md). |
| Tirar uma dúvida de uso, trocar ideia ou mostrar o que você montou | [GitHub Discussions](https://github.com/vallewillian-source/pop-harness/discussions) |
| Relatar um erro | [Nova issue](https://github.com/vallewillian-source/pop-harness/issues/new/choose), modelo de erro |
| Propor uma melhoria | [Nova issue](https://github.com/vallewillian-source/pop-harness/issues/new/choose), modelo de proposta |
| Relatar uma falha de segurança | Privado, pelo [`SECURITY.md`](SECURITY.md). **Não** use issue pública. |
| Contribuir com código ou documentação | [`CONTRIBUTING.md`](CONTRIBUTING.md) |

## Antes de abrir uma issue de erro

1. Rode `poph doctor` e veja se ele aponta o problema.
2. Procure nas issues abertas e fechadas.
3. Reproduza na versão mais recente (`main` ou a última versão publicada).
4. Junte: a versão (`poph --version`), o sistema operacional, o trecho **sem segredos** do `poph.toml`, o
   comando e a saída completa do erro.

**Nunca cole** chaves, tokens, o conteúdo de `POPH_CHAVE_CIFRA`, telefones ou conversas reais de clientes
numa issue.

## O que este projeto não oferece

Não há SLA, nem atendimento por e-mail ou telefone para a versão de código aberto. Respondemos issues e
discussões conforme a disponibilidade de quem mantém o projeto.

Empresas que precisam de implantação, integração com os sistemas delas, agentes sob medida ou suporte com
prazo podem contratar a [SofaCoding](https://sofacoding.com.br).
