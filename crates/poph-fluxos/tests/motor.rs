//! Testes do motor de execução (FX-13): iniciar, registrar campos, executar ação e avançar.
//!
//! Cada item do "pronto quando" do bloco FX-13 tem o seu teste aqui, e cada teste tem de cair
//! quando o comportamento é revertido (as mutações estão no relatório de entrega). As IRs são
//! montadas à mão para o teste poder criar o caso que precisa (inclusive os que a publicação
//! recusaria, como uma etapa de destino que não existe); um dos testes roda a IR dourada de
//! `agendar_visita`, que passou pelo compilador.

use poph_fluxos::ir::{Acao, Campo, Destino, Etapa, Fim, Fluxo, Gatilho, Ir};
use poph_fluxos::motor::{
    Campos, Contexto, Estado, atores_exigidos, campos_faltando, executar_acao, iniciar,
};
use poph_fluxos::regras::Avaliador;
use poph_types::{AtorAcao, DomainError, StatusExecucao, TipoCampo, TipoEventoFicha};
use serde_json::{Value as Json, json};
use std::sync::OnceLock;

// ------------------------------------------------------------------ auxiliares
/// Contexto das chamadas do motor: sem Ficha, sem cliente e com o avaliador só de combinadores.
///
/// Os testes de decisão montam o seu com o avaliador que tem as `def` do fluxo.
fn contexto() -> Contexto<'static> {
    static AVALIADOR: OnceLock<Avaliador> = OnceLock::new();
    Contexto::novo(AVALIADOR.get_or_init(Avaliador::sem_funcoes))
}

fn campo(id: &str, tipo: TipoCampo, obrigatorio: bool, opcoes: &[&str]) -> Campo {
    Campo {
        id: id.to_owned(),
        rotulo: id.to_uppercase(),
        tipo,
        obrigatorio,
        opcoes: opcoes.iter().map(|opcao| (*opcao).to_owned()).collect(),
        ajuda: None,
    }
}

fn acao(id: &str, ator: Option<AtorAcao>, exige: &[&str], vai_para: &str) -> Acao {
    Acao {
        id: id.to_owned(),
        rotulo: id.to_uppercase(),
        exige: exige.iter().map(|nome| (*nome).to_owned()).collect(),
        ator,
        vai_para: Destino::Etapa {
            id: vai_para.to_owned(),
        },
        efeitos: Vec::new(),
    }
}

fn acao_para_fim(id: &str, ator: Option<AtorAcao>) -> Acao {
    Acao {
        id: id.to_owned(),
        rotulo: id.to_uppercase(),
        exige: Vec::new(),
        ator,
        vai_para: Destino::Fim {
            id: "concluido".to_owned(),
            resultado: Some("Concluído".to_owned()),
            encerra_atendimento: false,
        },
        efeitos: Vec::new(),
    }
}

