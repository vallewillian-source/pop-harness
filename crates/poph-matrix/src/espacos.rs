//! Espaços por departamento (MX-12).
//!
//! No Element o atendente vê o trabalho organizado por departamento: cada departamento é um
//! **espaço**, e a sala de cada conversa fica **dentro** do espaço do departamento que a atende.
//! Este arquivo é quem põe a sala lá, e ele faz três coisas:
//!
//! 1. **Resolve o departamento** da conversa (P3): o departamento alvo da conversa; senão o
//!    departamento padrão do agente; senão nenhum — e sem departamento a sala simplesmente não
//!    entra em espaço nenhum, sem falha nenhuma.
//! 2. **Garante o espaço** do departamento: cria quando falta (alias determinístico
//!    `#dep_<id do departamento>`, que é a trava contra espaço duplicado, como o alias da sala na
//!    MX-11) ou liga um espaço que já existe no Element, criado pelo operador.
//! 3. **Põe a sala no espaço**: `m.space.child` no espaço, `m.space.parent` na sala, entrada
//!    restrita aos membros do espaço e convite dos membros do espaço para a sala, em lotes
//!    limitados. Quando a transferência muda o departamento, o mesmo caminho **move** a sala: põe
//!    no espaço novo e tira do antigo.
//!
//! ## Quem escreve o quê
//!
//! O espaço é do **bot do Application Service** (`@<prefixo>_as_bot`): ele é o dono de um espaço só
//! por org, independente de qual agente atende cada conversa, e é ele que escreve `m.space.child`.
//! A sala é do **fantasma do agente** (criada na MX-11), então é ele que escreve `m.space.parent` e
//! a regra de entrada na sala.
//!
//! Por isso ligar um espaço que já existe exige que o operador tenha convidado o bot e dado a ele
//! poder de escrever evento de estado (50 no padrão do Matrix): quando falta, o erro diz exatamente
//! isso, em vez de falhar em silêncio no meio.
//!
//! ## Fronteira e idempotência
//!
//! Como a provisão da sala (MX-11), a persistência entra pela porta [`PortaEspacos`]: `poph-matrix`
//! continua sem conhecer o banco e quem junta as peças (`poph-server`) implementa a porta sobre
//! `store.matrix()`. Toda a decisão mora aqui, e é isso que deixa o teste apontar um Synapse falso
//! com estado e provar criação, ligação, falta de permissão e mudança de espaço.
//!
//! Repetir a passagem não cria segundo espaço (o alias) nem reescreve o vínculo que já está
//! gravado; e a ordem das escrituras é de propósito: primeiro entra no espaço novo, só depois sai do
//! antigo — uma queda no meio deixa a sala visível em dois espaços (para corrigir) em vez de sumir
//! de todos.
//!
//! O vínculo só é gravado quando **todos** os convites da passagem foram entregues: um convite que
//! falhou não pode virar "sala já posicionada", senão o membro do espaço que ficou de fora nunca
//! mais seria convidado. Com convite pendente, a passagem seguinte refaz o caminho (as escritas no
//! Synapse são idempotentes) e convida só quem ainda não está na sala.
//!
//! Nada aqui guarda ou registra texto de cliente: os eventos levam id de departamento, nome de
//! departamento e ids de sala.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use poph_types::{ConversationId, DepartmentId, MatrixConfig, OrgId};
use serde_json::{Value, json};

use crate::cliente::{ClienteMatrix, CriarSala};
use crate::erro::{Classificacao, ErroChamada};
use crate::salas::ErroPorta;
use crate::usuarios::{ErroFantasmas, Fantasma, IdsFantasmas};

/// Marca do alias do espaço do departamento (`#dep_<id do departamento>:<servidor>`).
///
/// O alias `#dep_.*` é **compartilhado** com o registro do Application Service do Python (não é
/// exclusivo de ninguém), como o registro do Harness é gerado (MX-21): um espaço de departamento é
/// do ambiente, não de uma instância.
pub const MARCA_ESPACO: &str = "dep";

/// `creation_content.type` de um espaço do Matrix.
pub const TIPO_ESPACO: &str = "m.space";
/// Evento de estado que lista uma sala filha dentro do espaço (`state_key` = id da sala).
pub const TIPO_FILHO: &str = "m.space.child";
/// Evento de estado que diz de qual espaço a sala é filha (`state_key` = id do espaço).
pub const TIPO_PAI: &str = "m.space.parent";
/// Regras de entrada da sala.
pub const TIPO_REGRAS_ENTRADA: &str = "m.room.join_rules";
/// Poderes da sala/espaço — é dele que sai quem pode escrever `m.space.child`.
pub const TIPO_PODERES: &str = "m.room.power_levels";
/// Evento de estado do próprio módulo dentro do espaço que ele criou: qual departamento é aquele.
pub const TIPO_DEPARTAMENTO: &str = "com.sofacoding.departamento";

