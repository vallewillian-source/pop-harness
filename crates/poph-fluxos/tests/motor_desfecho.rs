//! Testes das decisões, do fim, do abandono e dos efeitos (FX-14).
//!
//! Cada item do "pronto quando" do bloco FX-14 tem o seu teste aqui, e cada teste cai quando o
//! comportamento é revertido (as mutações estão no relatório de entrega). As IRs são montadas à mão
//! para o teste criar o caso exato — inclusive os que a publicação recusaria, como o ciclo de
//! decisões (`FLX23`) — porque o motor também precisa falhar fechado com estrutura gravada à mão.

use poph_fluxos::ir::{
    Acao, Campo, Decisao, Destination, Effect, Etapa, Fim, Fluxo, Gatilho, Ir, Rule,
};
use poph_fluxos::motor::{Campos, Context, abandon, entry_effects, execute_action, start};
use poph_fluxos::regras::{Dados, RuleEvaluator};
use poph_types::{ActionActor, DomainError, ExecutionStatus, FieldKind, RecordEventKind};
use serde_json::{Value as Json, json};

// ------------------------------------------------------------------ auxiliares

fn campo(id: &str, tipo: FieldKind, obrigatorio: bool) -> Campo {
    Campo {
        id: id.to_owned(),
        label: id.to_uppercase(),
        tipo,
        required: obrigatorio,
        options: Vec::new(),
        help: None,
    }
}

fn acao_para(
    id: &str,
    ator: Option<ActionActor>,
    destino: Destination,
    efeitos: Vec<Effect>,
) -> Acao {
    Acao {
        id: id.to_owned(),
        label: id.to_uppercase(),
        requires: Vec::new(),
        actor: ator,
        goes_to: destino,
        effects: efeitos,
    }
}

fn etapa(id: &str, campos: Vec<Campo>, acoes: Vec<Acao>, ao_entrar: Vec<Effect>) -> Etapa {
    Etapa {
        id: id.to_owned(),
        nome: id.to_uppercase(),
        instruction: None,
        action_actors: vec![ActionActor::Agent],
        campos,
        actions: acoes,
        on_enter: ao_entrar,
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

/// Decisão que sempre é verdadeira (o campo não existe, logo está vazio).
fn decisao_sempre(id: &str, entao: Destination, senao: Destination) -> Decisao {
    decisao(
        id,
        Rule::Vazio {
            campo: "coleta.nada".to_owned(),
        },
        entao,
        senao,
    )
}

fn vai_para(id: &str) -> Destination {
    Destination::Etapa { id: id.to_owned() }
}

fn vai_para_decisao(id: &str) -> Destination {
    Destination::Decisao { id: id.to_owned() }
}

fn fim(id: &str, encerra_atendimento: bool) -> Destination {
    Destination::Fim {
        id: id.to_owned(),
        resultado: Some(format!("Resultado de {id}")),
        ends_service: encerra_atendimento,
    }
}

fn fluxo(etapas: Vec<Etapa>, decisoes: Vec<Decisao>) -> Ir {
    let inicio = etapas
        .first()
        .map(|etapa| etapa.id.clone())
        .unwrap_or_default();
    Ir {
        schema_versao: poph_fluxos::ir::SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "teste".to_owned(),
            nome: "Teste".to_owned(),
            descricao: None,
        },
        gatilho: Gatilho {
            when: "o cliente pede".to_owned(),
            not_when: None,
            examples: Vec::new(),
        },
        start: inicio,
        stages: etapas,
        decisions: decisoes,
        ends: vec![Fim {
            id: "concluido".to_owned(),
            resultado: Some("Concluído".to_owned()),
            ends_service: false,
        }],
        defs: Vec::new(),
    }
}

fn campos(valor: Json) -> Campos {
    valor
        .as_object()
        .expect("o teste informa um objeto de campos")
        .clone()
}

fn sem_campos() -> Campos {
    Campos::new()
}

fn tipos(resultado: &poph_fluxos::motor::Resultado) -> Vec<RecordEventKind> {
    resultado.events.iter().map(|evento| evento.tipo).collect()
}

fn motivo(erro: &DomainError) -> String {
    erro.to_string()
}

/// Fluxo com uma etapa `coleta` (campo `marca` e a ação `ir`) que bifurca numa decisão `escolher`.
fn fluxo_com_decisao(entao: Destination, senao: Destination) -> Ir {
    fluxo(
        vec![
            etapa(
                "coleta",
                vec![campo("marca", FieldKind::Text, false)],
                vec![acao_para(
                    "ir",
                    None,
                    vai_para_decisao("escolher"),
                    Vec::new(),
                )],
                Vec::new(),
            ),
            etapa("urgente", Vec::new(), Vec::new(), Vec::new()),
            etapa("normal", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![decisao(
            "escolher",
            Rule::Igual {
                campo: "coleta.marca".to_owned(),
                value: "urgente".to_owned(),
            },
            entao,
            senao,
        )],
    )
}

// ------------------------------------------------------------------ decisões

#[test]
fn decisao_vai_para_o_ramo_verdadeiro() {
    let ir = fluxo_com_decisao(vai_para("urgente"), vai_para("normal"));
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "urgente" })),
        ActionActor::Agent,
        Some("chave-1"),
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, vai_para("urgente"));
    assert_eq!(
        tipos(&resultado),
        vec![
            RecordEventKind::FieldsFilled,
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::DecisionResolved,
            RecordEventKind::StepEntered,
        ],
        "a decisão é resolvida na mesma passagem e a etapa de destino é a que entra"
    );
    assert_eq!(
        resultado.events[3].payload,
        json!({
            "decisao": "escolher",
            "resultado": true,
            "destino": vai_para("urgente"),
        }),
        "o evento diz a decisão, o resultado e o ramo"
    );
}

