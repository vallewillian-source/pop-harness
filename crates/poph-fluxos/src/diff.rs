//! Diferença entre duas revisões de um fluxo, em linguagem comum (FX-17), para a tela de
//! publicação e para o histórico.
//!
//! [`diferenca`] compara a revisão antiga (`ir_a`) com a nova (`ir_b`) e devolve uma [`Mudanca`]
//! para cada coisa que mudou. Cada uma traz:
//!
//! - `tipo` ([`TipoMudanca`]): o que mudou (`acao_nova`, `campo_agora_obrigatorio`...);
//! - `impacto` ([`Impacto`]): o que muda para o atendimento;
//! - `alvo`: onde mudou, em código (`etapa:coleta/acao:confirmar`), para a tela e a API;
//! - `texto`: a frase em português que a tela mostra.
//!
//! Regras de leitura:
//!
//! - Tudo é comparado pelo **id** (etapa, campo, ação, decisão, fim, def). A ordem dos itens no
//!   script não gera mudança. A lista sai sempre na mesma ordem: o fluxo primeiro, depois as
//!   etapas (na ordem da revisão nova, com o que foi removido logo depois), as decisões, os fins e
//!   as defs. Nada aqui depende da ordem interna de um mapa JSON, então a ordem não muda entre o
//!   build do workspace e o build isolado do crate.
//! - Etapa, decisão, fim ou def **nova ou removida** sai como uma mudança só: o que ela contém não
//!   é listado item a item. Os efeitos de uma etapa ou ação nova (ou removida) saem à parte, porque
//!   disparam mensagem ou transferência, e só quando a etapa é alcançada por alguma ação ou
//!   decisão. Etapa sem chegada não dispara nada.
//! - Impacto:
//!   - `nenhum`: o texto muda, o atendimento não (nome, rótulo, dica, resumo de fim);
//!   - `atendimentos_novos`: só vale para os atendimentos que começarem depois da publicação;
//!   - `pede_dado_novo`: o agente passa a pedir um dado que antes não pedia, ou a pedi-lo de outro
//!     jeito;
//!   - `muda_rota`: o caminho do atendimento muda (destino, ator, condição, encerramento);
//!   - `dispara_efeito`: passa a sair, ou deixa de sair, mensagem ao cliente, aviso à equipe ou
//!     transferência.
//! - Para etapa, decisão, fim e def novos ou removidos, o impacto depende de haver chegada até
//!   eles (ação ou decisão que aponta para o nó). Para o que já existia, o impacto é o da própria
//!   mudança.
//!
//! Função pura: não acessa banco, relógio nem rede. A contagem de atendimentos em andamento por
//! revisão não é daqui; quem a acrescenta é a API (FX-27).

use serde::{Deserialize, Serialize};

use crate::ir::{Acao, Campo, Decisao, Destination, Effect, Etapa, Fim, Ir, Rule};
use poph_types::{ActionActor, FieldKind};

/// O que uma mudança faz com o atendimento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impacto {
    /// O texto muda, o atendimento não.
    Nenhum,
    /// Só os atendimentos que começarem depois da publicação seguem a mudança.
    AtendimentosNovos,
    /// O agente passa a pedir um dado que antes não pedia, ou pede de outro jeito.
    PedeDadoNovo,
    /// O caminho do atendimento muda.
    MudaRota,
    /// Passa a sair, ou deixa de sair, mensagem ao cliente, aviso à equipe ou transferência.
    DisparaEfeito,
}

impl Impacto {
    /// Código estável: é o mesmo texto que o JSON da API usa.
    pub fn codigo(self) -> &'static str {
        match self {
            Self::Nenhum => "nenhum",
            Self::AtendimentosNovos => "atendimentos_novos",
            Self::PedeDadoNovo => "pede_dado_novo",
            Self::MudaRota => "muda_rota",
            Self::DisparaEfeito => "dispara_efeito",
        }
    }
}

