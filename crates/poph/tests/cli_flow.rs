//! Testes de ponta a ponta de `poph flow` e `poph ficha` (FX-31).
//!
//! Processo de verdade e banco de verdade numa pasta temporária. Os scripts e as regras de publicação
//! são testados na API (`crates/poph-server/tests/fluxos_api.rs`); aqui se confere a linha de comando:
//! códigos de saída, forma do `--json`, mensagens de erro e o efeito no banco, lido pelo próprio
//! repositório. Os dados são fictícios.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use poph_store::{NovaFicha, Store};
use poph_types::{AgentId, FlowId, OrgId};
use serde_json::Value;

/// Script válido: o mesmo da API, com a etapa `confirmacao` como ponto de partida da revisão 2.
const SCRIPT_V1: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
        nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
        exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    campos = [
        campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True),
        campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"),
        acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu")),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmar visita",
    instrucao = "Repita o endereço e o período combinados e peça a confirmação do cliente.",
    acoes = [
        acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

/// A mesma coisa com outra instrução na etapa `confirmacao`: a revisão 2 muda uma frase.
const SCRIPT_V2: &str = r#"fluxo(id = "agendar_visita", nome = "Agendar visita", descricao = "Marca uma visita técnica no endereço do cliente.", inicio = "coleta", gatilho = gatilho(quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.", nao_quando = "Só quer saber preço ou prazo, sem pedir visita.", exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"]))

etapa(id = "coleta", nome = "Coletar dados", instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.", campos = [campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True), campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True)], acoes = [acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"), acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu"))])

etapa(id = "confirmacao", nome = "Confirmar visita", instrucao = "Confirme o horário e o endereço com o cliente antes de fechar.", acoes = [acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada"))])
"#;

/// Script com erro de sintaxe na linha 3 (o valor do `nome` não existe).
const SCRIPT_COM_ERRO: &str = "fluxo(\n    id = \"quebrado\",\n    nome = ,\n)\n";

/// Pasta temporária com a pasta de dados. O banco nasce na primeira execução da CLI.
struct Cenario {
    pasta: PathBuf,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-flow-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("cria a pasta de dados");
        Self { pasta }
    }

    fn dados(&self) -> PathBuf {
        self.pasta.join("dados")
    }

    /// Roda `poph` com ambiente limpo, pasta atual isolada e a pasta de dados do teste.
    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dados())
            .output()
            .expect("executa o poph")
    }

    /// Grava um arquivo na pasta do teste e devolve o caminho.
    fn arquivo(&self, nome: &str, conteudo: &str) -> PathBuf {
        let caminho = self.pasta.join(nome);
        fs::write(&caminho, conteudo).expect("grava o arquivo do teste");
        caminho
    }

    /// Cria uma org pela CLI e devolve o id.
    fn criar_org(&self, slug: &str) -> String {
        let saida = self.poph(&[
            "--offline",
            "org",
            "create",
            "--name",
            &format!("Org {slug}"),
            "--slug",
            slug,
            "--json",
        ]);
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    /// Cria um agente pela CLI e devolve o id.
    fn criar_agente(&self, org: &str, nome: &str) -> String {
        let saida = self.poph(&["--org", org, "agent", "create", "--name", nome, "--json"]);
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["agent"]["id"]
            .as_str()
            .expect("id do agente")
            .to_owned()
    }

    /// Abre o banco do teste direto pelo repositório (a CLI já o migrou).
    fn store(&self) -> Store {
        Store::abrir_em(
            &self.dados().join("harness.db"),
            &self.dados().join("rag.db"),
        )
        .expect("abre o banco do teste")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn codigo(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(saida: &Output) -> Value {
    serde_json::from_str(&texto(&saida.stdout))
        .unwrap_or_else(|erro| panic!("saída não é JSON ({erro}): {}", texto(&saida.stdout)))
}

/// Chaves de primeiro nível de um objeto JSON, em ordem. Serve para provar que a forma não muda.
fn chaves(valor: &Value) -> Vec<String> {
    let mut nomes: Vec<String> = valor
        .as_object()
        .expect("objeto JSON")
        .keys()
        .cloned()
        .collect();
    nomes.sort();
    nomes
}

fn org_id(texto_org: &str) -> OrgId {
    texto_org.parse().expect("id de org")
}

/// Publica um script num arquivo novo e confere que deu certo (criação, revisão 1).
fn publicar_ok(cenario: &Cenario, org: &str, nome: &str, script: &str) {
    let arquivo = cenario.arquivo(nome, script);
    let saida = cenario.poph(&[
        "--org",
        org,
        "flow",
        "publicar",
        arquivo.to_str().expect("caminho"),
    ]);
    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
}

#[test]
fn validar_script_bom_sai_com_0_e_o_json_tem_forma_estavel() {
    let cenario = Cenario::novo("validar-bom");
    let arquivo = cenario.arquivo("bom.fluxo", SCRIPT_V1);
    let caminho = arquivo.to_str().expect("caminho");

    let primeira = cenario.poph(&["flow", "validar", caminho, "--json"]);
    let segunda = cenario.poph(&["flow", "validar", caminho, "--json"]);

    assert_eq!(codigo(&primeira), 0, "{}", texto(&primeira.stderr));
    let valor = json(&primeira);
    assert_eq!(valor["valid"], true);
    assert_eq!(valor["versao_relatorio"], 1);
    assert!(valor["ir"].is_object(), "a IR sai no JSON");
    assert_eq!(valor["diagnostics"].as_array().expect("lista").len(), 0);
    assert_eq!(
        chaves(&valor),
        [
            "diagnostics",
            "ir",
            "resolutions",
            "valid",
            "versao_relatorio"
        ]
    );
    assert_eq!(chaves(&json(&primeira)), chaves(&json(&segunda)));
}

#[test]
fn validar_script_ruim_sai_com_1_e_aponta_linha_e_coluna() {
    let cenario = Cenario::novo("validar-ruim");
    let arquivo = cenario.arquivo("ruim.fluxo", SCRIPT_COM_ERRO);
    let caminho = arquivo.to_str().expect("caminho");

    let texto_saida = cenario.poph(&["flow", "validar", caminho]);
    assert_eq!(codigo(&texto_saida), 1);
    let saida = texto(&texto_saida.stdout);
    assert!(saida.contains("script inválido"), "{saida}");
    assert!(saida.contains("linha 3, coluna"), "{saida}");

    let em_json = cenario.poph(&["flow", "validar", caminho, "--json"]);
    assert_eq!(codigo(&em_json), 1);
    let valor = json(&em_json);
    assert_eq!(valor["valid"], false);
    let primeiro = &valor["diagnostics"][0];
    assert_eq!(primeiro["linha"], 3);
    assert!(primeiro["coluna"].as_u64().is_some(), "coluna presente");
    assert_eq!(primeiro["severidade"], "erro");
}

#[test]
fn publicar_lista_e_mostra_o_fluxo() {
    let cenario = Cenario::novo("publicar-listar");
    let org = cenario.criar_org("clinica-sul");
    publicar_ok(&cenario, &org, "v1.fluxo", SCRIPT_V1);

    let lista = cenario.poph(&["--org", &org, "flow", "listar", "--json"]);
    assert_eq!(codigo(&lista), 0, "{}", texto(&lista.stderr));
    let itens = json(&lista)["items"].as_array().expect("itens").clone();
    assert_eq!(itens.len(), 1);
    assert_eq!(itens[0]["slug"], "agendar_visita");
    assert_eq!(itens[0]["current_revision"], 1);

    let mostra = cenario.poph(&["--org", &org, "flow", "mostrar", "agendar_visita", "--json"]);
    assert_eq!(codigo(&mostra), 0, "{}", texto(&mostra.stderr));
    let valor = json(&mostra);
    assert_eq!(valor["revision"]["number"], 1);
    assert_eq!(valor["draft_source"], SCRIPT_V1);
}

#[test]
fn publicar_revisao_nova_exige_versao_e_deixa_a_revisao_1_intacta() {
    let cenario = Cenario::novo("revisao-2");
    let org = cenario.criar_org("clinica-norte");
    publicar_ok(&cenario, &org, "v1.fluxo", SCRIPT_V1);
    let mostra = cenario.poph(&["--org", &org, "flow", "mostrar", "agendar_visita", "--json"]);
    let versao = json(&mostra)["version"].as_u64().expect("versão do fluxo");
    let arquivo_v2 = cenario.arquivo("v2.fluxo", SCRIPT_V2);
    let caminho_v2 = arquivo_v2.to_str().expect("caminho");

    // Sem --if-match, a publicação de um fluxo existente não acontece.
    let sem = cenario.poph(&["--org", &org, "flow", "publicar", caminho_v2]);
    assert_eq!(codigo(&sem), 1);
    assert!(
        texto(&sem.stderr).contains("--if-match"),
        "{}",
        texto(&sem.stderr)
    );

    // Versão velha: conflito, nada gravado.
    let velha = cenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        caminho_v2,
        "--if-match",
        &(versao + 7).to_string(),
    ]);
    assert_eq!(codigo(&velha), 1);
    assert!(
        texto(&velha.stderr).contains("não confere"),
        "{}",
        texto(&velha.stderr)
    );

    // Versão certa: revisão 2.
    let certa = cenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        caminho_v2,
        "--if-match",
        &versao.to_string(),
    ]);
    assert_eq!(codigo(&certa), 0, "{}", texto(&certa.stderr));
    assert!(
        texto(&certa.stdout).contains("revisão 2"),
        "{}",
        texto(&certa.stdout)
    );

    // O rascunho passa a ser o texto publicado: `mostrar` não pode trazer um texto antigo.
    let depois = cenario.poph(&["--org", &org, "flow", "mostrar", "agendar_visita", "--json"]);
    assert_eq!(json(&depois)["draft_source"], SCRIPT_V2);

    // A revisão 1 continua com o texto byte a byte que foi publicado.
    let store = cenario.store();
    let fluxo = fluxo_id(&cenario, &org);
    let revisao_1 = store
        .revisoes()
        .obter(org_id(&org), fluxo, 1)
        .expect("revisão 1");
    assert_eq!(revisao_1.fonte, SCRIPT_V1);
    let revisao_2 = store
        .revisoes()
        .obter(org_id(&org), fluxo, 2)
        .expect("revisão 2");
    assert_eq!(revisao_2.fonte, SCRIPT_V2);
}

