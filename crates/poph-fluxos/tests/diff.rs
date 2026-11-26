//! Testes da diferença entre revisões em linguagem comum (FX-17).
//!
//! Cada tipo de mudança tem um teste com o texto exato (`assert_eq!`, não `contains`) e o impacto.
//! Os outros testes cobrem fluxos idênticos (lista vazia), ordem estável, o impacto que não muda
//! nada e a correspondência entre os códigos e o JSON da API. A base é a IR dourada de
//! `agendar_visita` (a mesma do bloco do prompt, FX-18); cada caso altera só o que precisa.

use poph_fluxos::ir::{
    Acao, Campo, Decisao, Def, Destino, Efeito, Etapa, Fim, Fluxo, Gatilho, Ir, Regra,
};
use poph_fluxos::{Impacto, Mudanca, TipoMudanca, diferenca};
use poph_types::{ActionActor, FieldKind};

// ------------------------------------------------------------------ auxiliares

fn base() -> Ir {
    let caminho = format!(
        "{}/../../tests/fluxos/ir/agendar_visita.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let texto =
        std::fs::read_to_string(&caminho).unwrap_or_else(|erro| panic!("{caminho}: {erro}"));
    Ir::de_json(&texto).expect("a IR dourada é JSON válido")
}

/// Cópia da IR com a alteração aplicada.
fn com(ir: &Ir, alterar: impl FnOnce(&mut Ir)) -> Ir {
    let mut copia = ir.clone();
    alterar(&mut copia);
    copia
}

fn em_etapa<'a>(ir: &'a mut Ir, id: &str) -> &'a mut Etapa {
    ir.etapas
        .iter_mut()
        .find(|etapa| etapa.id == id)
        .expect("a etapa existe na base")
}

fn em_acao<'a>(ir: &'a mut Ir, etapa: &str, acao: &str) -> &'a mut Acao {
    em_etapa(ir, etapa)
        .acoes
        .iter_mut()
        .find(|candidata| candidata.id == acao)
        .expect("a ação existe na base")
}

fn campo(id: &str, rotulo: &str, tipo: FieldKind, obrigatorio: bool, opcoes: &[&str]) -> Campo {
    Campo {
        id: id.to_owned(),
        rotulo: rotulo.to_owned(),
        tipo,
        obrigatorio,
        opcoes: opcoes.iter().map(|opcao| (*opcao).to_owned()).collect(),
        ajuda: None,
    }
}

fn acao(
    id: &str,
    rotulo: &str,
    exige: &[&str],
    ator: Option<ActionActor>,
    vai_para: Destino,
) -> Acao {
    Acao {
        id: id.to_owned(),
        rotulo: rotulo.to_owned(),
        exige: exige.iter().map(|campo| (*campo).to_owned()).collect(),
        ator,
        vai_para,
        efeitos: Vec::new(),
    }
}

fn etapa(id: &str, nome: &str, instrucao: Option<&str>, acoes: Vec<Acao>) -> Etapa {
    Etapa {
        id: id.to_owned(),
        nome: nome.to_owned(),
        instrucao: instrucao.map(str::to_owned),
        atores_acao: vec![ActionActor::Agent],
        campos: Vec::new(),
        acoes,
        ao_entrar: Vec::new(),
    }
}

fn no_etapa(id: &str) -> Destino {
    Destino::Etapa { id: id.to_owned() }
}

fn no_fim(id: &str, resultado: Option<&str>, encerra_atendimento: bool) -> Destino {
    Destino::Fim {
        id: id.to_owned(),
        resultado: resultado.map(str::to_owned),
        encerra_atendimento,
    }
}

fn fim(id: &str, resultado: Option<&str>, encerra_atendimento: bool) -> Fim {
    Fim {
        id: id.to_owned(),
        resultado: resultado.map(str::to_owned),
        encerra_atendimento,
    }
}

fn decisao(id: &str, se: Regra, entao: Destino, senao: Destino) -> Decisao {
    Decisao {
        id: id.to_owned(),
        se,
        entao,
        senao,
    }
}

fn igual(campo: &str, valor: &str) -> Regra {
    Regra::Igual {
        campo: campo.to_owned(),
        valor: valor.to_owned(),
    }
}

