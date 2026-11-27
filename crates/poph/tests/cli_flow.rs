use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use poph_store::{NewRecord, Store};
use poph_types::{AgentId, FlowId, OrgId};
use serde_json::Value;

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

const SCRIPT_V2: &str = r#"fluxo(id = "agendar_visita", nome = "Agendar visita", descricao = "Marca uma visita técnica no endereço do cliente.", inicio = "coleta", gatilho = gatilho(quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.", nao_quando = "Só quer saber preço ou prazo, sem pedir visita.", exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"]))

etapa(id = "coleta", nome = "Coletar dados", instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.", campos = [campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True), campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True)], acoes = [acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"), acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu"))])

etapa(id = "confirmacao", nome = "Confirmar visita", instrucao = "Confirme o horário e o endereço com o cliente antes de fechar.", acoes = [acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada"))])
"#;

const BROKEN_SCRIPT: &str = "fluxo(\n    id = \"quebrado\",\n    nome = ,\n)\n";

struct Scenario {
    dir: PathBuf,
}

impl Scenario {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("poph-cli-flow-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dados")).expect("cria a pasta de dados");
        Self { dir }
    }

    fn dados(&self) -> PathBuf {
        self.dir.join("dados")
    }

    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dados())
            .output()
            .expect("executa o poph")
    }

    fn write_file(&self, nome: &str, content: &str) -> PathBuf {
        let path = self.dir.join(nome);
        fs::write(&path, content).expect("grava o arquivo do teste");
        path
    }

    fn create_org(&self, slug: &str) -> String {
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
        assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["org"]["id"]
            .as_str()
            .expect("id da org")
            .to_owned()
    }

    fn create_agent(&self, org: &str, nome: &str) -> String {
        let saida = self.poph(&["--org", org, "agent", "create", "--name", nome, "--json"]);
        assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
        json(&saida)["agent"]["id"]
            .as_str()
            .expect("id do agente")
            .to_owned()
    }

    fn store(&self) -> Store {
        Store::open_at(
            &self.dados().join("harness.db"),
            &self.dados().join("rag.db"),
        )
        .expect("abre o banco do teste")
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn exit_code(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json(saida: &Output) -> Value {
    serde_json::from_str(&texto(&saida.stdout))
        .unwrap_or_else(|error| panic!("saída não é JSON ({error}): {}", texto(&saida.stdout)))
}

fn keys(value: &Value) -> Vec<String> {
    let mut names: Vec<String> = value
        .as_object()
        .expect("objeto JSON")
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

fn org_id(org_text: &str) -> OrgId {
    org_text.parse().expect("id de org")
}

fn publish_ok(scenario: &Scenario, org: &str, nome: &str, script: &str) {
    let file = scenario.write_file(nome, script);
    let saida = scenario.poph(&[
        "--org",
        org,
        "flow",
        "publicar",
        file.to_str().expect("caminho"),
    ]);
    assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
}

#[test]
fn validate_good_script_exits_zero_and_json_has_stable_shape() {
    let scenario = Scenario::new("validar-bom");
    let file = scenario.write_file("bom.fluxo", SCRIPT_V1);
    let path = file.to_str().expect("caminho");

    let first = scenario.poph(&["flow", "validar", path, "--json"]);
    let second = scenario.poph(&["flow", "validar", path, "--json"]);

    assert_eq!(exit_code(&first), 0, "{}", texto(&first.stderr));
    let value = json(&first);
    assert_eq!(value["valid"], true);
    assert_eq!(value["versao_relatorio"], 1);
    assert!(value["ir"].is_object(), "a IR sai no JSON");
    assert_eq!(value["diagnostics"].as_array().expect("lista").len(), 0);
    assert_eq!(
        keys(&value),
        [
            "diagnostics",
            "ir",
            "resolutions",
            "valid",
            "versao_relatorio"
        ]
    );
    assert_eq!(keys(&json(&first)), keys(&json(&second)));
}

#[test]
fn validate_bad_script_exits_one_and_points_at_line_and_column() {
    let scenario = Scenario::new("validar-ruim");
    let file = scenario.write_file("ruim.fluxo", BROKEN_SCRIPT);
    let path = file.to_str().expect("caminho");

    let text_output = scenario.poph(&["flow", "validar", path]);
    assert_eq!(exit_code(&text_output), 1);
    let saida = texto(&text_output.stdout);
    assert!(saida.contains("script inválido"), "{saida}");
    assert!(saida.contains("linha 3, coluna"), "{saida}");

    let as_json = scenario.poph(&["flow", "validar", path, "--json"]);
    assert_eq!(exit_code(&as_json), 1);
    let value = json(&as_json);
    assert_eq!(value["valid"], false);
    let first = &value["diagnostics"][0];
    assert_eq!(first["linha"], 3);
    assert!(first["coluna"].as_u64().is_some(), "coluna presente");
    assert_eq!(first["severidade"], "erro");
}

#[test]
fn publish_lists_and_shows_the_flow() {
    let scenario = Scenario::new("publicar-listar");
    let org = scenario.create_org("clinica-sul");
    publish_ok(&scenario, &org, "v1.fluxo", SCRIPT_V1);

    let listing = scenario.poph(&["--org", &org, "flow", "listar", "--json"]);
    assert_eq!(exit_code(&listing), 0, "{}", texto(&listing.stderr));
    let items = json(&listing)["items"].as_array().expect("itens").clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["slug"], "agendar_visita");
    assert_eq!(items[0]["current_revision"], 1);

    let shown = scenario.poph(&["--org", &org, "flow", "mostrar", "agendar_visita", "--json"]);
    assert_eq!(exit_code(&shown), 0, "{}", texto(&shown.stderr));
    let value = json(&shown);
    assert_eq!(value["revision"]["number"], 1);
    assert_eq!(value["draft_source"], SCRIPT_V1);
}

#[test]
fn publish_new_revision_requires_version_and_leaves_revision_1_untouched() {
    let scenario = Scenario::new("revisao-2");
    let org = scenario.create_org("clinica-norte");
    publish_ok(&scenario, &org, "v1.fluxo", SCRIPT_V1);
    let shown = scenario.poph(&["--org", &org, "flow", "mostrar", "agendar_visita", "--json"]);
    let versao = json(&shown)["version"].as_u64().expect("versão do fluxo");
    let v2_file = scenario.write_file("v2.fluxo", SCRIPT_V2);
    let v2_path = v2_file.to_str().expect("caminho");

    let without_if_match = scenario.poph(&["--org", &org, "flow", "publicar", v2_path]);
    assert_eq!(exit_code(&without_if_match), 1);
    assert!(
        texto(&without_if_match.stderr).contains("--if-match"),
        "{}",
        texto(&without_if_match.stderr)
    );

    let stale = scenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        v2_path,
        "--if-match",
        &(versao + 7).to_string(),
    ]);
    assert_eq!(exit_code(&stale), 1);
    assert!(
        texto(&stale.stderr).contains("não confere"),
        "{}",
        texto(&stale.stderr)
    );

    let right = scenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        v2_path,
        "--if-match",
        &versao.to_string(),
    ]);
    assert_eq!(exit_code(&right), 0, "{}", texto(&right.stderr));
    assert!(
        texto(&right.stdout).contains("revisão 2"),
        "{}",
        texto(&right.stdout)
    );

    let afterwards = scenario.poph(&["--org", &org, "flow", "mostrar", "agendar_visita", "--json"]);
    assert_eq!(json(&afterwards)["draft_source"], SCRIPT_V2);

    let store = scenario.store();
    let fluxo = fluxo_id(&scenario, &org);
    let revision_1 = store
        .revisions()
        .get(org_id(&org), fluxo, 1)
        .expect("revisão 1");
    assert_eq!(revision_1.fonte, SCRIPT_V1);
    let revision_2 = store
        .revisions()
        .get(org_id(&org), fluxo, 2)
        .expect("revisão 2");
    assert_eq!(revision_2.fonte, SCRIPT_V2);
}

