//! Entrada do WhatsApp: o webhook da Evolution vira evento normalizado (HR-26).
//!
//! O que este módulo faz, e nada além: lê o corpo que a Evolution entrega, autentica, decide se é
//! mensagem do cliente ou evento de conexão, e devolve o [`InboundEvent`] do contrato (PRD §8.3)
//! mais a lista de mídias que ainda precisam ser baixadas. Gravar na `inbox`, deduplicar e rodar o
//! turno é da HR-12; baixar e guardar o arquivo é do cliente e do `poph-files`.
//!
//! Decisões que vieram do POP Atende (`core/server/routes/evolution_bridge.py` e
//! `core/conversations/whatsapp_media.py`), com o motivo:
//!
//! - **`fromMe` é descartado.** O webhook devolve a nossa própria mensagem como `fromMe = true`; sem
//!   o filtro, o agente conversaria consigo mesmo.
//! - **A chave de idempotência é o `key.id` do WhatsApp.** Reentrega do webhook não gera segundo
//!   turno (é o mesmo par `(integração, id externo)` da `inbox`).
//! - **Só imagem e documento (PDF/Office) entram como anexo.** Áudio, vídeo, figurinha, contato e
//!   localização são ignorados com motivo — a decisão do mantenedor é arquivo = imagem + PDF/Office,
//!   sem áudio.
//! - **Grupo vs. pessoa.** Em conversa individual o `peer` é o número (só dígitos, que é o que a
//!   Evolution aceita no envio); em grupo é o jid completo (`...@g.us`) e quem escreveu vai em
//!   `remetente`.
//! - **O segredo do webhook é obrigatório.** Sem segredo configurado o Harness recusa a entrega em
//!   vez de aceitar qualquer origem.

use base64::Engine as _;
use poph_types::{AnexoEntrada, EventoEstado, InboundEvent, IntegrationId, Segredo};
use serde_json::Value;
use thiserror::Error;

/// Evento de mensagem aceito (a Evolution manda em minúsculas e, em algumas versões, em maiúsculas).
pub const EVENTO_MENSAGEM: &str = "messages.upsert";

/// Nome do cabeçalho que carrega o segredo do webhook (o mesmo do POP Atende).
pub const CABECALHO_SEGREDO: &str = "X-Evolution-Webhook-Secret";

/// Prazo de guarda da mídia na Evolution, em dias (`docs/WHATSAPP_MEDIA.md`).
pub const DIAS_DE_MIDIA: i64 = 4;

/// O que deu errado ao autenticar ou ler o webhook.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ErroWebhook {
    /// A integração não tem segredo configurado: aceitar sem segredo seria aceitar qualquer origem.
    #[error("webhook recusado: a integração não tem segredo configurado")]
    SemSegredoConfigurado,
    /// A entrega veio sem o cabeçalho do segredo.
    #[error("webhook recusado: falta o cabeçalho `{CABECALHO_SEGREDO}`")]
    SegredoAusente,
    /// O segredo não confere.
    #[error("webhook recusado: segredo inválido")]
    SegredoInvalido,
}

/// Política de entrada da integração.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Politica {
    /// Aceitar mensagem com `fromMe = true` (só para teste; em produção fica `false`).
    pub aceitar_de_mim: bool,
}

/// Mídia que precisa ser baixada da Evolution antes de virar anexo.
///
/// O corpo do `getBase64FromMediaMessage` precisa do objeto `data` inteiro do webhook (só com o id
/// a Evolution responde "Message not found"/mídia expirada), então ele viaja aqui.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiaParaBaixar {
    /// Nome que o cliente deu ao arquivo (ou um nome montado a partir do id).
    pub nome: String,
    /// Tipo MIME informado pelo WhatsApp.
    pub mime: String,
    /// Id da mensagem no WhatsApp (a mesma chave de idempotência da entrada).
    pub message_id: String,
    /// Tamanho declarado pelo WhatsApp, quando ele informa (serve para recusar antes de baixar).
    pub tamanho_declarado: Option<u64>,
    /// O objeto `data` do webhook, para o download.
    pub dados: Value,
}

/// Entrada pronta para a `inbox`, com as mídias que faltam baixar.
#[derive(Debug, Clone, PartialEq)]
pub struct EntradaRecebida {
    /// Evento normalizado (sem anexos até o download terminar).
    pub evento: InboundEvent,
    /// Mídias a baixar e guardar antes de entregar o evento.
    pub midias: Vec<MidiaParaBaixar>,
}

impl EntradaRecebida {
    /// Evento final, com os anexos já guardados (blob) ou embutidos (bytes).
    pub fn com_anexos(mut self, anexos: Vec<AnexoEntrada>) -> InboundEvent {
        self.evento.anexos = anexos;
        self.evento
    }
}

