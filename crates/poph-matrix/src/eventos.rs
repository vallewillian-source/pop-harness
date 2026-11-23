//! Classificador dos eventos recebidos do Synapse — MX-17.
//!
//! Quando o atendente escreve na sala, o Synapse entrega o evento ao Application Service e o
//! receptor da MX-16 o repassa para cá. Antes de gravar qualquer coisa é preciso decidir **o que
//! aquele evento é**:
//!
//! - **resposta do atendente** (`m.room.message` de `m.text` ou `m.emote`, de gente de verdade);
//! - **mídia do atendente** (`m.image`, `m.file`, `m.audio`, `m.video`) — na fase 1 isso não vira
//!   resposta: quem avisa "envio de arquivo ainda não suportado" na sala é a MX-18;
//! - **ruído**, que não pode virar mensagem para o cliente de jeito nenhum.
//!
//! O ruído é a maior parte do que uma sala do Matrix produz: edição, reação, remoção, evento de
//! estado, `m.notice`, o próprio eco do que o módulo acabou de espelhar e o que o bot do Application
//! Service escreve. Cada um desses tem um motivo enumerado ([`MotivoIgnorar`]), com um código curto
//! que pode ir para log e métrica sem carregar texto de cliente.
//!
//! ## Pureza
//!
//! [`classificar`] não faz I/O, não fala com o banco, não fala com o Synapse e não olha o relógio:
//! recebe o evento cru da transação (o `serde_json::Value` que a MX-16 já validou) e o contexto do
//! módulo ([`IdsFantasmas`]), e devolve uma [`Acao`]. Isso é o que permite testar a tabela inteira de
//! tipos sem levantar servidor nenhum — e é por isso que ele mora em `poph-matrix`, junto do resto
//! da lógica do módulo, sem importar `poph-store`.
//!
//! ## Anti-eco
//!
//! Mensagem enviada por fantasma desta instância nunca é resposta de atendente — ela é o espelho do
//! que o próprio Harness publicou, e tratá-la como resposta criaria um laço (o módulo responderia a
//! si mesmo). A conferência é exata, em [`IdsFantasmas::eh_fantasma`]: mesmo domínio e `localpart`
//! no formato `<prefixo>_<marca>_<corpo>`. Um atendente humano cujo nome comece com o prefixo (por
//! exemplo `@ph_willian:…`) **não** é fantasma, e a resposta dele vale normalmente.

use serde_json::Value;

use crate::formato::{MSGTYPE_TEXTO, TIPO_EVENTO};
use crate::usuarios::IdsFantasmas;

/// `msgtype` de mensagem em itálico (a pessoa "entra" no personagem); é resposta de gente.
pub const MSGTYPE_EMOTE: &str = "m.emote";
/// `msgtype` de aviso automático de bot ou de integração: nunca é resposta de atendente.
pub const MSGTYPE_AVISO: &str = "m.notice";
/// Tipo do evento de reação (a reação em si não vem como `m.room.message`).
pub const TIPO_REACAO: &str = "m.reaction";
/// Tipo do evento de remoção (apagar mensagem).
pub const TIPO_REMOCAO: &str = "m.room.redaction";
/// `rel_type` de edição de mensagem (`m.replace`).
pub const RELACAO_SUBSTITUICAO: &str = "m.replace";
/// `rel_type` de reação (`m.annotation`).
pub const RELACAO_ANOTACAO: &str = "m.annotation";
/// Chave do conteúdo com o texto novo de uma edição.
pub const CAMPO_NOVO_CONTEUDO: &str = "m.new_content";

/// Tipo de mídia que o atendente tentou mandar.
///
/// Na fase 1 a mídia não vira resposta: a MX-18 responde na sala que envio de arquivo ainda não é
/// suportado. O tipo fica separado para a fase 2 poder tratar cada um do seu jeito e para o log
/// poder dizer o que chegou sem guardar o arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TipoMidia {
    /// `m.image` — foto.
    Imagem,
    /// `m.file` — documento qualquer.
    Arquivo,
    /// `m.audio` — áudio.
    Audio,
    /// `m.video` — vídeo.
    Video,
}

