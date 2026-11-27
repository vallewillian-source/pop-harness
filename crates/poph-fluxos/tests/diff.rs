//! Testes da diferença entre revisões em linguagem comum (FX-17).
//!
//! Cada tipo de mudança tem um teste com o texto exato (`assert_eq!`, não `contains`) e o impacto.
//! Os outros testes cobrem fluxos idênticos (lista vazia), ordem estável, o impacto que não muda
//! nada e a correspondência entre os códigos e o JSON da API. A base é a IR dourada de
//! `agendar_visita` (a mesma do bloco do prompt, FX-18); cada caso altera só o que precisa.

use poph_fluxos::ir::{
    Acao, Campo, Decisao, Def, Destination, Effect, Etapa, Fim, Fluxo, Gatilho, Ir, Rule,
};
use poph_fluxos::{Change, ChangeType, Impact, diff};
use poph_types::{ActionActor, FieldKind};

// ------------------------------------------------------------------ auxiliares

fn base() -> Ir {
    let path = format!(
        "{}/../../tests/fluxos/ir/agendar_visita.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let texto = std::fs::read_to_string(&path).unwrap_or_else(|erro| panic!("{path}: {erro}"));
    Ir::from_json(&texto).expect("a IR dourada é JSON válido")
}

/// Cópia da IR com a alteração aplicada.
fn com(ir: &Ir, alterar: impl FnOnce(&mut Ir)) -> Ir {
    let mut copia = ir.clone();
    alterar(&mut copia);
    copia
}

fn em_etapa<'a>(ir: &'a mut Ir, id: &str) -> &'a mut Etapa {
    ir.stages
        .iter_mut()
        .find(|etapa| etapa.id == id)
        .expect("a etapa existe na base")
}

fn em_acao<'a>(ir: &'a mut Ir, etapa: &str, acao: &str) -> &'a mut Acao {
    em_etapa(ir, etapa)
        .actions
        .iter_mut()
        .find(|candidata| candidata.id == acao)
        .expect("a ação existe na base")
}

fn campo(id: &str, rotulo: &str, tipo: FieldKind, obrigatorio: bool, options: &[&str]) -> Campo {
    Campo {
        id: id.to_owned(),
        label: rotulo.to_owned(),
        tipo,
        required: obrigatorio,
        options: options.iter().map(|opcao| (*opcao).to_owned()).collect(),
        help: None,
    }
}

fn acao(
    id: &str,
    rotulo: &str,
    exige: &[&str],
    actor: Option<ActionActor>,
    vai_para: Destination,
) -> Acao {
    Acao {
        id: id.to_owned(),
        label: rotulo.to_owned(),
        requires: exige.iter().map(|campo| (*campo).to_owned()).collect(),
        actor,
        goes_to: vai_para,
        effects: Vec::new(),
    }
}

fn etapa(id: &str, nome: &str, instrucao: Option<&str>, acoes: Vec<Acao>) -> Etapa {
    Etapa {
        id: id.to_owned(),
        nome: nome.to_owned(),
        instruction: instrucao.map(str::to_owned),
        action_actors: vec![ActionActor::Agent],
        campos: Vec::new(),
        actions: acoes,
        on_enter: Vec::new(),
    }
}

fn no_etapa(id: &str) -> Destination {
    Destination::Etapa { id: id.to_owned() }
}

fn no_fim(id: &str, resultado: Option<&str>, ends_service: bool) -> Destination {
    Destination::Fim {
        id: id.to_owned(),
        resultado: resultado.map(str::to_owned),
        ends_service,
    }
}

fn fim(id: &str, resultado: Option<&str>, ends_service: bool) -> Fim {
    Fim {
        id: id.to_owned(),
        resultado: resultado.map(str::to_owned),
        ends_service,
    }
}

fn decisao(id: &str, se: Rule, entao: Destination, senao: Destination) -> Decisao {
    Decisao {
        id: id.to_owned(),
        se,
        then_branch: entao,
        else_branch: senao,
    }
}

fn igual(campo: &str, value: &str) -> Rule {
    Rule::Igual {
        campo: campo.to_owned(),
        value: value.to_owned(),
    }
}

/// Acha a única mudança do tipo; falha com a lista inteira se houver outra ou nenhuma.
fn unica(lista: &[Change], tipo: ChangeType) -> &Change {
    let achadas: Vec<&Change> = lista
        .iter()
        .filter(|mudanca| mudanca.tipo == tipo)
        .collect();
    assert_eq!(
        achadas.len(),
        1,
        "esperava uma mudança {tipo:?}; veio {lista:#?}"
    );
    achadas[0]
}