#[test]
fn publicar_script_invalido_nao_grava_nada() {
    let cenario = Cenario::novo("publicar-invalido");
    let org = cenario.criar_org("clinica-oeste");
    let ruim = cenario.arquivo("ruim.fluxo", SCRIPT_COM_ERRO);

    let saida = cenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        ruim.to_str().expect("caminho"),
    ]);
    assert_eq!(codigo(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("nada foi gravado"),
        "{}",
        texto(&saida.stderr)
    );

    let lista = cenario.poph(&["--org", &org, "flow", "listar", "--json"]);
    assert_eq!(json(&lista)["items"].as_array().expect("itens").len(), 0);
}

#[test]
fn formatar_e_idempotente_e_escreve_no_proprio_arquivo() {
    let cenario = Cenario::novo("formatar");
    // O script entra na forma compacta, que não é a canônica: sem a gravação, o arquivo fica como
    // está e o teste cai.
    let arquivo = cenario.arquivo("feio.fluxo", SCRIPT_V2);
    let caminho = arquivo.to_str().expect("caminho");
    assert_ne!(
        fs::read_to_string(&arquivo).expect("lê o arquivo"),
        SCRIPT_V1,
        "a entrada do teste precisa estar fora da forma canônica"
    );

    let primeira = cenario.poph(&["flow", "formatar", caminho]);
    assert_eq!(codigo(&primeira), 0, "{}", texto(&primeira.stderr));
    let forma_canonica = texto(&primeira.stdout);
    assert!(!forma_canonica.is_empty());
    assert_ne!(
        fs::read_to_string(&arquivo).expect("lê o arquivo"),
        forma_canonica,
        "formatar sem --escrever não pode tocar no arquivo"
    );

    // Grava no arquivo: o conteúdo gravado é exatamente a forma canônica.
    let gravada = cenario.poph(&["flow", "formatar", caminho, "--escrever"]);
    assert_eq!(codigo(&gravada), 0, "{}", texto(&gravada.stderr));
    assert_eq!(
        fs::read_to_string(&arquivo).expect("lê o arquivo"),
        forma_canonica
    );

    // Formatar a forma canônica de novo não muda nada.
    let de_novo = cenario.poph(&["flow", "formatar", caminho]);
    assert_eq!(texto(&de_novo.stdout), forma_canonica);
}

