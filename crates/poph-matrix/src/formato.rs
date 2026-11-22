//! Formato das mensagens espelhadas (MX-13): decide, sem rede, sem banco e sem relógio, o que cada
//! mensagem do Harness vira no Matrix — qual usuário virtual fala, com que tipo de evento, com qual
//! corpo e com qual `txnId`.
//!
//! Regras do PRD (§5.1 passo 3 e §6.1):
//!
//! - cliente → fantasma do cliente; agente → fantasma do agente;
//! - origem humana **não espelha**: a resposta do atendente já veio do Matrix, espelhar de volta
//!   duplicaria a mensagem na sala;
//! - origem de dispositivo (digitada no celular, fase 2) também **não espelha** nesta fase;
//! - papel de ferramenta ou de sistema não vai para a sala;
//! - texto vazio ou só com espaço não espelha;
//! - o corpo sai como está (sem reformatar o texto do cliente); só é cortado quando passa do limite
//!   de evento do Matrix, e o corte é sinalizado no retorno para o laço registrar;
//! - o `txnId` vem do id da mensagem: a mesma mensagem gera sempre o mesmo valor, inclusive depois
//!   de uma queda, e o Synapse ignora a repetição (P7 do PRD).
//!
//! Esta função não loga nada. Quem chama decide o que registrar, e o que for registrado são só ids
//! e tamanhos, nunca o texto. Ela também não depende de `poph-store`: o chamador traduz as origens,
//! os papéis e os sentidos da sua tabela para os enums locais deste módulo.

use poph_types::{ConversationId, MessageId};

use crate::usuarios::{ErroFantasmas, Fantasma, IdsFantasmas};

/// Tipo do evento de mensagem do Matrix.
pub const TIPO_EVENTO: &str = "m.room.message";

/// `msgtype` de texto simples. Mensagem espelhada é sempre texto nesta fase.
pub const MSGTYPE_TEXTO: &str = "m.text";

/// Prefixo do `txnId` das mensagens espelhadas. Fixo: trocar o prefixo da instância não muda o
/// `txnId`, senão um reenvio depois de uma mudança de configuração duplicaria a mensagem.
pub const PREFIXO_TXN: &str = "poph-msg-";

/// Limite do corpo, medido como o Synapse vai ver: já com o escape do JSON.
///
/// O Matrix recusa evento acima de 65 536 bytes (envelope e corpo juntos). Deixo 48 KiB para o
/// corpo, para sobrar espaço para os campos do envelope (ids, assinaturas, a sala e o remetente).
pub const LIMITE_CORPO_JSON: usize = 48 * 1024;

/// Marca colocada no fim de um texto cortado.
pub const RETICENCIAS: &str = "…";

/// Origem da mensagem, como o Harness a grava (espelho do enum da `poph-store`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigemMensagem {
    /// Produzida pelo agente (turno).
    Agente,
    /// Respondida pelo atendente humano, pelo Element.
    Humana,
    /// Digitada no celular do cliente (fase 2).
    Dispositivo,
}

/// Quem fala no prompt (espelho de `role`, reduzido ao que importa aqui).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PapelMensagem {
    /// Cliente (`user`).
    Cliente,
    /// Agente (`assistant`).
    Agente,
    /// Ferramenta ou instrução de sistema: nunca vai para a sala.
    Outro,
}

/// Sentido da mensagem em relação ao agente (espelho de `direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sentido {
    /// Do cliente para o agente.
    Entrada,
    /// Do agente para o cliente.
    Saida,
}

/// Uma mensagem do Harness, já com os campos que a decisão precisa.
#[derive(Debug, Clone, Copy)]
pub struct MensagemParaEspelho<'a> {
    /// Id da mensagem; é dele que sai o `txnId`.
    pub id: MessageId,
    /// Conversa da mensagem (vira a sala).
    pub conversa: ConversationId,
    /// Origem como gravada.
    pub origem: OrigemMensagem,
    /// Sentido como gravado.
    pub sentido: Sentido,
    /// Papel no prompt.
    pub papel: PapelMensagem,
    /// Texto como gravado.
    pub texto: &'a str,
    /// Identificador do cliente da conversa (o `peer`); monta o fantasma do cliente.
    pub identificador_cliente: &'a str,
    /// Id do agente da conversa; monta o fantasma do agente.
    pub id_agente: &'a str,
}

