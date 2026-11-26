//! Contrato do G9 (FX-22): qual fluxo liberado o cliente quer iniciar.
//!
//! O que se prova aqui, sem rede e sem Jev real:
//! 1. a pergunta só existe com pelo menos um fluxo liberado no estado e no máximo
//!    [`MAX_FLUXOS_LIBERADOS`] — acima do teto o gate é inaplicável **com aviso**;
//! 2. as opções são os ids dos fluxos + `nenhum`, e o critério de cada uma é montado do
//!    `quando`/`nao_quando`/`exemplos` do gatilho, cada um cortado em 400 caracteres;
//! 3. escolha confiável vira decisão; `nenhum`, confiança baixa, margem curta e opção inventada
//!    caem em fallback **com motivo** e nada inicia;
//! 4. o motor manda a pergunta no **lote único** com os outros gates (uma requisição) e a flag por
//!    agente desliga o gate;
//! 5. o conjunto rotulado `tests/jev/g9.jsonl` existe, é coerente com o estado dos casos e serve
//!    para a medição com o Jev real (FX-37).

use std::collections::BTreeMap;
use std::sync::Arc;

use poph_jev::{
    AvaliadorGates, DecisionOrigin, G1_CONTINUE, G3_BASE, G9, G9_NONE, Gate, GateOverrides, IdGate,
    JevFalso, JevFlow, JevMessage, JevResponse, JevState, MAX_RELEASED_FLOWS, MAX_TRIGGER_CHARS,
    MotorJev, Thresholds, flow_policy, ler_conjunto,
};
use poph_types::JevGates;

fn fluxo(id: &str, nome: &str) -> JevFlow {
    JevFlow {
        id: id.to_owned(),
        nome: nome.to_owned(),
        when: format!("O cliente quer {nome}."),
        not_when: "Só está perguntando sobre outra coisa.".to_owned(),
        examples: vec![format!("queria {nome}"), format!("dá para {nome}?")],
    }
}

fn fluxos() -> Vec<JevFlow> {
    vec![
        fluxo("agendar_visita", "marcar uma visita técnica"),
        fluxo("segunda_via_fatura", "receber a segunda via da fatura"),
        fluxo("abrir_chamado", "abrir um chamado de suporte"),
    ]
}

fn estado() -> JevState {
    JevState::new("whatsapp")
        .with_messages([
            JevMessage::from_agent("Boa tarde! Como posso ajudar?"),
            JevMessage::from_customer("queria marcar uma visita técnica na minha casa"),
        ])
        .with_released_flows(fluxos())
}

fn resposta(escolha: &str, confianca: f64) -> JevResponse {
    let mut probabilidades = BTreeMap::new();
    probabilidades.insert(escolha.to_owned(), 0.9);
    JevResponse {
        choice: Some(escolha.to_owned()),
        probabilities: probabilidades,
        confidence: confianca,
        ..JevResponse::default()
    }
}

fn gates(g1: bool, g3: bool, g9: bool) -> JevGates {
    JevGates {
        g1,
        g3,
        g9,
        ..JevGates::default()
    }
}

#[test]
fn pergunta_tem_os_ids_dos_fluxos_e_nenhum_com_criterio_do_gatilho() {
    let gate = G9;
    assert_eq!(gate.id(), IdGate::G9);
    assert_eq!(gate.id().question_id(), "g9_iniciar_fluxo");

    let pergunta = gate.question(&estado());
    let opcoes: Vec<&str> = pergunta.rubric.keys().map(String::as_str).collect();
    assert_eq!(
        opcoes,
        vec![
            "abrir_chamado",
            "agendar_visita",
            "nenhum",
            "segunda_via_fatura"
        ],
        "os ids dos fluxos + nenhum, em ordem estável"
    );

    let criterio = &pergunta.rubric["agendar_visita"];
    assert!(criterio.contains("Quando usar:"), "{criterio}");
    assert!(criterio.contains("Não usar quando:"), "{criterio}");
    assert!(
        criterio.contains("Exemplos de fala do cliente:"),
        "{criterio}"
    );
    assert!(
        criterio.contains("\"queria marcar uma visita técnica\""),
        "{criterio}"
    );
    assert!(
        pergunta.instructions.contains("ÚLTIMA mensagem do cliente"),
        "a instrução diz o que julgar"
    );
}

