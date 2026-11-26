//! Manifesto de plugin e configuração/segredo de integração (HR-44c), de ponta a ponta.
//!
//! Servidor de verdade (porta efêmera) sobre banco temporário, cliente HTTP de verdade e um
//! **plugin falso em processo externo** que fala o protocolo por stdio (o mesmo caminho do
//! `poph-plugin-eco`). Nada de rede externa.
//!
//! O que se prova: a rota de manifesto devolve o que o plugin declarou no handshake e não sobe o
//! plugin a cada requisição (o falso conta as subidas); tipo sem plugin declarado dá 404; plugin
//! declarado que não responde ao handshake dá 503; configuração é validada campo a campo contra o
//! esquema (obrigatório, tipo, enum, campo fora do esquema); segredo é gravado cifrado (o arquivo
//! do banco não contém o valor), nunca volta na resposta e é mesclado no `PATCH`; sem manifesto,
//! configuração continua recusada com 422; `428`/`412` como nas outras escritas; token de outra org
//! dá 403. O valor do segredo também não aparece no log do processo.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::{ChaveDeCifra, Estado};
use poph_store::{NovaOrg, NovoAgente, Store};
use poph_types::config::GlobalTokenConfig;
use poph_types::{AgentId, DeclaredPlugin, HarnessConfig, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tracing_subscriber::fmt::MakeWriter;

/// Token global dos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-hr44c";
/// Chave de cifra usada no teste (nunca vai para arquivo de configuração).
const CHAVE_CIFRA: &str = "chave-de-cifra-do-teste-hr44c";
/// Valor de segredo que **não pode** aparecer em resposta, banco ou log.
const SEGREDO: &str = "segredo-que-nao-pode-vazar-hr44c";

// ───────────────────────── captura de log (para provar o que NÃO sai) ─────────────────────────

#[derive(Clone)]
struct Captura(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captura {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captura {
    type Writer = Captura;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn logs_capturados() -> Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing_subscriber::fmt()
            .with_writer(Captura(Arc::clone(&buffer)))
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
        buffer
    })
    .clone()
}

// ───────────────────────── plugin falso ─────────────────────────

/// Manifesto do plugin falso: configuração com obrigatório, enum e número; segredo obrigatório.
fn manifesto_falso() -> Value {
    json!({
        "nome": "Plugin de teste",
        "kind": "fake",
        "versao": "0.1.0-teste",
        "protocolo": 1,
        "capacidades": { "text": true },
        "limites": {},
        "configuracao": {
            "type": "object",
            "properties": {
                "remetente": { "type": "string", "minLength": 1, "title": "Remetente" },
                "modo": { "type": "string", "enum": ["inverter", "ecoar"] },
                "tentativas": { "type": "integer", "minimum": 1, "maximum": 5 },
                "avisar": { "type": "boolean" }
            },
            "required": ["remetente"]
        },
        "segredos": {
            "type": "object",
            "properties": {
                "token": { "type": "string", "title": "Token" }
            },
            "required": ["token"]
        }
    })
}

/// Escreve o plugin falso: responde ao handshake com o manifesto e conta as subidas num arquivo.
fn escrever_plugin(pasta: &std::path::Path) -> PathBuf {
    let contador = pasta.join("subidas.txt");
    let manifesto = serde_json::to_string(&manifesto_falso()).expect("manifesto em JSON");
    let script = pasta.join("plugin_falso.py");
    let codigo = format!(
        r#"import io, json, sys
with io.open({contador:?}, "a", encoding="utf-8") as registro:
    registro.write("subiu\n")
manifesto = json.loads({manifesto:?})
for linha in sys.stdin:
    linha = linha.strip()
    if not linha:
        continue
    mensagem = json.loads(linha)
    if mensagem.get("tipo") == "handshake":
        print(json.dumps({{"tipo": "handshake_ok", "protocolo": 1, "manifesto": manifesto}}), flush=True)
    elif mensagem.get("tipo") == "encerrar":
        sys.exit(0)
    # Qualquer outra mensagem é ignorada: este falso só serve para o manifesto.
"#,
        contador = contador.display().to_string(),
        manifesto = manifesto,
    );
    fs::write(&script, codigo).expect("plugin falso");
    script
}

// ───────────────────────── cenário ─────────────────────────

struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    org: OrgId,
    agente: AgentId,
    contador: PathBuf,
    logs: Arc<Mutex<Vec<u8>>>,
    desligar: Option<oneshot::Sender<()>>,
}