/// Motivo para não espelhar. Não é erro: a mensagem simplesmente não vai para a sala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotivoNaoEspelhar {
    /// Respondida pelo atendente no Matrix: espelhar duplicaria.
    OrigemHumana,
    /// Digitada no celular do cliente; fica para a fase 2.
    OrigemDispositivo,
    /// Ferramenta ou instrução de sistema.
    PapelNaoEspelhado,
    /// Texto vazio ou só com espaço.
    TextoVazio,
}

/// Erro de formato: a mensagem deveria espelhar, mas não dá para montar o envio. É dado ruim ou
/// id que não serve para montar usuário; o laço deve registrar em `matrix_falhas`, sem pular a
/// mensagem em silêncio.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErroFormato {
    /// O identificador do cliente ou do agente não serve para montar o usuário virtual.
    #[error("não foi possível montar o usuário virtual da mensagem: {0}")]
    Fantasma(#[from] ErroFantasmas),
    /// Papel e sentido não combinam (cliente que saiu do agente, agente que entrou do cliente).
    #[error("papel e sentido da mensagem não combinam")]
    Inconsistente,
}

/// Envio pronto para o laço: quem fala, o evento e o identificador da entrega.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envio {
    /// Conversa de origem (a sala é dela).
    pub conversa: ConversationId,
    /// Usuário virtual que envia (cliente ou agente), com `?user_id=`.
    pub remetente: Fantasma,
    /// Tipo do evento (`m.room.message`).
    pub tipo_evento: &'static str,
    /// `msgtype` do conteúdo (`m.text`).
    pub msgtype: &'static str,
    /// Corpo final, já cortado se foi preciso.
    pub corpo: String,
    /// `txnId` da entrega: estável para a mesma mensagem.
    pub txn_id: String,
    /// `true` quando o corpo foi cortado com `…` para caber no limite.
    pub cortado: bool,
    /// Tamanho do texto original em bytes. Só metadado: o texto em si não sai daqui.
    pub bytes_originais: usize,
}

/// Decisão para uma mensagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decisao {
    /// Espelhar com este envio.
    Espelhar(Envio),
    /// Não espelhar, com o motivo.
    NaoEspelhar(MotivoNaoEspelhar),
    /// Não dá para decidir com dado bom: o laço registra a falha.
    Erro(ErroFormato),
}

/// Decide o formato de uma mensagem. Função pura: mesma entrada, mesma saída.
pub fn decidir(ids: &IdsFantasmas, mensagem: &MensagemParaEspelho<'_>) -> Decisao {
    match mensagem.origem {
        OrigemMensagem::Humana => return Decisao::NaoEspelhar(MotivoNaoEspelhar::OrigemHumana),
        OrigemMensagem::Dispositivo => {
            return Decisao::NaoEspelhar(MotivoNaoEspelhar::OrigemDispositivo);
        }
        OrigemMensagem::Agente => {}
    }

    if mensagem.papel == PapelMensagem::Outro {
        return Decisao::NaoEspelhar(MotivoNaoEspelhar::PapelNaoEspelhado);
    }

    // Papel e sentido precisam dizer a mesma coisa; se não, a mensagem seria atribuída ao
    // fantasma errado. Isso é dado ruim: erro, não silêncio.
    let combina = match mensagem.papel {
        PapelMensagem::Cliente => mensagem.sentido == Sentido::Entrada,
        PapelMensagem::Agente => mensagem.sentido == Sentido::Saida,
        PapelMensagem::Outro => true,
    };
    if !combina {
        return Decisao::Erro(ErroFormato::Inconsistente);
    }

    if mensagem.texto.trim().is_empty() {
        return Decisao::NaoEspelhar(MotivoNaoEspelhar::TextoVazio);
    }

    let remetente = match mensagem.papel {
        PapelMensagem::Cliente => ids.cliente(mensagem.identificador_cliente),
        PapelMensagem::Agente => ids.agente(mensagem.id_agente),
        PapelMensagem::Outro => return Decisao::NaoEspelhar(MotivoNaoEspelhar::PapelNaoEspelhado),
    };
    let remetente = match remetente {
        Ok(fantasma) => fantasma,
        Err(erro) => return Decisao::Erro(ErroFormato::Fantasma(erro)),
    };

    let (corpo, cortado) = cortar_no_limite(mensagem.texto);

    Decisao::Espelhar(Envio {
        conversa: mensagem.conversa,
        remetente,
        tipo_evento: TIPO_EVENTO,
        msgtype: MSGTYPE_TEXTO,
        corpo,
        txn_id: txn_id_da_mensagem(mensagem.id),
        cortado,
        bytes_originais: mensagem.texto.len(),
    })
}