fn tipos(lista: &[Change]) -> Vec<ChangeType> {
    lista.iter().map(|mudanca| mudanca.tipo).collect()
}

const TODOS_OS_TIPOS: [ChangeType; 27] = [
    ChangeType::NewStage,
    ChangeType::RemovedStage,
    ChangeType::InstructionChanged,
    ChangeType::StageActorsChanged,
    ChangeType::NewField,
    ChangeType::RemovedField,
    ChangeType::FieldNowRequired,
    ChangeType::FieldNoLongerRequired,
    ChangeType::FieldTypeChanged,
    ChangeType::FieldOptionsChanged,
    ChangeType::ActionNowRequires,
    ChangeType::ActionNoLongerRequires,
    ChangeType::NewAction,
    ChangeType::RemovedAction,
    ChangeType::DestinationChanged,
    ChangeType::ActionActorChanged,
    ChangeType::RuleChanged,
    ChangeType::NewDecision,
    ChangeType::RemovedDecision,
    ChangeType::NewEnd,
    ChangeType::RemovedEnd,
    ChangeType::EndChanged,
    ChangeType::NewEffect,
    ChangeType::RemovedEffect,
    ChangeType::TriggerChanged,
    ChangeType::StartChanged,
    ChangeType::TextChanged,
];

// ------------------------------------------------------------------ identidade

#[test]
fn fluxos_identicos_devolvem_lista_vazia() {
    let ir = base();
    assert!(diff(&ir, &ir).is_empty());
}

#[test]
fn a_ida_e_volta_pelo_json_nao_gera_mudanca() {
    let ir = base();
    let volta = Ir::from_json(&ir.to_json()).expect("a IR gravada volta a ser lida");
    assert!(diff(&ir, &volta).is_empty());
}

#[test]
fn a_mesma_entrada_da_sempre_a_mesma_lista_na_mesma_ordem() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").nome = "Colher dados".to_owned();
        em_acao(ir, "coleta", "confirmar").goes_to =
            no_fim("concluida", Some("Visita agendada"), false);
        ir.gatilho.when = "Quando o cliente pedir remarcação.".to_owned();
    });
    let primeira = diff(&a, &b);
    let segunda = diff(&a, &b);
    assert_eq!(primeira, segunda);
    assert_eq!(
        tipos(&primeira),
        vec![
            ChangeType::TriggerChanged,
            ChangeType::TextChanged,
            ChangeType::DestinationChanged,
        ],
        "o fluxo vem primeiro, depois as etapas"
    );
}

#[test]
fn a_ordem_dos_itens_no_script_nao_gera_mudanca() {
    let a = base();
    let b = com(&a, |ir| ir.stages.reverse());
    assert!(diff(&a, &b).is_empty());
}

#[test]
fn codigos_de_tipo_e_impacto_batem_com_o_json_da_api() {
    for tipo in TODOS_OS_TIPOS {
        let json = serde_json::to_string(&tipo).expect("o tipo serializa");
        assert_eq!(json, format!("\"{}\"", tipo.code()), "tipo {tipo:?}");
    }
    for impact in [
        Impact::None,
        Impact::NewServices,
        Impact::NewDataRequested,
        Impact::RouteChanged,
        Impact::EffectTriggered,
    ] {
        let json = serde_json::to_string(&impact).expect("o impacto serializa");
        assert_eq!(json, format!("\"{}\"", impact.code()), "impacto {impact:?}");
    }
}

// ------------------------------------------------------------------ etapas

#[test]
fn etapa_nova_sem_chegada_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| {
        ir.stages
            .push(etapa("revisao", "Revisar dados", None, Vec::new()));
    });
    let lista = diff(&a, &b);
    assert_eq!(lista.len(), 1, "{lista:#?}");
    let nova = unica(&lista, ChangeType::NewStage);
    assert_eq!(nova.impact, Impact::None);
    assert_eq!(nova.target, "etapa:revisao");
    assert_eq!(
        nova.texto,
        "a etapa `revisao` (Revisar dados) é nova, mas nenhuma ação ou decisão leva a ela: nada \
         muda no atendimento."
    );
}

#[test]
fn etapa_nova_que_recebe_chegada_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        ir.stages
            .push(etapa("revisao", "Revisar dados", None, Vec::new()));
        em_acao(ir, "coleta", "confirmar").goes_to = no_etapa("revisao");
    });
    let lista = diff(&a, &b);
    let nova = unica(&lista, ChangeType::NewStage);
    assert_eq!(nova.impact, Impact::RouteChanged);
    assert_eq!(
        nova.texto,
        "a etapa `revisao` (Revisar dados) é nova e já é destino de alguma ação ou decisão: os \
         atendimentos novos passam por ela."
    );
}

