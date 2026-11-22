//! Testes de ponta a ponta de `poph backup` (HR-53): processo de verdade, banco e blobs de verdade.
//!
//! Cenário: uma org com agente, instruções, categoria, departamento, produto, item de base de
//! conhecimento, arquivo na biblioteca, integração com segredo cifrado e uma conversa com
//! mensagem, anexo, turno e item de saída. Exporta num arquivo único, importa numa instância
//! limpa e compara o estado (linha a linha). Também prova idempotência, ausência do segredo no
//! arquivo, isolamento da org vizinha e recusa de arquivo corrompido.
//!
//! A reindexação automática depende do modelo de embedding: sem ele instalado, o teste confere o
//! aviso e não a busca (como os testes de `poph kb`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use poph_files::ConfigArquivos;
use poph_store::backup::RetratoOrg;
use poph_store::{
    Direcao, FinalizacaoTurno, NovaConversa, NovaMensagem, NovaSaida, NovoAnexoCanal, Papel, Store,
};
use poph_types::{AgentId, OrgId};
use serde_json::Value;

/// Segredo de integração que **não** pode sair no arquivo (nem em claro, nem cifrado).
const SENTINELA_SEGREDO: &str = "SENTINELA-SEGREDO-hr53";
/// Chave de cifra do cenário (só para o segredo ser decifrável nos nomes dos campos).
const CHAVE_CIFRA: &str = "chave-de-teste-hr53";
/// Segredo de LLM/Jev que também não pode vazar.
const SENTINELA_LLM: &str = "SENTINELA-LLM-hr53";

/// PNG mínimo: a assinatura é o que a detecção por conteúdo usa.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

fn pasta_modelos_repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos"))
}

fn modelo_instalado() -> bool {
    let modelos = pasta_modelos_repo();
    modelos.join("model_int8.onnx").is_file() && modelos.join("tokenizer.json").is_file()
}

struct Cenario {
    pasta: PathBuf,
    slug: String,
}

impl Cenario {
    /// Cenário sem o modelo de embedding: os comandos rodam, e o índice fica pendente com aviso.
    fn novo(rotulo: &str) -> Self {
        Self::com_modelo(rotulo, false)
    }

    /// Cenário com o modelo de embedding, quando ele está instalado (busca e reindexação reais).
    fn com_modelo(rotulo: &str, com_modelo: bool) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-backup-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("cria a pasta de dados");
        if com_modelo && modelo_instalado() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(pasta_modelos_repo(), pasta.join("dados").join("modelos"))
                .expect("liga a pasta de modelos");
        }
        Self {
            pasta,
            slug: format!("org-{rotulo}"),
        }
    }

    fn dados(&self) -> PathBuf {
        self.pasta.join("dados")
    }

    fn poph(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_poph"))
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_DADOS_DIRETORIO", self.dados())
            .env("POPH_CHAVE_CIFRA", CHAVE_CIFRA)
            .env("ANTIGRAVITY_API_KEY", SENTINELA_LLM)
            .output()
            .expect("executa o poph")
    }

    fn ok_json(&self, args: &[&str]) -> Value {
        let saida = self.poph(args);
        assert!(
            saida.status.success(),
            "`poph {}` falhou: {}",
            args.join(" "),
            String::from_utf8_lossy(&saida.stderr)
        );
        let texto = String::from_utf8_lossy(&saida.stdout);
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

    fn abrir_store(&self) -> Store {
        Store::abrir_em(
            &self.dados().join("harness.db"),
            &self.dados().join("rag.db"),
        )
        .expect("abre o banco")
    }

    fn org_por_slug(&self, slug: &str) -> OrgId {
        self.abrir_store()
            .orgaos()
            .por_slug(slug)
            .expect("org do cenário")
            .id
    }
}