#[test]
fn formatar_script_que_nao_compila_sai_com_1() {
    let cenario = Cenario::novo("formatar-ruim");
    let arquivo = cenario.arquivo("ruim.fluxo", SCRIPT_COM_ERRO);
    let antes = fs::read_to_string(&arquivo).expect("lê");

    let saida = cenario.poph(&[
        "flow",
        "formatar",
        arquivo.to_str().expect("caminho"),
        "--escrever",
    ]);

    assert_eq!(codigo(&saida), 1);
    assert_eq!(
        fs::read_to_string(&arquivo).expect("lê"),
        antes,
        "não grava script ruim"
    );
}

#[test]
fn diff_mostra_o_que_mudou_e_os_atendimentos_em_cada_revisao() {
    let cenario = Cenario::novo("diff");
    let org = cenario.criar_org("clinica-leste");
    publicar_ok(&cenario, &org, "v1.fluxo", SCRIPT_V1);
    let versao = json(&cenario.poph(&[
        "--org",
        &org,
        "flow",
        "mostrar",
        "agendar_visita",
        "--json",
    ]))["version"]
        .as_u64()
        .expect("versão");
    let v2 = cenario.arquivo("v2.fluxo", SCRIPT_V2);
    let publicada = cenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        v2.to_str().expect("caminho"),
        "--if-match",
        &versao.to_string(),
    ]);
    assert_eq!(codigo(&publicada), 0, "{}", texto(&publicada.stderr));

    let saida = cenario.poph(&["--org", &org, "flow", "diff", "agendar_visita", "--json"]);
    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    let valor = json(&saida);
    assert_eq!(valor["from"]["revision"], 1);
    assert_eq!(valor["to"]["revision"], 2);
    assert_eq!(valor["from"]["active_runs"], 0);
    assert_eq!(valor["to"]["active_runs"], 0);
    let mudancas = valor["changes"].as_array().expect("mudanças");
    assert!(!mudancas.is_empty(), "a instrução mudou");

    let de_texto = cenario.poph(&["--org", &org, "flow", "diff", "agendar_visita"]);
    assert!(texto(&de_texto.stdout).contains("revisão 1 → revisão 2"));
}