/// O que o webhook significou.
#[derive(Debug, Clone, PartialEq)]
pub enum Decisao {
    /// Mensagem do cliente (texto e/ou anexo).
    Entrada(EntradaRecebida),
    /// Mudança de estado do canal (conectado, desconectado, QR).
    Estado(EventoEstado),
    /// Entrega reconhecida e descartada, com o motivo (vai para log e métrica).
    Ignorado(MotivoIgnorado),
}

/// Por que uma entrega foi descartada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotivoIgnorado {
    /// Evento que não é mensagem nem estado (`send.message`, `presence.update`…).
    EventoDesconhecido(String),
    /// Mensagem sem `key.id`: sem chave de idempotência não dá para garantir entrega única.
    SemId,
    /// Mensagem sem remetente (`remoteJid` ausente ou só o sufixo).
    SemRemetente,
    /// Eco da nossa própria mensagem.
    DeMimMesmo,
    /// Mensagem sem texto e sem mídia suportada (só reação, revogação, álbum…).
    SemConteudo,
    /// Mídia fora do escopo (áudio, vídeo, figurinha, contato, localização).
    MidiaNaoSuportada(String),
    /// Mídia com tipo MIME que o Harness não extrai.
    MimeNaoSuportado(String),
    /// Mensagem sem `messageTimestamp` válido (sem hora não há ordem de conversa).
    SemTimestamp,
}

/// Confere o segredo do webhook em tempo constante.
///
/// Comparação em tempo constante evita que o tamanho do prefixo correto vaze pelo tempo de resposta.
pub fn conferir_segredo(esperado: &Segredo, recebido: Option<&str>) -> Result<(), ErroWebhook> {
    if esperado.expor().trim().is_empty() {
        return Err(ErroWebhook::SemSegredoConfigurado);
    }
    let Some(recebido) = recebido else {
        return Err(ErroWebhook::SegredoAusente);
    };
    if igual_em_tempo_constante(esperado.expor().as_bytes(), recebido.trim().as_bytes()) {
        Ok(())
    } else {
        Err(ErroWebhook::SegredoInvalido)
    }
}

fn igual_em_tempo_constante(a: &[u8], b: &[u8]) -> bool {
    // O comprimento é público (o segredo é uma chave de origem), o conteúdo não.
    if a.len() != b.len() {
        return false;
    }
    let mut diferenca = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diferenca |= x ^ y;
    }
    diferenca == 0
}