/// Poder que o Matrix exige, por padrão, para escrever evento de estado (`state_default`).
///
/// É o piso de quem escreve `m.space.child` num espaço que não declara outro valor.
pub const PODER_PADRAO_ESTADO: i64 = 50;

/// Quantas vezes a criação do espaço tenta de novo quando o alias aparece em uso e some do
/// diretório (corrida entre duas instâncias criando o mesmo espaço).
const TENTATIVAS_ALIAS: u8 = 3;

/// `errcode` do Synapse quando o alias já existe (comprovado na MX-02).
const ERRCODE_ALIAS_EM_USO: &str = "M_ROOM_IN_USE";
/// `errcode` do Synapse quando o alias não está no diretório.
const ERRCODE_NAO_ENCONTRADO: &str = "M_NOT_FOUND";
/// `errcode` do Synapse quando falta poder ou o usuário não é membro.
const ERRCODE_PROIBIDO: &str = "M_FORBIDDEN";

// ───────────────────────────── departamento (P3) ─────────────────────────────

/// Departamento que manda a sala para um espaço (P3): o alvo da conversa; senão o padrão do agente;
/// senão nenhum.
///
/// É uma escolha binária e ordenada de propósito: o agente padrão do departamento é a configuração
/// do atendimento, e a conversa só se afasta dela quando o agente transferiu para outro
/// departamento. Sem os dois, a sala fica sem espaço — melhor uma sala solta do que na sala errada.
pub fn resolver_departamento(
    alvo: Option<DepartmentId>,
    padrao_do_agente: Option<DepartmentId>,
) -> Option<DepartmentId> {
    alvo.or(padrao_do_agente)
}

/// Departamento já resolvido, com o nome que o espaço vai mostrar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepartamentoEspaco {
    /// Id do departamento.
    pub id: DepartmentId,
    /// Nome do departamento (nome do espaço quando o módulo o cria; vazio cai no id).
    pub nome: String,
}

impl DepartamentoEspaco {
    /// Nome visível do espaço: o nome do departamento; vazio (ou só espaço) cai no id.
    pub fn nome_do_espaco(&self) -> String {
        let limpo = self.nome.split_whitespace().collect::<Vec<_>>().join(" ");
        if limpo.is_empty() {
            self.id.to_string()
        } else {
            limpo
        }
    }
}

/// Aliases e ids do espaço do departamento.
///
/// A `localpart` sai do id do departamento sem hífen (a `localpart` do alias no Matrix é estreita),
/// e o id é UUID: tirar o hífen é injetivo, então dois departamentos nunca caem no mesmo alias.
pub fn localpart_do_alias(departamento: DepartmentId) -> String {
    let corpo: String = departamento
        .to_string()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|caractere| caractere.to_ascii_lowercase())
        .collect();
    format!("{MARCA_ESPACO}_{corpo}")
}

/// Alias completo do espaço do departamento (`#dep_<id>:<servidor>`).
pub fn alias_do_departamento(server_name: &str, departamento: DepartmentId) -> String {
    format!("#{}:{}", localpart_do_alias(departamento), server_name)
}

// ───────────────────────────── entrada e saída ─────────────────────────────

/// Quem criou o espaço do departamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigemEspaco {
    /// O módulo criou o espaço no Synapse.
    Automatico,
    /// O space já existia no Element e foi ligado ao departamento.
    Ligado,
}

/// Espaço do departamento como está gravado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EspacoGravado {
    /// Id do espaço no Synapse (`!…:servidor`).
    pub room_id: String,
    /// Alias do departamento (`#dep_<id>:<servidor>`).
    pub alias: String,
    /// Quem criou o espaço.
    pub origem: OrigemEspaco,
}

/// A sala de uma conversa e o departamento que deve mandá-la para um espaço.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalaParaEspaco {
    /// Org dona.
    pub org: OrgId,
    /// Conversa da sala.
    pub conversa: ConversationId,
    /// Id da sala no Synapse (o que a MX-11 devolve).
    pub sala: String,
    /// Fantasma do agente: é ele que escreve o `m.space.parent` e as regras de entrada na sala.
    pub agente: Fantasma,
    /// Departamento já resolvido por [`resolver_departamento`]; `None` = sem departamento.
    pub departamento: Option<DepartamentoEspaco>,
    /// Espaço que já existe no Element e deve ser ligado a este departamento (id `!…` ou alias
    /// `#…`). `None` = o módulo cria o espaço quando falta.
    pub existente: Option<String>,
}

