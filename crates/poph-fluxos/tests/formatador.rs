//! Forma canônica do script de fluxo (FX-16): ida e volta, idempotência, comentários e erro.
//!
//! Os três exemplos do repositório (`tests/fluxos/exemplos/*.star`) são o material de partida. A
//! prova de ida e volta é `script → IR → script → IR`: a segunda IR tem de ser igual à primeira, e
//! o teste de amostragem faz o mesmo com estruturas geradas por semente fixa.

use poph_fluxos::formatador::{LARGURA, formatar, imprimir};
use poph_fluxos::ir::{
    Acao, Campo, Decisao, Def, Destino, Efeito, Etapa, Fim, Fluxo, Gatilho, Regra, SCHEMA_VERSAO,
};
use poph_fluxos::{Codigo, Ir, Limites, Severidade, compilar};
use poph_types::AtorAcao;

/// Os três exemplos do repositório.
const EXEMPLOS: [&str; 3] = [
    "agendar_visita.star",
    "triagem_suporte.star",
    "confirmar_entrega.star",
];

fn exemplo(nome: &str) -> String {
    let caminho = format!(
        "{}/../../tests/fluxos/exemplos/{nome}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&caminho).unwrap_or_else(|erro| panic!("{caminho}: {erro}"))
}

fn compila(fonte: &str) -> Ir {
    let compilado = compilar(fonte, &Limites::default());
    let erros: Vec<String> = compilado
        .diagnosticos
        .iter()
        .filter(|d| d.e_erro())
        .map(|d| {
            format!(
                "{:?} {}:{} {}",
                d.codigo,
                d.linha.unwrap_or(0),
                d.coluna.unwrap_or(0),
                d.mensagem
            )
        })
        .collect();
    assert!(erros.is_empty(), "o script tem erro:\n{}", erros.join("\n"));
    compilado.ir.expect("sem erro, a IR existe")
}

// ------------------------------------------------------------------ ida e volta

#[test]
fn os_tres_exemplos_voltam_a_mesma_ir() {
    for nome in EXEMPLOS {
        let fonte = exemplo(nome);
        let primeira = compila(&fonte);
        let impresso = imprimir(&primeira);
        let segunda = compila(&impresso);
        assert_eq!(
            primeira, segunda,
            "a ida e volta de {nome} mudou a IR:\n--- impresso ---\n{impresso}"
        );
        // E formatar o texto impresso não muda mais nada: ele já é canônico.
        let formatado = formatar(&impresso).expect("o texto canônico compila");
        assert_eq!(
            formatado, impresso,
            "o texto impresso de {nome} não é ponto fixo do formatador"
        );
    }
}

#[test]
fn formatar_e_idempotente_nos_tres_exemplos() {
    for nome in EXEMPLOS {
        let fonte = exemplo(nome);
        let uma_vez = formatar(&fonte).expect("o exemplo compila");
        let duas_vezes = formatar(&uma_vez).expect("a forma canônica compila");
        assert_eq!(uma_vez, duas_vezes, "formatar duas vezes mudou {nome}");
    }
}

#[test]
fn formatar_preserva_a_ir_dos_tres_exemplos() {
    for nome in EXEMPLOS {
        let fonte = exemplo(nome);
        let antes = compila(&fonte);
        let depois = compila(&formatar(&fonte).expect("o exemplo compila"));
        assert_eq!(antes, depois, "formatar {nome} mudou a estrutura");
    }
}

#[test]
fn os_exemplos_sem_def_fora_de_lugar_ja_estao_na_forma_canonica() {
    // `agendar_visita` e `confirmar_entrega` não têm `def`: o arquivo do repositório já é a forma
    // canônica, byte a byte (inclusive os comentários do topo). É o que prova que a largura, o
    // recuo e a ordem dos argumentos são os do repositório.
    for nome in ["agendar_visita.star", "confirmar_entrega.star"] {
        let fonte = exemplo(nome);
        assert_eq!(
            formatar(&fonte).expect("o exemplo compila"),
            fonte,
            "{nome} não está na forma canônica"
        );
    }
}

#[test]
fn a_def_vai_para_antes_de_quem_a_usa() {
    // `triagem_suporte` declara a `def` no fim do arquivo e a usa numa decisão antes: a forma
    // canônica move a `def` para cima do `fluxo` e mantém o resto igual.
    let fonte = exemplo("triagem_suporte.star");
    let canonico = formatar(&fonte).expect("o exemplo compila");
    let posicao_def = canonico
        .find("def problema_grave")
        .expect("a def está no texto");
    let posicao_fluxo = canonico.find("fluxo(").expect("o fluxo está no texto");
    let posicao_decisao = canonico.find("decisao(").expect("a decisão está no texto");
    assert!(posicao_fluxo < posicao_def, "a def veio antes do fluxo");
    assert!(
        posicao_def < posicao_decisao,
        "a def não veio antes da decisão"
    );
    // O corpo da `def` sai inteiro, com os comentários que ela guarda na IR.
    assert!(
        canonico
            .contains("    # Regra avançada: recebe os dados do fluxo e devolve True ou False."),
        "o comentário de dentro da def se perdeu"
    );
    // Fora a `def` movida, a IR é a mesma.
    assert_eq!(compila(&canonico), compila(&fonte));
}

#[test]
fn a_regra_composta_fica_aberta_mesmo_quando_cabe() {
    // `e(...)`/`ou(...)`/`nao(...)` são a exceção à regra de largura: uma regra composta se lê como
    // lista. Sem isso, o `se` de uma decisão curta voltaria para a linha única.
    let mut ir = ir_minima_com_instrucao("faça");
    ir.decisoes.push(Decisao {
        id: "escolha".to_owned(),
        se: Regra::E {
            args: vec![
                Regra::Vazio {
                    campo: "aa".to_owned(),
                },
                Regra::Vazio {
                    campo: "aa".to_owned(),
                },
            ],
        },
        entao: Destino::Etapa {
            id: "unica".to_owned(),
        },
        senao: Destino::Etapa {
            id: "unica".to_owned(),
        },
    });
    let impresso = imprimir(&ir);
    assert!(
        impresso.contains("    se = e(\n        vazio(\"aa\"),\n"),
        "a regra composta saiu em linha única:\n{impresso}"
    );
}

#[test]
fn as_defs_mantem_a_ordem_da_ir() {
    // A ordem das `def` é a da IR: o formatador as sobe para antes de quem as usa, mas não as
    // reordena entre si (a IR é comparada item a item).
    let mut ir = ir_minima_com_instrucao("faça");
    // Os nomes são de propósito fora da ordem alfabética: ordenar por nome mudaria a IR.
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
    let impresso = imprimir(&ir);
    let zeta = impresso.find("def zeta").expect("a def zeta saiu");
    let alfa = impresso.find("def alfa").expect("a def alfa saiu");
    assert!(zeta < alfa, "a ordem das defs mudou:\n{impresso}");
    assert_eq!(compila(&impresso), ir);
}

// ------------------------------------------------------------------ comentários

#[test]
fn comentario_ligado_a_declaracao_e_preservado() {
    let fonte = "# Cabeçalho do fluxo\nfluxo(\n    id = \"ff\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n)\n\n# Esta etapa recolhe\netapa(\n    id = \"aa\",\n    nome = \"Etapa\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = fim(\"fim\"))],\n)\n";
    let canonico = formatar(fonte).expect("compila");
    assert!(
        canonico.starts_with("# Cabeçalho do fluxo\nfluxo("),
        "o comentário do fluxo se perdeu:\n{canonico}"
    );
    assert!(
        canonico.contains("\n\n# Esta etapa recolhe\netapa("),
        "o comentário da etapa se perdeu:\n{canonico}"
    );
    // Idempotente: o comentário preservado não vira duplicado na segunda passagem.
    assert_eq!(formatar(&canonico).expect("compila"), canonico);
}

#[test]
fn comentario_solto_e_descartado() {
    // Três comentários soltos: dentro da declaração, no fim de uma linha e separado por linha em
    // branco. Nenhum deles sobrevive — e o texto não muda na segunda passagem.
    let fonte = "# sozinho, separado por linha em branco\n\nfluxo(\n    # dentro da declaração\n    id = \"ff\", # no fim da linha\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n)\n\netapa(\n    id = \"aa\",\n    nome = \"Etapa\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = fim(\"fim\"))],\n)\n\n# no fim do arquivo\n";
    let canonico = formatar(fonte).expect("compila");
    assert!(
        !canonico.contains('#'),
        "sobrou comentário solto:\n{canonico}"
    );
    assert_eq!(formatar(&canonico).expect("compila"), canonico);
}

// ------------------------------------------------------------------ determinismo e teto

#[test]
fn a_mesma_entrada_da_o_mesmo_texto() {
    for nome in EXEMPLOS {
        let fonte = exemplo(nome);
        let primeira = formatar(&fonte).expect("compila");
        let segunda = formatar(&fonte).expect("compila");
        assert_eq!(
            primeira, segunda,
            "{nome} formatou diferente na segunda chamada"
        );
        let ir = compila(&fonte);
        assert_eq!(imprimir(&ir), imprimir(&ir), "{nome} imprimiu diferente");
    }
}

#[test]
fn nenhuma_linha_da_forma_canonica_passa_da_largura() {
    // Texto sozinho pode não caber; fora isso, nenhuma linha dos exemplos passa do teto.
    for nome in EXEMPLOS {
        let canonico = formatar(&exemplo(nome)).expect("compila");
        for linha in canonico.lines() {
            assert!(
                linha.chars().count() <= LARGURA,
                "{nome} tem linha com {} colunas: {linha}",
                linha.chars().count()
            );
        }
    }
}

#[test]
fn texto_com_aspas_barra_e_quebra_de_linha_volta_igual() {
    // O que o autor digita pode ter aspas, barra invertida, quebra de linha, tabulação e acento. O
    // escape tem de sobreviver à recompilação, senão a IR volta diferente (ou o script nem abre).
    let complicado = "Ele disse: \"não\" \\ e quebrou\na linha\tcom tabulação — ação";
    let ir = com_textos_complicados(complicado);
    let impresso = imprimir(&ir);
    // A quebra de linha do dado é escape (`\n`), não quebra do arquivo: trocando a quebra por um
    // espaço (mesmo comprimento, mesmas decisões de largura), o número de linhas tem de ser o mesmo.
    let simples = imprimir(&com_textos_complicados(&complicado.replace('\n', " ")));
    assert_eq!(
        impresso.lines().count(),
        simples.lines().count(),
        "a quebra de linha do dado virou quebra de linha do arquivo:\n{impresso}"
    );
    assert!(
        impresso.contains("\\n") && impresso.contains("\\t"),
        "o dado não saiu escapado:\n{impresso}"
    );
    assert_eq!(
        compila(&impresso),
        ir,
        "o texto complicado mudou a IR:\n{impresso}"
    );
}

#[test]
fn a_forma_canonica_nao_deixa_espaco_no_fim_da_linha_nem_linha_em_branco_dupla() {
    for nome in EXEMPLOS {
        let canonico = formatar(&exemplo(nome)).expect("compila");
        for linha in canonico.lines() {
            assert_eq!(
                linha,
                linha.trim_end(),
                "{nome} tem espaço no fim: {linha:?}"
            );
        }
        assert!(
            !canonico.contains("\n\n\n"),
            "{nome} tem linha em branco dupla"
        );
        assert!(!canonico.contains("\n\n\n"));
        assert!(
            canonico.ends_with('\n'),
            "{nome} não termina com quebra de linha"
        );
        assert!(
            !canonico.ends_with("\n\n"),
            "{nome} termina com linha em branco a mais"
        );
        assert!(
            !canonico.starts_with('\n'),
            "{nome} começa com linha em branco"
        );
    }
}

#[test]
fn texto_longo_continua_numa_linha_so() {
    // Quebrar um literal mudaria o valor: a linha passa da largura, e a IR volta igual.
    let texto = "palavra ".repeat(40);
    let ir = ir_minima_com_instrucao(&texto);
    let impresso = imprimir(&ir);
    assert!(
        impresso.contains(&texto),
        "o texto foi partido:\n{impresso}"
    );
    assert_eq!(compila(&impresso), ir);
}

// ------------------------------------------------------------------ erro

#[test]
fn script_com_erro_devolve_erro_e_nenhum_texto() {
    // Dois tipos de erro: sintaxe (o parser não fecha a chamada) e forma (argumento desconhecido).
    let casos: Vec<(&str, Codigo)> = vec![
        ("fluxo(\n    id = \"f\",\n", Codigo::Sintaxe),
        (
            "fluxo(\n    id = \"ff\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n    cor = \"azul\",\n)\n",
            Codigo::ArgumentoDesconhecido,
        ),
        (
            "fluxo(\n    id = \"FF\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n)\n",
            Codigo::IdInvalido,
        ),
    ];
    for (fonte, esperado) in casos {
        let falha = formatar(fonte).expect_err("o script tem erro");
        assert_eq!(falha.codigo(), "fluxo_invalido");
        let erros: Vec<Codigo> = falha.erros().map(|d| d.codigo).collect();
        assert!(
            erros.contains(&esperado),
            "esperava {esperado:?} em {fonte:?}, veio {erros:?}"
        );
        assert!(
            falha
                .diagnosticos
                .iter()
                .filter(|d| d.e_erro())
                .all(|d| d.severidade == Severidade::Erro),
            "diagnóstico de erro com severidade trocada"
        );
        // A mensagem de exibição não vaza o script inteiro nem devolve texto parcial.
        let exibicao = format!("{falha}");
        assert!(
            !exibicao.contains('\n'),
            "a mensagem devia ser uma linha: {exibicao}"
        );
    }
}

#[test]
fn fonte_acima_do_limite_tambem_e_erro() {
    // Um único argumento gigante passa do teto de 32 KB de fonte.
    let gigante = "x".repeat(40 * 1024);
    let fonte = format!(
        "fluxo(\n    id = \"ff\",\n    nome = \"Fluxo\",\n    inicio = \"aa\",\n    descricao = \"{gigante}\",\n)\n"
    );
    let falha = formatar(&fonte).expect_err("a fonte passa do teto");
    assert_eq!(falha.diagnosticos[0].codigo, Codigo::FonteGrande);
}

// ------------------------------------------------------------------ amostragem

#[test]
fn a_amostragem_aleatoria_volta_a_mesma_ir() {
    // Semente fixa: o teste não vira intermitente. Cada estrutura gerada passa por
    // `IR → script → IR` e tem de voltar igual, sem erro de compilação.
    let mut gerador = Gerador::novo(0x5157_1A16);
    for caso in 0..60 {
        let ir = gerador.ir();
        let impresso = imprimir(&ir);
        let compilado = compilar(&impresso, &Limites::default());
        let erros: Vec<String> = compilado
            .diagnosticos
            .iter()
            .filter(|d| d.e_erro())
            .map(|d| format!("{:?} {}", d.codigo, d.mensagem))
            .collect();
        assert!(
            erros.is_empty(),
            "o caso {caso} gerou script com erro: {erros:?}\n{impresso}"
        );
        assert_eq!(
            compilado.ir.expect("sem erro, a IR existe"),
            ir,
            "o caso {caso} não voltou igual:\n{impresso}"
        );
        // E formatar o texto impresso devolve o mesmo texto.
        assert_eq!(formatar(&impresso).expect("compila"), impresso);
    }
}

/// Gerador determinístico de estruturas válidas (xorshift de 64 bits com semente fixa).
struct Gerador {
    estado: u64,
    contador: usize,
}

impl Gerador {
    fn novo(semente: u64) -> Self {
        Self {
            estado: semente | 1,
            contador: 0,
        }
    }

    fn sorteia(&mut self) -> u64 {
        let mut x = self.estado;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.estado = x;
        x
    }

    fn entre(&mut self, minimo: usize, maximo: usize) -> usize {
        minimo + (self.sorteia() as usize) % (maximo - minimo + 1)
    }

    fn sim(&mut self) -> bool {
        self.sorteia().is_multiple_of(2)
    }

    fn escolhe<'a, T>(&mut self, itens: &'a [T]) -> &'a T {
        let indice = (self.sorteia() as usize) % itens.len();
        &itens[indice]
    }

    fn ir(&mut self) -> Ir {
        self.contador += 1;
        let marca = self.contador;
        let quantas_etapas = self.entre(1, 4);
        let nomes: Vec<String> = (0..quantas_etapas).map(|i| format!("e{i}")).collect();

        let mut etapas = Vec::new();
        let mut fins: Vec<Fim> = Vec::new();
        for (indice, id) in nomes.iter().enumerate() {
            let quantos_campos = self.entre(0, 3);
            let mut campos = Vec::new();
            for c in 0..quantos_campos {
                let tipo_sorteado = self.sorteia() % 5;
                let (tipo, opcoes) = match tipo_sorteado {
                    0 => (poph_types::TipoCampo::Texto, Vec::new()),
                    1 => (poph_types::TipoCampo::Numero, Vec::new()),
                    2 => (poph_types::TipoCampo::Data, Vec::new()),
                    3 => (poph_types::TipoCampo::Booleano, Vec::new()),
                    _ => (
                        poph_types::TipoCampo::Opcao,
                        vec!["sim".to_owned(), "nao".to_owned()],
                    ),
                };
                campos.push(Campo {
                    id: format!("c{c}"),
                    rotulo: format!("Campo {c} da etapa {indice}"),
                    tipo,
                    obrigatorio: self.sim(),
                    opcoes,
                    ajuda: if self.sim() {
                        Some(format!("dica {marca}-{indice}-{c}"))
                    } else {
                        None
                    },
                });
            }
            let atores_acao = if self.sim() {
                vec![AtorAcao::Agente]
            } else {
                vec![AtorAcao::Agente, AtorAcao::Atendente]
            };

            let mut acoes = Vec::new();
            let quantas_acoes = self.entre(1, 2);
            for a in 0..quantas_acoes {
                let destino = if indice + 1 < quantas_etapas && a == 0 {
                    Destino::Etapa {
                        id: nomes[indice + 1].clone(),
                    }
                } else {
                    let id_fim = format!("fim{indice}{a}");
                    let fim = Fim {
                        id: id_fim.clone(),
                        resultado: if self.sim() {
                            Some(format!("resultado {marca}-{indice}-{a}"))
                        } else {
                            None
                        },
                        encerra_atendimento: self.sim(),
                    };
                    fins.push(fim.clone());
                    Destino::Fim {
                        id: fim.id,
                        resultado: fim.resultado,
                        encerra_atendimento: fim.encerra_atendimento,
                    }
                };
                let mut exige: Vec<String> = campos
                    .iter()
                    .filter(|_| self.sim())
                    .map(|campo| campo.id.clone())
                    .collect();
                if exige.is_empty() && self.sim() && !campos.is_empty() {
                    exige.push(campos[0].id.clone());
                }
                let ator = if atores_acao.len() > 1 && self.sim() {
                    Some(*self.escolhe(&atores_acao))
                } else {
                    None
                };
                let mut efeitos = Vec::new();
                if self.sim() {
                    efeitos.push(Efeito::MensagemAoCliente {
                        texto: format!("mensagem {marca}-{indice}-{a}"),
                    });
                }
                if self.sim() {
                    efeitos.push(Efeito::Encaminhar {
                        departamento: format!("departamento {a}"),
                    });
                }
                acoes.push(Acao {
                    id: format!("a{a}"),
                    rotulo: format!("Ação {a} da etapa {indice}"),
                    exige,
                    ator,
                    vai_para: destino,
                    efeitos,
                });
            }
            let mut ao_entrar = Vec::new();
            if self.sim() {
                ao_entrar.push(Efeito::NotificarWhatsapp {
                    para: "5521999990000".to_owned(),
                    mensagem: format!("aviso {marca}-{indice}"),
                });
            }
            etapas.push(Etapa {
                id: id.clone(),
                nome: format!("Etapa {indice}"),
                instrucao: if self.sim() {
                    Some(format!("faça {indice}"))
                } else {
                    None
                },
                atores_acao,
                campos,
                acoes,
                ao_entrar,
            });
        }

        let mut decisoes = Vec::new();
        let mut defs = Vec::new();
        if self.sim() && quantas_etapas > 1 {
            // Uma decisão que lê um campo da primeira etapa e escolhe entre duas etapas.
            let etapa = &etapas[0];
            if let Some(campo) = etapa.campos.first() {
                let regra = match self.sorteia() % 4 {
                    0 => Regra::Igual {
                        campo: format!("{}.{}", etapa.id, campo.id),
                        valor: "sim".to_owned(),
                    },
                    1 => Regra::Vazio {
                        campo: format!("{}.{}", etapa.id, campo.id),
                    },
                    2 => Regra::Nao {
                        args: vec![Regra::Igual {
                            campo: format!("{}.{}", etapa.id, campo.id),
                            valor: "nao".to_owned(),
                        }],
                    },
                    _ => Regra::E {
                        args: vec![
                            Regra::Vazio {
                                campo: format!("{}.{}", etapa.id, campo.id),
                            },
                            Regra::Igual {
                                campo: format!("{}.{}", etapa.id, campo.id),
                                valor: "sim".to_owned(),
                            },
                        ],
                    },
                };
                let nome_def = format!("regra{marca}");
                let com_def = self.sim();
                let regra = if com_def {
                    defs.push(Def {
                        nome: nome_def.clone(),
                        fonte: format!(
                            "def {nome_def}(dados):\n    # regra gerada no caso {marca}\n    return True"
                        ),
                    });
                    Regra::Funcao { nome: nome_def }
                } else {
                    regra
                };
                let id = format!("d{marca}");
                decisoes.push(Decisao {
                    id: id.clone(),
                    se: regra,
                    entao: Destino::Etapa {
                        id: nomes[quantas_etapas - 1].clone(),
                    },
                    senao: Destino::Fim {
                        id: format!("fim_decisao{marca}"),
                        resultado: Some("decidido".to_owned()),
                        encerra_atendimento: false,
                    },
                });
                fins.push(Fim {
                    id: format!("fim_decisao{marca}"),
                    resultado: Some("decidido".to_owned()),
                    encerra_atendimento: false,
                });
            }
        }

        Ir {
            schema_versao: SCHEMA_VERSAO,
            fluxo: Fluxo {
                id: format!("fluxo{marca}"),
                nome: format!("Fluxo {marca}"),
                descricao: if self.sim() {
                    Some(format!("descrição do fluxo {marca}"))
                } else {
                    None
                },
            },
            gatilho: Gatilho {
                quando: format!("quando {marca}"),
                nao_quando: if self.sim() {
                    Some(format!("não quando {marca}"))
                } else {
                    None
                },
                exemplos: if self.sim() {
                    vec![
                        format!("exemplo um {marca}"),
                        format!("exemplo dois {marca}"),
                    ]
                } else {
                    Vec::new()
                },
            },
            inicio: nomes[0].clone(),
            etapas,
            decisoes,
            fins,
            defs,
        }
    }
}