/// Acha a única mudança do tipo; falha com a lista inteira se houver outra ou nenhuma.
fn unica(lista: &[Mudanca], tipo: TipoMudanca) -> &Mudanca {
    let achadas: Vec<&Mudanca> = lista
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

fn tipos(lista: &[Mudanca]) -> Vec<TipoMudanca> {
    lista.iter().map(|mudanca| mudanca.tipo).collect()
}

const TODOS_OS_TIPOS: [TipoMudanca; 27] = [
    TipoMudanca::EtapaNova,
    TipoMudanca::EtapaRemovida,
    TipoMudanca::InstrucaoMudou,
    TipoMudanca::AtoresDaEtapaMudou,
    TipoMudanca::CampoNovo,
    TipoMudanca::CampoRemovido,
    TipoMudanca::CampoAgoraObrigatorio,
    TipoMudanca::CampoDeixouDeSerObrigatorio,
    TipoMudanca::CampoMudouTipo,
    TipoMudanca::CampoMudouOpcoes,
    TipoMudanca::AcaoPassouAExigir,
    TipoMudanca::AcaoDeixouDeExigir,
    TipoMudanca::AcaoNova,
    TipoMudanca::AcaoRemovida,
    TipoMudanca::DestinoMudou,
    TipoMudanca::AtorDaAcaoMudou,
    TipoMudanca::RegraMudou,
    TipoMudanca::DecisaoNova,
    TipoMudanca::DecisaoRemovida,
    TipoMudanca::FimNovo,
    TipoMudanca::FimRemovido,
    TipoMudanca::FimMudou,
    TipoMudanca::EfeitoNovo,
    TipoMudanca::EfeitoRemovido,
    TipoMudanca::GatilhoMudou,
    TipoMudanca::InicioMudou,
    TipoMudanca::TextoMudou,
];

// ------------------------------------------------------------------ identidade

#[test]
fn fluxos_identicos_devolvem_lista_vazia() {
    let ir = base();
    assert!(diferenca(&ir, &ir).is_empty());
}

#[test]
fn a_ida_e_volta_pelo_json_nao_gera_mudanca() {
    let ir = base();
    let volta = Ir::de_json(&ir.para_json()).expect("a IR gravada volta a ser lida");
    assert!(diferenca(&ir, &volta).is_empty());
}

#[test]
fn a_mesma_entrada_da_sempre_a_mesma_lista_na_mesma_ordem() {
    let a = base();
    let b = com(&a, |ir| {
        em_etapa(ir, "coleta").nome = "Colher dados".to_owned();
        em_acao(ir, "coleta", "confirmar").vai_para =
            no_fim("concluida", Some("Visita agendada"), false);
        ir.gatilho.quando = "Quando o cliente pedir remarcação.".to_owned();
    });
    let primeira = diferenca(&a, &b);
    let segunda = diferenca(&a, &b);
    assert_eq!(primeira, segunda);
    assert_eq!(
        tipos(&primeira),
        vec![
            TipoMudanca::GatilhoMudou,
            TipoMudanca::TextoMudou,
            TipoMudanca::DestinoMudou,
        ],
        "o fluxo vem primeiro, depois as etapas"
    );
}

#[test]
fn a_ordem_dos_itens_no_script_nao_gera_mudanca() {
    let a = base();
    let b = com(&a, |ir| ir.etapas.reverse());
    assert!(diferenca(&a, &b).is_empty());
}

#[test]
fn codigos_de_tipo_e_impacto_batem_com_o_json_da_api() {
    for tipo in TODOS_OS_TIPOS {
        let json = serde_json::to_string(&tipo).expect("o tipo serializa");
        assert_eq!(json, format!("\"{}\"", tipo.codigo()), "tipo {tipo:?}");
    }
    for impacto in [
        Impacto::Nenhum,
        Impacto::AtendimentosNovos,
        Impacto::PedeDadoNovo,
        Impacto::MudaRota,
        Impacto::DisparaEfeito,
    ] {
        let json = serde_json::to_string(&impacto).expect("o impacto serializa");
        assert_eq!(
            json,
            format!("\"{}\"", impacto.codigo()),
            "impacto {impacto:?}"
        );
    }
}

// ------------------------------------------------------------------ etapas

#[test]
fn etapa_nova_sem_chegada_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| {
        ir.etapas
            .push(etapa("revisao", "Revisar dados", None, Vec::new()));
    });
    let lista = diferenca(&a, &b);
    assert_eq!(lista.len(), 1, "{lista:#?}");
    let nova = unica(&lista, TipoMudanca::EtapaNova);
    assert_eq!(nova.impacto, Impacto::Nenhum);
    assert_eq!(nova.alvo, "etapa:revisao");
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
        ir.etapas
            .push(etapa("revisao", "Revisar dados", None, Vec::new()));
        em_acao(ir, "coleta", "confirmar").vai_para = no_etapa("revisao");
    });
    let lista = diferenca(&a, &b);
    let nova = unica(&lista, TipoMudanca::EtapaNova);
    assert_eq!(nova.impacto, Impacto::MudaRota);
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
        ir.etapas.retain(|etapa| etapa.id != "confirmacao");
        em_acao(ir, "coleta", "confirmar").vai_para =
            no_fim("concluida", Some("Visita agendada"), false);
    });
    let lista = diferenca(&a, &b);
    let removida = unica(&lista, TipoMudanca::EtapaRemovida);
    assert_eq!(removida.impacto, Impacto::MudaRota);
    assert_eq!(removida.alvo, "etapa:confirmacao");
    assert_eq!(
        removida.texto,
        "a etapa `confirmacao` (Confirmar visita) foi removida: os atendimentos novos não passam \
         mais por ela."
    );
}