/// O que a passagem fez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Posicao {
    /// Conversa sem departamento resolvido e sem espaço antigo: nada foi escrito, nada falou com o
    /// Synapse.
    SemDepartamento,
    /// A sala já estava gravada no espaço deste departamento: nada foi escrito no Synapse.
    JaPosicionada {
        /// Espaço em que a sala está.
        espaco: String,
    },
    /// A sala foi posta no espaço agora (espaço criado, ligado ou sala movida).
    Posicionada {
        /// Espaço em que a sala está.
        espaco: String,
        /// A sala veio de outro espaço nesta passagem.
        movida: bool,
        /// Convites entregues aos membros do espaço.
        convidados: u32,
        /// Convites que o Synapse recusou. Enquanto houver algum, o vínculo **não** foi gravado e a
        /// passagem seguinte refaz o caminho para convidar quem ficou de fora.
        convites_falhos: u32,
    },
    /// A conversa ficou sem departamento: a sala saiu do espaço em que estava.
    Removida {
        /// Espaço de onde a sala saiu.
        espaco: String,
    },
}

// ───────────────────────────── porta de persistência ─────────────────────────────

/// Persistência que o módulo usa para os espaços e para o vínculo da sala.
///
/// Quem implementa (o servidor, sobre `store.matrix()`) precisa garantir:
///
/// - [`PortaEspacos::garantir_espaco`] é **idempotente por departamento**: a segunda chamada devolve
///   a linha que já existe (o primeiro `room_id` ganha); espaço ou alias já ligado a outro
///   departamento da mesma org é conflito.
/// - [`PortaEspacos::garantir_espaco`] recebe **sempre** o alias do departamento
///   (`#dep_<id>:<servidor>`), inclusive quando o espaço foi ligado a um que já existia no Element:
///   nesse caso ele é o nome do departamento **para o módulo** e não é publicado no Synapse (quem
///   deu o alias foi o operador, no Element).
/// - [`PortaEspacos::definir_espaco_da_sala`] grava `matrix_rooms.space_room_id` e repetir o mesmo
///   valor não muda nada.
pub trait PortaEspacos {
    /// Espaço do departamento (`None` quando ainda não existe).
    fn espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
    ) -> impl Future<Output = Result<Option<EspacoGravado>, ErroPorta>> + Send;

    /// Grava o espaço do departamento e devolve a linha gravada (a que já existia, quando houver).
    fn garantir_espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
        room_id: &str,
        alias: &str,
        origem: OrigemEspaco,
    ) -> impl Future<Output = Result<EspacoGravado, ErroPorta>> + Send;

    /// Espaço em que a sala da conversa está gravada (`None` quando ela não está em nenhum).
    fn espaco_da_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<String>, ErroPorta>> + Send;

    /// Grava o espaço da sala (`None` tira a sala do espaço).
    fn definir_espaco_da_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        espaco: Option<&str>,
    ) -> impl Future<Output = Result<(), ErroPorta>> + Send;
}

// ───────────────────────────── erro ─────────────────────────────

/// Falha ao pôr a sala no espaço do departamento.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErroEspaco {
    /// O Synapse recusou ou não respondeu uma chamada.
    #[error(transparent)]
    Chamada(#[from] ErroChamada),
    /// O armazenamento do módulo falhou.
    #[error(transparent)]
    Armazenamento(#[from] ErroPorta),
    /// Falta permissão no espaço ou na sala, e quem resolve é o operador, no Element.
    #[error("falta permissão para pôr a sala no espaço: {o_que_falta}")]
    SemPermissao {
        /// O que o operador precisa fazer, em português, com o usuário e o espaço citados.
        o_que_falta: String,
    },
    /// O espaço informado não é id nem alias de sala.
    #[error("espaço informado inválido: {motivo}")]
    EspacoInvalido {
        /// Motivo curto (sem repetir o valor informado).
        motivo: &'static str,
    },
    /// Falha que é defeito deste módulo, não do ambiente.
    #[error("falha interna do módulo Matrix: {motivo}")]
    Interno {
        /// Motivo curto.
        motivo: String,
    },
}

impl ErroEspaco {
    /// Como o laço deve tratar a falha. Não repete nada aqui: só informa.
    pub fn classificacao(&self) -> Classificacao {
        match self {
            Self::Chamada(erro) => erro.classificacao(),
            Self::Armazenamento(_) => Classificacao::Transitoria,
            // Falta de permissão é **transitória** de propósito: quem conserta é o operador, no
            // Element, e a sala tem de entrar no espaço no ciclo seguinte à correção. Definitiva
            // faria a sala ser isolada e nunca mais tentada.
            Self::SemPermissao { .. } => Classificacao::Transitoria,
            Self::EspacoInvalido { .. } | Self::Interno { .. } => Classificacao::Definitiva,
        }
    }
}

// ───────────────────────────── limites dos convites ─────────────────────────────

/// Como os membros do espaço são convidados para a sala.
///
/// Um espaço grande não pode virar uma rajada de convites no Synapse: os membros entram em lotes,
/// com concorrência limitada dentro do lote e uma pausa entre lotes, e a passagem tem teto. O
/// resto fica para a passagem seguinte (o vínculo não espera por eles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitesConvite {
    /// Convites por lote.
    pub lote: usize,
    /// Convites simultâneos dentro do lote.
    pub concorrencia: usize,
    /// Pausa entre dois lotes.
    pub pausa: Duration,
    /// Teto de convites numa passagem.
    pub teto: usize,
}

