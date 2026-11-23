//! Mídia do atendente → cliente (MX-44, fase 2).
//!
//! O caminho inverso da [`crate::midia`]: lá o anexo do **cliente** sobe para a sala; aqui o arquivo
//! que o **atendente** manda pelo Element desce do Synapse e vira mensagem para o WhatsApp. O evento
//! chega ao Application Service (`m.image`, `m.file`, `m.audio`, `m.video`), o Harness baixa o
//! arquivo da sala, confere o tipo pelo conteúdo e deixa na fila de saída o item `file` que a
//! Evolution entrega. Áudio e vídeo entraram na MX-48: o vetor é o mesmo, e o tipo real vem de
//! `poph_files::envio` (lista fechada de saída, separada da lista de entrada).
//!
//! ## O que fica aqui
//!
//! Só o que dá para decidir sem rede, sem banco e sem disco: o texto do aviso quando o arquivo não
//! vai ao cliente, o `txnId` do aviso e o nome que o arquivo leva. Quem baixa, classifica e grava é
//! o `poph-server` (que conhece a pasta de blobs, a fila de saída e o banco), no mesmo desenho da
//! [`crate::midia`].
//!
//! ## Por que o aviso existe
//!
//! O atendente precisa saber que o arquivo **não** chegou ao cliente: sem o aviso, ele vê o arquivo
//! na propria sala (o próprio envio dele, no Element), supõe que o cliente recebeu e o atendimento
//! segue errado. O aviso é um `m.notice` na sala, com motivo curto e sem o nome do arquivo — nome de
//! arquivo é dado do cliente, e log/aviso automático não carrega dado de cliente.
//!
//! ## Duplicação na reentrega
//!
//! O `txnId` do aviso sai do id do evento, então o mesmo aviso não aparece duas vezes quando o
//! Synapse repete a transação. O envio do arquivo em si é **idempotente no banco** (a mensagem tem o
//! id do evento como chave) e **não é idempotente no canal**: a Evolution não tem chave de envio, e
//! a decisão de 17/11/2026 (a mesma do POP Atende) é reenviar aceitando o risco de o cliente receber
//! o arquivo duas vezes quando o resultado do envio não volta — o contrário (arquivo que nunca
//! chega, sem ninguém saber) é pior para o atendimento. Quem decide repetir é a fila de saída
//! (`saida_whatsapp`), não este módulo.

use serde_json::{Value, json};

use crate::eventos::MSGTYPE_AVISO;

/// Maior arquivo que sai para o cliente: o mesmo teto do recebimento e o do envio pela Evolution.
pub use crate::midia::LIMITE_MIDIA_BYTES;

/// Prefixo do aviso na sala quando o arquivo do atendente não vai ao cliente.
pub const PREFIXO_AVISO: &str = "Não consegui enviar o arquivo ao cliente: ";

/// Prefixo do `txnId` do aviso. Fixo: trocar o prefixo da instância não muda o `txnId`, senão um
/// reenvio depois de uma mudança de configuração duplicaria o aviso.
pub const PREFIXO_TXN_AVISO: &str = "poph-midia-atendente-";

/// Por que o arquivo do atendente não foi enviado ao cliente.
///
/// Cada motivo tem um código curto (log e métrica, sem dado de cliente) e um texto em português
/// (vai para a sala, depois de [`PREFIXO_AVISO`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotivoNaoEnviar {
    /// O arquivo passa do limite ([`LIMITE_MIDIA_BYTES`]).
    Grande,
    /// O arquivo tem zero byte.
    Vazio,
    /// O tipo real do conteúdo não está na lista fechada de saída (imagem, PDF, Office, áudio,
    /// vídeo).
    TipoRecusado,
    /// O evento de mídia não trouxe o `content_uri` (`mxc://…`), ou ele não serve.
    SemArquivo,
    /// O arquivo não pôde ser baixado da sala (apagado pelo servidor, sem permissão, fora do ar de
    /// forma definitiva).
    Indisponivel,
    /// O arquivo não pôde ser gravado como blob da org (pasta de blobs não configurada nesta
    /// montagem, disco cheio, permissão).
    NaoGravado,
}

impl MotivoNaoEnviar {
    /// Código curto e estável, para log e métrica.
    pub fn codigo(self) -> &'static str {
        match self {
            Self::Grande => "arquivo_grande",
            Self::Vazio => "arquivo_vazio",
            Self::TipoRecusado => "tipo_recusado",
            Self::SemArquivo => "sem_arquivo_no_evento",
            Self::Indisponivel => "arquivo_indisponivel",
            Self::NaoGravado => "arquivo_nao_gravado",
        }
    }

    /// Texto que vai à sala, depois de [`PREFIXO_AVISO`].
    pub fn texto(self) -> &'static str {
        match self {
            Self::Grande => "o arquivo passa de 16 MB",
            Self::Vazio => "o arquivo está vazio",
            Self::TipoRecusado => {
                "o tipo deste arquivo não é aceito (o cliente recebe imagem, PDF, Office, áudio e vídeo)"
            }
            Self::SemArquivo => "o evento não trouxe o arquivo",
            Self::Indisponivel => "não consegui baixar o arquivo da sala",
            Self::NaoGravado => "não consegui guardar o arquivo para envio",
        }
    }
}