fn etapa(id: &str, atores: &[AtorAcao], campos: Vec<Campo>, acoes: Vec<Acao>) -> Etapa {
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

/// Fluxo de teste com a primeira etapa como `inicio`.
fn fluxo(etapas: Vec<Etapa>) -> Ir {
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
            quando: "o cliente pede".to_owned(),
            nao_quando: None,
            exemplos: Vec::new(),
        },
        inicio,
        etapas,
        decisoes: Vec::new(),
        fins: vec![Fim {
            id: "concluido".to_owned(),
            resultado: Some("Concluído".to_owned()),
            encerra_atendimento: false,
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

/// Fluxo de uma etapa só: `coleta` (agente) com `endereco` obrigatório e a ação `confirmar` que
/// exige o campo e vai para `confirmacao`.
fn fluxo_simples() -> Ir {
    fluxo(vec![
        etapa(
            "coleta",
            &[AtorAcao::Agente],
            vec![campo("endereco", TipoCampo::Texto, true, &[])],
            vec![
                acao("confirmar", None, &["endereco"], "confirmacao"),
                acao("desistir", None, &[], "confirmacao"),
            ],
        ),
        etapa(
            "confirmacao",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ])
}

/// Fluxo com os dois atores na mesma etapa: o agente tem `registrar` e o atendente tem `assumir`,
/// as duas levando para `depois`.
fn fluxo_com_dois_atores() -> Ir {
    fluxo(vec![
        etapa(
            "triagem",
            &[AtorAcao::Agente, AtorAcao::Atendente],
            vec![campo("resumo", TipoCampo::Texto, false, &[])],
            vec![
                acao("registrar", Some(AtorAcao::Agente), &[], "depois"),
                acao("assumir", Some(AtorAcao::Atendente), &[], "depois"),
            ],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ])
}

/// Texto da recusa de ação (`acao_invalida`), para o teste conferir a mensagem que o agente lê.
fn motivo(erro: &DomainError) -> String {
    match erro {
        DomainError::AcaoInvalida { motivo, .. } => motivo.clone(),
        outro => panic!("esperava acao_invalida e veio {outro:?}"),
    }
}

/// Erros da recusa de campos (`campos_invalidos`).
fn erros_de_campos(erro: &DomainError) -> Vec<String> {
    match erro {
        DomainError::CamposDaEtapaInvalidos { erros, .. } => erros.clone(),
        outro => panic!("esperava campos_invalidos e veio {outro:?}"),
    }
}

fn tipos(resultado: &poph_fluxos::motor::Resultado) -> Vec<TipoEventoFicha> {
    resultado.eventos.iter().map(|evento| evento.tipo).collect()
}

// ------------------------------------------------------------------ iniciar

#[test]
fn iniciar_comeca_na_etapa_de_inicio_com_o_estado_vazio() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("a etapa de início existe");

    assert_eq!(estado.fluxo_id, "teste");
    assert_eq!(estado.status, StatusExecucao::Ativa);
    assert_eq!(
        estado.no_atual,
        Destino::Etapa {
            id: "coleta".to_owned()
        }
    );
    assert!(estado.dados.is_empty(), "a execução nova não tem dado");
    assert!(estado.atores_que_agiram.is_empty());
    assert!(estado.chaves.is_empty());
    assert_eq!(estado.resultado, None);
}

#[test]
fn iniciar_recusa_ir_que_aponta_para_etapa_inexistente() {
    // A publicação recusa isto (FX-11), mas uma IR gravada à mão não pode derrubar o processo.
    let mut ir = fluxo_simples();
    ir.inicio = "nao_existe".to_owned();

    let erro = iniciar(&ir).expect_err("a etapa de início não existe");
    assert!(
        matches!(erro, DomainError::IrDivergente { .. }),
        "esperava ir_divergente e veio {erro:?}"
    );
    assert_eq!(erro.codigo_api(), "ir_divergente");
}

// ------------------------------------------------------------------ ação válida avança

#[test]
fn acao_valida_grava_os_campos_e_avanca_para_a_etapa_de_destino() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let resultado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Etapa {
            id: "confirmacao".to_owned()
        }
    );
    assert_eq!(
        resultado.novo_estado.dados.get("coleta"),
        Some(&json!({ "endereco": "Rua das Flores, 10" })),
        "os campos ficam em dados[etapa][campo]"
    );
    assert!(
        resultado.novo_estado.atores_que_agiram.is_empty(),
        "a etapa nova começa sem ator que agiu"
    );
    assert!(resultado.aguardando.is_empty());
    assert!(!resultado.repetida);
    assert!(resultado.efeitos.is_empty(), "efeitos são da FX-14");
    assert_eq!(
        tipos(&resultado),
        vec![
            TipoEventoFicha::CamposPreenchidos,
            TipoEventoFicha::AcaoExecutada,
            TipoEventoFicha::EtapaConcluida,
            TipoEventoFicha::EtapaEntrou,
        ]
    );
    assert_eq!(
        resultado.eventos[3].payload,
        json!({ "etapa": "confirmacao" }),
        "o evento de entrada diz a etapa nova"
    );
}

#[test]
fn dados_das_etapas_anteriores_continuam_na_execucao() {
    let ir = fluxo_simples();
    let mut estado = iniciar(&ir).expect("início");
    estado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua A" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("primeira ação")
    .novo_estado;

    let resultado = executar_acao(
        &ir,
        &estado,
        "ok",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("segunda ação");

    assert_eq!(
        resultado.novo_estado.dados.get("coleta"),
        Some(&json!({ "endereco": "Rua A" })),
        "o dado da etapa anterior não é perdido"
    );
    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Fim {
            id: "concluido".to_owned(),
            resultado: Some("Concluído".to_owned()),
            encerra_atendimento: false,
        },
        "ação que vai para fim deixa o estado no nó do fim (a FX-14 conclui)"
    );
}

#[test]
fn o_texto_do_campo_e_gravado_sem_os_espacos_das_pontas() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let resultado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "  Rua B  " })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.novo_estado.dados.get("coleta"),
        Some(&json!({ "endereco": "Rua B" }))
    );
}

// ------------------------------------------------------------------ campos

#[test]
fn campo_exigido_faltando_devolve_a_lista_do_que_falta() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("`endereco` é exigido");

    let erros = erros_de_campos(&erro);
    assert_eq!(erro.codigo_api(), "campos_invalidos");
    assert!(
        erros
            .iter()
            .any(|erro| erro.contains("`confirmar`") && erro.contains("`endereco`")),
        "a mensagem diz a ação e o campo: {erros:?}"
    );
}

