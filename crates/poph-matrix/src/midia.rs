//! Mídia do cliente na sala (MX-43, fase 2).
//!
//! O anexo que o cliente manda pelo WhatsApp já está gravado quando o espelho passa por ele: uma
//! linha em `attachments` (nome, tipo, tamanho, sha256 e o caminho do blob) e o id dela em
//! `messages.attachments_json`. Este módulo transforma esse anexo no que o Element desenha: sobe o
//! arquivo ao Synapse **como o usuário virtual do cliente** e publica um `m.image` (imagem) ou um
//! `m.file` (PDF e Office) com o bloco `com.<marca>.attachment` que o frontend lê.
//!
//! ## O que fica aqui e o que não fica
//!
//! Aqui mora só o que dá para decidir sem rede e sem banco: qual `msgtype` o tipo MIME vira, se o
//! anexo cabe no limite, o corpo do evento, o `txnId` e o texto do aviso quando não dá para mostrar
//! o arquivo. A leitura do blob é do `poph-server` (que conhece a pasta de blobs) e a decisão de
//! quando chamar é do laço do espelho. A função [`enviar`] é a única que fala com o Synapse, e faz
//! as duas chamadas na ordem certa: sobe o arquivo, depois publica o evento.
//!
//! ## O bloco do anexo
//!
//! A chave do bloco **não é digitada à mão**: [`BLOCO_ANEXO`] é a mesma constante que o frontend
//! usa (`ATTACHMENT_BLOCK_KEY` em `apps/frontend/src/sofacoding/expiredAttachment.ts`), conferida
//! pelo teste de contrato `tests/contrato_frontend.rs`. É por esse bloco que o Element reconhece a
//! mídia do atendimento (e é ele que a expiração da MX-46 vai usar para redigir o evento e deixar o
//! aviso). O formato do conteúdo segue o que a projeção do Python já publica hoje nas mesmas salas:
//! `{ "id": <id do anexo>, "source": <canal> }`.
//!
//! ## Idempotência
//!
//! O `txnId` sai do id do anexo ([`txn_da_midia`]): a mesma mídia gera sempre o mesmo valor, e o
//! Synapse devolve o mesmo evento quando o `txnId` se repete (P7, comprovado na MX-02). Reprocessar
//! a mesma mensagem não duplica o evento na sala.
//!
//! **O upload não é idempotente.** O Synapse não tem chave para ele: uma repetição depois de uma
//! queda entre o upload e o evento deixa um arquivo sem evento na sala (lixo que a expiração da
//! MX-46 não alcança, porque ela procura pelo bloco do evento). O caminho normal não expõe isso ao
//! atendente; está registrado como limite conhecido no relatório da tarefa.
//!
//! ## Nada de dado de cliente em log
//!
//! O nome do arquivo e o id do anexo aparecem no evento (é o que o atendente precisa ver) e no
//! `?filename=` do upload. Nenhum deles vai para log, métrica ou mensagem de erro: os erros desta
//! camada são os do cliente da MX-09, que já recortam o corpo da resposta.

use serde_json::{Value, json};

use crate::cliente::ClienteMatrix;
use crate::erro::ErroChamada;
use crate::eventos::MSGTYPE_AVISO;
use crate::formato::TIPO_EVENTO;
use crate::usuarios::Fantasma;

/// Maior arquivo que vai para a sala (o mesmo teto do recebimento e o do PRD).
pub const LIMITE_MIDIA_BYTES: u64 = 16 * 1024 * 1024;

/// Chave do bloco que o frontend lê no evento de mídia (`ATTACHMENT_BLOCK_KEY` do Element).
///
/// Conferida contra o arquivo do frontend pelo teste de contrato: renomear de um lado só quebra o
/// teste, não o atendimento.
pub const BLOCO_ANEXO: &str = "com.sofacoding.attachment";

/// Campo do bloco com o id do anexo no Harness.
pub const CAMPO_ID: &str = "id";

/// Campo do bloco com o canal de onde o anexo veio (`whatsapp`, `console`…).
pub const CAMPO_FONTE: &str = "source";

/// `msgtype` de imagem.
pub const MSGTYPE_IMAGEM: &str = "m.image";

/// `msgtype` de arquivo (PDF, Office e qualquer outro tipo que não seja imagem).
pub const MSGTYPE_ARQUIVO: &str = "m.file";

