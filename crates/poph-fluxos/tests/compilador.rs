//! Testes do compilador de fluxos (FX-10): os três exemplos dourados, um caso por diagnóstico de
//! forma (com linha e coluna), limites, determinismo e `carregar` recusando IR adulterada.

use std::time::{Duration, Instant};

use poph_fluxos::compilador::MAX_FONTE_BYTES;
use poph_fluxos::{Code, Compilado, FalhaCarregamento, Limites, carregar, compilar};

/// Fluxo mínimo e **sem nenhum diagnóstico**: é a base das mutações de cada caso.
const BASE: &str = r#"fluxo(
    id = "teste",
    nome = "Teste",
    inicio = "entrada",
    gatilho = gatilho(
        quando = "O cliente quer testar.",
        nao_quando = "O cliente nao quer testar.",
        exemplos = ["quero testar"],
    ),
)

etapa(
    id = "entrada",
    nome = "Entrada",
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [acao("seguir", "Seguir", exige = ["nome"], vai_para = fim("pronto", resultado = "Feito"))],
)
"#;

fn limites() -> Limites {
    Limites::default()
}

fn compila(fonte: &str) -> Compilado {
    compilar(fonte, &limites())
}

fn exemplo(nome: &str) -> String {
    let caminho = format!(
        "{}/../../tests/fluxos/exemplos/{nome}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&caminho).unwrap_or_else(|erro| panic!("{caminho}: {erro}"))
}

/// Fluxo base com uma decisão extra, para os diagnósticos de regra.
fn com_regra(regra: &str) -> String {
    format!(
        "{BASE}\ndecisao(\n    id = \"escolha\",\n    se = {regra},\n    entao = \"entrada\",\n    senao = \"entrada\",\n)\n"
    )
}

/// Um caso por código da lista fechada: o script que dispara e onde.
struct Caso {
    codigo: Code,
    fonte: String,
    limites: Limites,
    linha: u32,
    coluna: u32,
}

/// Onde o diagnóstico de cada código tem de aparecer. Conferido à mão contra o texto de cada caso
/// (o ponto é sempre o trecho que o autor tem de corrigir).
fn posicoes() -> Vec<(Code, u32, u32)> {
    vec![
        (Code::Syntax, 3, 1),
        (Code::SourceTooLarge, 1, 1),
        (Code::Dialect, 1, 1),
        (Code::LoadLimit, 1, 1),
        (Code::InvalidTopLevel, 1, 1),
        (Code::WrongType, 15, 51),
        (Code::UnknownName, 20, 14),
        (Code::MissingFlow, 1, 1),
        (Code::MissingArgument, 1, 1),
        (Code::UnknownArgument, 15, 52),
        (Code::InvalidId, 2, 10),
        (Code::DuplicateId, 20, 10),
        (Code::UnknownDestination, 16, 68),
        (Code::DivergentEnd, 22, 46),
        (Code::UnknownRequirement, 16, 47),
        (Code::InvalidOptions, 15, 21),
        (Code::InvalidRuleOrActor, 15, 14),
        (Code::UnknownType, 15, 44),
        (Code::MalformedCombinator, 21, 16),
        (Code::UnknownField, 21, 16),
        (Code::DecisionCycle, 19, 1),
        (Code::InvalidDef, 19, 1),
        (Code::InvalidVariable, 15, 38),
        (Code::ForbiddenInterpolation, 15, 17),
        (Code::NodeLimit, 1, 1),
        (Code::InvalidNumber, 16, 94),
        (Code::StageWithoutAction, 12, 1),
        (Code::Unreachable, 19, 1),
        (Code::UnusedField, 15, 21),
        (Code::OutsideOptions, 21, 16),
        (Code::ContradictoryCondition, 21, 10),
        (Code::TriggerWithoutExamples, 5, 15),
        (Code::TriggerWithoutNegative, 5, 15),
        (Code::NotificationWithoutVariable, 16, 122),
        (Code::VariableWithoutField, 15, 38),
        (Code::TextComparison, 21, 16),
        (Code::UnusedDef, 19, 1),
        (Code::ActorWithoutAction, 15, 14),
        (Code::VariableFromOwnStep, 15, 38),
        (Code::LongText, 15, 17),
    ]
}

fn posicao_esperada(codigo: Code) -> (u32, u32) {
    posicoes()
        .into_iter()
        .find(|(c, _, _)| *c == codigo)
        .map(|(_, linha, coluna)| (linha, coluna))
        .unwrap_or_else(|| panic!("{codigo} sem posição esperada"))
}

/// Um caso simples, com os limites padrão.
fn caso(codigo: Code, fonte: impl Into<String>) -> Caso {
    let (linha, coluna) = posicao_esperada(codigo);
    Caso {
        codigo,
        fonte: fonte.into(),
        limites: limites(),
        linha,
        coluna,
    }
}

fn casos() -> Vec<Caso> {
    let (linha_carga, coluna_carga) = posicao_esperada(Code::LoadLimit);
    vec![
        // FLX01 — sintaxe inválida.
        caso(Code::Syntax, "fluxo(\n    id = \"teste\",\n"),
        // FLX02 — fonte acima de 32 KB.
        caso(Code::SourceTooLarge, "#".repeat(MAX_FONTE_BYTES + 1)),
        // FLX03 — construção fora do dialeto.
        caso(
            Code::Dialect,
            format!("load(\"outro.star\", \"ajuda\")\n{BASE}"),
        ),
        // FLX04 — limite de carga estourado (heap apertado de propósito).
        Caso {
            codigo: Code::LoadLimit,
            fonte: BASE.to_owned(),
            limites: Limites {
                max_heap_bytes: 1,
                ..limites()
            },
            linha: linha_carga,
            coluna: coluna_carga,
        },
        // FLX05 — item no topo que não é declaração.
        caso(Code::InvalidTopLevel, format!("contador = 0\n{BASE}")),
        // FLX06 — valor com tipo errado.
        caso(
            Code::WrongType,
            BASE.replace("obrigatorio = True", "obrigatorio = \"sim\""),
        ),
        // FLX07 — nome que não existe no dialeto.
        caso(
            Code::UnknownName,
            format!("{BASE}\ndef minha(dados):\n    aberto = open(\"x.txt\")\n    return len(aberto) > 0\n"),
        ),
        // FLX10 — falta `fluxo(...)`.
        caso(
            Code::MissingFlow,
            "etapa(\n    id = \"entrada\",\n    nome = \"Entrada\",\n)\n",
        ),
        // FLX11 — argumento obrigatório ausente.
        caso(
            Code::MissingArgument,
            BASE.replace("    inicio = \"entrada\",\n", ""),
        ),
        // FLX12 — argumento desconhecido.
        caso(
            Code::UnknownArgument,
            BASE.replace("obrigatorio = True", "obrigatorioo = True"),
        ),
        // FLX13 — id fora do formato.
        caso(
            Code::InvalidId,
            BASE.replace("id = \"teste\"", "id = \"Teste Ruim\""),
        ),
        // FLX14 — id repetido.
        caso(
            Code::DuplicateId,
            format!("{BASE}\netapa(\n    id = \"entrada\",\n    nome = \"Repetida\",\n    acoes = [acao(\"outra\", \"Outra\", vai_para = fim(\"repetida\"))],\n)\n"),
        ),
        // FLX15 — destino inexistente.
        caso(
            Code::UnknownDestination,
            BASE.replace(
                "vai_para = fim(\"pronto\", resultado = \"Feito\")",
                "vai_para = \"nao_existe\"",
            ),
        ),
        // FLX16 — mesmo `fim` com argumentos diferentes.
        caso(
            Code::DivergentEnd,
            format!("{BASE}\netapa(\n    id = \"outra\",\n    nome = \"Outra\",\n    acoes = [acao(\"sair\", \"Sair\", vai_para = fim(\"pronto\", resultado = \"Outro\"))],\n)\n"),
        ),
        // FLX17 — `exige` cita campo que não existe na etapa.
        caso(
            Code::UnknownRequirement,
            BASE.replace("exige = [\"nome\"]", "exige = [\"cep\"]"),
        ),
        // FLX18 — `opcao` sem opções.
        caso(
            Code::InvalidOptions,
            BASE.replace(
                "campo(\"nome\", \"Nome\", obrigatorio = True)",
                "campo(\"nome\", \"Nome\", tipo = \"opcao\", obrigatorio = True)",
            ),
        ),
        // FLX19 — ator inválido na etapa.
        caso(
            Code::InvalidRuleOrActor,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    atores = [\"cliente\"],\n",
            ),
        ),
        // FLX20 — tipo de campo desconhecido.
        caso(
            Code::UnknownType,
            BASE.replace(
                "campo(\"nome\", \"Nome\", obrigatorio = True)",
                "campo(\"nome\", \"Nome\", tipo = \"telefone\", obrigatorio = True)",
            ),
        ),
        // FLX21 — combinador malformado (campo sem `etapa.`).
        caso(Code::MalformedCombinator, com_regra("igual(\"nome\", \"x\")")),
        // FLX22 — combinador cita campo inexistente.
        caso(Code::UnknownField, com_regra("igual(\"entrada.cep\", \"x\")")),
        // FLX23 — ciclo entre decisões.
        caso(
            Code::DecisionCycle,
            format!(
                "{BASE}\ndecisao(\n    id = \"a\",\n    se = igual(\"entrada.nome\", \"x\"),\n    entao = \"b\",\n    senao = \"b\",\n)\n\ndecisao(\n    id = \"b\",\n    se = igual(\"entrada.nome\", \"x\"),\n    entao = \"a\",\n    senao = \"a\",\n)\n"
            ),
        ),
        // FLX24 — `def` fora do formato.
        caso(
            Code::InvalidDef,
            format!("{BASE}\ndef minha(dados, extra):\n    return True\n"),
        ),
        // FLX25 — variável com formato inválido.
        caso(
            Code::InvalidVariable,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {cliente.nome}\")],\n",
            ),
        ),
        // FLX26 — `{...}` em texto que não interpola.
        caso(
            Code::ForbiddenInterpolation,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    instrucao = \"Veja {entrada.nome}\",\n",
            ),
        ),
        // FLX27 — acima de 60 nós.
        caso(Code::NodeLimit, fluxo_com_61_nos()),
        // FLX28 — `notificar_whatsapp` com número inválido.
        caso(
            Code::InvalidNumber,
            BASE.replace(
                "exige = [\"nome\"], ",
                "exige = [\"nome\"], efeitos = [notificar_whatsapp(para = \"123\", mensagem = \"Pedido {entrada.nome}\")], ",
            ),
        ),
        // FLX51 — etapa sem nenhuma ação.
        caso(
            Code::StageWithoutAction,
            BASE.replace(
                "    acoes = [acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\", resultado = \"Feito\"))],\n",
                "",
            ),
        ),
        // FLX52 — etapa inalcançável.
        caso(
            Code::Unreachable,
            format!("{BASE}\netapa(\n    id = \"ilha\",\n    nome = \"Ilha\",\n    acoes = [acao(\"sair\", \"Sair\", vai_para = fim(\"ilha_fim\"))],\n)\n"),
        ),
        // FLX53 — campo que ninguém usa.
        caso(Code::UnusedField, BASE.replace("exige = [\"nome\"], ", "")),
        // FLX54 — comparação com valor fora das opções.
        caso(
            Code::OutsideOptions,
            format!(
                "{}\ndecisao(\n    id = \"escolha\",\n    se = igual(\"entrada.nome\", \"talvez\"),\n    entao = \"entrada\",\n    senao = \"entrada\",\n)\n",
                BASE.replace(
                    "campo(\"nome\", \"Nome\", obrigatorio = True)",
                    "campo(\"nome\", \"Nome\", tipo = \"opcao\", opcoes = [\"sim\", \"nao\"], obrigatorio = True)",
                )
            ),
        ),
        // FLX55 — condição que sempre dá o mesmo resultado.
        caso(
            Code::ContradictoryCondition,
            com_regra("e(igual(\"entrada.nome\", \"x\"), diferente(\"entrada.nome\", \"x\"))"),
        ),
        // FLX56 — gatilho sem exemplos.
        caso(
            Code::TriggerWithoutExamples,
            BASE.replace("        exemplos = [\"quero testar\"],\n", ""),
        ),
        // FLX57 — gatilho sem `nao_quando`.
        caso(
            Code::TriggerWithoutNegative,
            BASE.replace("        nao_quando = \"O cliente nao quer testar.\",\n", ""),
        ),
        // FLX58 — notificação sem variável.
        caso(
            Code::NotificationWithoutVariable,
            BASE.replace(
                "exige = [\"nome\"], ",
                "exige = [\"nome\"], efeitos = [notificar_whatsapp(para = \"5521999990000\", mensagem = \"Pedido entregue\")], ",
            ),
        ),
        // FLX59 — variável de campo que não existe no fluxo.
        caso(
            Code::VariableWithoutField,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {entrada.cep}\")],\n",
            ),
        ),
        // FLX60 — `maior` em campo que não é número.
        caso(Code::TextComparison, com_regra("maior(\"entrada.nome\", \"10\")")),
        // FLX61 — `def` definida e não usada.
        caso(
            Code::UnusedDef,
            format!("{BASE}\ndef minha(dados):\n    return True\n"),
        ),
        // FLX62 — ator da etapa sem nenhuma ação dele.
        caso(
            Code::ActorWithoutAction,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    atores = [\"agente\", \"atendente\"],\n",
            )
            .replace(
                "acao(\"seguir\", \"Seguir\",",
                "acao(\"seguir\", \"Seguir\", ator = \"agente\",",
            ),
        ),
        // FLX63 — `ao_entrar` usa variável do próprio passo.
        caso(
            Code::VariableFromOwnStep,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {entrada.nome}\")],\n",
            ),
        ),
        // FLX64 — texto acima do recomendado.
        caso(
            Code::LongText,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                &format!("    nome = \"Entrada\",\n    instrucao = \"{}\",\n", "a".repeat(1200)),
            ),
        ),
    ]
}

