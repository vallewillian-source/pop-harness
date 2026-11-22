//! Testes do compilador de fluxos (FX-10): os três exemplos dourados, um caso por diagnóstico de
//! forma (com linha e coluna), limites, determinismo e `carregar` recusando IR adulterada.

use std::time::{Duration, Instant};

use poph_fluxos::compilador::MAX_FONTE_BYTES;
use poph_fluxos::{Codigo, Compilado, FalhaCarregamento, Limites, carregar, compilar};

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
    codigo: Codigo,
    fonte: String,
    limites: Limites,
    linha: u32,
    coluna: u32,
}

/// Onde o diagnóstico de cada código tem de aparecer. Conferido à mão contra o texto de cada caso
/// (o ponto é sempre o trecho que o autor tem de corrigir).
fn posicoes() -> Vec<(Codigo, u32, u32)> {
    vec![
        (Codigo::Sintaxe, 3, 1),
        (Codigo::FonteGrande, 1, 1),
        (Codigo::Dialeto, 1, 1),
        (Codigo::LimiteDeCarga, 1, 1),
        (Codigo::TopoInvalido, 1, 1),
        (Codigo::TipoErrado, 15, 51),
        (Codigo::NomeInexistente, 20, 14),
        (Codigo::FluxoAusente, 1, 1),
        (Codigo::ArgumentoAusente, 1, 1),
        (Codigo::ArgumentoDesconhecido, 15, 52),
        (Codigo::IdInvalido, 2, 10),
        (Codigo::IdRepetido, 20, 10),
        (Codigo::DestinoInexistente, 16, 68),
        (Codigo::FimDivergente, 22, 46),
        (Codigo::ExigeInexistente, 16, 47),
        (Codigo::OpcoesInvalidas, 15, 21),
        (Codigo::RegraOuAtorInvalido, 15, 14),
        (Codigo::TipoDesconhecido, 15, 44),
        (Codigo::CombinadorMalformado, 21, 16),
        (Codigo::CampoInexistente, 21, 16),
        (Codigo::CicloDeDecisao, 19, 1),
        (Codigo::DefInvalida, 19, 1),
        (Codigo::VariavelInvalida, 15, 38),
        (Codigo::InterpolacaoProibida, 15, 17),
        (Codigo::NosDemais, 1, 1),
        (Codigo::NumeroInvalido, 16, 94),
        (Codigo::EtapaSemAcao, 12, 1),
        (Codigo::Inalcancavel, 19, 1),
        (Codigo::CampoNaoUsado, 15, 21),
        (Codigo::ForaDasOpcoes, 21, 16),
        (Codigo::CondicaoContraditoria, 21, 10),
        (Codigo::GatilhoSemExemplos, 5, 15),
        (Codigo::GatilhoSemNegativo, 5, 15),
        (Codigo::NotificacaoSemVariavel, 16, 122),
        (Codigo::VariavelSemCampo, 15, 38),
        (Codigo::ComparacaoDeTexto, 21, 16),
        (Codigo::DefNaoUsada, 19, 1),
        (Codigo::AtorSemAcao, 15, 14),
        (Codigo::VariavelDoProprioPasso, 15, 38),
        (Codigo::TextoLongo, 15, 17),
    ]
}

fn posicao_esperada(codigo: Codigo) -> (u32, u32) {
    posicoes()
        .into_iter()
        .find(|(c, _, _)| *c == codigo)
        .map(|(_, linha, coluna)| (linha, coluna))
        .unwrap_or_else(|| panic!("{codigo} sem posição esperada"))
}