/// Tipo de uma mudança entre duas revisões.
///
/// A lista da FX-17 foi completada por mudanças que não cabiam em nenhum tipo sem perder
/// informação: `instrucao_mudou`, `atores_da_etapa_mudou`, `campo_deixou_de_ser_obrigatorio`,
/// `campo_mudou_tipo`, `campo_mudou_opcoes`, `texto_mudou` (nome, rótulo, dica, descrição),
/// `acao_passou_a_exigir`, `acao_deixou_de_exigir`, `ator_da_acao_mudou`, `decisao_nova`,
/// `decisao_removida`, `fim_novo`, `fim_removido`, `fim_mudou` e `inicio_mudou`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoMudanca {
    /// Etapa que não existia na revisão antiga.
    EtapaNova,
    /// Etapa que deixou de existir.
    EtapaRemovida,
    /// Instrução da etapa (o que o agente deve fazer nela) mudou.
    InstrucaoMudou,
    /// Quem pode executar as ações da etapa mudou.
    AtoresDaEtapaMudou,
    /// Campo que a etapa passou a coletar.
    CampoNovo,
    /// Campo que a etapa deixou de coletar.
    CampoRemovido,
    /// Campo que passou a ser obrigatório.
    CampoAgoraObrigatorio,
    /// Campo que deixou de ser obrigatório.
    CampoDeixouDeSerObrigatorio,
    /// Campo com outro tipo de valor.
    CampoMudouTipo,
    /// Campo de opção com outra lista de valores.
    CampoMudouOpcoes,
    /// Ação que passou a exigir um campo da própria etapa.
    AcaoPassouAExigir,
    /// Ação que deixou de exigir um campo.
    AcaoDeixouDeExigir,
    /// Ação nova.
    AcaoNova,
    /// Ação que deixou de existir.
    AcaoRemovida,
    /// Ação, ou ramo de decisão, que leva para outro destino.
    DestinoMudou,
    /// Quem pode executar a ação mudou (ou deixou de ser restrita).
    AtorDaAcaoMudou,
    /// Condição de decisão, ou texto de regra especial (`def`), mudou.
    RegraMudou,
    /// Decisão nova.
    DecisaoNova,
    /// Decisão que deixou de existir.
    DecisaoRemovida,
    /// Desfecho novo.
    FimNovo,
    /// Desfecho que deixou de existir.
    FimRemovido,
    /// Desfecho com outro encerramento do atendimento ou outro resumo.
    FimMudou,
    /// Efeito novo: passa a sair mensagem, aviso ou transferência.
    EfeitoNovo,
    /// Efeito que deixou de sair.
    EfeitoRemovido,
    /// Critério para iniciar o fluxo mudou.
    GatilhoMudou,
    /// Etapa em que o fluxo começa mudou.
    InicioMudou,
    /// Texto sem efeito no atendimento (nome, rótulo, dica, descrição).
    TextoMudou,
}

impl TipoMudanca {
    /// Código estável: é o mesmo texto que o JSON da API usa.
    pub fn codigo(self) -> &'static str {
        match self {
            Self::EtapaNova => "etapa_nova",
            Self::EtapaRemovida => "etapa_removida",
            Self::InstrucaoMudou => "instrucao_mudou",
            Self::AtoresDaEtapaMudou => "atores_da_etapa_mudou",
            Self::CampoNovo => "campo_novo",
            Self::CampoRemovido => "campo_removido",
            Self::CampoAgoraObrigatorio => "campo_agora_obrigatorio",
            Self::CampoDeixouDeSerObrigatorio => "campo_deixou_de_ser_obrigatorio",
            Self::CampoMudouTipo => "campo_mudou_tipo",
            Self::CampoMudouOpcoes => "campo_mudou_opcoes",
            Self::AcaoPassouAExigir => "acao_passou_a_exigir",
            Self::AcaoDeixouDeExigir => "acao_deixou_de_exigir",
            Self::AcaoNova => "acao_nova",
            Self::AcaoRemovida => "acao_removida",
            Self::DestinoMudou => "destino_mudou",
            Self::AtorDaAcaoMudou => "ator_da_acao_mudou",
            Self::RegraMudou => "regra_mudou",
            Self::DecisaoNova => "decisao_nova",
            Self::DecisaoRemovida => "decisao_removida",
            Self::FimNovo => "fim_novo",
            Self::FimRemovido => "fim_removido",
            Self::FimMudou => "fim_mudou",
            Self::EfeitoNovo => "efeito_novo",
            Self::EfeitoRemovido => "efeito_removido",
            Self::GatilhoMudou => "gatilho_mudou",
            Self::InicioMudou => "inicio_mudou",
            Self::TextoMudou => "texto_mudou",
        }
    }
}

/// Uma mudança entre duas revisões, pronta para a tela e para a API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mudanca {
    /// O que mudou.
    pub tipo: TipoMudanca,
    /// O que a mudança faz com o atendimento.
    pub impacto: Impacto,
    /// Onde mudou, em código: `fluxo`, `gatilho`, `etapa:<id>`, `etapa:<id>/campo:<id>`,
    /// `etapa:<id>/acao:<id>`, `decisao:<id>`, `decisao:<id>/ramo:entao|senao`, `fim:<id>`,
    /// `def:<nome>`.
    pub alvo: String,
    /// A frase em português que a tela mostra.
    pub texto: String,
}

impl Mudanca {
    fn nova(tipo: TipoMudanca, impacto: Impacto, alvo: &str, texto: String) -> Self {
        Self {
            tipo,
            impacto,
            alvo: alvo.to_owned(),
            texto,
        }
    }
}

/// Diferença entre a revisão antiga (`ir_a`) e a nova (`ir_b`), em mudanças com texto em
/// português. Fluxos idênticos devolvem lista vazia.
pub fn diferenca(ir_a: &Ir, ir_b: &Ir) -> Vec<Mudanca> {
    let mut saida = Vec::new();
    cabecalho(ir_a, ir_b, &mut saida);
    etapas(ir_a, ir_b, &mut saida);
    decisoes(ir_a, ir_b, &mut saida);
    fins(ir_a, ir_b, &mut saida);
    defs(ir_a, ir_b, &mut saida);
    saida
}

// ------------------------------------------------------------------ fluxo

