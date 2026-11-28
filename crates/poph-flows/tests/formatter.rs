use poph_flows::formatter::{WIDTH, format, print};
use poph_flows::ir::{
    Acao, Campo, Decisao, Def, Destination, Effect, Etapa, Fim, Fluxo, Gatilho, Rule, SCHEMA_VERSAO,
};
use poph_flows::{Code, Ir, Limites, Severity, compile};
use poph_types::ActionActor;

const EXAMPLES: [&str; 3] = [
    "agendar_visita.star",
    "triagem_suporte.star",
    "confirmar_entrega.star",
];

fn example(nome: &str) -> String {
    let path = format!(
        "{}/../../tests/fluxos/exemplos/{nome}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn compiles(fonte: &str) -> Ir {
    let compiled = compile(fonte, &Limites::default());
    let errors: Vec<String> = compiled
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| {
            format!(
                "{:?} {}:{} {}",
                d.code,
                d.line.unwrap_or(0),
                d.column.unwrap_or(0),
                d.mensagem
            )
        })
        .collect();
    assert!(
        errors.is_empty(),
        "o script tem erro:\n{}",
        errors.join("\n")
    );
    compiled.ir.expect("sem erro, a IR existe")
}

#[test]
fn the_three_examples_come_back_to_the_same_ir() {
    for nome in EXAMPLES {
        let fonte = example(nome);
        let first = compiles(&fonte);
        let printed = print(&first);
        let second = compiles(&printed);
        assert_eq!(
            first, second,
            "a ida e volta de {nome} mudou a IR:\n--- impresso ---\n{printed}"
        );
        let formatted = format(&printed).expect("o texto canônico compila");
        assert_eq!(
            formatted, printed,
            "o texto impresso de {nome} não é ponto fixo do formatador"
        );
    }
}

#[test]
fn format_is_idempotent_on_the_three_examples() {
    for nome in EXAMPLES {
        let fonte = example(nome);
        let once = format(&fonte).expect("o exemplo compila");
        let twice = format(&once).expect("a forma canônica compila");
        assert_eq!(once, twice, "formatar duas vezes mudou {nome}");
    }
}

#[test]
fn format_preserves_the_ir_of_the_three_examples() {
    for nome in EXAMPLES {
        let fonte = example(nome);
        let before = compiles(&fonte);
        let after = compiles(&format(&fonte).expect("o exemplo compila"));
        assert_eq!(before, after, "formatar {nome} mudou a estrutura");
    }
}

#[test]
fn examples_without_misplaced_def_are_already_canonical() {
    for nome in ["agendar_visita.star", "confirmar_entrega.star"] {
        let fonte = example(nome);
        assert_eq!(
            format(&fonte).expect("o exemplo compila"),
            fonte,
            "{nome} não está na forma canônica"
        );
    }
}

#[test]
fn def_goes_before_its_users() {
    let fonte = example("triagem_suporte.star");
    let canonical = format(&fonte).expect("o exemplo compila");
    let def_position = canonical
        .find("def problema_grave")
        .expect("a def está no texto");
    let flow_position = canonical.find("fluxo(").expect("o fluxo está no texto");
    let decision_position = canonical.find("decisao(").expect("a decisão está no texto");
    assert!(flow_position < def_position, "a def veio antes do fluxo");
    assert!(
        def_position < decision_position,
        "a def não veio antes da decisão"
    );
    assert!(
        canonical
            .contains("    # Regra avançada: recebe os dados do fluxo e devolve True ou False."),
        "o comentário de dentro da def se perdeu"
    );
    assert_eq!(compiles(&canonical), compiles(&fonte));
}