/// 31 etapas + 31 fins = 62 nós (o teto é 60).
fn fluxo_com_61_nos() -> String {
    let mut fonte = String::from(
        "fluxo(\n    id = \"grande\",\n    nome = \"Grande\",\n    inicio = \"e0\",\n    gatilho = gatilho(\n        quando = \"O cliente quer testar.\",\n        nao_quando = \"O cliente nao quer testar.\",\n        exemplos = [\"quero testar\"],\n    ),\n)\n\n",
    );
    for indice in 0..31 {
        fonte.push_str(&format!(
            "etapa(\n    id = \"e{indice}\",\n    nome = \"Etapa {indice}\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = fim(\"f{indice}\"))],\n)\n\n"
        ));
    }
    fonte
}

#[test]
fn def_com_nome_de_declaracao_e_recusada_com_posicao() {
    // O caso que o FX-10 deixou sem teste: `def` com nome que já é declaração da linguagem. Antes
    // da FX-10 isso estourava em tempo de carga, com traceback; agora é erro de forma, no lugar.
    for nome in ["etapa", "fluxo"] {
        let fonte = format!("{BASE}\ndef {nome}(dados):\n    return True\n");
        let compilado = compila(&fonte);
        let diagnostico = compilado
            .diagnosticos
            .iter()
            .find(|d| d.code == Code::InvalidDef)
            .unwrap_or_else(|| {
                panic!(
                    "`def {nome}(dados)` devia dar FLX24; diagnósticos: {:?}",
                    compilado.diagnosticos
                )
            });
        assert_eq!(
            (diagnostico.line, diagnostico.column),
            (Some(19), Some(1)),
            "`def {nome}` tem de apontar o começo da declaração: {diagnostico}"
        );
        assert!(
            diagnostico.mensagem.contains(nome),
            "a mensagem tem de dizer qual é o nome reservado: {}",
            diagnostico.mensagem
        );
        assert!(compilado.ir.is_none(), "erro de forma não gera IR");
    }
}

