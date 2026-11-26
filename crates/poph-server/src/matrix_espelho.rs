//! Laço de espelhamento de mensagens do Matrix (MX-14).
//!
//! Transforma o que o Harness já gravou em eventos nas salas do Element. Ele **não** fica no
//! caminho do cliente: lê `messages` depois, em páginas, e entrega ao Synapse pelo
//! [`DestinoMatrix`]. O `poph serve` ainda não liga este laço (isso é a MX-20); quem liga chama
//! [`EspelhoMatrix::executar`].
//!
//! ## O que cada ciclo faz
//!
//! 1. Lê as orgs com o módulo ligado e o cursor de mensagens de cada uma (`matrix_estado`).
//! 2. Se algum cursor ficou à frente do maior `rowid` que existe, recua para esse ponto (R5).
//! 3. Lê `messages` a partir do menor cursor, em páginas de [`PAGINA`] linhas (memória limitada).
//! 4. Para cada mensagem, decide: já espelhada (pela posição ou pelo id no cursor), antes do corte
//!    do backfill, de org desligada, ou para espelhar. Para espelhar, passa pelo
//!    [`formato::decidir`]: `Erro` vai para `matrix_falhas`, sem chamar o Synapse. Mensagem do
//!    cliente com anexo vira também evento de mídia (MX-43), na frente do texto.
//! 5. Entrega em paralelo **entre salas** (até `concorrencia`) e em ordem **dentro da sala**. Uma
//!    falha transitória interrompe a sala no ponto da falha.
//! 6. Avança o cursor só até a primeira linha não resolvida (marca-d'água contígua). Nada é
//!    pulado: a linha pendente volta no ciclo seguinte.
//!
//! ## Regras de falha
//!
//! - **Transitória** (rede, 5xx, prazo, 429 sem espera): fica pendente; o ciclo para no ponto e o
//!   próximo espera um atraso que dobra a cada ciclo com falha, até [`TETO_ATRASO`].
//! - **Definitiva** (4xx do Synapse): conta a tentativa por mensagem, em memória. Até
//!   [`TENTATIVAS_DEFINITIVAS`] fica pendente; ao chegar lá, grava em `matrix_falhas` com o código
//!   curto do erro e a fila segue. Um `txnId` repetido não duplica no Synapse (MX-02), então
//!   repetir é seguro.
//!
//! ## Por que a leitura é global e o cursor é por org
//!
//! `messages` é uma tabela só, com todas as orgs. O cursor fica por org (`matrix_estado`), mas o
//! laço lê uma vez, do menor cursor entre as orgs ligadas, e filtra por org depois. Assim uma
//! consulta serve a todas, e cada org só processa o que é dela e é posterior ao próprio cursor.
//!
//! ## Por que o cursor guarda o id
//!
//! O `rowid` pode ser reaproveitado quando a última linha é apagada. Por isso o cursor guarda
//! também o id da mensagem daquela posição: uma linha com o mesmo `rowid` e outro id é mensagem
//! nova e é espelhada; com o mesmo id, já foi espelhada e é pulada.
//!
//! ## Mídia do cliente (MX-43)
//!
//! O anexo que o cliente manda já está no banco (`attachments`) quando o laço passa pela mensagem.
//! Para mensagem de entrada do cliente com anexo, o laço monta os passos de mídia **antes** do
//! texto: lê o blob (teto de [`poph_matrix::midia::LIMITE_MIDIA_BYTES`]), sobe ao Synapse como o
//! fantasma do cliente e publica `m.image`/`m.file` com o bloco `com.<marca>.attachment`
//! (MX-43). O texto do cliente, quando existe, é a legenda e sai depois da mídia, como já saía.
//!
//! O que **não** entra na sala, e por quê:
//!
//! - anexo acima do limite, vazio ou ilegível no disco → um `m.notice` na sala dizendo o motivo,
//!   e a mensagem segue resolvida (o cursor não fica preso nela);
//! - falha definitiva do Synapse na mídia → o mesmo aviso antes de isolar em `matrix_falhas`,
//!   depois de [`TENTATIVAS_DEFINITIVAS`] tentativas;
//! - falha transitória → fica pendente, como qualquer outro envio.
//!
//! `txnId` derivado do id do anexo: reprocessar a mesma mensagem não duplica o evento na sala. O
//! upload em si não tem chave de idempotência no Synapse; uma queda entre o upload e o evento pode
//! deixar um arquivo sem evento (limite conhecido).
//!
//! Toda mídia entregue é **registrada** em `matrix_midia` (MX-46) com a sala, o evento e o
//! `mxc://`: é essa linha que a rotina de expiração lê depois do prazo para redigir o evento e
//! apagar o arquivo. Sem a linha, a mídia ficaria no Synapse para sempre.
//!
//! ## O que este arquivo não faz
//!
//! Classificar eventos recebidos (MX-17), espelhar o estado da conversa (MX-15), ligar no `poph
//! serve` (MX-20) e métricas (MX-26). Nenhum log leva texto de cliente: só ids, contadores e o
//! código curto do erro.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use poph_matrix::cliente::ClienteMatrix;
use poph_matrix::erro::{Classificacao, ErroChamada};
use poph_matrix::espacos::{
    DepartamentoEspaco, Espacos, Posicao, SalaParaEspaco, resolver_departamento,
};
use poph_matrix::formato::{
    self, Decisao, Envio, MensagemParaEspelho, MotivoNaoEspelhar, OrigemMensagem, PapelMensagem,
    Sentido,
};
use poph_matrix::midia::{self, AnexoParaMidia, Midia, MotivoDeRecusa};
use poph_matrix::salas::{
    ConversaParaSala, ErroPorta, ErroSala, EstadoGravado, PortaSalas, Provisionador, SalaGravada,
};
use poph_matrix::usuarios::{ErroFantasmas, Fantasma, IdsFantasmas};
use poph_store::{
    AttachmentRecord, CategoriaMidia, ConversationStatus, Direction, EstadoSala, MensagemDesde,
    NovaMidia, Origin, Role, Store, StoreError,
};
use poph_types::{AgentId, ConversationId, MatrixConfig, MessageId, OrgId};
use serde_json::json;

use crate::matrix_inicio::PortaEspacosStore;
use crate::matrix_metricas::MetricasMatrix;

/// Linhas por página da leitura de `messages`. Define o teto de memória do ciclo.
pub const PAGINA: u32 = 200;

/// Páginas por ciclo. Depois disso o ciclo termina e o próximo continua do cursor.
pub const MAX_PAGINAS_POR_CICLO: u32 = 25;

/// Quantas falhas definitivas seguidas da mesma mensagem antes de isolá-la em `matrix_falhas`.
pub const TENTATIVAS_DEFINITIVAS: u32 = 3;

/// Espera máxima entre ciclos com falha transitória.
pub const TETO_ATRASO: Duration = Duration::from_secs(60);

/// Motivo gravado quando a mensagem não vira um envio válido.
const MOTIVO_FORMATO: &str = "mensagem_invalida";
/// Motivo gravado quando a sala não pôde ser garantida.
const MOTIVO_SALA: &str = "sala_rejeitada";
/// Motivo gravado quando o Synapse recusou o envio sem código curto.
const MOTIVO_ENVIO: &str = "envio_rejeitado";
/// Motivo gravado quando o usuário do aparelho não pôde entrar na sala (MX-42).
const MOTIVO_APARELHO: &str = "aparelho_fora_da_sala";
/// Motivo gravado quando a mídia do cliente não pôde ser montada (MX-43): fantasma inválido.
const MOTIVO_MIDIA: &str = "midia_invalida";
/// Tamanho máximo do motivo (o mesmo `CHECK` de `matrix_falhas`).
const LIMITE_MOTIVO: usize = 64;

// ───────────────────────────── ponto de extensão ─────────────────────────────