struct Resposta {
    status: reqwest::StatusCode,
    corpo: Value,
    cru: String,
}

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        let logs = logs_capturados();
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-manifestos-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
        let script = escrever_plugin(&pasta);

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.plugins.integrations = vec![
            DeclaredPlugin {
                kind: "fake".to_owned(),
                program: PathBuf::from("python3"),
                arguments: vec![script.display().to_string()],
            },
            // Plugin declarado que não fala o protocolo: prova o 503 explicado.
            DeclaredPlugin {
                kind: "quebrado".to_owned(),
                program: PathBuf::from("/bin/false"),
                arguments: Vec::new(),
            },
        ];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::abrir(&config).expect("abre o banco"));
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: "Org do manifesto".to_owned(),
                slug: "org-do-manifesto".to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: "Atendente".to_owned(),
                    slug: "atendente".to_owned(),
                    ..Default::default()
                },
            )
            .expect("cria o agente")
            .id;

        let estado = Estado::novo(Arc::clone(&store), config)
            .expect("estado do servidor")
            .com_chave_de_cifra(Some(
                ChaveDeCifra::derivar(CHAVE_CIFRA).expect("chave de cifra"),
            ));
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
            pasta: pasta.clone(),
            org,
            agente,
            contador: pasta.join("subidas.txt"),
            logs,
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

    fn rota_integracoes(&self) -> String {
        format!(
            "/admin/v1/orgs/{}/agents/{}/integrations",
            self.org, self.agente
        )
    }

    fn rota_manifesto(&self, kind: &str) -> String {
        format!("/admin/v1/orgs/{}/manifests/{kind}", self.org)
    }

    fn rota_lista(&self) -> String {
        format!("/admin/v1/orgs/{}/manifests", self.org)
    }

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
        if_match: Option<u64>,
    ) -> Resposta {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base));
        if !token.is_empty() {
            pedido = pedido.bearer_auth(token);
        }
        if let Some(versao) = if_match {
            pedido = pedido.header("If-Match", versao.to_string());
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

    fn subidas_do_plugin(&self) -> usize {
        fs::read_to_string(&self.contador)
            .map(|texto| texto.lines().count())
            .unwrap_or(0)
    }

    /// Conteúdo de todos os arquivos da pasta de dados (banco, WAL, blobs).
    ///
    /// O SQLite grava em WAL: procurar só em `harness.db` daria falso negativo.
    fn bytes_no_disco(&self) -> Vec<u8> {
        let mut tudo = Vec::new();
        let pasta = self.pasta.join("dados");
        for entrada in fs::read_dir(&pasta).expect("pasta de dados") {
            let caminho = entrada.expect("entrada").path();
            if caminho.is_file() {
                tudo.extend_from_slice(&fs::read(&caminho).unwrap_or_default());
            }
        }
        assert!(!tudo.is_empty(), "a pasta de dados não tem arquivo");
        tudo
    }

    fn logs(&self) -> String {
        String::from_utf8_lossy(&self.logs.lock().unwrap()).to_string()
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

fn codigo(corpo: &Value) -> &str {
    corpo["code"].as_str().unwrap_or_default()
}

// ───────────────────────── provas ─────────────────────────

#[tokio::test]
async fn manifesto_vem_do_handshake_e_fica_em_cache() {
    let c = Cenario::novo("manifesto").await;
    let (status, corpo) = {
        let r = c
            .pedir(
                Method::GET,
                &c.rota_manifesto("fake"),
                TOKEN_GLOBAL,
                None,
                None,
            )
            .await;
        (r.status, r.corpo)
    };
    assert_eq!(status, reqwest::StatusCode::OK, "{corpo}");
    assert_eq!(corpo["kind"], "fake");
    assert_eq!(corpo["nome"], "Plugin de teste");
    assert_eq!(
        corpo["configuracao"]["required"],
        json!(["remetente"]),
        "o esquema da configuração é o do plugin"
    );
    assert_eq!(corpo["segredos"]["required"], json!(["token"]));
    assert_eq!(
        c.subidas_do_plugin(),
        1,
        "a primeira leitura sobe o plugin uma vez"
    );

    // Segunda leitura: nada de subir o plugin de novo.
    let r = c
        .pedir(
            Method::GET,
            &c.rota_manifesto("fake"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK);
    assert_eq!(c.subidas_do_plugin(), 1, "o manifesto vem do cache");
}

#[tokio::test]
async fn tipo_sem_plugin_da_404_e_plugin_quebrado_da_503() {
    let c = Cenario::novo("falhas-manifesto").await;
    let r = c
        .pedir(
            Method::GET,
            &c.rota_manifesto("sms"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::NOT_FOUND, "{}", r.cru);
    assert_eq!(codigo(&r.corpo), "not_found");

    let r = c
        .pedir(
            Method::GET,
            &c.rota_manifesto("quebrado"),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        r.cru
    );
    assert_eq!(codigo(&r.corpo), "manifesto_indisponivel");
    assert!(
        !r.cru.contains("/bin/false") && !r.cru.contains("No such file"),
        "a resposta não expõe caminho nem erro do sistema: {}",
        r.cru
    );

    // Integração de tipo declarado mas quebrado: com configuração, 503 (não grava às cegas).
    let r = c
        .pedir(
            Method::POST,
            &c.rota_integracoes(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "quebrado", "config_json": {"remetente": "Loja"}})),
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        r.cru
    );
    assert_eq!(codigo(&r.corpo), "manifesto_indisponivel");
}

#[tokio::test]
async fn criacao_guarda_configuracao_e_segredo_cifrado_sem_expor_nada() {
    let c = Cenario::novo("criacao").await;
    let r = c
        .pedir(
            Method::POST,
            &c.rota_integracoes(),
            TOKEN_GLOBAL,
            Some(json!({
                "kind": "fake",
                "config_json": {"remetente": "Loja", "modo": "inverter", "tentativas": 2},
                "secrets": {"token": SEGREDO}
            })),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::CREATED, "{}", r.cru);
    assert_eq!(r.corpo["version"], 1, "criação não gasta versão extra");
    assert_eq!(r.corpo["configuracao"]["remetente"], "Loja");
    assert_eq!(r.corpo["tem_segredo"], true);
    assert_eq!(r.corpo["segredos_preenchidos"], json!(["token"]));
    assert!(
        !r.cru.contains(SEGREDO),
        "o segredo não volta na resposta de criação: {}",
        r.cru
    );
    let id = r.corpo["id"].as_str().expect("id").to_owned();

    // Leitura e listagem também não devolvem o valor.
    let detalhe = c
        .pedir(
            Method::GET,
            &format!("{}/{id}", c.rota_integracoes()),
            TOKEN_GLOBAL,
            None,
            None,
        )
        .await;
    assert_eq!(detalhe.status, reqwest::StatusCode::OK);
    assert_eq!(detalhe.corpo["configuracao"]["tentativas"], 2);
    assert!(!detalhe.cru.contains(SEGREDO), "{}", detalhe.cru);
    let lista = c
        .pedir(Method::GET, &c.rota_integracoes(), TOKEN_GLOBAL, None, None)
        .await;
    assert!(!lista.cru.contains(SEGREDO), "{}", lista.cru);

    // O banco guarda o segredo cifrado: o valor em claro não está no arquivo.
    let banco = c.bytes_no_disco();
    let contem = banco
        .windows(SEGREDO.len())
        .any(|janela| janela == SEGREDO.as_bytes());
    assert!(!contem, "o segredo não pode estar em claro em `harness.db`");

    // E o log do processo também não o recebeu.
    assert!(
        !c.logs().contains(SEGREDO),
        "o segredo não pode ir para o log"
    );
}

#[tokio::test]
async fn configuracao_e_segredo_sao_validados_campo_a_campo() {
    let c = Cenario::novo("validacao").await;
    let casos: Vec<(Value, &str)> = vec![
        (
            json!({"remetente": "Loja", "tentativas": "duas"}),
            "campo `config_json.tentativas`",
        ),
        (
            json!({"remetente": "Loja", "modo": "outro"}),
            "campo `config_json.modo`",
        ),
        (
            json!({"remetente": "Loja", "inventado": 1}),
            "campo `config_json.inventado`",
        ),
        (json!({"modo": "inverter"}), "campo `config_json.remetente`"),
    ];
    for (configuracao, campo) in casos {
        let r = c
            .pedir(
                Method::POST,
                &c.rota_integracoes(),
                TOKEN_GLOBAL,
                Some(
                    json!({"kind": "fake", "config_json": configuracao, "secrets": {"token": "x"}}),
                ),
                None,
            )
            .await;
        assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.cru);
        assert_eq!(codigo(&r.corpo), "validation_error", "{}", r.cru);
        assert!(
            r.corpo["message"]
                .as_str()
                .unwrap_or_default()
                .contains(campo),
            "esperava {campo}: {}",
            r.cru
        );
        assert_eq!(r.corpo["details"]["field"], &campo[7..campo.len() - 1]);
    }

    // Segredo obrigatório ausente.
    let r = c
        .pedir(
            Method::POST,
            &c.rota_integracoes(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "fake", "config_json": {"remetente": "Loja"}})),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.cru);
    assert_eq!(r.corpo["details"]["field"], "secrets.token");

    // Criação sem configuração nenhuma de um tipo com manifesto também é cobrada: a integração
    // nasceria sem o que o plugin exige.
    let r = c
        .pedir(
            Method::POST,
            &c.rota_integracoes(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "fake"})),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.cru);
    assert_eq!(r.corpo["details"]["field"], "config_json.remetente");

    // Nada foi criado nas tentativas recusadas.
    let lista = c
        .pedir(Method::GET, &c.rota_integracoes(), TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(
        lista.corpo["items"].as_array().map(Vec::len),
        Some(0),
        "{}",
        lista.cru
    );
}