#[test]
fn etapa_removida_que_era_alcancada_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        ir.stages.retain(|etapa| etapa.id != "confirmacao");
        em_acao(ir, "coleta", "confirmar").goes_to =
            no_fim("concluida", Some("Visita agendada"), false);
    });
    let lista = diff(&a, &b);
    let removida = unica(&lista, ChangeType::RemovedStage);
    assert_eq!(removida.impact, Impact::RouteChanged);
    assert_eq!(removida.target, "etapa:confirmacao");
    assert_eq!(
        removida.texto,
        "a etapa `confirmacao` (Confirmar visita) foi removida: os atendimentos novos não passam \
         mais por ela."
    );
}

#[test]
fn etapa_removida_sem_chegada_nao_muda_o_atendimento() {
    let a = com(&base(), |ir| {
        ir.stages.push(etapa("sobra", "Sobra", None, Vec::new()));
    });
    let b = base();
    let lista = diff(&a, &b);
    assert_eq!(lista.len(), 1, "{lista:#?}");
    let removida = unica(&lista, ChangeType::RemovedStage);
    assert_eq!(removida.impact, Impact::None);
    assert_eq!(
        removida.texto,
        "a etapa `sobra` (Sobra) foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
         atendimento."
    );
}

#[test]
fn instrucao_mudou_vale_so_para_atendimentos_novos() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").instruction = Some("Pergunte só o endereço.".to_owned());
    });
    let lista = diff(&a, &b);
    let mudou = unica(&lista, ChangeType::InstructionChanged);
    assert_eq!(mudou.impact, Impact::NewServices);
    assert_eq!(mudou.target, "etapa:coleta");
    assert_eq!(
        mudou.texto,
        "a instrução da etapa `coleta` mudou de \"Pergunte o endereço e o período preferido. Uma \
         pergunta por vez.\" para \"Pergunte só o endereço.\": vale para os atendimentos novos."
    );
}

#[test]
fn atores_da_etapa_mudou_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").action_actors = vec![ActionActor::Agent, ActionActor::Attendant];
    });
    let lista = diff(&a, &b);
    let mudou = unica(&lista, ChangeType::StageActorsChanged);
    assert_eq!(mudou.impact, Impact::RouteChanged);
    assert_eq!(
        mudou.texto,
        "as ações da etapa `coleta` passam a ser feitas por agente ou atendente."
    );
}

#[test]
fn renomear_etapa_e_campo_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").nome = "Colher dados".to_owned();
        em_etapa(ir, "coleta").campos[0].label = "Endereço".to_owned();
    });
    let lista = diff(&a, &b);
    assert_eq!(lista.len(), 2, "{lista:#?}");
    assert!(lista.iter().all(|m| m.impact == Impact::None), "{lista:#?}");
    let nome = lista
        .iter()
        .find(|m| m.target == "etapa:coleta")
        .expect("o nome da etapa aparece");
    assert_eq!(nome.tipo, ChangeType::TextChanged);
    assert_eq!(nome.target, "etapa:coleta");
    assert_eq!(
        nome.texto,
        "o nome da etapa `coleta` mudou de \"Coletar dados\" para \"Colher dados\"."
    );
    let rotulo = lista
        .iter()
        .find(|m| m.target == "etapa:coleta/campo:endereco")
        .expect("o rótulo do campo aparece");
    assert_eq!(rotulo.tipo, ChangeType::TextChanged);
    assert_eq!(
        rotulo.texto,
        "o rótulo do campo `endereco` na etapa `coleta` mudou de \"Endereço completo\" para \
         \"Endereço\": o agente passa a usar o novo nome ao perguntar."
    );
}

// ------------------------------------------------------------------ campos

#[test]
fn campo_novo_pede_dado_novo() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta")
            .campos
            .push(campo("cep", "CEP", FieldKind::Text, false, &[]));
    });
    let lista = diff(&a, &b);
    let novo = unica(&lista, ChangeType::NewField);
    assert_eq!(novo.impact, Impact::NewDataRequested);
    assert_eq!(novo.target, "etapa:coleta/campo:cep");
    assert_eq!(
        novo.texto,
        "a etapa `coleta` passa a coletar o campo `cep` (CEP): o agente pedirá esse dado nos \
         atendimentos novos."
    );
}