/// Lado Synapse do espelho: garantir a sala e enviar um evento.
///
/// A implementação real é [`DestinoReal`]; os testes usam um duplo em memória. Assim o laço e o
/// banco são de verdade nos testes, e só a rede é simulada.
pub trait DestinoMatrix: Send + Sync {
    /// Id da sala da conversa, criando-a se ainda não existir (MX-11).
    fn garantir_sala(
        &self,
        dados: &ConversaParaSala,
    ) -> impl Future<Output = Result<String, ErroSala>> + Send;

    /// Garante que o usuário do aparelho (MX-42) está dentro da sala antes de falar nela.
    fn garantir_aparelho(
        &self,
        dados: &ConversaParaSala,
        sala: &str,
        aparelho: &Fantasma,
    ) -> impl Future<Output = Result<(), ErroSala>> + Send;

    /// Envia um evento já decidido pelo formato, como o usuário virtual do envio.
    fn enviar(
        &self,
        sala: &str,
        envio: &Envio,
    ) -> impl Future<Output = Result<(), ErroChamada>> + Send;

    /// Sobe a mídia do cliente ao Synapse e publica o evento na sala (MX-43).
    ///
    /// Devolve o evento e o `mxc://` que ficaram na sala: a expiração (MX-46) precisa dos dois para
    /// redigir o evento e apagar o arquivo depois do prazo.
    fn enviar_midia(
        &self,
        sala: &str,
        envio: &EnvioMidia,
    ) -> impl Future<Output = Result<MidiaPublicada, ErroChamada>> + Send;

    /// Escreve um aviso em texto na sala (MX-43): mídia que não entrou, em `m.notice`.
    fn avisar(
        &self,
        sala: &str,
        aviso: &AvisoNaSala,
    ) -> impl Future<Output = Result<(), ErroChamada>> + Send;
}

/// Mídia do cliente pronta para a sala (MX-43): quem assina, o evento e o conteúdo do blob.
pub struct EnvioMidia {
    /// Conversa dona do anexo (a sala é dela).
    pub conversa: ConversationId,
    /// Usuário virtual do cliente: assina a subida e o evento.
    pub remetente: Fantasma,
    /// O que virou o anexo: `m.image`/`m.file`, nome, tipo, tamanho e `txnId`.
    pub midia: Midia,
    /// Bytes do blob, já lidos e validados no limite.
    pub conteudo: Vec<u8>,
}

/// O que o Synapse devolveu quando a mídia entrou na sala: o evento publicado e o arquivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiaPublicada {
    /// Id do evento de mídia (`m.image`/`m.file`) na sala.
    pub evento: String,
    /// `mxc://servidor/id` do arquivo que subiu.
    pub mxc: String,
}

/// Aviso em texto na sala de uma mídia que não entrou (MX-43).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvisoNaSala {
    /// Conversa dona do anexo.
    pub conversa: ConversationId,
    /// Quem assina o aviso: o fantasma do agente, dono da sala (como na MX-19).
    pub remetente: Fantasma,
    /// `txnId` do aviso: o mesmo anexo e o mesmo motivo não duplicam o aviso.
    pub txn_id: String,
    /// Motivo curto, que escolhe o texto e o código de log.
    pub motivo: MotivoDeRecusa,
}

/// Destino de verdade: a provisão de sala (MX-11) e o cliente da API do Matrix (MX-09).
pub struct DestinoReal {
    cliente: ClienteMatrix,
    provisionador: Provisionador<PortaSalasStore>,
    /// Banco do Harness: a leitura do departamento da conversa depois da criação da sala.
    store: Arc<Store>,
    /// Espaços por departamento (MX-12). `None` nos testes antigos, que só olham a sala.
    espacos: Option<Espacos<PortaEspacosStore>>,
    /// Números do módulo (MX-26): latência por operação e salas que ficaram prontas.
    metricas: Arc<MetricasMatrix>,
}

impl DestinoReal {
    /// Monta o destino a partir da seção `[matrix]` já validada e do cliente dela.
    pub fn montar(
        store: Arc<Store>,
        cliente: ClienteMatrix,
        config: &MatrixConfig,
    ) -> Result<Self, ErroFantasmas> {
        let provisionador =
            Provisionador::da_config(cliente.clone(), PortaSalasStore(Arc::clone(&store)), config)?;
        Ok(Self {
            cliente,
            provisionador,
            store,
            espacos: None,
            metricas: MetricasMatrix::descartaveis(),
        })
    }

    /// Liga os espaços por departamento (MX-12) ao espelho.
    ///
    /// Com eles, a sala entra no espaço **ao ser criada**, sem esperar a próxima alteração da
    /// conversa: o laço de estado (MX-15) pode ter visto esta conversa antes de a sala existir e,
    /// sem isso, a sala ficaria fora do espaço até o cliente escrever de novo.
    pub fn com_espacos(mut self, espacos: Espacos<PortaEspacosStore>) -> Self {
        self.espacos = Some(espacos);
        self
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }
    /// Põe a sala recém-criada no espaço do departamento dela (MX-12).
    ///
    /// Repetir não escreve nada: quando os convites da passagem fecharam, o vínculo fica gravado e a
    /// passagem seguinte sai por `JaPosicionada`. Convite pendente não grava o vínculo (regra da
    /// MX-12), e a próxima passagem do laço de estado completa o que faltou.
    async fn posicionar_recem_criada(&self, dados: &ConversaParaSala, sala: &str) {
        let Some(espacos) = &self.espacos else {
            return;
        };
        let departamento = match self.departamento_da_conversa(dados) {
            Ok(departamento) => departamento,
            Err(motivo) => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    motivo,
                    "não consegui ler o departamento da conversa; a sala fica sem espaço por ora"
                );
                return;
            }
        };
        let Some(departamento) = departamento else {
            return;
        };
        let agente = match self.provisionador.fantasma_do_agente(dados).await {
            Ok(agente) => agente,
            Err(_) => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    motivo = "fantasma_do_agente_indisponivel",
                    "sala criada mas não posicionada no espaço do departamento"
                );
                return;
            }
        };
        let pedido = SalaParaEspaco {
            org: dados.org,
            conversa: dados.conversa,
            sala: sala.to_owned(),
            agente,
            departamento: Some(departamento),
            // Ligar um espaço que já existe no Element é escolha do operador (MX-22/25); aqui o
            // módulo usa (ou cria) o espaço do departamento.
            existente: None,
        };
        let inicio = Instant::now();
        let resultado = espacos.posicionar(&pedido).await;
        self.metricas
            .latencia_synapse_segundos
            .observe(inicio.elapsed().as_secs_f64());
        match resultado {
            Ok(Posicao::Posicionada {
                convites_falhos, ..
            }) if convites_falhos > 0 => {
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    convites_falhos,
                    motivo = "convite_pendente_no_espaco",
                    "sala criada e posta no espaço, mas com convite pendente"
                );
            }
            Ok(_) => {
                tracing::info!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    "sala criada já dentro do espaço do departamento"
                );
            }
            Err(erro) => {
                match erro.classificacao() {
                    Classificacao::Transitoria => self.metricas.erros_transitorios.increment(1),
                    Classificacao::Definitiva => self.metricas.erros_definitivos.increment(1),
                }
                tracing::warn!(
                    target: "poph_server::matrix_espelho",
                    org = %dados.org,
                    conversa = %dados.conversa,
                    motivo = "sala_nao_posicionada_no_espaco",
                    "sala criada mas o espaço do departamento não aceitou a sala; tenta na próxima alteração da conversa"
                );
            }
        }
    }

    /// Departamento que manda a sala para um espaço (P3): o alvo da conversa; senão o padrão do
    /// agente; senão nenhum. O nome vem do banco, porque é ele que nomeia o espaço criado.
    fn departamento_da_conversa(
        &self,
        dados: &ConversaParaSala,
    ) -> Result<Option<DepartamentoEspaco>, &'static str> {
        let conversa = self
            .store
            .conversas()
            .get(dados.org, dados.conversa)
            .map_err(|_| "conversa_indisponivel")?;
        let padrao = self
            .store
            .agents()
            .get(dados.org, conversa.agent_id)
            .ok()
            .and_then(|agente| agente.default_department);
        let Some(escolhido) = resolver_departamento(conversa.target_department, padrao) else {
            return Ok(None);
        };
        match self.store.departments().obter(dados.org, escolhido) {
            Ok(departamento) => Ok(Some(DepartamentoEspaco {
                id: departamento.id,
                nome: departamento.name,
            })),
            Err(_) => Err("departamento_indisponivel"),
        }
    }
}

