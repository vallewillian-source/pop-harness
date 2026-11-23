//! Mensagens do WhatsApp que o Harness não abre (MX-45): áudio, vídeo, figurinha, contato,
//! localização e enquete.
//!
//! Decisão de produto (opção A, 22/11/2026): o Harness **não baixa** esses tipos. Ele grava a
//! mensagem como texto de histórico fixo, para o agente saber que o cliente mandou algo e poder
//! pedir que escreva. Na sala do Matrix, o espelho troca esse texto por um aviso `m.notice` quando
//! ele bate exatamente com o histórico. É paridade com o POP Atende
//! (`apps/backend/core/conversations/whatsapp_media.py`): mesmos textos, com uma exceção, a enquete,
//! que ganha o rótulo "uma enquete" em vez de "uma mensagem de tipo não suportado".
//!
//! Limite conhecido: um cliente que digite exatamente uma dessas frases entre colchetes verá o
//! aviso `m.notice` no lugar do texto. Não há como distinguir os dois sem uma coluna de tipo, que
//! esta fase não tem.
//!
//! As frases ficam aqui, num só lugar, porque dois crates dependem delas: o webhook (`poph-evolution`)
//! as grava e o espelho (`poph-matrix`) as reconhece.

/// Tipo de mensagem fora do escopo de download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiaForaDoEscopo {
    /// Áudio (e mensagem de voz).
    Audio,
    /// Vídeo, inclusive o "vídeo de bolha" (`ptvMessage`).
    Video,
    /// Figurinha.
    Figurinha,
    /// Contato (um ou vários).
    Contato,
    /// Localização, fixa ou ao vivo.
    Localizacao,
    /// Enquete.
    Enquete,
}

impl MidiaForaDoEscopo {
    /// Todos os tipos, na ordem da tabela.
    pub const TODAS: [Self; 6] = [
        Self::Audio,
        Self::Video,
        Self::Figurinha,
        Self::Contato,
        Self::Localizacao,
        Self::Enquete,
    ];

    /// Rótulo curto, para o motivo de descarte e para log. Sem dado de cliente.
    pub fn rotulo(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Figurinha => "figurinha",
            Self::Contato => "contato",
            Self::Localizacao => "localizacao",
            Self::Enquete => "enquete",
        }
    }

    /// Texto de histórico que o agente recebe no turno, no lugar do conteúdo que não dá para abrir.
    pub fn texto_de_historico(self) -> &'static str {
        match self {
            Self::Audio => "[O cliente enviou um áudio, que o sistema não consegue abrir]",
            Self::Video => "[O cliente enviou um vídeo, que o sistema não consegue abrir]",
            Self::Figurinha => "[O cliente enviou uma figurinha, que o sistema não consegue abrir]",
            Self::Contato => "[O cliente enviou um contato, que o sistema não consegue abrir]",
            Self::Localizacao => {
                "[O cliente enviou uma localização, que o sistema não consegue abrir]"
            }
            Self::Enquete => "[O cliente enviou uma enquete, que o sistema não consegue abrir]",
        }
    }

    /// Aviso que vai à sala do Matrix no lugar do texto de histórico.
    pub fn aviso_na_sala(self) -> &'static str {
        match self {
            Self::Audio => {
                "O cliente enviou um áudio. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Video => {
                "O cliente enviou um vídeo. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Figurinha => {
                "O cliente enviou uma figurinha. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Contato => {
                "O cliente enviou um contato. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Localizacao => {
                "O cliente enviou uma localização. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Enquete => {
                "O cliente enviou uma enquete. Esse tipo de mensagem ainda não é suportado."
            }
        }
    }

    /// Qual tipo é a frase de histórico, se o texto for exatamente uma delas.
    ///
    /// Comparação exata: qualquer diferença (espaço, pontuação, caixa) faz o texto seguir como
    /// texto comum.
    pub fn reconhecer_historico(texto: &str) -> Option<Self> {
        Self::TODAS
            .into_iter()
            .find(|tipo| tipo.texto_de_historico() == texto)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn cada_historico_se_reconhece_de_volta() {
        for tipo in MidiaForaDoEscopo::TODAS {
            assert_eq!(
                MidiaForaDoEscopo::reconhecer_historico(tipo.texto_de_historico()),
                Some(tipo)
            );
        }
    }

    #[test]
    fn qualquer_mudanca_na_frase_deixa_de_ser_historico() {
        let frase = MidiaForaDoEscopo::Audio.texto_de_historico();
        assert_eq!(
            MidiaForaDoEscopo::reconhecer_historico(&frase.to_uppercase()),
            None
        );
        assert_eq!(
            MidiaForaDoEscopo::reconhecer_historico(&format!("{frase} ")),
            None
        );
        assert_eq!(
            MidiaForaDoEscopo::reconhecer_historico("oi, tudo bem?"),
            None
        );
    }

    #[test]
    fn aviso_e_o_texto_do_python_com_o_tipo_certo() {
        assert_eq!(
            MidiaForaDoEscopo::Audio.aviso_na_sala(),
            "O cliente enviou um áudio. Esse tipo de mensagem ainda não é suportado."
        );
        assert_eq!(
            MidiaForaDoEscopo::Enquete.aviso_na_sala(),
            "O cliente enviou uma enquete. Esse tipo de mensagem ainda não é suportado."
        );
    }
}