#[test]
fn cada_pedaco_do_gatilho_e_cortado_em_400_caracteres() {
    let longo = "a".repeat(1000);
    let fluxo_longo = JevFlow {
        id: "agendar_visita".to_owned(),
        nome: "Agendar visita".to_owned(),
        when: longo.clone(),
        not_when: longo.clone(),
        examples: vec![longo.clone()],
    };
    let estado = JevState::new("cli")
        .with_messages([JevMessage::from_customer("queria agendar")])
        .with_released_flows([fluxo_longo]);

    let pergunta = G9.question(&estado);
    let criterio = &pergunta.rubric["agendar_visita"];
    // Três pedaços: o corte de 400 é do estado, e o critério compõe o que veio cortado.
    for pedaco in criterio.split('\n') {
        let valor = pedaco
            .split_once(": ")
            .map(|(_, valor)| valor)
            .unwrap_or("");
        let valor = valor.trim_matches('"');
        assert!(
            valor.chars().count() <= MAX_TRIGGER_CHARS + 1,
            "pedaço sem corte: {} caracteres",
            valor.chars().count()
        );
    }
    assert_eq!(criterio.matches('…').count(), 3, "{criterio}");

    // O estado também guarda cortado: nada de 1000 caracteres no pedido ao Jev.
    let guardado = &estado.released_flows[0];
    assert_eq!(guardado.when.chars().count(), MAX_TRIGGER_CHARS + 1);
    assert_eq!(guardado.examples[0].chars().count(), MAX_TRIGGER_CHARS + 1);
}

#[test]
fn criterio_cai_no_nome_quando_o_gatilho_esta_vazio() {
    let sem_gatilho = JevFlow {
        id: "abrir_chamado".to_owned(),
        nome: "Abrir chamado".to_owned(),
        when: String::new(),
        not_when: "  ".to_owned(),
        examples: Vec::new(),
    };
    let criterio = flow_policy(&sem_gatilho);
    assert_eq!(
        criterio,
        "O cliente quer o que o fluxo \"Abrir chamado\" faz."
    );
}

#[test]
fn aplicavel_so_com_fluxo_liberado_e_dentro_do_teto() {
    let gate = G9;

    assert!(gate.applies(&estado()), "três fluxos liberados: pergunta");
    assert_eq!(gate.warning(&estado()), None, "nada a avisar");

    let sem_fluxo = JevState::new("whatsapp")
        .with_messages([JevMessage::from_customer("queria marcar uma visita")]);
    assert!(
        !gate.applies(&sem_fluxo),
        "sem fluxo liberado a pergunta não existe"
    );
    assert_eq!(gate.warning(&sem_fluxo), None, "ausência de fluxo é normal");

    let sem_mensagem_do_cliente = JevState::new("whatsapp")
        .with_messages([JevMessage::from_agent("Olá!")])
        .with_released_flows(fluxos());
    assert!(
        !gate.applies(&sem_mensagem_do_cliente),
        "sem fala do cliente não há o que escolher"
    );

    // Exatamente no teto: pergunta, com os 20 ids + nenhum.
    let no_teto: Vec<JevFlow> = (0..MAX_RELEASED_FLOWS)
        .map(|indice| fluxo(&format!("fluxo_{indice:02}"), "atender o cliente"))
        .collect();
    let estado_no_teto = JevState::new("cli")
        .with_messages([JevMessage::from_customer("quero atendimento")])
        .with_released_flows(no_teto);
    assert!(gate.applies(&estado_no_teto));
    assert_eq!(
        gate.question(&estado_no_teto).rubric.len(),
        MAX_RELEASED_FLOWS + 1,
        "vinte fluxos + nenhum"
    );
}

