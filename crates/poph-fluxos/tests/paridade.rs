//! Casos de paridade com o POP Atende (FX-15).
//!
//! Cada caso desta suíte reproduz um comportamento que o motor do POP Atende tem e que
//! **permanece** no Harness. O cabeçalho de cada caso cita o teste ou a função de onde ele veio:
//! `apps/backend/tests/test_fichas/` (a suíte do POP Atende) ou `apps/backend/core/fichas/`
//! (`state_engine.py`, `validators.py`).
//!
//! As diferenças **intencionais** do PRD de Fluxos §8 estão em `docs/FLUXOS_PARIDADE.md`, com o
//! teste que prova cada uma. O que o PRD removeu não é reproduzido aqui: `completion`
//! `any`/`all`, auto-transição por busca em largura, atores `client`/`provider` e o fechamento da
//! Ficha por `next_step = "closed"`. Onde o caso serve justamente para provar que a remoção vale,
//! ele está nomeado como tal.
//!
//! As IRs são montadas à mão (como nos testes do motor) para o caso ser o menor possível; os nomes
//! de etapa e de campo seguem os templates do POP Atende (`abertura`, `orcamento`, `fechamento`).

use std::sync::OnceLock;

use poph_fluxos::ir::{
    Acao, Campo, Decisao, Destino, Efeito, Etapa, Fim, Fluxo, Gatilho, Ir, Regra,
};
use poph_fluxos::motor::{
    Campos, Contexto, atores_exigidos, efeitos_de_entrada, executar_acao, iniciar,
};
use poph_fluxos::regras::{Avaliador, Dados};
use poph_types::{ActionActor, ExecutionStatus, FieldKind, RecordEventKind};
use serde_json::{Value as Json, json};

// ------------------------------------------------------------------ auxiliares

/// Contexto mínimo das chamadas do motor (sem Ficha, sem cliente, só combinadores).
fn contexto() -> Contexto<'static> {
    static AVALIADOR: OnceLock<Avaliador> = OnceLock::new();
    Contexto::novo(AVALIADOR.get_or_init(Avaliador::sem_funcoes))
}

fn campo(id: &str, tipo: FieldKind, obrigatorio: bool) -> Campo {
    Campo {
        id: id.to_owned(),
        rotulo: id.to_uppercase(),
        tipo,
        obrigatorio,
        opcoes: Vec::new(),
        ajuda: None,
    }
}

fn campo_opcao(id: &str, opcoes: &[&str]) -> Campo {
    Campo {
        id: id.to_owned(),
        rotulo: id.to_uppercase(),
        tipo: FieldKind::Option,
        obrigatorio: true,
        opcoes: opcoes.iter().map(|opcao| (*opcao).to_owned()).collect(),
        ajuda: None,
    }
}

fn acao(id: &str, ator: Option<ActionActor>, exige: &[&str], destino: Destino) -> Acao {
    Acao {
        id: id.to_owned(),
        rotulo: id.to_uppercase(),
        exige: exige.iter().map(|nome| (*nome).to_owned()).collect(),
        ator,
        vai_para: destino,
        efeitos: Vec::new(),
    }
}

fn acao_para_etapa(id: &str, ator: Option<ActionActor>, exige: &[&str], vai_para: &str) -> Acao {
    acao(
        id,
        ator,
        exige,
        Destino::Etapa {
            id: vai_para.to_owned(),
        },
    )
}

fn acao_para_fim(
    id: &str,
    ator: Option<ActionActor>,
    resultado: &str,
    encerra_atendimento: bool,
) -> Acao {
    acao(
        id,
        ator,
        &[],
        Destino::Fim {
            id: "concluido".to_owned(),
            resultado: Some(resultado.to_owned()),
            encerra_atendimento,
        },
    )
}

fn etapa(id: &str, atores: &[ActionActor], campos: Vec<Campo>, acoes: Vec<Acao>) -> Etapa {
    Etapa {
        id: id.to_owned(),
        nome: id.to_uppercase(),
        instrucao: None,
        atores_acao: atores.to_vec(),
        campos,
        acoes,
        ao_entrar: Vec::new(),
    }
}

fn etapa_com_entrada(
    id: &str,
    atores: &[ActionActor],
    campos: Vec<Campo>,
    acoes: Vec<Acao>,
    ao_entrar: Vec<Efeito>,
) -> Etapa {
    Etapa {
        ao_entrar,
        ..etapa(id, atores, campos, acoes)
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

fn vai_para(id: &str) -> Destino {
    Destino::Etapa { id: id.to_owned() }
}

fn fluxo(inicio: &str, etapas: Vec<Etapa>, decisoes: Vec<Decisao>) -> Ir {
    Ir {
        schema_versao: poph_fluxos::ir::SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "teste_paridade".to_owned(),
            nome: "Teste da paridade".to_owned(),
            descricao: None,
        },
        gatilho: Gatilho {
            quando: "o cliente pede um orçamento".to_owned(),
            nao_quando: None,
            exemplos: vec!["quero um orçamento".to_owned()],
        },
        inicio: inicio.to_owned(),
        etapas,
        decisoes,
        fins: vec![
            Fim {
                id: "concluido".to_owned(),
                resultado: Some("Concluído".to_owned()),
                encerra_atendimento: false,
            },
            Fim {
                id: "encerrado".to_owned(),
                resultado: Some("Atendimento encerrado".to_owned()),
                encerra_atendimento: true,
            },
        ],
        defs: Vec::new(),
    }
}

fn campos(valor: Json) -> Campos {
    valor
        .as_object()
        .expect("o caso informa um objeto de campos")
        .clone()
}

fn sem_campos() -> Campos {
    Campos::new()
}

fn dados(valor: Json) -> Dados {
    valor
        .as_object()
        .expect("o caso informa um objeto de dados")
        .clone()
}

fn tipos(resultado: &poph_fluxos::motor::Resultado) -> Vec<RecordEventKind> {
    resultado.eventos.iter().map(|evento| evento.tipo).collect()
}

/// Fluxo de duas etapas em que `execucao` tem uma ação restrita ao agente e outra ao atendente,
/// ambas levando para `pos` — o cenário real da `fch_BB9AE9BDE1A4` do POP Atende.
fn fluxo_de_dois_atores() -> Ir {
    fluxo(
        "execucao",
        vec![
            etapa(
                "execucao",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![
                    acao_para_etapa("agente_conclui", Some(ActionActor::Agent), &[], "pos"),
                    acao_para_etapa("atendente_valida", Some(ActionActor::Attendant), &[], "pos"),
                ],
            ),
            etapa(
                "pos",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim(
                    "fechar",
                    Some(ActionActor::Agent),
                    "Feito",
                    false,
                )],
            ),
        ],
        Vec::new(),
    )
}