#[test]
fn o_fluxo_base_nao_tem_nenhum_diagnostico() {
    let compilado = compila(BASE);
    assert!(
        compilado.diagnosticos.is_empty(),
        "o fluxo base tem de passar limpo: {:?}",
        compilado.diagnosticos
    );
    assert!(compilado.ir.is_some(), "o fluxo base tem de gerar IR");
}

#[test]
fn cada_codigo_da_lista_fechada_tem_um_caso() {
    let mut vistos: Vec<Code> = casos().iter().map(|caso| caso.codigo).collect();
    let total = vistos.len();
    vistos.sort_unstable();
    vistos.dedup();
    assert_eq!(vistos.len(), total, "dois casos para o mesmo código");
    assert_eq!(
        vistos.len(),
        40,
        "a lista fechada tem 40 códigos do compilador"
    );
    for codigo in Code::from_compiler() {
        assert!(vistos.contains(&codigo), "{codigo} sem caso que o dispare");
    }
}

#[test]
fn cada_diagnostico_aponta_a_linha_e_a_coluna_do_trecho() {
    let mut falhas: Vec<String> = Vec::new();
    for caso in casos() {
        let compilado = compilar(&caso.fonte, &caso.limites);
        let Some(diagnostico) = compilado
            .diagnosticos
            .iter()
            .find(|d| d.code == caso.codigo)
        else {
            falhas.push(format!(
                "{} não apareceu (diagnósticos: {:?})",
                caso.codigo, compilado.diagnosticos
            ));
            continue;
        };
        if (diagnostico.line, diagnostico.column) != (Some(caso.linha), Some(caso.coluna)) {
            falhas.push(format!(
                "{} em {:?}:{:?} (esperado {}:{}); trecho: {}",
                caso.codigo,
                diagnostico.line,
                diagnostico.column,
                caso.linha,
                caso.coluna,
                diagnostico.mensagem
            ));
        }
    }
    assert!(falhas.is_empty(), "\n{}", falhas.join("\n"));
}

