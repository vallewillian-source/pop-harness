//! Provisão de sala por conversa (MX-11).
//!
//! Uma conversa tem **uma** sala no Synapse, criada de forma idempotente e recuperável. A trava é o
//! alias determinístico `#<prefixo>_c_<conversa>:<servidor>` (P6): se o processo cair entre o
//! `createRoom` e a gravação no banco, a tentativa seguinte bate em `M_ROOM_IN_USE`, resolve o alias
//! e continua na **mesma** sala. O mesmo mecanismo resolve duas chamadas concorrentes para a mesma
//! conversa.
//!
//! ## Por que a persistência entra por porta
//!
//! `poph-matrix` fala com o Synapse e não conhece o banco (`AGENTS.md`): a decisão de criar a sala
//! mora aqui, e o armazenamento entra pela porta [`PortaSalas`], implementada por quem junta as
//! peças (`poph-server`, sobre o repositório `store.matrix()` da MX-08). A porta é também o que
//! permite injetar a queda no teste — o banco "cai" no ponto exato e a retomada tem de achar a sala
//! pelo alias.
//!
//! ## O que a sala leva
//!
//! - `creation_content.type` = [`TIPO_SALA`] e o evento de estado [`TIPO_METADATA`] com os campos
//!   que o Element lê — os mesmos de `apps/frontend/src/sofacoding/atendimentoRoom.ts`, conferidos
//!   pelo teste de contrato `tests/contrato_frontend.rs`.
//! - Nome = nome do cliente; tópico curto com o canal.
//! - O fantasma do agente com o poder de `matrix.poder_agente` e o do cliente convidado e dentro da
//!   sala. Registrar e nomear os fantasmas é da MX-10.
//! - O aviso fixo [`AVISO_SEM_HISTORICO`] quando a conversa já tinha mensagem antes do corte do
//!   backfill (P8).
//!
//! ## O que **não** cabe aqui
//!
//! Espaço por departamento (MX-12), laço de espelhamento (MX-14) e as rotas HTTP (MX-22). A linha
//! nasce sem espaço: quem põe a sala no espaço do departamento é a MX-12.

use std::fmt;
use std::future::Future;

use poph_types::{AgentId, ConversationId, DepartmentId, MatrixConfig, OrgId};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cliente::{ClienteMatrix, CriarSala};
use crate::erro::{Classificacao, ErroChamada};
use crate::usuarios::{ErroFantasmas, Fantasma, NOME_CLIENTE_PADRAO, UsuariosVirtuais};

/// Tipo da sala de atendimento — o mesmo que o Element procura para tratar a sala como conversa.
pub const TIPO_SALA: &str = "com.sofacoding.atendimento";

/// Evento de estado com os dados da conversa (o Element lê o evento sem `state_key`).
pub const TIPO_METADATA: &str = "com.sofacoding.atendimento.metadata";

/// Evento de estado com o nome da sala (`m.room.name`). A MX-33 o reescreve quando o nome do
/// cliente chega depois da criação.
pub const TIPO_NOME: &str = "m.room.name";

/// Aviso fixo na sala quando a conversa já tinha mensagens antes de o módulo ser ligado (P8).
pub const AVISO_SEM_HISTORICO: &str = "histórico anterior não foi espelhado";

/// Marca da sala da conversa no alias (`#<prefixo>_c_<conversa>:<servidor>`).
pub const MARCA_SALA: &str = "c";

/// Poder do fantasma do agente **na criação** da sala.
///
/// O Synapse exige poder 100 do criador para escrever o próprio `m.room.power_levels`, e é isso que
/// o `createRoom` faz quando o corpo traz `power_level_content_override` com `users`. Com o criador
/// em 50 o Synapse responde `403 M_FORBIDDEN` (`user_level (50) < send_level (100)`) e **cria a sala
/// pela metade**: sem `m.room.name`, sem `m.room.topic` e sem o metadata. Defeito comprovado contra
/// o Synapse de desenvolvimento no aceite da MX-31.
///
/// Por isso a criação usa o poder máximo e, terminada a sala, o poder do agente desce para
/// `matrix.poder_agente` (respeitando [`PODER_MINIMO_AGENTE`]) numa escrita de `m.room.power_levels`
/// feita pelo próprio criador.
pub const PODER_CRIACAO: u8 = 100;