impl DestinoMatrix for DestinoReal {
    async fn garantir_sala(&self, dados: &ConversaParaSala) -> Result<String, ErroSala> {
        let inicio = Instant::now();
        let resultado = self.provisionador.provisionar(dados).await;
        self.metricas
            .latencia_synapse_segundos
            .observe(inicio.elapsed().as_secs_f64());
        let pronta = resultado?;
        if !pronta.ja_estava_pronta {
            self.metricas.salas_prontas.increment(1);
            // Sala recém-criada: entra no espaço do departamento agora. Falha aqui não derruba a
            // criação (a sala existe e as mensagens podem ser espelhadas); o laço de estado tenta
            // de novo na próxima alteração da conversa.
            self.posicionar_recem_criada(dados, &pronta.room_id).await;
        }
        Ok(pronta.room_id)
    }

    async fn garantir_aparelho(
        &self,
        dados: &ConversaParaSala,
        sala: &str,
        aparelho: &Fantasma,
    ) -> Result<(), ErroSala> {
        self.provisionador
            .garantir_aparelho_na_sala(dados, sala, aparelho)
            .await
    }

    async fn enviar(&self, sala: &str, envio: &Envio) -> Result<(), ErroChamada> {
        let corpo = json!({ "msgtype": envio.msgtype, "body": envio.corpo });
        let inicio = Instant::now();
        let resultado = self
            .cliente
            .enviar_evento(
                envio.remetente.user_id(),
                sala,
                envio.tipo_evento,
                &envio.txn_id,
                &corpo,
            )
            .await
            .map(|_| ());
        self.metricas
            .latencia_synapse_segundos
            .observe(inicio.elapsed().as_secs_f64());
        resultado
    }

    async fn enviar_midia(
        &self,
        sala: &str,
        envio: &EnvioMidia,
    ) -> Result<MidiaPublicada, ErroChamada> {
        let inicio = Instant::now();
        let resultado = midia::enviar(
            &self.cliente,
            &envio.remetente,
            sala,
            &envio.midia,
            &envio.conteudo,
        )
        .await
        .map(|publicada| MidiaPublicada {
            evento: publicada.evento,
            mxc: publicada.mxc,
        });
        self.metricas
            .latencia_synapse_segundos
            .observe(inicio.elapsed().as_secs_f64());
        resultado
    }

    async fn avisar(&self, sala: &str, aviso: &AvisoNaSala) -> Result<(), ErroChamada> {
        let corpo = midia::conteudo_do_aviso(aviso.motivo);
        let inicio = Instant::now();
        // `m.notice`: o classificador da MX-17 ignora aviso, então o aviso não volta como resposta.
        let resultado = self
            .cliente
            .enviar_evento(
                aviso.remetente.user_id(),
                sala,
                formato::TIPO_EVENTO,
                &aviso.txn_id,
                &corpo,
            )
            .await
            .map(|_| ());
        self.metricas
            .latencia_synapse_segundos
            .observe(inicio.elapsed().as_secs_f64());
        resultado
    }
}

/// Porta de persistência da provisão (MX-11) sobre o repositório `matrix` do store.
pub struct PortaSalasStore(pub Arc<Store>);

impl PortaSalas for PortaSalasStore {
    fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> impl Future<Output = Result<Option<SalaGravada>, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            let sala = store.matrix().sala(org, conversa).map_err(erro_porta)?;
            Ok(sala.map(|sala| SalaGravada {
                room_id: sala.room_id,
                estado: estado_gravado(sala.estado),
            }))
        }
    }

    fn garantir_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        room_id: &str,
    ) -> impl Future<Output = Result<SalaGravada, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        let room_id = room_id.to_owned();
        async move {
            let sala = store
                .matrix()
                .garantir_sala(org, conversa, &room_id, None)
                .map_err(erro_porta)?;
            Ok(SalaGravada {
                room_id: sala.room_id,
                estado: estado_gravado(sala.estado),
            })
        }
    }

    fn marcar_pronta(
        &self,
        org: OrgId,
        conversa: ConversationId,
        nome_espelhado: Option<&str>,
    ) -> impl Future<Output = Result<SalaGravada, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        let nome = nome_espelhado.map(str::to_owned);
        async move {
            let sala = store
                .matrix()
                .marcar_sala_pronta_com_nome(org, conversa, nome.as_deref())
                .map_err(erro_porta)?;
            Ok(SalaGravada {
                room_id: sala.room_id,
                estado: estado_gravado(sala.estado),
            })
        }
    }

    fn teve_mensagens_antes(
        &self,
        org: OrgId,
        conversa: ConversationId,
        corte: i64,
    ) -> impl Future<Output = Result<bool, ErroPorta>> + Send {
        let store = Arc::clone(&self.0);
        async move {
            store
                .messages()
                .teve_mensagens_antes(org, conversa, corte)
                .map_err(erro_porta)
        }
    }
}

fn erro_porta(_: StoreError) -> ErroPorta {
    // Sem o texto do erro do banco: ele pode carregar valor de cliente.
    ErroPorta::nova("armazenamento do módulo Matrix indisponível")
}

fn estado_gravado(estado: EstadoSala) -> EstadoGravado {
    match estado {
        EstadoSala::Criando => EstadoGravado::Criando,
        EstadoSala::Pronta => EstadoGravado::Pronta,
    }
}

// ───────────────────────────── erros e relatório ─────────────────────────────

/// Falha que interrompe o ciclo inteiro (o banco não respondeu).
#[derive(Debug, thiserror::Error)]
pub enum ErroEspelho {
    /// O armazenamento falhou antes ou no meio do ciclo.
    #[error("armazenamento do espelho do Matrix falhou: {0}")]
    Armazenamento(#[from] StoreError),
}

impl ErroEspelho {
    /// Código curto para log. O texto do erro do banco nunca vai para o log: pode trazer SQL ou
    /// valores de linha.
    pub fn codigo(&self) -> &'static str {
        match self {
            Self::Armazenamento(_) => "armazenamento_indisponivel",
        }
    }
}

/// O que um ciclo fez. Só contagens e ids de org: nada de texto de cliente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatorioCiclo {
    /// Eventos de texto entregues ao Synapse neste ciclo.
    pub enviadas: u32,
    /// Eventos de mídia do cliente entregues ao Synapse neste ciclo (MX-43).
    pub midias_enviadas: u32,
    /// Avisos na sala de mídia que não entrou (arquivo grande, vazio, ilegível, recusado).
    pub avisos_de_midia: u32,
    /// Mensagens que não entram na sala por regra (resposta humana, texto vazio, ferramenta).
    pub nao_espelhadas: u32,
    /// Mensagens já espelhadas antes (pela posição do cursor ou pelo id dele).
    pub ja_espelhadas: u32,
    /// Mensagens de orgs com o módulo desligado (não geram chamada alguma).
    pub de_org_desligada: u32,
    /// Mensagens anteriores ao corte do backfill (`enabled_at`).
    pub antes_do_corte: u32,
    /// Mensagens isoladas em `matrix_falhas` neste ciclo (formato ou falha definitiva repetida).
    pub isoladas: u32,
    /// Mensagens que ficaram pendentes (falha transitória, ou definitiva ainda dentro do limite).
    pub pendentes: u32,
    /// Quantas vezes o cursor de uma org recuou para o ponto seguro (R5).
    pub recuos: u32,
    /// Páginas lidas.
    pub paginas: u32,
    /// Maior página vista: o teto de linhas em memória ao mesmo tempo.
    pub maior_pagina: u32,
}

