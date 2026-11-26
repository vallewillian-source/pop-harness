#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedContent {
    Audio,
    Video,
    Sticker,
    Contact,
    Location,
    Poll,
}

impl UnsupportedContent {
    pub const ALL_VALUES: [Self; 6] = [
        Self::Audio,
        Self::Video,
        Self::Sticker,
        Self::Contact,
        Self::Location,
        Self::Poll,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Sticker => "figurinha",
            Self::Contact => "contato",
            Self::Location => "localizacao",
            Self::Poll => "enquete",
        }
    }

    pub fn history_text(self) -> &'static str {
        match self {
            Self::Audio => "[O cliente enviou um áudio, que o sistema não consegue abrir]",
            Self::Video => "[O cliente enviou um vídeo, que o sistema não consegue abrir]",
            Self::Sticker => "[O cliente enviou uma figurinha, que o sistema não consegue abrir]",
            Self::Contact => "[O cliente enviou um contato, que o sistema não consegue abrir]",
            Self::Location => {
                "[O cliente enviou uma localização, que o sistema não consegue abrir]"
            }
            Self::Poll => "[O cliente enviou uma enquete, que o sistema não consegue abrir]",
        }
    }

    pub fn room_notice(self) -> &'static str {
        match self {
            Self::Audio => {
                "O cliente enviou um áudio. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Video => {
                "O cliente enviou um vídeo. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Sticker => {
                "O cliente enviou uma figurinha. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Contact => {
                "O cliente enviou um contato. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Location => {
                "O cliente enviou uma localização. Esse tipo de mensagem ainda não é suportado."
            }
            Self::Poll => {
                "O cliente enviou uma enquete. Esse tipo de mensagem ainda não é suportado."
            }
        }
    }

    pub fn from_history_text(text: &str) -> Option<Self> {
        Self::ALL_VALUES
            .into_iter()
            .find(|tipo| tipo.history_text() == text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_history_text_recognises_itself() {
        for tipo in UnsupportedContent::ALL_VALUES {
            assert_eq!(
                UnsupportedContent::from_history_text(tipo.history_text()),
                Some(tipo)
            );
        }
    }

    #[test]
    fn any_change_in_sentence_stops_being_history() {
        let sentence = UnsupportedContent::Audio.history_text();
        assert_eq!(
            UnsupportedContent::from_history_text(&sentence.to_uppercase()),
            None
        );
        assert_eq!(
            UnsupportedContent::from_history_text(&format!("{sentence} ")),
            None
        );
        assert_eq!(UnsupportedContent::from_history_text("oi, tudo bem?"), None);
    }

    #[test]
    fn notice_matches_python_text() {
        assert_eq!(
            UnsupportedContent::Audio.room_notice(),
            "O cliente enviou um áudio. Esse tipo de mensagem ainda não é suportado."
        );
        assert_eq!(
            UnsupportedContent::Poll.room_notice(),
            "O cliente enviou uma enquete. Esse tipo de mensagem ainda não é suportado."
        );
    }
}