/// Piso do poder do fantasma do agente **depois** da criação.
///
/// Com a sala já criada, o que o agente escreve nela é o metadata (transferência/reabertura/
/// fechamento, MX-15) e o nome da sala quando o nome do cliente chega depois (MX-33) — os dois
/// exigem o poder padrão de estado do Matrix (50). `matrix.poder_agente` aceita de 0 a 100, então um
/// valor menor que 50 deixaria o módulo sem poder de manter a sala em dia; o módulo usa 50 como
/// piso em vez de aceitar um poder que o deixa mudo na sala.
pub const PODER_MINIMO_AGENTE: u8 = 50;

/// Evento de estado com os poderes da sala (`m.room.power_levels`).
const TIPO_PODERES: &str = "m.room.power_levels";

/// Tipos de integração que o frontend reconhece em `AtendimentoMetadata.integration_type`.
///
/// O canal do Harness (`whatsapp`, `console`, `cli`) só vira `integration_type` quando é um destes;
/// para os outros o campo não vai no metadata, em vez de inventar valor.
pub const INTEGRACOES_DO_FRONTEND: [&str; 4] = ["whatsapp", "site", "api", "email"];

/// Quantas vezes a criação tenta de novo quando o alias aparece em uso e some do diretório (corrida
/// entre duas instâncias criando a mesma sala).
const TENTATIVAS_ALIAS: u8 = 3;

/// `errcode` do Synapse quando o alias da sala já existe (comprovado na MX-02).
const ERRCODE_ALIAS_EM_USO: &str = "M_ROOM_IN_USE";

/// `errcode` do Synapse quando o alias não está no diretório.
const ERRCODE_NAO_ENCONTRADO: &str = "M_NOT_FOUND";

// ───────────────────────────── entrada ─────────────────────────────

/// O que a provisão precisa saber de uma conversa.
///
/// Quem monta é quem lê o banco (`poph-server`): a conversa, o nome que o canal deu ao cliente, o
/// agente e o departamento. Nada aqui é texto de cliente guardado pelo módulo — a struct viaja em
/// memória e só o que o Element precisa vira evento de estado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversaParaSala {
    /// Org dona da conversa.
    pub org: OrgId,
    /// Conversa a espelhar.
    pub conversa: ConversationId,
    /// Identificador do cliente no canal (o telefone, no WhatsApp).
    pub cliente: String,
    /// Nome do cliente como o canal informou. O Harness ainda não guarda esse nome: quando não
    /// houver, o fantasma cai no padrão da MX-10 e o nome da sala cai no identificador.
    pub nome_cliente: Option<String>,
    /// Agente que atende a conversa.
    pub agente: AgentId,
    /// Nome do agente; vazio cai no id.
    pub nome_agente: Option<String>,
    /// Canal da conversa (`whatsapp`, `console`, `cli`…).
    pub canal: String,
    /// Departamento alvo, quando houver (o espaço do departamento é da MX-12).
    pub departamento: Option<DepartmentId>,
    /// Conversa já encerrada: o metadata nasce com `status = closed`.
    pub fechada: bool,
    /// Instante (ms) em que a org ligou o módulo pela última vez — o corte do backfill (P8).
    /// `None` quando a org nunca ligou (e então não há aviso de histórico a dar).
    pub corte_backfill: Option<i64>,
}

impl ConversaParaSala {
    /// Nome visível da sala: o nome do cliente quando existe, senão o identificador dele.
    ///
    /// O identificador entra como último recurso de propósito: sem ele, todas as salas de clientes
    /// sem nome apareceriam iguais no Element e o atendente não distinguiria as conversas.
    pub fn nome_da_sala(&self) -> String {
        if let Some(nome) = self
            .nome_cliente
            .as_deref()
            .map(str::trim)
            .filter(|nome| !nome.is_empty())
        {
            return nome.to_owned();
        }
        let identificador = self.cliente.trim();
        if identificador.is_empty() {
            return NOME_CLIENTE_PADRAO.to_owned();
        }
        identificador.to_owned()
    }
}