#[test]
fn etapa_removida_sem_chegada_nao_muda_o_atendimento() {
    let a = com(&base(), |ir| {
        ir.etapas.push(etapa("sobra", "Sobra", None, Vec::new()));
    });
    let b = base();
    let lista = diferenca(&a, &b);
    assert_eq!(lista.len(), 1, "{lista:#?}");
    let removida = unica(&lista, TipoMudanca::EtapaRemovida);
    assert_eq!(removida.impacto, Impacto::Nenhum);
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
        em_etapa(ir, "coleta").instrucao = Some("Pergunte só o endereço.".to_owned());
    });
    let lista = diferenca(&a, &b);
    let mudou = unica(&lista, TipoMudanca::InstrucaoMudou);
    assert_eq!(mudou.impacto, Impacto::AtendimentosNovos);
    assert_eq!(mudou.alvo, "etapa:coleta");
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
        em_etapa(ir, "coleta").atores_acao = vec![ActionActor::Agent, ActionActor::Attendant];
    });
    let lista = diferenca(&a, &b);
    let mudou = unica(&lista, TipoMudanca::AtoresDaEtapaMudou);
    assert_eq!(mudou.impacto, Impacto::MudaRota);
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
        em_etapa(ir, "coleta").campos[0].rotulo = "Endereço".to_owned();
    });
    let lista = diferenca(&a, &b);
    assert_eq!(lista.len(), 2, "{lista:#?}");
    assert!(
        lista.iter().all(|m| m.impacto == Impacto::Nenhum),
        "{lista:#?}"
    );
    let nome = lista
        .iter()
        .find(|m| m.alvo == "etapa:coleta")
        .expect("o nome da etapa aparece");
    assert_eq!(nome.tipo, TipoMudanca::TextoMudou);
    assert_eq!(nome.alvo, "etapa:coleta");
    assert_eq!(
        nome.texto,
        "o nome da etapa `coleta` mudou de \"Coletar dados\" para \"Colher dados\"."
    );
    let rotulo = lista
        .iter()
        .find(|m| m.alvo == "etapa:coleta/campo:endereco")
        .expect("o rótulo do campo aparece");
    assert_eq!(rotulo.tipo, TipoMudanca::TextoMudou);
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
            .push(campo("cep", "CEP", FieldKind::Texto, false, &[]));
    });
    let lista = diferenca(&a, &b);
    let novo = unica(&lista, TipoMudanca::CampoNovo);
    assert_eq!(novo.impacto, Impacto::PedeDadoNovo);
    assert_eq!(novo.alvo, "etapa:coleta/campo:cep");
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
        em_acao(ir, "coleta", "confirmar").exige = vec!["endereco".to_owned()];
    });
    let lista = diferenca(&a, &b);
    let removido = unica(&lista, TipoMudanca::CampoRemovido);
    assert_eq!(removido.impacto, Impacto::AtendimentosNovos);
    assert_eq!(removido.alvo, "etapa:coleta/campo:periodo");
    assert_eq!(
        removido.texto,
        "a etapa `coleta` deixa de coletar o campo `periodo` (Período): nos atendimentos novos, o \
         agente não pedirá esse dado."
    );
}

