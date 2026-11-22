## O que muda

<!-- Em linguagem comum: o que fica diferente para quem usa. Uma ideia por pull request. -->

## Por que

<!-- O problema que isso resolve. Link para a issue: "Fecha #123". -->

## Como foi testado

<!-- Comandos que você rodou e o resultado. O que você NÃO conseguiu conferir? -->

## Tipo de mudança

- [ ] Correção de erro
- [ ] Recurso novo
- [ ] Documentação ou teste
- [ ] Refatoração sem mudança de comportamento
- [ ] Mudança incompatível (quebra API, formato de arquivo, configuração ou esquema de banco)

## Lista de conferência

- [ ] `bash scripts/ci.sh` passou (ou expliquei acima o que não passou e por quê).
- [ ] Atualizei o documento do assunto em `docs/` e, se for o caso, o `CHANGELOG.md`.
- [ ] Mudança de esquema de banco é uma **migração nova** (não editei uma já publicada).
- [ ] Mudança de rota regenerou o cliente da API (`bash console/gen-api.sh`).
- [ ] Código que toca dado de org tem teste de isolamento entre orgs.
- [ ] Nenhum segredo, token, telefone ou dado de cliente em código, teste, log ou captura de tela.
- [ ] Os commits estão assinados (`git commit -s`, Developer Certificate of Origin).
