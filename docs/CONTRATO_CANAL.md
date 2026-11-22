# Contrato do canal (integração) e eventos do turno

> Descreve o que existe no código hoje. A fonte dos tipos é
> `crates/poph-types/src/integracao.rs`; o protocolo de plugin em processo externo está em
> [`PLUGIN_PROTOCOLO.md`](PLUGIN_PROTOCOLO.md); o exemplo completo em [`PLUGIN_EXEMPLO.md`](PLUGIN_EXEMPLO.md).
> Se o código mudar, este documento muda no mesmo commit. A seção 7 lista o que **não existe** e seria
> preciso para um canal que observa a conversa (caso do Matrix); ela é levantamento, não decisão.

## 1. Papéis

Um **canal** é a ponta do cliente: de lá chegam as mensagens e para lá vai a resposta. Hoje há dois
tipos, com o mesmo contrato:

- **Nativo:** WhatsApp pela EvolutionAPI (`crates/poph-evolution`), módulo em Rust que implementa o
  trait `Integration`.
- **Processo externo:** qualquer programa que fale o protocolo JSON-lines sobre stdio, supervisionado
  por `poph-plugins`. O plugin `poph-plugin-eco` é o modelo.

Cada integração pertence a **um agente** (`integrations.agent_id`) e a uma org. A versão do protocolo é
`PROTOCOLO_VERSAO = 1`; diferença no handshake é recusada.

## 2. Manifesto

O canal se descreve no handshake (`Manifesto`):

| Campo | Para que serve |
|---|---|
| `nome`, `kind`, `versao`, `protocolo` | identificação; `kind` é o mesmo da tabela `integrations` |
| `capacidades` | `text`, `image_in`, `file_in`, `file_out`, `typing`, `read_receipt` (ausente = falso) |
| `limites` | `max_anexo_bytes` (padrão 16 MB), `max_texto_bytes` (0 = sem teto declarado), `tipos_aceitos` |
| `configuracao` | JSON Schema do formulário de configuração (o Console desenha a partir dele) |
| `segredos` | JSON Schema dos segredos que o operador preenche |

Segredos viajam só no handshake e nunca em evento, comando, erro ou log. O servidor não guarda
manifesto: pergunta ao plugin declarado em `[plugins].integracoes` e guarda o resultado em memória
(`PLUGIN_PROTOCOLO.md` §8.1).

## 3. Entrada: `InboundEvent`

O que o canal entrega ao Harness:

| Campo | Significado |
|---|---|
| `integracao` | integração que recebeu |
| `id_externo` | id da mensagem no canal; **chave de idempotência** (obrigatório) |
| `peer` | contato do outro lado: telefone, jid, sala (obrigatório) |
| `remetente` | quem escreveu, quando difere do `peer` (grupo) |
| `texto` | texto recebido; vazio quando é só anexo |
| `anexos` | lista de `AnexoEntrada`: `blob` (já gravado: nome, mime, sha256, tamanho) ou `bytes` (base64 no evento) |
| `ocorrido_em` | ms desde a época (obrigatório, positivo) |
| `metadados` | dados do canal sem campo próprio (id de resposta, menções) |

`InboundEvent::validar` recusa evento sem `id_externo`, sem `peer`, sem horário ou com anexo inválido
(sha256 que não tem 64 hexa, tamanho zero, sem nome ou mime).

**Nome do cliente (`push_name`, MX-32).** O plugin do WhatsApp põe o nome do contato em
`metadados["push_name"]` quando a mensagem é de contato individual recebido: não sai para mensagem
enviada por nós (`fromMe`, que traz o nome do dono da instância) nem para grupo (que traz o nome de quem
escreveu). Sem o campo, a conversa mantém o nome que já tinha. O campo não entra no `InboundEvent`
como campo próprio, então o contrato do canal não muda. Quem grava é `receber_do_canal`, dentro da
transação da mensagem (`DADOS.md`, migração `0008`), e não o `ConversaDoCanal`: o webhook do WhatsApp
não passa por ele.

**O que o Harness faz com ele** (`ENTRADA_E_SAIDA.md`): grava mensagem e item da `inbox` numa transação,
com chave única `(integration_id, external_message_id)`; duplicata devolve sucesso sem segundo turno; o
OK ao canal só sai depois do commit. O item da `inbox` guarda só `{conversa, mensagem}`
(`EventoEntrada`); o conteúdo está em `messages` e `attachments`. O item só é concluído quando o turno
termina, então queda no meio faz o despacho repetir (pelo menos uma vez).

## 4. Saída: `ComandoSaida`

O que o Harness manda o canal entregar. Todo comando traz uma chave de idempotência `turn_id:seq`
(menos a presença).

| Comando | Campos | Observação |
|---|---|---|
| `EnviarTexto` (`SendText`) | `idempotencia`, `peer`, `texto` | no WhatsApp, texto acima de 4000 caracteres é cortado com `…`, numa mensagem só |
| `EnviarArquivo` (`SendFile`) | `idempotencia`, `peer`, `nome`, `mime`, `tamanho_bytes`, `sha256`, `blob_ref`, `legenda?` | só arquivos da biblioteca do agente (D13); `blob_ref` é `<org>/<sha256>` |
| `DefinirPresenca` (`SetPresence`) | `peer`, `estado` (`digitando` ou `pausado`) | |