#[test]
fn campo_que_fica_obrigatorio_pede_dado_novo() {
    let a = com(&base(), |ir| {
        em_etapa(ir, "coleta").campos[0].obrigatorio = false;
    });
    let b = base();
    let lista = diferenca(&a, &b);
    let agora = unica(&lista, TipoMudanca::CampoAgoraObrigatorio);
    assert_eq!(agora.impacto, Impacto::PedeDadoNovo);
    assert_eq!(agora.alvo, "etapa:coleta/campo:endereco");
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
        em_etapa(ir, "coleta").campos[0].obrigatorio = false;
    });
    let lista = diferenca(&a, &b);
    let deixou = unica(&lista, TipoMudanca::CampoDeixouDeSerObrigatorio);
    assert_eq!(deixou.impacto, Impacto::AtendimentosNovos);
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
        em_etapa(ir, "coleta").campos[1].tipo = FieldKind::Texto;
    });
    let lista = diferenca(&a, &b);
    let mudou = unica(&lista, TipoMudanca::CampoMudouTipo);
    assert_eq!(mudou.impacto, Impacto::PedeDadoNovo);
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
            .opcoes
            .push("noite".to_owned());
    });
    let lista = diferenca(&a, &b);
    let mudou = unica(&lista, TipoMudanca::CampoMudouOpcoes);
    assert_eq!(mudou.impacto, Impacto::PedeDadoNovo);
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
        em_etapa(ir, "coleta").acoes.push(acao(
            "remarcar",
            "Cliente quer remarcar",
            &[],
            None,
            no_etapa("confirmacao"),
        ));
    });
    let lista = diferenca(&a, &b);
    let nova = unica(&lista, TipoMudanca::AcaoNova);
    assert_eq!(nova.impacto, Impacto::MudaRota);
    assert_eq!(nova.alvo, "etapa:coleta/acao:remarcar");
    assert_eq!(
        nova.texto,
        "a etapa `coleta` ganhou a ação `remarcar` (Cliente quer remarcar), que leva para a etapa \
         `confirmacao` (Confirmar visita)."
    );
}

#[test]
fn acao_removida_muda_a_rota() {
    let a = com(&base(), |ir| {
        em_etapa(ir, "coleta").acoes.push(acao(
            "remarcar",
            "Cliente quer remarcar",
            &[],
            None,
            no_etapa("confirmacao"),
        ));
    });
    let b = base();
    let lista = diferenca(&a, &b);
    let removida = unica(&lista, TipoMudanca::AcaoRemovida);
    assert_eq!(removida.impacto, Impacto::MudaRota);
    assert_eq!(removida.alvo, "etapa:coleta/acao:remarcar");
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
        em_acao(ir, "coleta", "confirmar").vai_para =
            no_fim("concluida", Some("Visita agendada"), false);
    });
    let lista = diferenca(&a, &b);
    let destino = unica(&lista, TipoMudanca::DestinoMudou);
    assert_eq!(destino.impacto, Impacto::MudaRota);
    assert_eq!(destino.alvo, "etapa:coleta/acao:confirmar");
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
            .push(campo("cep", "CEP", FieldKind::Texto, false, &[]));
        em_acao(ir, "coleta", "confirmar")
            .exige
            .push("cep".to_owned());
    });
    let lista = diferenca(&a, &b);
    assert_eq!(
        tipos(&lista),
        vec![TipoMudanca::CampoNovo, TipoMudanca::AcaoPassouAExigir]
    );
    let exige = unica(&lista, TipoMudanca::AcaoPassouAExigir);
    assert_eq!(exige.impacto, Impacto::PedeDadoNovo);
    assert_eq!(exige.alvo, "etapa:coleta/acao:confirmar");
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
            .exige
            .retain(|campo| campo != "endereco");
    });
    let lista = diferenca(&a, &b);
    let deixou = unica(&lista, TipoMudanca::AcaoDeixouDeExigir);
    assert_eq!(deixou.impacto, Impacto::AtendimentosNovos);
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
        em_acao(ir, "coleta", "confirmar").ator = Some(ActionActor::Attendant);
    });
    let lista = diferenca(&a, &b);
    let ator = unica(&lista, TipoMudanca::AtorDaAcaoMudou);
    assert_eq!(ator.impacto, Impacto::MudaRota);
    assert_eq!(
        ator.texto,
        "a ação `confirmar` da etapa `coleta` passa a ser feita só pelo atendente."
    );
}

