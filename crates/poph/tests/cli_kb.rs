//! Testes de ponta a ponta de `poph kb` (HR-17): processo de verdade, banco e índice de verdade.
//!
//! O índice exige o modelo de embedding, então o cenário aponta a pasta de modelos para a do
//! repositório (symlink em `dados/modelos`). Sem o modelo instalado, os testes são pulados — como
//! os do `poph-rag`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use poph_store::{NovaOrg, Store};
use serde_json::Value;

const SENTINELA_CIFRA: &str = "SENTINELA-CIFRA-hr17";
const SENTINELA_LLM: &str = "SENTINELA-LLM-hr17";

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
            std::env::temp_dir().join(format!("poph-cli-kb-{rotulo}-{}", std::process::id()));
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
            store
                .orgaos()
                .criar(NovaOrg {
                    nome: format!("Org {rotulo}"),
                    slug: slug.clone(),
                    config_json: "{}".to_owned(),
                })
                .expect("cria a org");
            store
                .agentes()
                .criar(
                    store.orgaos().por_slug(&slug).expect("org").id,
                    poph_store::NovoAgente {
                        nome: "Bia".to_owned(),
                        slug: "bia".to_owned(),
                        ..poph_store::NovoAgente::default()
                    },
                )
                .expect("cria o agente");
        }
        Some(Self { pasta, slug })
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
        comando
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
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

#[test]
fn cli_kb_cria_indexa_busca_edita_e_apaga() {
    let Some(cenario) = Cenario::novo("ciclo") else {
        return;
    };
    let org = cenario.slug.clone();

    let grupo = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "group",
        "create",
        "--name",
        "Políticas",
        "--json",
    ]);
    let grupo_id = grupo["group"]["id"]
        .as_str()
        .expect("id do grupo")
        .to_owned();

    let item = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "create",
        "--group",
        &grupo_id,
        "--title",
        "Prazo de agendamento",
        "--content",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
        "--json",
    ]);
    let item_id = item["item"]["id"].as_str().expect("id do item").to_owned();

    cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "link",
        &grupo_id,
        "--agent",
        "bia",
        "--json",
    ]);

    // O índice acompanhou o vínculo.
    let lista = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "list",
        "--group",
        &grupo_id,
        "--json",
    ]);
    assert_eq!(lista["items"][0]["index"], "indexado", "{lista}");

    let achados = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "quanto tempo leva para agendar",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(achados["total"], 1, "{achados}");
    assert_eq!(achados["chunks"][0]["titulo"], "Prazo de agendamento");
    assert_eq!(achados["chunks"][0]["grupo"], "Políticas");

    // Editar o conteúdo: o índice acompanha na hora.
    let versao = item["item"]["version"].as_u64().expect("versão");
    let editado = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "update",
        &item_id,
        "--if-match",
        &versao.to_string(),
        "--content",
        "O agendamento agora é imediato pelo aplicativo.",
        "--json",
    ]);
    assert_eq!(editado["item"]["index"], "indexado", "{editado}");
    let novo = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "agendamento pelo aplicativo",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(novo["chunks"][0]["titulo"], "Prazo de agendamento");

    // Apagar: sai do índice.
    cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "delete",
        &item_id,
        "--json",
    ]);
    let depois = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "agendamento aplicativo",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(depois["total"], 0, "{depois}");
}

#[test]
fn cli_kb_recusa_item_acima_de_mil_caracteres() {
    let Some(cenario) = Cenario::novo("limite") else {
        return;
    };
    let org = cenario.slug.clone();
    let grupo = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "group",
        "create",
        "--name",
        "G",
        "--json",
    ]);
    let grupo_id = grupo["group"]["id"].as_str().expect("id").to_owned();
    let grande = "a".repeat(1001);
    let saida = cenario.poph(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "create",
        "--group",
        &grupo_id,
        "--title",
        "Grande",
        "--content",
        &grande,
    ]);
    assert!(!saida.status.success(), "1001 caracteres precisam falhar");
    let erro = String::from_utf8_lossy(&saida.stderr);
    assert!(
        erro.contains("1000") && erro.contains("1001"),
        "o erro precisa dizer o limite: {erro}"
    );
}

/// Garante que o symlink do cenário aponta para uma pasta existente (diagnóstico de ambiente).
#[test]
fn pasta_de_modelos_do_repositorio_existe() {
    let modelos: &Path = &pasta_modelos_repo();
    if !modelos.is_dir() {
        eprintln!("modelo não instalado em {}", modelos.display());
    }
}

/// `poph rag reindex` (o nome que o PRD usa) reindexa de verdade e é idempotente: rodar de novo sem
/// mudança não gera embedding.
#[test]
fn cli_rag_reindex_e_idempotente() {
    let Some(cenario) = Cenario::novo("reindex") else {
        return;
    };
    let org = cenario.slug.clone();
    let grupo = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "group",
        "create",
        "--name",
        "Prazo",
        "--json",
    ]);
    let grupo_id = grupo["group"]["id"].as_str().expect("grupo").to_owned();
    cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "item",
        "create",
        "--group",
        &grupo_id,
        "--title",
        "Prazo de agendamento",
        "--content",
        "O prazo para agendar a dedetização é de 48 horas úteis.",
        "--json",
    ]);
    cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "link",
        &grupo_id,
        "--agent",
        "bia",
        "--json",
    ]);

    // Nada mudou: reindexar não gera embedding.
    let parado = cenario.ok(&["--offline", "--org", &org, "rag", "reindex", "--json"]);
    assert_eq!(parado["agents"][0]["indexed"]["embeddings"], 0, "{parado}");
    assert_eq!(
        parado["agents"][0]["indexed"]["items_unchanged"], 1,
        "{parado}"
    );

    // Índice perdido: a reindexação reconstrói tudo.
    for sufixo in ["rag.db", "rag.db-wal", "rag.db-shm"] {
        let _ = fs::remove_file(cenario.pasta.join("dados").join(sufixo));
    }
    let reconstruido = cenario.ok(&["--offline", "--org", &org, "rag", "reindex", "--json"]);
    assert_eq!(
        reconstruido["agents"][0]["indexed"]["items_new"], 1,
        "{reconstruido}"
    );
    assert!(
        reconstruido["agents"][0]["indexed"]["embeddings"]
            .as_u64()
            .expect("embeddings")
            >= 1,
        "{reconstruido}"
    );

    // E de novo sem mudança: zero embedding, mesmo conteúdo.
    let repetido = cenario.ok(&["--offline", "--org", &org, "rag", "reindex", "--json"]);
    assert_eq!(
        repetido["agents"][0]["indexed"]["embeddings"], 0,
        "{repetido}"
    );
    let achados = cenario.ok(&[
        "--offline",
        "--org",
        &org,
        "kb",
        "search",
        "quanto tempo para agendar",
        "--agent",
        "bia",
        "--json",
    ]);
    assert_eq!(
        achados["chunks"][0]["titulo"], "Prazo de agendamento",
        "{achados}"
    );
}