#[test]
fn composed_rule_stays_open_even_when_it_fits() {
    let mut ir = minimal_ir_with_instruction("faça");
    ir.decisions.push(Decisao {
        id: "escolha".to_owned(),
        se: Rule::E {
            args: vec![
                Rule::Vazio {
                    campo: "aa".to_owned(),
                },
                Rule::Vazio {
                    campo: "aa".to_owned(),
                },
            ],
        },
        then_branch: Destination::Etapa {
            id: "unica".to_owned(),
        },
        else_branch: Destination::Etapa {
            id: "unica".to_owned(),
        },
    });
    let printed = print(&ir);
    assert!(
        printed.contains("    se = e(\n        vazio(\"aa\"),\n"),
        "a regra composta saiu em linha única:\n{printed}"
    );
}

#[test]
fn defs_keep_the_ir_order() {
    let mut ir = minimal_ir_with_instruction("faça");
    ir.defs = vec![
        Def {
            nome: "zeta".to_owned(),
            fonte: "def zeta(dados):\n    return True".to_owned(),
        },
        Def {
            nome: "alfa".to_owned(),
            fonte: "def alfa(dados):\n    return False".to_owned(),
        },
    ];
    let printed = print(&ir);
    let zeta = printed.find("def zeta").expect("a def zeta saiu");
    let alpha = printed.find("def alfa").expect("a def alfa saiu");
    assert!(zeta < alpha, "a ordem das defs mudou:\n{printed}");
    assert_eq!(compiles(&printed), ir);
}

#[test]
fn comment_attached_to_declaration_is_preserved() {
    let fonte = "# Cabeçalho do fluxo\nfluxo(\n    id = \"ff\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n)\n\n# Esta etapa recolhe\netapa(\n    id = \"aa\",\n    nome = \"Etapa\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = fim(\"fim\"))],\n)\n";
    let canonical = format(fonte).expect("compila");
    assert!(
        canonical.starts_with("# Cabeçalho do fluxo\nfluxo("),
        "o comentário do fluxo se perdeu:\n{canonical}"
    );
    assert!(
        canonical.contains("\n\n# Esta etapa recolhe\netapa("),
        "o comentário da etapa se perdeu:\n{canonical}"
    );
    assert_eq!(format(&canonical).expect("compila"), canonical);
}

#[test]
fn loose_comment_is_discarded() {
    let fonte = "# sozinho, separado por linha em branco\n\nfluxo(\n    # dentro da declaração\n    id = \"ff\", # no fim da linha\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n)\n\netapa(\n    id = \"aa\",\n    nome = \"Etapa\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = fim(\"fim\"))],\n)\n\n# no fim do arquivo\n";
    let canonical = format(fonte).expect("compila");
    assert!(
        !canonical.contains('#'),
        "sobrou comentário solto:\n{canonical}"
    );
    assert_eq!(format(&canonical).expect("compila"), canonical);
}

#[test]
fn same_input_gives_same_text() {
    for nome in EXAMPLES {
        let fonte = example(nome);
        let first = format(&fonte).expect("compila");
        let second = format(&fonte).expect("compila");
        assert_eq!(
            first, second,
            "{nome} formatou diferente na segunda chamada"
        );
        let ir = compiles(&fonte);
        assert_eq!(print(&ir), print(&ir), "{nome} imprimiu diferente");
    }
}

#[test]
fn no_canonical_line_exceeds_the_width() {
    for nome in EXAMPLES {
        let canonical = format(&example(nome)).expect("compila");
        for line in canonical.lines() {
            assert!(
                line.chars().count() <= WIDTH,
                "{nome} tem linha com {} colunas: {line}",
                line.chars().count()
            );
        }
    }
}

#[test]
fn text_with_quotes_backslash_and_newline_round_trips() {
    let complicated = "Ele disse: \"não\" \\ e quebrou\na linha\tcom tabulação — ação";
    let ir = with_complicated_texts(complicated);
    let printed = print(&ir);
    let plain = print(&with_complicated_texts(&complicated.replace('\n', " ")));
    assert_eq!(
        printed.lines().count(),
        plain.lines().count(),
        "a quebra de linha do dado virou quebra de linha do arquivo:\n{printed}"
    );
    assert!(
        printed.contains("\\n") && printed.contains("\\t"),
        "o dado não saiu escapado:\n{printed}"
    );
    assert_eq!(
        compiles(&printed),
        ir,
        "o texto complicado mudou a IR:\n{printed}"
    );
}

