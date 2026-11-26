//! Testes de ponta a ponta de `poph org`, `token`, `category` e `department` (HR-08).
//!
//! Processo de verdade, banco de verdade numa pasta temporária, `--offline` em todos os comandos.
//! A org de teste é criada pela própria CLI, exceto quando o teste precisa de agentes (o `agent`
//! do teste é criado pelo armazenamento, como em `cli_agente.rs`).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordVerifier};
use poph_store::{NewAgent, Store};
use poph_types::{AdminTokenId, OrgId};
use serde_json::Value;

/// UUID v4 fixo: não é v7, então a CLI deve recusá-lo com o motivo.
const UUID_V4: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";

/// Pasta temporária com a pasta de dados. O banco nasce na primeira execução da CLI.
struct Cenario {
    pasta: PathBuf,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-admin-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("cria a pasta de dados");
        Self { pasta }
    }

    fn dados(&self) -> PathBuf {
        self.pasta.join("dados")
    }

    fn harness(&self) -> PathBuf {
        self.dados().join("harness.db")
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

    /// Abre o banco direto (para semear o que a CLI ainda não cria, como agentes).
    fn store(&self) -> Store {
        Store::open_at(&self.harness(), &self.dados().join("rag.db")).expect("abre o banco")
    }

    /// Cria a org pela CLI e devolve o id (UUID v7) lido do JSON.
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

/// Lê um campo de um JSON de saída, que tem de existir.
fn campo(valor: &Value, caminho: &str) -> String {
    let mut atual = valor;
    for parte in caminho.split('.') {
        atual = &atual[parte];
    }
    atual
        .as_str()
        .unwrap_or_else(|| panic!("campo {caminho} ausente ou não é texto"))
        .to_owned()
}

#[test]
fn ponta_a_ponta_cria_e_lista_os_quatro() {
    let cenario = Cenario::novo("ponta-a-ponta");

    let org = cenario.criar_org("loja-teste");
    assert!(org.len() == 36, "id da org: {org}");

    let token = cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "create",
        "--label",
        "site",
        "--json",
    ]);
    assert_eq!(codigo(&token), 0, "{}", texto(&token.stderr));
    assert!(campo(&json(&token), "valor_do_token").starts_with("poph_"));

    let categoria = cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "category",
        "create",
        "--name",
        "Planos",
        "--description",
        "Planos de assinatura",
        "--json",
    ]);
    assert_eq!(codigo(&categoria), 0, "{}", texto(&categoria.stderr));
    let id_categoria = campo(&json(&categoria), "category.id");

    let departamento = cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "department",
        "create",
        "--name",
        "Vendas",
        "--json",
    ]);
    assert_eq!(codigo(&departamento), 0, "{}", texto(&departamento.stderr));
    let id_departamento = campo(&json(&departamento), "department.id");

    // Lista os quatro e mostra cada um.
    let orgs = json(&cenario.poph(&["--offline", "org", "list", "--json"]));
    assert_eq!(orgs["total"], 1);
    let tokens = json(&cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "token",
        "list",
        "--json",
    ]));
    assert_eq!(tokens["total"], 1);
    let categorias = json(&cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "category",
        "list",
        "--json",
    ]));
    assert_eq!(categorias["total"], 1);
    let departamentos = json(&cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "department",
        "list",
        "--json",
    ]));
    assert_eq!(departamentos["total"], 1);

    let mostra_org = cenario.poph(&["--offline", "org", "show", "loja-teste"]);
    assert_eq!(codigo(&mostra_org), 0);
    assert!(texto(&mostra_org.stdout).contains("Org loja-teste"));

    let mostra_categoria = cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "category",
        "show",
        &id_categoria,
    ]);
    assert_eq!(codigo(&mostra_categoria), 0);
    assert!(texto(&mostra_categoria.stdout).contains("Planos"));

    let mostra_departamento = cenario.poph(&[
        "--offline",
        "--org",
        "loja-teste",
        "department",
        "show",
        &id_departamento,
        "--json",
    ]);
    assert_eq!(codigo(&mostra_departamento), 0);
    assert_eq!(
        campo(&json(&mostra_departamento), "department.name"),
        "Vendas"
    );
}