#[test]
fn publish_invalid_script_writes_nothing() {
    let scenario = Scenario::new("publicar-invalido");
    let org = scenario.create_org("clinica-oeste");
    let bad = scenario.write_file("ruim.fluxo", BROKEN_SCRIPT);

    let saida = scenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        bad.to_str().expect("caminho"),
    ]);
    assert_eq!(exit_code(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("nada foi gravado"),
        "{}",
        texto(&saida.stderr)
    );

    let listing = scenario.poph(&["--org", &org, "flow", "listar", "--json"]);
    assert_eq!(json(&listing)["items"].as_array().expect("itens").len(), 0);
}

#[test]
fn format_is_idempotent_and_writes_to_the_same_file() {
    let scenario = Scenario::new("formatar");
    let file = scenario.write_file("feio.fluxo", SCRIPT_V2);
    let path = file.to_str().expect("caminho");
    assert_ne!(
        fs::read_to_string(&file).expect("lê o arquivo"),
        SCRIPT_V1,
        "a entrada do teste precisa estar fora da forma canônica"
    );

    let first = scenario.poph(&["flow", "formatar", path]);
    assert_eq!(exit_code(&first), 0, "{}", texto(&first.stderr));
    let canonical_form = texto(&first.stdout);
    assert!(!canonical_form.is_empty());
    assert_ne!(
        fs::read_to_string(&file).expect("lê o arquivo"),
        canonical_form,
        "formatar sem --escrever não pode tocar no arquivo"
    );

    let written = scenario.poph(&["flow", "formatar", path, "--escrever"]);
    assert_eq!(exit_code(&written), 0, "{}", texto(&written.stderr));
    assert_eq!(
        fs::read_to_string(&file).expect("lê o arquivo"),
        canonical_form
    );

    let again = scenario.poph(&["flow", "formatar", path]);
    assert_eq!(texto(&again.stdout), canonical_form);
}