#[test]
fn canonical_form_has_no_trailing_space_nor_double_blank_line() {
    for nome in EXAMPLES {
        let canonical = format(&example(nome)).expect("compila");
        for line in canonical.lines() {
            assert_eq!(line, line.trim_end(), "{nome} tem espaço no fim: {line:?}");
        }
        assert!(
            !canonical.contains("\n\n\n"),
            "{nome} tem linha em branco dupla"
        );
        assert!(!canonical.contains("\n\n\n"));
        assert!(
            canonical.ends_with('\n'),
            "{nome} não termina com quebra de linha"
        );
        assert!(
            !canonical.ends_with("\n\n"),
            "{nome} termina com linha em branco a mais"
        );
        assert!(
            !canonical.starts_with('\n'),
            "{nome} começa com linha em branco"
        );
    }
}

#[test]
fn long_text_stays_on_one_line() {
    let texto = "palavra ".repeat(40);
    let ir = minimal_ir_with_instruction(&texto);
    let printed = print(&ir);
    assert!(printed.contains(&texto), "o texto foi partido:\n{printed}");
    assert_eq!(compiles(&printed), ir);
}

#[test]
fn script_with_error_returns_error_and_no_text() {
    let cases: Vec<(&str, Code)> = vec![
        ("fluxo(\n    id = \"f\",\n", Code::Syntax),
        (
            "fluxo(\n    id = \"ff\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n    cor = \"azul\",\n)\n",
            Code::UnknownArgument,
        ),
        (
            "fluxo(\n    id = \"FF\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n)\n",
            Code::InvalidId,
        ),
    ];
    for (fonte, expected) in cases {
        let failure = format(fonte).expect_err("o script tem erro");
        assert_eq!(failure.code(), "fluxo_invalido");
        let errors: Vec<Code> = failure.errors().map(|d| d.code).collect();
        assert!(
            errors.contains(&expected),
            "esperava {expected:?} em {fonte:?}, veio {errors:?}"
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .filter(|d| d.is_error())
                .all(|d| d.severity == Severity::Error),
            "diagnóstico de erro com severidade trocada"
        );
        let display = format!("{failure}");
        assert!(
            !display.contains('\n'),
            "a mensagem devia ser uma linha: {display}"
        );
    }
}

#[test]
fn source_above_the_limit_is_also_an_error() {
    let huge_text = "x".repeat(40 * 1024);
    let fonte = format!(
        "fluxo(\n    id = \"ff\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n    descricao = \"{huge_text}\",\n)\n"
    );
    let failure = format(&fonte).expect_err("a fonte passa do teto");
    assert_eq!(failure.diagnostics[0].code, Code::SourceTooLarge);
}

#[test]
fn random_sampling_comes_back_to_the_same_ir() {
    let mut generator = Generator::new(0x5157_1A16);
    for case_index in 0..60 {
        let ir = generator.ir();
        let printed = print(&ir);
        let compiled = compile(&printed, &Limites::default());
        let errors: Vec<String> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{:?} {}", d.code, d.mensagem))
            .collect();
        assert!(
            errors.is_empty(),
            "o caso {case_index} gerou script com erro: {errors:?}\n{printed}"
        );
        assert_eq!(
            compiled.ir.expect("sem erro, a IR existe"),
            ir,
            "o caso {case_index} não voltou igual:\n{printed}"
        );
        assert_eq!(format(&printed).expect("compila"), printed);
    }
}

struct Generator {
    state: u64,
    counter: usize,
}

impl Generator {
    fn new(seed: u64) -> Self {
        Self {
            state: seed | 1,
            counter: 0,
        }
    }