fn cabecalho(a: &Ir, b: &Ir, out: &mut Vec<Mudanca>) {
    if a.fluxo.nome != b.fluxo.nome {
        out.push(Mudanca::nova(
            TipoMudanca::TextoMudou,
            Impacto::Nenhum,
            "fluxo",
            format!(
                "o nome do fluxo mudou de \"{}\" para \"{}\".",
                a.fluxo.nome, b.fluxo.nome
            ),
        ));
    }
    if a.fluxo.descricao != b.fluxo.descricao {
        out.push(Mudanca::nova(
            TipoMudanca::TextoMudou,
            Impacto::Nenhum,
            "fluxo",
            format!(
                "a descrição do fluxo mudou de {} para {}.",
                texto_ou_vazio(a.fluxo.descricao.as_deref()),
                texto_ou_vazio(b.fluxo.descricao.as_deref())
            ),
        ));
    }
    if a.gatilho != b.gatilho {
        out.push(Mudanca::nova(
            TipoMudanca::GatilhoMudou,
            Impacto::AtendimentosNovos,
            "gatilho",
            "o critério para iniciar este fluxo mudou: nos atendimentos novos, o Jev passa a \
             usar o novo critério."
                .to_owned(),
        ));
    }
    if a.start != b.start {
        out.push(Mudanca::nova(
            TipoMudanca::InicioMudou,
            Impacto::AtendimentosNovos,
            "fluxo",
            format!(
                "os atendimentos novos passam a começar na etapa `{}`, e não mais na etapa `{}`.",
                b.start, a.start
            ),
        ));
    }
}

// ------------------------------------------------------------------ etapas

fn etapas(a: &Ir, b: &Ir, out: &mut Vec<Mudanca>) {
    for nova in &b.stages {
        match a.stages.iter().find(|antiga| antiga.id == nova.id) {
            Some(antiga) => comparar_etapa(a, b, antiga, nova, out),
            None => etapa_nova(b, nova, out),
        }
    }
    for antiga in &a.stages {
        if !b.stages.iter().any(|nova| nova.id == antiga.id) {
            etapa_removida(a, antiga, out);
        }
    }
}

fn etapa_nova(b: &Ir, etapa: &Etapa, out: &mut Vec<Mudanca>) {
    let alvo = alvo_etapa(&etapa.id);
    let chega = alcancada(b, &etapa.id);
    let (impacto, texto) = if chega {
        (
            Impacto::MudaRota,
            format!(
                "a etapa `{}`{} é nova e já é destino de alguma ação ou decisão: os atendimentos \
                 novos passam por ela.",
                etapa.id,
                entre_parenteses(&etapa.nome)
            ),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "a etapa `{}`{} é nova, mas nenhuma ação ou decisão leva a ela: nada muda no \
                 atendimento.",
                etapa.id,
                entre_parenteses(&etapa.nome)
            ),
        )
    };
    out.push(Mudanca::nova(TipoMudanca::EtapaNova, impacto, &alvo, texto));
    if chega {
        efeitos_da_etapa(out, etapa, true);
    }
}

fn etapa_removida(a: &Ir, etapa: &Etapa, out: &mut Vec<Mudanca>) {
    let alvo = alvo_etapa(&etapa.id);
    let chegava = alcancada(a, &etapa.id);
    let (impacto, texto) = if chegava {
        (
            Impacto::MudaRota,
            format!(
                "a etapa `{}`{} foi removida: os atendimentos novos não passam mais por ela.",
                etapa.id,
                entre_parenteses(&etapa.nome)
            ),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "a etapa `{}`{} foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
                 atendimento.",
                etapa.id,
                entre_parenteses(&etapa.nome)
            ),
        )
    };
    out.push(Mudanca::nova(
        TipoMudanca::EtapaRemovida,
        impacto,
        &alvo,
        texto,
    ));
    if chegava {
        efeitos_da_etapa(out, etapa, false);
    }
}

fn comparar_etapa(a: &Ir, b: &Ir, ea: &Etapa, eb: &Etapa, out: &mut Vec<Mudanca>) {
    let et = eb.id.as_str();
    let alvo = alvo_etapa(et);

    if ea.nome != eb.nome {
        out.push(Mudanca::nova(
            TipoMudanca::TextoMudou,
            Impacto::Nenhum,
            &alvo,
            format!(
                "o nome da etapa `{et}` mudou de \"{}\" para \"{}\".",
                ea.nome, eb.nome
            ),
        ));
    }
    if ea.instruction != eb.instruction {
        out.push(Mudanca::nova(
            TipoMudanca::InstrucaoMudou,
            Impacto::AtendimentosNovos,
            &alvo,
            format!(
                "a instrução da etapa `{et}` mudou de {} para {}: vale para os atendimentos novos.",
                texto_ou_vazio(ea.instruction.as_deref()),
                texto_ou_vazio(eb.instruction.as_deref())
            ),
        ));
    }
    if ea.action_actors != eb.action_actors {
        out.push(Mudanca::nova(
            TipoMudanca::AtoresDaEtapaMudou,
            Impacto::MudaRota,
            &alvo,
            format!(
                "as ações da etapa `{et}` passam a ser feitas por {}.",
                atores_em_texto(&eb.action_actors)
            ),
        ));
    }

    for cb in &eb.campos {
        match ea.campos.iter().find(|ca| ca.id == cb.id) {
            Some(ca) => comparar_campo(et, ca, cb, out),
            None => campo_novo(et, cb, out),
        }
    }
    for ca in &ea.campos {
        if !eb.campos.iter().any(|cb| cb.id == ca.id) {
            campo_removido(et, ca, out);
        }
    }

    for ab in &eb.actions {
        match ea.actions.iter().find(|aa| aa.id == ab.id) {
            Some(aa) => comparar_acao(a, b, eb, aa, ab, out),
            None => acao_nova(b, eb, ab, out),
        }
    }
    for aa in &ea.actions {
        if !eb.actions.iter().any(|ab| ab.id == aa.id) {
            acao_removida(a, ea, aa, out);
        }
    }

    for efeito in sobras(&eb.on_enter, &ea.on_enter) {
        efeito_mudou(out, Lugar::Entrada(et), efeito, true);
    }
    for efeito in sobras(&ea.on_enter, &eb.on_enter) {
        efeito_mudou(out, Lugar::Entrada(et), efeito, false);
    }
}

