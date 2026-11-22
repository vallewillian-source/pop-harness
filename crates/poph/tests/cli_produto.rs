//! Testes de ponta a ponta de `poph product` (HR-18): processo de verdade, banco e índice de verdade.
//!
//! O índice exige o modelo de embedding, então o cenário aponta a pasta de modelos para a do
//! repositório (symlink em `dados/modelos`). Sem o modelo instalado, os testes são pulados — como
//! os do `poph-rag` e do `poph kb`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use poph_store::{NovaCategoria, NovaOrg, Store};
use serde_json::Value;

const SENTINELA_CIFRA: &str = "SENTINELA-CIFRA-hr18";
const SENTINELA_LLM: &str = "SENTINELA-LLM-hr18";

fn pasta_modelos_repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos"))
}

struct Cenario {
    pasta: PathBuf,
    slug: String,
}

impl Cenario {
    /// `None` quando o modelo não está instalado (teste pulado).
    fn novo(rotulo: &str) -> Option<Self> {
        let modelos = pasta_modelos_repo();
        if !modelos.join("model_int8.onnx").is_file() || !modelos.join("tokenizer.json").is_file() {
            eprintln!("PULADO ({rotulo}): modelo não instalado; rode `poph model install`");
            return None;
        }
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-produto-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("cria a pasta de dados");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&modelos, pasta.join("dados").join("modelos"))
            .expect("liga a pasta de modelos");
        let slug = format!("org-{rotulo}");
        {
            let store = Store::abrir_em(
                &pasta.join("dados").join("harness.db"),
                &pasta.join("dados").join("rag.db"),
            )
            .expect("abre o banco");
            let org = store
                .orgaos()
                .criar(NovaOrg {
                    nome: format!("Org {rotulo}"),
                    slug: slug.clone(),
                    config_json: "{}".to_owned(),
                })
                .expect("cria a org")
                .id;
            store
                .agentes()
                .criar(
                    org,
                    poph_store::NovoAgente {
                        nome: "Bia".to_owned(),
                        slug: "bia".to_owned(),
                        ..poph_store::NovoAgente::default()
                    },
                )
                .expect("cria o agente");
            store
                .categorias()
                .criar(
                    org,
                    NovaCategoria {
                        nome: "Pragas urbanas".to_owned(),
                        descricao: String::new(),
                    },
                )
                .expect("cria a categoria");
        }
        Some(Self { pasta, slug })
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
        comando
            .arg("--offline")
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.pasta.join("dados"))
            .env("POPH_CHAVE_CIFRA", SENTINELA_CIFRA)
            .env("ANTIGRAVITY_API_KEY", SENTINELA_LLM);
        comando.output().expect("executa o poph")
    }

    fn ok(&self, args: &[&str]) -> Value {
        let saida = self.poph(args);
        assert!(
            saida.status.success(),
            "`poph {}` falhou: {}",
            args.join(" "),
            String::from_utf8_lossy(&saida.stderr)
        );
        let texto = String::from_utf8_lossy(&saida.stdout);
        assert!(!texto.contains(SENTINELA_CIFRA) && !texto.contains(SENTINELA_LLM));
        serde_json::from_str(&texto).unwrap_or_else(|erro| {
            panic!("saída não é JSON ({erro}): {texto}");
        })
    }

    fn falha(&self, args: &[&str]) -> String {
        let saida = self.poph(args);
        assert!(
            !saida.status.success(),
            "`poph {}` devia falhar",
            args.join(" ")
        );
        String::from_utf8_lossy(&saida.stderr).into_owned()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

#[test]
fn cli_product_cria_mostra_altera_e_apaga() {
    let Some(cenario) = Cenario::novo("ciclo") else {
        return;
    };
    let org = cenario.slug.clone();

    let criado = cenario.ok(&[
        "--org",
        &org,
        "product",
        "create",
        "--name",
        "Dedetização de baratas",
        "--category",
        "Pragas urbanas",
        "--alias",
        "baratas",
        "--alias",
        "barata",
        "--tag",
        "cozinha",
        "--price",
        "32000",
        "--price-text",
        "R$ 320,00",
        "--unit",
        "por visita",
        "--conditions",
        "Pagamento em até 3x",
        "--json",
    ]);
    let produto = &criado["product"];
    assert_eq!(produto["name"], "Dedetização de baratas");
    assert_eq!(produto["slug"], "dedetizacao-de-baratas");
    assert_eq!(produto["price_cents"], 32_000);
    assert_eq!(produto["aliases"][0], "baratas");
    assert_eq!(produto["status"], "active");
    // A escrita reindexa o agente da org.
    let agente = cenario.ok(&["--org", &org, "agent", "show", "bia", "--json"]);
    assert!(agente["agent"]["slug"] == "bia");
    let id = produto["id"].as_str().expect("id").to_owned();

    let lista = cenario.ok(&["--org", &org, "product", "list", "--json"]);
    assert_eq!(lista["total"], 1);
    assert_eq!(lista["products"][0]["id"], id);

    let mostrado = cenario.ok(&["--org", &org, "product", "show", &id, "--json"]);
    assert_eq!(mostrado["product"]["version"], 1);

    let alterado = cenario.ok(&[
        "--org",
        &org,
        "product",
        "update",
        &id,
        "--if-match",
        "1",
        "--price",
        "27990",
        "--price-text",
        "R$ 279,90",
        "--json",
    ]);
    assert_eq!(alterado["product"]["price_cents"], 27_990);
    assert_eq!(alterado["product"]["version"], 2);

    // `If-Match` velho é recusado.
    let erro = cenario.falha(&[
        "--org",
        &org,
        "product",
        "update",
        &id,
        "--if-match",
        "1",
        "--price",
        "1000",
    ]);
    assert!(
        erro.to_lowercase().contains("versão") || erro.to_lowercase().contains("versao"),
        "erro de conflito de versão esperado: {erro}"
    );

    // Sem `--force` a exclusão não acontece.
    let sem_force = cenario.falha(&["--org", &org, "product", "delete", &id]);
    assert!(sem_force.contains("--force"), "{sem_force}");

    let apagado = cenario.ok(&["--org", &org, "product", "delete", &id, "--force", "--json"]);
    assert_eq!(apagado["deleted"], id);
    let lista = cenario.ok(&["--org", &org, "product", "list", "--json"]);
    assert_eq!(lista["total"], 0);
}

#[test]
fn cli_product_reindexa_e_respeita_o_limite_de_preco() {
    let Some(cenario) = Cenario::novo("reindex") else {
        return;
    };
    let org = cenario.slug.clone();
    let criado = cenario.ok(&[
        "--org",
        &org,
        "product",
        "create",
        "--name",
        "Desratização",
        "--category",
        "Pragas urbanas",
        "--alias",
        "ratos",
        "--price",
        "25000",
        "--json",
    ]);
    let id = criado["product"]["id"].as_str().expect("id").to_owned();

    let reindexado = cenario.ok(&["--org", &org, "product", "reindex", "--json"]);
    let agentes = reindexado["agents"].as_array().expect("agentes");
    assert_eq!(agentes.len(), 1);
    assert_eq!(agentes[0]["unchanged"], 1, "nada mudou desde a escrita");

    // Preço negativo é recusado com erro claro, sem gravar nada.
    let erro = cenario.falha(&[
        "--org",
        &org,
        "product",
        "update",
        &id,
        "--if-match",
        "1",
        "--price=-1",
    ]);
    assert!(erro.contains("negativo"), "{erro}");

    // Categoria inexistente é recusada.
    let erro = cenario.falha(&[
        "--org",
        &org,
        "product",
        "create",
        "--name",
        "Produto sem categoria",
        "--category",
        "Não existe",
        "--price",
        "100",
    ]);
    assert!(erro.contains("não existe categoria"), "{erro}");
}