impl TipoMidia {
    /// Os quatro `msgtype` de mídia reconhecidos.
    pub const TODOS: [TipoMidia; 4] = [
        TipoMidia::Imagem,
        TipoMidia::Arquivo,
        TipoMidia::Audio,
        TipoMidia::Video,
    ];

    /// `msgtype` do Matrix correspondente.
    pub fn msgtype(self) -> &'static str {
        match self {
            TipoMidia::Imagem => "m.image",
            TipoMidia::Arquivo => "m.file",
            TipoMidia::Audio => "m.audio",
            TipoMidia::Video => "m.video",
        }
    }

    /// Traduz um `msgtype` de mídia; `None` para qualquer outro.
    pub fn de_msgtype(msgtype: &str) -> Option<Self> {
        TipoMidia::TODOS
            .into_iter()
            .find(|tipo| tipo.msgtype() == msgtype)
    }

    /// Código curto para log e métrica.
    pub fn codigo(self) -> &'static str {
        match self {
            TipoMidia::Imagem => "imagem",
            TipoMidia::Arquivo => "arquivo",
            TipoMidia::Audio => "audio",
            TipoMidia::Video => "video",
        }
    }
}

/// Por que um evento não vira resposta nem mídia.
///
/// O motivo é um código enumerado de propósito: ele pode ir para log e métrica sem carregar texto de
/// cliente, telefone ou conteúdo do evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotivoIgnorar {
    /// Mensagem do fantasma desta instância: é o eco do que o próprio módulo publicou.
    RemetenteFantasma,
    /// Mensagem do bot do Application Service (`<prefixo>_as_bot`).
    RemetenteBot,
    /// Evento sem `sender`: fora do contrato, não há de quem ser resposta.
    SemRemetente,
    /// Evento de estado (`state_key` presente), inclusive `m.room.member` e o `m.room.message
    /// .metadata` do módulo.
    EventoDeEstado,
    /// Edição de mensagem (`m.replace` ou `m.new_content`).
    Edicao,
    /// Reação (`m.reaction` ou `m.annotation`).
    Reacao,
    /// Remoção de mensagem (`m.room.redaction`).
    Remocao,
    /// Qualquer tipo que não é `m.room.message`.
    TipoNaoMensagem,
    /// `m.notice`: aviso de bot ou de integração.
    Aviso,
    /// `msgtype` que a fase 1 não trata (localização, enquete, verificação…).
    MsgtypeNaoSuportado,
    /// Conteúdo sem `msgtype` ou sem `body` de texto.
    ConteudoNaoTexto,
    /// Texto vazio ou só com espaço: não há o que mandar para o cliente.
    TextoVazio,
    /// Falta `room_id` ou `event_id`: sem eles não dá para gravar com idempotência.
    EventoIncompleto,
}

impl MotivoIgnorar {
    /// Código curto e estável, para log e métrica. Nunca carrega dado de cliente.
    pub fn codigo(self) -> &'static str {
        match self {
            MotivoIgnorar::RemetenteFantasma => "remetente_fantasma",
            MotivoIgnorar::RemetenteBot => "remetente_bot",
            MotivoIgnorar::SemRemetente => "sem_remetente",
            MotivoIgnorar::EventoDeEstado => "evento_de_estado",
            MotivoIgnorar::Edicao => "edicao",
            MotivoIgnorar::Reacao => "reacao",
            MotivoIgnorar::Remocao => "remocao",
            MotivoIgnorar::TipoNaoMensagem => "tipo_nao_mensagem",
            MotivoIgnorar::Aviso => "aviso",
            MotivoIgnorar::MsgtypeNaoSuportado => "msgtype_nao_suportado",
            MotivoIgnorar::ConteudoNaoTexto => "conteudo_nao_texto",
            MotivoIgnorar::TextoVazio => "texto_vazio",
            MotivoIgnorar::EventoIncompleto => "evento_incompleto",
        }
    }
}

impl std::fmt::Display for MotivoIgnorar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.codigo())
    }
}