/// Lê o corpo do webhook e devolve o que ele significa.
pub fn interpretar(integracao: IntegrationId, corpo: &Value, politica: &Politica) -> Decisao {
    let evento = corpo
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();

    if eh_evento_de_estado(&evento) {
        return Decisao::Estado(estado_do_canal(corpo, &evento));
    }
    if !evento.eq_ignore_ascii_case(EVENTO_MENSAGEM) {
        return Decisao::Ignorado(MotivoIgnorado::EventoDesconhecido(evento));
    }

    let dados = corpo.get("data").cloned().unwrap_or(Value::Null);
    let chave = dados.get("key").cloned().unwrap_or(Value::Null);

    let Some(id) = texto_em(&chave, "id") else {
        return Decisao::Ignorado(MotivoIgnorado::SemId);
    };
    let Some(jid) = texto_em(&chave, "remoteJid") else {
        return Decisao::Ignorado(MotivoIgnorado::SemRemetente);
    };
    let de_mim = chave
        .get("fromMe")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if de_mim && !politica.aceitar_de_mim {
        return Decisao::Ignorado(MotivoIgnorado::DeMimMesmo);
    }
    let Some(ocorrido_em) = timestamp_ms(&dados) else {
        return Decisao::Ignorado(MotivoIgnorado::SemTimestamp);
    };

    let conteudo = desembrulhar(dados.get("message").cloned().unwrap_or(Value::Null));
    let grupo = jid.ends_with("@g.us");
    let peer = if grupo {
        jid.clone()
    } else {
        numero_do_jid(&jid)
    };
    if peer.is_empty() {
        return Decisao::Ignorado(MotivoIgnorado::SemRemetente);
    }
    let remetente = if grupo {
        texto_em(&chave, "participant")
    } else {
        None
    };

    let mut texto = extrair_texto(&conteudo);
    let mut midias: Vec<MidiaParaBaixar> = Vec::new();

    if let Some(imagem) = conteudo.get("imageMessage") {
        let mime = texto_em(imagem, "mimetype").unwrap_or_else(|| "image/jpeg".to_owned());
        if texto.is_none() {
            texto = texto_em(imagem, "caption");
        }
        midias.push(MidiaParaBaixar {
            nome: nome_do_anexo(imagem, &id, &mime, "imagem"),
            mime,
            message_id: id.clone(),
            tamanho_declarado: tamanho_declarado(imagem),
            dados: dados.clone(),
        });
    } else if let Some(documento) = conteudo.get("documentMessage") {
        let mime = texto_em(documento, "mimetype")
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        if texto.is_none() {
            texto = texto_em(documento, "caption");
        }
        if !mime_suportado(&mime) {
            return Decisao::Ignorado(MotivoIgnorado::MimeNaoSuportado(mime));
        }
        midias.push(MidiaParaBaixar {
            nome: nome_do_anexo(documento, &id, &mime, "documento"),
            mime,
            message_id: id.clone(),
            tamanho_declarado: tamanho_declarado(documento),
            dados: dados.clone(),
        });
    } else if let Some(tipo) = midia_nao_suportada(&conteudo) {
        return Decisao::Ignorado(MotivoIgnorado::MidiaNaoSuportada(tipo));
    }

    if texto.is_none() && midias.is_empty() {
        return Decisao::Ignorado(MotivoIgnorado::SemConteudo);
    }

    let mut metadados = serde_json::json!({
        "canal": "whatsapp",
        "remote_jid": jid,
        "from_me": de_mim,
        "grupo": grupo,
        "instance": corpo.get("instance").and_then(Value::as_str),
    });
    // MX-32: o `pushName` é o nome que o cliente escolheu no WhatsApp. Só vale em mensagem recebida
    // de contato individual: em `fromMe` ele é o nome do dono da instância, e em grupo é o nome de
    // quem escreveu, não o do cliente da conversa. Nesses casos o campo não vai.
    if !de_mim
        && !grupo
        && let Some(nome) = texto_em(&dados, "pushName")
    {
        metadados["push_name"] = Value::String(nome.trim().to_owned());
    }

    let evento = InboundEvent {
        integracao,
        id_externo: id,
        peer: peer.clone(),
        remetente,
        texto: texto.unwrap_or_default(),
        anexos: Vec::new(),
        ocorrido_em,
        metadados,
    };

    Decisao::Entrada(EntradaRecebida { evento, midias })
}