// ------------------------------------------------------------------ campos

fn comparar_campo(et: &str, ca: &Campo, cb: &Campo, out: &mut Vec<Mudanca>) {
    let cid = cb.id.as_str();
    let alvo = format!("etapa:{et}/campo:{cid}");

    if ca.label != cb.label {
        out.push(Mudanca::nova(
            TipoMudanca::TextoMudou,
            Impacto::Nenhum,
            &alvo,
            format!(
                "o rótulo do campo `{cid}` na etapa `{et}` mudou de \"{}\" para \"{}\": o agente \
                 passa a usar o novo nome ao perguntar.",
                ca.label, cb.label
            ),
        ));
    }
    if ca.tipo != cb.tipo {
        out.push(Mudanca::nova(
            TipoMudanca::CampoMudouTipo,
            Impacto::PedeDadoNovo,
            &alvo,
            format!(
                "o campo `{cid}` da etapa `{et}` mudou de {} para {}: o agente passará a pedir o \
                 dado do novo jeito.",
                tipo_em_texto(ca.tipo),
                tipo_em_texto(cb.tipo)
            ),
        ));
    }
    if ca.options != cb.options {
        out.push(Mudanca::nova(
            TipoMudanca::CampoMudouOpcoes,
            Impacto::PedeDadoNovo,
            &alvo,
            format!(
                "as opções do campo `{cid}` da etapa `{et}` mudaram de {} para {}.",
                opcoes_em_texto(&ca.options),
                opcoes_em_texto(&cb.options)
            ),
        ));
    }
    if !ca.required && cb.required {
        out.push(Mudanca::nova(
            TipoMudanca::CampoAgoraObrigatorio,
            Impacto::PedeDadoNovo,
            &alvo,
            format!(
                "o campo `{cid}` da etapa `{et}` agora é obrigatório: o agente vai pedi-lo antes \
                 de concluir a etapa."
            ),
        ));
    }
    if ca.required && !cb.required {
        out.push(Mudanca::nova(
            TipoMudanca::CampoDeixouDeSerObrigatorio,
            Impacto::AtendimentosNovos,
            &alvo,
            format!(
                "o campo `{cid}` da etapa `{et}` deixou de ser obrigatório: nos atendimentos \
                 novos, a etapa pode ser concluída sem ele."
            ),
        ));
    }
    if ca.help != cb.help {
        out.push(Mudanca::nova(
            TipoMudanca::TextoMudou,
            Impacto::Nenhum,
            &alvo,
            format!(
                "a dica do campo `{cid}` na etapa `{et}` mudou: ela é só para quem edita o \
                 fluxo, o atendimento segue igual."
            ),
        ));
    }
}

fn campo_novo(et: &str, campo: &Campo, out: &mut Vec<Mudanca>) {
    let cid = campo.id.as_str();
    let obrigatorio = if campo.required {
        " Ele é obrigatório."
    } else {
        ""
    };
    out.push(Mudanca::nova(
        TipoMudanca::CampoNovo,
        Impacto::PedeDadoNovo,
        &format!("etapa:{et}/campo:{cid}"),
        format!(
            "a etapa `{et}` passa a coletar o campo `{cid}`{}: o agente pedirá esse dado nos \
             atendimentos novos.{obrigatorio}",
            entre_parenteses(&campo.label)
        ),
    ));
}

fn campo_removido(et: &str, campo: &Campo, out: &mut Vec<Mudanca>) {
    let cid = campo.id.as_str();
    out.push(Mudanca::nova(
        TipoMudanca::CampoRemovido,
        Impacto::AtendimentosNovos,
        &format!("etapa:{et}/campo:{cid}"),
        format!(
            "a etapa `{et}` deixa de coletar o campo `{cid}`{}: nos atendimentos novos, o agente \
             não pedirá esse dado.",
            entre_parenteses(&campo.label)
        ),
    ));
}

// ------------------------------------------------------------------ ações