/// O que fazer com um evento recebido do Synapse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acao {
    /// Resposta do atendente: grava e enfileira o envio para o cliente (MX-18).
    RespostaHumana {
        /// Sala de onde veio (o `room_id` do evento): é por ela que se acha a conversa.
        sala: String,
        /// Id do evento no Matrix; é a chave de idempotência do reenvio.
        evento_id: String,
        /// Id Matrix de quem escreveu (o atendente).
        remetente: String,
        /// Texto como o atendente escreveu, sem reformatar.
        texto: String,
    },
    /// Mídia do atendente: o arquivo vai ao cliente (MX-44) ou a sala recebe um aviso do motivo.
    MidiaDoAtendente {
        /// Sala de onde veio.
        sala: String,
        /// Id do evento no Matrix.
        evento_id: String,
        /// Id Matrix de quem mandou.
        remetente: String,
        /// Que tipo de mídia chegou, pelo `msgtype` do evento.
        tipo: TipoMidia,
        /// `content.url` do evento (`mxc://…`), quando veio: é por ele que o arquivo é baixado.
        url: Option<String>,
        /// Nome do arquivo: `filename`, ou `body` quando o evento não tem `filename`. Vazio quando
        /// não veio nenhum dos dois.
        nome: String,
    },
    /// Ruído: não vira nada. O motivo vai para log/métrica, sem conteúdo.
    Ignorar {
        /// Por que foi ignorado.
        motivo: MotivoIgnorar,
    },
}

/// Classifica um evento cru do Application Service. Função pura: mesma entrada, mesma saída.
///
/// A ordem das conferências é a que explica melhor o resultado: primeiro o que o evento **é** (tipo,
/// estado, remetente), depois o que o conteúdo diz (edição, reação, `msgtype`), e por último o que
/// falta para gravar (`room_id`, `event_id`).
pub fn classificar(ids: &IdsFantasmas, evento: &Value) -> Acao {
    let Some(tipo) = campo_texto(evento, "type") else {
        return ignorar(MotivoIgnorar::EventoIncompleto);
    };

    // Evento de estado vem com `state_key` — inclusive vazio (`m.room.member` de entrada na sala e
    // o `m.room.message.metadata` da MX-11). `state_key` é o que distingue, não o valor.
    if evento.get("state_key").is_some() {
        return ignorar(MotivoIgnorar::EventoDeEstado);
    }

    let Some(remetente) = campo_texto(evento, "sender") else {
        return ignorar(MotivoIgnorar::SemRemetente);
    };

    // Anti-eco antes de tudo: o que esta instância publicou não pode voltar como resposta dela
    // mesma. O bot do AS é conferido primeiro porque `eh_fantasma` não o cobre.
    if ids.eh_bot(remetente) {
        return ignorar(MotivoIgnorar::RemetenteBot);
    }
    if ids.eh_fantasma(remetente) {
        return ignorar(MotivoIgnorar::RemetenteFantasma);
    }

    match tipo {
        TIPO_REMOCAO => return ignorar(MotivoIgnorar::Remocao),
        TIPO_REACAO => return ignorar(MotivoIgnorar::Reacao),
        TIPO_EVENTO => {}
        _ => return ignorar(MotivoIgnorar::TipoNaoMensagem),
    }

    let Some(conteudo) = evento.get("content").filter(|c| c.is_object()) else {
        return ignorar(MotivoIgnorar::ConteudoNaoTexto);
    };

    // Edição chega como `m.room.message` com o texto novo em `m.new_content`; reagir a ela como
    // resposta mandaria o texto editado duas vezes para o cliente.
    if eh_edicao(conteudo) {
        return ignorar(MotivoIgnorar::Edicao);
    }
    if eh_reacao(conteudo) {
        return ignorar(MotivoIgnorar::Reacao);
    }

    let Some(msgtype) = conteudo.get("msgtype").and_then(Value::as_str) else {
        return ignorar(MotivoIgnorar::ConteudoNaoTexto);
    };

    match msgtype {
        MSGTYPE_TEXTO | MSGTYPE_EMOTE => {}
        MSGTYPE_AVISO => return ignorar(MotivoIgnorar::Aviso),
        outro => {
            return match TipoMidia::de_msgtype(outro) {
                Some(tipo_midia) => com_campos(evento, |sala, evento_id| Acao::MidiaDoAtendente {
                    sala: sala.to_owned(),
                    evento_id: evento_id.to_owned(),
                    remetente: remetente.to_owned(),
                    tipo: tipo_midia,
                    url: url_da_midia(conteudo),
                    nome: nome_da_midia(conteudo),
                }),
                None => ignorar(MotivoIgnorar::MsgtypeNaoSuportado),
            };
        }
    }

    let Some(texto) = conteudo.get("body").and_then(Value::as_str) else {
        return ignorar(MotivoIgnorar::ConteudoNaoTexto);
    };
    if texto.trim().is_empty() {
        return ignorar(MotivoIgnorar::TextoVazio);
    }

    com_campos(evento, |sala, evento_id| Acao::RespostaHumana {
        sala: sala.to_owned(),
        evento_id: evento_id.to_owned(),
        remetente: remetente.to_owned(),
        texto: texto.to_owned(),
    })
}