/// Semeia a org de origem com um recurso de cada tipo, uma integração com segredo e uma conversa.
fn semear_origem(cenario: &Cenario) {
    cenario.ok_json(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Org Origem",
        "--slug",
        &cenario.slug,
        "--json",
    ]);
    cenario.ok_json(&[
        "--offline",
        "org",
        "update",
        &cenario.slug,
        "--if-match",
        "1",
        "--config-json",
        "{\"retencao\":{\"dias\":7}}",
        "--json",
    ]);
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "agent",
        "create",
        "--name",
        "Bia",
        "--slug",
        "bia",
        "--json",
    ]);
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "department",
        "create",
        "--name",
        "Atendimento",
        "--description",
        "primeiro contato",
        "--json",
    ]);
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "category",
        "create",
        "--name",
        "Pragas",
        "--json",
    ]);
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "product",
        "create",
        "--name",
        "Dedetização de baratas",
        "--slug",
        "dedetizacao-de-baratas",
        "--category",
        "Pragas",
        "--price",
        "45000",
        "--alias",
        "baratas",
        "--json",
    ]);
    let grupo = cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "kb",
        "group",
        "create",
        "--name",
        "Atendimento",
        "--json",
    ]);
    let grupo_id = grupo["group"]["id"].as_str().expect("id do grupo");
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "kb",
        "item",
        "create",
        "--group",
        grupo_id,
        "--title",
        "Prazo de entrega",
        "--content",
        "O prazo é de 5 dias úteis.",
        "--json",
    ]);
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "kb",
        "link",
        grupo_id,
        "--agent",
        "bia",
        "--json",
    ]);

    let arquivo = cenario.pasta.join("logo.png");
    fs::write(&arquivo, PNG).expect("grava o PNG");
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "file",
        "upload",
        arquivo.to_str().expect("caminho"),
        "--agent",
        "bia",
        "--name",
        "Logo da empresa",
        "--description",
        "logo",
        "--json",
    ]);
    let prompt = cenario.pasta.join("prompt.md");
    fs::write(
        &prompt,
        "# Identidade\n\nVocê é a Bia, da Dedetizadora Exemplo.\n",
    )
    .expect("grava o prompt");
    cenario.ok_json(&[
        "--offline",
        "--org",
        &cenario.slug,
        "instructions",
        "create",
        "bia",
        "--markdown-file",
        prompt.to_str().expect("caminho"),
        "--json",
    ]);

    // O resto é semeado direto pelo repositório: integração com segredo cifrado e histórico.
    let store = cenario.abrir_store();
    let org = store.orgaos().por_slug(&cenario.slug).unwrap().id;
    let agente = store.agentes().por_slug(org, "bia").unwrap().id;
    let chave = poph_server::ChaveDeCifra::derivar(CHAVE_CIFRA).expect("deriva a chave");
    let mut segredos = std::collections::BTreeMap::new();
    segredos.insert("token".to_owned(), SENTINELA_SEGREDO.to_owned());
    let blob = chave.cifrar(&segredos).expect("cifra o segredo");
    store
        .integracao_criar_com_configuracao(
            org,
            agente,
            "whatsapp".to_owned(),
            "active".to_owned(),
            serde_json::json!({"instancia": "bia"}),
            Some(blob),
        )
        .expect("cria a integração");

    let conversa = store
        .conversas()
        .criar(
            org,
            NovaConversa {
                agent_id: agente,
                integration_id: None,
                canal: "test".to_owned(),
                peer_externo: None,
                departamento_alvo: None,
            },
        )
        .expect("cria a conversa");
    let conteudo_anexo = [PNG, b"-anexo-do-cliente"].concat();
    let anexo = poph_files::blob::gravar(
        &ConfigArquivos::padrao(cenario.dados().join("blobs")).pasta_blobs,
        org,
        &conteudo_anexo,
    )
    .expect("grava o blob do anexo");
    store
        .mensagens()
        .gravar_com_anexos(
            org,
            NovaMensagem {
                conversation_id: conversa.id,
                direcao: Direcao::Entrada,
                papel: Papel::Usuario,
                texto: "Qual o prazo? Segue o comprovante.".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
            &[NovoAnexoCanal {
                nome: "comprovante.png".to_owned(),
                mime: "image/png".to_owned(),
                tamanho_bytes: conteudo_anexo.len() as u64,
                sha256: anexo.sha256.clone(),
                blob_path: format!("{org}/{}", anexo.sha256),
                texto_extraido: None,
                status_extracao: "skipped",
            }],
        )
        .expect("grava a mensagem com anexo");
    let turno = store
        .turnos()
        .iniciar(org, conversa.id, Some(agente))
        .expect("abre o turno");
    store
        .turnos()
        .finalizar(
            org,
            turno.id,
            FinalizacaoTurno::concluido("{\"total_ms\":12}"),
        )
        .expect("fecha o turno");
    store
        .mensagens()
        .gravar(
            org,
            NovaMensagem {
                conversation_id: conversa.id,
                direcao: Direcao::Saida,
                papel: Papel::Assistente,
                texto: "O prazo é de 5 dias úteis.".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: Some(turno.id),
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
        )
        .expect("grava a resposta");
    store
        .caixa_saida()
        .enfileirar(
            org,
            NovaSaida {
                conversation_id: conversa.id,
                turn_id: turno.id,
                seq: 1,
                tipo: "text".to_owned(),
                conteudo_json: "{\"text\":\"O prazo é de 5 dias úteis.\"}".to_owned(),
            },
        )
        .expect("enfileira a saída");
}

/// Compara dois retratos linha a linha; as integrações ignoram a marca de segredo.
fn comparar_retratos(origem: &RetratoOrg, destino: &RetratoOrg) {
    assert_eq!(origem.org, destino.org, "linha da org");
    assert_eq!(
        origem.tabelas.keys().collect::<Vec<_>>(),
        destino.tabelas.keys().collect::<Vec<_>>(),
        "tabelas do retrato"
    );
    for (tabela, linhas) in &origem.tabelas {
        let esperadas: Vec<Value> = linhas
            .iter()
            .map(|linha| normalizar(tabela, linha))
            .collect();
        let obtidas: Vec<Value> = destino.tabelas[tabela]
            .iter()
            .map(|linha| normalizar(tabela, linha))
            .collect();
        assert_eq!(esperadas, obtidas, "conteúdo da tabela `{tabela}`");
    }
}

fn normalizar(tabela: &str, linha: &Value) -> Value {
    if tabela != "integrations" {
        return linha.clone();
    }
    let mut linha = linha.clone();
    if let Some(objeto) = linha.as_object_mut() {
        for campo in ["tem_segredo", "segredos_campos"] {
            objeto.remove(campo);
        }
    }
    linha
}

fn ler_retrato(cenario: &Cenario, org: OrgId) -> RetratoOrg {
    cenario
        .abrir_store()
        .retrato_da_org(org, true)
        .expect("retrato")
}

fn comparar_blobs(origem: &Cenario, destino: &Cenario, org_origem: OrgId, org_destino: OrgId) {
    let retrato = ler_retrato(origem, org_origem);
    let mut hashes = Vec::new();
    for tabela in ["file_library", "attachments"] {
        for linha in retrato.tabelas.get(tabela).into_iter().flatten() {
            if let Some(sha) = linha.get("sha256").and_then(Value::as_str) {
                hashes.push(sha.to_owned());
            }
        }
    }
    assert!(!hashes.is_empty(), "o cenário devia ter blobs");
    for sha in hashes {
        let original = fs::read(
            origem
                .dados()
                .join("blobs")
                .join(org_origem.to_string())
                .join(&sha),
        )
        .expect("lê o blob original");
        let copia = fs::read(
            destino
                .dados()
                .join("blobs")
                .join(org_destino.to_string())
                .join(&sha),
        )
        .expect("lê o blob importado");
        assert_eq!(original, copia, "conteúdo do blob {sha}");
        assert_eq!(
            poph_files::blob::sha256_hex(&copia),
            sha,
            "sha256 do blob importado"
        );
    }
}

fn agente_por_slug(store: &Store, org: OrgId, slug: &str) -> AgentId {
    store.agentes().por_slug(org, slug).expect("agente").id
}

#[test]
fn exportar_e_importar_reconstroi_a_org_num_arquivo_so() {
    let origem = Cenario::novo("origem");
    semear_origem(&origem);
    let org_origem = origem.org_por_slug(&origem.slug);
    let arquivo = origem.pasta.join("backup.json");

    let relatorio = origem.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origem.slug,
        "--out",
        arquivo.to_str().expect("caminho"),
        "--com-conversas",
        "--json",
    ]);
    assert_eq!(relatorio["versao_relatorio"], 1);
    assert_eq!(relatorio["com_conversas"], true);
    assert!(
        relatorio["blobs"].as_u64().unwrap() >= 2,
        "biblioteca + anexo"
    );

    // O arquivo: um só, com formato declarado, sem segredo e sem caminho de blob.
    let texto = fs::read_to_string(&arquivo).expect("lê o backup");
    assert!(
        !texto.contains(SENTINELA_SEGREDO),
        "segredo em claro no arquivo"
    );
    assert!(!texto.contains(SENTINELA_LLM), "segredo do LLM no arquivo");
    assert!(!texto.contains("blob_path"), "caminho de blob no arquivo");
    assert!(!texto.contains("secrets_enc"), "segredo cifrado no arquivo");
    let documento: Value = serde_json::from_str(&texto).expect("JSON do backup");
    assert_eq!(documento["formato"], "poph-backup-org");
    assert_eq!(documento["versao_formato"], 1);
    assert_eq!(documento["org_origem"]["slug"], origem.slug);
    let integracao = &documento["tabelas"]["integrations"][0];
    assert_eq!(
        integracao["tem_segredo"], true,
        "a marca de que havia segredo"
    );
    assert_eq!(
        integracao["segredos_campos"],
        serde_json::json!(["token"]),
        "o nome do campo, sem o valor"
    );

    // Instância limpa, com o modelo de embedding: importa, reindexa e compara o estado.
    let destino = Cenario::com_modelo("destino", true);
    let importacao = destino.ok_json(&[
        "--offline",
        "backup",
        "import",
        arquivo.to_str().expect("caminho"),
        "--json",
    ]);
    assert_eq!(importacao["org"]["criada"], true);
    assert_eq!(importacao["segredos_a_redigitar"], 1);
    assert_eq!(importacao["org"]["id"], org_origem.to_string());
    comparar_retratos(
        &ler_retrato(&origem, org_origem),
        &ler_retrato(&destino, org_origem),
    );
    comparar_blobs(&origem, &destino, org_origem, org_origem);

    // O segredo não veio: a integração existe, sem segredo, com a configuração do operador.
    {
        let store = destino.abrir_store();
        let agente = agente_por_slug(&store, org_origem, "bia");
        let integracoes = store
            .integracoes_listar(org_origem, agente, poph_store::Pagina::default())
            .expect("lista integrações");
        assert_eq!(integracoes.len(), 1);
        assert!(
            !integracoes[0].tem_segredo,
            "o segredo não pode ser importado"
        );
        assert_eq!(integracoes[0].configuracao["instancia"], "bia");
    }

    // Reindexação: com modelo, a base já responde sem passo manual; sem modelo, vem o aviso.
    if modelo_instalado() {
        assert!(
            importacao["aviso"].is_null(),
            "com o modelo instalado não deveria haver aviso: {importacao}"
        );
        let busca = destino.ok_json(&[
            "--offline",
            "--org",
            &origem.slug,
            "kb",
            "search",
            "prazo de entrega",
            "--agent",
            "bia",
            "--json",
        ]);
        assert!(
            busca["chunks"]
                .as_array()
                .is_some_and(|lista| !lista.is_empty()),
            "a base importada devia ser buscável: {busca}"
        );
    } else {
        assert!(
            importacao["aviso"].is_string(),
            "sem modelo, o import termina com aviso: {importacao}"
        );
    }

    // Sobrescrita é recusada por padrão (HR-48e-3): importar de novo sem `--sobrescrever` para
    // antes de gravar, dizendo quantas linhas e em quais tabelas.
    let recusa = destino.falha(&[
        "--offline",
        "backup",
        "import",
        arquivo.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(recusa.contains("--sobrescrever"), "{recusa}");
    assert!(recusa.contains("a importação atualizaria"), "{recusa}");
    assert!(recusa.contains("agents "), "{recusa}");
    assert!(
        recusa.contains(&format!(
            "{} linha(s)",
            importacao["total"].as_u64().expect("total") + 1
        )),
        "a conta inclui a linha da org: {recusa}"
    );
    let estado_depois_da_recusa = ler_retrato(&destino, org_origem);
    comparar_retratos(&ler_retrato(&origem, org_origem), &estado_depois_da_recusa);

    // Com --sobrescrever: importar de novo não duplica nada e atualiza as mesmas linhas.
    let segunda = destino.ok_json(&[
        "--offline",
        "backup",
        "import",
        arquivo.to_str().expect("caminho"),
        "--sobrescrever",
        "--json",
    ]);
    assert_eq!(segunda["org"]["criada"], false);
    assert_eq!(segunda["total"], importacao["total"]);
    for (tabela, gravado) in segunda["tabelas"].as_object().expect("tabelas") {
        assert_eq!(
            gravado["inseridas"], 0,
            "`{tabela}` duplicou na segunda importação"
        );
    }
    comparar_retratos(
        &ler_retrato(&origem, org_origem),
        &ler_retrato(&destino, org_origem),
    );
    let agentes = destino
        .abrir_store()
        .agentes()
        .listar(org_origem, poph_store::Pagina::default());
    assert_eq!(agentes.expect("lista").len(), 1);
}

#[test]
fn importar_numa_org_existente_nao_mexe_na_org_vizinha() {
    let origem = Cenario::novo("origem-org");
    semear_origem(&origem);
    let arquivo = origem.pasta.join("backup.json");
    origem.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origem.slug,
        "--out",
        arquivo.to_str().expect("caminho"),
        "--json",
    ]);

    let destino = Cenario::novo("destino-org");
    destino.ok_json(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Vizinha",
        "--slug",
        "vizinha",
        "--json",
    ]);
    destino.ok_json(&[
        "--offline",
        "--org",
        "vizinha",
        "agent",
        "create",
        "--name",
        "Vizinho",
        "--slug",
        "vizinho",
        "--json",
    ]);
    let vizinha = destino.org_por_slug("vizinha");
    let antes = ler_retrato(&destino, vizinha);

    let importacao = destino.ok_json(&[
        "--offline",
        "backup",
        "import",
        arquivo.to_str().expect("caminho"),
        "--org",
        "vizinha",
        "--json",
    ]);
    assert_eq!(importacao["org"]["id"], vizinha.to_string());
    assert_eq!(importacao["org"]["criada"], false);

    // A org de origem do arquivo não nasce nesta instância.
    let erro = destino.falha(&["--offline", "org", "show", &origem.slug, "--json"]);
    assert!(erro.contains("não existe org"), "{erro}");

    // A vizinha ganhou os recursos importados, com os ids do arquivo, sem perder os próprios.
    let depois = ler_retrato(&destino, vizinha);
    assert_eq!(
        depois.tabelas["agents"].len(),
        antes.tabelas["agents"].len() + 1,
        "um agente importado além do próprio"
    );
    let store = destino.abrir_store();
    assert!(store.agentes().por_slug(vizinha, "vizinho").is_ok());
    assert!(store.agentes().por_slug(vizinha, "bia").is_ok());
    assert_eq!(
        store
            .produtos()
            .listar(vizinha, poph_store::Pagina::default())
            .expect("produtos")
            .len(),
        1
    );
    // O que era da vizinha continua idêntico (agente próprio intacto).
    let proprio = antes.tabelas["agents"]
        .iter()
        .find(|linha| linha["slug"] == "vizinho")
        .expect("agente próprio")
        .clone();
    let mesmo = depois.tabelas["agents"]
        .iter()
        .find(|linha| linha["slug"] == "vizinho")
        .expect("agente próprio depois");
    assert_eq!(&proprio, mesmo);
}