#[tokio::test]
async fn sem_manifesto_configuracao_continua_recusada() {
    let c = Cenario::novo("sem-manifesto").await;
    let r = c
        .pedir(
            Method::POST,
            &c.rota_integracoes(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "sms", "config_json": {"qualquer": 1}})),
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        r.cru
    );
    assert_eq!(codigo(&r.corpo), "manifest_indisponivel");

    // Sem configuração, integração de tipo sem manifesto continua sendo criada como antes.
    let r = c
        .pedir(
            Method::POST,
            &c.rota_integracoes(),
            TOKEN_GLOBAL,
            Some(json!({"kind": "sms"})),
            None,
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::CREATED, "{}", r.cru);
    assert_eq!(r.corpo["configuracao"], json!({}));
    assert_eq!(r.corpo["tem_segredo"], false);
}

#[tokio::test]
async fn patch_troca_configuracao_mescla_segredo_e_exige_if_match() {
    let c = Cenario::novo("patch").await;
    let criada = c
        .pedir(
            Method::POST,
            &c.rota_integracoes(),
            TOKEN_GLOBAL,
            Some(json!({
                "kind": "fake",
                "config_json": {"remetente": "Loja"},
                "secrets": {"token": SEGREDO}
            })),
            None,
        )
        .await;
    assert_eq!(
        criada.status,
        reqwest::StatusCode::CREATED,
        "{}",
        criada.cru
    );
    let id = criada.corpo["id"].as_str().expect("id").to_owned();
    let rota = format!("{}/{id}", c.rota_integracoes());

    // 428 sem If-Match e 412 com versão velha.
    let r = c
        .pedir(
            Method::PATCH,
            &rota,
            TOKEN_GLOBAL,
            Some(json!({"config_json": {"remetente": "Nova"}})),
            None,
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::PRECONDITION_REQUIRED,
        "{}",
        r.cru
    );
    let r = c
        .pedir(
            Method::PATCH,
            &rota,
            TOKEN_GLOBAL,
            Some(json!({"config_json": {"remetente": "Nova"}})),
            Some(9),
        )
        .await;
    assert_eq!(
        r.status,
        reqwest::StatusCode::PRECONDITION_FAILED,
        "{}",
        r.cru
    );
    assert_eq!(codigo(&r.corpo), "version_conflict");

    // Configuração nova substitui a anterior e sobe a versão.
    let r = c
        .pedir(
            Method::PATCH,
            &rota,
            TOKEN_GLOBAL,
            Some(json!({"config_json": {"remetente": "Nova", "avisar": true}})),
            Some(1),
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.cru);
    assert_eq!(r.corpo["version"], 2);
    assert_eq!(
        r.corpo["configuracao"],
        json!({"remetente": "Nova", "avisar": true})
    );
    assert_eq!(
        r.corpo["segredos_preenchidos"],
        json!(["token"]),
        "o PATCH da configuração não mexe no segredo"
    );

    // Segredo novo é mesclado (o esquema só tem `token`, então o valor troca).
    let r = c
        .pedir(
            Method::PATCH,
            &rota,
            TOKEN_GLOBAL,
            Some(json!({"secrets": {"token": "outro-valor"}})),
            Some(2),
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.cru);
    assert_eq!(r.corpo["version"], 3);
    assert_eq!(r.corpo["segredos_preenchidos"], json!(["token"]));
    assert!(!r.cru.contains("outro-valor"), "{}", r.cru);

    // Remover o segredo obrigatório é recusado pelo esquema (a integração ficaria inválida).
    let r = c
        .pedir(
            Method::PATCH,
            &rota,
            TOKEN_GLOBAL,
            Some(json!({"secrets": {"token": ""}})),
            Some(3),
        )
        .await;
    assert_eq!(r.status, reqwest::StatusCode::BAD_REQUEST, "{}", r.cru);
    assert_eq!(r.corpo["details"]["field"], "secrets.token");

    // E o estado gravado continua sendo o da última gravação válida.
    let detalhe = c.pedir(Method::GET, &rota, TOKEN_GLOBAL, None, None).await;
    assert_eq!(detalhe.corpo["version"], 3);
    assert_eq!(detalhe.corpo["configuracao"]["remetente"], "Nova");
}