// ------------------------------------------------------------------ decisões, fins e defs

#[test]
fn regra_de_decisao_mudou_muda_a_rota() {
    let com_decisao = |se: Regra| {
        com(&base(), |ir| {
            ir.decisoes.push(decisao(
                "classificar",
                se,
                no_etapa("confirmacao"),
                no_fim("desistencia", Some("Cliente desistiu"), false),
            ));
        })
    };
    let a = com_decisao(igual("coleta.periodo", "manhã"));
    let b = com_decisao(Regra::Diferente {
        campo: "coleta.periodo".to_owned(),
        valor: "manhã".to_owned(),
    });
    let lista = diferenca(&a, &b);
    let regra = unica(&lista, TipoMudanca::RegraMudou);
    assert_eq!(regra.impacto, Impacto::MudaRota);
    assert_eq!(regra.alvo, "decisao:classificar");
    assert_eq!(
        regra.texto,
        "a condição da decisão `classificar` mudou: agora é verdadeira quando `coleta.periodo` é \
         diferente de \"manhã\"."
    );
}

#[test]
fn ramo_de_decisao_que_muda_de_destino_muda_a_rota() {
    let com_decisao = |senao: Destino| {
        com(&base(), |ir| {
            ir.decisoes.push(decisao(
                "classificar",
                igual("coleta.periodo", "manhã"),
                no_etapa("confirmacao"),
                senao,
            ));
        })
    };
    let a = com_decisao(no_fim("desistencia", Some("Cliente desistiu"), false));
    let b = com_decisao(no_fim("concluida", Some("Visita agendada"), false));
    let lista = diferenca(&a, &b);
    let destino = unica(&lista, TipoMudanca::DestinoMudou);
    assert_eq!(destino.impacto, Impacto::MudaRota);
    assert_eq!(destino.alvo, "decisao:classificar/ramo:senao");
    assert_eq!(
        destino.texto,
        "o ramo falso da decisão `classificar` agora leva para o fim `concluida`, e antes levava \
         para o fim `desistencia`."
    );
}

#[test]
fn ramo_verdadeiro_de_decisao_que_muda_de_destino_muda_a_rota() {
    let com_decisao = |entao: Destino| {
        com(&base(), |ir| {
            ir.decisoes.push(decisao(
                "classificar",
                igual("coleta.periodo", "manhã"),
                entao,
                no_fim("desistencia", Some("Cliente desistiu"), false),
            ));
        })
    };
    let a = com_decisao(no_etapa("confirmacao"));
    let b = com_decisao(no_fim("concluida", Some("Visita agendada"), false));
    let lista = diferenca(&a, &b);
    let destino = unica(&lista, TipoMudanca::DestinoMudou);
    assert_eq!(destino.impacto, Impacto::MudaRota);
    assert_eq!(destino.alvo, "decisao:classificar/ramo:entao");
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
        ir.decisoes.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            no_etapa("confirmacao"),
            no_fim("desistencia", Some("Cliente desistiu"), false),
        ));
    });
    let lista = diferenca(&a, &b);
    let nova = unica(&lista, TipoMudanca::DecisaoNova);
    assert_eq!(nova.impacto, Impacto::Nenhum);
    assert_eq!(nova.alvo, "decisao:classificar");
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
        ir.decisoes.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            no_etapa("confirmacao"),
            no_fim("desistencia", Some("Cliente desistiu"), false),
        ));
        em_acao(ir, "coleta", "confirmar").vai_para = Destino::Decisao {
            id: "classificar".to_owned(),
        };
    });
    let lista = diferenca(&a, &b);
    let nova = unica(&lista, TipoMudanca::DecisaoNova);
    assert_eq!(nova.impacto, Impacto::MudaRota);
    assert_eq!(
        nova.texto,
        "a decisão `classificar` é nova e já é destino de alguma ação ou decisão: os atendimentos \
         novos passam por ela."
    );
}

#[test]
fn decisao_removida_sem_chegada_nao_muda_o_atendimento() {
    let a = com(&base(), |ir| {
        ir.decisoes.push(decisao(
            "classificar",
            igual("coleta.periodo", "manhã"),
            no_etapa("confirmacao"),
            no_fim("desistencia", Some("Cliente desistiu"), false),
        ));
    });
    let b = base();
    let lista = diferenca(&a, &b);
    let removida = unica(&lista, TipoMudanca::DecisaoRemovida);
    assert_eq!(removida.impacto, Impacto::Nenhum);
    assert_eq!(
        removida.texto,
        "a decisão `classificar` foi removida, mas nenhuma ação ou decisão a usava: nada muda no \
         atendimento."
    );
}