#[test]
fn token_de_org_aparece_uma_unica_vez_e_fica_so_como_hash() {
    let cenario = Cenario::novo("token-unico");
    let org = cenario.criar_org("loja-token");

    let criado = cenario.poph(&[
        "--offline",
        "--org",
        "loja-token",
        "token",
        "create",
        "--label",
        "integração",
        "--json",
    ]);
    assert_eq!(codigo(&criado), 0, "{}", texto(&criado.stderr));
    let valor = campo(&json(&criado), "valor_do_token");
    let id_token = campo(&json(&criado), "token.id");
    assert!(valor.len() > 60, "valor curto demais");

    // Nenhum comando depois da criação mostra o valor, nem em texto, nem em JSON.
    let mut saidas = Vec::new();
    for argumentos in [
        vec!["--offline", "--org", "loja-token", "token", "list"],
        vec![
            "--offline",
            "--org",
            "loja-token",
            "token",
            "list",
            "--json",
        ],
        vec![
            "--offline",
            "--org",
            "loja-token",
            "token",
            "show",
            id_token.as_str(),
        ],
        vec![
            "--offline",
            "--org",
            "loja-token",
            "token",
            "show",
            id_token.as_str(),
            "--json",
        ],
        vec!["--offline", "org", "show", org.as_str(), "--json"],
        vec!["--offline", "org", "list", "--json"],
    ] {
        let saida = cenario.poph(&argumentos);
        assert_eq!(
            codigo(&saida),
            0,
            "{argumentos:?}: {}",
            texto(&saida.stderr)
        );
        saidas.push(texto(&saida.stdout));
        saidas.push(texto(&saida.stderr));
    }
    for saida in &saidas {
        assert!(!saida.contains(&valor), "o valor do token vazou: {saida}");
    }

    // Depois da criação, o `show` diz que o valor não está disponível (em vez de mostrá-lo).
    let mostra = cenario.poph(&[
        "--offline",
        "--org",
        "loja-token",
        "token",
        "show",
        id_token.as_str(),
    ]);
    assert!(
        texto(&mostra.stdout).contains("valor: não disponível"),
        "mensagem: {}",
        texto(&mostra.stdout)
    );

    // O banco também não guarda o valor em claro (nem no arquivo de escrita antecipada).
    for arquivo in [cenario.harness(), cenario.dados().join("harness.db-wal")] {
        if let Ok(bytes) = fs::read(&arquivo) {
            assert!(
                !bytes
                    .windows(valor.len())
                    .any(|trecho| trecho == valor.as_bytes()),
                "o valor em claro está no arquivo {}",
                arquivo.display()
            );
        }
    }

    // O que fica guardado é um hash argon2 do **segredo** (HR-20): o valor carrega o id do token
    // (`poph_<uuid>_<segredo>`) e a API localiza o registro pelo uuid antes de conferir o segredo.
    let store = cenario.store();
    let token = store
        .tokens()
        .obter(
            org.parse::<OrgId>().expect("id da org"),
            id_token.parse::<AdminTokenId>().expect("id do token"),
        )
        .expect("token gravado");
    assert!(token.hash.starts_with("$argon2id$"), "hash: {}", token.hash);
    let resto = valor.strip_prefix("poph_").expect("prefixo poph_");
    let (id_no_valor, segredo) = resto.split_once('_').expect("uuid e segredo no valor");
    assert_eq!(
        id_no_valor, id_token,
        "o valor tem de carregar o id do token"
    );
    let lido = PasswordHash::new(&token.hash).expect("hash no formato PHC");
    assert!(
        Argon2::default()
            .verify_password(segredo.as_bytes(), &lido)
            .is_ok(),
        "o hash não confere com o segredo"
    );
    assert!(
        Argon2::default()
            .verify_password(valor.as_bytes(), &lido)
            .is_err(),
        "o hash não pode conferir com o valor inteiro (o valor tem de ir por id)"
    );
}