/// Aplica `montar` com `room_id` e `event_id`; se faltar algum, o evento é incompleto e não dá para
/// gravar com idempotência.
fn com_campos(evento: &Value, montar: impl FnOnce(&str, &str) -> Acao) -> Acao {
    match (
        campo_texto(evento, "room_id"),
        campo_texto(evento, "event_id"),
    ) {
        (Some(sala), Some(evento_id)) => montar(sala, evento_id),
        _ => ignorar(MotivoIgnorar::EventoIncompleto),
    }
}

/// `content.url` do evento de mídia, quando é texto não vazio.
fn url_da_midia(conteudo: &Value) -> Option<String> {
    conteudo
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

/// Nome do arquivo do evento de mídia: `filename` e, quando ele falta, `body`.
///
/// Nos dois campos o Element põe o nome do arquivo; o `filename` é o campo próprio para isso e o
/// `body` é o que aparece na linha da mensagem. Vazio quando nenhum dos dois veio (o nome padrão do
/// tipo é decisão de quem envia, em `midia_atendente::nome_do_arquivo`).
fn nome_da_midia(conteudo: &Value) -> String {
    for campo in ["filename", "body"] {
        if let Some(nome) = conteudo
            .get(campo)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|nome| !nome.is_empty())
        {
            return nome.to_owned();
        }
    }
    String::new()
}

/// O conteúdo é uma edição (`m.replace` ou texto novo em `m.new_content`)?
fn eh_edicao(conteudo: &Value) -> bool {
    conteudo.get(CAMPO_NOVO_CONTEUDO).is_some() || rel_type(conteudo) == Some(RELACAO_SUBSTITUICAO)
}

/// O conteúdo é uma reação (`m.annotation`)?
fn eh_reacao(conteudo: &Value) -> bool {
    rel_type(conteudo) == Some(RELACAO_ANOTACAO)
}

/// `content.m.relates_to.rel_type`, quando existe.
fn rel_type(conteudo: &Value) -> Option<&str> {
    conteudo
        .get("m.relates_to")
        .and_then(|relacao| relacao.get("rel_type"))
        .and_then(Value::as_str)
}

/// Campo de texto do evento, se existir e for string não vazia.
fn campo_texto<'a>(evento: &'a Value, campo: &str) -> Option<&'a str> {
    evento
        .get(campo)
        .and_then(Value::as_str)
        .filter(|valor| !valor.is_empty())
}

/// Atalho para o braço de ruído.
fn ignorar(motivo: MotivoIgnorar) -> Acao {
    Acao::Ignorar { motivo }
}

#[cfg(test)]
mod testes {
    use serde_json::json;

    use super::*;

    const SERVIDOR: &str = "matrix-dev.local";

    fn ids() -> IdsFantasmas {
        IdsFantasmas::novo("ph", SERVIDOR).expect("prefixo e servidor válidos")
    }

    /// Atendente humano desta instância (não é fantasma).
    const ATENDENTE: &str = "@willian:matrix-dev.local";