impl Default for LimitesConvite {
    fn default() -> Self {
        Self {
            lote: 20,
            concorrencia: 4,
            pausa: Duration::from_millis(100),
            teto: 200,
        }
    }
}

impl LimitesConvite {
    /// Mesmos limites com outros valores, sem deixar nenhum em zero (que travaria o laço).
    pub fn com(lote: usize, concorrencia: usize, pausa: Duration, teto: usize) -> Self {
        Self {
            lote: lote.max(1),
            concorrencia: concorrencia.max(1),
            pausa,
            teto: teto.max(1),
        }
    }
}

// ───────────────────────────── o serviço ─────────────────────────────

/// Põe a sala de cada conversa no espaço do departamento e mantém o vínculo em dia.
pub struct Espacos<P: PortaEspacos> {
    cliente: ClienteMatrix,
    ids: IdsFantasmas,
    /// Id do bot do Application Service, montado uma vez (`@<prefixo>_as_bot:<servidor>`).
    bot: String,
    porta: P,
    limites: LimitesConvite,
}

impl<P: PortaEspacos> Espacos<P> {
    /// Monta com as peças prontas (os limites de convite vêm de fora).
    pub fn com(cliente: ClienteMatrix, ids: IdsFantasmas, porta: P) -> Self {
        Self {
            bot: ids.bot().user_id().to_owned(),
            cliente,
            ids,
            porta,
            limites: LimitesConvite::default(),
        }
    }

    /// Monta a partir da seção `[matrix]` já validada.
    pub fn da_config(
        cliente: ClienteMatrix,
        porta: P,
        config: &MatrixConfig,
    ) -> Result<Self, ErroFantasmas> {
        let ids = IdsFantasmas::da_config(config)?;
        Ok(Self::com(cliente, ids, porta))
    }

    /// Troca os limites dos convites (um espaço grande, ou um teste que quer o lote pequeno).
    pub fn com_limites(mut self, limites: LimitesConvite) -> Self {
        self.limites = limites;
        self
    }

    /// Bot do Application Service: o dono e o escritor do espaço (`@<prefixo>_as_bot`).
    pub fn bot(&self) -> &str {
        &self.bot
    }

    /// Alias do espaço deste departamento.
    pub fn alias(&self, departamento: DepartmentId) -> String {
        alias_do_departamento(self.cliente.server_name(), departamento)
    }

    /// Espaço gravado para o departamento (só leitura; não fala com o Synapse).
    pub async fn espaco(
        &self,
        org: OrgId,
        departamento: DepartmentId,
    ) -> Result<Option<EspacoGravado>, ErroEspaco> {
        Ok(self.porta.espaco(org, departamento).await?)
    }

    /// Garante o espaço do departamento e devolve onde ele está.
    ///
    /// - Já gravado: devolve o que está lá (não cria segundo espaço nem revalida permissão).
    /// - Liga a um espaço existente: confere que o bot é membro e pode escrever `m.space.child`.
    /// - Sem espaço gravado: cria pelo alias do departamento (repetir cai em `M_ROOM_IN_USE` e o
    ///   alias leva à sala que já existe).
    pub async fn garantir_espaco(
        &self,
        org: OrgId,
        departamento: &DepartamentoEspaco,
        existente: Option<&str>,
    ) -> Result<EspacoGravado, ErroEspaco> {
        if let Some(gravado) = self.porta.espaco(org, departamento.id).await? {
            return Ok(gravado);
        }
        let alias = self.alias(departamento.id);
        let (room_id, origem) = match existente {
            Some(referencia) => {
                let room_id = self.resolver_espaco(referencia).await?;
                self.conferir_permissao(&room_id).await?;
                (room_id, OrigemEspaco::Ligado)
            }
            None => (
                self.criar_espaco(departamento).await?,
                OrigemEspaco::Automatico,
            ),
        };
        let gravado = self
            .porta
            .garantir_espaco(org, departamento.id, &room_id, &alias, origem)
            .await?;
        Ok(gravado)
    }