#[test]
fn arquivo_corrompido_ou_de_outra_versao_e_recusado_sem_gravar() {
    let origem = Cenario::novo("corrompido");
    semear_origem(&origem);
    let arquivo = origem.pasta.join("backup.json");
    origem.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origem.slug,
        "--out",
        arquivo.to_str().expect("caminho"),
        "--json",
    ]);
    let texto = fs::read_to_string(&arquivo).expect("lê");

    let destino = Cenario::novo("corrompido-destino");
    // Truncado: não é JSON.
    let truncado = destino.pasta.join("truncado.json");
    fs::write(&truncado, &texto[..texto.len() / 2]).expect("grava o truncado");
    let erro = destino.falha(&[
        "--offline",
        "backup",
        "import",
        truncado.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(erro.contains("JSON inválido"), "{erro}");

    // Versão de formato desconhecida.
    let mut documento: Value = serde_json::from_str(&texto).expect("JSON");
    documento["versao_formato"] = serde_json::json!(99);
    let futuro = destino.pasta.join("futuro.json");
    fs::write(&futuro, documento.to_string()).expect("grava");
    let erro = destino.falha(&[
        "--offline",
        "backup",
        "import",
        futuro.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(erro.contains("versão de formato"), "{erro}");

    // Nada foi gravado: a instância continua sem org nenhuma.
    assert!(
        destino
            .abrir_store()
            .orgaos()
            .listar(poph_store::Pagina::default())
            .expect("orgs")
            .is_empty()
    );
}

#[test]
fn exportar_recusa_sobrescrever_sem_o_flag() {
    let origem = Cenario::novo("sobrescrever");
    semear_origem(&origem);
    let arquivo = origem.pasta.join("backup.json");
    fs::write(&arquivo, b"nao-apague-por-engano").expect("grava o arquivo existente");
    let erro = origem.falha(&[
        "--offline",
        "backup",
        "export",
        &origem.slug,
        "--out",
        arquivo.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(erro.contains("--sobrescrever"), "{erro}");
    assert_eq!(
        fs::read_to_string(&arquivo).expect("lê"),
        "nao-apague-por-engano"
    );
    // Com o flag, sobrescreve.
    origem.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origem.slug,
        "--out",
        arquivo.to_str().expect("caminho"),
        "--sobrescrever",
        "--json",
    ]);
    assert!(
        fs::read_to_string(&arquivo)
            .expect("lê")
            .contains("poph-backup-org")
    );
}

#[test]
fn exportar_recusa_blob_ausente_no_disco() {
    let origem = Cenario::novo("sem-blob");
    semear_origem(&origem);
    let org = origem.org_por_slug(&origem.slug);
    let retrato = origem
        .abrir_store()
        .retrato_da_org(org, false)
        .expect("retrato");
    let sha = retrato.tabelas["file_library"][0]["sha256"]
        .as_str()
        .expect("sha do arquivo da biblioteca")
        .to_owned();
    fs::remove_file(
        origem
            .dados()
            .join("blobs")
            .join(org.to_string())
            .join(&sha),
    )
    .expect("apaga o blob");

    let arquivo = origem.pasta.join("nao-deve-existir.json");
    let erro = origem.falha(&[
        "--offline",
        "backup",
        "export",
        &origem.slug,
        "--out",
        arquivo.to_str().expect("caminho"),
        "--json",
    ]);
    assert!(erro.contains("Logo da empresa"), "{erro}");
    assert!(erro.contains(&sha), "{erro}");
    assert!(!arquivo.exists(), "não pode deixar arquivo pela metade");
}

/// O caminho de blob é recalculado para a org de destino (a pasta muda com a org).
#[test]
fn blob_importado_fica_na_pasta_da_org_de_destino() {
    let origem = Cenario::novo("pasta");
    semear_origem(&origem);
    let org_origem = origem.org_por_slug(&origem.slug);
    let arquivo = origem.pasta.join("backup.json");
    origem.ok_json(&[
        "--offline",
        "backup",
        "export",
        &origem.slug,
        "--out",
        arquivo.to_str().expect("caminho"),
        "--json",
    ]);
    let destino = Cenario::novo("pasta-destino");
    destino.ok_json(&[
        "--offline",
        "org",
        "create",
        "--name",
        "Alvo",
        "--slug",
        "alvo",
        "--json",
    ]);
    let alvo = destino.org_por_slug("alvo");
    destino.ok_json(&[
        "--offline",
        "backup",
        "import",
        arquivo.to_str().expect("caminho"),
        "--org",
        "alvo",
        "--json",
    ]);
    let retrato = ler_retrato(&destino, alvo);
    for tabela in ["file_library", "attachments"] {
        for linha in retrato.tabelas.get(tabela).into_iter().flatten() {
            let sha = linha["sha256"].as_str().expect("sha");
            assert!(
                destino
                    .dados()
                    .join("blobs")
                    .join(alvo.to_string())
                    .join(sha)
                    .is_file(),
                "o blob {sha} devia estar na pasta da org de destino"
            );
        }
    }
    // E a pasta da org de origem não existe nesta instância.
    assert!(!Path::new(&destino.dados().join("blobs").join(org_origem.to_string())).exists());
}