    fn draw(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn between(&mut self, min: usize, max: usize) -> usize {
        min + (self.draw() as usize) % (max - min + 1)
    }

    fn yes(&mut self) -> bool {
        self.draw().is_multiple_of(2)
    }

    fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let index = (self.draw() as usize) % items.len();
        &items[index]
    }

    fn ir(&mut self) -> Ir {
        self.counter += 1;
        let marker = self.counter;
        let stage_count = self.between(1, 4);
        let names: Vec<String> = (0..stage_count).map(|i| format!("e{i}")).collect();

        let mut stages = Vec::new();
        let mut ends: Vec<Fim> = Vec::new();
        for (index, id) in names.iter().enumerate() {
            let field_count = self.between(0, 3);
            let mut campos = Vec::new();
            for c in 0..field_count {
                let drawn_kind = self.draw() % 5;
                let (tipo, options) = match drawn_kind {
                    0 => (poph_types::FieldKind::Text, Vec::new()),
                    1 => (poph_types::FieldKind::Number, Vec::new()),
                    2 => (poph_types::FieldKind::Data, Vec::new()),
                    3 => (poph_types::FieldKind::Boolean, Vec::new()),
                    _ => (
                        poph_types::FieldKind::Option,
                        vec!["sim".to_owned(), "nao".to_owned()],
                    ),
                };
                campos.push(Campo {
                    id: format!("c{c}"),
                    label: format!("Campo {c} da etapa {index}"),
                    tipo,
                    required: self.yes(),
                    options,
                    help: if self.yes() {
                        Some(format!("dica {marker}-{index}-{c}"))
                    } else {
                        None
                    },
                });
            }
            let action_actors = if self.yes() {
                vec![ActionActor::Agent]
            } else {
                vec![ActionActor::Agent, ActionActor::Attendant]
            };

            let mut actions = Vec::new();
            let action_count = self.between(1, 2);
            for a in 0..action_count {
                let destination = if index + 1 < stage_count && a == 0 {
                    Destination::Etapa {
                        id: names[index + 1].clone(),
                    }
                } else {
                    let end_id = format!("fim{index}{a}");
                    let fim = Fim {
                        id: end_id.clone(),
                        resultado: if self.yes() {
                            Some(format!("resultado {marker}-{index}-{a}"))
                        } else {
                            None
                        },
                        ends_service: self.yes(),
                    };
                    ends.push(fim.clone());
                    Destination::Fim {
                        id: fim.id,
                        resultado: fim.resultado,
                        ends_service: fim.ends_service,
                    }
                };
                let mut requires: Vec<String> = campos
                    .iter()
                    .filter(|_| self.yes())
                    .map(|campo| campo.id.clone())
                    .collect();
                if requires.is_empty() && self.yes() && !campos.is_empty() {
                    requires.push(campos[0].id.clone());
                }
                let actor = if action_actors.len() > 1 && self.yes() {
                    Some(*self.choose(&action_actors))
                } else {
                    None
                };
                let mut effects = Vec::new();
                if self.yes() {
                    effects.push(Effect::MessageToCustomer {
                        texto: format!("mensagem {marker}-{index}-{a}"),
                    });
                }
                if self.yes() {
                    effects.push(Effect::Encaminhar {
                        departamento: format!("departamento {a}"),
                    });
                }
                actions.push(Acao {
                    id: format!("a{a}"),
                    label: format!("Ação {a} da etapa {index}"),
                    requires,
                    actor,
                    goes_to: destination,
                    effects,
                });
            }
            let mut on_enter = Vec::new();
            if self.yes() {
                on_enter.push(Effect::NotifyWhatsApp {
                    para: "5521999990000".to_owned(),
                    mensagem: format!("aviso {marker}-{index}"),
                });
            }
            stages.push(Etapa {
                id: id.clone(),
                nome: format!("Etapa {index}"),
                instruction: if self.yes() {
                    Some(format!("faça {index}"))
                } else {
                    None
                },
                action_actors,
                campos,
                actions,
                on_enter,
            });
        }

        let mut decisions = Vec::new();
        let mut defs = Vec::new();
        if self.yes() && stage_count > 1 {
            let etapa = &stages[0];
            if let Some(campo) = etapa.campos.first() {
                let rule = match self.draw() % 4 {
                    0 => Rule::Igual {
                        campo: format!("{}.{}", etapa.id, campo.id),
                        value: "sim".to_owned(),
                    },
                    1 => Rule::Vazio {
                        campo: format!("{}.{}", etapa.id, campo.id),
                    },
                    2 => Rule::Nao {
                        args: vec![Rule::Igual {
                            campo: format!("{}.{}", etapa.id, campo.id),
                            value: "nao".to_owned(),
                        }],
                    },
                    _ => Rule::E {
                        args: vec![
                            Rule::Vazio {
                                campo: format!("{}.{}", etapa.id, campo.id),
                            },
                            Rule::Igual {
                                campo: format!("{}.{}", etapa.id, campo.id),
                                value: "sim".to_owned(),
                            },
                        ],
                    },
                };
                let def_name = format!("regra{marker}");
                let with_def = self.yes();
                let rule = if with_def {
                    defs.push(Def {
                        nome: def_name.clone(),
                        fonte: format!(
                            "def {def_name}(dados):\n    # regra gerada no caso {marker}\n    return True"
                        ),
                    });
                    Rule::Funcao { nome: def_name }
                } else {
                    rule
                };
                let id = format!("d{marker}");
                decisions.push(Decisao {
                    id: id.clone(),
                    se: rule,
                    then_branch: Destination::Etapa {
                        id: names[stage_count - 1].clone(),
                    },
                    else_branch: Destination::Fim {
                        id: format!("fim_decisao{marker}"),
                        resultado: Some("decidido".to_owned()),
                        ends_service: false,
                    },
                });
                ends.push(Fim {
                    id: format!("fim_decisao{marker}"),
                    resultado: Some("decidido".to_owned()),
                    ends_service: false,
                });
            }
        }

        Ir {
            schema_versao: SCHEMA_VERSAO,
            fluxo: Fluxo {
                id: format!("fluxo{marker}"),
                nome: format!("Fluxo {marker}"),
                descricao: if self.yes() {
                    Some(format!("descrição do fluxo {marker}"))
                } else {
                    None
                },
            },
            gatilho: Gatilho {
                when: format!("quando {marker}"),
                not_when: if self.yes() {
                    Some(format!("não quando {marker}"))
                } else {
                    None
                },
                examples: if self.yes() {
                    vec![
                        format!("exemplo um {marker}"),
                        format!("exemplo dois {marker}"),
                    ]
                } else {
                    Vec::new()
                },
            },
            start: names[0].clone(),
            stages,
            decisions,
            ends,
            defs,
        }
    }
}