#[test]
fn o_fluxo_base_nao_dispara_nenhum_dos_40_codigos() {
    // O outro lado de cada caso: um fluxo correto não recebe nenhum dos diagnósticos.
    let compilado = compila(BASE);
    for codigo in Code::from_compiler() {
        assert!(
            !compilado.diagnosticos.iter().any(|d| d.code == codigo),
            "{codigo} não devia aparecer num fluxo correto"
        );
    }
}

#[test]
fn o_erro_bloqueia_a_ir_e_o_aviso_nao() {
    for caso in casos() {
        let compilado = compilar(&caso.fonte, &caso.limites);
        let erro = !matches!(
            caso.codigo,
            Code::StageWithoutAction
                | Code::Unreachable
                | Code::UnusedField
                | Code::OutsideOptions
                | Code::ContradictoryCondition
                | Code::TriggerWithoutExamples
                | Code::TriggerWithoutNegative
                | Code::NotificationWithoutVariable
                | Code::VariableWithoutField
                | Code::TextComparison
                | Code::UnusedDef
                | Code::ActorWithoutAction
                | Code::VariableFromOwnStep
                | Code::LongText
        );
        assert_eq!(
            compilado.tem_erro(),
            erro,
            "{} deveria {}bloquear a publicação",
            caso.codigo,
            if erro { "" } else { "não " }
        );
        if erro {
            assert!(
                compilado.ir.is_none(),
                "{} é erro, então não pode sair IR",
                caso.codigo
            );
        }
    }
}