/// Uma IR mínima em que o mesmo texto aparece onde a linguagem aceita texto livre.
fn com_textos_complicados(texto: &str) -> Ir {
    Ir {
        schema_versao: SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "com_aspas".to_owned(),
            nome: texto.to_owned(),
            descricao: Some(texto.to_owned()),
        },
        gatilho: Gatilho {
            quando: texto.to_owned(),
            nao_quando: Some(texto.to_owned()),
            exemplos: vec![texto.to_owned()],
        },
        inicio: "unica".to_owned(),
        etapas: vec![Etapa {
            id: "unica".to_owned(),
            nome: texto.to_owned(),
            instrucao: Some(texto.to_owned()),
            atores_acao: vec![AtorAcao::Agente],
            campos: vec![Campo {
                id: "dado".to_owned(),
                rotulo: texto.to_owned(),
                tipo: poph_types::TipoCampo::Texto,
                obrigatorio: true,
                opcoes: Vec::new(),
                ajuda: Some(texto.to_owned()),
            }],
            acoes: vec![Acao {
                id: "seguir".to_owned(),
                rotulo: texto.to_owned(),
                exige: vec!["dado".to_owned()],
                ator: None,
                vai_para: Destino::Fim {
                    id: "fim".to_owned(),
                    resultado: Some(texto.to_owned()),
                    encerra_atendimento: false,
                },
                efeitos: vec![Efeito::MensagemAoCliente {
                    texto: format!("{texto} {{ficha.id}}"),
                }],
            }],
            ao_entrar: Vec::new(),
        }],
        decisoes: Vec::new(),
        fins: vec![Fim {
            id: "fim".to_owned(),
            resultado: Some(texto.to_owned()),
            encerra_atendimento: false,
        }],
        defs: Vec::new(),
    }
}