#[test]
fn token_revogado_deixa_de_estar_ativo_e_repetir_revogacao_nao_erra() {
    let cenario = Cenario::novo("token-revoga");
    cenario.criar_org("loja-revoga");
    let criado = json(&cenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "create",
        "--label",
        "antigo",
        "--json",
    ]));
    let id_token = campo(&criado, "token.id");

    let primeira = cenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "delete",
        &id_token,
        "--json",
    ]);
    assert_eq!(codigo(&primeira), 0, "{}", texto(&primeira.stderr));
    assert_eq!(json(&primeira)["ja_estava_revogado"], false);

    let mostra = json(&cenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "show",
        &id_token,
        "--json",
    ]));
    assert_eq!(campo(&mostra, "token.status"), "revoked");

    let segunda = cenario.poph(&[
        "--offline",
        "--org",
        "loja-revoga",
        "token",
        "delete",
        &id_token,
        "--json",
    ]);
    assert_eq!(codigo(&segunda), 0, "revogar de novo não é erro");
    assert_eq!(json(&segunda)["ja_estava_revogado"], true);
}

#[test]
fn org_com_agentes_so_sai_com_force() {
    let cenario = Cenario::novo("org-agentes");
    let org = cenario.criar_org("org-com-agente");
    {
        let store = cenario.store();
        let id = org.parse::<OrgId>().expect("id da org");
        store
            .agents()
            .criar(
                id,
                NewAgent {
                    name: "Bia".to_owned(),
                    slug: "bia".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
    }

    let recusada = cenario.poph(&["--offline", "org", "delete", "org-com-agente"]);
    assert_eq!(codigo(&recusada), 1, "sem --force a exclusão é recusada");
    let aviso = texto(&recusada.stderr);
    assert!(aviso.contains("--force"), "mensagem: {aviso}");
    assert!(aviso.contains("1 agente(s)"), "mensagem: {aviso}");

    // A recusa não pode ter apagado nada.
    let ainda_existe = cenario.poph(&["--offline", "org", "show", "org-com-agente"]);
    assert_eq!(codigo(&ainda_existe), 0);
    assert!(texto(&ainda_existe.stdout).contains("agentes: 1"));

    let confirmada = cenario.poph(&[
        "--offline",
        "org",
        "delete",
        "org-com-agente",
        "--force",
        "--json",
    ]);
    assert_eq!(codigo(&confirmada), 0, "{}", texto(&confirmada.stderr));
    assert_eq!(json(&confirmada)["agentes_removidos"], 1);

    let sumiu = cenario.poph(&["--offline", "org", "show", "org-com-agente"]);
    assert_eq!(codigo(&sumiu), 1);
    assert!(texto(&sumiu.stderr).contains("não existe org com o slug `org-com-agente`"));
}

#[test]
fn org_vazia_sai_sem_force() {
    let cenario = Cenario::novo("org-vazia");
    cenario.criar_org("org-vazia");
    let saida = cenario.poph(&["--offline", "org", "delete", "org-vazia"]);
    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
}

#[test]
fn org_com_categoria_tambem_pede_force() {
    // Além de agentes, a exclusão levaria junto categorias, departamentos e tokens.
    let cenario = Cenario::novo("org-conteudo");
    cenario.criar_org("org-conteudo");
    let criada = cenario.poph(&[
        "--offline",
        "--org",
        "org-conteudo",
        "category",
        "create",
        "--name",
        "Planos",
    ]);
    assert_eq!(codigo(&criada), 0);

    let recusada = cenario.poph(&["--offline", "org", "delete", "org-conteudo"]);
    assert_eq!(codigo(&recusada), 1);
    assert!(texto(&recusada.stderr).contains("1 categoria(s)"));
}

#[test]
fn codigos_de_saida_zero_um_e_dois() {
    let cenario = Cenario::novo("codigos");
    cenario.criar_org("org-codigos");

    // 0: deu certo.
    assert_eq!(codigo(&cenario.poph(&["--offline", "org", "list"])), 0);
    // 1: erro a corrigir (org inexistente).
    assert_eq!(
        codigo(&cenario.poph(&["--offline", "org", "show", "nao-existe"])),
        1
    );
    // 1: falta --offline.
    let sem_offline = cenario.poph(&["org", "list"]);
    assert_eq!(codigo(&sem_offline), 1);
    assert!(texto(&sem_offline.stderr).contains("--offline"));
    // 2: sem ação (uso incorreto da linha de comando).
    assert_eq!(codigo(&cenario.poph(&["--offline", "org"])), 2);
    // 2: falta argumento obrigatório (--if-match no update).
    assert_eq!(
        codigo(&cenario.poph(&["--offline", "org", "update", "org-codigos", "--name", "X"])),
        2
    );
}

#[test]
fn mensagens_de_erro_sao_legiveis() {
    let cenario = Cenario::novo("mensagens");
    cenario.criar_org("org-msg");

    let slug_ruim = cenario.poph(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Teste",
        "--slug",
        "A_B",
    ]);
    assert_eq!(codigo(&slug_ruim), 1);
    assert!(texto(&slug_ruim.stderr).contains("slug `A_B` inválido"));

    let duplicado = cenario.poph(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Outra",
        "--slug",
        "org-msg",
    ]);
    assert_eq!(codigo(&duplicado), 1);
    assert!(texto(&duplicado.stderr).contains("já existe uma org com o slug `org-msg`"));

    let v4 = cenario.poph(&["--offline", "org", "show", UUID_V4]);
    assert_eq!(codigo(&v4), 1);
    assert!(
        texto(&v4.stderr).contains("esperado UUID v7"),
        "{}",
        texto(&v4.stderr)
    );

    let sem_org = cenario.poph(&["--offline", "token", "list"]);
    assert_eq!(codigo(&sem_org), 1);
    assert!(texto(&sem_org.stderr).contains("informe --org"));

    let categoria_ruim =
        cenario.poph(&["--offline", "--org", "org-msg", "category", "show", "abc"]);
    assert_eq!(codigo(&categoria_ruim), 1);
    assert!(texto(&categoria_ruim.stderr).contains("id de categoria de produto inválido"));

    let token_inexistente = cenario.poph(&[
        "--offline",
        "--org",
        "org-msg",
        "token",
        "show",
        "0190a000-0000-7000-8000-000000000000",
    ]);
    assert_eq!(codigo(&token_inexistente), 1);
    assert!(texto(&token_inexistente.stderr).contains("não existe token"));

    // Nome repetido na mesma org, sem diferenciar maiúsculas.
    for nome in ["Planos", "planos"] {
        let _ = cenario.poph(&[
            "--offline",
            "--org",
            "org-msg",
            "category",
            "create",
            "--name",
            nome,
        ]);
    }
    let repetida = cenario.poph(&[
        "--offline",
        "--org",
        "org-msg",
        "category",
        "create",
        "--name",
        "PLANOS",
    ]);
    assert_eq!(codigo(&repetida), 1);
    assert!(
        texto(&repetida.stderr).contains("já existe um registro"),
        "{}",
        texto(&repetida.stderr)
    );

    // Ação sem nenhum campo para mudar.
    let nada = cenario.poph(&["--offline", "org", "update", "org-msg", "--if-match", "1"]);
    assert_eq!(codigo(&nada), 1);
    assert!(texto(&nada.stderr).contains("nada a alterar"));
}

