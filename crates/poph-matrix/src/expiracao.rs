//! Expiração da mídia na sala (MX-46, fase 2).
//!
//! Depois de `matrix.midia_retencao_dias` (padrão 4), a mídia que o espelho publicou numa sala é
//! apagada do repositório do Synapse e o evento é redigido. Aqui mora o que dá para decidir sem
//! banco e sem laço: **o conteúdo da redação**, o `txnId` dela, o instante em ISO-8601 que o
//! frontend lê e as duas chamadas ao Synapse.
//!
//! ## O que o frontend espera (não invente formato)
//!
//! O Element lê `unsigned.redacted_because.content["com.sofacoding.attachment_expired"]` e desenha
//! "📎 nome — enviado em dd/mm — não está mais disponível" (`expiredAttachmentMarker.ts`). Os campos
//! são `kind` (`image`/`document`; qualquer coisa que não seja `image` o Element trata como arquivo),
//! `filename` e `sent_at` (ISO-8601), mais `mimetype` e `size`. A chave e os campos **não são
//! digitados à mão aqui**: o teste de contrato
//! (`tests/contrato_frontend.rs`) lê o arquivo do frontend e compara.
//!
//! É o mesmo conteúdo que a projeção do Python já publica hoje (`expiry_redaction_content` em
//! `whatsapp_media_expiry_worker.py`), e é por isso que as duas origens desenham o mesmo aviso.
//!
//! ## As duas chamadas, e por que nesta ordem
//!
//! 1. **redigir** (`PUT /rooms/{sala}/redact/{evento}/{txn}`, como o fantasma do **agente**, que é
//!    moderador da sala) — o evento de mídia foi publicado pelo fantasma do cliente, e ele não tem
//!    poder para apagar o próprio evento depois de publicado;
//! 2. **apagar** (`DELETE /_synapse/admin/v1/media/{servidor}/{id}`, com o token de administração) —
//!    é a retenção, que vale mais do que a arrumação da conversa: se a redação falhar, o arquivo é
//!    apagado do mesmo jeito.
//!
//! As duas são idempotentes do lado do Synapse: o mesmo `txnId` não cria uma segunda redação e o
//! `DELETE` de mídia que já não existe responde `404`, contado como apagada.
//!
//! ## Nada de segredo nem de dado de cliente em log
//!
//! O nome do arquivo vai **no evento** (o atendente precisa vê-lo), nunca em log, métrica ou
//! mensagem de erro. O token de administração é `Sensivel` e só vira cabeçalho `Authorization`.

use serde_json::{Value, json};

use crate::cliente::ClienteMatrix;
use crate::erro::{ErroChamada, ErroMidia};
use crate::usuarios::Fantasma;
use poph_types::Segredo;

/// Chave do bloco que o frontend lê na redação (`ATTACHMENT_EXPIRED_KEY` do Element).
///
/// Conferida contra o arquivo do frontend pelo teste de contrato.
pub const BLOCO_EXPIRADO: &str = "com.sofacoding.attachment_expired";

/// Campo do bloco com o nome do arquivo (`filename` no Element).
pub const CAMPO_NOME: &str = "filename";

/// Campo do bloco com o tipo MIME.
pub const CAMPO_TIPO: &str = "mimetype";

/// Campo do bloco com o tamanho em bytes.
pub const CAMPO_TAMANHO: &str = "size";

/// Campo do bloco com o instante em que o arquivo chegou à sala, em ISO-8601 UTC (`sent_at`).
pub const CAMPO_ENVIADA_EM: &str = "sent_at";

/// Campo do bloco com a categoria do anexo (`kind`: `image` ou `document`).
pub const CAMPO_CATEGORIA: &str = "kind";

/// Motivo que vai no `reason` da redação (o Element não lê, o Synapse mostra na administração).
pub const MOTIVO_DA_REDACAO: &str = "arquivo expirado: o prazo de retenção terminou";

/// Prefixo do `txnId` da redação. Fixo: trocar o prefixo da instância não muda a chave, senão uma
/// segunda passagem depois de mudar a configuração criaria outra redação.
pub const PREFIXO_TXN_REDACAO: &str = "poph-expira-";

/// A mídia que vai expirar, com o que a redação precisa (quem lê o banco monta).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiaParaExpirar<'a> {
    /// Id do anexo no Harness.
    pub id: &'a str,
    /// Nome do arquivo, como está no evento.
    pub nome: &'a str,
    /// Tipo MIME real.
    pub mime: &'a str,
    /// Categoria: `image` ou `document` (a mesma que o evento declara).
    pub categoria: &'a str,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// Quando o arquivo entrou no Synapse (ms desde a época).
    pub enviada_em: i64,
}

/// `txnId` da redação: `poph-expira-` mais o id do anexo. Só letras, números e hífen, então sai
/// direto no caminho da URL sem escape.
pub fn txn_da_redacao(id_anexo: &str) -> String {
    format!("{PREFIXO_TXN_REDACAO}{id_anexo}")
}

