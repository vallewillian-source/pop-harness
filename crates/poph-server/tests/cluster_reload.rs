//! Recarga do mapa de cluster (HR-50a), de ponta a ponta: servidor de verdade, arquivo de verdade.
//!
//! O que se prova:
//! - `POST /admin/v1/cluster/reload` com token global relê o `cluster.toml` do processo vivo e
//!   devolve impressão digital antes/depois, resumo dos dois mapas e o que mudou;
//! - editar o arquivo muda o mapa (org que troca de instância aparece em `orgs_que_trocaram`);
//! - arquivo inválido responde `400 cluster_invalido` e **mantém o mapa antigo** (a recarga
//!   seguinte, com o arquivo bom, confirma que o mapa em uso era o anterior);
//! - sem token → `401`; token de org → `403` (a recarga exige token global);
//! - arquivo ausente volta para o modo instância única;
//! - o segredo compartilhado nunca aparece em resposta (nem o fingerprint o carrega).

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_server::cluster::{ClusterVivo, carregar};
use poph_store::Store;
use poph_types::HarnessConfig;
use poph_types::Secret;
use poph_types::config::GlobalTokenConfig;
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr50a";
const SEGREDO: &str = "SENTINELA-SEGREDO-CLUSTER-hr50a";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

// Desde a HR-50b o mapa é aplicado de verdade: org fora do `[orgs]` vai para
// `default_instance`. Este cenário cria uma org em tempo de execução (para ter um token de org),
// então a padrão é a instância local — senão o pedido seria encaminhado para `b`, que não existe
// neste teste.
const MAPA_ORIGINAL: &str = r#"
instance_id = "a"
default_instance = "a"

[[instances]]
id = "a"
url = "http://127.0.0.1:7481"

[[instances]]
id = "b"
url = "http://127.0.0.1:7482"

[orgs]
org_01 = "a"
org_02 = "b"
"#;

/// Mapa com `org_01` movida para a instância b.
const MAPA_TROCADO: &str = r#"
instance_id = "a"
default_instance = "a"

[[instances]]
id = "a"
url = "http://127.0.0.1:7481"

[[instances]]
id = "b"
url = "http://127.0.0.1:7482"

[orgs]
org_01 = "b"
org_02 = "b"
"#;

/// `org_01` aponta para uma instância que não existe: tem de ser recusado sem trocar o mapa.
const MAPA_INVALIDO: &str = r#"
instance_id = "a"
default_instance = "b"

[[instances]]
id = "a"
url = "http://127.0.0.1:7481"

[[instances]]
id = "b"
url = "http://127.0.0.1:7482"

[orgs]
org_01 = "c"
"#;

struct Resposta {
    status: StatusCode,
    corpo: Value,
    cru: String,
}

struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    caminho_cluster: PathBuf,
    desligar: Option<oneshot::Sender<()>>,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-cluster-reload-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
        let caminho_cluster = pasta.join("cluster.toml");
        fs::write(&caminho_cluster, MAPA_ORIGINAL).expect("cluster.toml");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let segredo = Secret::new(SEGREDO.to_owned());
        let mapa = carregar(&caminho_cluster, Some(&segredo)).expect("mapa inicial válido");
        let estado = Estado::novo(Arc::clone(&store), config)
            .expect("estado do servidor")
            .com_cluster(ClusterVivo {
                caminho: caminho_cluster.clone(),
                config: mapa,
                segredo: Some(segredo),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, estado, async {
                let _ = sinal.await;
            })
            .await;
        });
        let cenario = Self {
            base: format!("http://{endereco}"),
            cliente: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("cliente HTTP"),
            pasta,
            caminho_cluster,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
    }

    async fn esperar_subir(&self) {
        for _ in 0..500 {
            if let Ok(r) = self
                .cliente
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                && r.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("servidor não subiu");
    }

    fn escrever_mapa(&self, texto: &str) {
        fs::write(&self.caminho_cluster, texto).expect("escreve cluster.toml");
    }

    async fn recarregar(&self, token: &str) -> Resposta {
        self.pedir(Method::POST, "/admin/v1/cluster/reload", token, None)
            .await
    }

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
    ) -> Resposta {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base));
        if !token.is_empty() {
            pedido = pedido.bearer_auth(token);
        }
        if let Some(valor) = corpo {
            pedido = pedido.json(&valor);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        let cru = resposta.text().await.unwrap_or_default();
        let corpo = serde_json::from_str(&cru).unwrap_or(Value::Null);
        Resposta { status, corpo, cru }
    }

    /// Cria uma org e um token só dela (para provar o 403 da recarga).
    async fn token_de_org(&self) -> String {
        let org = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({"name": "Org do cluster", "slug": "org-do-cluster"})),
            )
            .await;
        assert_eq!(org.status, StatusCode::CREATED, "{}", org.cru);
        let id = org.corpo["id"].as_str().expect("id da org");
        let token = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({"label": "org"})),
            )
            .await;
        assert_eq!(token.status, StatusCode::CREATED, "{}", token.cru);
        token.corpo["value"]
            .as_str()
            .expect("valor do token")
            .to_owned()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