#[tokio::test]
async fn mais_de_vinte_fluxos_deixa_o_gate_inaplicavel_com_aviso() {
    let gate = G9;
    let demais: Vec<JevFlow> = (0..MAX_RELEASED_FLOWS + 1)
        .map(|indice| fluxo(&format!("fluxo_{indice:02}"), "atender o cliente"))
        .collect();
    let estado = JevState::new("cli")
        .with_messages([JevMessage::from_customer("quero atendimento")])
        .with_released_flows(demais);

    assert!(
        !gate.applies(&estado),
        "lista cortada faria o modelo escolher entre as que sobraram"
    );
    let aviso = gate.warning(&estado).expect("avisa o operador");
    assert!(aviso.contains("21"), "{aviso}");
    assert!(aviso.contains(&MAX_RELEASED_FLOWS.to_string()), "{aviso}");

    // E o motor não manda pergunta nenhuma com só o G9 ligado.
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::new()));
    let motor = MotorJev::novo(jev.clone(), gates(false, false, true), "jev-latest");
    let resultado = motor.avaliar(estado, None).await;
    assert_eq!(jev.chamadas(), 0, "acima do teto: nenhuma chamada");
    assert!(resultado.decisao(IdGate::G9).is_none());
}

#[test]
fn escolha_confiavel_vira_decisao() {
    let gate = G9;
    let limiares = gate.thresholds();
    assert_eq!(
        limiares,
        Thresholds::new(0.7, 0.1),
        "confiança 0,7 e margem 0,1 no padrão do PRD"
    );

    let decisao = gate.interpret(&estado(), Some(&resposta("agendar_visita", 0.92)), limiares);
    assert_eq!(decisao.origin, DecisionOrigin::Jev);
    assert_eq!(decisao.choice.as_deref(), Some("agendar_visita"));
    assert!(decisao.is_reliable());
    assert_eq!(decisao.motivo, None);
}

#[test]
fn nenhum_baixa_confianca_margem_curta_e_opcao_inventada_viram_fallback_com_motivo() {
    let gate = G9;
    let limiares = gate.thresholds();

    let nenhum = gate.interpret(&estado(), Some(&resposta(G9_NONE, 0.95)), limiares);
    assert_eq!(nenhum.origin, DecisionOrigin::Fallback);
    assert_eq!(nenhum.choice, None, "nada inicia");
    assert!(
        nenhum.motivo.as_deref().unwrap_or("").contains("nenhum"),
        "{:?}",
        nenhum.motivo
    );

    let baixa = gate.interpret(&estado(), Some(&resposta("agendar_visita", 0.4)), limiares);
    assert_eq!(baixa.origin, DecisionOrigin::Fallback);
    assert_eq!(baixa.choice, None);
    assert!(
        baixa
            .motivo
            .as_deref()
            .unwrap_or("")
            .contains("confiança 0.40"),
        "{:?}",
        baixa.motivo
    );

    // Margem curta: dois fluxos empatados; 0,70 × 0,68.
    let mut empatado = JevResponse {
        choice: Some("agendar_visita".to_owned()),
        confidence: 0.7,
        ..JevResponse::default()
    };
    empatado
        .probabilities
        .insert("agendar_visita".to_owned(), 0.7);
    empatado
        .probabilities
        .insert("abrir_chamado".to_owned(), 0.68);
    let margem = gate.interpret(&estado(), Some(&empatado), limiares);
    assert_eq!(margem.origin, DecisionOrigin::Fallback);
    assert!(
        margem.motivo.as_deref().unwrap_or("").contains("margem"),
        "{:?}",
        margem.motivo
    );

    let inventada = gate.interpret(&estado(), Some(&resposta("cancelar_tudo", 0.99)), limiares);
    assert_eq!(inventada.origin, DecisionOrigin::Fallback);
    assert_eq!(inventada.choice, None);
    assert!(
        inventada
            .motivo
            .as_deref()
            .unwrap_or("")
            .contains("fora das opções"),
        "{:?}",
        inventada.motivo
    );

    // Sem resposta nenhuma (Jev fora): fallback com motivo, nunca fluxo por engano.
    let ausente = gate.interpret(&estado(), None, limiares);
    assert_eq!(ausente.origin, DecisionOrigin::Fallback);
    assert_eq!(
        ausente.motivo.as_deref(),
        Some("sem resposta do Jev"),
        "motivo do fallback preenchido"
    );
}