#[test]
fn alteracao_exige_a_versao_certa() {
    let cenario = Cenario::novo("versao");
    cenario.criar_org("org-versao");
    let criada = json(&cenario.poph(&[
        "--offline",
        "--org",
        "org-versao",
        "category",
        "create",
        "--name",
        "Planos",
        "--json",
    ]));
    let id = campo(&criada, "category.id");

    let ok = cenario.poph(&[
        "--offline",
        "--org",
        "org-versao",
        "category",
        "update",
        &id,
        "--if-match",
        "1",
        "--name",
        "Planos 2",
    ]);
    assert_eq!(codigo(&ok), 0, "{}", texto(&ok.stderr));

    // Com a versão antiga (1), depois da alteração (agora 2), a escrita é recusada.
    let antiga = cenario.poph(&[
        "--offline",
        "--org",
        "org-versao",
        "category",
        "update",
        &id,
        "--if-match",
        "1",
        "--name",
        "Outro",
    ]);
    assert_eq!(codigo(&antiga), 1);
    assert!(texto(&antiga.stderr).contains("a versão não confere"));

    let org_ok = cenario.poph(&[
        "--offline",
        "org",
        "update",
        "org-versao",
        "--if-match",
        "1",
        "--name",
        "Nome novo",
    ]);
    assert_eq!(codigo(&org_ok), 0, "{}", texto(&org_ok.stderr));
    assert_eq!(
        json(&cenario.poph(&["--offline", "org", "show", "org-versao", "--json"]))["org"]["version"],
        2
    );
}