    /// Põe a sala no espaço do departamento (criando o espaço, ligando um existente ou movendo).
    ///
    /// Repetir com a sala já no espaço não escreve nada no Synapse. Quando um convite do lote falha,
    /// o vínculo **não** é gravado: a passagem seguinte refaz o caminho e convida quem ficou de fora.
    pub async fn posicionar(&self, dados: &SalaParaEspaco) -> Result<Posicao, ErroEspaco> {
        if dados.sala.trim().is_empty() {
            return Err(ErroEspaco::EspacoInvalido {
                motivo: "a sala da conversa está vazia",
            });
        }
        let atual = self.porta.espaco_da_sala(dados.org, dados.conversa).await?;

        let Some(departamento) = &dados.departamento else {
            // Sem departamento: se estava num espaço, sai dele; se não estava, nada acontece.
            let Some(antigo) = atual else {
                return Ok(Posicao::SemDepartamento);
            };
            self.remover_do_espaco(dados, &antigo).await?;
            self.porta
                .definir_espaco_da_sala(dados.org, dados.conversa, None)
                .await?;
            return Ok(Posicao::Removida { espaco: antigo });
        };

        let espaco = self
            .garantir_espaco(dados.org, departamento, dados.existente.as_deref())
            .await?;
        if atual.as_deref() == Some(espaco.room_id.as_str()) {
            return Ok(Posicao::JaPosicionada {
                espaco: espaco.room_id,
            });
        }

        // 1. Entra no espaço novo antes de sair do antigo: se algo falhar no meio, a sala aparece
        //    em dois espaços (fácil de corrigir) em vez de sumir de todos.
        self.escrever_filho(&espaco.room_id, &dados.sala).await?;
        self.escrever_pai(dados, &espaco.room_id).await?;
        self.escrever_regras_de_entrada(dados, &espaco.room_id)
            .await?;
        let (convidados, convites_falhos) = self.convidar_membros(dados, &espaco.room_id).await?;

        // Convite que falhou não pode virar "sala posicionada": se o vínculo fosse gravado agora, a
        // passagem seguinte sairia por `JaPosicionada` e o membro que não recebeu o convite ficaria
        // fora da conversa para sempre (defeito achado na MX-29). Sem gravar, a passagem seguinte
        // refaz o caminho — as escritas no Synapse são idempotentes — e convida só quem ainda não
        // está na sala (e nada é repetido aqui: a tentativa seguinte é a passagem seguinte).
        if convites_falhos > 0 {
            return Ok(Posicao::Posicionada {
                espaco: espaco.room_id,
                // A sala continua onde o banco diz que ela está: nada foi movido nem desfeito.
                movida: false,
                convidados,
                convites_falhos,
            });
        }

        // 2. Sai do espaço antigo: some o `m.space.child` lá e o `m.space.parent` daqui.
        let mut movida = false;
        if let Some(antigo) = atual.filter(|antigo| *antigo != espaco.room_id) {
            self.remover_filho(&antigo, &dados.sala).await?;
            self.remover_pai(dados, &antigo).await?;
            movida = true;
        }

        self.porta
            .definir_espaco_da_sala(dados.org, dados.conversa, Some(&espaco.room_id))
            .await?;
        Ok(Posicao::Posicionada {
            espaco: espaco.room_id,
            movida,
            convidados,
            convites_falhos,
        })
    }