#[test]
fn campo_removido_deixa_de_ser_pedido_nos_atendimentos_novos() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta")
            .campos
            .retain(|campo| campo.id != "periodo");
        em_acao(ir, "coleta", "confirmar").requires = vec!["endereco".to_owned()];
    });
    let lista = diff(&a, &b);
    let removido = unica(&lista, ChangeType::RemovedField);
    assert_eq!(removido.impact, Impact::NewServices);
    assert_eq!(removido.target, "etapa:coleta/campo:periodo");
    assert_eq!(
        removido.texto,
        "a etapa `coleta` deixa de coletar o campo `periodo` (Período): nos atendimentos novos, o \
         agente não pedirá esse dado."
    );
}

#[test]
fn campo_que_fica_obrigatorio_pede_dado_novo() {
    let a = com(&base(), |ir| {
        em_etapa(ir, "coleta").campos[0].required = false;
    });
    let b = base();
    let lista = diff(&a, &b);
    let agora = unica(&lista, ChangeType::FieldNowRequired);
    assert_eq!(agora.impact, Impact::NewDataRequested);
    assert_eq!(agora.target, "etapa:coleta/campo:endereco");
    assert_eq!(
        agora.texto,
        "o campo `endereco` da etapa `coleta` agora é obrigatório: o agente vai pedi-lo antes de \
         concluir a etapa."
    );
}

#[test]
fn campo_que_deixa_de_ser_obrigatorio_vale_para_atendimentos_novos() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").campos[0].required = false;
    });
    let lista = diff(&a, &b);
    let deixou = unica(&lista, ChangeType::FieldNoLongerRequired);
    assert_eq!(deixou.impact, Impact::NewServices);
    assert_eq!(
        deixou.texto,
        "o campo `endereco` da etapa `coleta` deixou de ser obrigatório: nos atendimentos novos, a \
         etapa pode ser concluída sem ele."
    );
}

#[test]
fn campo_com_outro_tipo_pede_dado_do_novo_jeito() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").campos[1].tipo = FieldKind::Text;
    });
    let lista = diff(&a, &b);
    let mudou = unica(&lista, ChangeType::FieldTypeChanged);
    assert_eq!(mudou.impact, Impact::NewDataRequested);
    assert_eq!(
        mudou.texto,
        "o campo `periodo` da etapa `coleta` mudou de opção para texto: o agente passará a pedir o \
         dado do novo jeito."
    );
}

#[test]
fn campo_de_opcao_com_outras_opcoes_pede_dado_novo() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").campos[1]
            .options
            .push("noite".to_owned());
    });
    let lista = diff(&a, &b);
    let mudou = unica(&lista, ChangeType::FieldOptionsChanged);
    assert_eq!(mudou.impact, Impact::NewDataRequested);
    assert_eq!(
        mudou.texto,
        "as opções do campo `periodo` da etapa `coleta` mudaram de (manhã, tarde) para (manhã, \
         tarde, noite)."
    );
}

// ------------------------------------------------------------------ ações

#[test]
fn acao_nova_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").actions.push(acao(
            "remarcar",
            "Cliente quer remarcar",
            &[],
            None,
            no_etapa("confirmacao"),
        ));
    });
    let lista = diff(&a, &b);
    let nova = unica(&lista, ChangeType::NewAction);
    assert_eq!(nova.impact, Impact::RouteChanged);
    assert_eq!(nova.target, "etapa:coleta/acao:remarcar");
    assert_eq!(
        nova.texto,
        "a etapa `coleta` ganhou a ação `remarcar` (Cliente quer remarcar), que leva para a etapa \
         `confirmacao` (Confirmar visita)."
    );
}

#[test]
fn acao_removida_muda_a_rota() {
    let a = com(&base(), |ir| {
        em_etapa(ir, "coleta").actions.push(acao(
            "remarcar",
            "Cliente quer remarcar",
            &[],
            None,
            no_etapa("confirmacao"),
        ));
    });
    let b = base();
    let lista = diff(&a, &b);
    let removida = unica(&lista, ChangeType::RemovedAction);
    assert_eq!(removida.impact, Impact::RouteChanged);
    assert_eq!(removida.target, "etapa:coleta/acao:remarcar");
    assert_eq!(
        removida.texto,
        "a etapa `coleta` perdeu a ação `remarcar` (Cliente quer remarcar): o agente não poderá \
         mais usá-la."
    );
}