/// Prefixo do `txnId` da mídia. Fixo: trocar o prefixo da instância não muda o `txnId`, senão um
/// reenvio depois de uma mudança de configuração duplicaria o evento.
pub const PREFIXO_TXN_MIDIA: &str = "poph-midia-";

/// Prefixo do `txnId` do aviso de mídia que não entrou na sala.
pub const PREFIXO_TXN_AVISO: &str = "poph-midia-aviso-";

/// Começo do aviso na sala quando o arquivo do cliente não aparece. O motivo curto vem depois.
pub const PREFIXO_AVISO: &str = "Não consegui mostrar na sala o arquivo que o cliente enviou: ";

/// Anexo do Harness, com o que a decisão precisa. Quem monta é quem lê o banco.
#[derive(Debug, Clone, Copy)]
pub struct AnexoParaMidia<'a> {
    /// Id do anexo (`attachments.id`): vai no bloco e no `txnId`.
    pub id: &'a str,
    /// Nome que o cliente deu ao arquivo.
    pub nome: &'a str,
    /// Tipo MIME real, pelo conteúdo (o `mime` da tabela).
    pub mime: &'a str,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// Canal da conversa, como o Harness o chama (`whatsapp`, `console`…).
    pub fonte: &'a str,
}

/// Por que o anexo não entra na sala.
///
/// Cada motivo tem um código curto (vai para log e para a coluna de falha, sem dado de cliente) e
/// um texto em português (vai para a sala).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotivoDeRecusa {
    /// O arquivo passa do limite ([`LIMITE_MIDIA_BYTES`]).
    Grande,
    /// O arquivo tem zero byte.
    Vazio,
    /// O blob não pôde ser lido no disco (arquivo ausente, pasta de blobs não configurada).
    Ilegivel,
    /// O Synapse recusou o envio de vez (definitivo, depois das tentativas).
    Rejeitada,
}

impl MotivoDeRecusa {
    /// Código curto para log, métrica e `matrix_falhas`.
    pub fn codigo(self) -> &'static str {
        match self {
            Self::Grande => "arquivo_grande",
            Self::Vazio => "arquivo_vazio",
            Self::Ilegivel => "arquivo_ilegivel",
            Self::Rejeitada => "midia_rejeitada",
        }
    }

    /// Texto que vai à sala, depois de [`PREFIXO_AVISO`].
    pub fn texto(self) -> &'static str {
        match self {
            Self::Grande => "o arquivo passa de 16 MB",
            Self::Vazio => "o arquivo está vazio",
            Self::Ilegivel => "o arquivo não pôde ser lido",
            Self::Rejeitada => "o serviço de atendimento recusou o arquivo",
        }
    }
}

/// Mídia decidida: tudo o que a subida e o evento precisam, menos o conteúdo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Midia {
    /// Id do anexo no Harness (no bloco, no `txnId` e na falha).
    pub id_anexo: String,
    /// `m.image` ou `m.file`.
    pub msgtype: &'static str,
    /// Nome que vai no `body` e no `filename` (é o que o Element mostra).
    pub nome: String,
    /// Tipo MIME real.
    pub mime: String,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// Canal de onde o anexo veio (vai no bloco).
    pub fonte: String,
    /// `txnId` do evento: estável para o mesmo anexo.
    pub txn_id: String,
}

/// Decide a mídia de um anexo. Função pura: mesma entrada, mesma saída.
///
/// Imagem (qualquer `image/…`) vira `m.image`; todo o resto vira `m.file`. Nome vazio ganha o
/// padrão do tipo, para o Element não mostrar uma linha em branco.
pub fn decidir(anexo: &AnexoParaMidia<'_>) -> Result<Midia, MotivoDeRecusa> {
    if anexo.tamanho_bytes == 0 {
        return Err(MotivoDeRecusa::Vazio);
    }
    if anexo.tamanho_bytes > LIMITE_MIDIA_BYTES {
        return Err(MotivoDeRecusa::Grande);
    }
    let imagem = anexo.mime.trim().to_ascii_lowercase().starts_with("image/");
    let msgtype = if imagem {
        MSGTYPE_IMAGEM
    } else {
        MSGTYPE_ARQUIVO
    };
    let nome = nome_visivel(anexo.nome, imagem);
    Ok(Midia {
        id_anexo: anexo.id.to_owned(),
        msgtype,
        nome,
        mime: anexo.mime.trim().to_owned(),
        tamanho_bytes: anexo.tamanho_bytes,
        fonte: anexo.fonte.trim().to_owned(),
        txn_id: txn_da_midia(anexo.id),
    })
}