#[tokio::test]
async fn token_de_outra_org_nao_le_manifesto() {
    let c = Cenario::novo("isolamento").await;
    let org = c
        .pedir(
            Method::POST,
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            Some(json!({"name": "Outra", "slug": "outra"})),
            None,
        )
        .await;
    assert_eq!(org.status, reqwest::StatusCode::CREATED, "{}", org.cru);
    let id = org.corpo["id"].as_str().expect("id da org");
    let token = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            Some(json!({"label": "outra"})),
            None,
        )
        .await;
    assert_eq!(token.status, reqwest::StatusCode::CREATED, "{}", token.cru);
    let valor = token.corpo["value"].as_str().expect("valor").to_owned();
    let r = c
        .pedir(Method::GET, &c.rota_manifesto("fake"), &valor, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::FORBIDDEN, "{}", r.cru);
    assert_eq!(
        c.subidas_do_plugin(),
        0,
        "nada subiu para um pedido barrado"
    );
}

/// Garante que os tipos exportados continuam utilizáveis de fora (a tela/CLI dependem deles).
#[test]
fn tipos_publicos_do_manifesto_serializam() {
    let manifesto = manifesto_falso();
    let texto = serde_json::to_string(&manifesto).expect("JSON");
    assert!(texto.contains("configuracao"));
}