#[test]
fn destino_de_acao_mudou_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        em_acao(ir, "coleta", "confirmar").goes_to =
            no_fim("concluida", Some("Visita agendada"), false);
    });
    let lista = diff(&a, &b);
    let destino = unica(&lista, ChangeType::DestinationChanged);
    assert_eq!(destino.impact, Impact::RouteChanged);
    assert_eq!(destino.target, "etapa:coleta/acao:confirmar");
    assert_eq!(
        destino.texto,
        "a ação `confirmar` da etapa `coleta` agora leva para o fim `concluida`, e antes levava \
         para a etapa `confirmacao` (Confirmar visita)."
    );
}

#[test]
fn acao_que_passa_a_exigir_campo_pede_o_dado() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta")
            .campos
            .push(campo("cep", "CEP", FieldKind::Text, false, &[]));
        em_acao(ir, "coleta", "confirmar")
            .requires
            .push("cep".to_owned());
    });
    let lista = diff(&a, &b);
    assert_eq!(
        tipos(&lista),
        vec![ChangeType::NewField, ChangeType::ActionNowRequires]
    );
    let exige = unica(&lista, ChangeType::ActionNowRequires);
    assert_eq!(exige.impact, Impact::NewDataRequested);
    assert_eq!(exige.target, "etapa:coleta/acao:confirmar");
    assert_eq!(
        exige.texto,
        "a ação `confirmar` da etapa `coleta` agora exige o campo `cep` (CEP): o agente passará a \
         pedir esse dado antes de avançar."
    );
}

#[test]
fn acao_que_deixa_de_exigir_campo_vale_para_atendimentos_novos() {
    let a = base();
    let b = com(&a, |ir| {
        em_acao(ir, "coleta", "confirmar")
            .requires
            .retain(|campo| campo != "endereco");
    });
    let lista = diff(&a, &b);
    let deixou = unica(&lista, ChangeType::ActionNoLongerRequires);
    assert_eq!(deixou.impact, Impact::NewServices);
    assert_eq!(
        deixou.texto,
        "a ação `confirmar` da etapa `coleta` deixou de exigir o campo `endereco`: nos atendimentos \
         novos, o agente pode avançar sem ele."
    );
}

#[test]
fn ator_da_acao_mudou_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        em_acao(ir, "coleta", "confirmar").actor = Some(ActionActor::Attendant);
    });
    let lista = diff(&a, &b);
    let actor = unica(&lista, ChangeType::ActionActorChanged);
    assert_eq!(actor.impact, Impact::RouteChanged);
    assert_eq!(
        actor.texto,
        "a ação `confirmar` da etapa `coleta` passa a ser feita só pelo atendente."
    );
}

// ------------------------------------------------------------------ decisões, fins e defs

#[test]
fn regra_de_decisao_mudou_muda_a_rota() {
    let com_decisao = |se: Rule| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                se,
                no_etapa("confirmacao"),
                no_fim("desistencia", Some("Cliente desistiu"), false),
            ));
        })
    };
    let a = com_decisao(igual("coleta.periodo", "manhã"));
    let b = com_decisao(Rule::Diferente {
        campo: "coleta.periodo".to_owned(),
        value: "manhã".to_owned(),
    });
    let lista = diff(&a, &b);
    let regra = unica(&lista, ChangeType::RuleChanged);
    assert_eq!(regra.impact, Impact::RouteChanged);
    assert_eq!(regra.target, "decisao:classificar");
    assert_eq!(
        regra.texto,
        "a condição da decisão `classificar` mudou: agora é verdadeira quando `coleta.periodo` é \
         diferente de \"manhã\"."
    );
}

#[test]
fn ramo_de_decisao_que_muda_de_destino_muda_a_rota() {
    let com_decisao = |senao: Destination| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                igual("coleta.periodo", "manhã"),
                no_etapa("confirmacao"),
                senao,
            ));
        })
    };
    let a = com_decisao(no_fim("desistencia", Some("Cliente desistiu"), false));
    let b = com_decisao(no_fim("concluida", Some("Visita agendada"), false));
    let lista = diff(&a, &b);
    let destino = unica(&lista, ChangeType::DestinationChanged);
    assert_eq!(destino.impact, Impact::RouteChanged);
    assert_eq!(destino.target, "decisao:classificar/ramo:senao");
    assert_eq!(
        destino.texto,
        "o ramo falso da decisão `classificar` agora leva para o fim `concluida`, e antes levava \
         para o fim `desistencia`."
    );
}