fn comparar_acao(a: &Ir, b: &Ir, et_b: &Etapa, aa: &Acao, ab: &Acao, out: &mut Vec<Mudanca>) {
    let et = et_b.id.as_str();
    let ac = ab.id.as_str();
    let alvo = format!("etapa:{et}/acao:{ac}");

    if aa.label != ab.label {
        out.push(Mudanca::nova(
            TipoMudanca::TextoMudou,
            Impacto::Nenhum,
            &alvo,
            format!(
                "o rótulo da ação `{ac}` da etapa `{et}` mudou de \"{}\" para \"{}\".",
                aa.label, ab.label
            ),
        ));
    }
    for campo in &ab.requires {
        if !aa.requires.contains(campo) {
            out.push(Mudanca::nova(
                TipoMudanca::AcaoPassouAExigir,
                Impacto::PedeDadoNovo,
                &alvo,
                format!(
                    "a ação `{ac}` da etapa `{et}` agora exige o campo `{campo}`{}: o agente \
                     passará a pedir esse dado antes de avançar.",
                    entre_parenteses(rotulo_do_campo(et_b, campo))
                ),
            ));
        }
    }
    for campo in &aa.requires {
        if !ab.requires.contains(campo) {
            out.push(Mudanca::nova(
                TipoMudanca::AcaoDeixouDeExigir,
                Impacto::AtendimentosNovos,
                &alvo,
                format!(
                    "a ação `{ac}` da etapa `{et}` deixou de exigir o campo `{campo}`: nos \
                     atendimentos novos, o agente pode avançar sem ele."
                ),
            ));
        }
    }
    if aa.actor != ab.actor {
        let texto = match ab.actor {
            Some(ator) => format!(
                "a ação `{ac}` da etapa `{et}` passa a ser feita só pelo {}.",
                ator.as_text()
            ),
            None => format!(
                "a ação `{ac}` da etapa `{et}` passa a poder ser feita por qualquer ator da etapa."
            ),
        };
        out.push(Mudanca::nova(
            TipoMudanca::AtorDaAcaoMudou,
            Impacto::MudaRota,
            &alvo,
            texto,
        ));
    }
    if chave_destino(&aa.goes_to) != chave_destino(&ab.goes_to) {
        out.push(Mudanca::nova(
            TipoMudanca::DestinoMudou,
            Impacto::MudaRota,
            &alvo,
            format!(
                "a ação `{ac}` da etapa `{et}` agora leva para {}, e antes levava para {}.",
                destino_frase(b, &ab.goes_to),
                destino_frase(a, &aa.goes_to)
            ),
        ));
    }
    for efeito in sobras(&ab.effects, &aa.effects) {
        efeito_mudou(out, Lugar::Acao(et, ac), efeito, true);
    }
    for efeito in sobras(&aa.effects, &ab.effects) {
        efeito_mudou(out, Lugar::Acao(et, ac), efeito, false);
    }
}

fn acao_nova(b: &Ir, etapa: &Etapa, acao: &Acao, out: &mut Vec<Mudanca>) {
    let et = etapa.id.as_str();
    let ac = acao.id.as_str();
    let chega = alcancada(b, et);
    let (impacto, texto) = if chega {
        (
            Impacto::MudaRota,
            format!(
                "a etapa `{et}` ganhou a ação `{ac}`{}, que leva para {}.",
                entre_parenteses(&acao.label),
                destino_frase(b, &acao.goes_to)
            ),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "a etapa `{et}` ganhou a ação `{ac}`{}, mas a etapa não é alcançada por nenhuma \
                 ação ou decisão: nada muda no atendimento.",
                entre_parenteses(&acao.label)
            ),
        )
    };
    out.push(Mudanca::nova(
        TipoMudanca::AcaoNova,
        impacto,
        &format!("etapa:{et}/acao:{ac}"),
        texto,
    ));
    if chega {
        for efeito in &acao.effects {
            efeito_mudou(out, Lugar::Acao(et, ac), efeito, true);
        }
    }
}

fn acao_removida(a: &Ir, etapa: &Etapa, acao: &Acao, out: &mut Vec<Mudanca>) {
    let et = etapa.id.as_str();
    let ac = acao.id.as_str();
    let chegava = alcancada(a, et);
    let (impacto, texto) = if chegava {
        (
            Impacto::MudaRota,
            format!(
                "a etapa `{et}` perdeu a ação `{ac}`{}: o agente não poderá mais usá-la.",
                entre_parenteses(&acao.label)
            ),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "a etapa `{et}` perdeu a ação `{ac}`{}, mas a etapa não era alcançada por nenhuma \
                 ação ou decisão: nada muda no atendimento.",
                entre_parenteses(&acao.label)
            ),
        )
    };
    out.push(Mudanca::nova(
        TipoMudanca::AcaoRemovida,
        impacto,
        &format!("etapa:{et}/acao:{ac}"),
        texto,
    ));
    if chegava {
        for efeito in &acao.effects {
            efeito_mudou(out, Lugar::Acao(et, ac), efeito, false);
        }
    }
}

// ------------------------------------------------------------------ decisões

fn decisoes(a: &Ir, b: &Ir, out: &mut Vec<Mudanca>) {
    for nova in &b.decisions {
        match a.decisions.iter().find(|antiga| antiga.id == nova.id) {
            Some(antiga) => comparar_decisao(a, b, antiga, nova, out),
            None => decisao_nova(b, nova, out),
        }
    }
    for antiga in &a.decisions {
        if !b.decisions.iter().any(|nova| nova.id == antiga.id) {
            decisao_removida(a, antiga, out);
        }
    }
}