/// `txnId` de uma mensagem: `poph-msg-` mais o id dela. Só letras, números e hífen, então sai
/// direto no caminho da URL sem escape.
pub fn txn_id_da_mensagem(id: MessageId) -> String {
    format!("{PREFIXO_TXN}{id}")
}

/// Corta o texto se o JSON dele passar de [`LIMITE_CORPO_JSON`], com `…` no fim. Devolve o corpo e
/// se houve corte. Texto dentro do limite sai igual, byte a byte.
pub fn cortar_no_limite(texto: &str) -> (String, bool) {
    if tamanho_json(texto) <= LIMITE_CORPO_JSON {
        return (texto.to_owned(), false);
    }
    let orcamento = LIMITE_CORPO_JSON.saturating_sub(tamanho_json(RETICENCIAS));
    let mut mantido = String::new();
    let mut usado = 0usize;
    for caractere in texto.chars() {
        let pedaco = caractere.to_string();
        let custo = tamanho_json(&pedaco);
        if usado + custo > orcamento {
            break;
        }
        usado += custo;
        mantido.push(caractere);
    }
    mantido.push_str(RETICENCIAS);
    (mantido, true)
}

/// Tamanho do texto dentro de uma string JSON, com o escape que o Synapse vai ver (sem as aspas).
fn tamanho_json(texto: &str) -> usize {
    serde_json::to_string(texto).map_or(texto.len().saturating_mul(6), |json| {
        json.len().saturating_sub(2)
    })
}

#[cfg(test)]
mod testes {
    use poph_types::{ConversationId, MessageId};

    use super::*;

    const SERVIDOR: &str = "matrix-dev.local";

    fn ids() -> IdsFantasmas {
        IdsFantasmas::novo("ph", SERVIDOR).expect("prefixo e servidor válidos")
    }

    fn mensagem_id(n: u64) -> MessageId {
        // UUID v7 fixo: o teste precisa do mesmo id em todas as execuções.
        format!("0190a000-0000-7000-8000-{n:012x}")
            .parse::<MessageId>()
            .expect("UUID v7 válido")
    }

    fn conversa() -> ConversationId {
        ConversationId::generate()
    }