/// Inventário para conferir as posições à mão: `cargo test -p poph-fluxos -- --ignored inventario`.
#[test]
#[ignore]
fn inventario() {
    for caso in casos() {
        let compilado = compilar(&caso.fonte, &caso.limites);
        match compilado
            .diagnosticos
            .iter()
            .find(|d| d.code == caso.codigo)
        {
            Some(d) => {
                let linha = caso
                    .fonte
                    .lines()
                    .nth(d.line.unwrap_or(1) as usize - 1)
                    .unwrap_or("<fora do texto>");
                println!(
                    "{} => {:?}:{:?} | {}\n    {}",
                    caso.codigo, d.line, d.column, d.mensagem, linha
                );
            }
            None => println!(
                "{} => AUSENTE (tem {:?})",
                caso.codigo,
                compilado
                    .diagnosticos
                    .iter()
                    .map(|d| d.code)
                    .collect::<Vec<_>>()
            ),
        }
    }
}

// ------------------------------------------------------------------ exemplos e determinismo

#[test]
fn os_tres_exemplos_compilam_para_a_ir_dourada() {
    for nome in [
        "agendar_visita.star",
        "triagem_suporte.star",
        "confirmar_entrega.star",
    ] {
        let fonte = exemplo(nome);
        let compilado = compila(&fonte);
        assert!(
            compilado.diagnosticos.is_empty(),
            "{nome} tem diagnóstico: {:?}",
            compilado.diagnosticos
        );
        let ir = compilado.ir.expect("o exemplo tem de gerar IR");
        let caminho = format!(
            "{}/../../tests/fluxos/ir/{}.json",
            env!("CARGO_MANIFEST_DIR"),
            nome.trim_end_matches(".star")
        );
        let dourada = std::fs::read_to_string(&caminho).unwrap_or_else(|erro| {
            panic!(
                "{caminho}: {erro} — rode `cargo test -p poph-fluxos -- --ignored gera_douradas`"
            )
        });
        let esperada = serde_json::from_str(&dourada).expect("a dourada é JSON da IR");
        assert_eq!(ir, esperada, "a IR de {nome} não bate com a dourada");
        // A forma compacta é a que vai para `definicao_json`: tem de ser estável e igual.
        assert_eq!(
            ir.to_json(),
            esperada_json(&caminho),
            "a forma compacta de {nome} não é a gravada"
        );
    }
}