    /// Cria o espaço do departamento pelo alias (idempotente pelo dois passos da MX-02).
    async fn criar_espaco(&self, departamento: &DepartamentoEspaco) -> Result<String, ErroEspaco> {
        let alias = self.alias(departamento.id);
        let corpo = CriarSala {
            room_alias_name: Some(localpart_do_alias(departamento.id)),
            name: Some(departamento.nome_do_espaco()),
            topic: Some(format!("Departamento · {}", departamento.nome_do_espaco())),
            creation_content: Some(json!({ "type": TIPO_ESPACO })),
            preset: Some("private_chat".to_owned()),
            ..CriarSala::default()
        };
        for _ in 0..TENTATIVAS_ALIAS {
            match self.cliente.criar_sala(&self.bot, &corpo).await {
                Ok(espaco) => {
                    self.marcar_departamento(&espaco, departamento).await?;
                    return Ok(espaco);
                }
                Err(erro) if errcode(&erro) == Some(ERRCODE_ALIAS_EM_USO) => {
                    match self.cliente.resolver_alias(&alias).await {
                        Ok(espaco) => return Ok(espaco),
                        Err(erro) if errcode(&erro) == Some(ERRCODE_NAO_ENCONTRADO) => continue,
                        Err(erro) => return Err(erro.into()),
                    }
                }
                Err(erro) => return Err(erro.into()),
            }
        }
        Err(ErroEspaco::Interno {
            motivo: format!(
                "o alias `{alias}` apareceu em uso e sumiu do diretório em {TENTATIVAS_ALIAS} tentativas"
            ),
        })
    }

    /// Escreve no espaço novo qual departamento ele representa (só quando o módulo o criou; num
    /// espaço do operador o módulo não escreve metadata dele).
    async fn marcar_departamento(
        &self,
        espaco: &str,
        departamento: &DepartamentoEspaco,
    ) -> Result<(), ErroEspaco> {
        let corpo = json!({
            "department_id": departamento.id.to_string(),
            "name": departamento.nome_do_espaco(),
        });
        self.cliente
            .enviar_estado(&self.bot, espaco, TIPO_DEPARTAMENTO, None, &corpo)
            .await?;
        Ok(())
    }

    /// Id do espaço já existente: alias (`#…`) resolve pelo diretório; id (`!…`) é usado como veio.
    async fn resolver_espaco(&self, referencia: &str) -> Result<String, ErroEspaco> {
        let referencia = referencia.trim();
        if referencia.starts_with('#') {
            return self
                .cliente
                .resolver_alias(referencia)
                .await
                .map_err(ErroEspaco::from);
        }
        if referencia.starts_with('!') {
            return Ok(referencia.to_owned());
        }
        Err(ErroEspaco::EspacoInvalido {
            motivo: "informe o id do espaço (`!…`) ou o alias dele (`#…`)",
        })
    }

    /// Confere que o bot é membro do espaço e pode escrever `m.space.child` nele.
    async fn conferir_permissao(&self, espaco: &str) -> Result<(), ErroEspaco> {
        let membros = self.cliente.membros(&self.bot, espaco).await?;
        if !membros.contains_key(&self.bot) {
            return Err(self.sem_permissao(espaco, "ele não é membro do espaço"));
        }
        let poderes = self
            .cliente
            .ler_estado(&self.bot, espaco, TIPO_PODERES, None)
            .await?;
        let necessario = nivel_de_evento(&poderes, TIPO_FILHO);
        let poder = poder_do_usuario(&poderes, &self.bot);
        if poder < necessario {
            return Err(self.sem_permissao(
                espaco,
                &format!(
                    "o poder dele no espaço é {poder} e escrever `{TIPO_FILHO}` exige {necessario}"
                ),
            ));
        }
        Ok(())
    }

    /// Erro de permissão com o que o operador tem de fazer no Element.
    fn sem_permissao(&self, espaco: &str, motivo: &str) -> ErroEspaco {
        ErroEspaco::SemPermissao {
            o_que_falta: format!(
                "no Element, convide {} para o espaço {espaco} e dê a ele poder {PODER_PADRAO_ESTADO} \
                 para eventos de estado ({motivo})",
                self.bot
            ),
        }
    }

    /// `m.space.child` no espaço (`state_key` = sala).
    async fn escrever_filho(&self, espaco: &str, sala: &str) -> Result<(), ErroEspaco> {
        match self
            .cliente
            .espaco_filho(&self.bot, espaco, sala, None)
            .await
        {
            Ok(_) => Ok(()),
            Err(erro) if errcode(&erro) == Some(ERRCODE_PROIBIDO) => {
                Err(self.sem_permissao(espaco, "o Synapse recusou a escrita do `m.space.child`"))
            }
            Err(erro) => Err(erro.into()),
        }
    }

    /// Tira o `m.space.child` do espaço (conteúdo vazio é como o Matrix remove o filho).
    async fn remover_filho(&self, espaco: &str, sala: &str) -> Result<(), ErroEspaco> {
        self.cliente
            .enviar_estado(&self.bot, espaco, TIPO_FILHO, Some(sala), &json!({}))
            .await?;
        Ok(())
    }