Estados de uma linha da `outbox`: `pending`, `sending` (gravado **antes** de chamar o canal),
`sent`, `failed`. Na retomada, o que ficou em `sending` é reenviado: é a única situação em que o cliente
pode receber duplicata (D11). Backoff padrão 500 ms × 2 até 60 s.

## 5. Estado, saúde e o trait

O canal avisa mudanças com `EventoEstado`: `conectado`, `desconectado`, `qr` (código e expiração) e
`erro` (código, mensagem, se vale retentar). A pergunta de saúde devolve `Saude { ok, detalhe }`.

O trait que os dois tipos de canal implementam:

```rust
pub trait Integration: Send + Sync + 'static {
    fn manifesto(&self) -> Option<Manifesto>;        // vazio antes do handshake
    fn estado(&self) -> EstadoIntegracao;            // Iniciando, Pronto, Reiniciando, Caida, Encerrada
    fn enviar_texto(&self, comando: SendText) -> FuturoComando;
    fn enviar_arquivo(&self, comando: SendFile) -> FuturoComando;
    fn definir_presenca(&self, comando: SetPresence) -> FuturoComando;
    fn saude(&self) -> FuturoSaude;
}
```

Quem recebe o que o canal produz implementa `Receptor::receber(EventoIntegracao)`, com
`EventoIntegracao::Entrada(InboundEvent)` ou `EventoIntegracao::Estado(EventoEstado)`. A chamada é
síncrona de propósito: quem recebe enfileira e volta. A ligação pronta entre o contrato e as filas está
em `poph-agent/src/fila_integracao.rs` (`InboxDoCanal`, `EnvioPeloCanal`).

Supervisão de plugin externo: teto de tamanho por linha, timeout por comando, reinício com backoff e
teto de falhas; plugin que trava, cai ou emite lixo não derruba o servidor nem outras integrações
(`PLUGIN_PROTOCOLO.md` §5).

## 6. Eventos do turno

`EventoTurno` (em `poph-agent/src/turno/mod.rs`) é o que o turno avisa a quem acompanha:
`Inicio`, `Texto` (pedaços da resposta), `Base` (fontes recuperadas), `Ferramenta` (nome e rodada),
`Transferencia` (departamento, tipo de risco, se saiu orientação) e `Fim` (estado e motivo).

- Eles são entregues a um `Observador` **dentro do processo**. Na integração (WhatsApp, plugin) o
  observador é nulo: a resposta sai pela `outbox`. No chat de teste do Console, os eventos viram SSE
  (`inicio`, `texto`, `base`, `ferramenta`, `transferencia`, `fim`; contrato em [`API.md`](API.md)).
- O que fica **gravado** e dá para consultar depois: `messages`, `turns` (decisões do Jev, tempos,
  tokens, ferramentas), `conversations.status` e `target_department_id`, `audit_log`. A API lê isso por
  `GET /admin/v1/orgs/{org}/conversations`, `…/{id}`, `…/{id}/messages`, `…/{id}/turns`, e reabre por
  `POST …/{id}/reopen`.

## 7. O que um canal observador (Matrix) exigiria e hoje não existe

> Nota: a ponte que faz o Element ler e fechar conversas do POP Harness pelo backend Python (repasse) não é um canal observador e não está descrita aqui. Ver [`REPASSE_PYTHON.md`](REPASSE_PYTHON.md).

Levantamento feito lendo o esquema e o código em 21/11/2026. Direção aprovada pelo mantenedor: módulo
desligável em crate separado, com poucas mudanças no núcleo, e um segundo papel de integração, o de
observador da conversa. **Nada disto está especificado nem implementado.**

1. **Não há saída de eventos para fora do processo.** `EventoTurno` só chega a observadores em memória.
   Para um módulo em crate separado receber cópia de cada mensagem e da transferência, falta um ponto de
   publicação (ou leitura incremental do que já é gravado).
2. **O contrato é de canal de cliente.** `Integration` serve a um peer por vez e pertence a um agente.
   Um observador acompanha todas as conversas de uma org (ou de um departamento) e não responde a um
   `peer`. Falta o tipo do papel e a regra de quais conversas ele recebe.
3. **Mensagem de atendente humano não tem onde entrar.** `messages.direction` aceita `in`/`out` e
   `messages.role` aceita `user`, `assistant`, `tool`, `system`: não há papel de atendente humano.
4. **Envio iniciado por humano não tem turno.** `outbox.turn_id` é obrigatório e a idempotência é
   `(turn_id, seq)`. Uma resposta digitada no Element chegaria sem turno e precisaria de outra chave.
5. **Pausar o agente quando o humano responde.** Hoje só a transferência (G1/G4) pausa, por
   `conversations.status = transferred` (D12). Humano que assume sem transferência não tem estado.
6. **Várias instâncias.** O encaminhamento entre instâncias hoje cobre `POST /webhook/evolution/{org_id}/{instancia}`
   e `/admin/v1/orgs/{org_id}/…`, porque a org está no caminho. Um webhook do Synapse só serviria ao
   mesmo mecanismo se também trouxesse a org no caminho. **Suposição**: não foi testado.
7. **Synapse fora do ar não pode atrasar o cliente.** A cópia ao Matrix teria de sair por fila própria,
   depois da resposta ao cliente. Hoje o único mecanismo de fila durável é a `outbox` do canal do cliente.
8. **Ligação com as Fichas** e com as salas por departamento não existe no Harness (Fichas estão fora).

Antes de escrever a especificação do Matrix, os pontos 1, 3, 4 e 5 são os que mexem no núcleo; os demais
ficam dentro do módulo novo.
