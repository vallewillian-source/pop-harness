//! Busca híbrida (HR-17): medição do acerto no top-3 contra a busca só vetorial, e o corte.
//!
//! A fixtura `fixtures/perguntas_rotuladas.json` tem 24 itens de uma empresa fictícia de controle
//! de pragas e 30 perguntas escritas como cliente escreveria, cada uma rotulada com o item que
//! deveria aparecer no top-3.
//!
//! A medição usa o **modelo real** (é o que mede a busca de verdade); sem o modelo instalado o
//! teste é pulado, como os outros do crate. O teste de corte usa o embedder falso, para rodar
//! sempre.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use poph_rag::{
    ConfigBusca, ConfigIndice, ConfigModelo, Embedder, EmbedderFake, EmbedderOrt, Indice, ItemRag,
};
use poph_types::{AgentId, KnowledgeItemId, OrgId};
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
            "poph-hibrida-{nome}-{}-{agora}",
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

/// Um item da fixtura.
struct Rotulado {
    id: String,
    titulo: String,
    texto: String,
}

fn fixtura() -> (Vec<Rotulado>, Vec<(String, Vec<String>)>) {
    let bruto: Value = serde_json::from_str(include_str!("fixtures/perguntas_rotuladas.json"))
        .expect("fixtura é JSON válido");
    let itens = bruto["itens"]
        .as_array()
        .expect("itens")
        .iter()
        .map(|item| Rotulado {
            id: item["id"].as_str().expect("id").to_owned(),
            titulo: item["titulo"].as_str().expect("titulo").to_owned(),
            texto: item["texto"].as_str().expect("texto").to_owned(),
        })
        .collect();
    let perguntas = bruto["perguntas"]
        .as_array()
        .expect("perguntas")
        .iter()
        .map(|pergunta| {
            (
                pergunta["pergunta"].as_str().expect("pergunta").to_owned(),
                pergunta["esperado"]
                    .as_array()
                    .expect("esperado")
                    .iter()
                    .map(|id| id.as_str().expect("id esperado").to_owned())
                    .collect(),
            )
        })
        .collect();
    (itens, perguntas)
}

/// Id de item para o teste. Não precisa ser estável entre execuções: o rótulo da fixtura é ligado
/// ao id no início do teste.
fn id_do_item(_texto: &str) -> KnowledgeItemId {
    KnowledgeItemId::generate()
}

#[tokio::test]
async fn hibrida_acerta_top3_pelo_menos_tanto_quanto_so_vetor() {
    let Some(pasta_modelos) = modelo_disponivel("hibrida_acerta_top3") else {
        return;
    };
    let (itens, perguntas) = fixtura();
    assert_eq!(itens.len(), 24);
    assert_eq!(perguntas.len(), 30);

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

    let mut ids = Vec::new();
    let mut para_indice = Vec::new();
    for item in &itens {
        let id = id_do_item(&item.id);
        ids.push((item.id.clone(), id));
        para_indice.push(ItemRag {
            id,
            titulo: item.titulo.clone(),
            texto: item.texto.clone(),
            versao: 1,
        });
    }
    indice
        .indexar_agente(org, agente, &para_indice)
        .await
        .expect("indexa a fixtura");

    let mut vetor_acertos = 0;
    let mut hibrida_acertos = 0;
    let mut vetor_falhas = Vec::new();
    let mut hibrida_falhas = Vec::new();
    for (pergunta, esperados) in &perguntas {
        let esperado: Vec<KnowledgeItemId> = ids
            .iter()
            .filter(|(rotulo, _)| esperados.contains(rotulo))
            .map(|(_, id)| *id)
            .collect();

        let vetor = indice
            .buscar_vetor(org, agente, pergunta, 3)
            .await
            .expect("busca vetorial");
        let acertou_vetor = vetor
            .iter()
            .any(|achado| esperado.contains(&achado.trecho.item));
        if acertou_vetor {
            vetor_acertos += 1;
        } else {
            vetor_falhas.push(pergunta.clone());
        }

        let hibrida = indice
            .buscar_hibrida(
                org,
                agente,
                pergunta,
                &ConfigBusca {
                    top_k: 3,
                    corte: 0.0,
                    profundidade: 20,
                },
            )
            .await
            .expect("busca híbrida");
        let acertou_hibrida = hibrida
            .iter()
            .any(|achado| esperado.contains(&achado.trecho.item));
        if acertou_hibrida {
            hibrida_acertos += 1;
        } else {
            hibrida_falhas.push(pergunta.clone());
        }
    }

    let total = perguntas.len();
    eprintln!(
        "acerto top-3: só vetor {vetor_acertos}/{total} ({:.1}%); híbrida {hibrida_acertos}/{total} ({:.1}%)",
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
        "a busca híbrida precisa acertar pelo menos tanto quanto a vetorial \
         (híbrida {hibrida_acertos}, vetor {vetor_acertos})"
    );
}

#[tokio::test]
async fn corte_de_relevancia_barra_trecho_sem_relacao() {
    let pasta = Pasta::nova("corte");
    let embedder: Arc<dyn Embedder> = Arc::new(EmbedderFake::new());
    let (indice, _) =
        Indice::abrir(&pasta.rag_db(), embedder, ConfigIndice::default()).expect("abre o índice");
    let org = OrgId::generate();
    let agente = AgentId::generate();
    let item = id_do_item("prazo");
    indice
        .indexar_agente(
            org,
            agente,
            &[ItemRag {
                id: item,
                titulo: "Prazo de agendamento".to_owned(),
                texto: "O prazo para agendar uma dedetização é de até 48 horas úteis.".to_owned(),
                versao: 1,
            }],
        )
        .await
        .expect("indexa");

    let sem_corte = indice
        .buscar_hibrida(
            org,
            agente,
            "qual o preço do sapato de couro",
            &ConfigBusca {
                top_k: 3,
                corte: 0.0,
                profundidade: 20,
            },
        )
        .await
        .expect("busca sem corte");
    assert!(!sem_corte.is_empty(), "sem corte, o top-3 volta");

    let com_corte = indice
        .buscar_hibrida(
            org,
            agente,
            "qual o preço do sapato de couro",
            &ConfigBusca {
                top_k: 3,
                corte: 0.6,
                profundidade: 20,
            },
        )
        .await
        .expect("busca com corte");
    assert!(
        com_corte.is_empty(),
        "com corte 0,6 o trecho sem relação não pode voltar: {com_corte:?}"
    );
}

#[tokio::test]
async fn hibrida_nao_cruza_org() {
    let pasta = Pasta::nova("orgs");
    let embedder: Arc<dyn Embedder> = Arc::new(EmbedderFake::new());
    let (indice, _) =
        Indice::abrir(&pasta.rag_db(), embedder, ConfigIndice::default()).expect("abre o índice");
    let org_a = OrgId::generate();
    let org_b = OrgId::generate();
    let agente = AgentId::generate();
    let item = id_do_item("segredo");
    indice
        .indexar_agente(
            org_a,
            agente,
            &[ItemRag {
                id: item,
                titulo: "Código interno".to_owned(),
                texto: "O código interno do cliente é azul-turquesa.".to_owned(),
                versao: 1,
            }],
        )
        .await
        .expect("indexa");
    let na_outra = indice
        .buscar_hibrida(
            org_b,
            agente,
            "código interno azul turquesa",
            &ConfigBusca::default(),
        )
        .await
        .expect("busca na outra org");
    assert!(na_outra.is_empty(), "trecho de outra org não pode voltar");
}