/// Conteúdo do evento [`TIPO_METADATA`] — exatamente os campos que o Element lê.
///
/// Campo sem valor no Harness não vai no JSON (é o caso de `integration_type` para um canal que o
/// frontend não conhece): o frontend trata cada campo como opcional, e mandar `null` ou `""` seria
/// inventar um valor que não existe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetadataSala {
    /// Conversa que a sala espelha.
    pub conversation_id: String,
    /// Agente que atende.
    pub agent_id: String,
    /// Tipo de integração, quando o canal é um dos que o frontend conhece.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integration_type: Option<String>,
    /// Canal da conversa, como o Harness o chama.
    pub channel: String,
    /// Identificador do cliente no canal.
    pub client_identifier: String,
    /// Nome do cliente, quando conhecido.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    /// Departamento alvo, quando houver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department_id: Option<String>,
    /// `active` ou `closed`, como o frontend espera.
    pub status: String,
}

impl MetadataSala {
    /// Monta o metadata a partir da conversa.
    pub fn da_conversa(dados: &ConversaParaSala) -> Self {
        let canal = dados.canal.trim().to_owned();
        let integracao = INTEGRACOES_DO_FRONTEND
            .iter()
            .find(|conhecido| canal.eq_ignore_ascii_case(conhecido))
            .map(|conhecido| (*conhecido).to_owned());
        Self {
            conversation_id: dados.conversa.to_string(),
            agent_id: dados.agente.to_string(),
            integration_type: integracao,
            channel: canal,
            client_identifier: dados.cliente.clone(),
            client_name: dados
                .nome_cliente
                .as_deref()
                .map(str::trim)
                .filter(|nome| !nome.is_empty())
                .map(str::to_owned),
            department_id: dados
                .departamento
                .map(|departamento| departamento.to_string()),
            status: if dados.fechada { "closed" } else { "active" }.to_owned(),
        }
    }
}

// ───────────────────────────── saída ─────────────────────────────

/// O que a provisão devolve quando a sala está pronta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalaPronta {
    /// Id da sala no Synapse.
    pub room_id: String,
    /// `true` quando a linha do banco já estava `ready`: nada foi escrito no Synapse.
    pub ja_estava_pronta: bool,
    /// `true` quando o aviso de histórico não espelhado foi enviado nesta passagem.
    pub aviso_enviado: bool,
}

// ───────────────────────────── porta de persistência ─────────────────────────────

/// Situação da linha da sala em `matrix_rooms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstadoGravado {
    /// Sala criada no Synapse, criação ainda não terminada.
    Criando,
    /// Sala criada e completa (metadata, participantes e aviso, quando for o caso).
    Pronta,
}

/// A linha da sala, como a porta devolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalaGravada {
    /// Id da sala no Synapse.
    pub room_id: String,
    /// Situação da criação.
    pub estado: EstadoGravado,
}

/// Falha do armazenamento do módulo, sem texto de cliente, telefone ou token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("armazenamento do módulo Matrix falhou: {motivo}")]
pub struct ErroPorta {
    /// Motivo curto, para log. Quem implementa a porta não põe valor de cliente aqui.
    pub motivo: String,
}

impl ErroPorta {
    /// Monta a falha com um motivo curto.
    pub fn nova(motivo: impl Into<String>) -> Self {
        Self {
            motivo: motivo.into(),
        }
    }
}

/// Persistência que a provisão usa: a linha da sala em `matrix_rooms` (MX-08).
///
/// Quem implementa (o servidor, sobre `store.matrix()`) precisa garantir:
///
/// - [`PortaSalas::garantir_sala`] é **idempotente por conversa**: a segunda chamada devolve a linha
///   que já existe, e o `room_id` da primeira ganha. Sala ligada a **outra** conversa é conflito.
/// - [`PortaSalas::marcar_pronta`] leva `creating` → `ready` gravando o nome do cliente que a sala
///   recebeu (ou `None`, quando ela nasceu com o identificador) e repetir não muda nada.
/// - [`PortaSalas::teve_mensagens_antes`] responde se a conversa tem mensagem anterior ao instante
///   do corte; mensagem que não existe ainda não conta.
///
/// A porta é `async` porque o banco do Harness é entregue por um escritor único; nos testes ela é
/// um mapa em memória.
pub trait PortaSalas {
    /// Linha da sala da conversa (`None` quando ainda não existe).
    ///
    /// A assinatura é `-> impl Future + Send` (e não `async fn`) para o futuro prometido carregar
    /// `Send`: quem liga o módulo sobe a provisão numa tarefa própria.
    fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<SalaGravada>, ErroPorta>> + Send;