fn comparar_decisao(a: &Ir, b: &Ir, da: &Decisao, db: &Decisao, out: &mut Vec<Mudanca>) {
    let id = db.id.as_str();
    let alvo = format!("decisao:{id}");
    if da.se != db.se {
        out.push(Mudanca::nova(
            TipoMudanca::RegraMudou,
            Impacto::MudaRota,
            &alvo,
            format!(
                "a condição da decisão `{id}` mudou: agora é verdadeira quando {}.",
                regra_em_texto(&db.se)
            ),
        ));
    }
    if chave_destino(&da.then_branch) != chave_destino(&db.then_branch) {
        out.push(Mudanca::nova(
            TipoMudanca::DestinoMudou,
            Impacto::MudaRota,
            &format!("{alvo}/ramo:entao"),
            format!(
                "o ramo verdadeiro da decisão `{id}` agora leva para {}, e antes levava para {}.",
                destino_frase(b, &db.then_branch),
                destino_frase(a, &da.then_branch)
            ),
        ));
    }
    if chave_destino(&da.else_branch) != chave_destino(&db.else_branch) {
        out.push(Mudanca::nova(
            TipoMudanca::DestinoMudou,
            Impacto::MudaRota,
            &format!("{alvo}/ramo:senao"),
            format!(
                "o ramo falso da decisão `{id}` agora leva para {}, e antes levava para {}.",
                destino_frase(b, &db.else_branch),
                destino_frase(a, &da.else_branch)
            ),
        ));
    }
}

fn decisao_nova(b: &Ir, decisao: &Decisao, out: &mut Vec<Mudanca>) {
    let id = decisao.id.as_str();
    let (impacto, texto) = if alcancada(b, id) {
        (
            Impacto::MudaRota,
            format!(
                "a decisão `{id}` é nova e já é destino de alguma ação ou decisão: os atendimentos \
                 novos passam por ela."
            ),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "a decisão `{id}` é nova, mas nenhuma ação ou decisão leva a ela: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Mudanca::nova(
        TipoMudanca::DecisaoNova,
        impacto,
        &format!("decisao:{id}"),
        texto,
    ));
}

fn decisao_removida(a: &Ir, decisao: &Decisao, out: &mut Vec<Mudanca>) {
    let id = decisao.id.as_str();
    let (impacto, texto) = if alcancada(a, id) {
        (
            Impacto::MudaRota,
            format!(
                "a decisão `{id}` foi removida: os atendimentos novos não passam mais por ela."
            ),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "a decisão `{id}` foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Mudanca::nova(
        TipoMudanca::DecisaoRemovida,
        impacto,
        &format!("decisao:{id}"),
        texto,
    ));
}

// ------------------------------------------------------------------ fins

fn fins(a: &Ir, b: &Ir, out: &mut Vec<Mudanca>) {
    for nova in &b.ends {
        match a.ends.iter().find(|antigo| antigo.id == nova.id) {
            Some(antigo) => comparar_fim(antigo, nova, out),
            None => fim_novo(b, nova, out),
        }
    }
    for antigo in &a.ends {
        if !b.ends.iter().any(|novo| novo.id == antigo.id) {
            fim_removido(a, antigo, out);
        }
    }
}

fn comparar_fim(antigo: &Fim, novo: &Fim, out: &mut Vec<Mudanca>) {
    let id = novo.id.as_str();
    let alvo = format!("fim:{id}");
    if antigo.ends_service != novo.ends_service {
        let texto = if novo.ends_service {
            format!(
                "o fim `{id}` agora também encerra o atendimento: a conversa e a Ficha são \
                 fechadas ao chegar nele."
            )
        } else {
            format!("o fim `{id}` deixou de encerrar o atendimento: a conversa continua aberta.")
        };
        out.push(Mudanca::nova(
            TipoMudanca::FimMudou,
            Impacto::MudaRota,
            &alvo,
            texto,
        ));
    }
    if antigo.resultado != novo.resultado {
        out.push(Mudanca::nova(
            TipoMudanca::FimMudou,
            Impacto::Nenhum,
            &alvo,
            format!(
                "o resumo do fim `{id}` mudou de {} para {}.",
                texto_ou_vazio(antigo.resultado.as_deref()),
                texto_ou_vazio(novo.resultado.as_deref())
            ),
        ));
    }
}

fn fim_novo(b: &Ir, fim: &Fim, out: &mut Vec<Mudanca>) {
    let id = fim.id.as_str();
    let (impacto, texto) = if alcancada(b, id) {
        (
            Impacto::MudaRota,
            format!(
                "o fim `{id}` é novo e já é destino de alguma ação ou decisão: os atendimentos \
                 novos podem terminar nele."
            ),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "o fim `{id}` é novo, mas nenhuma ação ou decisão leva a ele: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Mudanca::nova(
        TipoMudanca::FimNovo,
        impacto,
        &format!("fim:{id}"),
        texto,
    ));
}

fn fim_removido(a: &Ir, fim: &Fim, out: &mut Vec<Mudanca>) {
    let id = fim.id.as_str();
    let (impacto, texto) = if alcancada(a, id) {
        (
            Impacto::MudaRota,
            format!("o fim `{id}` foi removido: os atendimentos novos não terminam mais nele."),
        )
    } else {
        (
            Impacto::Nenhum,
            format!(
                "o fim `{id}` foi removido, mas nenhuma ação ou decisão o usava: nada muda no \
                 atendimento."
            ),
        )
    };
    out.push(Mudanca::nova(
        TipoMudanca::FimRemovido,
        impacto,
        &format!("fim:{id}"),
        texto,
    ));
}

