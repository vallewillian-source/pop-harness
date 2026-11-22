//! Testes de ponta a ponta de `poph retention run|show` (HR-51).
//!
//! Processo de verdade e banco de verdade numa pasta temporária. A varredura em si (o que sai,
//! o que fica, a simulação) é testada em `crates/poph-retention/tests/banco.rs`; aqui se confere
//! a linha de comando: códigos de saída, formato do JSON e mensagens de erro.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

/// Pasta temporária com a pasta de dados. O banco nasce na primeira execução da CLI.
struct Cenario {
    pasta: PathBuf,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-retencao-{rotulo}-{}", std::process::id()));
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

    /// Cria uma org pela CLI (versão 1) e devolve o id lido do JSON.
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

    /// Troca a configuração da org (a versão é 1, a da criação).
    fn configurar(&self, slug: &str, config: &str) {
        let saida = self.poph(&[
            "--offline",
            "org",
            "update",
            "--if-match",
            "1",
            slug,
            "--config-json",
            config,
        ]);
        assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
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

#[test]
fn run_com_dry_run_em_json_tem_forma_estavel_e_nao_apaga() {
    let cenario = Cenario::novo("dry-json");
    cenario.criar_org("clinica-norte");

    let saida = cenario.poph(&["retention", "run", "--dry-run", "--json"]);

    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    let valor = json(&saida);
    assert_eq!(valor["versao_relatorio"], 1);
    assert_eq!(valor["simulacao"], true);
    let orgs = valor["orgs"].as_array().expect("lista de orgs");
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0]["slug"], "clinica-norte");
    assert_eq!(orgs[0]["situacao"], "varrida");
    assert_eq!(orgs[0]["dias"], 30, "padrão da instância");
    assert_eq!(orgs[0]["banco"]["mensagens"], 0);
    assert!(
        orgs[0]["banco"]["ids_conversas"]
            .as_array()
            .expect("lista")
            .is_empty()
    );
}

#[test]
fn run_sem_orgs_sai_com_0_e_avisa() {
    let cenario = Cenario::novo("sem-orgs");

    let saida = cenario.poph(&["retention", "run"]);

    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    assert!(texto(&saida.stdout).contains("Nenhuma org para varrer"));
}

#[test]
fn org_com_configuracao_invalida_sai_com_1_e_avisa_que_nada_foi_apagado() {
    let cenario = Cenario::novo("config-invalida");
    cenario.criar_org("ruim");
    cenario.configurar("ruim", r#"{"retencao":{"dias":-3}}"#);

    let saida = cenario.poph(&["retention", "run"]);

    assert_eq!(codigo(&saida), 1, "erro de configuração tem de sair com 1");
    let texto_saida = texto(&saida.stdout);
    assert!(texto_saida.contains("Org ruim: erro"), "{texto_saida}");
    assert!(texto_saida.contains("Nada foi apagado"), "{texto_saida}");
}

#[test]
fn org_inexistente_sai_com_1_com_mensagem_clara() {
    let cenario = Cenario::novo("inexistente");
    cenario.criar_org("existe");

    let saida = cenario.poph(&["--org", "nao-existe", "retention", "run"]);

    assert_eq!(codigo(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("não existe org com o slug `nao-existe`"),
        "{}",
        texto(&saida.stderr)
    );
}

#[test]
fn show_mostra_padrao_e_retencao_zero_da_org() {
    let cenario = Cenario::novo("show");
    cenario.criar_org("guarda-tudo");
    cenario.criar_org("padrao");
    cenario.configurar("guarda-tudo", r#"{"retencao":{"dias":0}}"#);

    let saida = cenario.poph(&["retention", "show"]);

    assert_eq!(codigo(&saida), 0, "{}", texto(&saida.stderr));
    let texto_saida = texto(&saida.stdout);
    assert!(
        texto_saida.contains("Org guarda-tudo: nunca apagar (definida na org)"),
        "{texto_saida}"
    );
    assert!(
        texto_saida.contains("Org padrao: 30 dias (padrão da instância)"),
        "{texto_saida}"
    );
}

#[test]
fn show_em_json_com_org_invalida_sai_com_1() {
    let cenario = Cenario::novo("show-json");
    cenario.criar_org("ruim");
    cenario.configurar("ruim", r#"{"retencao":{"dias":"trinta"}}"#);

    let saida = cenario.poph(&["retention", "show", "--json"]);

    assert_eq!(codigo(&saida), 1);
    let valor = json(&saida);
    assert_eq!(valor["versao_relatorio"], 1);
    assert_eq!(valor["padrao_dias"], 30);
    assert_eq!(valor["orgs"][0]["slug"], "ruim");
    assert!(valor["orgs"][0]["dias"].is_null());
    assert!(
        valor["orgs"][0]["erro"]
            .as_str()
            .expect("mensagem de erro")
            .contains("Nada foi apagado")
    );
}