/// Nome que o Element mostra: o do cliente, ou o padrão do tipo quando veio vazio.
fn nome_visivel(nome: &str, imagem: bool) -> String {
    let limpo = nome.trim();
    if !limpo.is_empty() {
        return limpo.to_owned();
    }
    if imagem {
        "imagem".to_owned()
    } else {
        "arquivo".to_owned()
    }
    .to_owned()
}

/// `txnId` do evento de mídia: `poph-midia-` mais o id do anexo. Só letras, números e hífen, então
/// sai direto no caminho da URL sem escape.
pub fn txn_da_midia(id_anexo: &str) -> String {
    format!("{PREFIXO_TXN_MIDIA}{id_anexo}")
}

/// `txnId` do aviso de uma mídia que não entrou na sala. O motivo entra na chave: dois motivos
/// diferentes para o mesmo anexo não se confundem no Synapse.
pub fn txn_do_aviso(id_anexo: &str, motivo: MotivoDeRecusa) -> String {
    format!("{PREFIXO_TXN_AVISO}{id_anexo}-{}", motivo.codigo())
}

/// Corpo do aviso na sala para um motivo.
pub fn corpo_do_aviso(motivo: MotivoDeRecusa) -> String {
    format!("{PREFIXO_AVISO}{}", motivo.texto())
}

/// Conteúdo do evento `m.image`/`m.file`, com o bloco que o frontend lê.
///
/// `body` e `filename` são o nome do arquivo (é o que o Element e o cliente do WhatsApp usam) e
/// `info` leva tipo e tamanho. O bloco `com.<marca>.attachment` é **obrigatório**: é por ele que o
/// frontend reconhece a mídia do atendimento.
pub fn conteudo_do_evento(mxc: &str, midia: &Midia) -> Value {
    json!({
        "msgtype": midia.msgtype,
        "body": midia.nome,
        "filename": midia.nome,
        "url": mxc,
        "info": { "mimetype": midia.mime, "size": midia.tamanho_bytes },
        BLOCO_ANEXO: { CAMPO_ID: midia.id_anexo, CAMPO_FONTE: midia.fonte },
    })
}

/// Conteúdo do aviso (`m.notice`) de uma mídia que não entrou na sala.
pub fn conteudo_do_aviso(motivo: MotivoDeRecusa) -> Value {
    json!({ "msgtype": MSGTYPE_AVISO, "body": corpo_do_aviso(motivo) })
}

/// Sobe o arquivo e publica o evento na sala, como o usuário virtual do cliente.
///
/// Duas chamadas ao Synapse, nesta ordem: `upload` (devolve o `mxc://`) e `send` do evento com o
/// `txnId` da mídia. Devolve o id do evento criado. Uma falha em qualquer uma delas volta como
/// [`ErroChamada`], com a classificação que o laço do espelho usa para decidir entre repetir e
/// isolar.
pub async fn enviar(
    cliente: &ClienteMatrix,
    remetente: &Fantasma,
    sala: &str,
    midia: &Midia,
    conteudo: &[u8],
) -> Result<String, ErroChamada> {
    let mxc = cliente
        .enviar_midia(remetente.user_id(), &midia.nome, &midia.mime, conteudo)
        .await?;
    cliente
        .enviar_evento(
            remetente.user_id(),
            sala,
            TIPO_EVENTO,
            &midia.txn_id,
            &conteudo_do_evento(&mxc, midia),
        )
        .await
}

#[cfg(test)]
mod testes {
    use super::*;