/// Guarda o conteúdo de uma mídia como anexo embutido (o fluxo da HR-12 decide se grava blob).
pub fn anexo_embutido(nome: &str, mime: &str, bytes: &[u8]) -> AnexoEntrada {
    AnexoEntrada::Bytes {
        nome: nome.to_owned(),
        mime: mime.to_owned(),
        dados_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

fn eh_evento_de_estado(evento: &str) -> bool {
    matches!(
        evento.to_ascii_lowercase().as_str(),
        "connection.update" | "qrcode.updated" | "connection_update"
    )
}

fn estado_do_canal(corpo: &Value, evento: &str) -> EventoEstado {
    let dados = corpo.get("data").cloned().unwrap_or(Value::Null);
    if evento.to_ascii_lowercase().starts_with("qrcode") {
        let codigo = dados
            .get("qrcode")
            .and_then(|qrcode| texto_em(qrcode, "base64"))
            .or_else(|| texto_em(&dados, "base64"))
            .unwrap_or_default();
        return EventoEstado::Qr {
            codigo,
            expira_em: None,
        };
    }
    let situacao = texto_em(&dados, "state")
        .or_else(|| texto_em(&dados, "status"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    match situacao.as_str() {
        "open" | "connected" | "conectado" => EventoEstado::Conectado { desde: agora_ms() },
        "close" | "closed" | "disconnected" | "desconectado" => EventoEstado::Desconectado {
            motivo: Some("a Evolution reportou a instância desconectada".to_owned()),
        },
        outro => EventoEstado::Erro {
            codigo: "connection_state".to_owned(),
            mensagem: format!("estado de conexão `{outro}`"),
            transitorio: true,
        },
    }
}

/// Alguns eventos da Evolution vêm embrulhados (`ephemeralMessage`, `viewOnceMessage`).
fn desembrulhar(mensagem: Value) -> Value {
    let mut atual = mensagem;
    for _ in 0..4 {
        let proximo = atual
            .get("ephemeralMessage")
            .or_else(|| atual.get("viewOnceMessage"))
            .or_else(|| atual.get("viewOnceMessageV2"))
            .and_then(|embrulho| embrulho.get("message"))
            .cloned();
        match proximo {
            Some(proximo) => atual = proximo,
            None => break,
        }
    }
    atual
}

/// Texto da mensagem, na mesma ordem de preferência do POP Atende: texto simples, texto estendido,
/// legenda de imagem/vídeo/documento, resposta de botão e de lista.
fn extrair_texto(conteudo: &Value) -> Option<String> {
    let candidatos = [
        conteudo
            .get("conversation")
            .and_then(Value::as_str)
            .map(str::to_owned),
        conteudo
            .get("extendedTextMessage")
            .and_then(|valor| texto_em(valor, "text")),
        conteudo
            .get("imageMessage")
            .and_then(|valor| texto_em(valor, "caption")),
        conteudo
            .get("videoMessage")
            .and_then(|valor| texto_em(valor, "caption")),
        conteudo
            .get("documentMessage")
            .and_then(|valor| texto_em(valor, "caption")),
        conteudo
            .get("buttonsResponseMessage")
            .and_then(|valor| texto_em(valor, "selectedDisplayText")),
        conteudo
            .get("listResponseMessage")
            .and_then(|valor| valor.get("singleSelectReply"))
            .and_then(|valor| texto_em(valor, "selectedRowId")),
        conteudo
            .get("templateButtonReplyMessage")
            .and_then(|valor| texto_em(valor, "selectedDisplayText")),
    ];
    candidatos
        .into_iter()
        .flatten()
        .find(|candidato| !candidato.trim().is_empty())
        .map(|candidato| candidato.trim().to_owned())
}

fn midia_nao_suportada(conteudo: &Value) -> Option<String> {
    const TIPOS: [(&str, &str); 7] = [
        ("audioMessage", "audio"),
        ("videoMessage", "video"),
        ("stickerMessage", "figurinha"),
        ("contactMessage", "contato"),
        ("contactsArrayMessage", "contato"),
        ("locationMessage", "localizacao"),
        ("liveLocationMessage", "localizacao"),
    ];
    TIPOS
        .iter()
        .find(|(campo, _)| conteudo.get(*campo).is_some())
        .map(|(_, rotulo)| (*rotulo).to_owned())
}

/// Tipos que o Harness extrai texto (PRD: imagem + PDF/Office, sem áudio).
fn mime_suportado(mime: &str) -> bool {
    const ACEITOS: [&str; 6] = [
        "application/pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/msword",
        "application/vnd.ms-excel",
    ];
    let minusculo = mime.trim().to_ascii_lowercase();
    minusculo.starts_with("image/") || ACEITOS.contains(&minusculo.as_str())
}

fn nome_do_anexo(conteudo: &Value, id: &str, mime: &str, prefixo: &str) -> String {
    if let Some(nome) = texto_em(conteudo, "fileName") {
        return nome;
    }
    let minusculo = mime.trim().to_ascii_lowercase();
    let extensao = match minusculo.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        outro => outro.rsplit('/').next().unwrap_or("bin"),
    };
    format!("{prefixo}-{id}.{extensao}")
}

fn tamanho_declarado(conteudo: &Value) -> Option<u64> {
    // O WhatsApp manda `fileLength` como número, texto numérico ou Long do protobuf (`{low, high}`).
    let valor = conteudo.get("fileLength")?;
    if let Some(numero) = valor.as_u64() {
        return Some(numero);
    }
    if let Some(texto) = valor.as_str() {
        return texto.trim().parse::<u64>().ok();
    }
    let baixo = valor.get("low").and_then(Value::as_i64)? as u64 & 0xFFFF_FFFF;
    let alto = valor.get("high").and_then(Value::as_i64).unwrap_or(0) as u64 & 0xFFFF_FFFF;
    Some((alto << 32) | baixo)
}

/// Hora da mensagem em milissegundos. O WhatsApp manda em segundos; valor já em ms é respeitado.
fn timestamp_ms(dados: &Value) -> Option<i64> {
    let valor = dados.get("messageTimestamp")?;
    let segundos = if let Some(numero) = valor.as_i64() {
        numero
    } else {
        let texto = valor.as_str()?;
        texto.trim().parse::<i64>().ok()?
    };
    if segundos <= 0 {
        return None;
    }
    Some(if segundos > 10_000_000_000 {
        segundos
    } else {
        segundos * 1000
    })
}

/// `5511999999999:12@s.whatsapp.net` vira `5511999999999`.
fn numero_do_jid(jid: &str) -> String {
    let sem_sufixo = jid.split('@').next().unwrap_or_default();
    let sem_dispositivo = sem_sufixo.split(':').next().unwrap_or_default();
    sem_dispositivo
        .chars()
        .filter(char::is_ascii_digit)
        .collect()
}

fn texto_em(valor: &Value, campo: &str) -> Option<String> {
    valor
        .get(campo)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|texto| !texto.trim().is_empty())
}

fn agora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duracao| duracao.as_millis() as i64)
        .unwrap_or(0)
}