/// Conteúdo da redação: o bloco que o frontend lê, com o motivo.
///
/// Nada de conteúdo do arquivo: só nome, tipo, tamanho, categoria e quando ele chegou.
pub fn conteudo_da_redacao(midia: &MidiaParaExpirar<'_>) -> Value {
    json!({
        "reason": MOTIVO_DA_REDACAO,
        BLOCO_EXPIRADO: {
            CAMPO_NOME: midia.nome,
            CAMPO_TIPO: midia.mime,
            CAMPO_TAMANHO: midia.tamanho_bytes,
            CAMPO_ENVIADA_EM: instante_iso(midia.enviada_em),
            CAMPO_CATEGORIA: midia.categoria,
        },
    })
}

/// Instante em ISO-8601 UTC, com segundos (`2026-11-22T14:42:00Z`).
///
/// É o formato que `new Date("…")` do frontend lê sem depender do idioma do navegador. A aritmética
/// é inteira (algoritmo de Howard Hinnant, `civil_from_days`), sem dependência de data.
pub fn instante_iso(ms: i64) -> String {
    let segundos = ms.div_euclid(1_000);
    let dias = segundos.div_euclid(86_400);
    let no_dia = segundos.rem_euclid(86_400);
    let (ano, mes, dia) = data_civil(dias);
    let hora = no_dia / 3_600;
    let minuto = (no_dia % 3_600) / 60;
    let segundo = no_dia % 60;
    format!("{ano:04}-{mes:02}-{dia:02}T{hora:02}:{minuto:02}:{segundo:02}Z")
}

/// Data civil (ano, mês, dia) de um número de dias desde 1970-01-01, em UTC.
fn data_civil(dias: i64) -> (i64, u32, u32) {
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let dia_da_era = z.rem_euclid(146_097);
    let ano_da_era =
        (dia_da_era - dia_da_era / 1_460 + dia_da_era / 36_524 - dia_da_era / 146_096) / 365;
    let ano = ano_da_era + era * 400;
    let dia_do_ano = dia_da_era - (365 * ano_da_era + ano_da_era / 4 - ano_da_era / 100);
    let mes_ajustado = (5 * dia_do_ano + 2) / 153;
    let dia = dia_do_ano - (153 * mes_ajustado + 2) / 5 + 1;
    let mes = if mes_ajustado < 10 {
        mes_ajustado + 3
    } else {
        mes_ajustado - 9
    };
    let ano = if mes <= 2 { ano + 1 } else { ano };
    (ano, mes as u32, dia as u32)
}

/// Redige o evento de mídia na sala, como o fantasma de `remetente`.
///
/// Devolve o id do evento de redação. Não repete: quem decide esperar é a rotina de expiração, que
/// adia o item com espera crescente.
pub async fn redigir(
    cliente: &ClienteMatrix,
    remetente: &Fantasma,
    sala: &str,
    evento: &str,
    midia: &MidiaParaExpirar<'_>,
) -> Result<String, ErroChamada> {
    cliente
        .redigir(
            remetente.user_id(),
            sala,
            evento,
            &txn_da_redacao(midia.id),
            &conteudo_da_redacao(midia),
        )
        .await
}

/// Apaga o arquivo do repositório do Synapse pela API administrativa.
///
/// `true` quando existia e foi apagado; `false` quando já não existia (404 conta como apagado).
pub async fn apagar(
    cliente: &ClienteMatrix,
    token_admin: &Segredo,
    mxc: &str,
) -> Result<bool, ErroMidia> {
    cliente.apagar_midia(token_admin, mxc).await
}

/// Código curto do erro, para a coluna `last_error` e para o log: o `errcode` do Synapse quando ele
/// vem e é seguro, senão uma etiqueta fixa da classe da falha.
///
/// Nunca devolve a mensagem do Synapse nem o endereço da mídia.
pub fn codigo_do_erro(erro: &ErroMidia) -> String {
    match erro {
        ErroMidia::Chamada(ErroChamada::Resposta {
            errcode: Some(codigo),
            ..
        }) if codigo_seguro(codigo) => codigo.clone(),
        ErroMidia::Chamada(ErroChamada::Rede { .. }) => "rede".to_owned(),
        ErroMidia::Chamada(ErroChamada::Prazo { .. }) => "prazo".to_owned(),
        ErroMidia::Chamada(ErroChamada::Configuracao(_)) => "configuracao".to_owned(),
        ErroMidia::Chamada(ErroChamada::RespostaInvalida { .. }) => "resposta_invalida".to_owned(),
        ErroMidia::Chamada(ErroChamada::Resposta { status, .. }) => {
            format!("http_{status}")
        }
        ErroMidia::Endereco(_) => "endereco_da_midia".to_owned(),
        ErroMidia::Grande { .. } => "midia_grande".to_owned(),
        ErroMidia::Vazia => "midia_vazia".to_owned(),
    }
}