// ───────────────────── lista de plugins declarados (HR-48b) ─────────────────────

#[tokio::test]
async fn lista_traz_os_tipos_declarados_e_marca_o_quebrado_sem_derrubar() {
    let c = Cenario::novo("lista").await;
    let r = c
        .pedir(Method::GET, &c.rota_lista(), TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.cru);
    let itens = r.corpo["items"].as_array().expect("lista de itens");
    assert_eq!(itens.len(), 2, "os dois tipos declarados: {}", r.cru);

    // Ordem determinística: o `kind` ordenado.
    assert_eq!(itens[0]["kind"], "fake");
    assert_eq!(itens[0]["situacao"], "ok");
    assert_eq!(itens[0]["nome"], "Plugin de teste");
    assert_eq!(itens[0]["versao"], "0.1.0-teste");
    assert!(
        itens[0].get("mensagem").is_none(),
        "plugin ok não traz frase de erro: {}",
        r.cru
    );

    assert_eq!(itens[1]["kind"], "quebrado");
    assert_eq!(itens[1]["situacao"], "indisponivel");
    assert!(itens[1].get("nome").is_none(), "{}", r.cru);
    assert!(itens[1].get("versao").is_none(), "{}", r.cru);
    assert!(
        itens[1]["mensagem"]
            .as_str()
            .unwrap_or_default()
            .contains("não respondeu ao handshake"),
        "o item quebrado traz a frase de operação: {}",
        r.cru
    );

    // Nada de caminho de programa, argumento, segredo nem detalhe do sistema.
    for proibido in [
        "/bin/false",
        "python3",
        "plugin_falso.py",
        "\"programa\"",
        "\"argumentos\"",
        CHAVE_CIFRA,
        SEGREDO,
    ] {
        assert!(
            !r.cru.contains(proibido),
            "a lista não pode expor `{proibido}`: {}",
            r.cru
        );
    }

    // Segunda listagem: o manifesto vem do cache, o plugin não sobe de novo.
    let r = c
        .pedir(Method::GET, &c.rota_lista(), TOKEN_GLOBAL, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::OK, "{}", r.cru);
    assert_eq!(
        c.subidas_do_plugin(),
        1,
        "a listagem reaproveita o cache da consulta por tipo"
    );
}