#[test]
fn decisao_vai_para_o_ramo_falso() {
    let ir = fluxo_com_decisao(vai_para("urgente"), vai_para("normal"));
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "calmo" })),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, vai_para("normal"));
    assert_eq!(
        resultado.events[3].payload,
        json!({ "decisao": "escolher", "resultado": false, "destino": vai_para("normal") })
    );
}

#[test]
fn duas_decisoes_encadeadas_chegam_a_etapa_em_uma_passagem() {
    let ir = fluxo(
        vec![
            etapa(
                "coleta",
                Vec::new(),
                vec![acao_para(
                    "ir",
                    None,
                    vai_para_decisao("primeira"),
                    Vec::new(),
                )],
                Vec::new(),
            ),
            etapa("chegada", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![
            decisao_sempre("primeira", vai_para_decisao("segunda"), vai_para("chegada")),
            decisao_sempre("segunda", vai_para("chegada"), vai_para("chegada")),
        ],
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, vai_para("chegada"));
    let resolvidas: Vec<&Json> = resultado
        .events
        .iter()
        .filter(|evento| evento.tipo == RecordEventKind::DecisionResolved)
        .map(|evento| &evento.payload)
        .collect();
    assert_eq!(
        resolvidas.len(),
        2,
        "as duas decisões viram evento, na ordem: {resolvidas:?}"
    );
    assert_eq!(resolvidas[0]["decisao"], json!("primeira"));
    assert_eq!(resolvidas[1]["decisao"], json!("segunda"));
}

#[test]
fn ciclo_de_decisoes_para_a_execucao_em_falha() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![acao_para("ir", None, vai_para_decisao("a"), Vec::new())],
            Vec::new(),
        )],
        vec![
            decisao_sempre("a", vai_para_decisao("b"), vai_para_decisao("b")),
            decisao_sempre("b", vai_para_decisao("a"), vai_para_decisao("a")),
        ],
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a passagem volta com o desfecho, não como erro da chamada");

    assert_eq!(
        resultado.new_state.status,
        ExecutionStatus::Failed,
        "ciclo de decisões põe a execução em falha"
    );
    let gravado = resultado
        .new_state
        .motivo
        .clone()
        .expect("o motivo fica no estado");
    assert!(gravado.contains("ciclo de decisões"), "{gravado}");
    assert!(
        gravado.contains("a → b → a"),
        "o motivo mostra o caminho: {gravado}"
    );
    assert_eq!(
        tipos(&resultado).last(),
        Some(&RecordEventKind::ExecutionFailed),
        "o último evento é a falha"
    );
    assert_eq!(
        resultado.events.last().expect("evento").payload["motivo"],
        json!(gravado),
        "o evento carrega o mesmo motivo"
    );
    assert!(resultado.effects.is_empty());
}