#[test]
fn liberar_e_retirar_mudam_a_permissao_do_agente() {
    let cenario = Cenario::novo("permissao");
    let org = cenario.criar_org("clinica-centro");
    publicar_ok(&cenario, &org, "v1.fluxo", SCRIPT_V1);
    let agente = cenario.criar_agente(&org, "Ana Atendente");

    let liberar = cenario.poph(&[
        "--org",
        &org,
        "flow",
        "liberar",
        &agente,
        "--fluxo",
        "agendar_visita",
    ]);
    assert_eq!(codigo(&liberar), 0, "{}", texto(&liberar.stderr));
    let liberados = cenario
        .store()
        .fluxos()
        .listar_do_agente(org_id(&org), agente.parse::<AgentId>().expect("id"))
        .expect("lista");
    assert_eq!(liberados.len(), 1);

    let retirar = cenario.poph(&[
        "--org",
        &org,
        "flow",
        "retirar",
        &agente,
        "--fluxo",
        "agendar_visita",
    ]);
    assert_eq!(codigo(&retirar), 0, "{}", texto(&retirar.stderr));
    let depois = cenario
        .store()
        .fluxos()
        .listar_do_agente(org_id(&org), agente.parse::<AgentId>().expect("id"))
        .expect("lista");
    assert!(depois.is_empty());

    let inexistente = cenario.poph(&[
        "--org",
        &org,
        "flow",
        "liberar",
        "agente-que-nao-existe",
        "--fluxo",
        "agendar_visita",
    ]);
    assert_eq!(codigo(&inexistente), 1);
    assert!(texto(&inexistente.stderr).contains("não existe agente"));
}