// ------------------------------------------------------------------ defs

fn defs(a: &Ir, b: &Ir, out: &mut Vec<Mudanca>) {
    for nova in &b.defs {
        let alvo = format!("def:{}", nova.nome);
        match a.defs.iter().find(|antiga| antiga.nome == nova.nome) {
            None => {
                let usada = usada_por_decisao(b, &nova.nome);
                let (impacto, texto) = if usada {
                    (
                        Impacto::MudaRota,
                        format!(
                            "a regra especial `{}` é nova e já é usada por decisão: a condição \
                             dessas decisões passa a usá-la.",
                            nova.nome
                        ),
                    )
                } else {
                    (
                        Impacto::Nenhum,
                        format!(
                            "a regra especial `{}` é nova, mas nenhuma decisão a usa: nada muda \
                             no atendimento.",
                            nova.nome
                        ),
                    )
                };
                out.push(Mudanca::nova(
                    TipoMudanca::RegraMudou,
                    impacto,
                    &alvo,
                    texto,
                ));
            }
            Some(antiga) if antiga.fonte != nova.fonte => {
                let usada = usada_por_decisao(b, &nova.nome);
                let (impacto, texto) = if usada {
                    (
                        Impacto::MudaRota,
                        format!(
                            "o texto da regra especial `{}` mudou: as decisões que a usam passam \
                             a decidir com a nova versão.",
                            nova.nome
                        ),
                    )
                } else {
                    (
                        Impacto::Nenhum,
                        format!(
                            "o texto da regra especial `{}` mudou, mas nenhuma decisão a usa: \
                             nada muda no atendimento.",
                            nova.nome
                        ),
                    )
                };
                out.push(Mudanca::nova(
                    TipoMudanca::RegraMudou,
                    impacto,
                    &alvo,
                    texto,
                ));
            }
            Some(_) => {}
        }
    }
    for antiga in &a.defs {
        if !b.defs.iter().any(|nova| nova.nome == antiga.nome) {
            let usada = usada_por_decisao(a, &antiga.nome);
            let (impacto, texto) = if usada {
                (
                    Impacto::MudaRota,
                    format!(
                        "a regra especial `{}` foi removida: as decisões que a usavam deixam de \
                         ter essa condição.",
                        antiga.nome
                    ),
                )
            } else {
                (
                    Impacto::Nenhum,
                    format!(
                        "a regra especial `{}` foi removida; nenhuma decisão a usava.",
                        antiga.nome
                    ),
                )
            };
            out.push(Mudanca::nova(
                TipoMudanca::RegraMudou,
                impacto,
                &format!("def:{}", antiga.nome),
                texto,
            ));
        }
    }
}

// ------------------------------------------------------------------ efeitos

/// Onde um efeito é disparado: ao entrar na etapa, ou junto de uma ação dela.
#[derive(Clone, Copy)]
enum Lugar<'a> {
    Entrada(&'a str),
    Acao(&'a str, &'a str),
}

fn efeitos_da_etapa(out: &mut Vec<Mudanca>, etapa: &Etapa, novo: bool) {
    for efeito in &etapa.on_enter {
        efeito_mudou(out, Lugar::Entrada(&etapa.id), efeito, novo);
    }
    for acao in &etapa.actions {
        for efeito in &acao.effects {
            efeito_mudou(out, Lugar::Acao(&etapa.id, &acao.id), efeito, novo);
        }
    }
}

fn efeito_mudou(out: &mut Vec<Mudanca>, lugar: Lugar, efeito: &Effect, novo: bool) {
    let (tipo, verbo) = if novo {
        (TipoMudanca::EfeitoNovo, "passa a")
    } else {
        (TipoMudanca::EfeitoRemovido, "deixa de")
    };
    let descricao = descrever_efeito(efeito);
    let (alvo, texto) = match lugar {
        Lugar::Entrada(et) => (
            alvo_etapa(et),
            format!("ao entrar na etapa `{et}`, o fluxo {verbo} {descricao}."),
        ),
        Lugar::Acao(et, ac) => (
            format!("etapa:{et}/acao:{ac}"),
            format!("a ação `{ac}` da etapa `{et}` {verbo} {descricao}."),
        ),
    };
    out.push(Mudanca::nova(tipo, Impacto::DisparaEfeito, &alvo, texto));
}

fn descrever_efeito(efeito: &Effect) -> String {
    match efeito {
        Effect::MessageToCustomer { texto } => format!("enviar ao cliente: \"{texto}\""),
        Effect::NotifyWhatsApp { para, mensagem } => {
            format!("enviar à equipe, no número {para}: \"{mensagem}\"")
        }
        Effect::Encaminhar { departamento } => {
            format!("encaminhar o atendimento ao departamento `{departamento}`")
        }
    }
}

// ------------------------------------------------------------------ auxiliares

/// Itens de `origem` que não têm par em `outra`, contando repetição (multiconjunto). Mantém a
/// ordem de `origem`.
fn sobras<'a, T: PartialEq>(origem: &'a [T], outra: &[T]) -> Vec<&'a T> {
    let mut livres: Vec<&T> = outra.iter().collect();
    let mut sobra = Vec::new();
    for item in origem {
        match livres.iter().position(|livre| *livre == item) {
            Some(posicao) => {
                livres.swap_remove(posicao);
            }
            None => sobra.push(item),
        }
    }
    sobra
}