#[tokio::test]
async fn tres_gates_ligados_com_fluxo_geram_uma_requisicao_so() {
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::from([
        ("g1_transferir".to_owned(), G1_CONTINUE.to_owned()),
        ("g3_precisa".to_owned(), G3_BASE.to_owned()),
        ("g9_iniciar_fluxo".to_owned(), "agendar_visita".to_owned()),
    ])));
    let motor = MotorJev::novo(jev.clone(), gates(true, true, true), "jev-latest");

    let estado = JevState::new("whatsapp")
        .with_messages([
            JevMessage::from_customer("oi, boa tarde"),
            JevMessage::from_agent("Boa tarde! Como posso ajudar?"),
            JevMessage::from_customer("queria marcar uma visita técnica"),
        ])
        .with_handoff(vec!["cliente pede segunda via".to_owned()], true)
        .with_capabilities(true, true)
        .with_released_flows(fluxos());

    let resultado = motor.avaliar(estado, None).await;

    assert_eq!(jev.chamadas(), 1, "o lote é um pedido só");
    let pedido = &jev.pedidos()[0];
    assert_eq!(pedido.perguntas.len(), 3);
    assert!(pedido.perguntas.contains_key("g9_iniciar_fluxo"));
    // As opções do G9 vão no estado e no critério da pergunta.
    assert_eq!(
        pedido.estado["fluxos"]["liberados"][0]["id"],
        "agendar_visita"
    );
    assert!(
        pedido.perguntas["g9_iniciar_fluxo"].rubric["segunda_via_fatura"].contains("Quando usar:")
    );
    assert_eq!(
        resultado.escolha_confiavel(IdGate::G9),
        Some("agendar_visita")
    );
}

#[tokio::test]
async fn sem_fluxo_liberado_o_gate_nao_e_perguntado() {
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::from([(
        "g9_iniciar_fluxo".to_owned(),
        "agendar_visita".to_owned(),
    )])));
    let motor = MotorJev::novo(jev.clone(), gates(false, false, true), "jev-latest");

    let estado = JevState::new("whatsapp")
        .with_messages([JevMessage::from_customer("queria marcar uma visita")]);
    let resultado = motor.avaliar(estado, None).await;

    assert_eq!(jev.chamadas(), 0, "sem fluxo liberado não custa chamada");
    assert!(resultado.decisao(IdGate::G9).is_none());
}