#[test]
fn codigos_de_saida_de_uso_e_de_erro() {
    let cenario = Cenario::novo("codigos");
    let org = cenario.criar_org("clinica-sul-2");

    // Sem --org num comando que lê dados da org: erro de negócio, código 1.
    let sem_org = cenario.poph(&["flow", "listar"]);
    assert_eq!(codigo(&sem_org), 1);
    assert!(texto(&sem_org.stderr).contains("--org"));

    // Arquivo que não existe.
    let sem_arquivo = cenario.poph(&["flow", "validar", "nao-existe.fluxo"]);
    assert_eq!(codigo(&sem_arquivo), 1);

    // Situação fora do vocabulário.
    let status_ruim = cenario.poph(&["--org", &org, "flow", "listar", "--status", "ligado"]);
    assert_eq!(codigo(&status_ruim), 1);

    // Subcomando inexistente é uso incorreto: código 2 (o clap decide).
    let uso = cenario.poph(&["flow", "apagar-tudo"]);
    assert_eq!(codigo(&uso), 2);
}

#[test]
fn listar_fichas_vazio_e_mostrar_ficha_inexistente() {
    let cenario = Cenario::novo("ficha-vazia");
    let org = cenario.criar_org("clinica-ficha");

    let lista = cenario.poph(&["--org", &org, "ficha", "listar", "--json"]);
    assert_eq!(codigo(&lista), 0, "{}", texto(&lista.stderr));
    let valor = json(&lista);
    assert_eq!(valor["items"].as_array().expect("itens").len(), 0);
    assert_eq!(valor["versao_relatorio"], 1);

    let inexistente = cenario.poph(&[
        "--org",
        &org,
        "ficha",
        "mostrar",
        &uuid::Uuid::now_v7().to_string(),
    ]);
    assert_eq!(codigo(&inexistente), 1);
    assert!(texto(&inexistente.stderr).contains("não existe Ficha"));
}

#[test]
fn ficha_com_filtro_e_detalhe_aparecem_na_cli() {
    let cenario = Cenario::novo("ficha-dados");
    let org = cenario.criar_org("clinica-ficha-2");
    let agente = cenario.criar_agente(&org, "Bia Atendente");
    let ficha = cenario
        .store()
        .fichas()
        .criar(
            org_id(&org),
            NovaFicha {
                conversation_id: None,
                agent_id: agente.parse::<AgentId>().expect("id do agente"),
                canal: "whatsapp".to_owned(),
                peer_externo: Some("cliente-ficticio-cli".to_owned()),
            },
        )
        .expect("cria a Ficha")
        .id;

    let abertas = cenario.poph(&[
        "--org", &org, "ficha", "listar", "--status", "open", "--json",
    ]);
    assert_eq!(json(&abertas)["items"].as_array().expect("itens").len(), 1);

    let fechadas = cenario.poph(&[
        "--org", &org, "ficha", "listar", "--status", "closed", "--json",
    ]);
    assert_eq!(json(&fechadas)["items"].as_array().expect("itens").len(), 0);

    let por_agente = cenario.poph(&[
        "--org", &org, "ficha", "listar", "--agente", &agente, "--json",
    ]);
    assert_eq!(codigo(&por_agente), 0, "{}", texto(&por_agente.stderr));
    assert_eq!(
        json(&por_agente)["items"].as_array().expect("itens").len(),
        1
    );

    let detalhe = cenario.poph(&[
        "--org",
        &org,
        "ficha",
        "mostrar",
        &ficha.to_string(),
        "--json",
    ]);
    assert_eq!(codigo(&detalhe), 0, "{}", texto(&detalhe.stderr));
    let valor = json(&detalhe);
    assert_eq!(valor["id"], ficha.to_string());
    assert_eq!(valor["status"], "open");
    assert!(valor["execucoes"].is_array());
    assert!(valor["permissoes"]["fluxos_liberados_ao_agente"].is_array());
}

/// Id do fluxo `agendar_visita` da org, lido pelo repositório.
fn fluxo_id(cenario: &Cenario, org: &str) -> FlowId {
    let fluxo = cenario
        .store()
        .fluxos()
        .por_slug(org_id(org), "agendar_visita")
        .expect("consulta o fluxo")
        .expect("o fluxo existe");
    fluxo.id
}
