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
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::SegredosMatrix;
use crate::erro::ErroChamada;

/// Base da API cliente do Matrix.
const API: &str = "/_matrix/client/v3";

/// Prazo padrão de cada chamada ao Synapse.
pub const TEMPO_LIMITE_PADRAO: Duration = Duration::from_secs(10);

/// Espera máxima que o cliente aceita cumprir sozinho num `429`. Acima disso ele devolve o erro
/// com `retry_after_ms` e quem espera é o laço (uma espera de minutos não pode travar o cliente).
pub const ESPERA_MAXIMA_429: Duration = Duration::from_secs(60);

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

struct Interno {
    http: reqwest::Client,
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
        Ok(Self(Arc::new(Interno { http, config })))
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
                Some(&corpo),
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
            Some(&json!({ "user_id": convidado })),
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
                Some(&json!({})),
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
                None,
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
                None,
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
                Some(corpo),
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
                Some(corpo),
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
            None,
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
                None,
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
                None,
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
            Some(&json!({ "displayname": nome })),
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
                Some(&json!({
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
            None,
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
        corpo: Option<&Value>,
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
        corpo: Option<&Value>,
        operacao: &'static str,
    ) -> Result<Value, ErroChamada> {
        let resultado = self
            .tentar(&metodo, &caminho, consulta_fixa, usuario, corpo, operacao)
            .await;
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
                    self.tentar(&metodo, &caminho, consulta_fixa, usuario, corpo, operacao)
                        .await
                } else {
                    Err(erro)
                }
            }
        }
    }

    /// Uma requisição HTTP, sem repetição.
    async fn tentar(
        &self,
        metodo: &Method,
        caminho: &str,
        consulta_fixa: &[(&str, &str)],
        usuario: Option<&str>,
        corpo: Option<&Value>,
        operacao: &'static str,
    ) -> Result<Value, ErroChamada> {
        let url = format!("{}{}", self.0.config.url_base, caminho);
        let autorizacao =
            HeaderValue::from_str(&format!("Bearer {}", self.0.config.as_token.expor().trim()))
                .map_err(|_| {
                    ErroChamada::Configuracao(
                "o token do Application Service tem caractere que não cabe em cabeçalho HTTP"
                    .to_owned(),
            )
                })?;

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
        if let Some(corpo) = corpo {
            pedido = pedido.json(corpo);
        }

        let resposta = pedido.send().await.map_err(|erro| {
            if erro.is_timeout() {
                ErroChamada::Prazo {
                    operacao,
                    ms: u64::try_from(self.0.config.timeout.as_millis()).unwrap_or(u64::MAX),
                }
            } else if erro.is_connect() {
                ErroChamada::Rede {
                    operacao,
                    motivo: "não consegui conectar".to_owned(),
                }
            } else {
                // `without_url` tira o endereço (que carrega o id da sala) da mensagem.
                ErroChamada::Rede {
                    operacao,
                    motivo: erro.without_url().to_string(),
                }
            }
        })?;

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
