//! Busca de produtos (HR-18): medição do acerto no top-3 da híbrida contra a só vetorial, e o
//! comportamento do índice (preço fora do hash, produto inativo fora).
//!
//! A fixtura `fixtures/produtos_rotulados.json` tem 12 produtos de uma empresa fictícia de controle
//! de pragas e 20 consultas escritas como cliente escreveria — metade por apelido, metade com um
//! erro de digitação simples.
//!
//! A medição usa o **modelo real** (é o que mede a busca de verdade); sem o modelo instalado o teste
//! é pulado, como os outros do crate. Os testes de índice usam o embedder falso e rodam sempre.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use poph_rag::{
    ConfigBusca, ConfigIndice, ConfigModelo, Embedder, EmbedderOrt, FakeEmbedder, Indice,
    ProdutoRag,
};
use poph_types::{AgentId, OrgId, ProductId};
use serde_json::Value;

fn pasta_modelos() -> PathBuf {
    match std::env::var_os("POPH_MODELOS_TESTE") {
        Some(caminho) => PathBuf::from(caminho),
        None => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/modelos")),
    }
}

fn modelo_disponivel(nome: &str) -> Option<PathBuf> {
    let pasta = pasta_modelos();
    if pasta.join("model_int8.onnx").is_file() && pasta.join("tokenizer.json").is_file() {
        return Some(pasta);
    }
    eprintln!("PULADO ({nome}): modelo não instalado; rode `poph model install` antes");
    None
}

struct Pasta(PathBuf);

impl Pasta {
    fn nova(nome: &str) -> Self {
        let agora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let caminho = std::env::temp_dir().join(format!(
            "poph-produtos-{nome}-{}-{agora}",
            std::process::id()
        ));
        fs::create_dir_all(&caminho).expect("cria a pasta temporária");
        Self(caminho)
    }

    fn rag_db(&self) -> PathBuf {
        self.0.join("rag.db")
    }
}