#[test]
fn dez_saltos_passam_e_onze_derrubam_a_execucao() {
    /// Fluxo que atravessa `total` decisões encadeadas antes de chegar à etapa `chegada`.
    fn cadeia(total: usize) -> Ir {
        let mut decisoes = Vec::new();
        for indice in 0..total {
            let seguinte = if indice + 1 == total {
                vai_para("chegada")
            } else {
                vai_para_decisao(&format!("d{}", indice + 1))
            };
            decisoes.push(decisao_sempre(
                &format!("d{indice}"),
                seguinte.clone(),
                seguinte,
            ));
        }
        fluxo(
            vec![
                etapa(
                    "coleta",
                    Vec::new(),
                    vec![acao_para("ir", None, vai_para_decisao("d0"), Vec::new())],
                    Vec::new(),
                ),
                etapa("chegada", Vec::new(), Vec::new(), Vec::new()),
            ],
            decisoes,
        )
    }

    let avaliador = RuleEvaluator::without_functions();

    // O teto é 10 saltos: dez decisões encadeadas chegam na etapa.
    let ir = cadeia(10);
    let estado = start(&ir).expect("início");
    let dez = execute_action(
        &ir,
        &estado,
        "ir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("dez saltos cabem no teto");
    assert_eq!(dez.new_state.status, ExecutionStatus::Active);
    assert_eq!(dez.new_state.no_atual, vai_para("chegada"));
    assert_eq!(
        dez.events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::DecisionResolved)
            .count(),
        10
    );

    // Onze passam do teto e a execução falha, com o limite no motivo.
    let ir = cadeia(11);
    let estado = start(&ir).expect("início");
    let onze = execute_action(
        &ir,
        &estado,
        "ir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a passagem volta com o desfecho");
    assert_eq!(onze.new_state.status, ExecutionStatus::Failed);
    let gravado = onze.new_state.motivo.clone().expect("motivo");
    assert!(gravado.contains("10 decisões encadeadas"), "{gravado}");
    assert!(
        onze.effects.is_empty(),
        "a falha não enfileira efeito nenhum"
    );
}

#[test]
fn o_teto_de_saltos_vem_do_contexto() {
    // A configuração `fluxos.max_saltos_decisao` apertada no contexto: 3 saltos param a execução.
    let ir = fluxo(
        vec![
            etapa(
                "coleta",
                Vec::new(),
                vec![acao_para("ir", None, vai_para_decisao("d0"), Vec::new())],
                Vec::new(),
            ),
            etapa("chegada", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![
            decisao_sempre("d0", vai_para_decisao("d1"), vai_para_decisao("d1")),
            decisao_sempre("d1", vai_para_decisao("d2"), vai_para_decisao("d2")),
            decisao_sempre("d2", vai_para_decisao("d3"), vai_para_decisao("d3")),
            decisao_sempre("d3", vai_para("chegada"), vai_para("chegada")),
        ],
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let com_teto_de_tres = execute_action(
        &ir,
        &estado,
        "ir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador).with_jump_limit(3),
    )
    .expect("a passagem volta com o desfecho");
    assert_eq!(com_teto_de_tres.new_state.status, ExecutionStatus::Failed);

    let com_teto_de_quatro = execute_action(
        &ir,
        &estado,
        "ir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador).with_jump_limit(4),
    )
    .expect("a passagem volta com o desfecho");
    assert_eq!(com_teto_de_quatro.new_state.status, ExecutionStatus::Active);
    assert_eq!(com_teto_de_quatro.new_state.no_atual, vai_para("chegada"));
}

#[test]
fn decisao_que_aponta_para_etapa_inexistente_e_ir_divergente() {
    let ir = fluxo_com_decisao(vai_para("fantasma"), vai_para("normal"));
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let erro = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "urgente" })),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect_err("o destino do ramo não existe na estrutura");
    assert!(matches!(erro, DomainError::IrDivergent { .. }), "{erro:?}");
    assert!(motivo(&erro).contains("fantasma"), "{}", motivo(&erro));
}

