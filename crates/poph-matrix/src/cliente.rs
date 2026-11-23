//! Cliente da API do Matrix dentro do módulo (MX-09).
//!
//! Fala com a API cliente do Synapse como Application Service. Regras que valem para todas as
//! chamadas:
//!
//! - **O `as_token` vai só no cabeçalho `Authorization: Bearer …`.** Nunca na URL, no log, no
//!   `Debug`, no `Display` ou na mensagem de erro (o tipo [`Segredo`] garante a máscara).
//! - **Disfarce de usuário** (`?user_id=@…`) em toda chamada que age como um usuário virtual.
//!   Sem ele, o Synapse trataria a chamada como o próprio bot do Application Service.
//! - **Uma tentativa por chamada**, com uma única exceção: `429 M_LIMIT_EXCEEDED` com
//!   `retry_after_ms` é esperado e repetido **uma vez**. Qualquer outra repetição é decisão do
//!   laço do módulo, que lê [`ErroChamada::classificacao`].
//! - **Prazo fixo por chamada.** [`TEMPO_LIMITE_PADRAO`] (10 s) é o padrão documentado: o
//!   `[matrix]` do `poph.toml` não tem chave de prazo e `concorrencia`/`intervalo_ms` são do laço,
//!   não do cliente. Quem precisar de outro prazo usa [`ClienteMatrix::com_timeout`] (testes).
//! - **Resposta com teto**: o corpo é lido em pedaços e a leitura para em 1 MiB; o evento de
//!   estado do módulo é pequeno, e um corpo maior que isso é resposta inválida.
//!
//! Nada aqui repete entrega de mensagem nem mexe em banco: é transporte.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use poph_types::{MatrixConfig, Segredo};
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::SegredosMatrix;
use crate::erro::{ErroChamada, ErroMidia};

/// Base da API cliente do Matrix.
const API: &str = "/_matrix/client/v3";

/// Prazo padrão de cada chamada ao Synapse.
pub const TEMPO_LIMITE_PADRAO: Duration = Duration::from_secs(10);

/// Espera máxima que o cliente aceita cumprir sozinho num `429`. Acima disso ele devolve o erro
/// com `retry_after_ms` e quem espera é o laço (uma espera de minutos não pode travar o cliente).
pub const ESPERA_MAXIMA_429: Duration = Duration::from_secs(60);

/// Rota de upload de mídia que o Synapse de desenvolvimento atende (a depreciada, ainda viva).
const CAMINHO_UPLOAD_ANTIGO: &str = "/_matrix/media/v3/upload";

/// Rota de upload nova, tentada quando o servidor não reconhece a antiga.
const CAMINHO_UPLOAD_NOVO: &str = "/_matrix/client/v1/media/upload";

/// Rota de download de mídia que o Synapse de desenvolvimento atende (a depreciada, ainda viva).
const CAMINHO_DOWNLOAD_ANTIGO: &str = "/_matrix/media/v3/download";

/// Rota de download nova, tentada quando o servidor não reconhece a antiga.
const CAMINHO_DOWNLOAD_NOVO: &str = "/_matrix/client/v1/media/download";

/// Base da API **administrativa** do Synapse (MX-46): apagar mídia do repositório local.
///
/// A rota recebe `servidor` e id da mídia, e exige token de administrador — nunca o `as_token`.
const ADMIN_MEDIA: &str = "/_synapse/admin/v1/media";

/// `errcode` do Synapse para rota que ele não conhece.
const ERRCODE_NAO_RECONHECIDO: &str = "M_UNRECOGNIZED";

/// Teto de leitura do corpo de uma resposta (1 MiB).
const TETO_CORPO: u64 = 1024 * 1024;

/// Tamanho máximo da mensagem do Synapse que entra na mensagem de erro.
const TAMANHO_MENSAGEM: usize = 160;

// ───────────────────────────── configuração do cliente ─────────────────────────────

/// O que o cliente precisa para falar com o Synapse.
///
/// Sai da seção `[matrix]` mais os segredos do Application Service (MX-07). Não contém
/// `concorrencia` nem `intervalo_ms`: esses são do laço.
#[derive(Clone, Debug)]
pub struct ConfigCliente {
    /// Endereço base da API cliente, sem barra final (ex.: `http://127.0.0.1:18008`).
    pub url_base: String,
    /// Domínio dos ids Matrix, como no `server_name` do Synapse.
    pub server_name: String,
    /// Prazo de cada chamada.
    pub timeout: Duration,
    /// `as_token` do registro do Application Service. `Debug` mostra `[oculto]`.
    as_token: Segredo,
}

impl ConfigCliente {
    /// Monta a configuração a partir das partes, validando o que quebraria toda chamada.
    ///
    /// Mensagem de erro nomeia o campo, nunca o valor.
    pub fn novo(
        url_base: impl Into<String>,
        server_name: impl Into<String>,
        as_token: Segredo,
    ) -> Result<Self, ErroChamada> {
        let url_base = url_base.into().trim().trim_end_matches('/').to_owned();
        if url_base.is_empty() {
            return Err(ErroChamada::Configuracao(
                "`matrix.homeserver_url` está vazio".to_owned(),
            ));
        }
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(ErroChamada::Configuracao(
                "`matrix.homeserver_url` deve começar com http:// ou https://".to_owned(),
            ));
        }
        let server_name = server_name.into().trim().to_owned();
        if server_name.is_empty() {
            return Err(ErroChamada::Configuracao(
                "`matrix.server_name` está vazio".to_owned(),
            ));
        }
        if as_token.expor().trim().is_empty() {
            return Err(ErroChamada::Configuracao(
                "o token do Application Service (`POPH_MATRIX_AS_TOKEN`) está vazio".to_owned(),
            ));
        }
        Ok(Self {
            url_base,
            server_name,
            timeout: TEMPO_LIMITE_PADRAO,
            as_token,
        })
    }

    /// Mesma montagem, a partir da seção `[matrix]` já validada e dos segredos da MX-07.
    pub fn das_partes(
        config: &MatrixConfig,
        segredos: &SegredosMatrix,
    ) -> Result<Self, ErroChamada> {
        Self::novo(
            config.homeserver_url.clone(),
            config.server_name.clone(),
            segredos.as_token.clone(),
        )
    }

    /// Troca o prazo por chamada (testes de prazo).
    pub fn com_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Corpo do pedido HTTP: vazio, JSON (a maioria das chamadas) ou bytes (upload de mídia, MX-43).