#[test]
fn format_script_that_does_not_compile_exits_one() {
    let scenario = Scenario::new("formatar-ruim");
    let file = scenario.write_file("ruim.fluxo", BROKEN_SCRIPT);
    let before = fs::read_to_string(&file).expect("lê");

    let saida = scenario.poph(&[
        "flow",
        "formatar",
        file.to_str().expect("caminho"),
        "--escrever",
    ]);

    assert_eq!(exit_code(&saida), 1);
    assert_eq!(
        fs::read_to_string(&file).expect("lê"),
        before,
        "não grava script ruim"
    );
}

#[test]
fn diff_shows_what_changed_and_the_cases_in_each_revision() {
    let scenario = Scenario::new("diff");
    let org = scenario.create_org("clinica-leste");
    publish_ok(&scenario, &org, "v1.fluxo", SCRIPT_V1);
    let versao = json(&scenario.poph(&[
        "--org",
        &org,
        "flow",
        "mostrar",
        "agendar_visita",
        "--json",
    ]))["version"]
        .as_u64()
        .expect("versão");
    let v2 = scenario.write_file("v2.fluxo", SCRIPT_V2);
    let published = scenario.poph(&[
        "--org",
        &org,
        "flow",
        "publicar",
        v2.to_str().expect("caminho"),
        "--if-match",
        &versao.to_string(),
    ]);
    assert_eq!(exit_code(&published), 0, "{}", texto(&published.stderr));

    let saida = scenario.poph(&["--org", &org, "flow", "diff", "agendar_visita", "--json"]);
    assert_eq!(exit_code(&saida), 0, "{}", texto(&saida.stderr));
    let value = json(&saida);
    assert_eq!(value["from"]["revision"], 1);
    assert_eq!(value["to"]["revision"], 2);
    assert_eq!(value["from"]["active_runs"], 0);
    assert_eq!(value["to"]["active_runs"], 0);
    let changes = value["changes"].as_array().expect("mudanças");
    assert!(!changes.is_empty(), "a instrução mudou");

    let text_mode = scenario.poph(&["--org", &org, "flow", "diff", "agendar_visita"]);
    assert!(texto(&text_mode.stdout).contains("revisão 1 → revisão 2"));
}

#[test]
fn release_and_withdraw_change_agent_permission() {
    let scenario = Scenario::new("permissao");
    let org = scenario.create_org("clinica-centro");
    publish_ok(&scenario, &org, "v1.fluxo", SCRIPT_V1);
    let agente = scenario.create_agent(&org, "Ana Atendente");

    let liberar = scenario.poph(&[
        "--org",
        &org,
        "flow",
        "liberar",
        &agente,
        "--fluxo",
        "agendar_visita",
    ]);
    assert_eq!(exit_code(&liberar), 0, "{}", texto(&liberar.stderr));
    let released = scenario
        .store()
        .flows()
        .list_from_agent(org_id(&org), agente.parse::<AgentId>().expect("id"))
        .expect("lista");
    assert_eq!(released.len(), 1);

    let retirar = scenario.poph(&[
        "--org",
        &org,
        "flow",
        "retirar",
        &agente,
        "--fluxo",
        "agendar_visita",
    ]);
    assert_eq!(exit_code(&retirar), 0, "{}", texto(&retirar.stderr));
    let afterwards = scenario
        .store()
        .flows()
        .list_from_agent(org_id(&org), agente.parse::<AgentId>().expect("id"))
        .expect("lista");
    assert!(afterwards.is_empty());

    let missing = scenario.poph(&[
        "--org",
        &org,
        "flow",
        "liberar",
        "agente-que-nao-existe",
        "--fluxo",
        "agendar_visita",
    ]);
    assert_eq!(exit_code(&missing), 1);
    assert!(texto(&missing.stderr).contains("não existe agente"));
}