/// Um caso simples, com os limites padrão.
fn caso(codigo: Codigo, fonte: impl Into<String>) -> Caso {
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
    let (linha_carga, coluna_carga) = posicao_esperada(Codigo::LimiteDeCarga);
    vec![
        // FLX01 — sintaxe inválida.
        caso(Codigo::Sintaxe, "fluxo(\n    id = \"teste\",\n"),
        // FLX02 — fonte acima de 32 KB.
        caso(Codigo::FonteGrande, "#".repeat(MAX_FONTE_BYTES + 1)),
        // FLX03 — construção fora do dialeto.
        caso(
            Codigo::Dialeto,
            format!("load(\"outro.star\", \"ajuda\")\n{BASE}"),
        ),
        // FLX04 — limite de carga estourado (heap apertado de propósito).
        Caso {
            codigo: Codigo::LimiteDeCarga,
            fonte: BASE.to_owned(),
            limites: Limites {
                max_heap_bytes: 1,
                ..limites()
            },
            linha: linha_carga,
            coluna: coluna_carga,
        },
        // FLX05 — item no topo que não é declaração.
        caso(Codigo::TopoInvalido, format!("contador = 0\n{BASE}")),
        // FLX06 — valor com tipo errado.
        caso(
            Codigo::TipoErrado,
            BASE.replace("obrigatorio = True", "obrigatorio = \"sim\""),
        ),
        // FLX07 — nome que não existe no dialeto.
        caso(
            Codigo::NomeInexistente,
            format!("{BASE}\ndef minha(dados):\n    aberto = open(\"x.txt\")\n    return len(aberto) > 0\n"),
        ),
        // FLX10 — falta `fluxo(...)`.
        caso(
            Codigo::FluxoAusente,
            "etapa(\n    id = \"entrada\",\n    nome = \"Entrada\",\n)\n",
        ),
        // FLX11 — argumento obrigatório ausente.
        caso(
            Codigo::ArgumentoAusente,
            BASE.replace("    inicio = \"entrada\",\n", ""),
        ),
        // FLX12 — argumento desconhecido.
        caso(
            Codigo::ArgumentoDesconhecido,
            BASE.replace("obrigatorio = True", "obrigatorioo = True"),
        ),
        // FLX13 — id fora do formato.
        caso(
            Codigo::IdInvalido,
            BASE.replace("id = \"teste\"", "id = \"Teste Ruim\""),
        ),
        // FLX14 — id repetido.
        caso(
            Codigo::IdRepetido,
            format!("{BASE}\netapa(\n    id = \"entrada\",\n    nome = \"Repetida\",\n    acoes = [acao(\"outra\", \"Outra\", vai_para = fim(\"repetida\"))],\n)\n"),
        ),
        // FLX15 — destino inexistente.
        caso(
            Codigo::DestinoInexistente,
            BASE.replace(
                "vai_para = fim(\"pronto\", resultado = \"Feito\")",
                "vai_para = \"nao_existe\"",
            ),
        ),
        // FLX16 — mesmo `fim` com argumentos diferentes.
        caso(
            Codigo::FimDivergente,
            format!("{BASE}\netapa(\n    id = \"outra\",\n    nome = \"Outra\",\n    acoes = [acao(\"sair\", \"Sair\", vai_para = fim(\"pronto\", resultado = \"Outro\"))],\n)\n"),
        ),
        // FLX17 — `exige` cita campo que não existe na etapa.
        caso(
            Codigo::ExigeInexistente,
            BASE.replace("exige = [\"nome\"]", "exige = [\"cep\"]"),
        ),
        // FLX18 — `opcao` sem opções.
        caso(
            Codigo::OpcoesInvalidas,
            BASE.replace(
                "campo(\"nome\", \"Nome\", obrigatorio = True)",
                "campo(\"nome\", \"Nome\", tipo = \"opcao\", obrigatorio = True)",
            ),
        ),
        // FLX19 — ator inválido na etapa.
        caso(
            Codigo::RegraOuAtorInvalido,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    atores = [\"cliente\"],\n",
            ),
        ),
        // FLX20 — tipo de campo desconhecido.
        caso(
            Codigo::TipoDesconhecido,
            BASE.replace(
                "campo(\"nome\", \"Nome\", obrigatorio = True)",
                "campo(\"nome\", \"Nome\", tipo = \"telefone\", obrigatorio = True)",
            ),
        ),
        // FLX21 — combinador malformado (campo sem `etapa.`).
        caso(Codigo::CombinadorMalformado, com_regra("igual(\"nome\", \"x\")")),
        // FLX22 — combinador cita campo inexistente.
        caso(Codigo::CampoInexistente, com_regra("igual(\"entrada.cep\", \"x\")")),
        // FLX23 — ciclo entre decisões.
        caso(
            Codigo::CicloDeDecisao,
            format!(
                "{BASE}\ndecisao(\n    id = \"a\",\n    se = igual(\"entrada.nome\", \"x\"),\n    entao = \"b\",\n    senao = \"b\",\n)\n\ndecisao(\n    id = \"b\",\n    se = igual(\"entrada.nome\", \"x\"),\n    entao = \"a\",\n    senao = \"a\",\n)\n"
            ),
        ),
        // FLX24 — `def` fora do formato.
        caso(
            Codigo::DefInvalida,
            format!("{BASE}\ndef minha(dados, extra):\n    return True\n"),
        ),
        // FLX25 — variável com formato inválido.
        caso(
            Codigo::VariavelInvalida,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {cliente.nome}\")],\n",
            ),
        ),
        // FLX26 — `{...}` em texto que não interpola.
        caso(
            Codigo::InterpolacaoProibida,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    instrucao = \"Veja {entrada.nome}\",\n",
            ),
        ),
        // FLX27 — acima de 60 nós.
        caso(Codigo::NosDemais, fluxo_com_61_nos()),
        // FLX28 — `notificar_whatsapp` com número inválido.
        caso(
            Codigo::NumeroInvalido,
            BASE.replace(
                "exige = [\"nome\"], ",
                "exige = [\"nome\"], efeitos = [notificar_whatsapp(para = \"123\", mensagem = \"Pedido {entrada.nome}\")], ",
            ),
        ),
        // FLX51 — etapa sem nenhuma ação.
        caso(
            Codigo::EtapaSemAcao,
            BASE.replace(
                "    acoes = [acao(\"seguir\", \"Seguir\", exige = [\"nome\"], vai_para = fim(\"pronto\", resultado = \"Feito\"))],\n",
                "",
            ),
        ),
        // FLX52 — etapa inalcançável.
        caso(
            Codigo::Inalcancavel,
            format!("{BASE}\netapa(\n    id = \"ilha\",\n    nome = \"Ilha\",\n    acoes = [acao(\"sair\", \"Sair\", vai_para = fim(\"ilha_fim\"))],\n)\n"),
        ),
        // FLX53 — campo que ninguém usa.
        caso(Codigo::CampoNaoUsado, BASE.replace("exige = [\"nome\"], ", "")),
        // FLX54 — comparação com valor fora das opções.
        caso(
            Codigo::ForaDasOpcoes,
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
            Codigo::CondicaoContraditoria,
            com_regra("e(igual(\"entrada.nome\", \"x\"), diferente(\"entrada.nome\", \"x\"))"),
        ),
        // FLX56 — gatilho sem exemplos.
        caso(
            Codigo::GatilhoSemExemplos,
            BASE.replace("        exemplos = [\"quero testar\"],\n", ""),
        ),
        // FLX57 — gatilho sem `nao_quando`.
        caso(
            Codigo::GatilhoSemNegativo,
            BASE.replace("        nao_quando = \"O cliente nao quer testar.\",\n", ""),
        ),
        // FLX58 — notificação sem variável.
        caso(
            Codigo::NotificacaoSemVariavel,
            BASE.replace(
                "exige = [\"nome\"], ",
                "exige = [\"nome\"], efeitos = [notificar_whatsapp(para = \"5521999990000\", mensagem = \"Pedido entregue\")], ",
            ),
        ),
        // FLX59 — variável de campo que não existe no fluxo.
        caso(
            Codigo::VariavelSemCampo,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {entrada.cep}\")],\n",
            ),
        ),
        // FLX60 — `maior` em campo que não é número.
        caso(Codigo::ComparacaoDeTexto, com_regra("maior(\"entrada.nome\", \"10\")")),
        // FLX61 — `def` definida e não usada.
        caso(
            Codigo::DefNaoUsada,
            format!("{BASE}\ndef minha(dados):\n    return True\n"),
        ),
        // FLX62 — ator da etapa sem nenhuma ação dele.
        caso(
            Codigo::AtorSemAcao,
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
            Codigo::VariavelDoProprioPasso,
            BASE.replace(
                "    nome = \"Entrada\",\n",
                "    nome = \"Entrada\",\n    ao_entrar = [mensagem_ao_cliente(\"Olá {entrada.nome}\")],\n",
            ),
        ),
        // FLX64 — texto acima do recomendado.
        caso(
            Codigo::TextoLongo,
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
    let mut vistos: Vec<Codigo> = casos().iter().map(|caso| caso.codigo).collect();
    let total = vistos.len();
    vistos.sort_unstable();
    vistos.dedup();
    assert_eq!(vistos.len(), total, "dois casos para o mesmo código");
    assert_eq!(vistos.len(), 40, "a lista fechada tem 40 códigos");
    for codigo in Codigo::TODOS {
        assert!(vistos.contains(codigo), "{codigo} sem caso que o dispare");
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
            .find(|d| d.codigo == caso.codigo)
        else {
            falhas.push(format!(
                "{} não apareceu (diagnósticos: {:?})",
                caso.codigo, compilado.diagnosticos
            ));
            continue;
        };
        if diagnostico.linha != caso.linha || diagnostico.coluna != caso.coluna {
            falhas.push(format!(
                "{} em {}:{} (esperado {}:{}); trecho: {}",
                caso.codigo,
                diagnostico.linha,
                diagnostico.coluna,
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
    for codigo in Codigo::TODOS {
        assert!(
            !compilado.diagnosticos.iter().any(|d| d.codigo == *codigo),
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
            Codigo::EtapaSemAcao
                | Codigo::Inalcancavel
                | Codigo::CampoNaoUsado
                | Codigo::ForaDasOpcoes
                | Codigo::CondicaoContraditoria
                | Codigo::GatilhoSemExemplos
                | Codigo::GatilhoSemNegativo
                | Codigo::NotificacaoSemVariavel
                | Codigo::VariavelSemCampo
                | Codigo::ComparacaoDeTexto
                | Codigo::DefNaoUsada
                | Codigo::AtorSemAcao
                | Codigo::VariavelDoProprioPasso
                | Codigo::TextoLongo
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
            .find(|d| d.codigo == caso.codigo)
        {
            Some(d) => {
                let linha = caso
                    .fonte
                    .lines()
                    .nth(d.linha as usize - 1)
                    .unwrap_or("<fora do texto>");
                println!(
                    "{} => {}:{} | {}\n    {}",
                    caso.codigo, d.linha, d.coluna, d.mensagem, linha
                );
            }
            None => println!(
                "{} => AUSENTE (tem {:?})",
                caso.codigo,
                compilado
                    .diagnosticos
                    .iter()
                    .map(|d| d.codigo)
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
            ir.para_json(),
            esperada_json(&caminho),
            "a forma compacta de {nome} não é a gravada"
        );
    }
}

/// A IR dourada, lida como texto compacto (é o que a comparação de `carregar` usa).
fn esperada_json(caminho: &str) -> String {
    let texto = std::fs::read_to_string(caminho).expect("dourada legível");
    let ir: poph_fluxos::Ir = serde_json::from_str(&texto).expect("dourada é IR");
    ir.para_json()
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
        let primeira = compila(&fonte).ir.expect("IR").para_json();
        let segunda = compila(&fonte).ir.expect("IR").para_json();
        assert_eq!(primeira, segunda, "{nome} não é determinístico");
    }
}

#[test]
fn duas_threads_compilam_o_mesmo_texto_com_a_mesma_ir() {
    let fonte = exemplo("triagem_suporte.star");
    let esperado = compila(&fonte).ir.expect("IR").para_json();
    let resultados: Vec<String> = std::thread::scope(|escopo| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let fonte = fonte.clone();
                escopo.spawn(move || compila(&fonte).ir.expect("IR").para_json())
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
            let lida = poph_fluxos::Ir::de_json(&ir.para_json()).expect("a IR volta do JSON");
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
            .any(|d| d.codigo == Codigo::FluxoAusente)
    );
}

#[test]
fn fonte_grande_e_recusada_antes_de_compilar() {
    let fonte = format!("{BASE}\n{}", "#".repeat(MAX_FONTE_BYTES));
    let compilado = compila(&fonte);
    assert!(compilado.tem_erro());
    assert_eq!(compilado.diagnosticos[0].codigo, Codigo::FonteGrande);
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
            .any(|d| d.codigo == Codigo::LimiteDeCarga),
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
    ir.etapas[1].acoes[0].rotulo = "Visita combinada (adulterado)".to_owned();
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
                    .any(|d| d.codigo == Codigo::ArgumentoAusente),
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
    let mut json: serde_json::Value = serde_json::from_str(&ir.para_json()).expect("JSON");
    json["campo_do_futuro"] = serde_json::json!({"qualquer": "coisa"});
    let lida = poph_fluxos::Ir::de_json(&json.to_string()).expect("campo desconhecido é ignorado");
    assert_eq!(lida, ir);
}