    fn base<'a>(
        texto: &'a str,
        origem: OrigemMensagem,
        papel: PapelMensagem,
        sentido: Sentido,
    ) -> MensagemParaEspelho<'a> {
        MensagemParaEspelho {
            id: mensagem_id(1),
            conversa: conversa(),
            origem,
            sentido,
            papel,
            texto,
            identificador_cliente: "+55 21 99999-0001",
            id_agente: "agente-1",
        }
    }

    fn espelhada(decisao: Decisao) -> Envio {
        match decisao {
            Decisao::Espelhar(envio) => envio,
            outra => panic!("esperava espelhar, veio {outra:?}"),
        }
    }

    #[test]
    fn mensagem_do_agente_sai_pelo_fantasma_do_agente() {
        let m = base(
            "Olá, em que posso ajudar?",
            OrigemMensagem::Agente,
            PapelMensagem::Agente,
            Sentido::Saida,
        );
        let envio = espelhada(decidir(&ids(), &m));
        assert_eq!(
            envio.remetente.user_id(),
            "@ph_ag_agente-1:matrix-dev.local"
        );
        assert_eq!(envio.tipo_evento, "m.room.message");
        assert_eq!(envio.msgtype, "m.text");
        assert!(!envio.cortado);
    }

    #[test]
    fn mensagem_do_cliente_sai_pelo_fantasma_do_cliente() {
        let m = base(
            "Quero saber o preço",
            OrigemMensagem::Agente,
            PapelMensagem::Cliente,
            Sentido::Entrada,
        );
        let envio = espelhada(decidir(&ids(), &m));
        // O telefone é normalizado pelo `IdsFantasmas`; o que importa aqui é o prefixo de cliente.
        assert!(
            envio.remetente.user_id().starts_with("@ph_wa_"),
            "fantasma do cliente esperado, veio {}",
            envio.remetente
        );
        assert_eq!(envio.corpo, "Quero saber o preço");
    }

    #[test]
    fn origem_humana_nao_espelha_em_nenhum_papel() {
        for (papel, sentido) in [
            (PapelMensagem::Agente, Sentido::Saida),
            (PapelMensagem::Cliente, Sentido::Entrada),
        ] {
            let m = base(
                "Resposta do atendente",
                OrigemMensagem::Humana,
                papel,
                sentido,
            );
            assert_eq!(
                decidir(&ids(), &m),
                Decisao::NaoEspelhar(MotivoNaoEspelhar::OrigemHumana)
            );
        }
    }

    #[test]
    fn origem_dispositivo_nao_espelha_nesta_fase() {
        let m = base(
            "digitada no celular",
            OrigemMensagem::Dispositivo,
            PapelMensagem::Agente,
            Sentido::Saida,
        );
        assert_eq!(
            decidir(&ids(), &m),
            Decisao::NaoEspelhar(MotivoNaoEspelhar::OrigemDispositivo)
        );
    }

    #[test]
    fn ferramenta_e_sistema_nao_vao_para_a_sala() {
        let m = base(
            "resultado de ferramenta",
            OrigemMensagem::Agente,
            PapelMensagem::Outro,
            Sentido::Saida,
        );
        assert_eq!(
            decidir(&ids(), &m),
            Decisao::NaoEspelhar(MotivoNaoEspelhar::PapelNaoEspelhado)
        );
    }

    #[test]
    fn texto_vazio_ou_so_espaco_nao_espelha() {
        for texto in ["", " ", "   ", "\n", "\t \r\n", "\u{00a0}"] {
            let m = base(
                texto,
                OrigemMensagem::Agente,
                PapelMensagem::Agente,
                Sentido::Saida,
            );
            assert_eq!(
                decidir(&ids(), &m),
                Decisao::NaoEspelhar(MotivoNaoEspelhar::TextoVazio),
                "texto {texto:?} deveria ser tratado como vazio"
            );
        }
    }

    #[test]
    fn papel_e_sentido_que_nao_combinam_viram_erro_nao_silencio() {
        let trocado_cliente = base(
            "oi",
            OrigemMensagem::Agente,
            PapelMensagem::Cliente,
            Sentido::Saida,
        );
        assert_eq!(
            decidir(&ids(), &trocado_cliente),
            Decisao::Erro(ErroFormato::Inconsistente)
        );
        let trocado_agente = base(
            "oi",
            OrigemMensagem::Agente,
            PapelMensagem::Agente,
            Sentido::Entrada,
        );
        assert_eq!(
            decidir(&ids(), &trocado_agente),
            Decisao::Erro(ErroFormato::Inconsistente)
        );
    }

    #[test]
    fn identificador_que_nao_monta_usuario_vira_erro_de_formato() {
        let mut m = base(
            "oi",
            OrigemMensagem::Agente,
            PapelMensagem::Cliente,
            Sentido::Entrada,
        );
        m.identificador_cliente = "   ";
        assert!(matches!(
            decidir(&ids(), &m),
            Decisao::Erro(ErroFormato::Fantasma(_))
        ));
    }

    #[test]
    fn corpo_sai_como_foi_escrito() {
        let texto = "  *Oi*, tudo bem?\n\n- item 1\n- item 2  \n  ";
        let m = base(
            texto,
            OrigemMensagem::Agente,
            PapelMensagem::Cliente,
            Sentido::Entrada,
        );
        let envio = espelhada(decidir(&ids(), &m));
        assert_eq!(envio.corpo, texto, "o corpo não pode ser reformatado");
    }

    #[test]
    fn texto_exatamente_no_limite_nao_e_cortado() {
        let texto = "a".repeat(LIMITE_CORPO_JSON);
        let (corpo, cortado) = cortar_no_limite(&texto);
        assert!(!cortado);
        assert_eq!(corpo, texto);
    }

    #[test]
    fn texto_acima_do_limite_e_cortado_com_reticencias_e_sinalizado() {
        let texto = "a".repeat(LIMITE_CORPO_JSON + 1);
        let (corpo, cortado) = cortar_no_limite(&texto);
        assert!(cortado);
        assert!(corpo.ends_with(RETICENCIAS));
        assert_eq!(
            tamanho_json(&corpo),
            LIMITE_CORPO_JSON,
            "o corte fecha exatamente no limite"
        );
    }

    #[test]
    fn corte_conta_o_escape_do_json_e_nao_quebra_caractere() {
        // Controle vira `\u0001` (6 bytes no JSON); e `ç` tem 2 bytes em UTF-8.
        let controle = "\u{1}".repeat(LIMITE_CORPO_JSON);
        let (corpo, cortado) = cortar_no_limite(&controle);
        assert!(cortado);
        assert!(tamanho_json(&corpo) <= LIMITE_CORPO_JSON);
        assert!(corpo.ends_with(RETICENCIAS));

        let acentuado = "ç".repeat(LIMITE_CORPO_JSON);
        let (corpo, cortado) = cortar_no_limite(&acentuado);
        assert!(cortado);
        assert!(tamanho_json(&corpo) <= LIMITE_CORPO_JSON);
        assert!(corpo.ends_with(RETICENCIAS));
    }

    #[test]
    fn decisao_com_texto_longo_marca_corte_e_mantem_tamanho_original() {
        let texto = "b".repeat(LIMITE_CORPO_JSON * 2);
        let m = base(
            &texto,
            OrigemMensagem::Agente,
            PapelMensagem::Agente,
            Sentido::Saida,
        );
        let envio = espelhada(decidir(&ids(), &m));
        assert!(envio.cortado);
        assert_eq!(envio.bytes_originais, texto.len());
        assert!(tamanho_json(&envio.corpo) <= LIMITE_CORPO_JSON);
    }

    #[test]
    fn txn_id_e_estavel_para_a_mesma_mensagem() {
        let id = mensagem_id(7);
        assert_eq!(txn_id_da_mensagem(id), txn_id_da_mensagem(id));
        // Nova chamada com a mesma mensagem (mesmo que outra conversa/texto nos outros campos).
        let a = base(
            "um",
            OrigemMensagem::Agente,
            PapelMensagem::Agente,
            Sentido::Saida,
        );
        let mut b = base(
            "outro texto",
            OrigemMensagem::Agente,
            PapelMensagem::Agente,
            Sentido::Saida,
        );
        b.id = a.id;
        b.conversa = a.conversa;
        let txn_a = espelhada(decidir(&ids(), &a)).txn_id;
        let txn_b = espelhada(decidir(&ids(), &b)).txn_id;
        assert_eq!(txn_a, txn_b, "o txnId depende só do id da mensagem");
    }

    #[test]
    fn txn_id_e_distinto_entre_mensagens_e_seguro_para_url() {
        let a = txn_id_da_mensagem(mensagem_id(1));
        let b = txn_id_da_mensagem(mensagem_id(2));
        assert_ne!(a, b);
        assert!(a.starts_with(PREFIXO_TXN));
        assert!(
            a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "txnId com caractere fora de letra, número e hífen: {a}"
        );
    }

    #[test]
    fn decisao_inteira_e_reprodutivel() {
        let m = base(
            "mesma entrada",
            OrigemMensagem::Agente,
            PapelMensagem::Agente,
            Sentido::Saida,
        );
        assert_eq!(decidir(&ids(), &m), decidir(&ids(), &m));
    }
}
