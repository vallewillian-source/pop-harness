//! Testes da avaliação de regras (FX-12).
//!
//! Cada combinador tem o seu caso, e cada `def` que pode dar errado tem o desfecho tipado: a regra
//! responde `regra_invalida` quando o defeito é do fluxo e `regra_limite` quando é custo. O abuso
//! (`"x" * 1000000000`, recursão, laço) termina dentro do prazo, sem travar o processo.

use std::time::{Duration, Instant};

use poph_fluxos::compilador::Limites;
use poph_fluxos::ir::Regra;
use poph_fluxos::regras::{
    Avaliador, Dados, ErroRegra, LimitesRegra, MAX_PROFUNDIDADE, PRAZO_DA_REGRA,
    TOLERANCIA_DO_PRAZO, avaliar, dados_de_json,
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
    dados_de_json(json).expect("dados de teste válidos")
}

fn avaliador() -> Avaliador {
    Avaliador::novo(SCRIPT).expect("o script de apoio carrega")
}

fn codigo(erro: &ErroRegra) -> &'static str {
    erro.codigo_api()
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
            Regra::Igual {
                campo: "relato.nome".into(),
                valor: "Maria".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.nome".into(),
                valor: " maria ".into(),
            },
            false,
        ),
        // número: comparação em texto canônico
        (
            Regra::Igual {
                campo: "relato.idade".into(),
                valor: "42".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.idade".into(),
                valor: "42.0".into(),
            },
            false,
        ),
        // booleano: sim/nao, true/false e 1/0 são a mesma coisa (§5)
        (
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "sim".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "true".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "1".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.outro".into(),
                valor: "nao".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.outro".into(),
                valor: "0".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "nao".into(),
            },
            false,
        ),
        (
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "talvez".into(),
            },
            false,
        ),
        // opção: o id da opção
        (
            Regra::Igual {
                campo: "relato.periodo".into(),
                valor: "tarde".into(),
            },
            true,
        ),
        (
            Regra::Igual {
                campo: "relato.periodo".into(),
                valor: "Tarde".into(),
            },
            false,
        ),
        // anexo: o nome do arquivo
        (
            Regra::Igual {
                campo: "relato.anexo".into(),
                valor: "foto.jpg".into(),
            },
            true,
        ),
    ];
    for (regra, esperado) in casos {
        assert_eq!(avaliar(&regra, &base).unwrap(), esperado, "regra {regra:?}");
    }
}