#[tokio::test]
async fn recarga_le_o_arquivo_do_processo_vivo_e_aponta_o_que_mudou() {
    let c = Cenario::novo("troca").await;

    // Primeira recarga: nada mudou desde a subida.
    let primeira = c.recarregar(TOKEN_GLOBAL).await;
    assert_eq!(primeira.status, StatusCode::OK, "{}", primeira.cru);
    assert_eq!(primeira.corpo["mudou"], json!(false));
    assert_eq!(
        primeira.corpo["fingerprint_antes"],
        primeira.corpo["fingerprint_depois"]
    );
    assert_eq!(primeira.corpo["depois"]["modo"], json!("cluster"));
    assert_eq!(primeira.corpo["depois"]["instance_id"], json!("a"));
    assert_eq!(primeira.corpo["depois"]["segredo_configurado"], json!(true));
    let fingerprint_original = primeira.corpo["fingerprint_depois"]
        .as_str()
        .expect("fingerprint")
        .to_owned();

    // Edita o arquivo e recarrega: o mapa novo vale na hora.
    c.escrever_mapa(MAPA_TROCADO);
    let segunda = c.recarregar(TOKEN_GLOBAL).await;
    assert_eq!(segunda.status, StatusCode::OK, "{}", segunda.cru);
    assert_eq!(segunda.corpo["mudou"], json!(true));
    assert_eq!(
        segunda.corpo["fingerprint_antes"],
        json!(fingerprint_original),
        "a impressão anterior é a do mapa que estava em uso"
    );
    assert_ne!(
        segunda.corpo["fingerprint_depois"],
        json!(fingerprint_original)
    );
    let trocas = segunda.corpo["orgs_que_trocaram"]
        .as_array()
        .expect("trocas");
    assert_eq!(trocas.len(), 1, "só org_01 trocou: {trocas:?}");
    assert_eq!(trocas[0]["org"], json!("org_01"));
    assert_eq!(trocas[0]["de"], json!("a"));
    assert_eq!(trocas[0]["para"], json!("b"));
    let fingerprint_trocado = segunda.corpo["fingerprint_depois"]
        .as_str()
        .expect("fingerprint")
        .to_owned();

    // Arquivo inválido: 400 e o mapa antigo continua em uso.
    c.escrever_mapa(MAPA_INVALIDO);
    let invalida = c.recarregar(TOKEN_GLOBAL).await;
    assert_eq!(invalida.status, StatusCode::BAD_REQUEST, "{}", invalida.cru);
    assert_eq!(invalida.corpo["code"], json!("cluster_invalido"));
    let problemas = invalida.corpo["details"]["problemas"]
        .as_array()
        .expect("problemas");
    assert!(
        problemas
            .iter()
            .any(|p| p.as_str().is_some_and(|p| p.contains("orgs.org_01"))),
        "o problema aponta a chave: {problemas:?}"
    );

    // Volta o arquivo ao mapa trocado: a recarga confirma que o mapa em uso era o bom anterior.
    c.escrever_mapa(MAPA_TROCADO);
    let volta = c.recarregar(TOKEN_GLOBAL).await;
    assert_eq!(volta.status, StatusCode::OK, "{}", volta.cru);
    assert_eq!(
        volta.corpo["fingerprint_antes"],
        json!(fingerprint_trocado),
        "o arquivo inválido não trocou o mapa em uso"
    );
    assert_eq!(volta.corpo["mudou"], json!(false));

    // Sem arquivo: volta ao modo instância única (é o que a recarga reflete).
    fs::remove_file(&c.caminho_cluster).expect("remove cluster.toml");
    let sem_arquivo = c.recarregar(TOKEN_GLOBAL).await;
    assert_eq!(sem_arquivo.status, StatusCode::OK, "{}", sem_arquivo.cru);
    assert_eq!(
        sem_arquivo.corpo["depois"]["modo"],
        json!("instancia_unica")
    );
    assert_eq!(sem_arquivo.corpo["mudou"], json!(true));

    // Nenhuma resposta carrega o segredo compartilhado (nem o fingerprint).
    for corpo in [
        primeira.cru,
        segunda.cru,
        invalida.cru,
        volta.cru,
        sem_arquivo.cru,
    ] {
        assert!(
            !corpo.contains(SEGREDO),
            "segredo do cluster vazou na resposta: {corpo}"
        );
    }
}

#[tokio::test]
async fn recarga_exige_token_global() {
    let c = Cenario::novo("autorizacao").await;

    let sem_token = c.recarregar("").await;
    assert_eq!(
        sem_token.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        sem_token.cru
    );

    let de_org = c.token_de_org().await;
    let barrada = c.recarregar(&de_org).await;
    assert_eq!(barrada.status, StatusCode::FORBIDDEN, "{}", barrada.cru);
    assert_eq!(barrada.corpo["code"], json!("forbidden"));
    assert!(!barrada.cru.contains(SEGREDO));
}