impl Drop for Pasta {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Um produto da fixtura.
struct Rotulado {
    id: String,
    nome: String,
    aliases: Vec<String>,
    descricao: String,
    tags: Vec<String>,
}

fn fixtura() -> (Vec<Rotulado>, Vec<(String, Vec<String>)>) {
    let bruto: Value = serde_json::from_str(include_str!("fixtures/produtos_rotulados.json"))
        .expect("fixtura é JSON válido");
    let produtos = bruto["produtos"]
        .as_array()
        .expect("produtos")
        .iter()
        .map(|produto| Rotulado {
            id: produto["id"].as_str().expect("id").to_owned(),
            nome: produto["nome"].as_str().expect("nome").to_owned(),
            aliases: lista(&produto["aliases"]),
            descricao: produto["descricao"].as_str().expect("descricao").to_owned(),
            tags: lista(&produto["tags"]),
        })
        .collect();
    let consultas = bruto["consultas"]
        .as_array()
        .expect("consultas")
        .iter()
        .map(|consulta| {
            (
                consulta["consulta"].as_str().expect("consulta").to_owned(),
                lista(&consulta["esperado"]),
            )
        })
        .collect();
    (produtos, consultas)
}

fn lista(valor: &Value) -> Vec<String> {
    valor
        .as_array()
        .expect("lista")
        .iter()
        .map(|item| item.as_str().expect("texto").to_owned())
        .collect()
}

/// Texto que vai ao índice (o mesmo que o `poph-store` monta: nome, aliases, descrição e tags).
fn texto_do_produto(produto: &Rotulado) -> String {
    let mut texto = produto.nome.clone();
    if !produto.aliases.is_empty() {
        texto.push_str(&format!(
            "\nTambém chamado de: {}",
            produto.aliases.join(", ")
        ));
    }
    if !produto.descricao.trim().is_empty() {
        texto.push_str(&format!("\nDescrição: {}", produto.descricao.trim()));
    }
    if !produto.tags.is_empty() {
        texto.push_str(&format!("\nTags: {}", produto.tags.join(", ")));
    }
    texto
}

#[tokio::test]
async fn hibrida_de_produtos_acerta_top3_pelo_menos_tanto_quanto_so_vetor() {
    let Some(pasta_modelos) = modelo_disponivel("produtos_hibrida") else {
        return;
    };
    let (produtos, consultas) = fixtura();
    assert_eq!(produtos.len(), 12, "a fixtura tem 12 produtos");
    assert_eq!(consultas.len(), 20, "a fixtura tem 20 consultas");

    let pasta = Pasta::nova("top3");
    let embedder: Arc<dyn Embedder> = Arc::new(
        EmbedderOrt::abrir(
            &ConfigModelo {
                diretorio: pasta_modelos,
                ..ConfigModelo::default()
            },
            None,
        )
        .expect("abre o modelo"),
    );
    let (indice, _) =
        Indice::abrir(&pasta.rag_db(), embedder, ConfigIndice::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();

    let mut ids: Vec<(String, ProductId)> = Vec::new();
    let mut para_indice = Vec::new();
    for produto in &produtos {
        let id = ProductId::generate();
        ids.push((produto.id.clone(), id));
        para_indice.push(ProdutoRag {
            id,
            nome: produto.nome.clone(),
            texto: texto_do_produto(produto),
            versao: 1,
        });
    }
    indice
        .indexar_produtos(org, agente, &para_indice)
        .await
        .expect("indexa a fixtura");

    let config = ConfigBusca {
        top_k: 3,
        corte: 0.0,
        profundidade: 20,
    };
    let mut vetor_acertos = 0;
    let mut hibrida_acertos = 0;
    let mut vetor_falhas = Vec::new();
    let mut hibrida_falhas = Vec::new();
    for (consulta, esperados) in &consultas {
        let esperado: Vec<ProductId> = ids
            .iter()
            .filter(|(rotulo, _)| esperados.contains(rotulo))
            .map(|(_, id)| *id)
            .collect();

        let vetor = indice
            .buscar_produtos_vetor(org, agente, consulta, 3)
            .await
            .expect("busca vetorial");
        if vetor
            .iter()
            .any(|achado| esperado.contains(&achado.produto))
        {
            vetor_acertos += 1;
        } else {
            vetor_falhas.push(consulta.clone());
        }

        let hibrida = indice
            .buscar_produtos(org, agente, consulta, &config)
            .await
            .expect("busca híbrida");
        if hibrida
            .iter()
            .any(|achado| esperado.contains(&achado.produto))
        {
            hibrida_acertos += 1;
        } else {
            hibrida_falhas.push(consulta.clone());
        }
    }

    let total = consultas.len();
    eprintln!(
        "acerto top-3 (produtos): só vetor {vetor_acertos}/{total} ({:.1}%); híbrida \
         {hibrida_acertos}/{total} ({:.1}%)",
        vetor_acertos as f64 * 100.0 / total as f64,
        hibrida_acertos as f64 * 100.0 / total as f64,
    );
    if !vetor_falhas.is_empty() {
        eprintln!("falhas do vetor: {vetor_falhas:?}");
    }
    if !hibrida_falhas.is_empty() {
        eprintln!("falhas da híbrida: {hibrida_falhas:?}");
    }
    assert!(
        hibrida_acertos >= vetor_acertos,
        "a busca híbrida de produtos precisa acertar pelo menos tanto quanto a vetorial \
         (híbrida {hibrida_acertos}, vetor {vetor_acertos})"
    );
}

#[tokio::test]
async fn mudar_preco_nao_muda_o_hash_do_indice() {
    let pasta = Pasta::nova("preco");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let (indice, _) =
        Indice::abrir(&pasta.rag_db(), embedder, ConfigIndice::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();
    let produto = ProdutoRag {
        id: ProductId::generate(),
        nome: "Dedetização de baratas".to_owned(),
        texto: "Dedetização de baratas\nTambém chamado de: baratas, barata".to_owned(),
        versao: 1,
    };
    indice
        .indexar_produtos(org, agente, std::slice::from_ref(&produto))
        .await
        .expect("indexa");
    let antes = indice
        .produtos_indexados(org, agente)
        .expect("lê os hashes");

    // Só o preço mudou no banco: a lista que vai ao índice é a mesma.
    let depois = indice
        .indexar_produtos(org, agente, std::slice::from_ref(&produto))
        .await
        .expect("reindexa");
    assert_eq!(depois.itens_alterados, 0);
    assert_eq!(depois.embeddings_gerados, 0, "nada foi reembedado");
    assert_eq!(
        antes,
        indice.produtos_indexados(org, agente).expect("hashes")
    );
}

#[tokio::test]
async fn produto_que_sai_da_lista_sai_do_indice() {
    let pasta = Pasta::nova("sai");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new());
    let (indice, _) =
        Indice::abrir(&pasta.rag_db(), embedder, ConfigIndice::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();
    let baratas = ProdutoRag {
        id: ProductId::generate(),
        nome: "Dedetização de baratas".to_owned(),
        texto: "Dedetização de baratas\nTambém chamado de: baratas".to_owned(),
        versao: 1,
    };
    let cupins = ProdutoRag {
        id: ProductId::generate(),
        nome: "Dedetização de cupins".to_owned(),
        texto: "Dedetização de cupins\nTambém chamado de: cupins".to_owned(),
        versao: 1,
    };
    indice
        .indexar_produtos(org, agente, &[baratas.clone(), cupins.clone()])
        .await
        .expect("indexa");
    let config = ConfigBusca {
        top_k: 5,
        corte: 0.0,
        profundidade: 20,
    };
    let achados = indice
        .buscar_produtos(org, agente, "cupins", &config)
        .await
        .expect("busca");
    assert!(achados.iter().any(|achado| achado.produto == cupins.id));

    // A lista nova não tem mais cupins (produto ficou inativo no harness.db).
    let relatorio = indice
        .indexar_produtos(org, agente, std::slice::from_ref(&baratas))
        .await
        .expect("reindexa");
    assert_eq!(relatorio.itens_removidos, 1);
    let achados = indice
        .buscar_produtos(org, agente, "cupins", &config)
        .await
        .expect("busca");
    assert!(
        achados.iter().all(|achado| achado.produto != cupins.id),
        "produto fora da lista não volta da busca"
    );
}