    fn anexo<'a>(id: &'a str, nome: &'a str, mime: &'a str, tamanho: u64) -> AnexoParaMidia<'a> {
        AnexoParaMidia {
            id,
            nome,
            mime,
            tamanho_bytes: tamanho,
            fonte: "whatsapp",
        }
    }

    #[test]
    fn imagem_vira_m_image_e_o_resto_vira_m_file() {
        for (mime, esperado) in [
            ("image/jpeg", MSGTYPE_IMAGEM),
            ("IMAGE/PNG", MSGTYPE_IMAGEM),
            ("application/pdf", MSGTYPE_ARQUIVO),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                MSGTYPE_ARQUIVO,
            ),
            ("application/octet-stream", MSGTYPE_ARQUIVO),
        ] {
            let midia = decidir(&anexo("a1", "arquivo.bin", mime, 10)).expect("decide");
            assert_eq!(midia.msgtype, esperado, "mime {mime}");
        }
    }

    #[test]
    fn limite_de_dezesseis_megabytes_e_do_anexo_e_nao_do_evento() {
        let no_limite = decidir(&anexo("a1", "foto.jpg", "image/jpeg", LIMITE_MIDIA_BYTES))
            .expect("no limite entra");
        assert_eq!(no_limite.tamanho_bytes, LIMITE_MIDIA_BYTES);

        let acima = decidir(&anexo(
            "a2",
            "foto.jpg",
            "image/jpeg",
            LIMITE_MIDIA_BYTES + 1,
        ));
        assert_eq!(acima, Err(MotivoDeRecusa::Grande));

        assert_eq!(
            decidir(&anexo("a3", "vazio.pdf", "application/pdf", 0)),
            Err(MotivoDeRecusa::Vazio)
        );
    }

    #[test]
    fn nome_vazio_ganha_o_padrao_do_tipo() {
        let imagem = decidir(&anexo("a1", "  ", "image/png", 5)).expect("decide");
        assert_eq!(imagem.nome, "imagem");
        let arquivo = decidir(&anexo("a2", "", "application/pdf", 5)).expect("decide");
        assert_eq!(arquivo.nome, "arquivo");
        let com_espaco =
            decidir(&anexo("a3", "  laudo.pdf ", "application/pdf", 5)).expect("decide");
        assert_eq!(com_espaco.nome, "laudo.pdf");
    }

    #[test]
    fn txn_e_estavel_e_diferente_por_anexo_e_por_motivo() {
        assert_eq!(txn_da_midia("anx-1"), txn_da_midia("anx-1"));
        assert_ne!(txn_da_midia("anx-1"), txn_da_midia("anx-2"));
        assert_ne!(
            txn_do_aviso("anx-1", MotivoDeRecusa::Grande),
            txn_do_aviso("anx-1", MotivoDeRecusa::Ilegivel)
        );
        // Só o que cabe no caminho da URL sem escape.
        for txn in [
            txn_da_midia("018f-0000-7000-8000-000000000001"),
            txn_do_aviso(
                "018f-0000-7000-8000-000000000001",
                MotivoDeRecusa::Rejeitada,
            ),
        ] {
            // Só o que a RFC 3986 deixa passar num segmento de caminho sem escape.
            assert!(
                txn.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')),
                "txnId precisa ser seguro em URL: {txn}"
            );
        }
    }

    #[test]
    fn evento_leva_o_bloco_do_frontend_com_id_e_fonte() {
        let midia =
            decidir(&anexo("anx-9", "laudo.pdf", "application/pdf", 2_048)).expect("decide");
        let evento = conteudo_do_evento("mxc://matrix-dev.local/abc123", &midia);

        assert_eq!(evento["msgtype"], json!("m.file"));
        assert_eq!(evento["body"], json!("laudo.pdf"));
        assert_eq!(evento["filename"], json!("laudo.pdf"));
        assert_eq!(evento["url"], json!("mxc://matrix-dev.local/abc123"));
        assert_eq!(evento["info"]["mimetype"], json!("application/pdf"));
        assert_eq!(evento["info"]["size"], json!(2_048));
        assert_eq!(
            evento[BLOCO_ANEXO],
            json!({ "id": "anx-9", "source": "whatsapp" })
        );
    }

    #[test]
    fn imagem_tambem_leva_o_bloco() {
        let midia = decidir(&anexo("anx-1", "foto.jpg", "image/jpeg", 10)).expect("decide");
        let evento = conteudo_do_evento("mxc://s/x", &midia);
        assert_eq!(evento["msgtype"], json!("m.image"));
        assert_eq!(evento[BLOCO_ANEXO]["id"], json!("anx-1"));
    }

    #[test]
    fn aviso_diz_o_motivo_sem_nome_de_arquivo() {
        for motivo in [
            MotivoDeRecusa::Grande,
            MotivoDeRecusa::Vazio,
            MotivoDeRecusa::Ilegivel,
            MotivoDeRecusa::Rejeitada,
        ] {
            let aviso = conteudo_do_aviso(motivo);
            assert_eq!(aviso["msgtype"], json!("m.notice"));
            let corpo = aviso["body"].as_str().expect("corpo é texto");
            assert!(corpo.starts_with(PREFIXO_AVISO), "corpo: {corpo}");
            assert!(corpo.ends_with(motivo.texto()), "corpo: {corpo}");
            assert!(!corpo.contains("laudo"), "o aviso não leva nome de arquivo");
        }
    }
}