// ───────────────────────────── estado interno ─────────────────────────────

/// Org ligada, com o corte do backfill e o cursor atual dela.
struct OrgLigada {
    /// Instante (ms) em que a org ligou o módulo (`enabled_at`); 0 quando não há.
    corte: i64,
    /// `(rowid, id)` da última posição espelhada da org.
    cursor: (u64, String),
}

/// Dados da conversa necessários para o formato e para a sala, lidos uma vez por ciclo.
#[derive(Clone)]
struct DadosConversa {
    /// Identificador do cliente (o `peer`, ou o id da conversa quando o canal não tem um).
    identificador: String,
    /// Id do agente, como texto.
    agente: String,
    /// Id da integração (instância da Evolution) da conversa, como texto (MX-42).
    integracao: Option<String>,
    /// Dados para a provisão da sala.
    sala: ConversaParaSala,
}

/// Situação de uma mensagem depois de processada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Estado {
    /// Resolvida: enviada, pulada por regra, ou isolada. O cursor pode passar dela.
    Resolvida,
    /// Pendente: o cursor não pode passar dela.
    Pendente,
}

/// Resultado de uma tentativa de envio dentro de um grupo.
enum Desfecho {
    Enviada,
    Pendente,
    /// Falha definitiva repetida o bastante: isolar com este motivo e esta contagem.
    Isolar {
        motivo: String,
        tentativas: u32,
    },
}

/// Que tipo de passo um desfecho fechou (só para contar cada coisa no seu contador).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassoFeito {
    /// Texto (ou legenda) da mensagem.
    Texto,
    /// Mídia do cliente (MX-43).
    Midia,
    /// Aviso em texto na sala de uma mídia que não entrou.
    Aviso,
}

/// Um passo da entrega de uma mensagem: o texto, a mídia ou o aviso de uma mídia que não entrou.
enum Passo {
    /// Texto da mensagem (a legenda, quando há mídia).
    Texto(Envio),
    /// Mídia do cliente pronta para subir, com o aviso que entra na sala se o envio falhar de vez.
    Midia {
        envio: EnvioMidia,
        aviso: AvisoNaSala,
    },
    /// Nada de mídia a subir: só o aviso na sala (arquivo grande, vazio, ilegível).
    Aviso(AvisoNaSala),
}

impl Passo {
    /// Tipo do passo, para o contador do relatório.
    fn tipo(&self) -> PassoFeito {
        match self {
            Self::Texto(_) => PassoFeito::Texto,
            Self::Midia { .. } => PassoFeito::Midia,
            Self::Aviso(_) => PassoFeito::Aviso,
        }
    }
}

/// Uma mensagem da página com os passos que ela vai entregar, em ordem.
struct ItemDoGrupo {
    /// Índice na página (o estado do cursor é por índice).
    indice: usize,
    /// Org dona.
    org: OrgId,
    /// Id da mensagem.
    id: MessageId,
    /// O que entregar, em ordem: mídia (MX-43) antes do texto.
    passos: Vec<Passo>,
}

/// Mensagens de uma conversa, em ordem, para serem entregues na mesma sala.
struct Grupo {
    dados: ConversaParaSala,
    /// Itens em ordem de `rowid`.
    itens: Vec<ItemDoGrupo>,
}

/// O que a entrega de um grupo devolveu para uma mensagem.
struct DesfechoDoGrupo {
    /// Índice na página.
    indice: usize,
    /// Org dona.
    org: OrgId,
    /// Id da mensagem.
    id: MessageId,
    /// Tipo do passo que fechou o desfecho.
    tipo: PassoFeito,
    /// O que aconteceu.
    desfecho: Desfecho,
}

/// O que sobrou de uma página: até onde o cursor pode andar e se algo ficou pendente.
struct ResultadoPagina {
    /// Última posição da sequência contígua resolvida.
    prefixo: Option<(u64, MessageId)>,
    /// Alguma linha ficou pendente: o ciclo para aqui.
    pendente: bool,
}

// ───────────────────────────── o laço ─────────────────────────────

/// O laço de espelhamento. Um por processo; o estado em memória é só o de falhas definitivas e o
/// de falhas seguidas, ambos pequenos.
pub struct EspelhoMatrix<D: DestinoMatrix> {
    store: Arc<Store>,
    destino: D,
    ids: IdsFantasmas,
    concorrencia: usize,
    intervalo: Duration,
    pagina: u32,
    /// Falhas definitivas seguidas por mensagem. Só guarda mensagens que estão falhando.
    definitivas: Mutex<HashMap<MessageId, u32>>,
    /// Quantos ciclos seguidos terminaram com pendência.
    falhas_seguidas: AtomicU32,
    /// Pedido de parada; o ciclo em andamento termina e o laço não começa outro.
    parar: tokio::sync::watch::Sender<bool>,
    /// Números do módulo (MX-26). Sem injeção, cai num registro descartável.
    metricas: Arc<MetricasMatrix>,
    /// Relógio em milissegundos, para medir a idade da mensagem pendente mais antiga.
    agora_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// Onde os blobs das orgs estão no disco (MX-43). `None` só em montagem de teste sem mídia:
    /// com mídia e sem pasta, o anexo vira aviso na sala em vez de sumir em silêncio.
    pasta_blobs: Option<PathBuf>,
}

impl<D: DestinoMatrix> EspelhoMatrix<D> {
    /// Monta o laço com a seção `[matrix]` já validada (prefixo, servidor, concorrência e intervalo).
    pub fn novo(
        store: Arc<Store>,
        destino: D,
        config: &MatrixConfig,
    ) -> Result<Self, ErroFantasmas> {
        let ids = IdsFantasmas::da_config(config)?;
        let (parar, _) = tokio::sync::watch::channel(false);
        Ok(Self {
            store,
            destino,
            ids,
            concorrencia: (config.concurrency as usize).max(1),
            intervalo: Duration::from_millis(config.interval_ms.max(1)),
            pagina: PAGINA,
            definitivas: Mutex::new(HashMap::new()),
            falhas_seguidas: AtomicU32::new(0),
            parar,
            metricas: MetricasMatrix::descartaveis(),
            agora_ms: Arc::new(relogio_do_sistema),
            pasta_blobs: None,
        })
    }

    /// Usa os números do processo (`Estado::metricas_matrix`) em vez de um registro descartável.
    pub fn com_metricas(mut self, metricas: Arc<MetricasMatrix>) -> Self {
        self.metricas = metricas;
        self
    }

    /// Troca o relógio do laço (testes medem o atraso sem esperar de verdade).
    pub fn com_relogio(mut self, agora_ms: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.agora_ms = agora_ms;
        self
    }

    /// Diz onde os blobs das orgs estão no disco, para a mídia do cliente (MX-43) ser lida.
    ///
    /// Quem liga o módulo chama isto com a pasta do `ConfigArquivos` do servidor. Sem a pasta, o
    /// laço não lê anexo nenhum: cada mídia vira um aviso na sala, e nenhuma mensagem fica presa.
    pub fn com_pasta_de_blobs(mut self, pasta: impl Into<PathBuf>) -> Self {
        self.pasta_blobs = Some(pasta.into());
        self
    }

    /// Pede a parada. O ciclo em andamento termina a página atual; depois o laço sai.
    pub fn pedir_parada(&self) {
        self.parar.send_replace(true);
    }