#[tokio::test]
async fn flag_do_agente_e_a_global_governam_a_pergunta() {
    let jev = Arc::new(JevFalso::com_escolhas(BTreeMap::from([(
        "g9_iniciar_fluxo".to_owned(),
        "agendar_visita".to_owned(),
    )])));

    // Global ligada, agente desliga: não pergunta.
    let motor = MotorJev::novo(jev.clone(), gates(false, false, true), "jev-latest");
    let desligado = GateOverrides::from_json(r#"{"gates": {"g9": false}}"#).expect("ajuste válido");
    assert!(!motor.gates_efetivos(Some(desligado)).g9);
    let resultado = motor.avaliar(estado(), Some(desligado)).await;
    assert_eq!(jev.chamadas(), 0, "flag do agente desliga o G9");
    assert!(resultado.decisao(IdGate::G9).is_none());

    // Global desligada, agente liga: pergunta (o agente só ajusta o que declarar).
    let motor = MotorJev::novo(jev.clone(), gates(false, false, false), "jev-latest");
    let ligado = GateOverrides::from_json(r#"{"g9": true}"#).expect("ajuste válido");
    assert!(motor.gates_efetivos(Some(ligado)).g9);
    let resultado = motor.avaliar(estado(), Some(ligado)).await;
    assert_eq!(jev.chamadas(), 1);
    assert_eq!(
        resultado.escolha_confiavel(IdGate::G9),
        Some("agendar_visita")
    );

    // E o ajuste que liga só um gate (`apenas`) funciona para o G9.
    let apenas = GateOverrides::only(IdGate::G9);
    assert_eq!(apenas.g9, Some(true));
    assert_eq!(apenas.g8, Some(false));
}

#[test]
fn o_estado_so_leva_fluxos_quando_existem_e_muda_o_hash() {
    let sem_fluxo = JevState::new("cli").with_messages([JevMessage::from_customer("oi")]);
    assert!(
        sem_fluxo.to_json().get("fluxos").is_none(),
        "sem fluxo o estado dos outros gates não muda de forma"
    );

    let com_fluxo = sem_fluxo.clone().with_released_flows(fluxos());
    let json = com_fluxo.to_json();
    assert_eq!(json["fluxos"]["liberados"].as_array().unwrap().len(), 3);
    assert_eq!(json["fluxos"]["liberados"][1]["id"], "segunda_via_fatura");
    assert_ne!(sem_fluxo.hash(), com_fluxo.hash(), "o cache separa os dois");
}

#[test]
fn exemplos_do_fluxo_sao_limitados_no_estado() {
    let muitos = JevFlow {
        id: "agendar_visita".to_owned(),
        nome: "Agendar visita".to_owned(),
        when: "O cliente quer marcar.".to_owned(),
        not_when: String::new(),
        examples: (0..20).map(|indice| format!("exemplo {indice}")).collect(),
    };
    let estado = JevState::new("cli")
        .with_messages([JevMessage::from_customer("queria agendar")])
        .with_released_flows([muitos]);
    assert_eq!(estado.released_flows[0].examples.len(), 5);
}

/// O conjunto medido com o Jev real (FX-37) existe e é coerente com o gate.
#[test]
fn conjunto_rotulado_do_g9_serve_para_a_medicao() {
    let caminho = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/jev/g9.jsonl");
    let casos = ler_conjunto(&caminho).expect("lê tests/jev/g9.jsonl");

    assert!(
        casos.len() >= 40,
        "o conjunto precisa de pelo menos 40 casos, tem {}",
        casos.len()
    );

    for (indice, caso) in casos.iter().enumerate() {
        let numero = indice + 1;
        assert!(
            G9.applies(&caso.estado),
            "caso {numero}: o G9 precisa se aplicar para a medição valer"
        );
        let ids: Vec<&str> = caso
            .estado
            .released_flows
            .iter()
            .map(|fluxo| fluxo.id.as_str())
            .collect();
        assert!(
            caso.esperado == G9_NONE || ids.contains(&caso.esperado.as_str()),
            "caso {numero}: esperado `{}` não é um fluxo do caso nem `nenhum`",
            caso.esperado
        );
        assert!(
            caso.categoria.is_some() && caso.justificativa.is_some(),
            "caso {numero}: falta categoria ou justificativa"
        );
    }

    let positivos = casos.iter().filter(|caso| caso.esperado != G9_NONE).count();
    let negativos = casos.len() - positivos;
    assert!(
        positivos >= 15 && negativos >= 15,
        "o conjunto precisa de positivos e negativos: {positivos} × {negativos}"
    );
}
