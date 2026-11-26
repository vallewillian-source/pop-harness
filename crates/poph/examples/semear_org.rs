//! Ajuda de demonstração da CLI: cria uma org (e um departamento) direto no banco.
//!
//! Existe só porque o `poph org` ainda não foi implementado (HR-08). Quando ele existir, este
//! exemplo pode ser apagado: nada do motor depende dele.
//!
//! Uso (com a pasta de dados padrão `data/` ou `POPH_DADOS_DIRETORIO`):
//! ```text
//! cargo run -p poph --example semear_org -- demo "Org Demonstração"
//! ```

use std::path::PathBuf;

use poph_store::{NewDepartment, NewOrg, Store};
use poph_types::HarnessConfig;

fn main() {
    let mut args = std::env::args().skip(1);
    let slug = args.next().unwrap_or_else(|| {
        eprintln!("uso: semear_org <slug> [nome]  (ex.: semear_org demo \"Org Demo\")");
        std::process::exit(2);
    });
    let nome = args.next().unwrap_or_else(|| format!("Org {slug}"));
    let config = HarnessConfig::default();
    let pasta: PathBuf = std::env::var_os("POPH_DADOS_DIRETORIO")
        .map(PathBuf::from)
        .unwrap_or(config.data.directory);
    let store = match Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")) {
        Ok(store) => store,
        Err(erro) => {
            eprintln!("não consegui abrir o banco em {}: {erro}", pasta.display());
            std::process::exit(1);
        }
    };
    let org = match store.orgs().create(NewOrg {
        name: nome,
        slug: slug.clone(),
        config_json: String::new(),
    }) {
        Ok(org) => org,
        Err(erro) => {
            eprintln!("não consegui criar a org `{slug}`: {erro}");
            std::process::exit(1);
        }
    };
    let departamento = store
        .departments()
        .create(
            org.id,
            NewDepartment {
                name: "Atendimento".to_owned(),
                description: "Departamento padrão da demo".to_owned(),
            },
        )
        .expect("cria o departamento da demo");
    println!("org_id={}", org.id);
    println!("slug={}", org.slug);
    println!("department_id={}", departamento.id);
    println!("\nUse: poph --org {slug} agent create --name \"...\"");
}