    /// `m.space.parent` na sala (`state_key` = espaço), escrito pelo fantasma do agente.
    async fn escrever_pai(&self, dados: &SalaParaEspaco, espaco: &str) -> Result<(), ErroEspaco> {
        let corpo = json!({
            "via": [self.cliente.server_name()],
            "canonical": true,
        });
        self.cliente
            .enviar_estado(
                dados.agente.user_id(),
                &dados.sala,
                TIPO_PAI,
                Some(espaco),
                &corpo,
            )
            .await?;
        Ok(())
    }

    /// Entrada na sala restrita a quem é membro do espaço.
    async fn escrever_regras_de_entrada(
        &self,
        dados: &SalaParaEspaco,
        espaco: &str,
    ) -> Result<(), ErroEspaco> {
        let corpo = json!({
            "join_rule": "restricted",
            "allow": [{ "type": "m.room_membership", "room_id": espaco }],
        });
        self.cliente
            .enviar_estado(
                dados.agente.user_id(),
                &dados.sala,
                TIPO_REGRAS_ENTRADA,
                None,
                &corpo,
            )
            .await?;
        Ok(())
    }

    /// Tira a sala do espaço: sem `m.space.child` no espaço e sem `m.space.parent` na sala.
    async fn remover_do_espaco(
        &self,
        dados: &SalaParaEspaco,
        espaco: &str,
    ) -> Result<(), ErroEspaco> {
        self.remover_filho(espaco, &dados.sala).await?;
        self.remover_pai(dados, espaco).await?;
        Ok(())
    }

    /// Tira o `m.space.parent` da sala para um espaço (conteúdo vazio).
    async fn remover_pai(&self, dados: &SalaParaEspaco, espaco: &str) -> Result<(), ErroEspaco> {
        self.cliente
            .enviar_estado(
                dados.agente.user_id(),
                &dados.sala,
                TIPO_PAI,
                Some(espaco),
                &json!({}),
            )
            .await?;
        Ok(())
    }

    /// Convida os membros do espaço para a sala, em lotes limitados.
    ///
    /// Ficam de fora o bot do Application Service e os fantasmas desta instância (cliente, agente e
    /// aparelho): eles não são gente no Element e não precisam de convite para uma sala que o
    /// módulo controla. Quem já está na sala também não é convidado de novo.
    ///
    /// Falha de um convite não derruba o vínculo: quem decide o vínculo é quem chama, e um convite
    /// falho faz a passagem seguinte repetir o caminho (só quem ainda não está na sala é convidado).
    /// O que volta é `(entregues, falhos)`.
    async fn convidar_membros(
        &self,
        dados: &SalaParaEspaco,
        espaco: &str,
    ) -> Result<(u32, u32), ErroEspaco> {
        let no_espaco = self.cliente.membros(&self.bot, espaco).await?;
        let na_sala = self
            .cliente
            .membros(dados.agente.user_id(), &dados.sala)
            .await?;
        let alvos: Vec<String> = no_espaco
            .keys()
            .filter(|id| !self.ids.eh_bot(id) && !self.ids.eh_fantasma(id))
            .filter(|id| !na_sala.contains_key(*id))
            .take(self.limites.teto)
            .cloned()
            .collect();

        let mut convidados = 0u32;
        let mut falhos = 0u32;
        for lote in alvos.chunks(self.limites.lote) {
            let usuario = dados.agente.user_id().to_owned();
            let sala = dados.sala.clone();
            let entregues: Vec<Result<(), ErroChamada>> = stream::iter(
                lote.iter()
                    .cloned()
                    .map(|alvo| self.convidar_um(&usuario, &sala, alvo)),
            )
            .buffer_unordered(self.limites.concorrencia)
            .collect()
            .await;
            for resultado in entregues {
                match resultado {
                    Ok(()) => convidados += 1,
                    Err(_) => falhos += 1,
                }
            }
            tokio::time::sleep(self.limites.pausa).await;
        }
        Ok((convidados, falhos))
    }

    /// Um convite, com o alvo já em mãos.
    ///
    /// O alvo vai por valor de propósito: um futuro que empresta o alvo de um `map` sobre `&String`
    /// cria uma família de closures que não se generaliza para quem compõe este futuro (o laço de
    /// estado da conversa, no servidor).
    async fn convidar_um(
        &self,
        usuario: &str,
        sala: &str,
        alvo: String,
    ) -> Result<(), ErroChamada> {
        self.cliente.convidar(usuario, sala, &alvo).await
    }
}

impl<P: PortaEspacos> fmt::Debug for Espacos<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A porta fica de fora: quem a implementa pode ter dado do banco no `Debug`.
        f.debug_struct("Espacos")
            .field("cliente", &self.cliente)
            .field("bot", &self.bot)
            .field("limites", &self.limites)
            .finish()
    }
}

