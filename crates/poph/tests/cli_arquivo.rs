//! Testes de ponta a ponta de `poph file` (HR-24): processo de verdade e banco de verdade.
//!
//! O upload de imagem não depende do `poph-extrator`; os documentos (PDF/Office) dependem, e são
//! cobertos pelos testes do crate `poph-files`. Aqui o foco é o contrato da CLI: recusa legível com
//! código 1 (arquivo grande, extensão enganosa), blob deduplicado por hash, `list`/`show`/`delete`,
//! `--json` estável e isolamento por agente.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use poph_store::{NewAgent, NewOrg, Store};
use serde_json::Value;

/// PNG mínimo: a assinatura é o que a detecção por conteúdo usa.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

struct Cenario {
    pasta: PathBuf,
    slug: String,
    agente: String,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-arquivo-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("cria a pasta de dados");
        let slug = format!("org-{rotulo}");
        {
            let store = Store::open_at(
                &pasta.join("dados").join("harness.db"),
                &pasta.join("dados").join("rag.db"),
            )
            .expect("abre o banco");
            let org = store
                .orgs()
                .create(NewOrg {
                    name: format!("Org {rotulo}"),
                    slug: slug.clone(),
                    config_json: "{}".to_owned(),
                })
                .expect("cria a org")
                .id;
            store
                .agents()
                .create(
                    org,
                    NewAgent {
                        name: "Bia".to_owned(),
                        slug: "bia".to_owned(),
                        ..NewAgent::default()
                    },
                )
                .expect("cria o agente");
        }
        Self {
            pasta,
            slug,
            agente: "bia".to_owned(),
        }
    }

    fn poph(&self, args: &[&str]) -> Output {
        let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
        comando
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.pasta.join("dados"));
        comando.output().expect("executa o poph")
    }

    /// Roda com `--org` e `--offline` e devolve a saída padrão.
    fn ok(&self, args: &[&str]) -> String {
        let mut todos: Vec<&str> = vec!["--offline", "--org", &self.slug];
        todos.extend_from_slice(args);
        let saida = self.poph(&todos);
        assert!(
            saida.status.success(),
            "`poph {}` falhou: {}",
            todos.join(" "),
            String::from_utf8_lossy(&saida.stderr)
        );
        String::from_utf8_lossy(&saida.stdout).into_owned()
    }

    fn ok_json(&self, args: &[&str]) -> Value {
        let texto = self.ok(args);
        serde_json::from_str(&texto)
            .unwrap_or_else(|erro| panic!("saída não é JSON ({erro}): {texto}"))
    }

    /// Roda esperando falha: devolve (código de saída, erro padrão).
    fn falhou(&self, args: &[&str]) -> (i32, String) {
        let mut todos: Vec<&str> = vec!["--offline", "--org", &self.slug];
        todos.extend_from_slice(args);
        let saida = self.poph(&todos);
        (
            saida.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&saida.stderr).into_owned(),
        )
    }

    fn escrever(&self, nome: &str, bytes: &[u8]) -> PathBuf {
        let caminho = self.pasta.join(nome);
        fs::write(&caminho, bytes).expect("grava o arquivo de teste");
        caminho
    }

    /// Quantos blobs existem na pasta da org.
    fn blobs(&self) -> Vec<String> {
        let pasta = self.pasta.join("dados").join("blobs");
        let Ok(orgs) = fs::read_dir(&pasta) else {
            return Vec::new();
        };
        let mut nomes = Vec::new();
        for org in orgs.flatten() {
            if let Ok(arquivos) = fs::read_dir(org.path()) {
                for arquivo in arquivos.flatten() {
                    nomes.push(arquivo.file_name().to_string_lossy().into_owned());
                }
            }
        }
        nomes
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn upload_json(cenario: &Cenario, caminho: &Path, extra: &[&str]) -> Value {
    let caminho = caminho.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec![
        "file",
        "upload",
        &caminho,
        "--agent",
        &cenario.agente,
        "--json",
    ];
    args.extend_from_slice(extra);
    cenario.ok_json(&args)
}

#[test]
fn envia_lista_mostra_e_exclui_arquivo() {
    let cenario = Cenario::novo("crud");
    let caminho = cenario.escrever("logo.png", PNG);

    let criado = upload_json(
        &cenario,
        &caminho,
        &[
            "--name",
            "Logo da empresa",
            "--description",
            "logotipo para o cliente",
        ],
    );
    let arquivo = &criado["arquivo"];
    let id = arquivo["id"].as_str().expect("id no JSON").to_owned();
    assert_eq!(arquivo["nome"], "Logo da empresa");
    assert_eq!(arquivo["descricao"], "logotipo para o cliente");
    assert_eq!(arquivo["mime"], "image/png");
    assert!(
        arquivo["agent_id"].as_str().is_some(),
        "o dono do arquivo vem no JSON"
    );
    assert!(!arquivo["blob_path"].as_str().unwrap().starts_with('/'));

    // Lista: aparece com o nome cadastrado.
    let lista = cenario.ok_json(&["file", "list", "--agent", &cenario.agente, "--json"]);
    assert_eq!(lista["total"], 1);
    assert_eq!(lista["arquivos"][0]["id"], id.as_str());

    // Mostra.
    let mostrado = cenario.ok_json(&["file", "show", &id, "--agent", &cenario.agente, "--json"]);
    assert_eq!(mostrado["arquivo"]["id"], id.as_str());

    // Exclui e a lista volta vazia.
    let excluido = cenario.ok_json(&["file", "delete", &id, "--agent", &cenario.agente, "--json"]);
    assert_eq!(excluido["excluido"]["id"], id.as_str());
    let lista = cenario.ok_json(&["file", "list", "--agent", &cenario.agente, "--json"]);
    assert_eq!(lista["total"], 0);

    // O blob fica no disco (quem apaga órfão é a retenção).
    assert_eq!(cenario.blobs().len(), 1);
}

#[test]
fn mesmo_conteudo_duas_vezes_gera_um_blob_so() {
    let cenario = Cenario::novo("dedupe");
    let caminho = cenario.escrever("logo.png", PNG);
    let primeiro = upload_json(&cenario, &caminho, &[]);
    let segundo = upload_json(&cenario, &caminho, &[]);
    assert_eq!(
        primeiro["arquivo"]["sha256"], segundo["arquivo"]["sha256"],
        "mesmo conteúdo, mesmo hash"
    );
    assert_ne!(primeiro["arquivo"]["id"], segundo["arquivo"]["id"]);
    assert_eq!(cenario.blobs().len(), 1, "um blob só por hash");
}

#[test]
fn arquivo_acima_de_16_mb_e_recusado() {
    let cenario = Cenario::novo("grande");
    // 17 MB de zeros: recusado pelo tamanho, antes de qualquer leitura de documento.
    let caminho = cenario.escrever("grande.pdf", &vec![0u8; 17 * 1024 * 1024]);
    let (codigo, erro) = cenario.falhou(&[
        "file",
        "upload",
        &caminho.to_string_lossy(),
        "--agent",
        &cenario.agente,
    ]);
    assert_eq!(codigo, 1, "{erro}");
    assert!(
        erro.contains("16"),
        "mensagem precisa dizer o limite: {erro}"
    );
    // Nada foi gravado.
    let lista = cenario.ok_json(&["file", "list", "--agent", &cenario.agente, "--json"]);
    assert_eq!(lista["total"], 0);
    assert!(cenario.blobs().is_empty());
}

#[test]
fn extensao_enganosa_e_recusada() {
    let cenario = Cenario::novo("enganosa");
    // Conteúdo de imagem com nome de PDF: o tipo real não é o declarado.
    let caminho = cenario.escrever("contrato.pdf", PNG);
    let (codigo, erro) = cenario.falhou(&[
        "file",
        "upload",
        &caminho.to_string_lossy(),
        "--agent",
        &cenario.agente,
    ]);
    assert_eq!(codigo, 1, "{erro}");
    assert!(
        erro.to_lowercase().contains("pdf") && erro.to_lowercase().contains("png"),
        "{erro}"
    );
    let lista = cenario.ok_json(&["file", "list", "--agent", &cenario.agente, "--json"]);
    assert_eq!(lista["total"], 0);
}

#[test]
fn arquivo_inexistente_e_agente_errado_falham_com_codigo_1() {
    let cenario = Cenario::novo("erros");
    let (codigo, erro) = cenario.falhou(&[
        "file",
        "upload",
        "/caminho/que/nao/existe.png",
        "--agent",
        &cenario.agente,
    ]);
    assert_eq!(codigo, 1);
    assert!(erro.contains("não consegui abrir"), "{erro}");

    let caminho = cenario.escrever("logo.png", PNG);
    let (codigo, erro) = cenario.falhou(&[
        "file",
        "upload",
        &caminho.to_string_lossy(),
        "--agent",
        "agente-que-nao-existe",
    ]);
    assert_eq!(codigo, 1, "{erro}");

    // Id inválido (não é UUID v7).
    let (codigo, erro) =
        cenario.falhou(&["file", "show", "nao-e-uuid", "--agent", &cenario.agente]);
    assert_eq!(codigo, 1, "{erro}");
}

#[test]
fn sem_offline_o_comando_recusa() {
    // Sem `--offline` o comando falha com código 1 e explica o motivo.
    let cenario = Cenario::novo("offline");
    let saida = cenario.poph(&[
        "--org",
        &cenario.slug,
        "file",
        "list",
        "--agent",
        &cenario.agente,
    ]);
    assert_eq!(saida.status.code(), Some(1));
    let erro = String::from_utf8_lossy(&saida.stderr);
    assert!(erro.contains("--offline"), "{erro}");
}

#[test]
fn biblioteca_e_por_agente() {
    let cenario = Cenario::novo("isolamento");
    // Segundo agente na mesma org.
    {
        let store = Store::open_at(
            &cenario.pasta.join("dados").join("harness.db"),
            &cenario.pasta.join("dados").join("rag.db"),
        )
        .expect("abre o banco");
        let org = store.orgs().by_slug(&cenario.slug).expect("org criada").id;
        store
            .agents()
            .create(
                org,
                NewAgent {
                    name: "Caio".to_owned(),
                    slug: "caio".to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o segundo agente");
    }
    let caminho = cenario.escrever("logo.png", PNG);
    let criado = upload_json(&cenario, &caminho, &[]);
    let id = criado["arquivo"]["id"].as_str().expect("id").to_owned();

    // A biblioteca do outro agente não tem este arquivo.
    let lista = cenario.ok_json(&["file", "list", "--agent", "caio", "--json"]);
    assert_eq!(lista["total"], 0);
    let (codigo, _erro) = cenario.falhou(&["file", "show", &id, "--agent", "caio"]);
    assert_eq!(codigo, 1);
    let (codigo, _erro) = cenario.falhou(&["file", "delete", &id, "--agent", "caio"]);
    assert_eq!(codigo, 1);
    // E o do dono continua lá.
    let lista = cenario.ok_json(&["file", "list", "--agent", &cenario.agente, "--json"]);
    assert_eq!(lista["total"], 1);
}