#[test]
fn diferente_e_o_inverso_de_igual() {
    let base = dados(r#"{"relato": {"periodo": "manha"}}"#);
    assert!(
        avaliar(
            &Regra::Diferente {
                campo: "relato.periodo".into(),
                valor: "tarde".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !avaliar(
            &Regra::Diferente {
                campo: "relato.periodo".into(),
                valor: "manha".into()
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
        avaliar(
            &Regra::Maior {
                campo: "coleta.dias".into(),
                valor: "10".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !avaliar(
            &Regra::Menor {
                campo: "coleta.dias".into(),
                valor: "10".into()
            },
            &base
        )
        .unwrap()
    );
    // texto que é número compara pelo número: "12" > "9", o que não aconteceria em texto
    assert!(
        avaliar(
            &Regra::Maior {
                campo: "coleta.dias_texto".into(),
                valor: "9".into()
            },
            &base
        )
        .unwrap()
    );
    // texto que não é número: falso, sem erro (é o FLX60 avisado na publicação)
    assert!(
        !avaliar(
            &Regra::Maior {
                campo: "coleta.nome".into(),
                valor: "9".into()
            },
            &base
        )
        .unwrap()
    );
    // nulo: falso
    assert!(
        !avaliar(
            &Regra::Maior {
                campo: "coleta.nulo".into(),
                valor: "9".into()
            },
            &base
        )
        .unwrap()
    );
    // decimal
    let decimal = dados(r#"{"coleta": {"dias": 10.5}}"#);
    assert!(
        avaliar(
            &Regra::Maior {
                campo: "coleta.dias".into(),
                valor: "10.25".into()
            },
            &decimal
        )
        .unwrap()
    );
}

#[test]
fn comparar_com_literal_que_nao_e_numero_e_regra_invalida() {
    let base = dados(r#"{"coleta": {"dias": 12}}"#);
    let erro = avaliar(
        &Regra::Maior {
            campo: "coleta.dias".into(),
            valor: "dez".into(),
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
        avaliar(
            &Regra::Contem {
                campo: "relato.resumo".into(),
                valor: "nao abre".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        avaliar(
            &Regra::Contem {
                campo: "relato.resumo".into(),
                valor: "NÃO".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !avaliar(
            &Regra::Contem {
                campo: "relato.resumo".into(),
                valor: "fechou".into()
            },
            &base
        )
        .unwrap()
    );
    // lista: qualquer item que contenha o trecho
    assert!(
        avaliar(
            &Regra::Contem {
                campo: "relato.tags".into(),
                valor: "urgen".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        avaliar(
            &Regra::Contem {
                campo: "relato.tags".into(),
                valor: "fatura".into()
            },
            &base
        )
        .unwrap()
    );
    assert!(
        !avaliar(
            &Regra::Contem {
                campo: "relato.tags".into(),
                valor: "cobranca".into()
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
            avaliar(
                &Regra::Vazio {
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
    let afeta = Regra::Igual {
        campo: "relato.afeta_agora".into(),
        valor: "sim".into(),
    };
    let vazio = Regra::Vazio {
        campo: "relato.resumo".into(),
    };
    let parado = Regra::Contem {
        campo: "relato.resumo".into(),
        valor: "parado".into(),
    };
    let aceitou = Regra::Igual {
        campo: "relato.aceitou".into(),
        valor: "sim".into(),
    };

    // e(igual, nao(vazio), ou(contem, igual)) — a regra do exemplo do documento
    let regra = Regra::E {
        args: vec![
            afeta,
            Regra::Nao { args: vec![vazio] },
            Regra::Ou {
                args: vec![parado, aceitou],
            },
        ],
    };
    assert!(avaliar(&regra, &base).unwrap());

    // e() com uma condição falsa derruba o conjunto
    let falso = Regra::E {
        args: vec![
            Regra::Igual {
                campo: "relato.resumo".into(),
                valor: "outro".into(),
            },
            Regra::Vazio {
                campo: "relato.resumo".into(),
            },
        ],
    };
    assert!(!avaliar(&falso, &base).unwrap());

    // ou() com todas falsas
    let nenhuma = Regra::Ou {
        args: vec![
            Regra::Vazio {
                campo: "relato.resumo".into(),
            },
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "nao".into(),
            },
        ],
    };
    assert!(!avaliar(&nenhuma, &base).unwrap());

    // nao() com dois argumentos é regra malformada
    let malformada = Regra::Nao {
        args: vec![
            Regra::Vazio {
                campo: "relato.resumo".into(),
            },
            Regra::Vazio {
                campo: "relato.aceitou".into(),
            },
        ],
    };
    assert_eq!(
        codigo(&avaliar(&malformada, &base).unwrap_err()),
        "regra_invalida"
    );
}

#[test]
fn campo_ausente_segue_a_tabela_do_documento() {
    let base = dados(r#"{"relato": {}}"#);
    let casos = [
        (
            Regra::Igual {
                campo: "relato.x".into(),
                valor: "y".into(),
            },
            false,
        ),
        (
            Regra::Igual {
                campo: "relato.x".into(),
                valor: "".into(),
            },
            false,
        ),
        (
            Regra::Diferente {
                campo: "relato.x".into(),
                valor: "y".into(),
            },
            true,
        ),
        (
            Regra::Diferente {
                campo: "relato.x".into(),
                valor: "".into(),
            },
            true,
        ),
        (
            Regra::Maior {
                campo: "relato.x".into(),
                valor: "1".into(),
            },
            false,
        ),
        (
            Regra::Menor {
                campo: "relato.x".into(),
                valor: "1".into(),
            },
            false,
        ),
        (
            Regra::Contem {
                campo: "relato.x".into(),
                valor: "".into(),
            },
            false,
        ),
        (
            Regra::Vazio {
                campo: "relato.x".into(),
            },
            true,
        ),
    ];
    for (regra, esperado) in casos {
        assert_eq!(avaliar(&regra, &base).unwrap(), esperado, "regra {regra:?}");
    }
}

#[test]
fn regra_aninhada_alem_do_teto_e_recusada_sem_derrubar_a_pilha() {
    let mut regra = Regra::Vazio {
        campo: "relato.x".into(),
    };
    for _ in 0..(MAX_PROFUNDIDADE + 5) {
        regra = Regra::Nao { args: vec![regra] };
    }
    let erro = avaliar(&regra, &dados(r#"{"relato": {"x": "1"}}"#))
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
    let regra = Regra::Funcao {
        nome: "grave".into(),
    };
    assert!(avaliador.avaliar(&regra, &urgente).unwrap());
    assert!(!avaliador.avaliar(&regra, &normal).unwrap());
}

#[test]
fn funcao_recebe_o_dicionario_inteiro_inclusive_anexo_e_nulo() {
    let avaliador = avaliador();
    let regra = Regra::Funcao {
        nome: "viu_anexo".into(),
    };
    assert!(
        avaliador
            .avaliar(
                &regra,
                &dados(r#"{"relato": {"anexo": {"nome": "nota.pdf"}}}"#)
            )
            .unwrap()
    );
    assert!(
        !avaliador
            .avaliar(&regra, &dados(r#"{"relato": {"anexo": null}}"#))
            .unwrap()
    );
    assert!(!avaliador.avaliar(&regra, &dados("{}")).unwrap());
}

#[test]
fn funcao_que_devolve_texto_e_regra_invalida() {
    let avaliador = avaliador();
    let erro = avaliador
        .avaliar(
            &Regra::Funcao {
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
        .avaliar(
            &Regra::Funcao {
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
        .avaliar(
            &Regra::Funcao {
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
        .avaliar(
            &Regra::Funcao {
                nome: "estoura_ticks".into(),
            },
            &dados("{}"),
        )
        .expect_err("laço longo tem de ser cortado");
    let gasto = inicio.elapsed();
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
    assert!(gasto < PRAZO_DA_REGRA, "levou {gasto:?}");
}

#[test]
fn funcao_recursiva_e_regra_limite() {
    let avaliador = avaliador();
    let inicio = Instant::now();
    let erro = avaliador
        .avaliar(
            &Regra::Funcao {
                nome: "recursiva".into(),
            },
            &dados("{}"),
        )
        .expect_err("recursão tem de ser cortada");
    let gasto = inicio.elapsed();
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
    assert!(gasto < PRAZO_DA_REGRA, "levou {gasto:?}");
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
            .avaliar(
                &Regra::Funcao {
                    nome: "aloca_gigante".into(),
                },
                &base,
            )
            .expect_err("alocação gigante tem de ser cortada");
        let gasto = inicio.elapsed();
        assert_eq!(codigo(&erro), "regra_limite", "n={n}: {erro}");
        assert!(
            gasto <= PRAZO_DA_REGRA + TOLERANCIA_DO_PRAZO + Duration::from_millis(250),
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
    let erro = Avaliador::novo(fonte).expect_err("carga com constante gigante tem de ser cortada");
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
    let erro = Avaliador::novo_com(SCRIPT, &apertado, LimitesRegra::default())
        .expect_err("heap de 1 byte não carrega o script");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
}

#[test]
fn funcao_sem_def_e_regra_invalida() {
    let avaliador = avaliador();
    let erro = avaliador
        .avaliar(
            &Regra::Funcao {
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
    let erro = avaliar(
        &Regra::Funcao {
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
    let apertado = LimitesRegra {
        max_ticks: 100,
        ..LimitesRegra::default()
    };
    let avaliador = Avaliador::novo_com(SCRIPT, &Limites::default(), apertado)
        .expect("o script carrega com os limites de carga");
    let erro = avaliador
        .avaliar(
            &Regra::Funcao {
                nome: "estoura_ticks".into(),
            },
            &dados("{}"),
        )
        .expect_err("o laço não cabe em 100 ticks");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");

    // Heap apertado: a `def` não consegue nem montar o dicionário dos dados.
    let sem_heap = LimitesRegra {
        max_heap_bytes: 1,
        ..LimitesRegra::default()
    };
    let avaliador = Avaliador::novo_com(SCRIPT, &Limites::default(), sem_heap).expect("carrega");
    let erro = avaliador
        .avaliar(
            &Regra::Funcao {
                nome: "grave".into(),
            },
            &dados("{}"),
        )
        .expect_err("heap de 1 byte não dá para nada");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");

    // Profundidade de chamada: a recursão bate no limite de pilha da regra (não no padrão).
    let sem_pilha = LimitesRegra {
        max_callstack: 2,
        ..LimitesRegra::default()
    };
    let avaliador = Avaliador::novo_com(SCRIPT, &Limites::default(), sem_pilha).expect("carrega");
    let erro = avaliador
        .avaliar(
            &Regra::Funcao {
                nome: "recursiva".into(),
            },
            &dados("{}"),
        )
        .expect_err("recursão com pilha de 2 tem de ser cortada");
    assert_eq!(codigo(&erro), "regra_limite", "{erro}");
}

#[test]
fn script_que_nao_carrega_e_regra_invalida() {
    let erro = Avaliador::novo("fluxo(\n  id = \n").expect_err("sintaxe quebrada não carrega");
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
            .avaliar(
                &Regra::Funcao {
                    nome: "ecoa".into()
                },
                &base
            )
            .unwrap()
    );
    // E os combinadores tratam igual: o trecho é procurado no texto, não avaliado.
    assert!(
        avaliar(
            &Regra::Contem {
                campo: "relato.resumo".into(),
                valor: "system('ls')".into()
            },
            &base
        )
        .unwrap()
    );
    // Nome de declaração no dado também não tem poder nenhum.
    let perigoso = dados(r#"{"relato": {"resumo": "load(\"x.star\", \"y\")"}}"#);
    assert!(
        avaliador
            .avaliar(
                &Regra::Funcao {
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
    let regra = Regra::E {
        args: vec![
            Regra::Funcao {
                nome: "grave".into(),
            },
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "sim".into(),
            },
        ],
    };
    let primeiro = avaliador.avaliar(&regra, &base).unwrap();
    for _ in 0..5 {
        assert_eq!(avaliador.avaliar(&regra, &base).unwrap(), primeiro);
    }
}

#[test]
fn p95_dos_combinadores_fica_dentro_da_meta() {
    let base = dados(r#"{"relato": {"resumo": "está parado", "aceitou": true, "dias": 12}}"#);
    let regra = Regra::E {
        args: vec![
            Regra::Igual {
                campo: "relato.aceitou".into(),
                valor: "sim".into(),
            },
            Regra::Contem {
                campo: "relato.resumo".into(),
                valor: "parado".into(),
            },
            Regra::Ou {
                args: vec![
                    Regra::Maior {
                        campo: "relato.dias".into(),
                        valor: "10".into(),
                    },
                    Regra::Vazio {
                        campo: "relato.resumo".into(),
                    },
                ],
            },
        ],
    };
    // Aquecimento.
    for _ in 0..1000 {
        assert!(avaliar(&regra, &base).unwrap());
    }
    let mut tempos = Vec::with_capacity(20_000);
    for _ in 0..20_000 {
        let inicio = Instant::now();
        let _ = avaliar(&regra, &base).unwrap();
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
    let regra = Regra::Funcao {
        nome: "grave".into(),
    };
    for _ in 0..50 {
        assert!(avaliador.avaliar(&regra, &base).unwrap());
    }
    let mut tempos = Vec::with_capacity(300);
    for _ in 0..300 {
        let inicio = Instant::now();
        let _ = avaliador.avaliar(&regra, &base).unwrap();
        tempos.push(inicio.elapsed());
    }
    tempos.sort_unstable();
    let p50 = tempos[tempos.len() / 2];
    let p95 = tempos[tempos.len() * 95 / 100];
    println!("função (com thread e prazo): p50 {p50:?}, p95 {p95:?} (build de teste)");
    assert!(p95 < PRAZO_DA_REGRA, "p95 {p95:?}");
}

#[test]
fn cada_erro_de_regra_tem_codigo_de_api() {
    let base = dados(r#"{"relato": {}}"#);
    let invalida = avaliar(
        &Regra::Maior {
            campo: "relato.x".into(),
            valor: "abc".into(),
        },
        &base,
    )
    .unwrap_err();
    assert_eq!(invalida.codigo_api(), "regra_invalida");
    assert_eq!(invalida.status_http(), 400);

    let limite_erro = DomainError::RegraLimite {
        motivo: "a função passou do prazo".to_owned(),
    };
    assert_eq!(limite_erro.codigo_api(), "regra_limite");
    assert_eq!(limite_erro.status_http(), 400);
}