// ───────────────────────────── auxiliares ─────────────────────────────

/// `errcode` do Matrix dentro de um erro de resposta.
fn errcode(erro: &ErroChamada) -> Option<&str> {
    match erro {
        ErroChamada::Resposta { errcode, .. } => errcode.as_deref(),
        _ => None,
    }
}

/// Poder de um usuário num evento `m.room.power_levels`, com os padrões do Matrix.
fn poder_do_usuario(poderes: &Value, usuario: &str) -> i64 {
    poderes
        .get("users")
        .and_then(|usuarios| usuarios.get(usuario))
        .and_then(Value::as_i64)
        .or_else(|| poderes.get("users_default").and_then(Value::as_i64))
        .unwrap_or(0)
}

/// Poder necessário para escrever um evento de estado, com os padrões do Matrix
/// (`state_default` = 50).
fn nivel_de_evento(poderes: &Value, tipo: &str) -> i64 {
    poderes
        .get("events")
        .and_then(|eventos| eventos.get(tipo))
        .and_then(Value::as_i64)
        .or_else(|| poderes.get("state_default").and_then(Value::as_i64))
        .unwrap_or(PODER_PADRAO_ESTADO)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn resolver_departamento_prefere_o_alvo_e_cai_no_padrao() {
        let alvo = DepartmentId::generate();
        let padrao = DepartmentId::generate();
        assert_eq!(resolver_departamento(Some(alvo), Some(padrao)), Some(alvo));
        assert_eq!(resolver_departamento(None, Some(padrao)), Some(padrao));
        assert_eq!(resolver_departamento(None, None), None);
    }

    #[test]
    fn alias_do_espaco_e_deterministico_e_unico_por_departamento() {
        let um = DepartmentId::generate();
        let outro = DepartmentId::generate();
        assert_eq!(
            alias_do_departamento("matrix-dev.local", um),
            alias_do_departamento("matrix-dev.local", um)
        );
        assert_ne!(
            alias_do_departamento("matrix-dev.local", um),
            alias_do_departamento("matrix-dev.local", outro)
        );
        let localpart = localpart_do_alias(um);
        assert!(localpart.starts_with("dep_"), "{localpart}");
        assert!(
            localpart
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{localpart}"
        );
    }

    #[test]
    fn nome_do_espaco_cai_no_id_quando_o_departamento_nao_tem_nome() {
        let id = DepartmentId::generate();
        let departamento = DepartamentoEspaco {
            id,
            nome: "  Comercial   Rio ".to_owned(),
        };
        assert_eq!(departamento.nome_do_espaco(), "Comercial Rio");
        let sem_nome = DepartamentoEspaco {
            id,
            nome: "   ".to_owned(),
        };
        assert_eq!(sem_nome.nome_do_espaco(), id.to_string());
    }

    #[test]
    fn poder_e_nivel_de_evento_seguem_os_padroes_do_matrix() {
        let vazio = json!({});
        assert_eq!(poder_do_usuario(&vazio, "@ph_as_bot:dev"), 0);
        assert_eq!(nivel_de_evento(&vazio, TIPO_FILHO), PODER_PADRAO_ESTADO);

        let poderes = json!({
            "users": { "@ph_as_bot:dev": 60 },
            "users_default": 5,
            "events": { TIPO_FILHO: 70 },
            "state_default": 50,
        });
        assert_eq!(poder_do_usuario(&poderes, "@ph_as_bot:dev"), 60);
        assert_eq!(poder_do_usuario(&poderes, "@outro:dev"), 5);
        assert_eq!(nivel_de_evento(&poderes, TIPO_FILHO), 70);
        assert_eq!(nivel_de_evento(&poderes, "m.room.name"), 50);
    }

    #[test]
    fn limites_de_convite_nunca_ficam_em_zero() {
        let limites = LimitesConvite::com(0, 0, Duration::ZERO, 0);
        assert_eq!(limites.lote, 1);
        assert_eq!(limites.concorrencia, 1);
        assert_eq!(limites.teto, 1);
        assert_eq!(LimitesConvite::default().lote, 20);
    }

    #[test]
    fn falta_de_permissao_e_transitoria_para_a_sala_entrar_depois_da_correcao() {
        let erro = ErroEspaco::SemPermissao {
            o_que_falta: "convide o bot".to_owned(),
        };
        assert_eq!(erro.classificacao(), Classificacao::Transitoria);
        let invalido = ErroEspaco::EspacoInvalido {
            motivo: "nem id nem alias",
        };
        assert_eq!(invalido.classificacao(), Classificacao::Definitiva);
    }
}
