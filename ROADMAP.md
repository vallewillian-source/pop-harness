# Roteiro

Este arquivo diz o que o projeto prioriza e o que está fora do escopo. Ele muda conforme as decisões de
quem mantém o projeto e não é uma promessa de data. Para propor algo, abra uma issue de proposta.

## Em desenvolvimento (estado em 22/11/2026)

Estes itens já têm desenho fechado e aparecem no README como parte do produto, mas ainda não estão no binário:

- [ ] **Fluxos:** `poph flow` (validar, publicar, diff, formatar), efeitos, revisões, a ferramenta `fluxo_iniciar`
      e o gate G9. A linguagem está em [`docs/FLUXOS_LINGUAGEM.md`](docs/FLUXOS_LINGUAGEM.md).
- [ ] **Fichas:** a Ficha já é criada em cada atendimento e fechada junto com ele; falta `poph ficha`, a
      listagem e o histórico de fluxos como subitem.
- [ ] **Matrix, fase 2:** imagem e arquivo nos dois sentidos, mensagem do aparelho e expiração.
- [ ] **Matrix, fase 3:** várias instâncias no mesmo Synapse.
- [ ] **Console:** telas de Matrix, fluxos e Fichas (hoje aparecem como "em breve" no menu).
- [ ] **Console fora do monorepo:** publicar ou incluir o pacote `@sofacoding/sofa-ui`.

## Antes da `1.0`

O que falta para o projeto ser recomendável em produção por qualquer pessoa:

- [ ] **Teste de carga completo.** 10 mil agentes ociosos e 500 conversas ativas, em x86_64 e ARM64, com a
      tabela "meta × medido" publicada. Hoje as metas estão fixadas ([`docs/DECISOES.md`](docs/DECISOES.md), D-07)
      e só há medições parciais.
- [ ] **Benchmark reproduzível** de latência e memória contra alternativas, com método e scripts públicos.
- [ ] **WhatsApp com número real.** Hoje o canal foi provado só com uma EvolutionAPI falsa.
- [ ] **Provedor de modelo genérico.** Trocar o identificador `antigravity2api` por um provedor
      compatível com OpenAI documentado e testado contra mais de um endpoint, e documentar como escrever
      outro provedor.
- [ ] **Decisão do Jev trocável.** Hoje o Jev é um serviço de terceiros opcional. Permitir outro serviço de
      decisão atrás da mesma interface, com o mesmo princípio (o serviço escolhe, o código valida).
- [ ] **Plugin externo como canal no servidor.** Ligar o contrato e o supervisor, que estão prontos, ao
      recebimento e ao envio de mensagens no `poph serve`.
- [ ] **Paridade de busca medida com um agente real**, não só com corpus sintético, e calibração do corte
      de relevância padrão.
- [ ] **Imagem de container** e pacote ARM64 reconstruído a cada versão.
- [ ] **Login de pessoa no Console**, papéis e limite de taxa por token.

## Depois

Ideias que cabem no projeto, sem ordem e sem data:

- Retenção própria das Fichas.
- Editor visual de fluxos no Console (o formato de script já foi pensado para ele editar sem perder nada).
- Transcrição de áudio e vídeo.
- Plugins em WASM.
- Novos canais como plugin: e-mail e widget de site.

## Fora do escopo

Coisas que decidimos **não** fazer, para ninguém gastar tempo nelas:

- Framework genérico de agentes. O Harness é um motor de atendimento, não uma biblioteca de orquestração.
- Redis, Postgres ou outro serviço obrigatório. A instalação continua sendo um binário e arquivos.
- Regras de rota por intenção no núcleo (a passagem é para um departamento).
- Compatibilidade com o JSON do editor de fluxos de outros produtos.