#[test]
fn exige_de_campo_que_a_etapa_nao_declara_avisa_isso() {
    // IR de fora (ou revisão antiga) com `exige` apontando para campo inexistente: o validador
    // recusa isto na publicação (`FLX17`), e o motor diz o que está errado em vez de pedir um dado
    // que a etapa não coleta.
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao("confirmar", None, &["cep"], "depois")],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("`cep` não existe na etapa");

    let erros = erros_de_campos(&erro);
    assert!(
        erros[0].contains("`cep`") && erros[0].contains("não tem"),
        "a mensagem diz que a etapa não declara o campo: {erros:?}"
    );
}

#[test]
fn o_campo_informado_antes_conta_para_o_exige_da_acao_seguinte() {
    // O agente informa o endereço numa chamada e só depois pede a ação que o exige: repetir o
    // dado não pode ser obrigatório.
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[AtorAcao::Agente],
            vec![campo("endereco", TipoCampo::Texto, true, &[])],
            vec![
                acao("anotar", None, &[], "coleta"),
                acao("confirmar", None, &["endereco"], "confirmacao"),
            ],
        ),
        etapa(
            "confirmacao",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ]);
    let mut estado = iniciar(&ir).expect("início");

    estado = executar_acao(
        &ir,
        &estado,
        "anotar",
        &campos(json!({ "endereco": "Rua C" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("anotar o campo")
    .novo_estado;

    let resultado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("o campo já estava gravado");
    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Etapa {
            id: "confirmacao".to_owned()
        }
    );
}

#[test]
fn campo_com_tipo_errado_e_recusado_com_o_que_veio() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[AtorAcao::Agente],
            vec![campo("idade", TipoCampo::Numero, false, &[])],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("fim", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "ok",
        &campos(json!({ "idade": "quarenta" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("número recebeu texto");

    let erros = erros_de_campos(&erro);
    assert_eq!(erros.len(), 1);
    assert!(
        erros[0].contains("idade") && erros[0].contains("número") && erros[0].contains("texto"),
        "a mensagem diz o campo, o esperado e o que veio: {erros:?}"
    );
}

#[test]
fn cada_tipo_de_campo_recusa_o_valor_que_nao_e_dele() {
    let ir = fluxo(vec![
        etapa(
            "cadastro",
            &[AtorAcao::Agente],
            vec![
                campo("texto", TipoCampo::Texto, false, &[]),
                campo("data", TipoCampo::Data, false, &[]),
                campo("booleano", TipoCampo::Booleano, false, &[]),
                campo("anexo", TipoCampo::Anexo, false, &[]),
            ],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("fim", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "ok",
        &campos(json!({
            "texto": 10,
            "data": "22/11/2026",
            "booleano": "sim",
            "anexo": { "tipo": "application/pdf" },
        })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("todos os valores estão no tipo errado");

    let erros = erros_de_campos(&erro);
    assert_eq!(erros.len(), 4, "{erros:?}");
    assert!(erros[0].starts_with("anexo:"), "{erros:?}");
    assert!(erros[1].starts_with("booleano:"), "{erros:?}");
    assert!(erros[2].starts_with("data:"), "{erros:?}");
    assert!(erros[3].starts_with("texto:"), "{erros:?}");
}

#[test]
fn os_valores_validos_de_cada_tipo_sao_gravados() {
    let ir = fluxo(vec![
        etapa(
            "cadastro",
            &[AtorAcao::Agente],
            vec![
                campo("texto", TipoCampo::Texto, false, &[]),
                campo("numero", TipoCampo::Numero, false, &[]),
                campo("data", TipoCampo::Data, false, &[]),
                campo("booleano", TipoCampo::Booleano, false, &[]),
                campo("anexo", TipoCampo::Anexo, false, &[]),
            ],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("fim", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");
    let anexo = json!({ "nome": "laudo.pdf", "tipo": "application/pdf", "tamanho": 1024 });

    let resultado = executar_acao(
        &ir,
        &estado,
        "ok",
        &campos(json!({
            "texto": "ok",
            "numero": 42,
            "data": "2026-11-22",
            "booleano": true,
            "anexo": anexo,
        })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("todos os valores são do tipo certo");

    assert_eq!(
        resultado.novo_estado.dados.get("cadastro"),
        Some(&json!({
            "texto": "ok",
            "numero": 42,
            "data": "2026-11-22",
            "booleano": true,
            "anexo": anexo,
        }))
    );
}

#[test]
fn opcao_fora_da_lista_e_recusada_com_as_opcoes() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[AtorAcao::Agente],
            vec![campo(
                "periodo",
                TipoCampo::Opcao,
                false,
                &["manhã", "tarde"],
            )],
            vec![acao("ok", None, &[], "depois")],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("fim", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "ok",
        &campos(json!({ "periodo": "noite" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("`noite` não está na lista");

    let erros = erros_de_campos(&erro);
    assert!(
        erros[0].contains("`manhã`") && erros[0].contains("`tarde`"),
        "a mensagem lista as opções: {erros:?}"
    );
}

#[test]
fn campo_de_outra_etapa_e_recusado_com_os_campos_da_etapa() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "cep": "20000-000", "endereco": "Rua D" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("`cep` não é campo de `coleta`");

    let erros = erros_de_campos(&erro);
    assert_eq!(erros.len(), 1);
    assert!(
        erros[0].contains("`cep`") && erros[0].contains("`endereco`"),
        "a mensagem cita o campo recusado e os válidos: {erros:?}"
    );
}

#[test]
fn campo_com_o_prefixo_da_etapa_e_recusado_com_a_dica() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "coleta.endereco": "Rua E" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("o valor vem por id de campo, sem prefixo");

    let erros = erros_de_campos(&erro);
    assert!(
        erros[0].contains("sem o prefixo da etapa"),
        "a mensagem ensina a forma certa: {erros:?}"
    );
}

#[test]
fn campo_obrigatorio_nao_aceita_valor_vazio() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "   " })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("campo obrigatório veio vazio");

    assert!(erros_de_campos(&erro)[0].contains("obrigatório"));
}

#[test]
fn campo_opcional_aceita_nulo_e_o_campo_conta_como_vazio() {
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[AtorAcao::Agente],
            vec![campo("bairro", TipoCampo::Texto, false, &[])],
            vec![
                acao("limpar", None, &[], "coleta"),
                acao("confirmar", None, &["bairro"], "depois"),
            ],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("fim", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    // `null` no campo opcional é aceito (limpar)…
    let com_nulo = executar_acao(
        &ir,
        &estado,
        "limpar",
        &campos(json!({ "bairro": Json::Null })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("campo opcional aceita nulo");
    assert_eq!(
        com_nulo.novo_estado.dados.get("coleta"),
        Some(&json!({ "bairro": Json::Null }))
    );

    // …e não conta como preenchido para o `exige` da ação seguinte.
    let erro = executar_acao(
        &ir,
        &com_nulo.novo_estado,
        "confirmar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("nulo não preenche");
    assert!(erros_de_campos(&erro)[0].contains("`bairro`"));
}

// ------------------------------------------------------------------ ator

#[test]
fn ator_fora_dos_atores_da_etapa_e_recusado() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua F" })),
        AtorAcao::Atendente,
        None,
        &contexto(),
    )
    .expect_err("a etapa só permite o agente");

    let texto = motivo(&erro);
    assert!(
        texto.contains("`atendente`") && texto.contains("`agente`"),
        "a mensagem diz quem tentou e quem pode: {texto}"
    );
}

#[test]
fn acao_restrita_ao_atendente_recusada_para_o_agente() {
    let ir = fluxo_com_dois_atores();
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "assumir",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("`assumir` é do atendente");

    let texto = motivo(&erro);
    assert!(
        texto.contains("`assumir`") && texto.contains("`atendente`"),
        "a mensagem diz a ação e o dono: {texto}"
    );
}

// ------------------------------------------------------------------ critério auto

#[test]
fn etapa_com_dois_atores_fica_parcial_e_diz_quem_falta() {
    let ir = fluxo_com_dois_atores();
    let estado = iniciar(&ir).expect("início");

    let resultado = executar_acao(
        &ir,
        &estado,
        "registrar",
        &campos(json!({ "resumo": "cliente quer troca" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Etapa {
            id: "triagem".to_owned()
        },
        "a etapa não avança enquanto o atendente não agir"
    );
    assert_eq!(resultado.aguardando, vec![AtorAcao::Atendente]);
    assert_eq!(
        resultado.novo_estado.atores_que_agiram,
        vec![AtorAcao::Agente]
    );
    assert_eq!(
        tipos(&resultado),
        vec![
            TipoEventoFicha::CamposPreenchidos,
            TipoEventoFicha::AcaoExecutada,
        ],
        "etapa parcial não tem evento de conclusão nem de entrada"
    );
    assert_eq!(
        resultado.eventos[1].payload["parcial"],
        json!(true),
        "o evento da ação registra que a etapa ficou parcial"
    );
}

#[test]
fn a_etapa_avanca_quando_o_segundo_ator_age() {
    let ir = fluxo_com_dois_atores();
    let mut estado = iniciar(&ir).expect("início");
    estado = executar_acao(
        &ir,
        &estado,
        "registrar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("primeiro ator")
    .novo_estado;

    let resultado = executar_acao(
        &ir,
        &estado,
        "assumir",
        &sem_campos(),
        AtorAcao::Atendente,
        None,
        &contexto(),
    )
    .expect("segundo ator");

    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Etapa {
            id: "depois".to_owned()
        }
    );
    assert!(resultado.aguardando.is_empty());
    assert!(
        resultado.novo_estado.atores_que_agiram.is_empty(),
        "a etapa nova não herda quem agiu na anterior"
    );
    assert_eq!(
        tipos(&resultado),
        vec![
            TipoEventoFicha::AcaoExecutada,
            TipoEventoFicha::EtapaConcluida,
            TipoEventoFicha::EtapaEntrou,
        ],
        "o segundo ator conclui a etapa e entra na próxima"
    );
}

#[test]
fn os_dois_atores_chegam_ao_mesmo_destino_em_qualquer_ordem() {
    // Duas ordens, o mesmo fim: a etapa avança quando o último ator age, e o destino é o da ação
    // dele. Nada depende de quem começou.
    let ir = fluxo_com_dois_atores();

    let mut agente_primeiro = iniciar(&ir).expect("início");
    agente_primeiro = executar_acao(
        &ir,
        &agente_primeiro,
        "registrar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("agente primeiro")
    .novo_estado;
    let fim_agente_primeiro = executar_acao(
        &ir,
        &agente_primeiro,
        "assumir",
        &sem_campos(),
        AtorAcao::Atendente,
        None,
        &contexto(),
    )
    .expect("atendente depois");

    let mut atendente_primeiro = iniciar(&ir).expect("início");
    atendente_primeiro = executar_acao(
        &ir,
        &atendente_primeiro,
        "assumir",
        &sem_campos(),
        AtorAcao::Atendente,
        None,
        &contexto(),
    )
    .expect("atendente primeiro")
    .novo_estado;
    let fim_atendente_primeiro = executar_acao(
        &ir,
        &atendente_primeiro,
        "registrar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("agente depois");

    assert_eq!(
        fim_agente_primeiro.novo_estado.no_atual, fim_atendente_primeiro.novo_estado.no_atual,
        "as duas ordens terminam na mesma etapa"
    );
    assert_eq!(
        fim_agente_primeiro.novo_estado.no_atual,
        Destino::Etapa {
            id: "depois".to_owned()
        }
    );
}

#[test]
fn acao_irrestrita_avanca_sozinha() {
    // Etapa em que a única ação que leva para frente é irrestrita: quem a executa decide a rota
    // sem esperar ninguém (o atendente aparece na etapa, mas a ação dele é de saída).
    let ir = fluxo(vec![
        etapa(
            "triagem",
            &[AtorAcao::Agente, AtorAcao::Atendente],
            Vec::new(),
            vec![
                acao("resolver", None, &[], "outro"),
                acao_para_fim("encaminhar", Some(AtorAcao::Atendente)),
            ],
        ),
        etapa(
            "outro",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let resultado = executar_acao(
        &ir,
        &estado,
        "resolver",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a ação irrestrita é válida");

    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Etapa {
            id: "outro".to_owned()
        }
    );
    assert!(resultado.aguardando.is_empty());
}

#[test]
fn a_acao_irrestrita_ainda_espera_a_acao_restrita_do_outro_ator() {
    // Ação irrestrita não **exige** ninguém, mas também não atropela: enquanto o dono da outra ação
    // (que leva para frente) não agir, a etapa fica parcial — é o `resolve_auto_advance` do POP
    // Atende, e o que impede duas rotas concorrentes de avançarem em ordens diferentes.
    let ir = fluxo(vec![
        etapa(
            "triagem",
            &[AtorAcao::Agente, AtorAcao::Atendente],
            Vec::new(),
            vec![
                acao("encaminhar", Some(AtorAcao::Atendente), &[], "depois"),
                acao("resolver", None, &[], "outro"),
            ],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
        etapa(
            "outro",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ]);
    let mut estado = iniciar(&ir).expect("início");

    let parcial = executar_acao(
        &ir,
        &estado,
        "resolver",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a ação irrestrita é válida");
    assert_eq!(
        parcial.novo_estado.no_atual,
        Destino::Etapa {
            id: "triagem".to_owned()
        },
        "o atendente ainda não agiu"
    );
    assert_eq!(parcial.aguardando, vec![AtorAcao::Atendente]);

    estado = parcial.novo_estado;
    let avancou = executar_acao(
        &ir,
        &estado,
        "encaminhar",
        &sem_campos(),
        AtorAcao::Atendente,
        None,
        &contexto(),
    )
    .expect("o atendente age");
    assert_eq!(
        avancou.novo_estado.no_atual,
        Destino::Etapa {
            id: "depois".to_owned()
        },
        "a rota é a da última ação executada"
    );
}

#[test]
fn acao_que_vai_para_o_fim_e_decisiva_mesmo_com_outro_ator_pendente() {
    // Paridade com `state_engine.py`: encerrar é decisivo. O atendente ainda não agiu, e isso não
    // segura a ação do agente que leva ao fim.
    let ir = fluxo(vec![
        etapa(
            "triagem",
            &[AtorAcao::Agente, AtorAcao::Atendente],
            Vec::new(),
            vec![
                acao("assumir", Some(AtorAcao::Atendente), &[], "depois"),
                acao_para_fim("encerrar", None),
            ],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let resultado = executar_acao(
        &ir,
        &estado,
        "encerrar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a ação é válida");

    assert!(
        matches!(resultado.novo_estado.no_atual, Destino::Fim { .. }),
        "a ação de saída leva ao fim: {:?}",
        resultado.novo_estado.no_atual
    );
    assert!(
        resultado.aguardando.is_empty(),
        "não há ninguém a esperar depois do fim"
    );
    assert_eq!(
        tipos(&resultado),
        vec![
            TipoEventoFicha::AcaoExecutada,
            TipoEventoFicha::EtapaConcluida,
            TipoEventoFicha::ExecucaoConcluida,
        ],
        "sem `etapa_entrou`: o destino é o fim, que conclui a execução"
    );
    assert_eq!(
        resultado.novo_estado.status,
        StatusExecucao::Concluida,
        "o fim conclui a execução na mesma passagem"
    );
    assert!(
        !resultado.encerra_atendimento,
        "este fim não encerra o atendimento"
    );
}

#[test]
fn a_etapa_com_acao_restrita_de_um_ator_so_nao_espera_o_outro() {
    // O atendente aparece na etapa, mas a única ação dele vai para o fim: ele não trava nada.
    let ir = fluxo(vec![
        etapa(
            "coleta",
            &[AtorAcao::Agente, AtorAcao::Atendente],
            Vec::new(),
            vec![
                acao("confirmar", None, &[], "depois"),
                acao_para_fim("desistir", Some(AtorAcao::Atendente)),
            ],
        ),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let resultado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a ação é válida");

    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Etapa {
            id: "depois".to_owned()
        }
    );
    assert!(resultado.aguardando.is_empty());
}

#[test]
fn os_atores_exigidos_vem_das_acoes_e_na_ordem_do_texto() {
    let etapa = etapa(
        "mista",
        &[AtorAcao::Agente, AtorAcao::Atendente],
        Vec::new(),
        vec![
            acao("assumir", Some(AtorAcao::Atendente), &[], "depois"),
            acao_para_fim("desistir", Some(AtorAcao::Atendente)),
            acao("livre", None, &[], "depois"),
            acao("registrar", Some(AtorAcao::Agente), &[], "depois"),
            acao("registrar_de_novo", Some(AtorAcao::Agente), &[], "depois"),
        ],
    );

    assert_eq!(
        atores_exigidos(&etapa),
        vec![AtorAcao::Agente, AtorAcao::Atendente],
        "sem repetição, sem o ator da ação de fim, na ordem alfabética do texto"
    );
}

#[test]
fn campos_faltando_lista_so_os_obrigatorios_na_ordem_do_script() {
    let etapa = etapa(
        "coleta",
        &[AtorAcao::Agente],
        vec![
            campo("endereco", TipoCampo::Texto, true, &[]),
            campo("bairro", TipoCampo::Texto, false, &[]),
            campo("periodo", TipoCampo::Opcao, true, &["manhã", "tarde"]),
        ],
        Vec::new(),
    );
    let mut dados = poph_fluxos::Dados::new();
    dados.insert(
        "coleta".to_owned(),
        json!({ "endereco": "Rua G", "bairro": "Centro" }),
    );

    assert_eq!(campos_faltando(&etapa, &dados), vec!["periodo".to_owned()]);

    dados.insert(
        "coleta".to_owned(),
        json!({ "endereco": "Rua G", "periodo": "tarde" }),
    );
    assert!(campos_faltando(&etapa, &dados).is_empty());
}

// ------------------------------------------------------------------ idempotência

#[test]
fn repetir_a_chave_nao_gera_segundo_evento() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");
    let entrada = campos(json!({ "endereco": "Rua H" }));

    let primeira = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &entrada,
        AtorAcao::Agente,
        Some("chave-1"),
        &contexto(),
    )
    .expect("primeira passagem");
    assert!(!primeira.repetida);
    assert!(!primeira.eventos.is_empty());

    let segunda = executar_acao(
        &ir,
        &primeira.novo_estado,
        "confirmar",
        &entrada,
        AtorAcao::Agente,
        Some("chave-1"),
        &contexto(),
    )
    .expect("a repetição não é erro");

    assert!(segunda.repetida);
    assert!(segunda.eventos.is_empty(), "repetir não grava evento");
    assert!(segunda.efeitos.is_empty());
    assert_eq!(
        segunda.novo_estado, primeira.novo_estado,
        "o estado não muda na repetição"
    );

    // Outra chave, na etapa nova, aplica de verdade.
    let terceira = executar_acao(
        &ir,
        &segunda.novo_estado,
        "ok",
        &sem_campos(),
        AtorAcao::Agente,
        Some("chave-2"),
        &contexto(),
    )
    .expect("chave nova aplica");
    assert!(!terceira.repetida);
    assert!(!terceira.eventos.is_empty());
}

#[test]
fn a_chave_repetida_devolve_quem_faltava_quando_a_etapa_ficou_parcial() {
    let ir = fluxo_com_dois_atores();
    let estado = iniciar(&ir).expect("início");
    let primeira = executar_acao(
        &ir,
        &estado,
        "registrar",
        &sem_campos(),
        AtorAcao::Agente,
        Some("chave-triagem"),
        &contexto(),
    )
    .expect("primeira passagem");
    assert_eq!(primeira.aguardando, vec![AtorAcao::Atendente]);

    let repetida = executar_acao(
        &ir,
        &primeira.novo_estado,
        "registrar",
        &sem_campos(),
        AtorAcao::Agente,
        Some("chave-triagem"),
        &contexto(),
    )
    .expect("repetição");

    assert!(repetida.repetida);
    assert_eq!(
        repetida.aguardando,
        vec![AtorAcao::Atendente],
        "a repetição devolve o mesmo desfecho"
    );
    assert!(repetida.eventos.is_empty());
}

// ------------------------------------------------------------------ sem transição escondida

#[test]
fn acao_inexistente_devolve_as_validas_e_nao_muda_nada() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "concluir",
        &campos(json!({ "endereco": "Rua I" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("`concluir` não existe");

    let texto = motivo(&erro);
    assert!(
        texto.contains("`confirmar`") && texto.contains("`desistir`"),
        "a mensagem lista as ações válidas: {texto}"
    );
    assert_eq!(erro.status_http(), 409);
    // O estado passado não é tocado (a função é pura) e nada foi gravado.
    assert!(estado.dados.is_empty());
    assert_eq!(
        estado.no_atual,
        Destino::Etapa {
            id: "coleta".to_owned()
        }
    );
}

#[test]
fn etapa_sem_acao_avisa_que_nao_tem_saida() {
    let ir = fluxo(vec![
        etapa("coleta", &[AtorAcao::Agente], Vec::new(), Vec::new()),
        etapa(
            "depois",
            &[AtorAcao::Agente],
            Vec::new(),
            vec![acao_para_fim("ok", None)],
        ),
    ]);
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "qualquer",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("a etapa não tem ação");
    assert!(motivo(&erro).contains("nenhuma (a etapa não tem saída)"));
}

// ------------------------------------------------------------------ estado que não aceita ação

#[test]
fn execucao_ja_encerrada_recusa_acao() {
    let ir = fluxo_simples();
    let mut estado = iniciar(&ir).expect("início");
    estado.status = StatusExecucao::Concluida;

    let erro = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua J" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("execução concluída não aceita ação");
    assert!(motivo(&erro).contains("concluida"));
}

#[test]
fn execucao_parada_numa_decisao_resolve_antes_de_executar_a_acao() {
    // Estado gravado por uma versão anterior do motor: a execução ficou parada na decisão. Desde a
    // FX-14 o motor atravessa a decisão na mesma passagem, então a ação é aplicada na etapa em que
    // a decisão desemboca (o caso de teste da FX-13 que exigia recusa virou este).
    let mut ir = fluxo_simples();
    ir.decisoes = vec![poph_fluxos::ir::Decisao {
        id: "escolher_rota".to_owned(),
        // Sem endereço ainda, a decisão manda para a coleta; com ele, para a confirmação.
        se: poph_fluxos::ir::Regra::Vazio {
            campo: "coleta.endereco".to_owned(),
        },
        entao: Destino::Etapa {
            id: "coleta".to_owned(),
        },
        senao: Destino::Etapa {
            id: "confirmacao".to_owned(),
        },
    }];
    let mut estado = iniciar(&ir).expect("início");
    estado.no_atual = Destino::Decisao {
        id: "escolher_rota".to_owned(),
    };

    let resultado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10" })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("a decisão é resolvida e a ação vale na etapa de destino");

    assert_eq!(
        tipos(&resultado),
        vec![
            TipoEventoFicha::DecisaoResolvida,
            TipoEventoFicha::EtapaEntrou,
            TipoEventoFicha::CamposPreenchidos,
            TipoEventoFicha::AcaoExecutada,
            TipoEventoFicha::EtapaConcluida,
            TipoEventoFicha::EtapaEntrou,
        ],
        "a decisão é resolvida antes da ação"
    );
    assert_eq!(
        resultado.novo_estado.no_atual,
        Destino::Etapa {
            id: "confirmacao".to_owned()
        }
    );
}

#[test]
fn acao_para_etapa_inexistente_e_ir_divergente() {
    let ir = fluxo(vec![etapa(
        "coleta",
        &[AtorAcao::Agente],
        Vec::new(),
        vec![acao("ok", None, &[], "etapa_que_nao_existe")],
    )]);
    let estado = iniciar(&ir).expect("início");

    let erro = executar_acao(
        &ir,
        &estado,
        "ok",
        &sem_campos(),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect_err("o destino não existe na estrutura");
    assert!(matches!(erro, DomainError::IrDivergente { .. }), "{erro:?}");
}

// ------------------------------------------------------------------ determinismo e IR real

#[test]
fn a_mesma_entrada_produz_o_mesmo_resultado() {
    let ir = fluxo_com_dois_atores();
    let estado = iniciar(&ir).expect("início");
    let entrada = campos(json!({ "resumo": "troca de produto" }));

    let primeiro = executar_acao(
        &ir,
        &estado,
        "registrar",
        &entrada,
        AtorAcao::Agente,
        Some("chave"),
        &contexto(),
    )
    .expect("primeira");
    let segundo = executar_acao(
        &ir,
        &estado,
        "registrar",
        &entrada,
        AtorAcao::Agente,
        Some("chave"),
        &contexto(),
    )
    .expect("segunda");

    assert_eq!(primeiro, segundo);
}

#[test]
fn a_ir_dourada_de_agendar_visita_roda_do_inicio_ao_fim_da_etapa() {
    // A IR veio do compilador (FX-10) e passou pelo validador (FX-11): o motor tem de conduzi-la.
    let caminho = format!(
        "{}/../../tests/fluxos/ir/agendar_visita.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let texto =
        std::fs::read_to_string(&caminho).unwrap_or_else(|erro| panic!("{caminho}: {erro}"));
    let ir = Ir::de_json(&texto).expect("a IR dourada é JSON válido");

    let estado = iniciar(&ir).expect("início");
    assert_eq!(
        estado.no_atual,
        Destino::Etapa {
            id: "coleta".to_owned()
        }
    );

    let coletado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": "Rua das Flores, 10", "periodo": "manhã" })),
        AtorAcao::Agente,
        Some("acao-1"),
        &contexto(),
    )
    .expect("a ação confirmar é válida");
    assert_eq!(
        coletado.novo_estado.no_atual,
        Destino::Etapa {
            id: "confirmacao".to_owned()
        }
    );

    let confirmado = executar_acao(
        &ir,
        &coletado.novo_estado,
        "ok",
        &sem_campos(),
        AtorAcao::Agente,
        Some("acao-2"),
        &contexto(),
    )
    .expect("a ação ok é válida");
    assert!(
        matches!(confirmado.novo_estado.no_atual, Destino::Fim { .. }),
        "{:?}",
        confirmado.novo_estado.no_atual
    );
}

#[test]
fn o_dado_do_cliente_e_gravado_como_valor_e_nunca_interpretado() {
    let ir = fluxo_simples();
    let estado = iniciar(&ir).expect("início");
    let cliente = "__import__('os').system('ls')";

    let resultado = executar_acao(
        &ir,
        &estado,
        "confirmar",
        &campos(json!({ "endereco": cliente })),
        AtorAcao::Agente,
        None,
        &contexto(),
    )
    .expect("o texto do cliente é só um texto");

    assert_eq!(
        resultado.novo_estado.dados["coleta"]["endereco"],
        json!(cliente),
        "o valor sai como entrou, sem virar código"
    );
}

// ------------------------------------------------------------------ tipos do estado

#[test]
fn o_estado_sobrevive_a_ida_e_volta_em_json() {
    let ir = fluxo_simples();
    let resultado = executar_acao(
        &ir,
        &iniciar(&ir).expect("início"),
        "confirmar",
        &campos(json!({ "endereco": "Rua K" })),
        AtorAcao::Agente,
        Some("chave"),
        &contexto(),
    )
    .expect("ação válida");

    let json = serde_json::to_string(&resultado.novo_estado).expect("serializa");
    let voltou: Estado = serde_json::from_str(&json).expect("lê");
    assert_eq!(voltou, resultado.novo_estado);
}