#[derive(Clone, Copy)]
enum Corpo<'a> {
    /// Sem corpo (GET, por exemplo).
    Nenhum,
    /// Corpo JSON, como todas as chamadas da API cliente.
    Json(&'a Value),
    /// Conteúdo binário de um arquivo, com o tipo que vai no `Content-Type`.
    Bytes {
        /// Tipo MIME do arquivo.
        mime: &'a str,
        /// Bytes do arquivo.
        bytes: &'a [u8],
    },
}

// ───────────────────────────── corpo do `createRoom` ─────────────────────────────

/// Corpo do `createRoom`, com os campos que o módulo usa. Campo `None` não vai no JSON.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct CriarSala {
    /// Nome curto do alias, sem `#` e sem `:servidor` (ex.: `ph_c_conv_123`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_alias_name: Option<String>,
    /// Nome visível da sala (nome do cliente).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tópico da sala.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Visibilidade (`private` por padrão no Synapse; o módulo não mexe).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// `creation_content`: tipo da sala (`type`), entre outros.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_content: Option<Value>,
    /// Eventos de estado iniciais (ex.: `atendimento.metadata`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_state: Option<Vec<Value>>,
    /// Usuários convidados já na criação.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite: Option<Vec<String>>,
    /// `preset` do Synapse (`private_chat`, `trusted_private_chat`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Poderes da sala, mesclados por cima do padrão do Synapse. O módulo usa para dar ao fantasma
    /// do agente o poder de `matrix.poder_agente` (MX-11).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_level_content_override: Option<Value>,
}

// ───────────────────────────── tipos de resposta ─────────────────────────────

/// Um membro da sala, como `joined_members` devolve.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct MembroSala {
    /// Nome de exibição, quando definido.
    pub display_name: Option<String>,
    /// Avatar, quando definido.
    pub avatar_url: Option<String>,
}

/// Perfil de um usuário, como o `GET /profile/{userId}` devolve.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Perfil {
    /// Nome de exibição.
    pub displayname: Option<String>,
    /// Avatar.
    pub avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct JuntaResposta {
    joined: BTreeMap<String, MembroSala>,
}

// ───────────────────────────── o cliente ─────────────────────────────

/// Os dados de uma requisição, sem o cabeçalho de autorização.
///
/// Existe para o mesmo caminho de transporte servir ao `as_token` (API cliente) e ao token de
/// administração (API admin, MX-46) sem uma função de oito parâmetros.
#[derive(Clone, Copy)]
struct Pedido<'a> {
    metodo: &'a Method,
    caminho: &'a str,
    consulta_fixa: &'a [(&'a str, &'a str)],
    usuario: Option<&'a str>,
    corpo: Corpo<'a>,
    operacao: &'static str,
}

struct Interno {
    http: reqwest::Client,
    /// Cliente só do download de mídia: **não segue redirecionamento** (ver [`ClienteMatrix::baixar_midia`]).
    http_sem_redirecionar: reqwest::Client,
    config: ConfigCliente,
}

/// Cliente da API do Matrix. Clonar é barato (o `reqwest::Client` é compartilhado).
#[derive(Clone)]
pub struct ClienteMatrix(Arc<Interno>);

impl fmt::Debug for ClienteMatrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClienteMatrix")
            .field("url_base", &self.0.config.url_base)
            .field("server_name", &self.0.config.server_name)
            .field("timeout", &self.0.config.timeout)
            .field("as_token", &self.0.config.as_token)
            .finish()
    }
}