#[tokio::test]
async fn lista_exige_token_e_respeita_a_org_do_token() {
    let c = Cenario::novo("lista-isolamento").await;
    let sem_token = c.pedir(Method::GET, &c.rota_lista(), "", None, None).await;
    assert_eq!(
        sem_token.status,
        reqwest::StatusCode::UNAUTHORIZED,
        "{}",
        sem_token.cru
    );

    let org = c
        .pedir(
            Method::POST,
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            Some(json!({"name": "Outra", "slug": "outra"})),
            None,
        )
        .await;
    assert_eq!(org.status, reqwest::StatusCode::CREATED, "{}", org.cru);
    let id = org.corpo["id"].as_str().expect("id da org");
    let token = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            Some(json!({"label": "outra"})),
            None,
        )
        .await;
    assert_eq!(token.status, reqwest::StatusCode::CREATED, "{}", token.cru);
    let valor = token.corpo["value"].as_str().expect("valor").to_owned();

    let r = c
        .pedir(Method::GET, &c.rota_lista(), &valor, None, None)
        .await;
    assert_eq!(r.status, reqwest::StatusCode::FORBIDDEN, "{}", r.cru);
    assert_eq!(
        c.subidas_do_plugin(),
        0,
        "nada subiu para um pedido barrado"
    );
}
