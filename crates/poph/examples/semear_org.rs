use std::path::PathBuf;

use poph_store::{NewDepartment, NewOrg, Store};
use poph_types::HarnessConfig;

fn main() {
    let mut args = std::env::args().skip(1);
    let slug = args.next().unwrap_or_else(|| {
        eprintln!("uso: semear_org <slug> [nome]  (ex.: semear_org demo \"Org Demo\")");
        std::process::exit(2);
    });
    let name = args.next().unwrap_or_else(|| format!("Org {slug}"));
    let config = HarnessConfig::default();
    let data_dir: PathBuf = std::env::var_os("POPH_DADOS_DIRETORIO")
        .map(PathBuf::from)
        .unwrap_or(config.data.directory);
    let store = match Store::open_at(&data_dir.join("harness.db"), &data_dir.join("rag.db")) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "não consegui abrir o banco em {}: {error}",
                data_dir.display()
            );
            std::process::exit(1);
        }
    };
    let org = match store.orgs().create(NewOrg {
        name,
        slug: slug.clone(),
        config_json: String::new(),
    }) {
        Ok(org) => org,
        Err(error) => {
            eprintln!("não consegui criar a org `{slug}`: {error}");
            std::process::exit(1);
        }
    };
    let department = store
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
    println!("department_id={}", department.id);
    println!("\nUse: poph --org {slug} agent create --name \"...\"");
}