/// Uma IR mínima com a instrução dada, para provar que texto não é partido.
fn ir_minima_com_instrucao(instrucao: &str) -> Ir {
    Ir {
        schema_versao: SCHEMA_VERSAO,
        fluxo: Fluxo {
            id: "longo".to_owned(),
            nome: "Fluxo com texto longo".to_owned(),
            descricao: None,
        },
        gatilho: Gatilho {
            quando: "quando".to_owned(),
            nao_quando: None,
            exemplos: Vec::new(),
        },
        inicio: "unica".to_owned(),
        etapas: vec![Etapa {
            id: "unica".to_owned(),
            nome: "Única".to_owned(),
            instrucao: Some(instrucao.to_owned()),
            atores_acao: vec![AtorAcao::Agente],
            campos: Vec::new(),
            acoes: vec![Acao {
                id: "seguir".to_owned(),
                rotulo: "Seguir".to_owned(),
                exige: Vec::new(),
                ator: None,
                vai_para: Destino::Fim {
                    id: "fim".to_owned(),
                    resultado: None,
                    encerra_atendimento: false,
                },
                efeitos: Vec::new(),
            }],
            ao_entrar: Vec::new(),
        }],
        decisoes: Vec::new(),
        fins: vec![Fim {
            id: "fim".to_owned(),
            resultado: None,
            encerra_atendimento: false,
        }],
        defs: Vec::new(),
    }
}