/// A IR dourada, lida como texto compacto (é o que a comparação de `carregar` usa).
fn esperada_json(caminho: &str) -> String {
    let texto = std::fs::read_to_string(caminho).expect("dourada legível");
    let ir: poph_fluxos::Ir = serde_json::from_str(&texto).expect("dourada é IR");
    ir.to_json()
}

/// Regrava as douradas. Roda só à mão: `cargo test -p poph-fluxos -- --ignored gera_douradas`.
#[test]
#[ignore]
fn gera_douradas() {
    for nome in [
        "agendar_visita.star",
        "triagem_suporte.star",
        "confirmar_entrega.star",
    ] {
        let fonte = exemplo(nome);
        let compilado = compila(&fonte);
        assert!(
            compilado.diagnosticos.is_empty(),
            "{nome} tem diagnóstico: {:?}",
            compilado.diagnosticos
        );
        let ir = compilado.ir.expect("o exemplo tem de gerar IR");
        let caminho = format!(
            "{}/../../tests/fluxos/ir/{}.json",
            env!("CARGO_MANIFEST_DIR"),
            nome.trim_end_matches(".star")
        );
        std::fs::create_dir_all(
            std::path::Path::new(&caminho)
                .parent()
                .expect("a dourada tem pasta"),
        )
        .expect("criar a pasta das douradas");
        std::fs::write(&caminho, serde_json::to_string_pretty(&ir).unwrap() + "\n")
            .unwrap_or_else(|erro| panic!("{caminho}: {erro}"));
        println!("gravada {caminho}");
    }
}