    /// Grava a sala em `creating` e devolve a linha gravada (a que já existia, quando houver).
    fn garantir_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        room_id: &str,
    ) -> impl Future<Output = Result<SalaGravada, ErroPorta>> + Send;

    /// Passa a sala de `creating` para `ready` e grava o nome do cliente usado na criação.
    ///
    /// `nome_espelhado` é `None` quando a sala nasceu com o identificador do cliente. É o que o
    /// laço de estado compara depois para decidir se renomeia a sala quando o nome chegar (MX-33).
    fn marcar_pronta(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome_espelhado: Option<&str>,
    ) -> impl Future<Output = Result<SalaGravada, ErroPorta>> + Send;

    /// A conversa já tinha mensagem antes de `corte` (ms)?
    fn teve_mensagens_antes(
        &self,
        org: OrgId,
        conversa: ConversationId,
        corte: i64,
    ) -> impl Future<Output = Result<bool, ErroPorta>> + Send;
}

// ───────────────────────────── erro ─────────────────────────────

/// Falha ao provisionar a sala de uma conversa.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErroSala {
    /// O fantasma do cliente ou do agente não ficou pronto (MX-10).
    #[error(transparent)]
    Fantasmas(#[from] ErroFantasmas),
    /// O Synapse recusou ou não respondeu uma chamada.
    #[error(transparent)]
    Chamada(#[from] ErroChamada),
    /// O armazenamento do módulo falhou.
    #[error(transparent)]
    Armazenamento(#[from] ErroPorta),
    /// O alias ficou em disputa: apareceu em uso e sumiu do diretório em todas as tentativas.
    #[error("o alias `{alias}` apareceu em uso e sumiu do diretório em {tentativas} tentativas")]
    AliasEmDisputa {
        /// Alias completo (`#…:servidor`).
        alias: String,
        /// Quantas tentativas foram feitas.
        tentativas: u8,
    },
    /// Falha que é defeito deste módulo, não do ambiente.
    #[error("falha interna do módulo Matrix: {motivo}")]
    Interno {
        /// Motivo curto.
        motivo: String,
    },
}

impl ErroSala {
    /// Como o laço deve tratar a falha. Não repete nada aqui: só informa.
    pub fn classificacao(&self) -> Classificacao {
        match self {
            Self::Chamada(erro) => erro.classificacao(),
            Self::Fantasmas(ErroFantasmas::Chamada(erro)) => erro.classificacao(),
            Self::Fantasmas(_) => Classificacao::Definitiva,
            // Banco indisponível ou escritor ocupado passa; é o mesmo tratamento de uma rede que
            // caiu: a sala continua pendente e o cursor não anda.
            Self::Armazenamento(_) => Classificacao::Transitoria,
            Self::AliasEmDisputa { .. } => Classificacao::Transitoria,
            Self::Interno { .. } => Classificacao::Definitiva,
        }
    }
}

// ───────────────────────────── ids da sala ─────────────────────────────

/// `localpart` do alias da sala: `<prefixo>_c_<conversa sem hífen>`.
///
/// O id da conversa é UUID v7 (só hexadecimal e hífen), e o hífen sai porque a `localpart` do alias
/// no Matrix é mais estreita que a do usuário. Tirar o hífen é injetivo no UUID: conversas
/// diferentes nunca caem no mesmo alias.
pub fn localpart_do_alias(prefixo: &str, conversa: ConversationId) -> String {
    let corpo: String = conversa
        .to_string()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|caractere| caractere.to_ascii_lowercase())
        .collect();
    format!("{prefixo}_{MARCA_SALA}_{corpo}")
}

/// Alias completo da sala da conversa (`#<prefixo>_c_<conversa>:<servidor>`).
pub fn alias_da_conversa(prefixo: &str, server_name: &str, conversa: ConversationId) -> String {
    format!("#{}:{}", localpart_do_alias(prefixo, conversa), server_name)
}

/// Tópico curto da sala.
fn topico_da_sala(canal: &str) -> String {
    let canal = canal.trim();
    if canal.is_empty() {
        "Atendimento".to_owned()
    } else {
        format!("Atendimento · {canal}")
    }
}

// ───────────────────────────── o provisionador ─────────────────────────────

/// Cria (uma única vez) a sala de cada conversa e mantém a linha de `matrix_rooms` em dia.
///
/// Quem chama já sabe que a org está ligada: a leitura de `matrix_orgs` e o corte do backfill são do
/// laço (MX-12), que entrega o corte em [`ConversaParaSala::corte_backfill`].
pub struct Provisionador<P: PortaSalas> {
    cliente: ClienteMatrix,
    usuarios: UsuariosVirtuais,
    porta: P,
    poder_agente: u8,
    prefixo: String,
    server_name: String,
}

impl<P: PortaSalas> Provisionador<P> {
    /// Monta com a seção `[matrix]` já validada: os ids dos fantasmas e o poder do agente saem dela.
    pub fn da_config(
        cliente: ClienteMatrix,
        porta: P,
        config: &MatrixConfig,
    ) -> Result<Self, ErroFantasmas> {
        let usuarios = UsuariosVirtuais::da_config(cliente.clone(), config)?;
        Ok(Self::com(cliente, usuarios, porta, config.poder_agente))
    }

    /// Monta com as peças já prontas (o teto do cache de fantasmas vem de fora).
    pub fn com(
        cliente: ClienteMatrix,
        usuarios: UsuariosVirtuais,
        porta: P,
        poder_agente: u8,
    ) -> Self {
        Self {
            prefixo: usuarios.ids().prefixo().to_owned(),
            server_name: cliente.server_name().to_owned(),
            cliente,
            usuarios,
            porta,
            poder_agente,
        }
    }

    /// Fantasma do agente da conversa, pronto (registro e nome pela MX-10).
    ///
    /// Quem põe a sala no espaço do departamento logo depois da criação (MX-12) precisa do fantasma
    /// do agente para escrever `m.space.parent` e as regras de entrada na sala.
    pub async fn fantasma_do_agente(&self, dados: &ConversaParaSala) -> Result<Fantasma, ErroSala> {
        Ok(self
            .usuarios
            .garantir_agente(
                &dados.agente.to_string(),
                dados.nome_agente.as_deref().unwrap_or_default(),
            )
            .await?)
    }

    /// Põe o usuário do aparelho dentro da sala antes de ele falar nela (MX-42).
    ///
    /// Sala criada antes do aparelho existir não o tem como participante, e o Synapse recusa evento
    /// de quem não está na sala. Por isso confere `joined_members`: se ele já está, não faz nada; se
    /// não está, o agente convida e o próprio aparelho entra. Repetir a passagem é seguro.
    pub async fn garantir_aparelho_na_sala(
        &self,
        dados: &ConversaParaSala,
        room_id: &str,
        aparelho: &Fantasma,
    ) -> Result<(), ErroSala> {
        let agente = self.fantasma_do_agente(dados).await?;
        let aparelho = self.usuarios.garantir_aparelho(aparelho.clone()).await?;
        let membros = self.cliente.membros(agente.user_id(), room_id).await?;
        if membros.contains_key(aparelho.user_id()) {
            return Ok(());
        }
        self.cliente
            .convidar(agente.user_id(), room_id, aparelho.user_id())
            .await?;
        self.cliente.entrar(aparelho.user_id(), room_id).await?;
        Ok(())
    }

    /// Alias que a sala desta conversa usa (o mesmo em qualquer instância com o mesmo prefixo).
    pub fn alias(&self, conversa: ConversationId) -> String {
        alias_da_conversa(&self.prefixo, &self.server_name, conversa)
    }

    /// Garante a sala da conversa e devolve o id dela.
    ///
    /// Repetir com a linha `ready` não fala com o Synapse. Com a linha `creating` (queda no meio),
    /// termina a criação na sala que já existe. Sem linha, cria pelo alias — e uma criação anterior
    /// que não chegou ao banco é encontrada pelo alias, não duplicada.
    pub async fn provisionar(&self, dados: &ConversaParaSala) -> Result<SalaPronta, ErroSala> {
        if let Some(gravada) = self.porta.sala(dados.org, dados.conversa).await? {
            if gravada.estado == EstadoGravado::Pronta {
                return Ok(SalaPronta {
                    room_id: gravada.room_id,
                    ja_estava_pronta: true,
                    aviso_enviado: false,
                });
            }
            return self.finalizar(dados, &gravada.room_id).await;
        }

        // Os fantasmas nascem antes da sala: o agente a cria já com o nome certo e o cliente é
        // convidado com o nome de exibição já aplicado.
        let (_, agente) = self.fantasmas(dados).await?;
        let sala_no_synapse = self.sala_no_synapse(dados, &agente).await?;

        // A linha é a verdade: se outra tarefa gravou primeiro, vale o `room_id` que está lá.
        let gravada = self
            .porta
            .garantir_sala(dados.org, dados.conversa, &sala_no_synapse)
            .await?;
        self.finalizar(dados, &gravada.room_id).await
    }

    /// Deixa a sala completa: participantes dentro, metadata do frontend e aviso de histórico.
    async fn finalizar(
        &self,
        dados: &ConversaParaSala,
        room_id: &str,
    ) -> Result<SalaPronta, ErroSala> {
        let (cliente, agente) = self.fantasmas(dados).await?;

        // O agente entra (idempotente). Na criação ele é o dono da sala; na recuperação, este
        // `join` é o que garante que ele está lá mesmo que a criação tenha sido de outra tentativa.
        self.cliente.entrar(agente.user_id(), room_id).await?;

        // O cliente entra. O convite só sai quando ele ainda não está na sala: repetir a passagem
        // não enche a sala de convites.
        let membros = self.cliente.membros(agente.user_id(), room_id).await?;
        if !membros.contains_key(cliente.user_id()) {
            self.cliente
                .convidar(agente.user_id(), room_id, cliente.user_id())
                .await?;
            self.cliente.entrar(cliente.user_id(), room_id).await?;
        }

        let metadata_sala = MetadataSala::da_conversa(dados);
        let metadata = serde_json::to_value(&metadata_sala).map_err(|erro| ErroSala::Interno {
            motivo: format!("o metadata da sala não serializou: {erro}"),
        })?;
        self.cliente
            .enviar_estado(agente.user_id(), room_id, TIPO_METADATA, None, &metadata)
            .await?;

        let aviso_enviado = self
            .avisar_historico(dados, room_id, agente.user_id())
            .await?;

        // A sala já está completa: agora o poder do agente desce de 100 (o da criação) para o
        // configurado. Falha aqui não desfaz nada — a sala continua utilizável com o poder da
        // criação, que é maior — e fica só no log.
        self.ajustar_poder_final(room_id, agente.user_id()).await;

        // O nome que a sala de fato recebeu (o `client_name` do metadata) fica gravado junto com o
        // `ready`: é o que a MX-33 compara para saber se precisa renomear a sala depois.
        let gravada = self
            .porta
            .marcar_pronta(
                dados.org,
                dados.conversa,
                metadata_sala.client_name.as_deref(),
            )
            .await?;
        Ok(SalaPronta {
            room_id: gravada.room_id,
            ja_estava_pronta: false,
            aviso_enviado,
        })
    }

    /// Aviso de que o histórico anterior não foi espelhado (P8), só quando havia mensagem antes do
    /// corte.
    async fn avisar_historico(
        &self,
        dados: &ConversaParaSala,
        room_id: &str,
        remetente: &str,
    ) -> Result<bool, ErroSala> {
        let Some(corte) = dados.corte_backfill else {
            return Ok(false);
        };
        if !self
            .porta
            .teve_mensagens_antes(dados.org, dados.conversa, corte)
            .await?
        {
            return Ok(false);
        }
        // `txnId` derivado da conversa: repetir a passagem não repete o aviso.
        let txn = format!(
            "ph_hist_{}",
            localpart_do_alias(&self.prefixo, dados.conversa)
        );
        self.cliente
            .enviar_evento(
                remetente,
                room_id,
                "m.room.message",
                &txn,
                &json!({
                    "msgtype": "m.notice",
                    "body": AVISO_SEM_HISTORICO,
                }),
            )
            .await?;
        Ok(true)
    }

    /// Fantasmas do cliente e do agente, prontos (registro e nome pela MX-10).
    async fn fantasmas(&self, dados: &ConversaParaSala) -> Result<(Fantasma, Fantasma), ErroSala> {
        let cliente = self
            .usuarios
            .garantir_cliente(
                &dados.cliente,
                dados.nome_cliente.as_deref().unwrap_or_default(),
            )
            .await?;
        let agente = self
            .usuarios
            .garantir_agente(
                &dados.agente.to_string(),
                dados.nome_agente.as_deref().unwrap_or_default(),
            )
            .await?;
        Ok((cliente, agente))
    }

    /// Poder que o fantasma do agente deve ter na sala **depois** da criação.
    fn poder_final(&self) -> u8 {
        self.poder_agente.max(PODER_MINIMO_AGENTE)
    }

    /// Rebaixa o poder do criador de [`PODER_CRIACAO`] para [`Self::poder_final`].
    ///
    /// Lê o `m.room.power_levels` atual e troca **só** o poder do agente, preservando os demais
    /// campos (escrever o evento inteiro do zero apagaria as regras de evento da sala). Já no valor
    /// final, não escreve nada. Qualquer falha fica no log com código curto e a sala segue como
    /// está: o poder da criação é maior que o final, então nada deixa de funcionar.
    async fn ajustar_poder_final(&self, room_id: &str, agente: &str) {
        let final_ = self.poder_final();
        if final_ >= PODER_CRIACAO {
            return;
        }
        let mut poderes = match self
            .cliente
            .ler_estado(agente, room_id, TIPO_PODERES, None)
            .await
        {
            Ok(poderes) => poderes,
            Err(_) => {
                tracing::warn!(
                    target: "poph_matrix::salas",
                    motivo = "poderes_da_sala_ilegiveis",
                    "não consegui ler os poderes da sala; o agente continua com o poder da criação"
                );
                return;
            }
        };
        let atual = poderes
            .get("users")
            .and_then(|usuarios| usuarios.get(agente))
            .and_then(Value::as_i64);
        if atual == Some(i64::from(final_)) {
            return;
        }
        let Some(usuarios) = poderes.get_mut("users").and_then(Value::as_object_mut) else {
            tracing::warn!(
                target: "poph_matrix::salas",
                motivo = "poderes_da_sala_sem_usuarios",
                "os poderes da sala não têm `users`; o agente continua com o poder da criação"
            );
            return;
        };
        usuarios.insert(agente.to_owned(), json!(final_));
        if self
            .cliente
            .enviar_estado(agente, room_id, TIPO_PODERES, None, &poderes)
            .await
            .is_err()
        {
            tracing::warn!(
                target: "poph_matrix::salas",
                motivo = "rebaixamento_de_poder_falhou",
                "não consegui rebaixar o poder do agente; a sala segue com o poder da criação"
            );
        }
    }

    /// Corpo do `createRoom` da conversa.
    fn corpo_da_criacao(&self, dados: &ConversaParaSala, agente: &Fantasma) -> CriarSala {
        let mut usuarios = serde_json::Map::new();
        // O criador nasce com 100: é o que o Synapse exige para ele escrever o próprio
        // `m.room.power_levels` na criação. O poder final desce em `ajustar_poder_final`.
        usuarios.insert(agente.user_id().to_owned(), json!(PODER_CRIACAO));
        let poder = json!({ "users": Value::Object(usuarios) });
        CriarSala {
            room_alias_name: Some(localpart_do_alias(&self.prefixo, dados.conversa)),
            name: Some(dados.nome_da_sala()),
            topic: Some(topico_da_sala(&dados.canal)),
            creation_content: Some(json!({ "type": TIPO_SALA })),
            power_level_content_override: Some(poder),
            // `private_chat`: o convidado não nasce com poder de administrador (o
            // `trusted_private_chat` daria 100 ao fantasma do cliente).
            preset: Some("private_chat".to_owned()),
            ..CriarSala::default()
        }
    }

    /// Cria a sala pelo alias ou acha a que já existe com ele.
    ///
    /// `M_ROOM_IN_USE` significa "a sala desta conversa já existe" — o alias é derivado do id da
    /// conversa, então não há outra dona possível. O único caso em que a criação é repetida é a
    /// corrida em que o alias aparece em uso e ainda não está no diretório.
    async fn sala_no_synapse(
        &self,
        dados: &ConversaParaSala,
        agente: &Fantasma,
    ) -> Result<String, ErroSala> {
        let alias = self.alias(dados.conversa);
        let corpo = self.corpo_da_criacao(dados, agente);
        for _ in 0..TENTATIVAS_ALIAS {
            match self.cliente.criar_sala(agente.user_id(), &corpo).await {
                Ok(sala) => return Ok(sala),
                Err(erro) if errcode(&erro) == Some(ERRCODE_ALIAS_EM_USO) => {
                    match self.cliente.resolver_alias(&alias).await {
                        Ok(sala) => return Ok(sala),
                        Err(erro) if errcode(&erro) == Some(ERRCODE_NAO_ENCONTRADO) => continue,
                        Err(erro) => return Err(erro.into()),
                    }
                }
                Err(erro) => return Err(erro.into()),
            }
        }
        Err(ErroSala::AliasEmDisputa {
            alias,
            tentativas: TENTATIVAS_ALIAS,
        })
    }
}

impl<P: PortaSalas> fmt::Debug for Provisionador<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A porta fica de fora: quem a implementa pode ter dado do banco no `Debug`.
        f.debug_struct("Provisionador")
            .field("cliente", &self.cliente)
            .field("fantasmas", &format_args!("{:?}", self.usuarios.cache()))
            .field("prefixo", &self.prefixo)
            .field("server_name", &self.server_name)
            .field("poder_agente", &self.poder_agente)
            .finish()
    }
}