#[test]
fn fim_novo_sem_chegada_nao_muda_o_atendimento() {
    let a = base();
    let b = com(&a, |ir| ir.fins.push(fim("nova_saida", None, false)));
    let lista = diferenca(&a, &b);
    let novo = unica(&lista, TipoMudanca::FimNovo);
    assert_eq!(novo.impacto, Impacto::Nenhum);
    assert_eq!(novo.alvo, "fim:nova_saida");
    assert_eq!(
        novo.texto,
        "o fim `nova_saida` é novo, mas nenhuma ação ou decisão leva a ele: nada muda no \
         atendimento."
    );
}

#[test]
fn fim_removido_sem_chegada_nao_muda_o_atendimento() {
    let a = com(&base(), |ir| ir.fins.push(fim("extra", None, false)));
    let b = base();
    let lista = diferenca(&a, &b);
    let removido = unica(&lista, TipoMudanca::FimRemovido);
    assert_eq!(removido.impacto, Impacto::Nenhum);
    assert_eq!(
        removido.texto,
        "o fim `extra` foi removido, mas nenhuma ação ou decisão o usava: nada muda no atendimento."
    );
}

#[test]
fn fim_que_passa_a_encerrar_o_atendimento_muda_a_rota() {
    let a = base();
    let b = com(&a, |ir| {
        ir.fins
            .iter_mut()
            .find(|fim| fim.id == "desistencia")
            .expect("o fim existe")
            .encerra_atendimento = true;
    });
    let lista = diferenca(&a, &b);
    let mudou = unica(&lista, TipoMudanca::FimMudou);
    assert_eq!(mudou.impacto, Impacto::MudaRota);
    assert_eq!(mudou.alvo, "fim:desistencia");
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
        ir.fins
            .iter_mut()
            .find(|fim| fim.id == "concluida")
            .expect("o fim existe")
            .resultado = Some("Visita confirmada".to_owned());
    });
    let lista = diferenca(&a, &b);
    let mudou = unica(&lista, TipoMudanca::FimMudou);
    assert_eq!(mudou.impacto, Impacto::Nenhum);
    assert_eq!(
        mudou.texto,
        "o resumo do fim `concluida` mudou de \"Visita agendada\" para \"Visita confirmada\"."
    );
}