#[test]
fn duas_compilacoes_produzem_a_mesma_ir_byte_a_byte() {
    for nome in ["triagem_suporte.star", "confirmar_entrega.star"] {
        let fonte = exemplo(nome);
        let primeira = compila(&fonte).ir.expect("IR").to_json();
        let segunda = compila(&fonte).ir.expect("IR").to_json();
        assert_eq!(primeira, segunda, "{nome} não é determinístico");
    }
}

#[test]
fn duas_threads_compilam_o_mesmo_texto_com_a_mesma_ir() {
    let fonte = exemplo("triagem_suporte.star");
    let esperado = compila(&fonte).ir.expect("IR").to_json();
    let resultados: Vec<String> = std::thread::scope(|escopo| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let fonte = fonte.clone();
                escopo.spawn(move || compila(&fonte).ir.expect("IR").to_json())
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread sem pânico"))
            .collect()
    });
    for resultado in resultados {
        assert_eq!(resultado, esperado, "a IR mudou entre threads");
    }
}

// ------------------------------------------------------------------ limites e abuso

/// Cada script patológico termina com erro, rápido e sem estourar a memória do processo.
#[test]
fn script_patologico_termina_com_erro_e_sem_travar() {
    let casos: Vec<(&str, String)> = vec![
        ("load", format!("load(\"outro.star\", \"x\")\n{BASE}")),
        (
            "while",
            format!("{BASE}\ndef gira(dados):\n    while True:\n        pass\n    return True\n"),
        ),
        (
            "recursao",
            format!("{BASE}\ndef gira(dados):\n    return gira(dados)\n"),
        ),
        (
            "alocacao monstro",
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    instrucao = \"x\" * 1000000000,\n",
            ),
        ),
        (
            "relogio",
            format!("{BASE}\ndef agora(dados):\n    return time.time() > 0\n"),
        ),
        (
            "arquivo",
            format!("{BASE}\ndef ler(dados):\n    return open(\"/etc/passwd\").read() != \"\"\n"),
        ),
        (
            "rede",
            format!("{BASE}\ndef baixar(dados):\n    return socket.gethostname() != \"\"\n"),
        ),
        (
            "aleatorio",
            format!("{BASE}\ndef sorte(dados):\n    return random.random() > 0.5\n"),
        ),
    ];
    for (nome, fonte) in casos {
        let inicio = Instant::now();
        let compilado = compila(&fonte);
        let gasto = inicio.elapsed();
        assert!(
            compilado.tem_erro(),
            "o caso `{nome}` tinha de terminar com erro, e passou"
        );
        assert!(
            gasto < Duration::from_secs(2),
            "o caso `{nome}` demorou {gasto:?}"
        );
    }
}

/// Entrada estranha vira diagnóstico, nunca pânico.
#[test]
fn entrada_estranha_nao_derruba_o_compilador() {
    let casos = [
        "",
        "   \n\n\t\n",
        "# só comentário\n",
        "def minha(dados):\n    return True\n",
        "fluxo()\n",
        "fluxo(gatilho = \"x\")\n",
        "etapa()\n",
        "decisao()\n",
        "\\u{feff}fluxo(\n",
        "fluxo(\n    id = \"abc\",\n    nome = \"A\",\n    inicio = \"et\",\n    gatilho = gatilho(quando = \"q\"),\n)\netapa(\n    id = \"et\",\n    nome = \"E\",\n    acoes = [acao(\"ir\", \"Ir\", vai_para = fim(\"fim_ok\"))],\n)\n",
    ];
    for fonte in casos {
        let compilado = compila(fonte);
        let _ = compilado.ir();
        // O que importa: voltou (não entrou em pânico) e, se tem IR, o JSON é legível de volta.
        if let Some(ir) = &compilado.ir {
            let lida = poph_fluxos::Ir::from_json(&ir.to_json()).expect("a IR volta do JSON");
            assert_eq!(&lida, ir, "ida e volta da IR em {fonte:?}");
        }
    }
    // A última entrada é um fluxo válido: tem de gerar IR.
    assert!(compila(casos[casos.len() - 1]).ir.is_some());
    // Fonte vazia não declara fluxo.
    assert!(
        compila("")
            .diagnosticos
            .iter()
            .any(|d| d.code == Code::MissingFlow)
    );
}