#[test]
fn exit_codes_for_usage_and_error() {
    let scenario = Scenario::new("codigos");
    let org = scenario.create_org("clinica-sul-2");

    let without_org = scenario.poph(&["flow", "listar"]);
    assert_eq!(exit_code(&without_org), 1);
    assert!(texto(&without_org.stderr).contains("--org"));

    let without_file = scenario.poph(&["flow", "validar", "nao-existe.fluxo"]);
    assert_eq!(exit_code(&without_file), 1);

    let bad_status = scenario.poph(&["--org", &org, "flow", "listar", "--status", "ligado"]);
    assert_eq!(exit_code(&bad_status), 1);

    let usage = scenario.poph(&["flow", "apagar-tudo"]);
    assert_eq!(exit_code(&usage), 2);
}

#[test]
fn list_empty_cases_and_show_missing_case() {
    let scenario = Scenario::new("ficha-vazia");
    let org = scenario.create_org("clinica-ficha");

    let listing = scenario.poph(&["--org", &org, "ficha", "listar", "--json"]);
    assert_eq!(exit_code(&listing), 0, "{}", texto(&listing.stderr));
    let value = json(&listing);
    assert_eq!(value["items"].as_array().expect("itens").len(), 0);
    assert_eq!(value["versao_relatorio"], 1);

    let missing = scenario.poph(&[
        "--org",
        &org,
        "ficha",
        "mostrar",
        &uuid::Uuid::now_v7().to_string(),
    ]);
    assert_eq!(exit_code(&missing), 1);
    assert!(texto(&missing.stderr).contains("não existe Ficha"));
}

#[test]
fn case_with_filter_and_detail_appear_in_the_cli() {
    let scenario = Scenario::new("ficha-dados");
    let org = scenario.create_org("clinica-ficha-2");
    let agente = scenario.create_agent(&org, "Bia Atendente");
    let ficha = scenario
        .store()
        .cases()
        .create(
            org_id(&org),
            NewRecord {
                conversation_id: None,
                agent_id: agente.parse::<AgentId>().expect("id do agente"),
                canal: "whatsapp".to_owned(),
                peer_externo: Some("cliente-ficticio-cli".to_owned()),
            },
        )
        .expect("cria a Ficha")
        .id;

    let open_cases = scenario.poph(&[
        "--org", &org, "ficha", "listar", "--status", "open", "--json",
    ]);
    assert_eq!(
        json(&open_cases)["items"].as_array().expect("itens").len(),
        1
    );

    let closed_cases = scenario.poph(&[
        "--org", &org, "ficha", "listar", "--status", "closed", "--json",
    ]);
    assert_eq!(
        json(&closed_cases)["items"]
            .as_array()
            .expect("itens")
            .len(),
        0
    );

    let by_agent = scenario.poph(&[
        "--org", &org, "ficha", "listar", "--agente", &agente, "--json",
    ]);
    assert_eq!(exit_code(&by_agent), 0, "{}", texto(&by_agent.stderr));
    assert_eq!(json(&by_agent)["items"].as_array().expect("itens").len(), 1);

    let detail = scenario.poph(&[
        "--org",
        &org,
        "ficha",
        "mostrar",
        &ficha.to_string(),
        "--json",
    ]);
    assert_eq!(exit_code(&detail), 0, "{}", texto(&detail.stderr));
    let value = json(&detail);
    assert_eq!(value["id"], ficha.to_string());
    assert_eq!(value["status"], "open");
    assert!(value["execucoes"].is_array());
    assert!(value["permissoes"]["fluxos_liberados_ao_agente"].is_array());
}

fn fluxo_id(scenario: &Scenario, org: &str) -> FlowId {
    let fluxo = scenario
        .store()
        .flows()
        .by_slug(org_id(org), "agendar_visita")
        .expect("consulta o fluxo")
        .expect("o fluxo existe");
    fluxo.id
}