/// Nome que o arquivo leva ao cliente: o do evento, ou o padrão de quem chama quando ele não veio.
///
/// O padrão depende do tipo **real** ([`poph_files::TipoEnvio::nome_padrao`]), não do `msgtype` do
/// evento: um PDF mandado como `m.image` continua sendo documento, e um áudio mandado como
/// `m.file` continua sendo áudio. Quem chama passa o padrão porque este crate não conhece o
/// `poph-files` — e não precisa conhecer: o nome padrão é dado de apresentação, não regra.
pub fn nome_do_arquivo(nome: &str, padrao: &str) -> String {
    let limpo = nome.trim();
    if limpo.is_empty() {
        padrao.to_owned()
    } else {
        limpo.to_owned()
    }
}

/// Corpo do aviso na sala para um motivo.
pub fn corpo_do_aviso(motivo: MotivoNaoEnviar) -> String {
    format!("{PREFIXO_AVISO}{}", motivo.texto())
}

/// Conteúdo do aviso (`m.notice`) de um arquivo que não vai ao cliente.
pub fn conteudo_do_aviso(motivo: MotivoNaoEnviar) -> Value {
    json!({ "msgtype": MSGTYPE_AVISO, "body": corpo_do_aviso(motivo) })
}

/// `txnId` do aviso de um arquivo que não saiu. Só o que cabe num segmento de URL sem escape.
pub fn txn_do_aviso(evento_id: &str, motivo: MotivoNaoEnviar) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in evento_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{PREFIXO_TXN_AVISO}{hash:016x}-{}", motivo.codigo())
}

#[cfg(test)]
mod testes {
    use super::*;

    const TODOS: [MotivoNaoEnviar; 6] = [
        MotivoNaoEnviar::Grande,
        MotivoNaoEnviar::Vazio,
        MotivoNaoEnviar::TipoRecusado,
        MotivoNaoEnviar::SemArquivo,
        MotivoNaoEnviar::Indisponivel,
        MotivoNaoEnviar::NaoGravado,
    ];

    #[test]
    fn cada_motivo_tem_codigo_unico_e_texto_proprio() {
        let mut codigos = Vec::new();
        let mut textos = Vec::new();
        for motivo in TODOS {
            assert!(!codigos.contains(&motivo.codigo()), "{}", motivo.codigo());
            assert!(!textos.contains(&motivo.texto()), "{}", motivo.texto());
            codigos.push(motivo.codigo());
            textos.push(motivo.texto());
        }
        assert_eq!(codigos.len(), TODOS.len());
    }

    #[test]
    fn aviso_leva_o_motivo_e_nunca_o_nome_do_arquivo() {
        for motivo in TODOS {
            let aviso = conteudo_do_aviso(motivo);
            assert_eq!(aviso["msgtype"], json!(MSGTYPE_AVISO));
            let corpo = aviso["body"].as_str().expect("corpo é texto");
            assert!(corpo.starts_with(PREFIXO_AVISO), "corpo: {corpo}");
            assert!(corpo.ends_with(motivo.texto()), "corpo: {corpo}");
            assert!(!corpo.contains("laudo"), "o aviso não leva nome de arquivo");
        }
    }

    #[test]
    fn txn_do_aviso_e_estavel_por_evento_e_diferente_por_motivo() {
        let evento = "$evento-mx44:matrix-dev.local";
        assert_eq!(
            txn_do_aviso(evento, MotivoNaoEnviar::Grande),
            txn_do_aviso(evento, MotivoNaoEnviar::Grande)
        );
        assert_ne!(
            txn_do_aviso(evento, MotivoNaoEnviar::Grande),
            txn_do_aviso("$outro:matrix-dev.local", MotivoNaoEnviar::Grande)
        );
        assert_ne!(
            txn_do_aviso(evento, MotivoNaoEnviar::Grande),
            txn_do_aviso(evento, MotivoNaoEnviar::TipoRecusado)
        );
        for motivo in TODOS {
            let txn = txn_do_aviso(evento, motivo);
            assert!(
                txn.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')),
                "txnId precisa ser seguro em URL: {txn}"
            );
        }
    }

    #[test]
    fn nome_vazio_ganha_o_padrao_de_quem_chama() {
        assert_eq!(nome_do_arquivo("  laudo.pdf ", "arquivo"), "laudo.pdf");
        assert_eq!(nome_do_arquivo("", "imagem"), "imagem");
        assert_eq!(nome_do_arquivo("   ", "arquivo"), "arquivo");
        assert_eq!(nome_do_arquivo("", "audio"), "audio");
        assert_eq!(nome_do_arquivo("", "video"), "video");
    }
}