// ---------------------------------------------------------------------------------------------
// Avanço `auto` (PRD §6.4; `state_engine.resolve_auto_advance`)
// ---------------------------------------------------------------------------------------------

/// Origem: `test_state_engine.py::TestStateEngineExecution::test_any_completion_advances_immediately`
/// e `state_engine.execute_action` (completion `auto` com um único ator exigido).
#[test]
fn auto_avanca_quando_o_unico_ator_exigido_e_quem_agiu() {
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent],
                vec![campo("relato", FieldKind::Text, true)],
                vec![acao_para_etapa(
                    "registrar_relato",
                    None,
                    &["relato"],
                    "orcamento",
                )],
            ),
            etapa(
                "orcamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "registrar_relato",
        &campos(json!({ "relato": "Chuveiro queimado" })),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.novo_estado.no_atual, vai_para("orcamento"));
    assert_eq!(resultado.novo_estado.status, ExecutionStatus::Active);
    assert_eq!(
        resultado.novo_estado.dados["abertura"]["relato"],
        json!("Chuveiro queimado")
    );
    assert!(
        resultado.aguardando.is_empty(),
        "quem age sozinho não deixa ninguém pendente"
    );
    assert!(tipos(&resultado).contains(&RecordEventKind::StepEntered));
}

/// Origem: `test_state_engine.py::TestCompletionGatedActors::test_all_waits_for_second_gated_actor_then_advances`
/// (primeira metade) — o atendente age e o agente ainda não, então a etapa fica `parcial`.
#[test]
fn auto_fica_parcial_quando_falta_o_outro_ator_restrito() {
    let ir = fluxo_de_dois_atores();
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "atendente_valida",
        &sem_campos(),
        ActionActor::Attendant,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.novo_estado.no_atual, vai_para("execucao"));
    assert_eq!(resultado.novo_estado.status, ExecutionStatus::Active);
    assert_eq!(
        resultado.novo_estado.atores_que_agiram,
        vec![ActionActor::Attendant]
    );
    assert_eq!(
        resultado.aguardando,
        vec![ActionActor::Agent],
        "o Python devolve `waiting_on == [agent]`"
    );
    assert_eq!(tipos(&resultado), vec![RecordEventKind::ActionExecuted]);
    assert_eq!(
        atores_exigidos(ir.etapas.first().expect("há etapa")),
        vec![ActionActor::Agent, ActionActor::Attendant]
    );
}