fn with_complicated_texts(texto: &str) -> Ir {
    Ir {
        schema_versao: SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "com_aspas".to_owned(),
            nome: texto.to_owned(),
            descricao: Some(texto.to_owned()),
        },
        gatilho: Gatilho {
            when: texto.to_owned(),
            not_when: Some(texto.to_owned()),
            examples: vec![texto.to_owned()],
        },
        start: "unica".to_owned(),
        stages: vec![Etapa {
            id: "unica".to_owned(),
            nome: texto.to_owned(),
            instruction: Some(texto.to_owned()),
            action_actors: vec![ActionActor::Agent],
            campos: vec![Campo {
                id: "dado".to_owned(),
                label: texto.to_owned(),
                tipo: poph_types::FieldKind::Text,
                required: true,
                options: Vec::new(),
                help: Some(texto.to_owned()),
            }],
            actions: vec![Acao {
                id: "seguir".to_owned(),
                label: texto.to_owned(),
                requires: vec!["dado".to_owned()],
                actor: None,
                goes_to: Destination::Fim {
                    id: "fim".to_owned(),
                    resultado: Some(texto.to_owned()),
                    ends_service: false,
                },
                effects: vec![Effect::MessageToCustomer {
                    texto: format!("{texto} {{ficha.id}}"),
                }],
            }],
            on_enter: Vec::new(),
        }],
        decisions: Vec::new(),
        ends: vec![Fim {
            id: "fim".to_owned(),
            resultado: Some(texto.to_owned()),
            ends_service: false,
        }],
        defs: Vec::new(),
    }
}