#[test]
fn ramo_verdadeiro_de_decisao_que_muda_de_destino_muda_a_rota() {
    let com_decisao = |entao: Destination| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                igual("coleta.periodo", "manhã"),
                entao,
                no_fim("desistencia", Some("Cliente desistiu"), false),
            ));
        })
    };
    let a = com_decisao(no_etapa("confirmacao"));
    let b = com_decisao(no_fim("concluida", Some("Visita agendada"), false));
    let lista = diff(&a, &b);
    let destino = unica(&lista, ChangeType::DestinationChanged);
    assert_eq!(destino.impact, Impact::RouteChanged);
    assert_eq!(destino.target, "decisao:classificar/ramo:entao");
    assert_eq!(
        destino.texto,
        "o ramo verdadeiro da decisão `classificar` agora leva para o fim `concluida`, e antes \
         levava para a etapa `confirmacao` (Confirmar visita)."
    );
}

#[test]
fn decisao_nova_sem_chegada_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| {
        ir.decisions.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            no_etapa("confirmacao"),
            no_fim("desistencia", Some("Cliente desistiu"), false),
        ));
    });
    let lista = diff(&a, &b);
    let nova = unica(&lista, ChangeType::NewDecision);
    assert_eq!(nova.impact, Impact::None);
    assert_eq!(nova.target, "decisao:classificar");
    assert_eq!(
        nova.texto,
        "a decisão `classificar` é nova, mas nenhuma ação ou decisão leva a ela: nada muda no \
         atendimento."
    );
}

#[test]
fn decisao_nova_que_recebe_chegada_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        ir.decisions.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            no_etapa("confirmacao"),
            no_fim("desistencia", Some("Cliente desistiu"), false),
        ));
        em_acao(ir, "coleta", "confirmar").goes_to = Destination::Decisao {
            id: "classificar".to_owned(),
        };
    });
    let lista = diff(&a, &b);
    let nova = unica(&lista, ChangeType::NewDecision);
    assert_eq!(nova.impact, Impact::RouteChanged);
    assert_eq!(
        nova.texto,
        "a decisão `classificar` é nova e já é destino de alguma ação ou decisão: os atendimentos \
         novos passam por ela."
    );
}

#[test]
fn decisao_removida_sem_chegada_nao_muda_o_atendimento() {
    let a = com(&base(), |ir| {
        ir.decisions.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            no_etapa("confirmacao"),
            no_fim("desistencia", Some("Cliente desistiu"), false),
        ));
    });
    let b = base();
    let lista = diff(&a, &b);
    let removida = unica(&lista, ChangeType::RemovedDecision);
    assert_eq!(removida.impact, Impact::None);
    assert_eq!(
        removida.texto,
        "a decisão `classificar` foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
         atendimento."
    );
}

#[test]
fn fim_novo_sem_chegada_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| ir.ends.push(fim("nova_saida", None, false)));
    let lista = diff(&a, &b);
    let novo = unica(&lista, ChangeType::NewEnd);
    assert_eq!(novo.impact, Impact::None);
    assert_eq!(novo.target, "fim:nova_saida");
    assert_eq!(
        novo.texto,
        "o fim `nova_saida` é novo, mas nenhuma ação ou decisão leva a ele: nada muda no \
         atendimento."
    );
}

#[test]
fn fim_removido_sem_chegada_nao_muda_o_atendimento() {
    let a = com(&base(), |ir| ir.ends.push(fim("extra", None, false)));
    let b = base();
    let lista = diff(&a, &b);
    let removido = unica(&lista, ChangeType::RemovedEnd);
    assert_eq!(removido.impact, Impact::None);
    assert_eq!(
        removido.texto,
        "o fim `extra` foi removido, mas nenhuma ação ou decisão o usava: nada muda no atendimento."
    );
}

#[test]
fn fim_que_passa_a_encerrar_o_atendimento_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        ir.ends
            .iter_mut()
            .find(|fim| fim.id == "desistencia")
            .expect("o fim existe")
            .ends_service = true;
    });
    let lista = diff(&a, &b);
    let mudou = unica(&lista, ChangeType::EndChanged);
    assert_eq!(mudou.impact, Impact::RouteChanged);
    assert_eq!(mudou.target, "fim:desistencia");
    assert_eq!(
        mudou.texto,
        "o fim `desistencia` agora também encerra o atendimento: a conversa e a Ficha são \
         fechadas ao chegar nele."
    );
}

#[test]
fn resumo_de_fim_mudou_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| {
        ir.ends
            .iter_mut()
            .find(|fim| fim.id == "concluida")
            .expect("o fim existe")
            .resultado = Some("Visita confirmada".to_owned());
    });
    let lista = diff(&a, &b);
    let mudou = unica(&lista, ChangeType::EndChanged);
    assert_eq!(mudou.impact, Impact::None);
    assert_eq!(
        mudou.texto,
        "o resumo do fim `concluida` mudou de \"Visita agendada\" para \"Visita confirmada\"."
    );
}