#[test]
fn categoria_so_sai_com_force_e_department_sai_direto() {
    let cenario = Cenario::novo("exclusoes");
    cenario.criar_org("org-exclui");
    let categoria = json(&cenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "create",
        "--name",
        "Planos",
        "--json",
    ]));
    let id_categoria = campo(&categoria, "category.id");

    let sem_force = cenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "delete",
        &id_categoria,
    ]);
    assert_eq!(codigo(&sem_force), 1);
    assert!(texto(&sem_force.stderr).contains("--force"));
    let lista = json(&cenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "list",
        "--json",
    ]));
    assert_eq!(lista["total"], 1, "a recusa não pode apagar a categoria");

    let com_force = cenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "category",
        "delete",
        &id_categoria,
        "--force",
    ]);
    assert_eq!(codigo(&com_force), 0, "{}", texto(&com_force.stderr));

    let departamento = json(&cenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "department",
        "create",
        "--name",
        "Vendas",
        "--json",
    ]));
    let id_departamento = campo(&departamento, "department.id");
    let sai = cenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "department",
        "delete",
        &id_departamento,
    ]);
    assert_eq!(codigo(&sai), 0, "{}", texto(&sai.stderr));
    let sumiu = cenario.poph(&[
        "--offline",
        "--org",
        "org-exclui",
        "department",
        "show",
        &id_departamento,
    ]);
    assert_eq!(codigo(&sumiu), 1);
    assert!(texto(&sumiu.stderr).contains("não existe departamento"));
}

#[test]
fn dados_de_uma_org_nao_aparecem_em_outra() {
    let cenario = Cenario::novo("isolamento");
    cenario.criar_org("org-a");
    cenario.criar_org("org-b");

    let categoria = json(&cenario.poph(&[
        "--offline",
        "--org",
        "org-a",
        "category",
        "create",
        "--name",
        "Planos",
        "--json",
    ]));
    let id_categoria = campo(&categoria, "category.id");
    let token = json(&cenario.poph(&[
        "--offline",
        "--org",
        "org-a",
        "token",
        "create",
        "--label",
        "a",
        "--json",
    ]));
    let id_token = campo(&token, "token.id");

    let lista_b =
        json(&cenario.poph(&["--offline", "--org", "org-b", "category", "list", "--json"]));
    assert_eq!(lista_b["total"], 0, "org B não pode ver categoria da org A");

    let mostra_b = cenario.poph(&[
        "--offline",
        "--org",
        "org-b",
        "category",
        "show",
        &id_categoria,
    ]);
    assert_eq!(codigo(&mostra_b), 1);

    let token_b = cenario.poph(&["--offline", "--org", "org-b", "token", "show", &id_token]);
    assert_eq!(codigo(&token_b), 1, "token da org A não aparece na org B");
    let revoga_b = cenario.poph(&["--offline", "--org", "org-b", "token", "delete", &id_token]);
    assert_eq!(codigo(&revoga_b), 1, "org B não revoga token da org A");
}