#[test]
fn regra_que_nao_avalia_para_a_execucao_em_falha() {
    // `funcao("grave")` sem `def` no avaliador é `regra_invalida` (FLXR01): a execução falha, com o
    // motivo, e a chamada não vira erro — a Ficha continua aberta.
    let ir = fluxo(
        vec![
            etapa(
                "coleta",
                Vec::new(),
                vec![acao_para(
                    "ir",
                    None,
                    vai_para_decisao("escolher"),
                    Vec::new(),
                )],
                Vec::new(),
            ),
            etapa("normal", Vec::new(), Vec::new(), Vec::new()),
        ],
        vec![decisao(
            "escolher",
            Rule::Funcao {
                nome: "grave".to_owned(),
            },
            vai_para("normal"),
            vai_para("normal"),
        )],
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a passagem volta com o desfecho");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Failed);
    let gravado = resultado.new_state.motivo.clone().expect("motivo");
    assert!(gravado.contains("regra inválida"), "{gravado}");
    assert_eq!(
        resultado.new_state.no_atual,
        vai_para_decisao("escolher"),
        "a execução para na decisão que não pôde avaliar"
    );
    assert_eq!(
        tipos(&resultado).last(),
        Some(&RecordEventKind::ExecutionFailed)
    );
}

// ------------------------------------------------------------------ fim

#[test]
fn fim_sem_encerrar_atendimento_conclui_a_execucao() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![acao_para(
                "concluir",
                None,
                fim("concluido", false),
                Vec::new(),
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "concluir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert_eq!(
        resultado.new_state.resultado,
        Some("Resultado de concluido".to_owned())
    );
    assert!(
        !resultado.ends_service,
        "sem `encerra_atendimento`, a conversa continua com o agente"
    );
    assert_eq!(
        tipos(&resultado),
        vec![
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::ExecutionCompleted,
        ]
    );
    assert_eq!(
        resultado.events[2].payload,
        json!({
            "fim": "concluido",
            "resultado": "Resultado de concluido",
            "encerra_atendimento": false,
        })
    );
}

#[test]
fn fim_com_encerra_atendimento_pede_o_encerramento() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![acao_para(
                "concluir",
                None,
                fim("concluido", true),
                Vec::new(),
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "concluir",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert!(
        resultado.ends_service,
        "o fim pede o encerramento; quem fecha é o chamador"
    );
    assert_eq!(
        resultado.events.last().expect("evento").payload["encerra_atendimento"],
        json!(true)
    );
}

#[test]
fn decisao_que_desemboca_num_fim_tambem_conclui() {
    let ir = fluxo_com_decisao(fim("concluido", true), vai_para("normal"));
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "ir",
        &campos(json!({ "marca": "urgente" })),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Completed);
    assert!(resultado.ends_service);
    assert_eq!(
        tipos(&resultado),
        vec![
            RecordEventKind::FieldsFilled,
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::DecisionResolved,
            RecordEventKind::ExecutionCompleted,
        ]
    );
}

// ------------------------------------------------------------------ efeitos

/// Fluxo com os três tipos de efeito na ação `confirmar` e um `ao_entrar` na etapa de destino.
fn fluxo_com_efeitos() -> Ir {
    fluxo(
        vec![
            etapa(
                "coleta",
                vec![campo("endereco", FieldKind::Text, true)],
                vec![acao_para(
                    "confirmar",
                    None,
                    vai_para("confirmacao"),
                    vec![
                        Effect::MessageToCustomer {
                            texto: "Pedido para {coleta.endereco}, do contato {cliente.peer}"
                                .to_owned(),
                        },
                        Effect::NotifyWhatsApp {
                            para: "5521999990000".to_owned(),
                            mensagem: "Ficha {ficha.id}: {coleta.endereco}".to_owned(),
                        },
                        Effect::Encaminhar {
                            departamento: "Logística".to_owned(),
                        },
                    ],
                )],
                Vec::new(),
            ),
            etapa(
                "confirmacao",
                Vec::new(),
                Vec::new(),
                vec![Effect::MessageToCustomer {
                    texto: "Confirmando o pedido.".to_owned(),
                }],
            ),
        ],
        Vec::new(),
    )
}

#[test]
fn efeitos_saem_na_ordem_da_ia_com_o_texto_resolvido() {
    let ir = fluxo_com_efeitos();
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();
    let contexto = Context::new(&avaliador)
        .with_case("ficha-1")
        .with_client("5521900000000");

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10" })),
        ActionActor::Agent,
        None,
        &contexto,
    )
    .expect("a ação é válida");

    assert_eq!(resultado.effects.len(), 4, "três da ação e um da entrada");
    assert_eq!(
        resultado
            .effects
            .iter()
            .map(|efeito| efeito.chave.as_str())
            .collect::<Vec<_>>(),
        vec![
            "acao:coleta.confirmar/0",
            "acao:coleta.confirmar/1",
            "acao:coleta.confirmar/2",
            "etapa:confirmacao/0",
        ],
        "a ordem é a declarada, e a chave diz o nó e a posição"
    );
    assert_eq!(
        resultado.effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Pedido para Rua das Flores, 10, do contato 5521900000000".to_owned()
        }
    );
    assert_eq!(
        resultado.effects[1].effect,
        Effect::NotifyWhatsApp {
            para: "5521999990000".to_owned(),
            mensagem: "Ficha ficha-1: Rua das Flores, 10".to_owned()
        }
    );
    assert_eq!(
        resultado.effects[2].effect,
        Effect::Encaminhar {
            departamento: "Logística".to_owned()
        }
    );
    assert_eq!(resultado.effects[3].tipo(), "mensagem_ao_cliente");
    assert_eq!(
        resultado
            .events
            .iter()
            .filter(|evento| evento.tipo == RecordEventKind::EffectCreated)
            .count(),
        4,
        "cada efeito entra na fila com o seu evento"
    );
}