#[test]
fn regra_especial_usada_por_decisao_muda_a_rota_quando_o_texto_muda() {
    let com_def = |fonte: &str| {
        com(&base(), |ir| {
            ir.decisions.push(decisao(
                "classificar",
                Rule::Funcao {
                    nome: "afinidade".to_owned(),
                },
                no_etapa("confirmacao"),
                no_fim("desistencia", Some("Cliente desistiu"), false),
            ));
            ir.defs.push(Def {
                nome: "afinidade".to_owned(),
                fonte: fonte.to_owned(),
            });
        })
    };
    let a = com_def("def afinidade(dados):\n    return True\n");
    let b = com_def("def afinidade(dados):\n    return False\n");
    let lista = diff(&a, &b);
    let regra = unica(&lista, ChangeType::RuleChanged);
    assert_eq!(regra.impact, Impact::RouteChanged);
    assert_eq!(regra.target, "def:afinidade");
    assert_eq!(
        regra.texto,
        "o texto da regra especial `afinidade` mudou: as decisões que a usam passam a decidir com \
         a nova versão."
    );
}

#[test]
fn regra_especial_nova_sem_uso_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| {
        ir.defs.push(Def {
            nome: "sem_uso".to_owned(),
            fonte: "def sem_uso(dados):\n    return True\n".to_owned(),
        });
    });
    let lista = diff(&a, &b);
    let nova = unica(&lista, ChangeType::RuleChanged);
    assert_eq!(nova.impact, Impact::None);
    assert_eq!(nova.target, "def:sem_uso");
    assert_eq!(
        nova.texto,
        "a regra especial `sem_uso` é nova, mas nenhuma decisão a usa: nada muda no atendimento."
    );
}

// ------------------------------------------------------------------ efeitos

#[test]
fn efeito_novo_numa_acao_dispara_efeito() {
    let a = base();
    let b = com(&a, |ir| {
        em_acao(ir, "coleta", "confirmar")
            .effects
            .push(Effect::MessageToCustomer {
                texto: "Visita marcada!".to_owned(),
            });
    });
    let lista = diff(&a, &b);
    let novo = unica(&lista, ChangeType::NewEffect);
    assert_eq!(novo.impact, Impact::EffectTriggered);
    assert_eq!(novo.target, "etapa:coleta/acao:confirmar");
    assert_eq!(
        novo.texto,
        "a ação `confirmar` da etapa `coleta` passa a enviar ao cliente: \"Visita marcada!\"."
    );
}

#[test]
fn efeito_removido_de_uma_acao_dispara_efeito() {
    let a = com(&base(), |ir| {
        em_acao(ir, "coleta", "confirmar")
            .effects
            .push(Effect::MessageToCustomer {
                texto: "Visita marcada!".to_owned(),
            });
    });
    let b = base();
    let lista = diff(&a, &b);
    let removido = unica(&lista, ChangeType::RemovedEffect);
    assert_eq!(removido.impact, Impact::EffectTriggered);
    assert_eq!(
        removido.texto,
        "a ação `confirmar` da etapa `coleta` deixa de enviar ao cliente: \"Visita marcada!\"."
    );
}

#[test]
fn efeito_de_entrada_e_aviso_a_equipe_tem_texto_proprio() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "confirmacao")
            .on_enter
            .push(Effect::NotifyWhatsApp {
                para: "+55 11 90000-0000".to_owned(),
                mensagem: "Nova visita".to_owned(),
            });
        em_etapa(ir, "confirmacao")
            .on_enter
            .push(Effect::Encaminhar {
                departamento: "Comercial".to_owned(),
            });
    });
    let lista = diff(&a, &b);
    assert_eq!(
        tipos(&lista),
        vec![ChangeType::NewEffect, ChangeType::NewEffect]
    );
    assert_eq!(
        lista[0].texto,
        "ao entrar na etapa `confirmacao`, o fluxo passa a enviar à equipe, no número +55 11 \
         90000-0000: \"Nova visita\"."
    );
    assert_eq!(
        lista[1].texto,
        "ao entrar na etapa `confirmacao`, o fluxo passa a encaminhar o atendimento ao \
         departamento `Comercial`."
    );
    assert!(lista.iter().all(|m| m.impact == Impact::EffectTriggered));
}