#[test]
fn regra_especial_usada_por_decisao_muda_a_rota_quando_o_texto_muda() {
    let com_def = |fonte: &str| {
        com(&base(), |ir| {
            ir.decisoes.push(decisao(
                "classificar",
                Regra::Funcao {
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
    let lista = diferenca(&a, &b);
    let regra = unica(&lista, TipoMudanca::RegraMudou);
    assert_eq!(regra.impacto, Impacto::MudaRota);
    assert_eq!(regra.alvo, "def:afinidade");
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
    let lista = diferenca(&a, &b);
    let nova = unica(&lista, TipoMudanca::RegraMudou);
    assert_eq!(nova.impacto, Impacto::Nenhum);
    assert_eq!(nova.alvo, "def:sem_uso");
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
            .efeitos
            .push(Efeito::MensagemAoCliente {
                texto: "Visita marcada!".to_owned(),
            });
    });
    let lista = diferenca(&a, &b);
    let novo = unica(&lista, TipoMudanca::EfeitoNovo);
    assert_eq!(novo.impacto, Impacto::DisparaEfeito);
    assert_eq!(novo.alvo, "etapa:coleta/acao:confirmar");
    assert_eq!(
        novo.texto,
        "a ação `confirmar` da etapa `coleta` passa a enviar ao cliente: \"Visita marcada!\"."
    );
}

#[test]
fn efeito_removido_de_uma_acao_dispara_efeito() {
    let a = com(&base(), |ir| {
        em_acao(ir, "coleta", "confirmar")
            .efeitos
            .push(Efeito::MensagemAoCliente {
                texto: "Visita marcada!".to_owned(),
            });
    });
    let b = base();
    let lista = diferenca(&a, &b);
    let removido = unica(&lista, TipoMudanca::EfeitoRemovido);
    assert_eq!(removido.impacto, Impacto::DisparaEfeito);
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
            .ao_entrar
            .push(Efeito::NotificarWhatsapp {
                para: "+55 11 90000-0000".to_owned(),
                mensagem: "Nova visita".to_owned(),
            });
        em_etapa(ir, "confirmacao")
            .ao_entrar
            .push(Efeito::Encaminhar {
                departamento: "Comercial".to_owned(),
            });
    });
    let lista = diferenca(&a, &b);
    assert_eq!(
        tipos(&lista),
        vec![TipoMudanca::EfeitoNovo, TipoMudanca::EfeitoNovo]
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
    assert!(lista.iter().all(|m| m.impacto == Impacto::DisparaEfeito));
}

#[test]
fn efeito_de_etapa_nova_sem_chegada_nao_dispara() {
    let a = base();
    let b = com(&a, |ir| {
        let mut sobra = etapa("sobra", "Sobra", None, Vec::new());
        sobra.ao_entrar.push(Efeito::MensagemAoCliente {
            texto: "Oi".to_owned(),
        });
        ir.etapas.push(sobra);
    });
    let lista = diferenca(&a, &b);
    assert_eq!(tipos(&lista), vec![TipoMudanca::EtapaNova], "{lista:#?}");
    assert_eq!(lista[0].impacto, Impacto::Nenhum);
}

#[test]
fn efeito_que_ja_existia_nao_aparece_de_novo() {
    let com_efeito_1 = |ir: &mut Ir| {
        em_acao(ir, "coleta", "confirmar")
            .efeitos
            .push(Efeito::MensagemAoCliente {
                texto: "Visita marcada!".to_owned(),
            });
    };
    let a = com(&base(), com_efeito_1);
    let b = com(&a, |ir| {
        em_acao(ir, "coleta", "confirmar")
            .efeitos
            .push(Efeito::Encaminhar {
                departamento: "Comercial".to_owned(),
            });
    });
    let lista = diferenca(&a, &b);
    assert_eq!(tipos(&lista), vec![TipoMudanca::EfeitoNovo], "{lista:#?}");
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
            quando: "Quando o cliente pedir remarcação.".to_owned(),
            nao_quando: None,
            exemplos: Vec::new(),
        };
    });
    let lista = diferenca(&a, &b);
    let gatilho = unica(&lista, TipoMudanca::GatilhoMudou);
    assert_eq!(gatilho.impacto, Impacto::AtendimentosNovos);
    assert_eq!(gatilho.alvo, "gatilho");
    assert_eq!(
        gatilho.texto,
        "o critério para iniciar este fluxo mudou: nos atendimentos novos, o Jev passa a usar o \
         novo critério."
    );
}

#[test]
fn etapa_inicial_mudou_vale_para_atendimentos_novos() {
    let a = base();
    let b = com(&a, |ir| ir.inicio = "confirmacao".to_owned());
    let lista = diferenca(&a, &b);
    let inicio = unica(&lista, TipoMudanca::InicioMudou);
    assert_eq!(inicio.impacto, Impacto::AtendimentosNovos);
    assert_eq!(inicio.alvo, "fluxo");
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
    let lista = diferenca(&a, &b);
    let nome = unica(&lista, TipoMudanca::TextoMudou);
    assert_eq!(nome.impacto, Impacto::Nenhum);
    assert_eq!(nome.alvo, "fluxo");
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
        ir.inicio = "confirmacao".to_owned();
        em_etapa(ir, "coleta").campos[0].obrigatorio = false;
        em_etapa(ir, "coleta")
            .campos
            .push(campo("cep", "CEP", FieldKind::Texto, false, &[]));
        em_acao(ir, "coleta", "confirmar")
            .exige
            .push("cep".to_owned());
        em_acao(ir, "coleta", "confirmar").vai_para =
            no_fim("concluida", Some("Visita agendada"), false);
        ir.etapas.retain(|etapa| etapa.id != "confirmacao");
    });
    let lista = diferenca(&a, &b);
    assert_eq!(
        tipos(&lista),
        vec![
            TipoMudanca::InicioMudou,
            TipoMudanca::CampoDeixouDeSerObrigatorio,
            TipoMudanca::CampoNovo,
            TipoMudanca::AcaoPassouAExigir,
            TipoMudanca::DestinoMudou,
            TipoMudanca::EtapaRemovida,
        ]
    );
    assert_eq!(lista, diferenca(&a, &b), "duas chamadas dão a mesma lista");
}