    /// Evento de mensagem bem formado, com o conteúdo informado.
    fn mensagem(sender: &str, conteudo: Value) -> Value {
        json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev1:matrix-dev.local",
            "sender": sender,
            "content": conteudo,
        })
    }

    /// Evento de mensagem com `msgtype` e corpo dados.
    fn com_msgtype(sender: &str, msgtype: &str, body: &str) -> Value {
        mensagem(sender, json!({ "msgtype": msgtype, "body": body }))
    }

    fn motivo(acao: Acao) -> MotivoIgnorar {
        match acao {
            Acao::Ignorar { motivo } => motivo,
            outra => panic!("esperava ignorar, veio {outra:?}"),
        }
    }

    fn texto(acao: Acao) -> String {
        match acao {
            Acao::RespostaHumana { texto, .. } => texto,
            outra => panic!("esperava resposta humana, veio {outra:?}"),
        }
    }

    #[test]
    fn texto_do_atendente_vira_resposta_com_sala_evento_e_remetente() {
        let acao = classificar(
            &ids(),
            &com_msgtype(ATENDENTE, "m.text", "Já estou verificando seu pedido."),
        );
        assert_eq!(
            acao,
            Acao::RespostaHumana {
                sala: "!abc:matrix-dev.local".to_owned(),
                evento_id: "$ev1:matrix-dev.local".to_owned(),
                remetente: ATENDENTE.to_owned(),
                texto: "Já estou verificando seu pedido.".to_owned(),
            }
        );
    }

    #[test]
    fn emote_do_atendente_tambem_vira_resposta() {
        assert_eq!(
            texto(classificar(
                &ids(),
                &com_msgtype(ATENDENTE, "m.emote", "confere o pedido")
            )),
            "confere o pedido"
        );
    }

    #[test]
    fn corpo_sai_igual_ao_que_o_atendente_escreveu() {
        // Sem reformatar: espaço, quebra de linha e emoji ficam como estão.
        let original = "  Olá!\n\nSeu pedido saiu 🚚  ";
        assert_eq!(
            texto(classificar(
                &ids(),
                &com_msgtype(ATENDENTE, "m.text", original)
            )),
            original
        );
    }

    #[test]
    fn cada_tipo_de_midia_vira_midia_do_atendente() {
        for tipo in TipoMidia::TODOS {
            let acao = classificar(&ids(), &com_msgtype(ATENDENTE, tipo.msgtype(), "foto.jpg"));
            assert_eq!(
                acao,
                Acao::MidiaDoAtendente {
                    sala: "!abc:matrix-dev.local".to_owned(),
                    evento_id: "$ev1:matrix-dev.local".to_owned(),
                    remetente: ATENDENTE.to_owned(),
                    tipo,
                    url: None,
                    nome: "foto.jpg".to_owned(),
                },
                "msgtype {} deveria ser mídia {:?}",
                tipo.msgtype(),
                tipo
            );
        }
    }

    #[test]
    fn midia_do_atendente_traz_url_e_nome_do_arquivo() {
        // Como o Element manda uma imagem: `body` e `filename` com o nome, e o `mxc://` em `url`.
        let evento = mensagem(
            ATENDENTE,
            json!({
                "msgtype": "m.image",
                "body": "foto.jpg",
                "filename": "foto.jpg",
                "url": "mxc://matrix-dev.local/abc123",
                "info": { "mimetype": "image/jpeg", "size": 2_048 },
            }),
        );
        match classificar(&ids(), &evento) {
            Acao::MidiaDoAtendente {
                tipo, url, nome, ..
            } => {
                assert_eq!(tipo, TipoMidia::Imagem);
                assert_eq!(url.as_deref(), Some("mxc://matrix-dev.local/abc123"));
                assert_eq!(nome, "foto.jpg");
            }
            outra => panic!("esperava mídia do atendente, veio {outra:?}"),
        }
    }

    #[test]
    fn nome_do_arquivo_prefere_filename_e_cai_no_body() {
        let com_filename = mensagem(
            ATENDENTE,
            json!({
                "msgtype": "m.file",
                "body": "clique para baixar",
                "filename": "laudo.pdf",
                "url": "mxc://matrix-dev.local/abc123",
            }),
        );
        assert!(matches!(
            classificar(&ids(), &com_filename),
            Acao::MidiaDoAtendente { nome, .. } if nome == "laudo.pdf"
        ));

        let so_body = mensagem(
            ATENDENTE,
            json!({ "msgtype": "m.file", "body": "  laudo.pdf  ", "url": "mxc://s/id" }),
        );
        assert!(matches!(
            classificar(&ids(), &so_body),
            Acao::MidiaDoAtendente { nome, .. } if nome == "laudo.pdf"
        ));

        // Sem nome nenhum: string vazia (o padrão do tipo é de quem envia).
        let sem_nome = mensagem(ATENDENTE, json!({ "msgtype": "m.image", "body": "   " }));
        match classificar(&ids(), &sem_nome) {
            Acao::MidiaDoAtendente { nome, url, .. } => {
                assert_eq!(nome, "");
                assert_eq!(url, None, "sem url, o arquivo não tem de onde ser baixado");
            }
            outra => panic!("esperava mídia do atendente, veio {outra:?}"),
        }
    }

    #[test]
    fn aviso_do_bot_nao_e_resposta_de_atendente() {
        assert_eq!(
            motivo(classificar(
                &ids(),
                &com_msgtype(ATENDENTE, "m.notice", "conectado")
            )),
            MotivoIgnorar::Aviso
        );
    }

    #[test]
    fn reacao_e_ignorada_nos_dois_formatos() {
        // Reação de verdade vem como evento próprio…
        let evento = json!({
            "type": "m.reaction",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev2:matrix-dev.local",
            "sender": ATENDENTE,
            "content": { "m.relates_to": { "rel_type": "m.annotation", "event_id": "$ev1" } },
        });
        assert_eq!(motivo(classificar(&ids(), &evento)), MotivoIgnorar::Reacao);

        // …e um `m.room.message` marcado como anotação também não é resposta.
        let esquisito = mensagem(
            ATENDENTE,
            json!({
                "msgtype": "m.text",
                "body": "👍",
                "m.relates_to": { "rel_type": "m.annotation", "event_id": "$ev1" },
            }),
        );
        assert_eq!(
            motivo(classificar(&ids(), &esquisito)),
            MotivoIgnorar::Reacao
        );
    }

    #[test]
    fn edicao_e_ignorada_pelo_rel_type_e_pelo_texto_novo() {
        let por_rel_type = mensagem(
            ATENDENTE,
            json!({
                "msgtype": "m.text",
                "body": "* pedido enviado",
                "m.relates_to": { "rel_type": "m.replace", "event_id": "$ev1" },
            }),
        );
        assert_eq!(
            motivo(classificar(&ids(), &por_rel_type)),
            MotivoIgnorar::Edicao
        );

        // Cliente que manda `m.new_content` sem `m.replace` também é edição: o corpo é o texto
        // novo, e mandá-lo como resposta duplicaria a mensagem.
        let por_texto_novo = mensagem(
            ATENDENTE,
            json!({
                "msgtype": "m.text",
                "body": "* pedido enviado",
                "m.new_content": { "msgtype": "m.text", "body": "pedido enviado" },
            }),
        );
        assert_eq!(
            motivo(classificar(&ids(), &por_texto_novo)),
            MotivoIgnorar::Edicao
        );
    }

    #[test]
    fn remocao_e_ignorada() {
        let evento = json!({
            "type": "m.room.redaction",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev3:matrix-dev.local",
            "sender": ATENDENTE,
            "redacts": "$ev1:matrix-dev.local",
            "content": {},
        });
        assert_eq!(motivo(classificar(&ids(), &evento)), MotivoIgnorar::Remocao);
    }

    #[test]
    fn evento_de_estado_e_ignorado_mesmo_com_state_key_vazio() {
        // `m.room.member` de quem entrou na sala: `state_key` é o id do membro.
        let membro = json!({
            "type": "m.room.member",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev4:matrix-dev.local",
            "sender": ATENDENTE,
            "state_key": ATENDENTE,
            "content": { "membership": "join" },
        });
        assert_eq!(
            motivo(classificar(&ids(), &membro)),
            MotivoIgnorar::EventoDeEstado
        );

        // `state_key` vazio também é evento de estado (o Sinapse devolve assim em alguns casos).
        let vazio = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev5:matrix-dev.local",
            "sender": ATENDENTE,
            "state_key": "",
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(
            motivo(classificar(&ids(), &vazio)),
            MotivoIgnorar::EventoDeEstado
        );
    }

    #[test]
    fn mensagem_do_bot_do_application_service_e_ignorada() {
        let bot = ids().bot();
        assert_eq!(bot.user_id(), "@ph_as_bot:matrix-dev.local");
        assert_eq!(
            motivo(classificar(
                &ids(),
                &com_msgtype(bot.user_id(), "m.text", "oi")
            )),
            MotivoIgnorar::RemetenteBot
        );
    }

    #[test]
    fn eco_dos_fantasmas_da_instancia_e_ignorado() {
        let ids = ids();
        for fantasma in [
            ids.cliente("+55 21 99999-0001").expect("cliente"),
            ids.agente("agente-1").expect("agente"),
            ids.aparelho("inst-1").expect("aparelho"),
        ] {
            assert_eq!(
                motivo(classificar(
                    &ids,
                    &com_msgtype(fantasma.user_id(), "m.text", "mensagem do próprio módulo")
                )),
                MotivoIgnorar::RemetenteFantasma,
                "fantasma {} deveria ser reconhecido como eco",
                fantasma
            );
        }
    }

    #[test]
    fn humano_com_localpart_parecida_com_o_prefixo_nao_e_fantasma() {
        // Nenhum destes é fantasma: falta a marca, sobra caractere na marca, não tem corpo, o
        // domínio é outro ou o prefixo é só parecido. Tratá-los como fantasma jogaria fora a
        // resposta de um atendente de verdade — em silêncio.
        for humano in [
            "@ph_willian:matrix-dev.local",
            "@ph:matrix-dev.local",
            "@ph_wa:matrix-dev.local",
            "@ph_wa_:matrix-dev.local",
            "@ph_wax_1:matrix-dev.local",
            "@ph_agente:matrix-dev.local",
            "@ph_as_bot_extra:matrix-dev.local",
            "@phx_wa_1:matrix-dev.local",
            "@wa_5511999999999:matrix-dev.local",
            "@ph_wa_5511999999999:outro.servidor",
            "willian",
        ] {
            let acao = classificar(&ids(), &com_msgtype(humano, "m.text", "oi"));
            assert!(
                matches!(acao, Acao::RespostaHumana { .. }),
                "{humano} deveria ser tratado como humano, veio {acao:?}"
            );
        }

        // E o fantasma exato continua sendo reconhecido (o teste anterior prova os três tipos).
        assert!(ids().eh_fantasma("@ph_wa_5511999999999:matrix-dev.local"));
        assert!(!ids().eh_fantasma("@ph_AG_1:matrix-dev.local"));
    }

    #[test]
    fn resposta_citando_outra_mensagem_continua_sendo_resposta() {
        let citacao = mensagem(
            ATENDENTE,
            json!({
                "msgtype": "m.text",
                "body": "> pergunta do cliente\n\nSim, temos.",
                "m.relates_to": { "rel_type": "m.in_reply_to", "event_id": "$ev1" },
            }),
        );
        assert_eq!(
            texto(classificar(&ids(), &citacao)),
            "> pergunta do cliente\n\nSim, temos."
        );
    }

    #[test]
    fn msgtype_que_a_fase_um_nao_trata_e_ignorado() {
        for msgtype in [
            "m.location",
            "m.key.verification.request",
            "com.exemplo.tipo",
        ] {
            assert_eq!(
                motivo(classificar(&ids(), &com_msgtype(ATENDENTE, msgtype, "x"))),
                MotivoIgnorar::MsgtypeNaoSuportado,
                "msgtype {msgtype}"
            );
        }
    }

    #[test]
    fn conteudo_sem_msgtype_ou_sem_texto_e_ignorado() {
        for conteudo in [
            json!({}),
            json!({ "body": "sem msgtype" }),
            json!({ "msgtype": "m.text" }),
            json!({ "msgtype": "m.text", "body": 42 }),
            json!({ "msgtype": 7, "body": "x" }),
            json!("não é objeto"),
        ] {
            assert_eq!(
                motivo(classificar(&ids(), &mensagem(ATENDENTE, conteudo.clone()))),
                MotivoIgnorar::ConteudoNaoTexto,
                "conteúdo {conteudo}"
            );
        }

        let sem_conteudo = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev6:matrix-dev.local",
            "sender": ATENDENTE,
        });
        assert_eq!(
            motivo(classificar(&ids(), &sem_conteudo)),
            MotivoIgnorar::ConteudoNaoTexto
        );
    }

    #[test]
    fn texto_vazio_nao_vira_resposta() {
        for body in ["", " ", "\n\t", "\u{00a0}"] {
            assert_eq!(
                motivo(classificar(&ids(), &com_msgtype(ATENDENTE, "m.text", body))),
                MotivoIgnorar::TextoVazio,
                "corpo {body:?}"
            );
        }
    }

    #[test]
    fn evento_sem_room_id_ou_sem_event_id_e_incompleto() {
        let sem_sala = json!({
            "type": "m.room.message",
            "event_id": "$ev7:matrix-dev.local",
            "sender": ATENDENTE,
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(
            motivo(classificar(&ids(), &sem_sala)),
            MotivoIgnorar::EventoIncompleto
        );

        let sem_id = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "sender": ATENDENTE,
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(
            motivo(classificar(&ids(), &sem_id)),
            MotivoIgnorar::EventoIncompleto
        );

        // Mídia também precisa dos dois campos: é por eles que a MX-18 acha a conversa.
        let midia_sem_sala = json!({
            "type": "m.room.message",
            "event_id": "$ev8:matrix-dev.local",
            "sender": ATENDENTE,
            "content": { "msgtype": "m.image", "body": "foto.jpg" },
        });
        assert_eq!(
            motivo(classificar(&ids(), &midia_sem_sala)),
            MotivoIgnorar::EventoIncompleto
        );
    }

    #[test]
    fn tipo_que_nao_e_mensagem_e_ignorado() {
        for tipo in [
            "m.typing",
            "m.receipt",
            "com.exemplo.evento",
            "m.call.invite",
        ] {
            let evento = json!({
                "type": tipo,
                "room_id": "!abc:matrix-dev.local",
                "event_id": "$ev9:matrix-dev.local",
                "sender": ATENDENTE,
                "content": { "msgtype": "m.text", "body": "oi" },
            });
            assert_eq!(
                motivo(classificar(&ids(), &evento)),
                MotivoIgnorar::TipoNaoMensagem,
                "tipo {tipo}"
            );
        }
    }

    #[test]
    fn evento_sem_sender_e_sem_type_e_ignorado() {
        let sem_sender = json!({
            "type": "m.room.message",
            "room_id": "!abc:matrix-dev.local",
            "event_id": "$ev10:matrix-dev.local",
            "content": { "msgtype": "m.text", "body": "oi" },
        });
        assert_eq!(
            motivo(classificar(&ids(), &sem_sender)),
            MotivoIgnorar::SemRemetente
        );

        let vazio = json!({});
        assert_eq!(
            motivo(classificar(&ids(), &vazio)),
            MotivoIgnorar::EventoIncompleto
        );
    }

    #[test]
    fn codigos_dos_motivos_sao_curtos_e_distintos_entre_si() {
        let motivos = [
            MotivoIgnorar::RemetenteFantasma,
            MotivoIgnorar::RemetenteBot,
            MotivoIgnorar::SemRemetente,
            MotivoIgnorar::EventoDeEstado,
            MotivoIgnorar::Edicao,
            MotivoIgnorar::Reacao,
            MotivoIgnorar::Remocao,
            MotivoIgnorar::TipoNaoMensagem,
            MotivoIgnorar::Aviso,
            MotivoIgnorar::MsgtypeNaoSuportado,
            MotivoIgnorar::ConteudoNaoTexto,
            MotivoIgnorar::TextoVazio,
            MotivoIgnorar::EventoIncompleto,
        ];
        let mut vistos: Vec<&str> = Vec::new();
        for motivo in motivos {
            let codigo = motivo.codigo();
            assert!(
                (3..=32).contains(&codigo.len()),
                "código {codigo:?} fora do tamanho aceitável"
            );
            assert!(
                codigo.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "código {codigo:?} deveria ser só a-z e _"
            );
            assert!(!vistos.contains(&codigo), "código repetido: {codigo}");
            vistos.push(codigo);
            assert_eq!(motivo.to_string(), codigo);
        }
    }

    #[test]
    fn tipo_de_midia_e_reconhecido_nos_dois_sentidos() {
        for tipo in TipoMidia::TODOS {
            assert_eq!(TipoMidia::de_msgtype(tipo.msgtype()), Some(tipo));
            assert!(tipo.codigo().chars().all(|c| c.is_ascii_lowercase()));
        }
        assert_eq!(TipoMidia::de_msgtype("m.text"), None);
    }
}