fn minimal_ir_with_instruction(instruction: &str) -> Ir {
    Ir {
        schema_versao: SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "longo".to_owned(),
            nome: "Fluxo com texto longo".to_owned(),
            descricao: None,
        },
        gatilho: Gatilho {
            when: "quando".to_owned(),
            not_when: None,
            examples: Vec::new(),
        },
        start: "unica".to_owned(),
        stages: vec![Etapa {
            id: "unica".to_owned(),
            nome: "Única".to_owned(),
            instruction: Some(instruction.to_owned()),
            action_actors: vec![ActionActor::Agent],
            campos: Vec::new(),
            actions: vec![Acao {
                id: "seguir".to_owned(),
                label: "Seguir".to_owned(),
                requires: Vec::new(),
                actor: None,
                goes_to: Destination::Fim {
                    id: "fim".to_owned(),
                    resultado: None,
                    ends_service: false,
                },
                effects: Vec::new(),
            }],
            on_enter: Vec::new(),
        }],
        decisions: Vec::new(),
        ends: vec![Fim {
            id: "fim".to_owned(),
            resultado: None,
            ends_service: false,
        }],
        defs: Vec::new(),
    }
}

const SCHEDULE: &str = r#"fluxo(
    id = "agenda",
    nome = "Agenda",
    inicio = "entrada",
    gatilho = gatilho(
        quando = "O cliente quer agendar.",
        nao_quando = "O cliente nao quer agendar.",
        exemplos = ["quero agendar"],
    ),
)

etapa(
    id = "entrada",
    nome = "Entrada",
    campos = [
        campo("data", "Data", obrigatorio = True),
        campo("fim", "Fim"),
        campo("endereco", "Endereço"),
        campo("executor", "Executor"),
    ],
    ao_entrar = [sincronizar_agenda("update", inicio = "{entrada.data}")],
    acoes = [acao(
        "seguir",
        "Seguir",
        exige = ["data", "fim", "endereco", "executor"],
        efeitos = [sincronizar_agenda(
            "create",
            inicio = "{entrada.data}",
            fim = "{entrada.fim}",
            titulo = "Visita {ficha.id}",
            local = "{entrada.endereco}",
            executor = "{entrada.executor}",
        )],
        vai_para = fim("pronto"),
    )],
)

etapa(
    id = "fim_visita",
    nome = "Fim da visita",
    acoes = [acao("fechar", "Fechar", efeitos = [sincronizar_agenda("remove")], vai_para = fim("pronto"))],
)
"#;

#[test]
fn calendar_effect_round_trips_through_the_canonical_form() {
    let first = compiles(SCHEDULE);
    let printed = print(&first);
    let second = compiles(&printed);
    assert_eq!(first, second, "a ida e volta mudou a IR:\n{printed}");
    assert_eq!(
        format(&printed).expect("o texto canônico compila"),
        printed,
        "o texto impresso não é ponto fixo"
    );
    assert!(
        printed.contains("sincronizar_agenda(\"remove\")"),
        "a remoção tem de sair com a forma curta:\n{printed}"
    );
    assert!(
        !printed.contains("operacao ="),
        "a operação sai posicional, sem o nome do argumento:\n{printed}"
    );
}