/// O nó é alcançado quando é o início do fluxo ou o destino de alguma ação ou ramo de decisão.
fn alcancada(ir: &Ir, id: &str) -> bool {
    ir.start == id
        || ir
            .stages
            .iter()
            .flat_map(|etapa| etapa.actions.iter())
            .any(|acao| acao.goes_to.id() == id)
        || ir
            .decisions
            .iter()
            .any(|decisao| decisao.then_branch.id() == id || decisao.else_branch.id() == id)
}

fn usada_por_decisao(ir: &Ir, nome: &str) -> bool {
    ir.decisions
        .iter()
        .any(|decisao| regra_chama(&decisao.se, nome))
}

fn regra_chama(regra: &Rule, nome: &str) -> bool {
    match regra {
        Rule::Funcao { nome: chamada } => chamada == nome,
        Rule::E { args } | Rule::Ou { args } | Rule::Nao { args } => {
            args.iter().any(|filha| regra_chama(filha, nome))
        }
        Rule::Igual { .. }
        | Rule::Diferente { .. }
        | Rule::Maior { .. }
        | Rule::Menor { .. }
        | Rule::Contem { .. }
        | Rule::Vazio { .. } => false,
    }
}

/// Chave de comparação de um destino: o tipo e o id do nó. O resumo de um fim (resultado e
/// encerramento) não entra aqui: essa mudança é do próprio fim (`fim_mudou`).
fn chave_destino(destino: &Destination) -> (&'static str, &str) {
    match destino {
        Destination::Etapa { id } => ("etapa", id),
        Destination::Decisao { id } => ("decisao", id),
        Destination::Fim { id, .. } => ("fim", id),
    }
}

/// Destino em palavras, lido da revisão que contém o nó (para dar o nome da etapa).
fn destino_frase(ir: &Ir, destino: &Destination) -> String {
    match destino {
        Destination::Etapa { id } => match ir.stages.iter().find(|etapa| etapa.id == *id) {
            Some(etapa) => format!("a etapa `{id}`{}", entre_parenteses(&etapa.nome)),
            None => format!("a etapa `{id}`"),
        },
        Destination::Decisao { id } => format!("a decisão `{id}`"),
        Destination::Fim {
            id,
            ends_service: encerra_atendimento,
            ..
        } => {
            if *encerra_atendimento {
                format!("o fim `{id}`, que encerra o atendimento")
            } else {
                format!("o fim `{id}`")
            }
        }
    }
}

fn regra_em_texto(regra: &Rule) -> String {
    match regra {
        Rule::Igual { campo, value: valor } => format!("`{campo}` é igual a \"{valor}\""),
        Rule::Diferente { campo, value: valor } => format!("`{campo}` é diferente de \"{valor}\""),
        Rule::Maior { campo, value: valor } => format!("`{campo}` é maior que {valor}"),
        Rule::Menor { campo, value: valor } => format!("`{campo}` é menor que {valor}"),
        Rule::Contem { campo, value: valor } => format!("`{campo}` contém \"{valor}\""),
        Rule::Vazio { campo } => format!("`{campo}` está vazio"),
        Rule::E { args } => format!("({})", juntar(args, " e ")),
        Rule::Ou { args } => format!("({})", juntar(args, " ou ")),
        Rule::Nao { args } => format!("não ({})", juntar(args, " e ")),
        Rule::Funcao { nome } => format!("a regra especial `{nome}` for verdadeira"),
    }
}

fn juntar(regras: &[Rule], separador: &str) -> String {
    regras
        .iter()
        .map(regra_em_texto)
        .collect::<Vec<_>>()
        .join(separador)
}

fn alvo_etapa(id: &str) -> String {
    format!("etapa:{id}")
}

fn rotulo_do_campo<'a>(etapa: &'a Etapa, campo: &str) -> &'a str {
    etapa
        .campos
        .iter()
        .find(|candidato| candidato.id == campo)
        .map_or("", |candidato| candidato.label.as_str())
}

/// ` (texto)` quando há texto; vazio quando não há.
fn entre_parenteses(texto: &str) -> String {
    if texto.trim().is_empty() {
        String::new()
    } else {
        format!(" ({texto})")
    }
}

fn texto_ou_vazio(texto: Option<&str>) -> String {
    match texto {
        Some(valor) => format!("\"{valor}\""),
        None => "(vazio)".to_owned(),
    }
}

fn atores_em_texto(atores: &[ActionActor]) -> String {
    if atores.is_empty() {
        return "ninguém (a etapa não define ator)".to_owned();
    }
    atores
        .iter()
        .map(|ator| ator.as_text())
        .collect::<Vec<_>>()
        .join(" ou ")
}

fn opcoes_em_texto(opcoes: &[String]) -> String {
    if opcoes.is_empty() {
        "(nenhuma)".to_owned()
    } else {
        format!("({})", opcoes.join(", "))
    }
}

fn tipo_em_texto(tipo: FieldKind) -> &'static str {
    match tipo {
        FieldKind::Text => "texto",
        FieldKind::Number => "número",
        FieldKind::Data => "data",
        FieldKind::Boolean => "sim ou não",
        FieldKind::Attachment => "anexo",
        FieldKind::Option => "opção",
    }
}