#[test]
fn a_chave_do_efeito_e_estavel_para_a_mesma_entrada() {
    let ir = fluxo_com_efeitos();
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();
    let contexto = Context::new(&avaliador).with_case("ficha-1");
    let entrada = campos(json!({ "endereco": "Rua A" }));

    let primeiro = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entrada,
        ActionActor::Agent,
        None,
        &contexto,
    )
    .expect("primeira");
    let segundo = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entrada,
        ActionActor::Agent,
        None,
        &contexto,
    )
    .expect("segunda");

    assert_eq!(
        primeiro.effects, segundo.effects,
        "mesma entrada, mesmas chaves"
    );
}

#[test]
fn variavel_de_efeito_que_nao_existe_vira_texto_vazio() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            Vec::new(),
            vec![acao_para(
                "confirmar",
                None,
                fim("concluido", false),
                vec![Effect::MessageToCustomer {
                    texto: "Pedido para {coleta.endereco}.".to_owned(),
                }],
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &sem_campos(),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Pedido para .".to_owned()
        },
        "variável inexistente sai vazia (FLX59 avisa na publicação)"
    );
}

#[test]
fn o_dado_do_cliente_entra_como_valor_no_texto_do_efeito() {
    let ir = fluxo(
        vec![etapa(
            "coleta",
            vec![campo("endereco", FieldKind::Text, true)],
            vec![acao_para(
                "confirmar",
                None,
                fim("concluido", false),
                vec![Effect::MessageToCustomer {
                    texto: "Pedido: {coleta.endereco}".to_owned(),
                }],
            )],
            Vec::new(),
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "  Rua A  " })),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.effects[0].effect,
        Effect::MessageToCustomer {
            texto: "Pedido: Rua A".to_owned()
        },
        "o texto canônico do campo entra como valor"
    );
}

#[test]
fn efeitos_da_entrada_da_primeira_etapa_saem_pela_funcao_publica() {
    let ir = fluxo(
        vec![etapa(
            "abertura",
            Vec::new(),
            Vec::new(),
            vec![Effect::NotifyWhatsApp {
                para: "5521999990000".to_owned(),
                mensagem: "Atendimento {ficha.id} começou".to_owned(),
            }],
        )],
        Vec::new(),
    );
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();
    let contexto = Context::new(&avaliador).with_case("ficha-9");
    let dados: Dados = estado.dados.clone();

    let efeitos = entry_effects(&ir, "abertura", &dados, &contexto).expect("etapa existe");
    assert_eq!(efeitos.len(), 1);
    assert_eq!(efeitos[0].chave, "etapa:abertura/0");
    assert_eq!(
        efeitos[0].effect,
        Effect::NotifyWhatsApp {
            para: "5521999990000".to_owned(),
            mensagem: "Atendimento ficha-9 começou".to_owned()
        }
    );

    let erro = entry_effects(&ir, "fantasma", &dados, &contexto).expect_err("etapa não existe");
    assert!(matches!(erro, DomainError::IrDivergent { .. }), "{erro:?}");
}