    /// Atraso antes do próximo ciclo: o intervalo normal, ou dobrando a cada ciclo com falha, até
    /// [`TETO_ATRASO`].
    pub fn atraso_apos(&self, falhas_seguidas: u32) -> Duration {
        if falhas_seguidas == 0 {
            return self.intervalo;
        }
        let fator = 1u32
            .checked_shl(falhas_seguidas.min(10))
            .unwrap_or(u32::MAX);
        self.intervalo.saturating_mul(fator).min(TETO_ATRASO)
    }

    /// Roda ciclos até [`EspelhoMatrix::pedir_parada`]. Erro de um ciclo é registrado e o laço
    /// segue no próximo.
    pub async fn executar(self: Arc<Self>) {
        let mut parar = self.parar.subscribe();
        loop {
            if *parar.borrow() {
                break;
            }
            let falhas = match self.ciclo().await {
                Ok(relatorio) => {
                    tracing::debug!(
                        target: "poph_server::matrix_espelho",
                        enviadas = relatorio.enviadas,
                        pendentes = relatorio.pendentes,
                        isoladas = relatorio.isoladas,
                        "ciclo do espelho do Matrix"
                    );
                    if relatorio.pendentes > 0 {
                        self.falhas_seguidas.fetch_add(1, Ordering::Relaxed) + 1
                    } else {
                        self.falhas_seguidas.store(0, Ordering::Relaxed);
                        0
                    }
                }
                Err(erro) => {
                    tracing::warn!(
                        target: "poph_server::matrix_espelho",
                        motivo = erro.codigo(),
                        "ciclo do espelho do Matrix falhou; segue no próximo"
                    );
                    self.falhas_seguidas.fetch_add(1, Ordering::Relaxed) + 1
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(self.atraso_apos(falhas)) => {}
                _ = parar.changed() => break,
            }
        }
    }

    /// Um ciclo completo: lê, decide, entrega e avança o cursor até a primeira pendência.
    pub async fn ciclo(&self) -> Result<RelatorioCiclo, ErroEspelho> {
        let mut relatorio = RelatorioCiclo::default();
        let ligadas = self.store.matrix().orgs_ligadas()?;
        if ligadas.is_empty() {
            self.metricas.atraso_cursor_segundos.set(0);
            return Ok(relatorio);
        }

        let mut orgs: HashMap<OrgId, OrgLigada> = HashMap::new();
        for estado in ligadas.iter().filter(|estado| estado.ligado) {
            let cursores = self.store.matrix().ler_cursores(estado.org_id)?;
            orgs.insert(
                estado.org_id,
                OrgLigada {
                    corte: estado.ligado_em.unwrap_or(0),
                    cursor: (cursores.mensagens, cursores.mensagens_id),
                },
            );
        }
        if orgs.is_empty() {
            self.metricas.atraso_cursor_segundos.set(0);
            return Ok(relatorio);
        }

        // R5: cursor à frente do maior rowid que existe (banco restaurado ou linhas apagadas).
        // Volta para o ponto seguro; o que vier depois é lido normalmente.
        let (maior, maior_id) = match self.store.messages().ultima_rowid()? {
            Some((rowid, id)) => (rowid, id.to_string()),
            None => (0, String::new()),
        };
        for (org, info) in orgs.iter_mut() {
            if info.cursor.0 > maior {
                self.store
                    .matrix()
                    .recuar_cursor_mensagens(*org, maior, &maior_id)?;
                info.cursor = (maior, maior_id.clone());
                relatorio.recuos += 1;
            }
        }

        // Lê a partir do menor cursor. `desde(x)` devolve rowid > x; com `x = cursor - 1` a linha
        // do próprio cursor entra, para o laço conferir o id dela.
        let menor = orgs.values().map(|info| info.cursor.0).min().unwrap_or(0);
        let mut posicao = menor.saturating_sub(1);

        let mut paginas = 0u32;
        while paginas < MAX_PAGINAS_POR_CICLO && !*self.parar.borrow() {
            let pagina = self.store.messages().desde(posicao, self.pagina)?;
            if pagina.is_empty() {
                break;
            }
            paginas += 1;
            relatorio.paginas += 1;
            relatorio.maior_pagina = relatorio.maior_pagina.max(pagina.len() as u32);

            let ultimo = pagina.last().map(|m| m.rowid).unwrap_or(posicao);
            let ultima_pagina = pagina.len() < self.pagina as usize;

            let resultado = self.processar_pagina(&orgs, pagina, &mut relatorio).await?;

            if let Some((rowid, id)) = &resultado.prefixo {
                let id_texto = id.to_string();
                for (org, info) in orgs.iter_mut() {
                    if *rowid > info.cursor.0 {
                        self.store
                            .matrix()
                            .avancar_mensagens_com_id(*org, *rowid, &id_texto)?;
                        info.cursor = (*rowid, id_texto.clone());
                    }
                }
            }

            if resultado.pendente {
                break;
            }
            posicao = ultimo;
            if ultima_pagina {
                break;
            }
        }
        self.medir_atraso(&orgs)?;
        Ok(relatorio)
    }

    /// Atualiza o atraso do cursor: a idade da primeira mensagem depois do cursor mais atrasado.
    /// Sem nada depois dele, o atraso é `0`. Assim o número sobe enquanto o Synapse recusa e volta
    /// a zero quando o laço alcança o fim.
    fn medir_atraso(&self, orgs: &HashMap<OrgId, OrgLigada>) -> Result<(), ErroEspelho> {
        let Some(menor) = orgs.values().map(|info| info.cursor.0).min() else {
            self.metricas.atraso_cursor_segundos.set(0);
            return Ok(());
        };
        let atraso_ms = match self.store.messages().desde(menor, 1)?.first() {
            Some(primeira) => ((self.agora_ms)() - primeira.criado_em).max(0),
            None => 0,
        };
        self.metricas.atraso_cursor_segundos.set(atraso_ms / 1000);
        Ok(())
    }

    /// Decide e entrega uma página, e devolve até onde ela resolveu.
    async fn processar_pagina(
        &self,
        orgs: &HashMap<OrgId, OrgLigada>,
        pagina: Vec<MensagemDesde>,
        relatorio: &mut RelatorioCiclo,
    ) -> Result<ResultadoPagina, ErroEspelho> {
        let mut estados = vec![Estado::Resolvida; pagina.len()];
        let mut grupos: Vec<Grupo> = Vec::new();
        let mut indice_grupo: HashMap<ConversationId, usize> = HashMap::new();
        // `None` quando a leitura da conversa falhou nesta página: as linhas dela ficam pendentes.
        let mut conversas: HashMap<ConversationId, Option<DadosConversa>> = HashMap::new();

        for (i, mensagem) in pagina.iter().enumerate() {
            let Some(org) = orgs.get(&mensagem.org_id) else {
                relatorio.de_org_desligada += 1;
                continue;
            };
            if mensagem.rowid < org.cursor.0
                || (mensagem.rowid == org.cursor.0 && mensagem.id.to_string() == org.cursor.1)
            {
                relatorio.ja_espelhadas += 1;
                continue;
            }
            if mensagem.criado_em < org.corte {
                relatorio.antes_do_corte += 1;
                continue;
            }

            let lida = conversas
                .entry(mensagem.conversation_id)
                .or_insert_with(|| {
                    self.dados_conversa(mensagem.org_id, mensagem.conversation_id, org.corte)
                        .ok()
                })
                .clone();
            let Some(dados) = lida else {
                // Não consegui ler a conversa agora: a linha fica pendente e volta no próximo ciclo.
                estados[i] = Estado::Pendente;
                continue;
            };

            // Mídia do cliente (MX-43): o anexo já está gravado; a mensagem vira mídia + legenda.
            // Sem saber os anexos, entregar só o texto esconderia a mídia: a linha fica pendente.
            let anexos = match self.anexos_do_cliente(mensagem) {
                Ok(anexos) => anexos,
                Err(_) => {
                    estados[i] = Estado::Pendente;
                    continue;
                }
            };
            let mut passos: Vec<Passo> = Vec::new();
            let mut midia_invalida = false;
            for anexo in &anexos {
                match self.passo_de_midia(mensagem.org_id, &dados, anexo).await {
                    Ok(passo) => passos.push(passo),
                    // Sem o fantasma do agente não há nem mídia nem aviso: isola como formato.
                    Err(_) => {
                        midia_invalida = true;
                        break;
                    }
                }
            }
            if midia_invalida {
                passos.clear();
            }

            let decisao = formato::decidir(
                &self.ids,
                &MensagemParaEspelho {
                    id: mensagem.id,
                    conversa: mensagem.conversation_id,
                    origem: origem_de(mensagem.origem),
                    sentido: sentido_de(mensagem.direcao),
                    papel: papel_de(mensagem.papel),
                    texto: &mensagem.texto,
                    identificador_cliente: &dados.identificador,
                    id_agente: &dados.agente,
                    id_integracao: dados.integracao.as_deref(),
                },
            );
            let motivo_de_falha = match decisao {
                // Com mídia já montada, texto vazio é só legenda vazia: a mídia vai sozinha.
                Decisao::NaoEspelhar(MotivoNaoEspelhar::TextoVazio) if !passos.is_empty() => None,
                Decisao::NaoEspelhar(_) => {
                    passos.clear();
                    relatorio.nao_espelhadas += 1;
                    None
                }
                Decisao::Erro(_) => {
                    passos.clear();
                    Some(if midia_invalida {
                        MOTIVO_MIDIA
                    } else {
                        MOTIVO_FORMATO
                    })
                }
                Decisao::Espelhar(envio) => {
                    passos.push(Passo::Texto(envio));
                    None
                }
            };
            if let Some(motivo) = motivo_de_falha {
                match self
                    .store
                    .matrix()
                    .registrar_falha(mensagem.org_id, mensagem.id, motivo, 1)
                {
                    Ok(_) => {
                        relatorio.isoladas += 1;
                        self.metricas.mensagens_isoladas.increment(1);
                        tracing::warn!(
                            target: "poph_server::matrix_espelho",
                            org = %mensagem.org_id,
                            conversa = %mensagem.conversation_id,
                            motivo,
                            "mensagem fora do formato do espelho: isolada em matrix_falhas"
                        );
                    }
                    Err(_) => estados[i] = Estado::Pendente,
                }
                continue;
            }
            if passos.is_empty() {
                continue;
            }
            let posicao = match indice_grupo.get(&mensagem.conversation_id) {
                Some(posicao) => *posicao,
                None => {
                    grupos.push(Grupo {
                        dados: dados.sala.clone(),
                        itens: Vec::new(),
                    });
                    indice_grupo.insert(mensagem.conversation_id, grupos.len() - 1);
                    grupos.len() - 1
                }
            };
            grupos[posicao].itens.push(ItemDoGrupo {
                indice: i,
                org: mensagem.org_id,
                id: mensagem.id,
                passos,
            });
        }

        // Entrega: salas em paralelo, mensagens da mesma sala em sequência.
        let desfechos: Vec<Vec<DesfechoDoGrupo>> = futures::stream::iter(grupos)
            .map(|grupo| self.entregar_grupo(grupo))
            .buffer_unordered(self.concorrencia)
            .collect()
            .await;

        for desfecho in desfechos.into_iter().flatten() {
            let DesfechoDoGrupo {
                indice,
                org,
                id,
                tipo,
                desfecho,
            } = desfecho;
            match desfecho {
                Desfecho::Enviada => {
                    estados[indice] = Estado::Resolvida;
                    match tipo {
                        PassoFeito::Texto => {
                            relatorio.enviadas += 1;
                            self.metricas.mensagens_espelhadas.increment(1);
                        }
                        PassoFeito::Midia => {
                            relatorio.midias_enviadas += 1;
                            self.metricas.midias_espelhadas.increment(1);
                        }
                        PassoFeito::Aviso => relatorio.avisos_de_midia += 1,
                    }
                }
                Desfecho::Pendente => {
                    estados[indice] = Estado::Pendente;
                }
                Desfecho::Isolar { motivo, tentativas } => match self
                    .store
                    .matrix()
                    .registrar_falha(org, id, &motivo, tentativas)
                {
                    Ok(_) => {
                        estados[indice] = Estado::Resolvida;
                        relatorio.isoladas += 1;
                        self.metricas.mensagens_isoladas.increment(1);
                    }
                    Err(_) => estados[indice] = Estado::Pendente,
                },
            }
        }

        // Marca-d'água: até a primeira linha pendente, tudo resolvido.
        let mut prefixo = None;
        let mut pendente = false;
        for (i, mensagem) in pagina.iter().enumerate() {
            if estados[i] == Estado::Pendente {
                pendente = true;
                break;
            }
            prefixo = Some((mensagem.rowid, mensagem.id));
        }
        relatorio.pendentes += estados.iter().filter(|e| **e == Estado::Pendente).count() as u32;

        Ok(ResultadoPagina { prefixo, pendente })
    }

    /// Entrega as mensagens de uma sala em ordem. Para na primeira falha: o resto fica pendente.
    async fn entregar_grupo(&self, grupo: Grupo) -> Vec<DesfechoDoGrupo> {
        let mut saida = Vec::with_capacity(grupo.itens.len());
        let mut itens = grupo.itens.into_iter();

        let conversa = grupo.dados.conversa;
        let agente_da_conversa = grupo.dados.agente.to_string();
        self.metricas.tentativas.increment(1);
        let sala = match self.destino.garantir_sala(&grupo.dados).await {
            Ok(sala) => sala,
            Err(erro) => {
                if chamada_da_sala(&erro).is_some_and(eh_429) {
                    self.metricas.respostas_429.increment(1);
                }
                // A falha da sala é cobrada no primeiro passo da primeira mensagem do grupo.
                let motivo = MOTIVO_SALA.to_owned();
                let primeiro = itens.next();
                if let Some(item) = primeiro {
                    let tipo = item
                        .passos
                        .first()
                        .map(Passo::tipo)
                        .unwrap_or(PassoFeito::Texto);
                    let desfecho = self.desfecho_de(
                        item.org,
                        conversa,
                        &item.id,
                        erro.classificacao(),
                        motivo,
                    );
                    saida.push(DesfechoDoGrupo {
                        indice: item.indice,
                        org: item.org,
                        id: item.id,
                        tipo,
                        desfecho,
                    });
                }
                for item in itens {
                    saida.push(DesfechoDoGrupo {
                        indice: item.indice,
                        org: item.org,
                        id: item.id,
                        tipo: PassoFeito::Texto,
                        desfecho: Desfecho::Pendente,
                    });
                }
                return saida;
            }
        };

        // O usuário do aparelho entra na sala uma vez por passagem, e só se o grupo tem mensagem dele.
        let mut aparelho_na_sala = false;
        while let Some(item) = itens.next() {
            for passo in item.passos {
                self.metricas.tentativas.increment(1);
                let tipo = passo.tipo();
                // Texto do aparelho precisa do fantasma do aparelho dentro da sala antes de falar
                // nela (MX-42). Mídia não vem do aparelho nesta fase.
                if let Passo::Texto(envio) = &passo
                    && envio.origem == OrigemMensagem::Dispositivo
                    && !aparelho_na_sala
                {
                    match self
                        .destino
                        .garantir_aparelho(&grupo.dados, &sala, &envio.remetente)
                        .await
                    {
                        Ok(()) => aparelho_na_sala = true,
                        Err(erro) => {
                            // Mesma regra dos outros fantasmas: classifica, repete na passagem
                            // seguinte (transitória) ou isola após as tentativas (definitiva). A
                            // mensagem continua gravada no Harness; só a entrega à sala espera.
                            let desfecho = self.desfecho_de(
                                item.org,
                                conversa,
                                &item.id,
                                erro.classificacao(),
                                MOTIVO_APARELHO.to_owned(),
                            );
                            saida.push(DesfechoDoGrupo {
                                indice: item.indice,
                                org: item.org,
                                id: item.id,
                                tipo,
                                desfecho,
                            });
                            self.resto_pendente(&mut itens, &mut saida);
                            return saida;
                        }
                    }
                }

                let resultado = match &passo {
                    Passo::Texto(envio) => self.destino.enviar(&sala, envio).await.map(|()| None),
                    Passo::Midia { envio, .. } => {
                        self.destino.enviar_midia(&sala, envio).await.map(Some)
                    }
                    Passo::Aviso(aviso) => self.destino.avisar(&sala, aviso).await.map(|()| None),
                };

                match resultado {
                    Ok(publicada) => {
                        // MX-46: a mídia que entrou na sala fica registrada com o evento e o
                        // `mxc`, para a expiração saber o que redigir e apagar depois do prazo.
                        if let (Passo::Midia { envio, .. }, Some(publicada)) = (&passo, &publicada)
                        {
                            self.registrar_midia(
                                item.org,
                                &agente_da_conversa,
                                &sala,
                                envio,
                                publicada,
                            );
                        }
                        self.esquecer(&item.id);
                        saida.push(DesfechoDoGrupo {
                            indice: item.indice,
                            org: item.org,
                            id: item.id,
                            tipo,
                            desfecho: Desfecho::Enviada,
                        });
                    }
                    Err(erro) => {
                        if eh_429(&erro) {
                            self.metricas.respostas_429.increment(1);
                        }
                        let motivo = motivo_de_envio(&erro);
                        let desfecho = self.desfecho_de(
                            item.org,
                            conversa,
                            &item.id,
                            erro.classificacao(),
                            motivo,
                        );
                        // Mídia que não entra de vez deixa o aviso na sala (MX-43): o atendente
                        // precisa saber que o cliente mandou um arquivo que não apareceu.
                        if let (Desfecho::Isolar { .. }, Passo::Midia { aviso, .. }) =
                            (&desfecho, &passo)
                        {
                            self.escrever_aviso(&sala, aviso).await;
                        }
                        saida.push(DesfechoDoGrupo {
                            indice: item.indice,
                            org: item.org,
                            id: item.id,
                            tipo,
                            desfecho,
                        });
                        self.resto_pendente(&mut itens, &mut saida);
                        return saida;
                    }
                }
            }
        }
        saida
    }

    /// Marca como pendente o que sobrou do item atual (passos) e das mensagens seguintes do grupo.
    fn resto_pendente(
        &self,
        itens: &mut std::vec::IntoIter<ItemDoGrupo>,
        saida: &mut Vec<DesfechoDoGrupo>,
    ) {
        for item in itens.by_ref() {
            saida.push(DesfechoDoGrupo {
                indice: item.indice,
                org: item.org,
                id: item.id,
                tipo: PassoFeito::Texto,
                desfecho: Desfecho::Pendente,
            });
        }
    }

    /// Escreve o aviso de mídia na sala, sem deixar a falha dele mudar o desfecho da mensagem.
    async fn escrever_aviso(&self, sala: &str, aviso: &AvisoNaSala) {
        if let Err(erro) = self.destino.avisar(sala, aviso).await {
            tracing::warn!(
                target: "poph_server::matrix_espelho",
                conversa = %aviso.conversa,
                motivo = aviso.motivo.codigo(),
                classe = %erro.classificacao(),
                "aviso de mídia recusada não entrou na sala; a mensagem foi isolada do mesmo jeito"
            );
        }
    }

    /// Decide o desfecho de uma falha: transitória fica pendente; definitiva conta a tentativa e,
    /// ao chegar em [`TENTATIVAS_DEFINITIVAS`], isola a mensagem.
    fn desfecho_de(
        &self,
        org: OrgId,
        conversa: ConversationId,
        id: &MessageId,
        classificacao: Classificacao,
        motivo: String,
    ) -> Desfecho {
        match classificacao {
            Classificacao::Transitoria => {
                self.metricas.erros_transitorios.increment(1);
                tracing::debug!(
                    target: "poph_server::matrix_espelho",
                    org = %org,
                    conversa = %conversa,
                    motivo = %motivo,
                    classe = "transitoria",
                    "falha no espelho do Matrix: a mensagem fica pendente"
                );
                Desfecho::Pendente
            }
            Classificacao::Definitiva => {
                self.metricas.erros_definitivos.increment(1);
                let tentativas = {
                    let mut mapa = self
                        .definitivas
                        .lock()
                        .unwrap_or_else(|veneno| veneno.into_inner());
                    let contagem = mapa.entry(*id).or_insert(0);
                    *contagem += 1;
                    *contagem
                };
                if tentativas >= TENTATIVAS_DEFINITIVAS {
                    self.esquecer(id);
                    tracing::warn!(
                        target: "poph_server::matrix_espelho",
                        org = %org,
                        conversa = %conversa,
                        motivo = %motivo,
                        tentativas,
                        classe = "definitiva",
                        "falha definitiva repetida no espelho do Matrix: mensagem isolada"
                    );
                    Desfecho::Isolar { motivo, tentativas }
                } else {
                    tracing::info!(
                        target: "poph_server::matrix_espelho",
                        org = %org,
                        conversa = %conversa,
                        motivo = %motivo,
                        tentativas,
                        classe = "definitiva",
                        "falha definitiva no espelho do Matrix: nova tentativa no próximo ciclo"
                    );
                    Desfecho::Pendente
                }
            }
        }
    }

    fn esquecer(&self, id: &MessageId) {
        let mut mapa = self
            .definitivas
            .lock()
            .unwrap_or_else(|veneno| veneno.into_inner());
        mapa.remove(id);
    }

    // ───────────────────────────── mídia do cliente (MX-43) ─────────────────────────────

    /// Anexos que a mensagem vai levar à sala, na ordem em que chegaram.
    ///
    /// Só mensagem de entrada do cliente tem mídia espelhada: mídia do agente é do caminho de saída
    /// (MX-44), mídia de origem humana veio do próprio Matrix e mídia do aparelho não existe nesta
    /// fase. `Err` quando a lista de anexos não lê ou o banco falha — a linha fica pendente, para
    /// não entregar o texto escondendo a mídia.
    fn anexos_do_cliente(
        &self,
        mensagem: &MensagemDesde,
    ) -> Result<Vec<AttachmentRecord>, StoreError> {
        if mensagem.origem != Origin::Agente
            || mensagem.papel != Role::User
            || mensagem.direcao != Direction::Inbound
        {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = serde_json::from_str(&mensagem.anexos_json).map_err(|erro| {
            StoreError::Internal(format!(
                "lista de anexos da mensagem não lê como JSON: {erro}"
            ))
        })?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.store.messages().anexos(mensagem.org_id, mensagem.id)
    }

    /// Um passo de mídia de um anexo: o envio pronto, ou o aviso de por que ele não entra.
    ///
    /// `Err` só quando nem o fantasma do agente nem o do cliente podem ser montados — aí a mensagem
    /// é isolada como formato inválido, porque não há como avisar na sala.
    async fn passo_de_midia(
        &self,
        org: OrgId,
        dados: &DadosConversa,
        anexo: &AttachmentRecord,
    ) -> Result<Passo, ()> {
        let id_anexo = anexo.id.to_string();
        let remetente = self.ids.cliente(&dados.identificador).map_err(|_| ())?;
        let decidida = midia::decidir(&AnexoParaMidia {
            id: &id_anexo,
            nome: &anexo.name,
            mime: &anexo.mime,
            tamanho_bytes: anexo.size_bytes,
            fonte: &dados.sala.canal,
        });
        let decidida = match decidida {
            Ok(decidida) => decidida,
            Err(motivo) => {
                return Ok(Passo::Aviso(self.aviso_de_midia(dados, &id_anexo, motivo)?));
            }
        };
        let conteudo = match self.ler_blob(org, anexo).await {
            Ok(conteudo) => conteudo,
            Err(motivo) => {
                return Ok(Passo::Aviso(self.aviso_de_midia(dados, &id_anexo, motivo)?));
            }
        };
        Ok(Passo::Midia {
            envio: EnvioMidia {
                conversa: dados.sala.conversa,
                remetente,
                midia: decidida,
                conteudo,
            },
            aviso: self.aviso_de_midia(dados, &id_anexo, MotivoDeRecusa::Rejeitada)?,
        })
    }

    /// Grava a mídia entregue em `matrix_midia` (MX-46), com o evento e o `mxc` que ficaram na sala.
    ///
    /// Falha de gravação **não** muda o desfecho da mensagem: o evento já está na sala, e repetir o
    /// envio não desfaria nada. O que se perde é a expiração daquele arquivo, e é isso que o log
    /// registra com um código curto.
    fn registrar_midia(
        &self,
        org: OrgId,
        agente: &str,
        sala: &str,
        envio: &EnvioMidia,
        publicada: &MidiaPublicada,
    ) {
        let midia = &envio.midia;
        let registro = NovaMidia {
            id: midia.id_anexo.parse().unwrap_or_else(|_| {
                // O id vem do banco (é `attachments.id`); se não for um id do Harness, a gravação
                // falha na validação e é registrada como as demais.
                poph_types::AttachmentId::generate()
            }),
            conversa: envio.conversa,
            agente: agente.to_owned(),
            sala: sala.to_owned(),
            evento: publicada.evento.clone(),
            mxc: publicada.mxc.clone(),
            nome: midia.nome.clone(),
            mime: midia.mime.clone(),
            categoria: if midia.msgtype == midia::MSGTYPE_IMAGEM {
                CategoriaMidia::Imagem
            } else {
                CategoriaMidia::Documento
            },
            tamanho_bytes: midia.tamanho_bytes,
            enviada_em: (self.agora_ms)(),
        };
        if let Err(erro) = self.store.matrix_midia().registrar(org, &registro) {
            tracing::warn!(
                target: "poph_server::matrix_espelho",
                org = %org,
                conversa = %envio.conversa,
                motivo = "midia_nao_registrada",
                "mídia entrou na sala mas não foi registrada para expirar: o arquivo fica no Synapse"
            );
            let _ = erro;
        }
    }

    /// Aviso na sala de uma mídia que não entrou. Quem assina é o fantasma do agente, dono da sala.
    fn aviso_de_midia(
        &self,
        dados: &DadosConversa,
        id_anexo: &str,
        motivo: MotivoDeRecusa,
    ) -> Result<AvisoNaSala, ()> {
        let remetente = self.ids.agente(&dados.agente).map_err(|_| ())?;
        Ok(AvisoNaSala {
            conversa: dados.sala.conversa,
            remetente,
            txn_id: midia::txn_do_aviso(id_anexo, motivo),
            motivo,
        })
    }

    /// Lê os bytes do blob do anexo, com o teto da mídia.
    ///
    /// O tamanho da linha já foi conferido em `midia::decidir`; aqui o teto é o mesmo, de rede de
    /// segurança contra uma linha que minta. Arquivo ausente ou ilegível vira [`MotivoDeRecusa`],
    /// nunca pânico e nunca pendência eterna.
    async fn ler_blob(
        &self,
        org: OrgId,
        anexo: &AttachmentRecord,
    ) -> Result<Vec<u8>, MotivoDeRecusa> {
        let Some(pasta) = self.pasta_blobs.clone() else {
            return Err(MotivoDeRecusa::Ilegivel);
        };
        let sha256 = anexo.sha256.clone();
        let limite = midia::LIMITE_MIDIA_BYTES;
        // Leitura de disco é bloqueante: sai da tarefa do laço.
        tokio::task::spawn_blocking(move || {
            poph_files::blob::ler(&pasta, org, &sha256, limite)
                .map_err(|_| MotivoDeRecusa::Ilegivel)
        })
        .await
        .map_err(|_| MotivoDeRecusa::Ilegivel)?
    }

    /// Dados da conversa para formato e sala. Canal sem `peer` usa o id da conversa como cliente.
    fn dados_conversa(
        &self,
        org: OrgId,
        conversa: ConversationId,
        corte: i64,
    ) -> Result<DadosConversa, StoreError> {
        let conversa_lida = self.store.conversas().get(org, conversa)?;
        // Mesma regra que a listagem usa para `external_peer` (RP-02): um só lugar, na store.
        let identificador = conversa_lida.identificador_cliente();
        let nome_agente = self
            .store
            .agents()
            .get(org, conversa_lida.agent_id)
            .ok()
            .map(|agente| agente.name);
        let agente: AgentId = conversa_lida.agent_id;
        let sala = ConversaParaSala {
            org,
            conversa,
            cliente: identificador.clone(),
            // Nome que o WhatsApp mandou (`pushName`, MX-32), já normalizado no banco. Sem ele, a
            // sala e o fantasma do cliente caem no identificador.
            nome_cliente: conversa_lida.customer_name.clone(),
            agente,
            nome_agente,
            canal: conversa_lida.canal.clone(),
            departamento: conversa_lida.target_department,
            fechada: conversa_lida.status == ConversationStatus::Closed,
            corte_backfill: Some(corte),
        };
        Ok(DadosConversa {
            identificador,
            agente: agente.to_string(),
            integracao: conversa_lida.integration_id.map(|id| id.to_string()),
            sala,
        })
    }
}

// ───────────────────────────── conversões e motivos ─────────────────────────────

fn origem_de(origem: Origin) -> OrigemMensagem {
    match origem {
        Origin::Agente => OrigemMensagem::Agente,
        Origin::Human => OrigemMensagem::Humana,
        Origin::Device => OrigemMensagem::Dispositivo,
    }
}

fn sentido_de(direcao: Direction) -> Sentido {
    match direcao {
        Direction::Inbound => Sentido::Entrada,
        Direction::Outbound => Sentido::Saida,
    }
}

fn papel_de(papel: Role) -> PapelMensagem {
    match papel {
        Role::User => PapelMensagem::Cliente,
        Role::Assistant => PapelMensagem::Agente,
        Role::Tool | Role::System => PapelMensagem::Outro,
    }
}

/// Código curto do erro do Synapse, quando ele existe e é seguro para gravar.
fn motivo_de_envio(erro: &ErroChamada) -> String {
    match erro {
        ErroChamada::Resposta {
            errcode: Some(codigo),
            ..
        } if codigo_seguro(codigo) => codigo.clone(),
        _ => MOTIVO_ENVIO.to_owned(),
    }
}

/// Chamada ao Synapse por trás de um erro de sala, quando o erro veio de uma chamada.
fn chamada_da_sala(erro: &ErroSala) -> Option<&ErroChamada> {
    match erro {
        ErroSala::Chamada(chamada) | ErroSala::Fantasmas(ErroFantasmas::Chamada(chamada)) => {
            Some(chamada)
        }
        _ => None,
    }
}

/// `429` do Synapse (limite de taxa).
fn eh_429(erro: &ErroChamada) -> bool {
    matches!(erro, ErroChamada::Resposta { status: 429, .. })
}

/// Relógio do sistema em milissegundos (o mesmo carimbo que o Harness grava nas mensagens).
pub(crate) fn relogio_do_sistema() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duracao| i64::try_from(duracao.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Só letras, números e `_`, até o limite da coluna: o que o Synapse manda como `errcode`.
fn codigo_seguro(codigo: &str) -> bool {
    !codigo.is_empty()
        && codigo.len() <= LIMITE_MOTIVO
        && codigo
            .chars()
            .all(|caractere| caractere.is_ascii_alphanumeric() || caractere == '_')
}
