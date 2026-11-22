//! Trait do provedor de LLM e as regras de pedido que valem para qualquer provedor.

use std::collections::HashSet;

use futures::future::BoxFuture;
use futures::stream::BoxStream;

use super::erro::ErroLlm;
use super::tipos::{EventoLlm, Mensagem, Parte, Pedido};

/// Stream de eventos de uma resposta. Quem consome pode parar a qualquer momento: soltar o stream cancela a chamada.
pub type FluxoLlm = BoxStream<'static, Result<EventoLlm, ErroLlm>>;

/// Provedor de LLM. Hoje só existe o cliente do antigravity2api e o provedor falso para testes.
pub trait LlmProvider: Send + Sync {
    /// Abre a resposta. Quando devolve `Ok`, o stream já está no ar; os eventos chegam por ele.
    fn stream<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<FluxoLlm, ErroLlm>>;
}

/// Tipos de imagem que o gateway aceita.
pub const MIMES_IMAGEM: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];

/// Confere o pedido antes de qualquer envio.
///
/// Regra que evita o HTTP 400 "Requests ending with a model turn are not supported" do gateway:
/// a última mensagem tem de ser do usuário ou um resultado de ferramenta. Esta função não
/// corrige o pedido, só recusa, para o erro aparecer aqui com a causa.
pub fn validar_pedido(pedido: &Pedido) -> Result<(), ErroLlm> {
    let Some(ultima) = pedido.mensagens.last() else {
        return Err(ErroLlm::PedidoInvalido("nenhuma mensagem".to_owned()));
    };
    if let Mensagem::Assistente { .. } = ultima {
        return Err(ErroLlm::PedidoInvalido(
            "a última mensagem é do modelo; o provedor recusa esse formato. Termine com a instrução do usuário ou com o resultado de uma ferramenta".to_owned(),
        ));
    }

    let mut nomes = HashSet::new();
    for ferramenta in &pedido.ferramentas {
        if ferramenta.nome.trim().is_empty() {
            return Err(ErroLlm::PedidoInvalido("ferramenta sem nome".to_owned()));
        }
        if !nomes.insert(ferramenta.nome.as_str()) {
            return Err(ErroLlm::PedidoInvalido(format!(
                "ferramenta repetida: {}",
                ferramenta.nome
            )));
        }
    }

    for mensagem in &pedido.mensagens {
        if let Mensagem::Usuario(partes) = mensagem {
            for parte in partes {
                if let Parte::Imagem { mime, bytes } = parte {
                    if !MIMES_IMAGEM.contains(&mime.as_str()) {
                        return Err(ErroLlm::PedidoInvalido(format!(
                            "tipo de imagem não aceito: {mime}"
                        )));
                    }
                    if bytes.is_empty() {
                        return Err(ErroLlm::PedidoInvalido("imagem vazia".to_owned()));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod testes {
    use poph_types::OrgId;

    use super::*;
    use crate::llm::tipos::Ferramenta;

    fn pedido(mensagens: Vec<Mensagem>) -> Pedido {
        Pedido::new(OrgId::generate(), mensagens)
    }

    #[test]
    fn recusa_pedido_que_termina_em_turno_do_modelo() {
        let p = pedido(vec![
            Mensagem::Usuario(vec![Parte::Texto("oi".into())]),
            Mensagem::Assistente {
                texto: "olá".into(),
                chamadas: vec![],
            },
        ]);
        assert!(matches!(
            validar_pedido(&p),
            Err(ErroLlm::PedidoInvalido(_))
        ));
    }

    #[test]
    fn aceita_pedido_que_termina_em_resultado_de_ferramenta() {
        let p = pedido(vec![
            Mensagem::Usuario(vec![Parte::Texto("preço?".into())]),
            Mensagem::Assistente {
                texto: String::new(),
                chamadas: vec![crate::llm::tipos::ChamadaFerramenta {
                    id: "c1".into(),
                    nome: "buscar".into(),
                    argumentos: serde_json::json!({}),
                }],
            },
            Mensagem::Resultado {
                id: "c1".into(),
                conteudo: "R$ 10".into(),
            },
        ]);
        assert!(validar_pedido(&p).is_ok());
    }

    #[test]
    fn recusa_tipo_de_imagem_e_ferramenta_repetida() {
        let mut p = pedido(vec![Mensagem::Usuario(vec![Parte::Imagem {
            mime: "image/bmp".into(),
            bytes: vec![1],
        }])]);
        assert!(validar_pedido(&p).is_err());

        p.mensagens = vec![Mensagem::Usuario(vec![Parte::Texto("x".into())])];
        let f = Ferramenta {
            nome: "a".into(),
            descricao: String::new(),
            parametros: serde_json::json!({"type": "object"}),
        };
        p.ferramentas = vec![f.clone(), f];
        assert!(validar_pedido(&p).is_err());
    }
}