/// O que o Synapse manda como `errcode` cabe na coluna de texto (`matrix_falhas` usa a mesma regra).
fn codigo_seguro(codigo: &str) -> bool {
    !codigo.is_empty()
        && codigo.len() <= 64
        && codigo
            .chars()
            .all(|caractere| caractere.is_ascii_alphanumeric() || caractere == '_')
}

#[cfg(test)]
mod testes {
    use super::*;

    fn midia() -> MidiaParaExpirar<'static> {
        MidiaParaExpirar {
            id: "anx-9",
            nome: "laudo.pdf",
            mime: "application/pdf",
            categoria: "document",
            tamanho_bytes: 2_048,
            enviada_em: 1_797_900_000_000,
        }
    }

    #[test]
    fn a_redacao_leva_o_bloco_do_frontend_com_os_cinco_campos() {
        let conteudo = conteudo_da_redacao(&midia());
        let bloco = &conteudo[BLOCO_EXPIRADO];
        assert_eq!(conteudo["reason"], json!(MOTIVO_DA_REDACAO));
        assert_eq!(bloco[CAMPO_NOME], json!("laudo.pdf"));
        assert_eq!(bloco[CAMPO_TIPO], json!("application/pdf"));
        assert_eq!(bloco[CAMPO_TAMANHO], json!(2_048));
        assert_eq!(bloco[CAMPO_CATEGORIA], json!("document"));
        assert_eq!(bloco[CAMPO_ENVIADA_EM], json!("2026-12-22T00:40:00Z"));
        // Nada do conteúdo do arquivo: só os metadados que o aviso mostra.
        assert_eq!(
            bloco.as_object().map(|objeto| objeto.len()),
            Some(5),
            "o bloco não leva campo a mais: {bloco}"
        );
    }

    #[test]
    fn instante_em_iso_utc_nao_depende_do_idioma() {
        for (ms, esperado) in [
            (0, "1970-01-01T00:00:00Z"),
            (1_000, "1970-01-01T00:00:01Z"),
            (1_797_900_000_000, "2026-12-22T00:40:00Z"),
            // Virada de ano e ano bissexto.
            (1_767_225_599_000, "2025-12-31T23:59:59Z"),
            (951_782_400_000, "2000-02-29T00:00:00Z"),
        ] {
            assert_eq!(instante_iso(ms), esperado, "ms {ms}");
        }
        // Antes da época (relógio errado do passado) não estoura: devolve a data certa.
        assert_eq!(instante_iso(-86_400_000), "1969-12-31T00:00:00Z");
    }

    #[test]
    fn txn_da_redacao_e_estavel_e_seguro_em_url() {
        assert_eq!(txn_da_redacao("anx-1"), txn_da_redacao("anx-1"));
        assert_ne!(txn_da_redacao("anx-1"), txn_da_redacao("anx-2"));
        let txn = txn_da_redacao("018f-0000-7000-8000-000000000001");
        assert!(txn.starts_with(PREFIXO_TXN_REDACAO));
        assert!(
            txn.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')),
            "txnId precisa ser seguro em URL: {txn}"
        );
    }

    #[test]
    fn o_codigo_do_erro_e_curto_e_nao_carrega_mensagem_nem_endereco() {
        let casos = [
            (
                ErroMidia::Chamada(ErroChamada::Resposta {
                    operacao: "media (administração)",
                    status: 403,
                    errcode: Some("M_FORBIDDEN".to_owned()),
                    mensagem: "nao posso apagar mxc://matrix-dev.local/abc".to_owned(),
                    retry_after_ms: None,
                }),
                "M_FORBIDDEN",
            ),
            (
                ErroMidia::Chamada(ErroChamada::Resposta {
                    operacao: "media (administração)",
                    status: 500,
                    errcode: None,
                    mensagem: "erro interno".to_owned(),
                    retry_after_ms: None,
                }),
                "http_500",
            ),
            (
                ErroMidia::Chamada(ErroChamada::Rede {
                    operacao: "media (administração)",
                    motivo: "não consegui conectar".to_owned(),
                }),
                "rede",
            ),
            (
                ErroMidia::Endereco("sem servidor".to_owned()),
                "endereco_da_midia",
            ),
        ];
        for (erro, esperado) in casos {
            let codigo = codigo_do_erro(&erro);
            assert_eq!(codigo, esperado);
            assert!(
                !codigo.contains("mxc://"),
                "código não leva endereço: {codigo}"
            );
            assert!(codigo.len() <= 64);
        }

        // `errcode` com caractere estranho não entra como está: vira o código do status.
        let estranho = ErroMidia::Chamada(ErroChamada::Resposta {
            operacao: "media (administração)",
            status: 400,
            errcode: Some("M_ERRADO; DROP TABLE".to_owned()),
            mensagem: String::new(),
            retry_after_ms: None,
        });
        assert_eq!(codigo_do_erro(&estranho), "http_400");
    }
}
