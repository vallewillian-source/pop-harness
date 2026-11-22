# Política de segurança

## Versões que recebem correção

O POP Harness está na série `0.x`. Até a `1.0`, só a versão mais recente publicada e o `main` recebem
correções de segurança.

| Versão | Recebe correção de segurança |
|---|---|
| `main` e a última `0.x` publicada | sim |
| versões `0.x` anteriores | não |

## Como relatar uma vulnerabilidade

**Não abra uma issue pública, um pull request nem uma discussão** para falar de uma falha de segurança.

Use o relato privado do GitHub:

1. Abra a aba **Security** do repositório.
2. Clique em **Report a vulnerability**.
3. Descreva o problema.

Inclua, se puder:

- o que acontece e qual o impacto (por exemplo: uma org lê dado de outra, vazamento de segredo, execução
  de código por um script de fluxo, negação de serviço);
- o passo a passo para reproduzir, ou uma prova de conceito;
- a versão ou o commit afetado, o sistema operacional e a configuração relevante;
- uma sugestão de correção, se tiver.

Não inclua dados reais de clientes, tokens ou chaves. Se a prova de conceito precisar deles, use valores
de teste.

## O que esperar

- Confirmamos o recebimento em até **5 dias úteis**.
- Avaliamos, reproduzimos e respondemos com o que concluímos e o que vamos fazer.
- Seguimos a divulgação coordenada: trabalhamos a correção com você e publicamos os detalhes em até
  **90 dias** do relato, ou antes, quando a correção sair. Se precisarmos de mais tempo, dizemos o motivo e
  combinamos uma nova data.
- Damos o crédito a quem relatou, se você quiser.

## O que consideramos falha de segurança neste projeto

Em especial, e por ordem de gravidade:

1. **Quebra do isolamento entre orgs**: uma org, token de org ou conversa alcançar dado de outra org.
2. **Vazamento de segredo**: chave, token ou segredo de integração aparecendo em log, resposta de API,
   mensagem de erro, backup ou arquivo.
3. **Escalada de privilégio**: token de org fazendo ação global; chamada sem token aceita.
4. **Execução ou abuso a partir de entrada não confiável**: script de fluxo, arquivo recebido (PDF e Office),
   mensagem de cliente tratada como instrução, resposta de plugin externo.
5. **Negação de serviço** que um cliente ou autor de fluxo consiga causar a outras orgs.

Erros comuns sem impacto de segurança (um valor de configuração mal documentado, um aviso de dependência
sem caminho de exploração) podem ir numa issue pública normal.

## Boas práticas para quem opera uma instância

- Guarde `POPH_CHAVE_CIFRA` num gerenciador de segredos. Quem a perde perde acesso aos segredos cifrados.
- O Console e a API administrativa não foram feitos para a internet aberta: ponha um proxy reverso com TLS
  na frente e restrinja quem alcança `/admin/v1`.
- Use token de org, e não o global, para tudo que não for administrar o servidor.
- Rode `poph doctor` depois de cada mudança de configuração.
- Mantenha o `cargo audit` e o `cargo deny` no seu pipeline se você compila a partir do código-fonte.
