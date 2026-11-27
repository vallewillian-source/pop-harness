//! Testes da avaliação de regras (FX-12).
//!
//! Cada combinador tem o seu caso, e cada `def` que pode dar errado tem o desfecho tipado: a regra
//! responde `regra_invalida` quando o defeito é do fluxo e `regra_limite` quando é custo. O abuso
//! (`"x" * 1000000000`, recursão, laço) termina dentro do prazo, sem travar o processo.

use std::time::{Duration, Instant};

use poph_fluxos::compilador::Limites;
use poph_fluxos::ir::Rule;
use poph_fluxos::regras::{
    DEADLINE_TOLERANCE, Dados, MAX_NESTING, RULE_DEADLINE, RuleError, RuleEvaluator, RuleLimits,
    evaluate, snapshot_from_json,
};
use poph_types::DomainError;

/// Script de apoio: um fluxo mínimo e as `def` que os testes exercitam.
///
/// Só carrega; nada aqui é executado na carga (o corpo da `def` roda quando a regra é avaliada).
const SCRIPT: &str = r#"
fluxo(
    id = "teste_de_regras",
    nome = "Teste das regras",
    inicio = "relato",
    gatilho = gatilho(quando = "o cliente relata um problema", exemplos = ["deu erro"]),
)

etapa(
    id = "relato",
    nome = "Relato",
    campos = [campo("resumo", "O que aconteceu", tipo = "texto")],
    acoes = [acao("ok", "Registrado", vai_para = fim("concluido"))],
)

def grave(dados):
    resumo = dados.get("relato", {}).get("resumo", "").lower()
    return "parado" in resumo

def viu_anexo(dados):
    return dados.get("relato", {}).get("anexo") != None

def devolve_texto(dados):
    return "sim"

def devolve_nada(dados):
    pass

def estoura_ticks(dados):
    total = 0
    for i in range(200000):
        total = total + i
    return total > 0

def le_ausente(dados):
    return dados["relato"]["nao_existe"] == 1

def recursiva(dados):
    return recursiva(dados)

def aloca_gigante(dados):
    # O tamanho vem dos dados, não do script: multiplicação de **constantes** é feita na carga do
    # script (e o prazo do Starlark só a percebe depois); com o valor vindo dos dados, o custo cai
    # na avaliação da regra, que é o que o prazo de 200 ms protege.
    return len("x" * int(dados["relato"]["n"])) > 0

def ecoa(dados):
    return dados["relato"]["resumo"] == "__import__('os').system('ls')"
"#;

fn dados(json: &str) -> Dados {
    snapshot_from_json(json).expect("dados de teste válidos")
}

fn avaliador() -> RuleEvaluator {
    RuleEvaluator::new(SCRIPT).expect("o script de apoio carrega")
}

fn codigo(erro: &RuleError) -> &'static str {
    erro.api_code()
}

// -------------------------------------------------------------------------------------------
// Combinadores
// -------------------------------------------------------------------------------------------

#[test]
fn igual_compara_texto_numero_booleano_e_anexo() {
    let base = dados(
        r#"{"relato": {"nome": "  Maria  ", "idade": 42, "aceitou": true, "outro": false,
                        "periodo": "tarde", "anexo": {"nome": "foto.jpg", "tipo": "image/jpeg"}}}"#,
    );
    let casos = [
        // texto: tira espaço das pontas
        (
            Rule::Igual {
                campo: "relato.nome".into(),
                value: "Maria".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.nome".into(),
                value: " maria ".into(),
            },
            false,
        ),
        // número: comparação em texto canônico
        (
            Rule::Igual {
                campo: "relato.idade".into(),
                value: "42".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.idade".into(),
                value: "42.0".into(),
            },
            false,
        ),
        // booleano: sim/nao, true/false e 1/0 são a mesma coisa (§5)
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "sim".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "true".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "1".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.outro".into(),
                value: "nao".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.outro".into(),
                value: "0".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "nao".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "talvez".into(),
            },
            false,
        ),
        // opção: o id da opção
        (
            Rule::Igual {
                campo: "relato.periodo".into(),
                value: "tarde".into(),
            },
            true,
        ),
        (
            Rule::Igual {
                campo: "relato.periodo".into(),
                value: "Tarde".into(),
            },
            false,
        ),
        // anexo: o nome do arquivo
        (
            Rule::Igual {
                campo: "relato.anexo".into(),
                value: "foto.jpg".into(),
            },
            true,
        ),
    ];
    for (regra, esperado) in casos {
        assert_eq!(
            evaluate(&regra, &base).unwrap(),
            esperado,
            "regra {regra:?}"
        );
    }
}