/// Origem: `test_state_engine.py::TestCompletionGatedActors::test_all_waits_for_second_gated_actor_then_advances`
/// (segunda metade) — o segundo ator age e a etapa avança.
#[test]
fn auto_avanca_quando_o_segundo_ator_restrito_age() {
    let ir = fluxo_de_dois_atores();
    let estado = iniciar(&ir).expect("a execução começa");
    let parcial = executar_acao(
        &ir,
        &estado,
        "atendente_valida",
        &sem_campos(),
        ActionActor::Attendant,
        Some("k1"),
        &contexto(),
    )
    .expect("a primeira ação é aceita");

    let resultado = executar_acao(
        &ir,
        &parcial.novo_estado,
        "agente_conclui",
        &sem_campos(),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect("a segunda ação é aceita");

    assert_eq!(resultado.novo_estado.no_atual, vai_para("pos"));
    assert!(resultado.aguardando.is_empty());
    assert_eq!(resultado.novo_estado.atores_que_agiram, Vec::new());
    assert_eq!(tipos(&resultado)[0], RecordEventKind::ActionExecuted);
    assert!(tipos(&resultado).contains(&RecordEventKind::StepCompleted));
}

/// Origem: `state_engine.resolve_auto_advance` — a decisão olha as ações **não executadas**, então
/// a ordem dos dois atores não muda o destino.
#[test]
fn auto_os_dois_atores_chegam_ao_mesmo_destino_em_qualquer_ordem() {
    let ir = fluxo_de_dois_atores();
    let estado = iniciar(&ir).expect("a execução começa");

    // Agente primeiro: o atendente fica pendente.
    let agente_primeiro = executar_acao(
        &ir,
        &estado,
        "agente_conclui",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação do agente é aceita");
    assert_eq!(agente_primeiro.aguardando, vec![ActionActor::Attendant]);
    let depois = executar_acao(
        &ir,
        &agente_primeiro.novo_estado,
        "atendente_valida",
        &sem_campos(),
        ActionActor::Attendant,
        Some("k2"),
        &contexto(),
    )
    .expect("a ação do atendente é aceita");
    assert_eq!(depois.novo_estado.no_atual, vai_para("pos"));

    // Atendente primeiro: o agente fica pendente, e o destino é o mesmo.
    let atendente_primeiro = executar_acao(
        &ir,
        &estado,
        "atendente_valida",
        &sem_campos(),
        ActionActor::Attendant,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação do atendente é aceita");
    assert_eq!(atendente_primeiro.aguardando, vec![ActionActor::Agent]);
    let depois = executar_acao(
        &ir,
        &atendente_primeiro.novo_estado,
        "agente_conclui",
        &sem_campos(),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect("a ação do agente é aceita");
    assert_eq!(depois.novo_estado.no_atual, vai_para("pos"));
}

/// Origem: `test_state_engine.py::TestCompletionGatedActors::test_all_with_client_actor_advances_on_agent_action`
/// — ator que está em `actors` mas não tem ação própria restrita nunca trava a etapa.
#[test]
fn auto_ator_sem_acao_propria_nao_trava_a_etapa() {
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![acao_para_etapa(
                    "relatar",
                    Some(ActionActor::Agent),
                    &[],
                    "agendamento",
                )],
            ),
            etapa(
                "agendamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "relatar",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.novo_estado.no_atual, vai_para("agendamento"));
    assert!(resultado.aguardando.is_empty());
}

/// Origem: `test_state_engine.py::TestAutoCompletionExecution::test_auto_close_action_is_decisive`
/// — ação de encerramento efetiva mesmo com outro participante pendente.
#[test]
fn auto_a_acao_que_leva_ao_fim_e_decisiva_com_outro_ator_pendente() {
    let ir = fluxo(
        "orcamento",
        vec![
            etapa(
                "orcamento",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![
                    acao_para_fim("cancelar", Some(ActionActor::Agent), "Cancelado", false),
                    acao_para_etapa("aprovar", Some(ActionActor::Attendant), &[], "fechamento"),
                ],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "cancelar",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.novo_estado.status, ExecutionStatus::Completed);
    assert_eq!(
        resultado.novo_estado.resultado.as_deref(),
        Some("Cancelado")
    );
    assert_eq!(
        tipos(&resultado).last(),
        Some(&RecordEventKind::ExecutionCompleted)
    );
    assert!(resultado.aguardando.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Ação restrita por ator (PRD §8, linha dos atores; `state_engine.execute_action` passos 2 e 3)
// ---------------------------------------------------------------------------------------------

/// Origem: `test_state_engine.py::TestStateEngineValidation::test_actor_not_permitted_in_step` e
/// `state_engine.execute_action` (conferência de `action_actor_required`).
#[test]
fn acao_restrita_ao_atendente_recusada_para_o_agente() {
    let ir = fluxo(
        "validacao",
        vec![
            etapa(
                "validacao",
                &[ActionActor::Agent, ActionActor::Attendant],
                Vec::new(),
                vec![
                    acao_para_etapa("validar", Some(ActionActor::Attendant), &[], "pos"),
                    acao_para_fim("encerrar", None, "Feito", false),
                ],
            ),
            etapa(
                "pos",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let erro = executar_acao(
        &ir,
        &estado,
        "validar",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect_err("o agente não executa ação do atendente");

    assert_eq!(erro.api_code(), "acao_invalida");
    assert!(
        erro.to_string().contains("restrita ao ator `atendente`"),
        "{erro}"
    );
}

/// Origem: `test_state_engine.py::TestStateEngineValidation::test_actor_not_permitted_in_step` —
/// ator fora da lista `actors` do step.
#[test]
fn ator_fora_dos_atores_da_etapa_e_recusado() {
    let ir = fluxo(
        "atendimento_humano",
        vec![etapa(
            "atendimento_humano",
            &[ActionActor::Attendant],
            Vec::new(),
            vec![acao_para_fim("encerrar", None, "Feito", false)],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let erro = executar_acao(
        &ir,
        &estado,
        "encerrar",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect_err("o agente não age nesta etapa");

    assert_eq!(erro.api_code(), "acao_invalida");
    assert!(
        erro.to_string()
            .contains("não age na etapa `atendimento_humano`"),
        "{erro}"
    );
}

/// Origem: `state_engine.execute_action` (passo 3, `InvalidAction`) e PRD §8, linha da
/// auto-transição: o motor do POP Atende procurava a ação por busca em largura; aqui a ação que não
/// existe na etapa devolve **as válidas** e nada é executado.
#[test]
fn acao_inexistente_devolve_as_validas_e_nao_muda_nada() {
    let ir = fluxo(
        "orcamento",
        vec![
            etapa(
                "orcamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![
                    acao_para_etapa("enviar_orcamento", None, &[], "fechamento"),
                    acao_para_fim("cancelar", None, "Cancelado", false),
                ],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let erro = executar_acao(
        &ir,
        &estado,
        "aprovar",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect_err("a ação não existe na etapa");

    let mensagem = erro.to_string();
    assert_eq!(erro.api_code(), "acao_invalida");
    assert!(mensagem.contains("`enviar_orcamento`"), "{mensagem}");
    assert!(mensagem.contains("`cancelar`"), "{mensagem}");
    // O estado de entrada segue intacto: nada foi gravado e nenhuma chave foi consumida.
    assert!(estado.dados.is_empty());
    assert!(estado.chaves.is_empty());
    assert_eq!(estado.no_atual, vai_para("orcamento"));
}

/// Origem: `test_state_engine.py::TestFichaStateValidation::test_closed_ficha_rejects_actions`
/// (`FichaAlreadyClosed`) — execução encerrada não aceita ação.
#[test]
fn execucao_encerrada_nao_aceita_acao() {
    let ir = fluxo(
        "coleta",
        vec![etapa(
            "coleta",
            &[ActionActor::Agent],
            Vec::new(),
            vec![
                acao_para_fim("concluir", None, "Feito", false),
                acao_para_etapa("voltar", None, &[], "coleta"),
            ],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");
    let concluida = executar_acao(
        &ir,
        &estado,
        "concluir",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");
    assert_eq!(concluida.novo_estado.status, ExecutionStatus::Completed);

    let erro = executar_acao(
        &ir,
        &concluida.novo_estado,
        "voltar",
        &sem_campos(),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect_err("execução concluída não aceita ação");

    assert_eq!(erro.api_code(), "acao_invalida");
    assert!(erro.to_string().contains("não aceita ação"), "{erro}");
}

/// Origem: `test_state_engine.py::TestFichaStateValidation::test_closing_pending_rejects_actions`
/// (`ClosingPendingError`). No POP Atende a Ficha passava pelo estado `closing_pending` antes de
/// fechar; no Harness esse estado **não existe** — fechar atendimento é decisão explícita (PRD §8).
#[test]
fn nao_existe_estado_closing_pending() {
    let erro = ExecutionStatus::from_text("closing_pending")
        .expect_err("`closing_pending` não é situação de execução no Harness");

    assert_eq!(erro.api_code(), "validation_error");
    assert!(
        erro.to_string()
            .contains("aceitos: ativa, concluida, abandonada, falha"),
        "{erro}"
    );
    for situacao in ["ativa", "concluida", "abandonada", "falha"] {
        assert!(
            ExecutionStatus::from_text(situacao).is_ok(),
            "`{situacao}` é situação válida"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Condicionais (PRD §8, linha da condicional; `core/fichas/conditionals.py`, resolvido em
// `state_engine._resolve_conditional_route`)
// ---------------------------------------------------------------------------------------------

/// Origem: `test_conditional_runtime.py::TestEvaluatePopExpr` — cada caso do avaliador de
/// `pop_expr` do POP Atende, agora como combinador declarativo.
#[test]
fn condicional_com_os_operadores_do_pop_atende_escolhe_o_ramo() {
    let avaliador = Avaliador::sem_funcoes();
    let execucao = dados(json!({
        "orcamento": {
            "tipo": "hidraulica",
            "valor": 800,
            "confirmou": true,
            "ativo": false,
            "servicos": "agua e gas",
            "a": 10,
            "b": 20,
        }
    }));

    let casos: Vec<(&str, Regra, bool)> = vec![
        (
            "`tipo == \"hidraulica\"`",
            Regra::Igual {
                campo: "orcamento.tipo".to_owned(),
                valor: "hidraulica".to_owned(),
            },
            true,
        ),
        (
            "`tipo == \"eletrica\"`",
            Regra::Igual {
                campo: "orcamento.tipo".to_owned(),
                valor: "eletrica".to_owned(),
            },
            false,
        ),
        (
            "`valor > 500`",
            Regra::Maior {
                campo: "orcamento.valor".to_owned(),
                valor: "500".to_owned(),
            },
            true,
        ),
        (
            "`valor > 1000`",
            Regra::Maior {
                campo: "orcamento.valor".to_owned(),
                valor: "1000".to_owned(),
            },
            false,
        ),
        (
            "`valor >= 800`, montado como `nao(menor(...))`",
            Regra::Nao {
                args: vec![Regra::Menor {
                    campo: "orcamento.valor".to_owned(),
                    valor: "800".to_owned(),
                }],
            },
            true,
        ),
        (
            "`confirmou == true`",
            Regra::Igual {
                campo: "orcamento.confirmou".to_owned(),
                valor: "sim".to_owned(),
            },
            true,
        ),
        (
            "`a > 5 and b > 15`",
            Regra::E {
                args: vec![
                    Regra::Maior {
                        campo: "orcamento.a".to_owned(),
                        valor: "5".to_owned(),
                    },
                    Regra::Maior {
                        campo: "orcamento.b".to_owned(),
                        valor: "15".to_owned(),
                    },
                ],
            },
            true,
        ),
        (
            "`a > 15 and b > 5`",
            Regra::E {
                args: vec![
                    Regra::Maior {
                        campo: "orcamento.a".to_owned(),
                        valor: "15".to_owned(),
                    },
                    Regra::Maior {
                        campo: "orcamento.b".to_owned(),
                        valor: "5".to_owned(),
                    },
                ],
            },
            false,
        ),
        (
            "`a > 15 or b > 15`",
            Regra::Ou {
                args: vec![
                    Regra::Maior {
                        campo: "orcamento.a".to_owned(),
                        valor: "15".to_owned(),
                    },
                    Regra::Maior {
                        campo: "orcamento.b".to_owned(),
                        valor: "15".to_owned(),
                    },
                ],
            },
            true,
        ),
        (
            "`not ativo`",
            Regra::Nao {
                args: vec![Regra::Igual {
                    campo: "orcamento.ativo".to_owned(),
                    valor: "sim".to_owned(),
                }],
            },
            true,
        ),
        (
            "`\"agua\" in servicos`",
            Regra::Contem {
                campo: "orcamento.servicos".to_owned(),
                valor: "agua".to_owned(),
            },
            true,
        ),
        (
            "`\"eletrica\" in servicos`",
            Regra::Contem {
                campo: "orcamento.servicos".to_owned(),
                valor: "eletrica".to_owned(),
            },
            false,
        ),
    ];

    for (nome, regra, esperado) in casos {
        let obtido = avaliador
            .avaliar(&regra, &execucao)
            .unwrap_or_else(|erro| panic!("{nome}: {erro}"));
        assert_eq!(obtido, esperado, "{nome}");
    }
}

/// Origem: `state_engine._resolve_conditional_route` — condicional que leva a outra condicional,
/// resolvida na mesma passagem (o `while` do Python).
#[test]
fn duas_condicionais_encadeadas_escolhem_a_etapa_final() {
    let ir = fluxo(
        "triagem",
        vec![
            etapa(
                "triagem",
                &[ActionActor::Agent],
                vec![
                    campo_opcao("tipo", &["hidraulica", "eletrica"]),
                    campo("valor", FieldKind::Number, true),
                ],
                vec![acao(
                    "seguir",
                    None,
                    &["tipo", "valor"],
                    Destino::Decisao {
                        id: "tipo_define".to_owned(),
                    },
                )],
            ),
            etapa(
                "caro",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Caro", false)],
            ),
            etapa(
                "barato",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Barato", false)],
            ),
        ],
        vec![
            decisao(
                "tipo_define",
                Regra::Igual {
                    campo: "triagem.tipo".to_owned(),
                    valor: "hidraulica".to_owned(),
                },
                Destino::Decisao {
                    id: "valor_define".to_owned(),
                },
                vai_para("barato"),
            ),
            decisao(
                "valor_define",
                Regra::Maior {
                    campo: "triagem.valor".to_owned(),
                    valor: "500".to_owned(),
                },
                vai_para("caro"),
                vai_para("barato"),
            ),
        ],
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "seguir",
        &campos(json!({ "tipo": "hidraulica", "valor": 800 })),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.novo_estado.no_atual, vai_para("caro"));
    let resolvidas: Vec<&poph_fluxos::motor::Evento> = resultado
        .eventos
        .iter()
        .filter(|evento| evento.tipo == RecordEventKind::DecisionResolved)
        .collect();
    assert_eq!(resolvidas.len(), 2, "duas decisões numa passagem só");
    assert_eq!(resolvidas[0].payload["decisao"], json!("tipo_define"));
    assert_eq!(resolvidas[1].payload["decisao"], json!("valor_define"));
}

/// Script de apoio do caso seguinte: um fluxo válido com a `def` que decide o ramo.
const SCRIPT_COM_DEF: &str = r#"
fluxo(
    id = "teste_paridade_def",
    nome = "Teste da paridade com def",
    inicio = "triagem",
    gatilho = gatilho(
        quando = "o cliente relata um problema",
        nao_quando = "o cliente quer falar com uma pessoa",
        exemplos = ["o chuveiro parou"],
    ),
)

etapa(
    id = "triagem",
    nome = "Triagem",
    campos = [campo("relato", "Relato", obrigatorio = True)],
    acoes = [acao("seguir", "Seguir", exige = ["relato"], vai_para = fim("pronto"))],
)

fim("pronto", resultado = "Registrado")

def grave(dados):
    resumo = dados.get("triagem", {}).get("relato", "").lower()
    return "parou" in resumo
"#;

/// Origem: PRD §8, linha da condicional (o lado avançado: `def`) e `conditionals.py`, que avaliava
/// expressão em texto. Aqui a regra pode ser uma função declarada no script.
#[test]
fn condicional_com_def_escolhe_o_ramo() {
    let avaliador = Avaliador::novo(SCRIPT_COM_DEF).expect("o script de apoio carrega");
    let ir = fluxo(
        "triagem",
        vec![
            etapa(
                "triagem",
                &[ActionActor::Agent],
                vec![campo("relato", FieldKind::Text, true)],
                vec![acao(
                    "seguir",
                    None,
                    &["relato"],
                    Destino::Decisao {
                        id: "grave_ou_simples".to_owned(),
                    },
                )],
            ),
            etapa(
                "grave",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Grave", false)],
            ),
            etapa(
                "simples",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Simples", false)],
            ),
        ],
        vec![decisao(
            "grave_ou_simples",
            Regra::Funcao {
                nome: "grave".to_owned(),
            },
            vai_para("grave"),
            vai_para("simples"),
        )],
    );
    let contexto = Contexto::novo(&avaliador);
    let estado = iniciar(&ir).expect("a execução começa");

    let caso_grave = executar_acao(
        &ir,
        &estado,
        "seguir",
        &campos(json!({ "relato": "O chuveiro parou de esquentar" })),
        ActionActor::Agent,
        Some("k1"),
        &contexto,
    )
    .expect("a ação é aceita");
    assert_eq!(caso_grave.novo_estado.no_atual, vai_para("grave"));

    let caso_simples = executar_acao(
        &ir,
        &estado,
        "seguir",
        &campos(json!({ "relato": "Quero trocar a torneira da pia" })),
        ActionActor::Agent,
        Some("k2"),
        &contexto,
    )
    .expect("a ação é aceita");
    assert_eq!(caso_simples.novo_estado.no_atual, vai_para("simples"));
}

/// Origem: `state_engine._resolve_conditional_route` — no POP Atende, condicional que aponta para
/// etapa inexistente simplesmente ficava no nó; no Harness o motor **falha fechado**.
#[test]
fn condicional_que_vai_para_etapa_inexistente_falha_fechado() {
    let ir = fluxo(
        "triagem",
        vec![etapa(
            "triagem",
            &[ActionActor::Agent],
            Vec::new(),
            vec![acao(
                "seguir",
                None,
                &[],
                Destino::Decisao {
                    id: "bifurcar".to_owned(),
                },
            )],
        )],
        vec![decisao(
            "bifurcar",
            Regra::Vazio {
                campo: "triagem.nada".to_owned(),
            },
            vai_para("fantasma"),
            vai_para("triagem"),
        )],
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let erro = executar_acao(
        &ir,
        &estado,
        "seguir",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect_err("destino inexistente é divergência de IR");

    assert_eq!(erro.api_code(), "ir_divergente");
    assert!(erro.to_string().contains("`fantasma`"), "{erro}");
}

// ---------------------------------------------------------------------------------------------
// Campos obrigatórios e tipos de campo (PRD §8, linha dos campos; `core/fichas/validators.py`)
// ---------------------------------------------------------------------------------------------

/// Origem: `test_state_engine.py::TestFieldValidation::test_required_field_missing` e
/// `TestStateEngineValidation::test_missing_required_field` — erro com a lista do que falta.
#[test]
fn campo_obrigatorio_faltando_devolve_a_lista() {
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent],
                vec![
                    campo("valor", FieldKind::Number, true),
                    campo("descricao", FieldKind::Text, true),
                ],
                vec![acao_para_etapa(
                    "enviar_orcamento",
                    None,
                    &["valor", "descricao"],
                    "fechamento",
                )],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let erro = executar_acao(
        &ir,
        &estado,
        "enviar_orcamento",
        &campos(json!({ "valor": 1000.0 })),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect_err("falta o campo obrigatório");

    assert_eq!(erro.api_code(), "campos_invalidos");
    let mensagem = erro.to_string();
    assert!(mensagem.contains("`descricao`"), "{mensagem}");
    assert!(mensagem.contains("ainda não foi preenchido"), "{mensagem}");
}

/// Origem: `test_state_engine.py::TestFieldValidation::test_text_field_validation` — o tipo é
/// conferido de forma estrita (o `max_length` do POP Atende não existe na linguagem).
#[test]
fn campo_de_texto_recusa_numero() {
    let ir = fluxo(
        "abertura",
        vec![etapa(
            "abertura",
            &[ActionActor::Agent],
            vec![campo("relato", FieldKind::Text, true)],
            vec![acao_para_fim("registrar", None, "Feito", false)],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let aceito = executar_acao(
        &ir,
        &estado,
        "registrar",
        &campos(json!({ "relato": "  texto curto  " })),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("texto é aceito");
    assert_eq!(
        aceito.novo_estado.dados["abertura"]["relato"],
        json!("texto curto"),
        "o texto é gravado sem os espaços das pontas"
    );

    let erro = executar_acao(
        &ir,
        &estado,
        "registrar",
        &campos(json!({ "relato": 42 })),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect_err("número não é texto");

    assert_eq!(erro.api_code(), "campos_invalidos");
    assert!(erro.to_string().contains("esperava texto"), "{erro}");
}

/// Origem: `test_state_engine.py::TestFieldValidation::test_number_field_validation` (o `min`/`max`
/// do POP Atende não existe na linguagem; o tipo é estrito).
#[test]
fn campo_de_numero_recusa_texto() {
    let ir = fluxo(
        "orcamento",
        vec![etapa(
            "orcamento",
            &[ActionActor::Agent],
            vec![campo("valor", FieldKind::Number, true)],
            vec![acao_para_fim("enviar", None, "Enviado", false)],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let aceito = executar_acao(
        &ir,
        &estado,
        "enviar",
        &campos(json!({ "valor": 1000.0 })),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("número é aceito");
    assert_eq!(
        aceito.novo_estado.dados["orcamento"]["valor"],
        json!(1000.0)
    );

    let erro = executar_acao(
        &ir,
        &estado,
        "enviar",
        &campos(json!({ "valor": "mil" })),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect_err("texto não é número");

    assert_eq!(erro.api_code(), "campos_invalidos");
    assert!(erro.to_string().contains("esperava número"), "{erro}");
}

/// Origem: `test_state_engine.py::TestFieldValidation::test_select_field_validation` — só as
/// opções declaradas, com a lista na mensagem.
#[test]
fn campo_de_opcao_recusa_valor_fora_da_lista() {
    let ir = fluxo(
        "triagem",
        vec![etapa(
            "triagem",
            &[ActionActor::Agent],
            vec![campo_opcao("periodo", &["manha", "tarde"])],
            vec![acao_para_fim("agendar", None, "Agendado", false)],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let aceito = executar_acao(
        &ir,
        &estado,
        "agendar",
        &campos(json!({ "periodo": "tarde" })),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("opção declarada é aceita");
    assert_eq!(
        aceito.novo_estado.dados["triagem"]["periodo"],
        json!("tarde")
    );

    let erro = executar_acao(
        &ir,
        &estado,
        "agendar",
        &campos(json!({ "periodo": "madrugada" })),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect_err("valor fora da lista é recusado");

    assert_eq!(erro.api_code(), "campos_invalidos");
    let mensagem = erro.to_string();
    assert!(mensagem.contains("esperava uma das opções"), "{mensagem}");
    assert!(mensagem.contains("manha"), "{mensagem}");
    assert!(mensagem.contains("tarde"), "{mensagem}");
}

/// Origem: `core/fichas/validators.py` (linhas 50–61: tipo `datetime` aceita ISO 8601 e recusa o
/// resto). Diferença registrada em `FLUXOS_PARIDADE.md`: o Harness confere **forma e faixas**, não
/// o calendário (`2026-02-30` passa).
#[test]
fn campo_de_data_aceita_iso_e_recusa_texto_livre() {
    let ir = fluxo(
        "agendamento",
        vec![etapa(
            "agendamento",
            &[ActionActor::Agent],
            vec![campo("data_hora_visita", FieldKind::Data, true)],
            vec![acao_para_fim("confirmar", None, "Confirmado", false)],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let aceito = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "data_hora_visita": "2026-03-15T14:00:00" })),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("data ISO é aceita");
    assert_eq!(
        aceito.novo_estado.dados["agendamento"]["data_hora_visita"],
        json!("2026-03-15T14:00:00")
    );

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "data_hora_visita": "amanhã de manhã" })),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect_err("texto livre não é data");

    assert_eq!(erro.api_code(), "campos_invalidos");
    assert!(
        erro.to_string().contains("esperava data ISO 8601"),
        "{erro}"
    );
}

// ---------------------------------------------------------------------------------------------
// `next_step: "closed"` → `fim(...)` (PRD §8, linha do encerramento)
// ---------------------------------------------------------------------------------------------

/// Origem: `test_state_engine.py::TestGlobalStateDerivation::test_closed_state` e o tratamento de
/// `next_step == "closed"` em `execute_action` — no Harness o `closed` virou `fim(...)`, que
/// conclui a **execução** com resultado.
#[test]
fn closed_do_pop_atende_virou_fim_que_conclui_a_execucao() {
    let ir = fluxo(
        "fechamento",
        vec![etapa(
            "fechamento",
            &[ActionActor::Agent],
            Vec::new(),
            vec![acao_para_fim(
                "concluir",
                None,
                "Atendimento concluído",
                false,
            )],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "concluir",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.novo_estado.status, ExecutionStatus::Completed);
    assert_eq!(
        resultado.novo_estado.resultado.as_deref(),
        Some("Atendimento concluído")
    );
    assert_eq!(resultado.novo_estado.motivo, None);
    assert_eq!(
        tipos(&resultado),
        vec![
            RecordEventKind::ActionExecuted,
            RecordEventKind::StepCompleted,
            RecordEventKind::ExecutionCompleted
        ]
    );
}

/// Origem: PRD §8, linha do encerramento — antes, `next_step: "closed"` fechava a Ficha; agora o
/// `fim(...)` conclui a execução e **fechar o atendimento é decisão explícita**
/// (`fim(encerra_atendimento = True)`).
#[test]
fn fim_so_pede_o_fechamento_do_atendimento_quando_declara_isso() {
    let ir = fluxo(
        "fechamento",
        vec![etapa(
            "fechamento",
            &[ActionActor::Agent],
            Vec::new(),
            vec![
                acao_para_fim("concluir", None, "Concluído", false),
                acao_para_fim("encerrar_tudo", None, "Encerrado", true),
            ],
        )],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let sem_encerrar = executar_acao(
        &ir,
        &estado,
        "concluir",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");
    assert_eq!(sem_encerrar.novo_estado.status, ExecutionStatus::Completed);
    assert!(
        !sem_encerrar.encerra_atendimento,
        "concluir o fluxo não fecha o atendimento"
    );

    let encerrando = executar_acao(
        &ir,
        &estado,
        "encerrar_tudo",
        &sem_campos(),
        ActionActor::Agent,
        Some("k2"),
        &contexto(),
    )
    .expect("a ação é aceita");
    assert_eq!(encerrando.novo_estado.status, ExecutionStatus::Completed);
    assert!(
        encerrando.encerra_atendimento,
        "o fim que declara o encerramento pede o fechamento ao chamador"
    );
}

/// Origem: `test_completion_step.py` (`find_completion_step_key`) — o POP Atende descobria a etapa
/// de fechamento **pelo nome** dela ("Encerrado", "Fechamento", "FIM"); no Harness o encerramento é
/// declarado no `fim(...)`, então uma etapa chamada "Encerrado" não encerra nada.
#[test]
fn o_encerramento_e_declarado_e_nao_deduzido_pelo_nome_da_etapa() {
    let mut encerrado = etapa(
        "encerrado",
        &[ActionActor::Agent],
        Vec::new(),
        vec![
            acao_para_etapa("reabrir", None, &[], "abertura"),
            acao_para_fim("concluir", None, "Concluído", false),
        ],
    );
    encerrado.nome = "Encerrado".to_owned();
    let ir = fluxo(
        "abertura",
        vec![
            etapa(
                "abertura",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_etapa("seguir", None, &[], "encerrado")],
            ),
            encerrado,
        ],
        Vec::new(),
    );
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "seguir",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto(),
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.novo_estado.no_atual, vai_para("encerrado"));
    assert_eq!(
        resultado.novo_estado.status,
        ExecutionStatus::Active,
        "chegar numa etapa chamada `Encerrado` não encerra nada"
    );
    assert!(!resultado.encerra_atendimento);
}

// ---------------------------------------------------------------------------------------------
// Atores (PRD §8, linha dos atores)
// ---------------------------------------------------------------------------------------------

/// Origem: PRD §8, linha dos atores — `client`, `attendant`, `agent`, `system`, `provider` e
/// `operator` viraram `agente` e `atendente`. O cliente nunca é ator de ação (decisão D5).
#[test]
fn so_existem_dois_atores_de_acao() {
    for aceito in ["agente", "atendente"] {
        assert!(
            ActionActor::from_text(aceito).is_ok(),
            "`{aceito}` é ator de ação"
        );
    }
    for recusado in [
        "cliente", "client", "provider", "operator", "system", "sistema",
    ] {
        let erro =
            ActionActor::from_text(recusado).expect_err("ator do POP Atende que o Harness não tem");
        assert_eq!(erro.api_code(), "validation_error");
        assert!(
            erro.to_string().contains("aceitos: agente, atendente"),
            "`{recusado}`: {erro}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Efeitos (PRD §8, linha dos nós "satélite")
// ---------------------------------------------------------------------------------------------

/// Origem: PRD §8, linha da notificação/encaminhamento (`notifications`/`forwards` do POP Atende,
/// `test_department_forward.py`) — deixaram de ser nós ligados por arestas e viraram **efeitos**
/// dentro da ação.
#[test]
fn notificacao_e_encaminhamento_saem_como_efeito_da_acao() {
    let mut avisar = acao_para_fim("concluir", None, "Concluído", false);
    avisar.efeitos = vec![
        Efeito::NotificarWhatsapp {
            para: "5521999990000".to_owned(),
            mensagem: "O atendimento do cliente {cliente.peer} foi concluído".to_owned(),
        },
        Efeito::Encaminhar {
            departamento: "financeiro".to_owned(),
        },
    ];
    let ir = fluxo(
        "coleta",
        vec![etapa(
            "coleta",
            &[ActionActor::Agent],
            Vec::new(),
            vec![avisar],
        )],
        Vec::new(),
    );
    let avaliador = Avaliador::sem_funcoes();
    let contexto = Contexto::novo(&avaliador).com_cliente("cli_ficticio");
    let estado = iniciar(&ir).expect("a execução começa");

    let resultado = executar_acao(
        &ir,
        &estado,
        "concluir",
        &sem_campos(),
        ActionActor::Agent,
        Some("k1"),
        &contexto,
    )
    .expect("a ação é aceita");

    assert_eq!(resultado.efeitos.len(), 2);
    assert_eq!(resultado.efeitos[0].chave, "acao:coleta.concluir/0");
    assert_eq!(resultado.efeitos[0].tipo(), "notificar_whatsapp");
    assert_eq!(
        resultado.efeitos[0].efeito,
        Efeito::NotificarWhatsapp {
            para: "5521999990000".to_owned(),
            mensagem: "O atendimento do cliente cli_ficticio foi concluído".to_owned(),
        }
    );
    assert_eq!(resultado.efeitos[1].chave, "acao:coleta.concluir/1");
    assert_eq!(resultado.efeitos[1].tipo(), "encaminhar");
    assert_eq!(
        resultado.efeitos[1].efeito,
        Efeito::Encaminhar {
            departamento: "financeiro".to_owned(),
        }
    );
    let criados = tipos(&resultado)
        .into_iter()
        .filter(|tipo| *tipo == RecordEventKind::EffectCreated)
        .count();
    assert_eq!(criados, 2);
}

/// Origem: `state_engine` / templates do POP Atende (a mensagem de entrada do step, `messages`) —
/// a etapa que fala com o cliente ao ser entrada continua existindo, agora como `ao_entrar`.
#[test]
fn mensagem_de_entrada_da_etapa_sai_ao_iniciar_a_execucao() {
    let ir = fluxo(
        "abertura",
        vec![
            etapa_com_entrada(
                "abertura",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
                vec![Efeito::MensagemAoCliente {
                    texto: "Olá, {cliente.peer}! Já vou te atender.".to_owned(),
                }],
            ),
            etapa(
                "fechamento",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_fim("fechar", None, "Feito", false)],
            ),
        ],
        Vec::new(),
    );
    let avaliador = Avaliador::sem_funcoes();
    let contexto = Contexto::novo(&avaliador).com_cliente("cli_ficticio");
    let estado = iniciar(&ir).expect("a execução começa");

    let efeitos =
        efeitos_de_entrada(&ir, "abertura", &estado.dados, &contexto).expect("a etapa existe");

    assert_eq!(efeitos.len(), 1);
    assert_eq!(efeitos[0].chave, "etapa:abertura/0");
    assert_eq!(
        efeitos[0].efeito,
        Efeito::MensagemAoCliente {
            texto: "Olá, cli_ficticio! Já vou te atender.".to_owned(),
        }
    );
}

// ---------------------------------------------------------------------------------------------
// Variável inexistente no texto (PRD §8, linha da variável inexistente)
// ---------------------------------------------------------------------------------------------

/// Origem: PRD §8, linha da variável inexistente — antes ela virava vazio **em silêncio**; agora
/// vira vazio **e a publicação avisa** (`FLX59`, provado em `tests/compilador.rs`). O efeito sai
/// mesmo assim.
#[test]
fn variavel_inexistente_no_texto_sai_vazia() {
    let ir = fluxo(
        "triagem",
        vec![etapa_com_entrada(
            "triagem",
            &[ActionActor::Agent],
            Vec::new(),
            vec![acao_para_fim("fechar", None, "Feito", false)],
            vec![
                Efeito::MensagemAoCliente {
                    texto: "Endereço registrado: {triagem.cep}.".to_owned(),
                },
                Efeito::MensagemAoCliente {
                    texto: "Chave sem par: {triagem.cep".to_owned(),
                },
            ],
        )],
        Vec::new(),
    );
    let avaliador = Avaliador::sem_funcoes();
    let contexto = Contexto::novo(&avaliador);
    let estado = iniciar(&ir).expect("a execução começa");

    let efeitos =
        efeitos_de_entrada(&ir, "triagem", &estado.dados, &contexto).expect("a etapa existe");

    assert_eq!(
        efeitos[0].efeito,
        Efeito::MensagemAoCliente {
            texto: "Endereço registrado: .".to_owned(),
        },
        "a variável que não existe sai vazia, sem quebrar o texto"
    );
    assert_eq!(
        efeitos[1].efeito,
        Efeito::MensagemAoCliente {
            texto: "Chave sem par: {triagem.cep".to_owned(),
        },
        "`{{` sem fechamento é texto do autor, não variável"
    );
}

// ---------------------------------------------------------------------------------------------
// Início do fluxo (PRD §8, linha da etapa inicial)
// ---------------------------------------------------------------------------------------------

/// Origem: PRD §8, linha da etapa inicial — antes a primeira etapa era marcada com
/// `is_entry: true` no JSON (e esquecer a marca quebrava o fluxo); agora o `inicio` do fluxo
/// aponta a etapa por id, e a ordem de declaração não decide nada.
#[test]
fn inicio_e_declarado_no_fluxo_e_nao_marcado_na_etapa() {
    let ir = fluxo(
        "meio",
        vec![
            etapa(
                "primeira_nao_e_a_de_entrada",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_etapa("ir", None, &[], "meio")],
            ),
            etapa(
                "meio",
                &[ActionActor::Agent],
                Vec::new(),
                vec![acao_para_etapa(
                    "ir",
                    None,
                    &[],
                    "primeira_nao_e_a_de_entrada",
                )],
            ),
        ],
        Vec::new(),
    );

    let estado = iniciar(&ir).expect("a execução começa");

    assert_eq!(estado.no_atual, vai_para("meio"));
    assert_eq!(estado.status, ExecutionStatus::Active);
    assert!(estado.dados.is_empty());
    assert!(estado.atores_que_agiram.is_empty());
}