impl ClienteMatrix {
    /// Cria o cliente HTTP com o prazo por chamada.
    pub fn novo(config: ConfigCliente) -> Result<Self, ErroChamada> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent(format!("poph-harness/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|erro| ErroChamada::Configuracao(format!("cliente HTTP do Matrix: {erro}")))?;
        let http_sem_redirecionar = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent(format!("poph-harness/{}", env!("CARGO_PKG_VERSION")))
            // MX-44: o arquivo do atendente sai da sala, e o download não pode desviar para um
            // endereço fora do homeserver. Sem seguir redirecionamento, o desvio é recusado.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|erro| {
                ErroChamada::Configuracao(format!("cliente HTTP de mídia do Matrix: {erro}"))
            })?;
        Ok(Self(Arc::new(Interno {
            http,
            http_sem_redirecionar,
            config,
        })))
    }

    /// Cliente a partir da seção `[matrix]` e dos segredos do Application Service.
    pub fn das_partes(
        config: &MatrixConfig,
        segredos: &SegredosMatrix,
    ) -> Result<Self, ErroChamada> {
        Self::novo(ConfigCliente::das_partes(config, segredos)?)
    }

    /// Mesmo cliente com outro prazo por chamada.
    pub fn com_timeout(&self, timeout: Duration) -> Result<Self, ErroChamada> {
        let mut config = self.0.config.clone();
        config.timeout = timeout;
        Self::novo(config)
    }

    /// Endereço base configurado.
    pub fn url_base(&self) -> &str {
        &self.0.config.url_base
    }

    /// `server_name` configurado (usado no `via` de `m.space.child`).
    pub fn server_name(&self) -> &str {
        &self.0.config.server_name
    }

    // ───────────────────────────── salas ─────────────────────────────

    /// Cria uma sala agindo como `usuario` e devolve o id da sala.
    ///
    /// Alias repetido devolve `M_ROOM_IN_USE` (erro definitivo): quem trata idempotência é a
    /// MX-11, resolvendo o alias para a sala já criada.
    pub async fn criar_sala(&self, usuario: &str, sala: &CriarSala) -> Result<String, ErroChamada> {
        let corpo = serde_json::to_value(sala).map_err(|erro| {
            ErroChamada::Configuracao(format!("corpo do createRoom não serializou: {erro}"))
        })?;
        let valor = self
            .chamar(
                Method::POST,
                format!("{API}/createRoom"),
                Some(usuario),
                Corpo::Json(&corpo),
                "createRoom",
            )
            .await?;
        texto_obrigatorio(&valor, "room_id", "createRoom")
    }

    /// Convida `convidado` para a sala.
    pub async fn convidar(
        &self,
        usuario: &str,
        sala: &str,
        convidado: &str,
    ) -> Result<(), ErroChamada> {
        self.chamar(
            Method::POST,
            format!("{API}/rooms/{}/invite", codificar(sala)),
            Some(usuario),
            Corpo::Json(&json!({ "user_id": convidado })),
            "invite",
        )
        .await
        .map(|_| ())
    }

    /// Entra numa sala ou aceita um alias. Devolve o id da sala em que entrou.
    pub async fn entrar(&self, usuario: &str, sala_ou_alias: &str) -> Result<String, ErroChamada> {
        let valor = self
            .chamar(
                Method::POST,
                format!("{API}/join/{}", codificar(sala_ou_alias)),
                Some(usuario),
                Corpo::Json(&json!({})),
                "join",
            )
            .await?;
        texto_obrigatorio(&valor, "room_id", "join")
    }

    /// Resolve um alias de sala para o id da sala (`GET /directory/room/{alias}`).
    ///
    /// É o outro lado da idempotência da criação (P6): `createRoom` com alias repetido devolve
    /// `M_ROOM_IN_USE`, e o alias resolve para a sala que já existe — que é a nossa, porque o alias
    /// é derivado do id da conversa. Alias inexistente devolve `M_NOT_FOUND`.
    pub async fn resolver_alias(&self, alias: &str) -> Result<String, ErroChamada> {
        let valor = self
            .chamar(
                Method::GET,
                format!("{API}/directory/room/{}", codificar(alias)),
                None,
                Corpo::Nenhum,
                "directory/room",
            )
            .await?;
        texto_obrigatorio(&valor, "room_id", "directory/room")
    }

    /// Membros que estão na sala agora (`joined_members`), por id de usuário.
    pub async fn membros(
        &self,
        usuario: &str,
        sala: &str,
    ) -> Result<BTreeMap<String, MembroSala>, ErroChamada> {
        let valor = self
            .chamar(
                Method::GET,
                format!("{API}/rooms/{}/joined_members", codificar(sala)),
                Some(usuario),
                Corpo::Nenhum,
                "joined_members",
            )
            .await?;
        serde_json::from_value::<JuntaResposta>(valor)
            .map(|junta| junta.joined)
            .map_err(|erro| ErroChamada::RespostaInvalida {
                operacao: "joined_members",
                motivo: format!("corpo fora do contrato: {erro}"),
            })
    }

    // ───────────────────────────── eventos ─────────────────────────────

    /// Envia um evento de mensagem para a sala e devolve o `event_id`.
    ///
    /// `txn_id` é a chave de idempotência do Synapse (P7): repetir o mesmo `txn_id` com o mesmo
    /// corpo devolve o mesmo evento em vez de duplicar. Depois de um prazo, quem chama não sabe se o
    /// evento entrou; repetir com o mesmo `txn_id` é seguro.
    pub async fn enviar_evento(
        &self,
        usuario: &str,
        sala: &str,
        tipo: &str,
        txn_id: &str,
        corpo: &Value,
    ) -> Result<String, ErroChamada> {
        let valor = self
            .chamar(
                Method::PUT,
                format!(
                    "{API}/rooms/{}/send/{}/{}",
                    codificar(sala),
                    codificar(tipo),
                    codificar(txn_id)
                ),
                Some(usuario),
                Corpo::Json(corpo),
                "send",
            )
            .await?;
        texto_obrigatorio(&valor, "event_id", "send")
    }

    /// Escreve um evento de estado (PUT). Devolve o `event_id` que o Synapse gerou.
    ///
    /// `chave` é a `state_key`; `None` ou `""` escrevem o evento sem chave.
    pub async fn enviar_estado(
        &self,
        usuario: &str,
        sala: &str,
        tipo: &str,
        chave: Option<&str>,
        corpo: &Value,
    ) -> Result<String, ErroChamada> {
        let valor = self
            .chamar(
                Method::PUT,
                caminho_estado(sala, tipo, chave),
                Some(usuario),
                Corpo::Json(corpo),
                "state",
            )
            .await?;
        texto_obrigatorio(&valor, "event_id", "state")
    }

    /// Lê um evento de estado (GET). Sem `chave`, lê o evento sem chave.
    pub async fn ler_estado(
        &self,
        usuario: &str,
        sala: &str,
        tipo: &str,
        chave: Option<&str>,
    ) -> Result<Value, ErroChamada> {
        self.chamar(
            Method::GET,
            caminho_estado(sala, tipo, chave),
            Some(usuario),
            Corpo::Nenhum,
            "state (leitura)",
        )
        .await
    }

    /// Escreve `m.space.child` no espaço apontando para a sala filha (MX-12).
    ///
    /// A `state_key` é o id da sala filha e o corpo leva `via` com o `server_name`: é isso que faz
    /// o Element mostrar a sala sob o espaço.
    pub async fn espaco_filho(
        &self,
        usuario: &str,
        espaco: &str,
        sala_filha: &str,
        ordem: Option<&str>,
    ) -> Result<String, ErroChamada> {
        let mut corpo = json!({ "via": [self.0.config.server_name] });
        if let Some(ordem) = ordem {
            corpo["order"] = Value::String(ordem.to_owned());
        }
        self.enviar_estado(usuario, espaco, "m.space.child", Some(sala_filha), &corpo)
            .await
    }

    /// Salas filhas de um espaço (`GET /_matrix/client/v1/rooms/{roomId}/hierarchy`).
    ///
    /// É a leitura que prova do lado do Synapse que a sala aparece **sob** o espaço (MX-12): o
    /// `m.space.child` é a intenção; a hierarquia é o que o Element desenha. Devolve os `room_id`
    /// das salas filhas, na ordem em que o Synapse as lista.
    pub async fn hierarquia(
        &self,
        usuario: &str,
        espaco: &str,
    ) -> Result<Vec<String>, ErroChamada> {
        let valor = self
            .chamar_com_consulta(
                Method::GET,
                format!("/_matrix/client/v1/rooms/{}/hierarchy", codificar(espaco)),
                &[("limit", "100")],
                Some(usuario),
                Corpo::Nenhum,
                "hierarchy",
            )
            .await?;
        let salas = valor
            .get("rooms")
            .and_then(Value::as_array)
            .ok_or_else(|| ErroChamada::RespostaInvalida {
                operacao: "hierarchy",
                motivo: "campo `rooms` ausente".to_owned(),
            })?;
        Ok(salas
            .iter()
            .filter_map(|sala| sala.get("room_id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect())
    }

    // ───────────────────────────── perfil e registro ─────────────────────────────

    /// Lê o perfil de um usuário.
    pub async fn perfil(&self, usuario_id: &str) -> Result<Perfil, ErroChamada> {
        let valor = self
            .chamar(
                Method::GET,
                format!("{API}/profile/{}", codificar(usuario_id)),
                None,
                Corpo::Nenhum,
                "profile",
            )
            .await?;
        serde_json::from_value::<Perfil>(valor).map_err(|erro| ErroChamada::RespostaInvalida {
            operacao: "profile",
            motivo: format!("corpo fora do contrato: {erro}"),
        })
    }

    /// Define o nome de exibição de `usuario_id`, agindo como `usuario`.
    ///
    /// Para o nome de um usuário virtual, `usuario` precisa ser o próprio usuário virtual: o
    /// Synapse recusa (`M_FORBIDDEN`, "Cannot set another user's displayname") quando o
    /// disfarce aponta para outro usuário. É o que o teste real da MX-09 comprova.
    pub async fn definir_nome_exibicao(
        &self,
        usuario: &str,
        usuario_id: &str,
        nome: &str,
    ) -> Result<(), ErroChamada> {
        self.chamar(
            Method::PUT,
            format!("{API}/profile/{}/displayname", codificar(usuario_id)),
            Some(usuario),
            Corpo::Json(&json!({ "displayname": nome })),
            "profile (nome)",
        )
        .await
        .map(|_| ())
    }

    /// Registra um usuário virtual do Application Service e devolve o id completo.
    ///
    /// `localpart` é o nome sem `@` e sem `:servidor` (ex.: `ph_wa_5511999999999`). O Synapse
    /// responde `M_USER_IN_USE` se já existir; registro idempotente é a MX-10.
    ///
    /// Vai com `inhibit_login`: o usuário virtual nasce **sem** token de acesso próprio (o módulo
    /// age pelo disfarce do Application Service). Sem isso o Synapse criaria um dispositivo e um
    /// token por cliente atendido.
    pub async fn registrar_usuario(&self, localpart: &str) -> Result<String, ErroChamada> {
        let valor = self
            .chamar_com_consulta(
                Method::POST,
                format!("{API}/register"),
                &[("kind", "user")],
                None,
                Corpo::Json(&json!({
                    "type": "m.login.application_service",
                    "username": localpart,
                    // Sem token de acesso: o módulo age pelo disfarce do AS, e um token por
                    // usuário virtual viraria milhares de dispositivos no Synapse.
                    "inhibit_login": true,
                })),
                "register",
            )
            .await?;
        texto_obrigatorio(&valor, "user_id", "register")
    }

    // ───────────────────────────── mídia ─────────────────────────────

    /// Sobe um arquivo ao Synapse como `usuario` e devolve o `content_uri` (`mxc://…`).
    ///
    /// Duas rotas conhecidas, na ordem:
    ///
    /// 1. `POST /_matrix/media/v3/upload` — a que o Synapse de desenvolvimento atende hoje
    ///    (comprovado contra o Synapse 1.120 do ambiente da MX-01);
    /// 2. `POST /_matrix/client/v1/media/upload` — a rota nova, tentada quando o servidor responde
    ///    `404 M_UNRECOGNIZED` à antiga (versões que removeram a rota depreciada).
    ///
    /// O `filename` vai na consulta e o tipo do arquivo no `Content-Type`. Nada do conteúdo, do
    /// nome ou do `mxc` vai para log ou mensagem de erro.
    pub async fn enviar_midia(
        &self,
        usuario: &str,
        nome: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<String, ErroChamada> {
        let valor = match self
            .subir_arquivo(CAMINHO_UPLOAD_ANTIGO, usuario, nome, mime, bytes)
            .await
        {
            Ok(valor) => valor,
            Err(ErroChamada::Resposta {
                status: 404,
                errcode,
                ..
            }) if errcode
                .as_deref()
                .is_none_or(|codigo| codigo == ERRCODE_NAO_RECONHECIDO) =>
            {
                self.subir_arquivo(CAMINHO_UPLOAD_NOVO, usuario, nome, mime, bytes)
                    .await?
            }
            Err(erro) => return Err(erro),
        };
        texto_obrigatorio(&valor, "content_uri", "upload")
    }

    /// Uma tentativa de upload numa rota. Devolve o corpo (o `content_uri` está nele).
    async fn subir_arquivo(
        &self,
        caminho: &'static str,
        usuario: &str,
        nome: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<Value, ErroChamada> {
        self.chamar_com_consulta(
            Method::POST,
            caminho.to_owned(),
            &[("filename", nome)],
            Some(usuario),
            Corpo::Bytes { mime, bytes },
            "upload",
        )
        .await
    }

    /// Baixa o conteúdo de uma mídia da sala a partir do `content_uri` do evento
    /// (`mxc://servidor/id`), com teto de bytes (MX-44).
    ///
    /// O que o caminho garante:
    ///
    /// - **nada de redirecionamento para fora.** O pedido vai com `allow_redirect=false` (o Synapse
    ///   entrega o conteúdo direto) e o cliente HTTP deste pedido **não segue** `3xx`: uma resposta
    ///   de redirecionamento é falha definitiva, e não um desvio para outro servidor;
    /// - **teto antes da memória.** O `Content-Length` declarado é conferido antes de ler, e a
    ///   leitura para no teto: acima disso, [`ErroMidia::Grande`] sem carregar o resto;
    /// - **a rota nova primeiro** (`/_matrix/client/v1/media/download`, a que o Synapse atual
    ///   atende) e, num `404`, a depreciada (`/_matrix/media/v3/download`), que servidores anteriores
    ///   usavam. Se as duas falharem, vale a resposta da rota nova — é a que o contrato promete hoje
    ///   (medido no Synapse 1.120 do ambiente da MX-01: a rota antiga já não existe e responde
    ///   `404 M_NOT_FOUND`);
    /// - **disfarce de usuário** (`?user_id=`), como nas demais chamadas — e o usuário tem de ser do
    ///   **namespace do Application Service** (`@<prefixo>_…`). O Synapse recusa com
    ///   `403 M_FORBIDDEN` ("Application service cannot masquerade as this user") quem é usuário
    ///   comum do homeserver, que é o caso do atendente humano que manda o arquivo pelo Element.
    ///   Quem lê é, portanto, o fantasma do agente dono da conversa (MX-44), nunca o humano que
    ///   publicou o evento.
    ///
    /// O conteúdo, o nome e o `mxc://` não entram em log nem em mensagem de erro.
    pub async fn baixar_midia(
        &self,
        usuario: &str,
        mxc: &str,
        teto: u64,
    ) -> Result<Vec<u8>, ErroMidia> {
        let (servidor, id) = partes_do_mxc(mxc)?;
        let caminho_novo = format!(
            "{CAMINHO_DOWNLOAD_NOVO}/{}/{}",
            codificar(&servidor),
            codificar(&id)
        );
        let primeira = self.baixar(&caminho_novo, usuario, teto).await;
        match primeira {
            Err(ErroMidia::Chamada(ErroChamada::Resposta { status: 404, .. })) => {
                let caminho_antigo = format!(
                    "{CAMINHO_DOWNLOAD_ANTIGO}/{}/{}",
                    codificar(&servidor),
                    codificar(&id)
                );
                match self.baixar(&caminho_antigo, usuario, teto).await {
                    Ok(bytes) => Ok(bytes),
                    // Mídia que não existe responde 404 nas duas rotas: fica valendo a da rota
                    // principal, que é a do contrato de hoje.
                    Err(_) => primeira,
                }
            }
            outro => outro,
        }
    }

    /// Uma tentativa de download numa rota, com o teto de bytes.
    async fn baixar(&self, caminho: &str, usuario: &str, teto: u64) -> Result<Vec<u8>, ErroMidia> {
        let operacao = "download";
        let url = format!("{}{}", self.0.config.url_base, caminho);
        let autorizacao = autorizacao_do_token(&self.0.config)?;
        let resposta = self
            .0
            .http_sem_redirecionar
            .get(&url)
            .header(AUTHORIZATION, autorizacao)
            .query(&[("user_id", usuario), ("allow_redirect", "false")])
            .send()
            .await
            .map_err(|erro| falha_de_transporte(erro, operacao, self.0.config.timeout))?;

        let status = resposta.status();
        if status.is_redirection() {
            return Err(ErroMidia::Chamada(ErroChamada::RespostaInvalida {
                operacao,
                motivo: "o Synapse respondeu com redirecionamento, que não é seguido".to_owned(),
            }));
        }
        if !status.is_success() {
            let bruto = ler_corpo(resposta, operacao).await?;
            let valor: Value = serde_json::from_slice(&bruto).unwrap_or(Value::Null);
            let (errcode, mensagem, retry_after_ms) = detalhe_de_erro(&valor);
            return Err(ErroMidia::Chamada(ErroChamada::Resposta {
                operacao,
                status: status.as_u16(),
                errcode,
                mensagem,
                retry_after_ms,
            }));
        }

        // O tamanho declarado evita carregar 16 MB para descobrir que passa do teto.
        if let Some(declarado) = resposta.content_length()
            && declarado > teto
        {
            return Err(ErroMidia::Grande {
                bytes: declarado,
                teto,
            });
        }

        let mut acumulado = Vec::new();
        let mut resposta = resposta;
        while let Some(pedaco) = resposta
            .chunk()
            .await
            .map_err(|erro| falha_de_transporte(erro, operacao, self.0.config.timeout))?
        {
            let total = acumulado.len() as u64 + pedaco.len() as u64;
            if total > teto {
                return Err(ErroMidia::Grande { bytes: total, teto });
            }
            acumulado.extend_from_slice(&pedaco);
        }
        if acumulado.is_empty() {
            return Err(ErroMidia::Vazia);
        }
        Ok(acumulado)
    }

    /// Lê um evento da sala pelo id (`GET /rooms/{sala}/event/{evento}`).
    ///
    /// Serve para conferir o que ficou na sala depois de um envio (aceite e testes contra o
    /// Synapse de verdade). O evento volta como o Synapse o guarda, inclusive o `sender`.
    pub async fn ler_evento(
        &self,
        usuario: &str,
        sala: &str,
        evento: &str,
    ) -> Result<Value, ErroChamada> {
        self.chamar(
            Method::GET,
            format!(
                "{API}/rooms/{}/event/{}",
                codificar(sala),
                codificar(evento)
            ),
            Some(usuario),
            Corpo::Nenhum,
            "event (leitura)",
        )
        .await
    }

    // ───────────────────────────── expiração da mídia (MX-46) ─────────────────────────────

    /// Redige um evento da sala em nome de `usuario` (`PUT /rooms/{sala}/redact/{evento}/{txn}`).
    ///
    /// O corpo vai como está: é nele que a expiração publica o bloco que o frontend lê em
    /// `unsigned.redacted_because.content`. O `txn_id` é a chave de idempotência do Synapse, como no
    /// envio: repetir o mesmo `txn_id` não cria uma segunda redação.
    ///
    /// Quem redige é um moderador da sala (o fantasma do agente, dono da sala): o remetente do
    /// evento de mídia é o fantasma do cliente, e ele não tem poder para apagar o próprio evento
    /// depois de publicado.
    pub async fn redigir(
        &self,
        usuario: &str,
        sala: &str,
        evento: &str,
        txn_id: &str,
        conteudo: &Value,
    ) -> Result<String, ErroChamada> {
        let valor = self
            .chamar(
                Method::PUT,
                format!(
                    "{API}/rooms/{}/redact/{}/{}",
                    codificar(sala),
                    codificar(evento),
                    codificar(txn_id)
                ),
                Some(usuario),
                Corpo::Json(conteudo),
                "redact",
            )
            .await?;
        texto_obrigatorio(&valor, "event_id", "redact")
    }

    /// Apaga o arquivo do repositório local do Synapse pela API **administrativa** (MX-46).
    ///
    /// `DELETE /_synapse/admin/v1/media/{servidor}/{id}`, com o token de administrador (nunca o
    /// `as_token`). Devolve `true` quando o arquivo existia e foi apagado e `false` quando ele já
    /// não existia — no Synapse atual o `DELETE` de mídia inexistente responde `404`, e isso conta
    /// como apagado (repetir a expiração não pode falhar por já ter dado certo antes).
    ///
    /// Não repete sozinho: um `429` volta como erro e quem decide esperar é a rotina de expiração,
    /// que adia o item com espera crescente.
    pub async fn apagar_midia(&self, token_admin: &Segredo, mxc: &str) -> Result<bool, ErroMidia> {
        let (servidor, id) = partes_do_mxc(mxc)?;
        let caminho = format!("{ADMIN_MEDIA}/{}/{}", codificar(&servidor), codificar(&id));
        let autorizacao = autorizacao_de(token_admin, "de administrador do Synapse")
            .map_err(ErroMidia::Chamada)?;
        let pedido = Pedido {
            metodo: &Method::DELETE,
            caminho: &caminho,
            consulta_fixa: &[],
            usuario: None,
            corpo: Corpo::Nenhum,
            operacao: "media (administração)",
        };
        match self.tentar_com(pedido, autorizacao).await {
            Ok(_) => Ok(true),
            // Mídia que já não existe: o objetivo da chamada está cumprido.
            Err(ErroChamada::Resposta { status: 404, .. }) => Ok(false),
            Err(erro) => Err(ErroMidia::Chamada(erro)),
        }
    }

    // ───────────────────────────── conexão ─────────────────────────────

    /// Confere se o Synapse responde (`GET /_matrix/client/versions`).
    ///
    /// É a sonda barata de conexão da MX-20: não pede token de usuário, não cria nada e não muda
    /// estado. Erro de rede/prazo é classificado como transitório pelo [`ErroChamada`].
    pub async fn conferir_conexao(&self) -> Result<(), ErroChamada> {
        self.chamar(
            Method::GET,
            "/_matrix/client/versions".to_owned(),
            None,
            Corpo::Nenhum,
            "versions",
        )
        .await
        .map(|_| ())
    }

    // ───────────────────────────── transporte ─────────────────────────────

    /// Uma chamada completa: tenta uma vez e, em `429` com espera curta, repete uma única vez.
    async fn chamar(
        &self,
        metodo: Method,
        caminho: String,
        usuario: Option<&str>,
        corpo: Corpo<'_>,
        operacao: &'static str,
    ) -> Result<Value, ErroChamada> {
        self.chamar_com_consulta(metodo, caminho, &[], usuario, corpo, operacao)
            .await
    }

    async fn chamar_com_consulta(
        &self,
        metodo: Method,
        caminho: String,
        consulta_fixa: &[(&str, &str)],
        usuario: Option<&str>,
        corpo: Corpo<'_>,
        operacao: &'static str,
    ) -> Result<Value, ErroChamada> {
        let pedido = Pedido {
            metodo: &metodo,
            caminho: &caminho,
            consulta_fixa,
            usuario,
            corpo,
            operacao,
        };
        let resultado = self.tentar(pedido).await;
        match resultado {
            Ok(valor) => Ok(valor),
            Err(erro) => {
                // Única repetição automática: o Synapse mandou o tempo de espera e ele cabe no
                // que o cliente aceita esperar sozinho.
                if let ErroChamada::Resposta {
                    status: 429,
                    retry_after_ms: Some(ms),
                    ..
                } = &erro
                    && Duration::from_millis(*ms) <= ESPERA_MAXIMA_429
                {
                    tokio::time::sleep(Duration::from_millis(*ms)).await;
                    self.tentar(pedido).await
                } else {
                    Err(erro)
                }
            }
        }
    }

    /// Uma requisição HTTP, sem repetição, com o `as_token` do Application Service.
    async fn tentar(&self, pedido: Pedido<'_>) -> Result<Value, ErroChamada> {
        let autorizacao = autorizacao_do_token(&self.0.config)?;
        self.tentar_com(pedido, autorizacao).await
    }

    /// Igual a [`ClienteMatrix::tentar`], com o cabeçalho `Authorization` já montado.
    ///
    /// É o caminho da API administrativa (MX-46), que usa outro token no mesmo cliente.
    async fn tentar_com(
        &self,
        pedido: Pedido<'_>,
        autorizacao: HeaderValue,
    ) -> Result<Value, ErroChamada> {
        let Pedido {
            metodo,
            caminho,
            consulta_fixa,
            usuario,
            corpo,
            operacao,
        } = pedido;
        let url = format!("{}{}", self.0.config.url_base, caminho);

        let mut pedido = self
            .0
            .http
            .request(metodo.clone(), &url)
            .header(AUTHORIZATION, autorizacao);
        for (nome, valor) in consulta_fixa {
            pedido = pedido.query(&[(nome, valor)]);
        }
        if let Some(usuario) = usuario {
            pedido = pedido.query(&[("user_id", usuario)]);
        }
        match corpo {
            Corpo::Nenhum => {}
            Corpo::Json(valor) => pedido = pedido.json(valor),
            // Arquivo: os bytes como estão, com o tipo que o recebimento reconheceu. O `filename`
            // vai na consulta (é lá que o Synapse o guarda), nunca num cabeçalho.
            Corpo::Bytes { mime, bytes } => {
                pedido = pedido
                    .header(
                        CONTENT_TYPE,
                        HeaderValue::from_str(mime).map_err(|_| {
                            ErroChamada::Configuracao(
                                "o tipo do arquivo não cabe num cabeçalho HTTP".to_owned(),
                            )
                        })?,
                    )
                    .body(bytes.to_vec());
            }
        }

        let resposta = pedido
            .send()
            .await
            .map_err(|erro| falha_de_transporte(erro, operacao, self.0.config.timeout))?;

        let status = resposta.status();
        let bruto = ler_corpo(resposta, operacao).await?;
        let texto = String::from_utf8_lossy(&bruto);
        let valor: Value = if texto.trim().is_empty() {
            Value::Null
        } else {
            match serde_json::from_str(&texto) {
                Ok(valor) => valor,
                Err(_) if status.is_success() => {
                    return Err(ErroChamada::RespostaInvalida {
                        operacao,
                        motivo: "o corpo da resposta não é JSON".to_owned(),
                    });
                }
                Err(_) => Value::Null,
            }
        };

        if status.is_success() {
            return Ok(valor);
        }

        let (errcode, mensagem, retry_after_ms) = detalhe_de_erro(&valor);
        Err(ErroChamada::Resposta {
            operacao,
            status: status.as_u16(),
            errcode,
            mensagem,
            retry_after_ms,
        })
    }
}

// ───────────────────────────── auxiliares ─────────────────────────────

/// Cabeçalho `Authorization` do Application Service.
///
/// O token é `Sensivel`: o valor só é exposto aqui, para virar cabeçalho, e nunca entra em mensagem
/// de erro.
fn autorizacao_do_token(config: &ConfigCliente) -> Result<HeaderValue, ErroChamada> {
    autorizacao_de(&config.as_token, "do Application Service")
}

/// Cabeçalho `Authorization` de um token qualquer (`as_token` ou o de administrador da MX-46).
///
/// `rotulo` nomeia o token no erro — o valor nunca aparece.
fn autorizacao_de(token: &Segredo, rotulo: &'static str) -> Result<HeaderValue, ErroChamada> {
    HeaderValue::from_str(&format!("Bearer {}", token.expor().trim())).map_err(|_| {
        ErroChamada::Configuracao(format!(
            "o token {rotulo} tem caractere que não cabe em cabeçalho HTTP"
        ))
    })
}

/// Traduz a falha de transporte da `reqwest` no erro do cliente, sem o endereço na mensagem.
fn falha_de_transporte(
    erro: reqwest::Error,
    operacao: &'static str,
    timeout: Duration,
) -> ErroChamada {
    if erro.is_timeout() {
        ErroChamada::Prazo {
            operacao,
            ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        }
    } else if erro.is_connect() {
        ErroChamada::Rede {
            operacao,
            motivo: "não consegui conectar".to_owned(),
        }
    } else {
        // `without_url` tira o endereço (que carrega o id da sala ou da mídia) da mensagem.
        ErroChamada::Rede {
            operacao,
            motivo: erro.without_url().to_string(),
        }
    }
}

/// `(servidor, id)` de um `content_uri` do Matrix (`mxc://servidor/id`).
///
/// O id de mídia do Matrix não tem barra; qualquer coisa depois da segunda barra recusa o endereço
/// em vez de virar parte do caminho da requisição.
fn partes_do_mxc(mxc: &str) -> Result<(String, String), ErroMidia> {
    let resto = mxc
        .trim()
        .strip_prefix("mxc://")
        .ok_or_else(|| ErroMidia::Endereco("o endereço não começa com `mxc://`".to_owned()))?;
    let (servidor, id) = resto.split_once('/').ok_or_else(|| {
        ErroMidia::Endereco("o endereço não tem servidor e identificador".to_owned())
    })?;
    if servidor.is_empty() || id.is_empty() {
        return Err(ErroMidia::Endereco(
            "o endereço tem servidor ou identificador vazio".to_owned(),
        ));
    }
    if id.contains('/') {
        return Err(ErroMidia::Endereco(
            "o identificador da mídia tem barra".to_owned(),
        ));
    }
    Ok((servidor.to_owned(), id.to_owned()))
}

/// Lê o corpo em pedaços, até [`TETO_CORPO`]. Um corpo maior que o teto é resposta inválida.
async fn ler_corpo(
    mut resposta: reqwest::Response,
    operacao: &'static str,
) -> Result<Vec<u8>, ErroChamada> {
    let mut acumulado = Vec::new();
    while let Some(pedaco) = resposta.chunk().await.map_err(|erro| ErroChamada::Rede {
        operacao,
        motivo: erro.without_url().to_string(),
    })? {
        if acumulado.len() as u64 + pedaco.len() as u64 > TETO_CORPO {
            return Err(ErroChamada::RespostaInvalida {
                operacao,
                motivo: format!("a resposta passou de {TETO_CORPO} bytes"),
            });
        }
        acumulado.extend_from_slice(&pedaco);
    }
    Ok(acumulado)
}

/// `(errcode, mensagem, retry_after_ms)` de um corpo de erro do Matrix, já limpos.
///
/// Lê só os campos do contrato: o corpo nunca vira mensagem de erro inteiro.
fn detalhe_de_erro(valor: &Value) -> (Option<String>, String, Option<u64>) {
    let errcode = valor.get("errcode").and_then(Value::as_str).map(recortar);
    let mensagem = valor
        .get("error")
        .and_then(Value::as_str)
        .map(recortar)
        .unwrap_or_else(|| "sem mensagem do Synapse".to_owned());
    let retry_after_ms = valor.get("retry_after_ms").and_then(Value::as_u64);
    (errcode, mensagem, retry_after_ms)
}

/// Uma linha, no máximo [`TAMANHO_MENSAGEM`] caracteres.
fn recortar(texto: &str) -> String {
    let limpo = texto.split_whitespace().collect::<Vec<_>>().join(" ");
    if limpo.chars().count() <= TAMANHO_MENSAGEM {
        return limpo;
    }
    let mut recorte: String = limpo.chars().take(TAMANHO_MENSAGEM).collect();
    recorte.push('…');
    recorte
}

/// Campo de texto obrigatório da resposta de sucesso.
fn texto_obrigatorio(
    valor: &Value,
    campo: &str,
    operacao: &'static str,
) -> Result<String, ErroChamada> {
    valor
        .get(campo)
        .and_then(Value::as_str)
        .filter(|texto| !texto.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ErroChamada::RespostaInvalida {
            operacao,
            motivo: format!("campo `{campo}` ausente ou vazio"),
        })
}

/// Caminho do evento de estado, com `state_key` quando existe.
fn caminho_estado(sala: &str, tipo: &str, chave: Option<&str>) -> String {
    let mut caminho = format!("{API}/rooms/{}/state/{}", codificar(sala), codificar(tipo));
    if let Some(chave) = chave.filter(|chave| !chave.is_empty()) {
        caminho.push('/');
        caminho.push_str(&codificar(chave));
    }
    caminho
}

/// Percent-encoding de um segmento de caminho (RFC 3986 `pchar`).
///
/// Ids de sala (`!abc:servidor`), aliases (`#ph_c_1:servidor`) e ids de usuário (`@ph_x:servidor`)
/// passam por aqui: `#` nunca vira fragmento, `/` nunca quebra o caminho.
fn codificar(segmento: &str) -> String {
    let mut saida = String::with_capacity(segmento.len());
    for byte in segmento.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':'
            | b'@' => saida.push(char::from(byte)),
            outro => saida.push_str(&format!("%{outro:02X}")),
        }
    }
    saida
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn caminho_de_estado_codifica_sala_tipo_e_chave() {
        assert_eq!(
            caminho_estado(
                "!abc:matrix-dev.local",
                "m.space.child",
                Some("!filha:matrix-dev.local")
            ),
            "/_matrix/client/v3/rooms/!abc:matrix-dev.local/state/m.space.child/!filha:matrix-dev.local"
        );
    }

    #[test]
    fn caminho_de_estado_sem_chave_nao_deixa_barra_no_fim() {
        for chave in [None, Some("")] {
            assert_eq!(
                caminho_estado("!abc:dev", "m.room.topic", chave),
                "/_matrix/client/v3/rooms/!abc:dev/state/m.room.topic"
            );
        }
    }

    #[test]
    fn codificar_nao_deixa_hash_quebrar_o_caminho_nem_escapa_id_de_sala() {
        assert_eq!(
            codificar("#ph_c_1:matrix-dev.local"),
            "%23ph_c_1:matrix-dev.local"
        );
        assert_eq!(codificar("!sala:a/b"), "!sala:a%2Fb");
        assert_eq!(codificar("a b"), "a%20b");
    }

    #[test]
    fn recortar_mantem_uma_linha_e_limita_o_tamanho() {
        let longa = "erro\ncom   quebras ".repeat(30);
        let recorte = recortar(&longa);
        assert!(!recorte.contains('\n'), "{recorte}");
        assert!(recorte.chars().count() <= TAMANHO_MENSAGEM + 1, "{recorte}");
    }

    #[test]
    fn partes_do_mxc_aceita_o_contrato_e_recusa_o_resto() {
        assert_eq!(
            partes_do_mxc("mxc://matrix-dev.local/abc123").expect("endereço válido"),
            ("matrix-dev.local".to_owned(), "abc123".to_owned())
        );
        // Espaço nas pontas vem de conteúdo de evento e não invalida o endereço.
        assert_eq!(
            partes_do_mxc("  mxc://s/id  ").expect("endereço válido"),
            ("s".to_owned(), "id".to_owned())
        );
        for invalido in [
            "",
            "https://exemplo/mxc",
            "mxc://",
            "mxc://servidor",
            "mxc:///id",
            "mxc://servidor/",
            "mxc://servidor/a/b",
        ] {
            assert!(
                matches!(partes_do_mxc(invalido), Err(ErroMidia::Endereco(_))),
                "deveria recusar: {invalido}"
            );
        }
    }

    #[test]
    fn detalhe_de_erro_le_so_os_campos_do_contrato() {
        let corpo = json!({
            "errcode": "M_FORBIDDEN",
            "error": "You are not allowed",
            "retry_after_ms": 700,
            "extra": "ignorado",
        });
        let (errcode, mensagem, retry) = detalhe_de_erro(&corpo);
        assert_eq!(errcode.as_deref(), Some("M_FORBIDDEN"));
        assert_eq!(mensagem, "You are not allowed");
        assert_eq!(retry, Some(700));
    }
}