#[test]
fn diferente_e_o_inverso_de_igual() {
    let base = dados(r#"{"relato": {"periodo": "manha"}}"#);
    assert!(
        evaluate(
            &Rule::Diferente {
                campo: "relato.periodo".into(),
                value: "tarde".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Diferente {
                campo: "relato.periodo".into(),
                value: "manha".into()
            },
            &base
        )
        .unwrap()
    );
}

#[test]
fn maior_e_menor_comparam_numero_e_texto_numerico() {
    let base =
        dados(r#"{"coleta": {"dias": 12, "dias_texto": "12", "nome": "doze", "nulo": null}}"#);
    // número × número
    assert!(
        evaluate(
            &Rule::Maior {
                campo: "coleta.dias".into(),
                value: "10".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Menor {
                campo: "coleta.dias".into(),
                value: "10".into()
            },
            &base
        )
        .unwrap()
    );
    // texto que é número compara pelo número: "12" > "9", o que não aconteceria em texto
    assert!(
        evaluate(
            &Rule::Maior {
                campo: "coleta.dias_texto".into(),
                value: "9".into()
            },
            &base
        )
        .unwrap()
    );
    // texto que não é número: falso, sem erro (é o FLX60 avisado na publicação)
    assert!(
        !evaluate(
            &Rule::Maior {
                campo: "coleta.nome".into(),
                value: "9".into()
            },
            &base
        )
        .unwrap()
    );
    // nulo: falso
    assert!(
        !evaluate(
            &Rule::Maior {
                campo: "coleta.nulo".into(),
                value: "9".into()
            },
            &base
        )
        .unwrap()
    );
    // decimal
    let decimal = dados(r#"{"coleta": {"dias": 10.5}}"#);
    assert!(
        evaluate(
            &Rule::Maior {
                campo: "coleta.dias".into(),
                value: "10.25".into()
            },
            &decimal
        )
        .unwrap()
    );
}

#[test]
fn comparar_com_literal_que_nao_e_numero_e_regra_invalida() {
    let base = dados(r#"{"coleta": {"dias": 12}}"#);
    let erro = evaluate(
        &Rule::Maior {
            campo: "coleta.dias".into(),
            value: "dez".into(),
        },
        &base,
    )
    .expect_err("literal não numérico tem de ser recusado");
    assert_eq!(codigo(&erro), "regra_invalida");
    assert!(erro.to_string().contains("dez"), "{erro}");
}

#[test]
fn contem_ignora_caixa_e_acento_em_texto_e_em_lista() {
    let base = dados(
        r#"{"relato": {"resumo": "O sistema NÃO ABRE desde ontem", "tags": ["urgente", "Fatura"]}}"#,
    );
    // texto, com acento e maiúscula do outro lado
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "nao abre".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "NÃO".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "fechou".into()
            },
            &base
        )
        .unwrap()
    );
    // lista: qualquer item que contenha o trecho
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.tags".into(),
                value: "urgen".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.tags".into(),
                value: "fatura".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !evaluate(
            &Rule::Contem {
                campo: "relato.tags".into(),
                value: "cobranca".into()
            },
            &base
        )
        .unwrap()
    );
}

#[test]
fn vazio_cobre_ausente_nulo_espacos_e_lista() {
    let base = dados(
        r#"{"relato": {"vazio": "", "espacos": "   ", "nulo": null, "lista": [], "numero": 0, "texto": "x"}}"#,
    );
    let casos = [
        ("relato.vazio", true),
        ("relato.espacos", true),
        ("relato.nulo", true),
        ("relato.lista", true),
        ("relato.numero", false),
        ("relato.texto", false),
        ("relato.nao_existe", true),
    ];
    for (campo, esperado) in casos {
        assert_eq!(
            evaluate(
                &Rule::Vazio {
                    campo: campo.into()
                },
                &base
            )
            .unwrap(),
            esperado,
            "campo {campo}"
        );
    }
}

#[test]
fn e_ou_nao_aninhados() {
    let base =
        dados(r#"{"relato": {"afeta_agora": "sim", "resumo": "está parado", "aceitou": true}}"#);
    let afeta = Rule::Igual {
        campo: "relato.afeta_agora".into(),
        value: "sim".into(),
    };
    let vazio = Rule::Vazio {
        campo: "relato.resumo".into(),
    };
    let parado = Rule::Contem {
        campo: "relato.resumo".into(),
        value: "parado".into(),
    };
    let aceitou = Rule::Igual {
        campo: "relato.aceitou".into(),
        value: "sim".into(),
    };

    // e(igual, nao(vazio), ou(contem, igual)) — a regra do exemplo do documento
    let regra = Rule::E {
        args: vec![
            afeta,
            Rule::Nao { args: vec![vazio] },
            Rule::Ou {
                args: vec![parado, aceitou],
            },
        ],
    };
    assert!(evaluate(&regra, &base).unwrap());

    // e() com uma condição falsa derruba o conjunto
    let falso = Rule::E {
        args: vec![
            Rule::Igual {
                campo: "relato.resumo".into(),
                value: "outro".into(),
            },
            Rule::Vazio {
                campo: "relato.resumo".into(),
            },
        ],
    };
    assert!(!evaluate(&falso, &base).unwrap());

    // ou() com todas falsas
    let nenhuma = Rule::Ou {
        args: vec![
            Rule::Vazio {
                campo: "relato.resumo".into(),
            },
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "nao".into(),
            },
        ],
    };
    assert!(!evaluate(&nenhuma, &base).unwrap());

    // nao() com dois argumentos é regra malformada
    let malformada = Rule::Nao {
        args: vec![
            Rule::Vazio {
                campo: "relato.resumo".into(),
            },
            Rule::Vazio {
                campo: "relato.aceitou".into(),
            },
        ],
    };
    assert_eq!(
        codigo(&evaluate(&malformada, &base).unwrap_err()),
        "regra_invalida"
    );
}

#[test]
fn campo_ausente_segue_a_tabela_do_documento() {
    let base = dados(r#"{"relato": {}}"#);
    let casos = [
        (
            Rule::Igual {
                campo: "relato.x".into(),
                value: "y".into(),
            },
            false,
        ),
        (
            Rule::Igual {
                campo: "relato.x".into(),
                value: "".into(),
            },
            false,
        ),
        (
            Rule::Diferente {
                campo: "relato.x".into(),
                value: "y".into(),
            },
            true,
        ),
        (
            Rule::Diferente {
                campo: "relato.x".into(),
                value: "".into(),
            },
            true,
        ),
        (
            Rule::Maior {
                campo: "relato.x".into(),
                value: "1".into(),
            },
            false,
        ),
        (
            Rule::Menor {
                campo: "relato.x".into(),
                value: "1".into(),
            },
            false,
        ),
        (
            Rule::Contem {
                campo: "relato.x".into(),
                value: "".into(),
            },
            false,
        ),
        (
            Rule::Vazio {
                campo: "relato.x".into(),
            },
            true,
        ),
    ];
    for (regra, esperado) in casos {
        assert_eq!(
            evaluate(&regra, &base).unwrap(),
            esperado,
            "regra {regra:?}"
        );
    }
}

#[test]
fn regra_aninhada_alem_do_teto_e_recusada_sem_derrubar_a_pilha() {
    let mut regra = Rule::Vazio {
        campo: "relato.x".into(),
    };
    for _ in 0..(MAX_NESTING + 5) {
        regra = Rule::Nao { args: vec![regra] };
    }
    let erro = evaluate(&regra, &dados(r#"{"relato": {"x": "1"}}"#))
        .expect_err("aninhamento acima do teto tem de ser recusado");
    assert_eq!(codigo(&erro), "regra_invalida");
    assert!(erro.to_string().contains("aninhamento"), "{erro}");
}

// -------------------------------------------------------------------------------------------
// `def` (funções avançadas)
// -------------------------------------------------------------------------------------------

#[test]
fn funcao_le_os_dados_e_decide() {
    let avaliador = avaliador();
    let urgente = dados(r#"{"relato": {"resumo": "o sistema está PARADO"}}"#);
    let normal = dados(r#"{"relato": {"resumo": "gostaria de um orçamento"}}"#);
    let regra = Rule::Funcao {
        nome: "grave".into(),
    };
    assert!(avaliador.evaluate(&regra, &urgente).unwrap());
    assert!(!avaliador.evaluate(&regra, &normal).unwrap());
}

#[test]
fn funcao_recebe_o_dicionario_inteiro_inclusive_anexo_e_nulo() {
    let avaliador = avaliador();
    let regra = Rule::Funcao {
        nome: "viu_anexo".into(),
    };
    assert!(
        avaliador
            .evaluate(
                &regra,
                &dados(r#"{"relato": {"anexo": {"nome": "nota.pdf"}}}"#)
            )
            .unwrap()
    );
    assert!(
        !avaliador
            .evaluate(&regra, &dados(r#"{"relato": {"anexo": null}}"#))
            .unwrap()
    );
    assert!(!avaliador.evaluate(&regra, &dados("{}")).unwrap());
}

#[test]
fn funcao_que_devolve_texto_e_regra_invalida() {
    let avaliador = avaliador();
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "devolve_texto".into(),
            },
            &dados("{}"),
        )
        .expect_err("devolver texto tem de ser erro");
    assert_eq!(codigo(&erro), "regra_invalida");
    assert!(erro.to_string().contains("True"), "{erro}");
}

#[test]
fn funcao_que_devolve_none_e_regra_invalida() {
    let avaliador = avaliador();
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "devolve_nada".into(),
            },
            &dados("{}"),
        )
        .expect_err("devolver None tem de ser erro");
    assert_eq!(codigo(&erro), "regra_invalida");
}

#[test]
fn funcao_que_le_dado_inexistente_falha_sem_panico() {
    let avaliador = avaliador();
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "le_ausente".into(),
            },
            &dados(r#"{"relato": {}}"#),
        )
        .expect_err("chave ausente tem de virar erro tipado");
    assert_eq!(codigo(&erro), "regra_invalida");
}

#[test]
fn funcao_que_estoura_ticks_e_regra_limite() {
    let avaliador = avaliador();
    let inicio = Instant::now();
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "estoura_ticks".into(),
            },
            &dados("{}"),
        )
        .expect_err("laço longo tem de ser cortado");
    let gasto = inicio.elapsed();
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
    assert!(gasto < RULE_DEADLINE, "levou {gasto:?}");
}

#[test]
fn funcao_recursiva_e_regra_limite() {
    let avaliador = avaliador();
    let inicio = Instant::now();
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "recursiva".into(),
            },
            &dados("{}"),
        )
        .expect_err("recursão tem de ser cortada");
    let gasto = inicio.elapsed();
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
    assert!(gasto < RULE_DEADLINE, "levou {gasto:?}");
}

#[test]
fn funcao_que_aloca_gigante_responde_no_prazo() {
    // O caso que a FX-10 deixou aberto: o Starlark confere o heap *depois* de alocar, então uma
    // alocação única e enorme não é cortada pelas conferências dele. Quem corta é o prazo: a `def`
    // roda em thread própria e o chamador recebe o erro tipado no prazo combinado. O tamanho entra
    // pelos dados, porque constante escrita no script é calculada na carga, não na avaliação.
    let avaliador = avaliador();
    for n in [100_000_000_u64, 1_000_000_000_u64] {
        let base = dados(&format!(r#"{{"relato": {{"n": {n}}}}}"#));
        let inicio = Instant::now();
        let erro = avaliador
            .evaluate(
                &Rule::Funcao {
                    nome: "aloca_gigante".into(),
                },
                &base,
            )
            .expect_err("alocação gigante tem de ser cortada");
        let gasto = inicio.elapsed();
        assert_eq!(codigo(&erro), "regra_limite", "n={n}: {erro}");
        assert!(
            gasto <= RULE_DEADLINE + DEADLINE_TOLERANCE + Duration::from_millis(250),
            "n={n}: a regra respondeu em {gasto:?}, acima do prazo"
        );
    }
}

#[test]
fn carga_de_script_com_constante_gigante_responde_no_prazo() {
    // Cálculo entre **constantes** dentro de uma `def` é feito quando o script é compilado, e o
    // Starlark só percebe o prazo segundos depois (medido: ~4 s para 10⁸). A carga roda em thread
    // própria, então o chamador recebe `regra_limite` no prazo da carga em vez de ficar travado.
    let fonte = r#"
fluxo(
    id = "constante_gigante",
    nome = "Constante gigante",
    inicio = "relato",
    gatilho = gatilho(quando = "o cliente relata", nao_quando = "o cliente agradece", exemplos = ["deu erro"]),
)

etapa(
    id = "relato",
    nome = "Relato",
    campos = [campo("resumo", "O que aconteceu", tipo = "texto")],
    acoes = [acao("ok", "Registrado", vai_para = fim("concluido"))],
)

def aloca_gigante(dados):
    return len("x" * 100000000) > 0
"#;
    let inicio = Instant::now();
    let erro =
        RuleEvaluator::new(fonte).expect_err("carga com constante gigante tem de ser cortada");
    let gasto = inicio.elapsed();
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
    assert!(
        gasto <= Duration::from_secs(2),
        "a carga respondeu em {gasto:?}, acima do prazo da carga mais a margem"
    );
}

#[test]
fn limites_da_carga_viram_regra_limite() {
    // Heap de 1 byte: o script não carrega por limite, não por defeito de autor.
    let apertado = Limites {
        max_heap_bytes: 1,
        ..Limites::default()
    };
    let erro = RuleEvaluator::new_with(SCRIPT, &apertado, RuleLimits::default())
        .expect_err("heap de 1 byte não carrega o script");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
}

#[test]
fn funcao_sem_def_e_regra_invalida() {
    let avaliador = avaliador();
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "nao_existe".into(),
            },
            &dados("{}"),
        )
        .expect_err("def ausente tem de ser erro");
    assert_eq!(codigo(&erro), "regra_invalida");
    assert!(erro.to_string().contains("nao_existe"), "{erro}");
}

#[test]
fn funcao_sem_modulo_carregado_e_regra_invalida() {
    let erro = evaluate(
        &Rule::Funcao {
            nome: "grave".into(),
        },
        &dados("{}"),
    )
    .expect_err("sem módulo não há como rodar def");
    assert_eq!(codigo(&erro), "regra_invalida");
    assert!(erro.to_string().contains("Avaliador::novo"), "{erro}");
}

#[test]
fn os_limites_da_regra_estao_ligados_de_verdade() {
    // Orçamento de ticks apertado: o laço de 200.000 voltas não cabe em 100 ticks. (Uma `def`
    // trivial como `return True` cabe em 1 tick — o limite é de trabalho, não de chamada.)
    let apertado = RuleLimits {
        max_ticks: 100,
        ..RuleLimits::default()
    };
    let avaliador = RuleEvaluator::new_with(SCRIPT, &Limites::default(), apertado)
        .expect("o script carrega com os limites de carga");
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "estoura_ticks".into(),
            },
            &dados("{}"),
        )
        .expect_err("o laço não cabe em 100 ticks");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");

    // Heap apertado: a `def` não consegue nem montar o dicionário dos dados.
    let sem_heap = RuleLimits {
        max_heap_bytes: 1,
        ..RuleLimits::default()
    };
    let avaliador =
        RuleEvaluator::new_with(SCRIPT, &Limites::default(), sem_heap).expect("carrega");
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "grave".into(),
            },
            &dados("{}"),
        )
        .expect_err("heap de 1 byte não dá para nada");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");

    // Profundidade de chamada: a recursão bate no limite de pilha da regra (não no padrão).
    let sem_pilha = RuleLimits {
        max_callstack: 2,
        ..RuleLimits::default()
    };
    let avaliador =
        RuleEvaluator::new_with(SCRIPT, &Limites::default(), sem_pilha).expect("carrega");
    let erro = avaliador
        .evaluate(
            &Rule::Funcao {
                nome: "recursiva".into(),
            },
            &dados("{}"),
        )
        .expect_err("recursão com pilha de 2 tem de ser cortada");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
}