#[test]
fn etapa_com_efeito_de_entrada_vira_efeito_ao_avancar() {
    let ir = fluxo_com_efeitos();
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();

    let resultado = execute_action(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua B" })),
        ActionActor::Agent,
        None,
        &Context::new(&avaliador),
    )
    .expect("a ação é válida");

    assert_eq!(resultado.new_state.no_atual, vai_para("confirmacao"));
    assert_eq!(
        resultado.effects.last().expect("efeito").chave,
        "etapa:confirmacao/0"
    );
    assert_eq!(
        resultado.events.last().expect("evento").tipo,
        RecordEventKind::EffectCreated,
        "o efeito da etapa nova é o último evento da passagem"
    );
}

// ------------------------------------------------------------------ abandono e idempotência

#[test]
fn abandono_marca_a_execucao_e_nao_produz_efeito() {
    let ir = fluxo_com_efeitos();
    let estado = start(&ir).expect("início");

    let resultado = abandon(
        &estado,
        "  o cliente pediu para parar  ",
        ActionActor::Agent,
        None,
    )
    .expect("a execução está ativa");

    assert_eq!(resultado.new_state.status, ExecutionStatus::Abandoned);
    assert_eq!(
        resultado.new_state.motivo,
        Some("o cliente pediu para parar".to_owned()),
        "o motivo é gravado sem os espaços das pontas"
    );
    assert!(resultado.effects.is_empty(), "abandono não dispara efeito");
    assert!(resultado.waiting.is_empty());
    assert!(!resultado.ends_service);
    assert_eq!(tipos(&resultado), vec![RecordEventKind::ExecutionAbandoned]);
    assert_eq!(
        resultado.events[0].payload,
        json!({ "motivo": "o cliente pediu para parar", "no": "coleta" })
    );
}

#[test]
fn abandono_sem_motivo_e_recusado_e_execucao_encerrada_nao_e_abandonada_de_novo() {
    let ir = fluxo_com_efeitos();
    let estado = start(&ir).expect("início");

    let erro = abandon(&estado, "   ", ActionActor::Agent, None).expect_err("motivo vazio");
    assert!(matches!(erro, DomainError::Validation { .. }), "{erro:?}");

    let abandonada = abandon(&estado, "desistiu", ActionActor::Attendant, None)
        .expect("abandona")
        .new_state;
    let erro =
        abandon(&abandonada, "de novo", ActionActor::Agent, None).expect_err("já está abandonada");
    assert!(matches!(erro, DomainError::Conflict { .. }), "{erro:?}");
}

#[test]
fn repetir_a_chave_nao_duplica_evento_nem_efeito() {
    let ir = fluxo_com_efeitos();
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();
    let contexto = Context::new(&avaliador).with_case("ficha-1");
    let entrada = campos(json!({ "endereco": "Rua C" }));

    let primeiro = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entrada,
        ActionActor::Agent,
        Some("chave-1"),
        &contexto,
    )
    .expect("primeira");

    let segundo = execute_action(
        &ir,
        &primeiro.new_state,
        "confirmar",
        &entrada,
        ActionActor::Agent,
        Some("chave-1"),
        &contexto,
    )
    .expect("repetida");

    assert!(segundo.repeated);
    assert!(segundo.events.is_empty(), "repetir não grava evento");
    assert!(segundo.effects.is_empty(), "repetir não enfileira efeito");
    assert!(!segundo.ends_service, "repetir não pede encerramento");
    assert_eq!(
        segundo.new_state, primeiro.new_state,
        "o estado não muda na repetição"
    );
    assert_eq!(primeiro.new_state.keys.len(), 1);
}

#[test]
fn a_mesma_entrada_com_decisao_e_efeito_da_o_mesmo_resultado() {
    let ir = fluxo_com_efeitos();
    let estado = start(&ir).expect("início");
    let avaliador = RuleEvaluator::without_functions();
    let contexto = Context::new(&avaliador)
        .with_case("ficha-1")
        .with_client("5521900000000");
    let entrada = campos(json!({ "endereco": "Rua D" }));

    let primeiro = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entrada,
        ActionActor::Agent,
        Some("chave"),
        &contexto,
    )
    .expect("primeira");
    let segundo = execute_action(
        &ir,
        &estado,
        "confirmar",
        &entrada,
        ActionActor::Agent,
        Some("chave"),
        &contexto,
    )
    .expect("segunda");

    assert_eq!(primeiro, segundo);
    assert!(primeiro.ends_service == segundo.ends_service);
}