#[test]
fn fonte_grande_e_recusada_antes_de_compilar() {
    let fonte = format!("{BASE}\n{}", "#".repeat(MAX_FONTE_BYTES));
    let compilado = compila(&fonte);
    assert!(compilado.tem_erro());
    assert_eq!(compilado.diagnosticos[0].code, Code::SourceTooLarge);
    assert!(compilado.ir.is_none());
}

#[test]
fn os_limites_de_carga_estao_ligados_de_verdade() {
    // Prazo zero: a carga é cancelada e vira FLX04 (é o caminho de emergência, medido no FX-01).
    let apertado = Limites {
        prazo_de_carga: Duration::ZERO,
        ..Limites::default()
    };
    let compilado = compilar(BASE, &apertado);
    assert!(
        compilado
            .diagnosticos
            .iter()
            .any(|d| d.code == Code::LoadLimit),
        "prazo zero tinha de estourar o limite de carga: {:?}",
        compilado.diagnosticos
    );
    // E o normal passa no mesmo script.
    assert!(compila(BASE).ir.is_some());
}

// ------------------------------------------------------------------ carregar (ir_divergente)

#[test]
fn carregar_aceita_a_ir_que_veio_do_proprio_texto() {
    let fonte = exemplo("agendar_visita.star");
    let ir = compila(&fonte).ir.expect("IR");
    let carregada = carregar(&ir, &fonte, &limites()).expect("a IR do texto tem de carregar");
    assert_eq!(carregada, ir);
}

#[test]
fn carregar_recusa_ir_adulterada_com_ir_divergente() {
    let fonte = exemplo("agendar_visita.star");
    let mut ir = compila(&fonte).ir.expect("IR");
    // Adultera a mensagem de uma etapa: a IR gravada deixa de ser o que o texto diz.
    ir.stages[1].actions[0].label = "Visita combinada (adulterado)".to_owned();
    match carregar(&ir, &fonte, &limites()) {
        Err(FalhaCarregamento::Divergente { fluxo, motivo }) => {
            assert_eq!(fluxo, "agendar_visita");
            assert!(
                motivo.contains("as etapas"),
                "o motivo tem de dizer o que divergiu: {motivo}"
            );
        }
        outro => panic!("devia falhar com ir_divergente, e voltou {outro:?}"),
    }
}

#[test]
fn carregar_recusa_texto_que_nao_compila() {
    let fonte = exemplo("agendar_visita.star");
    let ir = compila(&fonte).ir.expect("IR");
    let quebrado = fonte.replace("inicio = \"coleta\",", "");
    match carregar(&ir, &quebrado, &limites()) {
        Err(FalhaCarregamento::Forma(diagnosticos)) => {
            assert!(
                diagnosticos
                    .iter()
                    .any(|d| d.code == Code::MissingArgument),
                "esperava FLX11: {diagnosticos:?}"
            );
            assert_eq!(
                FalhaCarregamento::Forma(diagnosticos).codigo(),
                "fluxo_invalido"
            );
        }
        outro => panic!("devia falhar por forma, e voltou {outro:?}"),
    }
}

#[test]
fn a_ir_ignora_campo_desconhecido_na_leitura() {
    // Mudança aditiva: uma IR de versão mais nova tem campo que este crate não conhece.
    let fonte = exemplo("agendar_visita.star");
    let ir = compila(&fonte).ir.expect("IR");
    let mut json: serde_json::Value = serde_json::from_str(&ir.to_json()).expect("JSON");
    json["campo_do_futuro"] = serde_json::json!({"qualquer": "coisa"});
    let lida = poph_fluxos::Ir::from_json(&json.to_string()).expect("campo desconhecido é ignorado");
    assert_eq!(lida, ir);
}