#[test]
fn script_que_nao_carrega_e_regra_invalida() {
    let erro = RuleEvaluator::new("fluxo(\n  id = \n").expect_err("sintaxe quebrada não carrega");
    assert_eq!(codigo(&erro), "regra_invalida");
}

#[test]
fn o_dado_do_cliente_entra_como_valor_e_nunca_como_codigo() {
    // O texto do cliente contendo código Starlark continua sendo texto: a `def` compara com a
    // string e o resultado é verdadeiro porque nada foi interpretado.
    let avaliador = avaliador();
    let base = dados(r#"{"relato": {"resumo": "__import__('os').system('ls')"}}"#);
    assert!(
        avaliador
            .evaluate(
                &Rule::Funcao {
                    nome: "ecoa".into()
                },
                &base
            )
            .unwrap()
    );
    // E os combinadores tratam igual: o trecho é procurado no texto, não avaliado.
    assert!(
        evaluate(
            &Rule::Contem {
                campo: "relato.resumo".into(),
                value: "system('ls')".into()
            },
            &base
        )
        .unwrap()
    );
    // Nome de declaração no dado também não tem poder nenhum.
    let perigoso = dados(r#"{"relato": {"resumo": "load(\"x.star\", \"y\")"}}"#);
    assert!(
        avaliador
            .evaluate(
                &Rule::Funcao {
                    nome: "ecoa".into()
                },
                &perigoso
            )
            .is_ok()
    );
}

// -------------------------------------------------------------------------------------------
// Determinismo e custo
// -------------------------------------------------------------------------------------------

#[test]
fn a_mesma_regra_com_os_mesmos_dados_da_o_mesmo_resultado() {
    let avaliador = avaliador();
    let base = dados(r#"{"relato": {"resumo": "está parado", "aceitou": true}}"#);
    let regra = Rule::E {
        args: vec![
            Rule::Funcao {
                nome: "grave".into(),
            },
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "sim".into(),
            },
        ],
    };
    let primeiro = avaliador.evaluate(&regra, &base).unwrap();
    for _ in 0..5 {
        assert_eq!(avaliador.evaluate(&regra, &base).unwrap(), primeiro);
    }
}

#[test]
fn p95_dos_combinadores_fica_dentro_da_meta() {
    let base = dados(r#"{"relato": {"resumo": "está parado", "aceitou": true, "dias": 12}}"#);
    let regra = Rule::E {
        args: vec![
            Rule::Igual {
                campo: "relato.aceitou".into(),
                value: "sim".into(),
            },
            Rule::Contem {
                campo: "relato.resumo".into(),
                value: "parado".into(),
            },
            Rule::Ou {
                args: vec![
                    Rule::Maior {
                        campo: "relato.dias".into(),
                        value: "10".into(),
                    },
                    Rule::Vazio {
                        campo: "relato.resumo".into(),
                    },
                ],
            },
        ],
    };
    // Aquecimento.
    for _ in 0..1000 {
        assert!(evaluate(&regra, &base).unwrap());
    }
    let mut tempos = Vec::with_capacity(20_000);
    for _ in 0..20_000 {
        let inicio = Instant::now();
        let _ = evaluate(&regra, &base).unwrap();
        tempos.push(inicio.elapsed());
    }
    tempos.sort_unstable();
    let p50 = tempos[tempos.len() / 2];
    let p95 = tempos[tempos.len() * 95 / 100];
    println!("combinadores: p50 {p50:?}, p95 {p95:?} (build de teste)");
    // A meta de 20 µs p95 é medida em `--release` (relatório da FX-12). No build de teste, com
    // verificações e sem otimização, o teto aceito é o do PRD original (200 µs).
    assert!(p95 < Duration::from_micros(200), "p95 {p95:?}");
}

#[test]
fn p95_da_funcao_fica_dentro_do_prazo() {
    let avaliador = avaliador();
    let base = dados(r#"{"relato": {"resumo": "está parado"}}"#);
    let regra = Rule::Funcao {
        nome: "grave".into(),
    };
    for _ in 0..50 {
        assert!(avaliador.evaluate(&regra, &base).unwrap());
    }
    let mut tempos = Vec::with_capacity(300);
    for _ in 0..300 {
        let inicio = Instant::now();
        let _ = avaliador.evaluate(&regra, &base).unwrap();
        tempos.push(inicio.elapsed());
    }
    tempos.sort_unstable();
    let p50 = tempos[tempos.len() / 2];
    let p95 = tempos[tempos.len() * 95 / 100];
    println!("função (com thread e prazo): p50 {p50:?}, p95 {p95:?} (build de teste)");
    assert!(p95 < RULE_DEADLINE, "p95 {p95:?}");
}

#[test]
fn cada_erro_de_regra_tem_codigo_de_api() {
    let base = dados(r#"{"relato": {}}"#);
    let invalida = evaluate(
        &Rule::Maior {
            campo: "relato.x".into(),
            value: "abc".into(),
        },
        &base,
    )
    .unwrap_err();
    assert_eq!(invalida.api_code(), "regra_invalida");
    assert_eq!(invalida.http_status(), 400);

    let limite_erro = DomainError::RuleLimit {
        reason: "a função passou do prazo".to_owned(),
    };
    assert_eq!(limite_erro.api_code(), "regra_limite");
    assert_eq!(limite_erro.http_status(), 400);
}