#[test]
fn efeito_de_etapa_nova_sem_chegada_nao_dispara() {
    let a = base();
    let b = com(&a, |ir| {
        let mut sobra = etapa("sobra", "Sobra", None, Vec::new());
        sobra.on_enter.push(Effect::MessageToCustomer {
            texto: "Oi".to_owned(),
        });
        ir.stages.push(sobra);
    });
    let lista = diff(&a, &b);
    assert_eq!(tipos(&lista), vec![ChangeType::NewStage], "{lista:#?}");
    assert_eq!(lista[0].impact, Impact::None);
}

#[test]
fn efeito_que_ja_existia_nao_aparece_de_novo() {
    let com_efeito_1 = |ir: &mut Ir| {
        em_acao(ir, "coleta", "confirmar")
            .effects
            .push(Effect::MessageToCustomer {
                texto: "Visita marcada!".to_owned(),
            });
    };
    let a = com(&base(), com_efeito_1);
    let b = com(&a, |ir| {
        em_acao(ir, "coleta", "confirmar")
            .effects
            .push(Effect::Encaminhar {
                departamento: "Comercial".to_owned(),
            });
    });
    let lista = diff(&a, &b);
    assert_eq!(tipos(&lista), vec![ChangeType::NewEffect], "{lista:#?}");
    assert_eq!(
        lista[0].texto,
        "a ação `confirmar` da etapa `coleta` passa a encaminhar o atendimento ao departamento \
         `Comercial`."
    );
}

// ------------------------------------------------------------------ fluxo

#[test]
fn gatilho_mudou_vale_para_atendimentos_novos() {
    let a = base();
    let b = com(&a, |ir| {
        ir.gatilho = Gatilho {
            when: "Quando o cliente pedir remarcação.".to_owned(),
            not_when: None,
            examples: Vec::new(),
        };
    });
    let lista = diff(&a, &b);
    let gatilho = unica(&lista, ChangeType::TriggerChanged);
    assert_eq!(gatilho.impact, Impact::NewServices);
    assert_eq!(gatilho.target, "gatilho");
    assert_eq!(
        gatilho.texto,
        "o critério para iniciar este fluxo mudou: nos atendimentos novos, o Jev passa a usar o \
         novo critério."
    );
}

#[test]
fn etapa_inicial_mudou_vale_para_atendimentos_novos() {
    let a = base();
    let b = com(&a, |ir| ir.start = "confirmacao".to_owned());
    let lista = diff(&a, &b);
    let inicio = unica(&lista, ChangeType::StartChanged);
    assert_eq!(inicio.impact, Impact::NewServices);
    assert_eq!(inicio.target, "fluxo");
    assert_eq!(
        inicio.texto,
        "os atendimentos novos passam a começar na etapa `confirmacao`, e não mais na etapa \
         `coleta`."
    );
}

#[test]
fn nome_do_fluxo_mudou_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| {
        ir.fluxo = Fluxo {
            id: ir.fluxo.id.clone(),
            nome: "Marcar visita".to_owned(),
            descricao: ir.fluxo.descricao.clone(),
        };
    });
    let lista = diff(&a, &b);
    let nome = unica(&lista, ChangeType::TextChanged);
    assert_eq!(nome.impact, Impact::None);
    assert_eq!(nome.target, "fluxo");
    assert_eq!(
        nome.texto,
        "o nome do fluxo mudou de \"Agendar visita\" para \"Marcar visita\"."
    );
}

// ------------------------------------------------------------------ ordem

#[test]
fn ordem_da_lista_numa_mudanca_grande_e_sempre_a_mesma() {
    let a = base();
    let b = com(&a, |ir| {
        ir.start = "confirmacao".to_owned();
        em_etapa(ir, "coleta").campos[0].required = false;
        em_etapa(ir, "coleta")
            .campos
            .push(campo("cep", "CEP", FieldKind::Text, false, &[]));
        em_acao(ir, "coleta", "confirmar")
            .requires
            .push("cep".to_owned());
        em_acao(ir, "coleta", "confirmar").goes_to =
            no_fim("concluida", Some("Visita agendada"), false);
        ir.stages.retain(|etapa| etapa.id != "confirmacao");
    });
    let lista = diff(&a, &b);
    assert_eq!(
        tipos(&lista),
        vec![
            ChangeType::StartChanged,
            ChangeType::FieldNoLongerRequired,
            ChangeType::NewField,
            ChangeType::ActionNowRequires,
            ChangeType::DestinationChanged,
            ChangeType::RemovedStage,
        ]
    );
    assert_eq!(lista, diff(&a, &b), "duas chamadas dão a mesma lista");
}
