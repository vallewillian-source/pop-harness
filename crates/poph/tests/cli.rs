//! Testes do binário `poph` de ponta a ponta: processo de verdade, ambiente controlado.
//!
//! Os valores de segredo usados aqui são sentinelas (`SENTINELA-...`). Os testes provam que
//! nenhum deles aparece no stdout, no stderr nem no JSON, mesmo com log em nível `trace`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SENTINELA_CIFRA: &str = "SENTINELA-CIFRA-7f3a";
const SENTINELA_LLM: &str = "SENTINELA-LLM-91bc";
const SENTINELA_JEV: &str = "SENTINELA-JEV-204d";

fn pasta_de_teste(rotulo: &str) -> PathBuf {
    let pasta = std::env::temp_dir().join(format!("poph-cli-{rotulo}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria pasta de teste");
    pasta
}

/// Roda `poph` com ambiente limpo, pasta atual isolada e as variáveis dadas.
fn poph(pasta_atual: &Path, args: &[&str], variaveis: &[(&str, &str)]) -> Output {
    let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
    comando.args(args).current_dir(pasta_atual).env_clear();
    for (chave, valor) in variaveis {
        comando.env(chave, valor);
    }
    comando.output().expect("executa o poph")
}

/// Ambiente completo e correto, com a pasta de dados dentro da pasta de teste.
fn ambiente_completo(pasta: &Path) -> Vec<(&'static str, String)> {
    vec![
        (
            "POPH_DADOS_DIRETORIO",
            pasta.join("dados").display().to_string(),
        ),
        ("POPH_CHAVE_CIFRA", SENTINELA_CIFRA.to_owned()),
        ("ANTIGRAVITY_API_KEY", SENTINELA_LLM.to_owned()),
    ]
}

fn como_refs<'a>(pares: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    pares.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

fn texto(saida: &[u8]) -> String {
    String::from_utf8_lossy(saida).into_owned()
}

fn sem_sentinelas(saida: &str) {
    for sentinela in [SENTINELA_CIFRA, SENTINELA_LLM, SENTINELA_JEV] {
        assert!(
            !saida.contains(sentinela),
            "segredo vazou na saída: {saida}"
        );
    }
}

#[test]
fn versao_e_ajuda_continuam_funcionando() {
    let pasta = pasta_de_teste("versao");
    let versao = poph(&pasta, &["--version"], &[]);
    assert!(versao.status.success());
    assert_eq!(texto(&versao.stdout).trim(), "poph 0.1.0");

    let ajuda = poph(&pasta, &["--help"], &[]);
    assert!(ajuda.status.success());
    assert!(texto(&ajuda.stdout).contains("doctor"));
}

#[test]
fn sem_comando_mostra_ajuda_e_sai_com_2() {
    let pasta = pasta_de_teste("sem-comando");
    let saida = poph(&pasta, &[], &[]);
    assert_eq!(saida.status.code(), Some(2));
    // O clap escreve a ajuda no erro padrão quando falta comando.
    assert!(texto(&saida.stderr).contains("doctor"));
}

#[test]
fn doctor_com_ambiente_correto_sai_0_e_nao_mostra_segredo() {
    let pasta = pasta_de_teste("ok");
    let ambiente = ambiente_completo(&pasta);
    let saida = poph(&pasta, &["doctor"], &como_refs(&ambiente));
    let stdout = texto(&saida.stdout);
    assert_eq!(saida.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("Ambiente pronto"), "stdout: {stdout}");
    assert!(stdout.contains("variável POPH_CHAVE_CIFRA: definida"));
    sem_sentinelas(&stdout);
    sem_sentinelas(&texto(&saida.stderr));
}

#[test]
fn doctor_json_e_valido_e_nao_mostra_segredo() {
    let pasta = pasta_de_teste("json");
    let ambiente = ambiente_completo(&pasta);
    let saida = poph(&pasta, &["doctor", "--json"], &como_refs(&ambiente));
    let stdout = texto(&saida.stdout);
    assert_eq!(saida.status.code(), Some(0), "stdout: {stdout}");
    sem_sentinelas(&stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("stdout é JSON válido");
    assert_eq!(json["versao_relatorio"], 1);
    assert_eq!(json["status"], "ok");
}

#[test]
fn variavel_obrigatoria_faltando_sai_diferente_de_0_e_diz_qual() {
    let pasta = pasta_de_teste("sem-cifra");
    let mut ambiente = ambiente_completo(&pasta);
    ambiente.retain(|(k, _)| *k != "POPH_CHAVE_CIFRA");
    let saida = poph(&pasta, &["doctor"], &como_refs(&ambiente));
    let stdout = texto(&saida.stdout);
    assert_ne!(saida.status.code(), Some(0));
    assert!(stdout.contains("POPH_CHAVE_CIFRA"), "stdout: {stdout}");
    assert!(stdout.contains("erro(s) a corrigir"), "stdout: {stdout}");
    sem_sentinelas(&stdout);
}

#[test]
fn segredos_nao_vazam_no_log_mesmo_em_nivel_trace_e_json() {
    let pasta = pasta_de_teste("log");
    let mut ambiente = ambiente_completo(&pasta);
    ambiente.push(("TYPESAFE_API_KEY", SENTINELA_JEV.to_owned()));
    ambiente.push(("POPH_JEV_ATIVO", "true".to_owned()));
    ambiente.push(("POPH_LOG_NIVEL", "trace".to_owned()));
    ambiente.push(("POPH_LOG_FORMATO", "json".to_owned()));
    let saida = poph(&pasta, &["doctor", "--json"], &como_refs(&ambiente));
    let stderr = texto(&saida.stderr);
    assert!(
        stderr.contains("verificacao"),
        "o log em trace deveria ter eventos: {stderr}"
    );
    for linha in stderr.lines().filter(|l| !l.trim().is_empty()) {
        serde_json::from_str::<serde_json::Value>(linha)
            .unwrap_or_else(|erro| panic!("linha de log não é JSON ({erro}): {linha}"));
    }
    sem_sentinelas(&stderr);
    sem_sentinelas(&texto(&saida.stdout));
}

#[test]
fn toml_invalido_diz_arquivo_e_chave_e_sai_diferente_de_0() {
    let pasta = pasta_de_teste("toml");
    let arquivo = pasta.join("poph.toml");
    fs::write(&arquivo, "[servidor]\nportaa = 2000\n").expect("escreve");
    let ambiente = ambiente_completo(&pasta);
    let caminho = arquivo.display().to_string();
    let args = ["--config", caminho.as_str(), "doctor"];
    let saida = poph(&pasta, &args, &como_refs(&ambiente));
    let stdout = texto(&saida.stdout);
    assert_ne!(saida.status.code(), Some(0));
    assert!(stdout.contains(&caminho), "faltou o arquivo: {stdout}");
    assert!(
        stdout.contains("servidor.portaa"),
        "faltou a chave: {stdout}"
    );
    sem_sentinelas(&stdout);
}

#[test]
fn precedencia_do_arquivo_e_do_ambiente_aparece_no_doctor() {
    let pasta = pasta_de_teste("precedencia");
    let arquivo = pasta.join("poph.toml");
    fs::write(&arquivo, "[dados]\ndiretorio = \"do-arquivo\"\n").expect("escreve");
    let caminho = arquivo.display().to_string();
    let args = ["--config", caminho.as_str(), "doctor", "--json"];
    let ambiente = ambiente_completo(&pasta);
    // O ambiente vence o arquivo: a pasta de dados vira a do ambiente.
    let saida = poph(&pasta, &args, &como_refs(&ambiente));
    let stdout = texto(&saida.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("JSON");
    let pasta_dados = json["verificacoes"]
        .as_array()
        .expect("lista")
        .iter()
        .find(|v| v["nome"] == "pasta de dados")
        .expect("tem pasta de dados")["detalhe"]
        .as_str()
        .expect("texto")
        .to_owned();
    assert!(pasta_dados.contains("dados"), "detalhe: {pasta_dados}");
    assert!(
        !pasta_dados.contains("do-arquivo"),
        "arquivo venceu o ambiente: {pasta_dados}"
    );
}

#[test]
fn migrate_cria_os_bancos_e_e_idempotente() {
    let pasta = pasta_de_teste("migrate");
    let ambiente = ambiente_completo(&pasta);

    // Primeira vez: cria os dois bancos do zero e aplica o esquema inicial.
    let primeira = poph(&pasta, &["migrate"], &como_refs(&ambiente));
    let saida = texto(&primeira.stdout);
    assert_eq!(primeira.status.code(), Some(0), "saída: {saida}");
    let alvo = poph_store::harness_target_version();
    assert!(
        saida.contains(&format!("versão 0 → {alvo}")),
        "saída: {saida}"
    );
    assert!(saida.contains("0001_esquema_inicial"), "saída: {saida}");
    assert!(
        saida.contains("0003_origem_autor_mensagens"),
        "saída: {saida}"
    );
    assert!(
        saida.contains("0002_entrada_proxima_tentativa"),
        "saída: {saida}"
    );
    assert!(
        pasta.join("dados").join("harness.db").is_file(),
        "harness.db não foi criado"
    );
    sem_sentinelas(&saida);

    // Segunda vez: nada a aplicar, mesmo código de saída.
    let segunda = poph(&pasta, &["migrate", "--json"], &como_refs(&ambiente));
    let json: serde_json::Value =
        serde_json::from_str(&texto(&segunda.stdout)).expect("JSON do migrate");
    assert_eq!(segunda.status.code(), Some(0));
    assert_eq!(json["ja_estava_em_dia"], true);
    // A versão vem da fonte única do `poph-store` (versão alvo do esquema do Harness).
    let versao_do_esquema = poph_store::harness_target_version();
    assert_eq!(
        json["harness"]["versao_depois"].as_u64(),
        Some(u64::from(versao_do_esquema))
    );
    assert_eq!(json["rag"]["versao_depois"], 0);
    assert_eq!(
        json["harness"]["aplicadas"].as_array().map(Vec::len),
        Some(0)
    );

    // O doctor passa a mostrar a versão real do esquema e continua sem vazar segredo.
    let doctor = poph(&pasta, &["doctor"], &como_refs(&ambiente));
    let saida = texto(&doctor.stdout);
    assert_eq!(doctor.status.code(), Some(0), "saída: {saida}");
    assert!(
        saida.contains(&format!("esquema versão {versao_do_esquema}")),
        "saída: {saida}"
    );
    sem_sentinelas(&saida);
}