/// `errcode` do Matrix dentro de um erro de resposta.
fn errcode(erro: &ErroChamada) -> Option<&str> {
    match erro {
        ErroChamada::Resposta { errcode, .. } => errcode.as_deref(),
        _ => None,
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn conversa() -> ConversationId {
        "0192f0aa-1111-7abc-8def-0123456789ab"
            .parse()
            .expect("uuid v7")
    }

    #[test]
    fn alias_e_deterministico_e_unico_por_conversa() {
        let uma = conversa();
        let outra: ConversationId = "0192f0aa-2222-7abc-8def-0123456789ab"
            .parse()
            .expect("uuid v7");
        assert_eq!(
            alias_da_conversa("ph", "matrix-dev.local", uma),
            alias_da_conversa("ph", "matrix-dev.local", uma)
        );
        assert_ne!(
            alias_da_conversa("ph", "matrix-dev.local", uma),
            alias_da_conversa("ph", "matrix-dev.local", outra)
        );
        let localpart = localpart_do_alias("ph", uma);
        assert!(
            localpart
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{localpart}"
        );
        assert!(!localpart.contains('-'), "{localpart}");
        assert!(localpart.starts_with("ph_c_"), "{localpart}");
    }

    #[test]
    fn prefixo_pequeno_nao_gera_alias_repetido_entre_conversas() {
        let mut vistos = std::collections::BTreeSet::new();
        for _ in 0..64 {
            let id = ConversationId::generate();
            let alias = localpart_do_alias("p", id);
            assert!(
                vistos.insert(alias),
                "alias repetido para conversa diferente"
            );
        }
    }

    #[test]
    fn nome_da_sala_cai_no_identificador_quando_nao_ha_nome() {
        let mut dados = dados_de_teste();
        dados.nome_cliente = Some("   ".to_owned());
        assert_eq!(dados.nome_da_sala(), "+55 11 99999-9999");
        dados.nome_cliente = Some("Maria".to_owned());
        assert_eq!(dados.nome_da_sala(), "Maria");
        dados.nome_cliente = None;
        dados.cliente = String::new();
        assert_eq!(dados.nome_da_sala(), NOME_CLIENTE_PADRAO);
    }

    #[test]
    fn metadata_so_leva_integracao_conhecida_pelo_frontend() {
        let mut dados = dados_de_teste();
        dados.canal = "whatsapp".to_owned();
        let metadata = MetadataSala::da_conversa(&dados);
        assert_eq!(metadata.integration_type.as_deref(), Some("whatsapp"));
        assert_eq!(metadata.channel, "whatsapp");

        dados.canal = "console".to_owned();
        let metadata = MetadataSala::da_conversa(&dados);
        assert_eq!(metadata.integration_type, None);
        assert_eq!(metadata.channel, "console");

        dados.canal = "WhatsApp".to_owned();
        assert_eq!(
            MetadataSala::da_conversa(&dados)
                .integration_type
                .as_deref(),
            Some("whatsapp")
        );
    }

    #[test]
    fn metadata_marca_conversa_fechada() {
        let mut dados = dados_de_teste();
        assert_eq!(MetadataSala::da_conversa(&dados).status, "active");
        dados.fechada = true;
        assert_eq!(MetadataSala::da_conversa(&dados).status, "closed");
    }

    fn dados_de_teste() -> ConversaParaSala {
        ConversaParaSala {
            org: OrgId::generate(),
            conversa: conversa(),
            cliente: "+55 11 99999-9999".to_owned(),
            nome_cliente: Some("Maria".to_owned()),
            agente: AgentId::generate(),
            nome_agente: Some("Agente".to_owned()),
            canal: "whatsapp".to_owned(),
            departamento: None,
            fechada: false,
            corte_backfill: None,
        }
    }
}
