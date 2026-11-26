//! Suíte automática de isolamento entre organizações (HR-54).
//!
//! Tudo contra servidor de verdade (porta efêmera, banco temporário, cliente HTTP).
//!
//! `isolamento_gerado_do_openapi` lê a OpenAPI que o servidor publica e exige um caso de teste
//! para cada operação `/admin/v1/orgs/...`: operação publicada sem caso reprova, e caso sem
//! operação publicada também. Para cada operação, duas orgs (A e B, cada uma com agente,
//! categoria, departamento, produto, grupo e item de conhecimento, arquivo, integração, conversa
//! e token) se atacam nas duas direções:
//!
//! - org da vítima no caminho e ids da vítima: `403` (token de outra org);
//! - org do próprio token no caminho e ids da vítima: `403`, `404` ou `409`, nunca sucesso;
//! - `DELETE` com `force` declarado na OpenAPI também é atacado com `?force=true`;
//! - listas do próprio token nunca trazem dado da outra org;
//! - sem token: `401`;
//! - cada `GET` do dono: `200` (controle: o caminho montado existe e a rota responde);
//! - cruzamento por corpo (produto com categoria da outra, agente com categoria ou departamento
//!   da outra): erro de referência, nunca criação;
//! - no fim, o estado de cada org, lido pelo dono, é igual ao de antes dos ataques.
//!
//! Os cabeçalhos `If-Match` e os parâmetros `force` vêm da própria OpenAPI.
//!
//! `ferramentas_do_llm_nao_alcancam_outra_org` e `busca_da_base_nao_devolve_trecho_de_outra_org`
//! usam o turno de verdade com modelo falso: o modelo pede ferramentas com ids e termos da outra
//! org, e um controle com os ids da própria org prova que a ferramenta de fato executa.
//!
//! `caso_faltando_reprova_a_cobertura` e os testes de detector provam que a própria suíte
//! reprova o que deve reprovar.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use futures::StreamExt;
use futures::future::BoxFuture;
use poph_agent::atores::{Atores, OpcoesAtores};
use poph_agent::base_conhecimento::{
    BaseConhecimento, BuscadorBase, ConsultarBase, ParametrosBusca,
};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::entrada_saida::{Fluxo, OpcoesFluxo};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::produtos::{BuscadorProdutos, BuscarProduto, CatalogoProdutos, ParametrosProdutos};
use poph_agent::turno::{Motor, OpcoesTurno, ProcessadorTurno};
use poph_core::llm::{
    ChamadaFerramenta, ErroLlm, EventoLlm, FluxoLlm, LlmProvider, Mensagem, MotivoFim, Pedido,
    ProvedorFake, Roteiro,
};
use poph_files::ConfigArquivos;
use poph_rag::{ConfigIndice, EmbedderFake, Indice};
use poph_server::Estado;
use poph_store::{
    AuditEntry, Direction, NewChannelAttachment, NewMessage, OutboxState, Page, Role, Store,
};
use poph_types::HarnessConfig;
use poph_types::config::GlobalTokenConfig;
use poph_types::{ConversationId, OrgId};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const TOKEN_GLOBAL: &str = "pophg_teste-global-hr54";
const MARCADOR_A: &str = "MARCADOR-ORG-A";
const MARCADOR_B: &str = "MARCADOR-ORG-B";
/// Ferramentas que o agente de teste tem habilitadas (as da V1 que o turno executa).
const FERRAMENTAS: &[&str] = &[
    "consultar_base",
    "buscar_produto",
    "enviar_arquivo",
    "cart_add_item",
    "cart_summary",
];
/// Operações publicadas fora de `/admin/v1/orgs` (não são de tenant e não têm org).
const NAO_TENANT: &[(&str, &str)] = &[
    ("get", "/healthz"),
    ("get", "/metrics"),
    ("post", "/admin/v1/cluster/reload"),
];

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn png() -> &'static [u8] {
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89"
}

/// Servidor no ar com banco próprio. O `Drop` desliga e apaga a pasta.
struct Servidor {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    #[allow(dead_code)]
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Servidor {
    async fn novo(
        rotulo: &str,
        estado: impl FnOnce(&Arc<Store>, &HarnessConfig, &PathBuf) -> Estado,
    ) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-isolamento-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7490;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let estado = estado(&store, &config, &pasta);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("porta efêmera");
        let endereco = listener.local_addr().expect("endereço local");
        let (desligar, sinal) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = poph_server::servir_em(listener, estado, async {
                let _ = sinal.await;
            })
            .await;
        });
        let servidor = Self {
            base: format!("http://{endereco}"),
            cliente: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("cliente HTTP"),
            pasta,
            store,
            desligar: Some(desligar),
        };
        servidor.esperar_subir().await;
        servidor
    }

    async fn esperar_subir(&self) {
        let inicio = Instant::now();
        loop {
            match self
                .cliente
                .get(format!("{}/healthz", self.base))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => return,
                _ if inicio.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
    }

    /// Pedido com o token dado (vazio = anônimo). Devolve status, texto cru e JSON.
    async fn chamar(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Corpo,
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
        pedido = match corpo {
            Corpo::Nenhum => pedido,
            Corpo::Json(valor) => pedido.json(&valor),
            Corpo::Multipart(texto) => {
                let (tipo, bytes) = multipart(&texto);
                pedido.header("content-type", tipo).body(bytes)
            }
            Corpo::Arquivo(bytes) => pedido
                .header("content-type", "application/octet-stream")
                .body(bytes),
        };
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        let texto = resposta.text().await.unwrap_or_default();
        let json = serde_json::from_str(&texto).unwrap_or(Value::Null);
        Resposta {
            status,
            texto,
            json,
        }
    }

    /// Cria algo com o token global ou da própria org e exige o status esperado.
    async fn criar(&self, caminho: &str, token: &str, corpo: Value, esperado: StatusCode) -> Value {
        let r = self
            .chamar(Method::POST, caminho, token, Corpo::Json(corpo), None)
            .await;
        assert_eq!(r.status, esperado, "criar em {caminho}: {}", r.texto);
        r.json
    }

    /// Estado que o dono enxerga de uma org: todas as leituras que a API oferece sobre ela.
    async fn foto(&self, org: &Org) -> Vec<String> {
        let mut caminhos = vec![
            format!("/admin/v1/orgs/{}", org.id),
            format!("/admin/v1/orgs/{}/agents", org.id),
            format!("/admin/v1/orgs/{}/agents/{}", org.id, org.agente),
            format!(
                "/admin/v1/orgs/{}/agents/{}/instructions/current",
                org.id, org.agente
            ),
            format!("/admin/v1/orgs/{}/agents/{}/files", org.id, org.agente),
            format!(
                "/admin/v1/orgs/{}/agents/{}/files/{}",
                org.id, org.agente, org.arquivo
            ),
            format!(
                "/admin/v1/orgs/{}/agents/{}/integrations",
                org.id, org.agente
            ),
            format!(
                "/admin/v1/orgs/{}/agents/{}/integrations/{}",
                org.id, org.agente, org.integracao
            ),
            format!("/admin/v1/orgs/{}/agents/{}/test-chat", org.id, org.agente),
            format!(
                "/admin/v1/orgs/{}/agents/{}/test-chat/{}",
                org.id, org.agente, org.conversa
            ),
            format!("/admin/v1/orgs/{}/conversations", org.id),
            format!(
                "/admin/v1/orgs/{}/conversations/{}/messages",
                org.id, org.conversa
            ),
            format!(
                "/admin/v1/orgs/{}/conversations/{}/turns",
                org.id, org.conversa
            ),
            format!("/admin/v1/orgs/{}/categories", org.id),
            format!("/admin/v1/orgs/{}/categories/{}", org.id, org.categoria),
            format!("/admin/v1/orgs/{}/departments", org.id),
            format!("/admin/v1/orgs/{}/departments/{}", org.id, org.departamento),
            format!("/admin/v1/orgs/{}/products", org.id),
            format!("/admin/v1/orgs/{}/products/{}", org.id, org.produto),
            format!("/admin/v1/orgs/{}/knowledge", org.id),
            format!("/admin/v1/orgs/{}/knowledge/{}", org.id, org.grupo),
            format!("/admin/v1/orgs/{}/knowledge/{}/items", org.id, org.grupo),
            format!(
                "/admin/v1/orgs/{}/knowledge/{}/items/{}",
                org.id, org.grupo, org.item
            ),
            format!("/admin/v1/orgs/{}/tokens", org.id),
            format!("/admin/v1/orgs/{}/tokens/{}", org.id, org.token_id),
            format!("/admin/v1/orgs/{}/flows", org.id),
            format!("/admin/v1/orgs/{}/flows/{}", org.id, org.fluxo),
            format!("/admin/v1/orgs/{}/flows/{}/revisions", org.id, org.fluxo),
            format!("/admin/v1/orgs/{}/fichas", org.id),
            format!("/admin/v1/orgs/{}/fichas/{}", org.id, org.ficha),
            format!("/admin/v1/orgs/{}/fichas/{}/events", org.id, org.ficha),
        ];
        caminhos.sort();
        let mut fotos = Vec::new();
        for caminho in caminhos {
            let r = self
                .chamar(Method::GET, &caminho, &org.token, Corpo::Nenhum, None)
                .await;
            fotos.push(format!("{caminho} -> {} {}", r.status, r.texto));
        }
        fotos
    }

    fn auditoria(&self, org: &Org) -> Vec<AuditEntry> {
        let id = org.id.parse::<OrgId>().expect("id da org");
        self.store
            .audit()
            .listar(id, Page::with_limit(200))
            .expect("auditoria da org")
    }
}

impl Drop for Servidor {
    fn drop(&mut self) {
        if let Some(desligar) = self.desligar.take() {
            let _ = desligar.send(());
        }
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// Corpo de um pedido já montado.
enum Corpo {
    Nenhum,
    Json(Value),
    Multipart(String),
    Arquivo(Vec<u8>),
}

struct Resposta {
    status: StatusCode,
    texto: String,
    json: Value,
}

/// Corpo multipart montado à mão (o cliente de teste não traz a feature `multipart`).
fn multipart(texto: &str) -> (String, Vec<u8>) {
    let limite = "hr54-boundary";
    let corpo = format!(
        "--{limite}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{texto}\r\n--{limite}--\r\n"
    );
    (
        format!("multipart/form-data; boundary={limite}"),
        corpo.into_bytes(),
    )
}

/// Uma org de teste com um de cada recurso que a API expõe.
#[derive(Clone)]
struct Org {
    id: String,
    token: String,
    token_id: String,
    agente: String,
    grupo: String,
    item: String,
    arquivo: String,
    integracao: String,
    categoria: String,
    departamento: String,
    produto: String,
    conversa: String,
    /// Fluxo publicado da org (FX-27), liberado ao agente.
    fluxo: String,
    /// Versão do agente depois da liberação do fluxo (FX-28 sobe a versão a cada troca).
    versao_agente: u64,
    /// Ficha da conversa de teste (FX-29).
    ficha: String,
    /// Execução de fluxo aberta na Ficha (FX-30).
    execucao: String,
    /// Mensagem do cliente com anexo dentro da conversa de teste (HR-46b).
    mensagem: String,
    /// Anexo dessa mensagem (HR-46b).
    anexo: String,
    marcador: &'static str,
}

impl Org {
    /// Parâmetros de caminho que são ids desta org (o `kind` é o tipo de plugin, não é dado).
    fn params(&self) -> Vec<(&'static str, String)> {
        vec![
            ("agent_id", self.agente.clone()),
            ("group_id", self.grupo.clone()),
            ("item_id", self.item.clone()),
            ("file_id", self.arquivo.clone()),
            ("integration_id", self.integracao.clone()),
            ("category_id", self.categoria.clone()),
            ("department_id", self.departamento.clone()),
            ("product_id", self.produto.clone()),
            ("conversation_id", self.conversa.clone()),
            ("message_id", self.mensagem.clone()),
            ("attachment_id", self.anexo.clone()),
            ("token_id", self.token_id.clone()),
            ("flow_id", self.fluxo.clone()),
            ("ficha_id", self.ficha.clone()),
            ("run_id", self.execucao.clone()),
            ("kind", "sms".to_owned()),
        ]
    }

    /// Todo id desta org que não deveria aparecer para outra org.
    fn ids(&self) -> Vec<String> {
        let mut ids = vec![self.id.clone()];
        ids.extend(
            self.params()
                .into_iter()
                .filter(|(nome, _)| *nome != "kind")
                .map(|(_, valor)| valor),
        );
        ids
    }

    /// Caminho do modelo com o `org_id` e os ids desta org.
    fn caminho(&self, modelo: &str, org_id: &str) -> String {
        // `{number}` é a revisão 1 do fluxo da vítima (publicada na montagem).
        let mut caminho = modelo.replace("{org_id}", org_id).replace("{number}", "1");
        for (nome, valor) in self.params() {
            caminho = caminho.replace(&format!("{{{nome}}}"), &valor);
        }
        caminho
    }
}

/// Monta uma org completa pela própria API (token global para criar org e token; token da org
/// para o resto).
async fn montar_org(s: &Servidor, slug: &str, marcador: &'static str) -> Org {
    let org = s
        .criar(
            "/admin/v1/orgs",
            TOKEN_GLOBAL,
            json!({"name": format!("Org {marcador}"), "slug": slug}),
            StatusCode::CREATED,
        )
        .await;
    let id = org["id"].as_str().unwrap().to_owned();
    let token_criado = s
        .criar(
            &format!("/admin/v1/orgs/{id}/tokens"),
            TOKEN_GLOBAL,
            json!({"label": format!("token {marcador}")}),
            StatusCode::CREATED,
        )
        .await;
    let token = token_criado["value"].as_str().unwrap().to_owned();
    let token_id = token_criado["token"]["id"].as_str().unwrap().to_owned();
    let raiz = format!("/admin/v1/orgs/{id}");

    let agente = s
        .criar(
            &format!("{raiz}/agents"),
            &token,
            json!({"name": format!("Agente {marcador}"), "slug": format!("agente-{slug}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    s.criar(
        &format!("{raiz}/agents/{agente}/instructions"),
        &token,
        json!({"structured": {"tools": FERRAMENTAS.iter().map(|nome| json!({"name": nome, "enabled": true})).collect::<Vec<_>>()}}),
        StatusCode::CREATED,
    )
    .await;
    let categoria = s
        .criar(
            &format!("{raiz}/categories"),
            &token,
            json!({"name": format!("Categoria {marcador}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let departamento = s
        .criar(
            &format!("{raiz}/departments"),
            &token,
            json!({"name": format!("Departamento {marcador}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let produto = s
        .criar(
            &format!("{raiz}/products"),
            &token,
            json!({"name": format!("Produto {marcador}"), "price_cents": 12345, "category_id": categoria}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let grupo = s
        .criar(
            &format!("{raiz}/knowledge"),
            &token,
            json!({"name": format!("Grupo {marcador}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let item = s
        .criar(
            &format!("{raiz}/knowledge/{grupo}/items"),
            &token,
            json!({"title": format!("Item {marcador}"), "content": format!("Conteúdo sigiloso {marcador}")}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    s.criar(
        &format!("{raiz}/agents/{agente}/knowledge/{grupo}"),
        &token,
        json!({}),
        StatusCode::OK,
    )
    .await;
    let arquivo = s
        .chamar(
            Method::POST,
            &format!("{raiz}/agents/{agente}/files?filename=arquivo-{marcador}.png"),
            &token,
            Corpo::Arquivo(png().to_vec()),
            None,
        )
        .await;
    assert_eq!(
        arquivo.status,
        StatusCode::CREATED,
        "arquivo: {}",
        arquivo.texto
    );
    let arquivo = arquivo.json["id"].as_str().unwrap().to_owned();
    let integracao = s
        .criar(
            &format!("{raiz}/agents/{agente}/integrations"),
            &token,
            json!({"kind": "sms", "status": "active"}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conversa = s
        .chamar(
            Method::POST,
            &format!("{raiz}/agents/{agente}/test-chat"),
            &token,
            Corpo::Nenhum,
            None,
        )
        .await;
    assert_eq!(
        conversa.status,
        StatusCode::CREATED,
        "conversa: {}",
        conversa.texto
    );
    let conversa_id: ConversationId = conversa.json["id"].as_str().unwrap().parse().unwrap();
    let conversa = conversa_id.to_string();
    // Mensagem do cliente com um anexo (HR-46b): o blob é o mesmo do arquivo da biblioteca, que a
    // subida acima já gravou na pasta da org, então o dono lê o conteúdo e a outra org não.
    let org_id: OrgId = id.parse().unwrap();
    let mensagem = s
        .store
        .messages()
        .gravar_com_anexos(
            org_id,
            NewMessage {
                conversation_id: conversa_id,
                direction: Direction::Inbound,
                role: Role::User,
                texto: format!("anexo {marcador}"),
                attachments_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
                origin: poph_store::Origin::Agente,
                author_name: None,
            },
            &[NewChannelAttachment {
                name: format!("arquivo-{marcador}.png"),
                mime: "image/png".to_owned(),
                size_bytes: png().len() as u64,
                sha256: poph_files::blob::sha256_hex(png()),
                blob_path: format!("{id}/{}", poph_files::blob::sha256_hex(png())),
                extracted_text: None,
                extraction_status: "skipped",
            }],
        )
        .expect("mensagem com anexo");
    let anexo = s
        .store
        .messages()
        .anexos(org_id, mensagem.id)
        .expect("anexos da mensagem")
        .remove(0)
        .id
        .to_string();

    // FX-27/29/30: a org vítima tem um fluxo publicado (revisão 1), liberado ao agente, e uma
    // execução aberta na Ficha da conversa de teste (a conversa cria a Ficha). O ataque precisa de
    // ids reais: ids inventados não provam que a checagem de org acontece antes da busca.
    let fluxo = s
        .criar(
            &format!("{raiz}/flows"),
            &token,
            json!({"source": FLUXO_DA_VITIMA, "publish": true}),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let versao_antes = s
        .chamar(
            Method::GET,
            &format!("{raiz}/agents/{agente}"),
            &token,
            Corpo::Nenhum,
            None,
        )
        .await
        .json["version"]
        .as_u64()
        .unwrap();
    let liberado = s
        .chamar(
            Method::PUT,
            &format!("{raiz}/agents/{agente}/flows"),
            &token,
            Corpo::Json(json!({"flow_ids": [fluxo]})),
            Some(versao_antes),
        )
        .await;
    assert_eq!(
        liberado.status,
        StatusCode::OK,
        "liberar fluxo: {}",
        liberado.texto
    );
    let versao_agente = liberado.json["version"].as_u64().unwrap();
    let ficha = s
        .chamar(
            Method::GET,
            &format!("{raiz}/fichas"),
            &token,
            Corpo::Nenhum,
            None,
        )
        .await
        .json["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let versao_ficha = s
        .chamar(
            Method::GET,
            &format!("{raiz}/fichas/{ficha}"),
            &token,
            Corpo::Nenhum,
            None,
        )
        .await
        .json["version"]
        .as_u64()
        .unwrap();
    let abertura = s
        .chamar(
            Method::POST,
            &format!("{raiz}/fichas/{ficha}/flow-runs"),
            &token,
            Corpo::Json(json!({"fluxo_id": fluxo, "chave_idempotencia": "montagem-vitima"})),
            Some(versao_ficha),
        )
        .await;
    assert_eq!(
        abertura.status,
        StatusCode::CREATED,
        "abrir fluxo: {}",
        abertura.texto
    );
    let execucao = abertura.json["id"].as_str().unwrap().to_owned();

    Org {
        id,
        token,
        token_id,
        agente,
        grupo,
        item,
        arquivo,
        integracao,
        categoria,
        departamento,
        produto,
        conversa,
        fluxo,
        versao_agente,
        ficha,
        execucao,
        mensagem: mensagem.id.to_string(),
        anexo,
        marcador,
    }
}

/// Tipo do corpo de cada caso (o que a OpenAPI pede para a operação).
#[derive(Clone, Copy)]
enum Tipo {
    Vazio,
    Json(&'static str),
    Multipart,
    Arquivo,
}

/// Fluxo mínimo e válido que a org vítima publica (revisão 1): a suíte ataca ids reais dele.
const FLUXO_DA_VITIMA: &str = r#"fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
        nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
        exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    campos = [
        campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco"], vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

#[derive(Clone)]
struct Caso {
    metodo: &'static str,
    modelo: String,
    tipo: Tipo,
}

fn caso(metodo: &'static str, modelo: &str, tipo: Tipo) -> Caso {
    Caso {
        metodo,
        modelo: modelo.to_owned(),
        tipo,
    }
}

/// Um caso para cada operação de tenant publicada na OpenAPI. Para criar a cobertura, quem
/// acrescenta rota acrescenta caso aqui: o teste reprova sem ele.
fn casos() -> Vec<Caso> {
    use Tipo::*;
    const ORG: &str = "/admin/v1/orgs/{org_id}";
    const AG: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}";
    const AGF: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}/flows";
    const KN: &str = "/admin/v1/orgs/{org_id}/knowledge/{group_id}";
    const IT: &str = "/admin/v1/orgs/{org_id}/knowledge/{group_id}/items/{item_id}";
    const IG: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}";
    const WA: &str =
        "/admin/v1/orgs/{org_id}/agents/{agent_id}/integrations/{integration_id}/whatsapp";
    const TC: &str = "/admin/v1/orgs/{org_id}/agents/{agent_id}/test-chat";
    const CV: &str = "/admin/v1/orgs/{org_id}/conversations/{conversation_id}";
    const MX: &str = "/admin/v1/orgs/{org_id}/matrix";
    const MXESP: &str = "/admin/v1/orgs/{org_id}/matrix/spaces";
    const FL: &str = "/admin/v1/orgs/{org_id}/flows/{flow_id}";
    const FI: &str = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}";
    const FR: &str = "/admin/v1/orgs/{org_id}/fichas/{ficha_id}/flow-runs/{run_id}";
    vec![
        caso("get", "/admin/v1/orgs", Vazio),
        caso("post", "/admin/v1/orgs", Json(r#"{"name":"Invadido"}"#)),
        caso("get", ORG, Vazio),
        caso("patch", ORG, Json(r#"{"name":"Invadido"}"#)),
        caso("delete", ORG, Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/agents", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/agents",
            Json(r#"{"name":"Invadido","slug":"invadido"}"#),
        ),
        caso("get", AG, Vazio),
        caso("patch", AG, Json(r#"{"description":"Invadido"}"#)),
        caso("delete", AG, Vazio),
        caso("get", AGF, Vazio),
        caso("put", AGF, Json(r#"{"flow_ids":[]}"#)),
        caso("get", &format!("{AG}/instructions"), Vazio),
        caso(
            "post",
            &format!("{AG}/instructions"),
            Json(r#"{"markdown":"Invadido"}"#),
        ),
        caso("get", &format!("{AG}/instructions/current"), Vazio),
        caso(
            "patch",
            &format!("{AG}/instructions/current"),
            Json(r#"{"markdown":"Invadido"}"#),
        ),
        caso("delete", &format!("{AG}/instructions/current"), Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/knowledge", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/knowledge",
            Json(r#"{"name":"Invadido"}"#),
        ),
        caso("get", KN, Vazio),
        caso("patch", KN, Json(r#"{"name":"Invadido"}"#)),
        caso("delete", KN, Vazio),
        caso("get", &format!("{KN}/items"), Vazio),
        caso(
            "post",
            &format!("{KN}/items"),
            Json(r#"{"title":"Invadido","content":"Invadido"}"#),
        ),
        caso("get", IT, Vazio),
        caso("patch", IT, Json(r#"{"title":"Invadido"}"#)),
        caso("delete", IT, Vazio),
        caso("post", &format!("{AG}/knowledge/{{group_id}}"), Vazio),
        caso("delete", &format!("{AG}/knowledge/{{group_id}}"), Vazio),
        caso("post", &format!("{AG}/knowledge/reindex"), Vazio),
        caso("get", &format!("{AG}/integrations"), Vazio),
        caso(
            "post",
            &format!("{AG}/integrations"),
            Json(r#"{"kind":"sms","status":"active"}"#),
        ),
        caso("get", IG, Vazio),
        caso("patch", IG, Json(r#"{"status":"inactive"}"#)),
        caso("delete", IG, Vazio),
        caso("post", &format!("{WA}/qrcode"), Vazio),
        caso("get", &format!("{WA}/state"), Vazio),
        caso("post", &format!("{WA}/disconnect"), Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/backup", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/backup/import",
            Json(
                r#"{"formato":"poph-backup-org","versao_formato":1,"org":{},"tabelas":{},"blobs":[]}"#,
            ),
        ),
        caso("get", "/admin/v1/orgs/{org_id}/manifests", Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/manifests/{kind}", Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/tokens", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/tokens",
            Json(r#"{"label":"Invadido"}"#),
        ),
        caso("get", "/admin/v1/orgs/{org_id}/tokens/{token_id}", Vazio),
        caso("delete", "/admin/v1/orgs/{org_id}/tokens/{token_id}", Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/categories", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/categories",
            Json(r#"{"name":"Invadido"}"#),
        ),
        caso(
            "get",
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            Vazio,
        ),
        caso(
            "patch",
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            Json(r#"{"name":"Invadido"}"#),
        ),
        caso(
            "delete",
            "/admin/v1/orgs/{org_id}/categories/{category_id}",
            Vazio,
        ),
        caso("get", "/admin/v1/orgs/{org_id}/conversations", Vazio),
        caso("get", CV, Vazio),
        caso("get", &format!("{CV}/messages"), Vazio),
        caso("get", &format!("{CV}/turns"), Vazio),
        caso("post", &format!("{CV}/reopen"), Vazio),
        caso("post", &format!("{CV}/close"), Vazio),
        caso(
            "post",
            &format!("{CV}/transfer"),
            Json(r#"{"department_id":"018f0000-0000-7000-8000-000000000002","reason":"Invadido"}"#),
        ),
        caso("get", "/admin/v1/orgs/{org_id}/departments", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/departments",
            Json(r#"{"name":"Invadido"}"#),
        ),
        caso(
            "get",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            Vazio,
        ),
        caso(
            "patch",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            Json(r#"{"name":"Invadido"}"#),
        ),
        caso(
            "delete",
            "/admin/v1/orgs/{org_id}/departments/{department_id}",
            Vazio,
        ),
        caso("get", &format!("{AG}/files"), Vazio),
        caso("post", &format!("{AG}/files"), Arquivo),
        caso("get", &format!("{AG}/files/{{file_id}}"), Vazio),
        caso("get", &format!("{AG}/files/{{file_id}}/content"), Vazio),
        caso("delete", &format!("{AG}/files/{{file_id}}"), Vazio),
        caso("get", TC, Vazio),
        caso("post", TC, Vazio),
        caso("get", &format!("{TC}/{{conversation_id}}"), Vazio),
        caso(
            "post",
            &format!("{TC}/{{conversation_id}}/messages"),
            Multipart,
        ),
        caso("post", &format!("{TC}/{{conversation_id}}/reopen"), Vazio),
        caso(
            "get",
            &format!(
                "{TC}/{{conversation_id}}/messages/{{message_id}}/attachments/{{attachment_id}}/content"
            ),
            Vazio,
        ),
        caso("get", "/admin/v1/orgs/{org_id}/products", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/products",
            Json(r#"{"name":"Invadido","price_cents":100}"#),
        ),
        caso(
            "get",
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            Vazio,
        ),
        caso(
            "patch",
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            Json(r#"{"name":"Invadido"}"#),
        ),
        caso(
            "delete",
            "/admin/v1/orgs/{org_id}/products/{product_id}",
            Vazio,
        ),
        // Módulo Matrix (MX-22): estado da org e espaços por departamento.
        caso("get", MX, Vazio),
        caso("put", MX, Json(r#"{"enabled":true}"#)),
        caso("get", MXESP, Vazio),
        caso(
            "put",
            MXESP,
            Json(r#"{"department_id":"00000000-0000-0000-0000-000000000000"}"#),
        ),
        caso("delete", &format!("{MXESP}/{{department_id}}"), Vazio),
        // FX-27: API de fluxos.
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/flows/validate",
            Json(r#"{"source":"Invadido"}"#),
        ),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/flows/format",
            Json(r#"{"source":"Invadido"}"#),
        ),
        caso("get", "/admin/v1/orgs/{org_id}/flows", Vazio),
        caso(
            "post",
            "/admin/v1/orgs/{org_id}/flows",
            Json(r#"{"source":"Invadido"}"#),
        ),
        caso("get", FL, Vazio),
        caso("patch", FL, Json(r#"{"name":"Invadido"}"#)),
        caso("delete", FL, Vazio),
        caso("post", &format!("{FL}/publish"), Json(r#"{}"#)),
        caso("get", &format!("{FL}/revisions"), Vazio),
        caso("get", &format!("{FL}/revisions/{{number}}"), Vazio),
        caso("get", &format!("{FL}/diff"), Vazio),
        caso("get", &format!("{FL}/preview"), Vazio),
        // FX-29 e FX-30: Fichas (leitura e ações humanas).
        caso("get", "/admin/v1/orgs/{org_id}/fichas", Vazio),
        caso("get", FI, Vazio),
        caso("get", &format!("{FI}/events"), Vazio),
        caso(
            "post",
            &format!("{FI}/flow-runs"),
            Json(
                r#"{"fluxo_id":"018f0000-0000-7000-8000-000000000001","chave_idempotencia":"Invadido"}"#,
            ),
        ),
        caso(
            "post",
            &format!("{FR}/actions"),
            Json(r#"{"acao":"Invadido","chave_idempotencia":"Invadido"}"#),
        ),
        caso(
            "post",
            &format!("{FR}/abandon"),
            Json(r#"{"motivo":"Invadido","chave_idempotencia":"Invadido"}"#),
        ),
        caso("post", &format!("{FI}/close"), Vazio),
        caso(
            "get",
            "/admin/v1/orgs/{org_id}/analytics/conversations",
            Vazio,
        ),
        caso("get", "/admin/v1/orgs/{org_id}/analytics/sla", Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/usage/tokens", Vazio),
        caso(
            "get",
            "/admin/v1/orgs/{org_id}/analytics/operational-summary",
            Vazio,
        ),
    ]
}

/// O que a OpenAPI declara sobre uma operação: cabeçalho `If-Match` e parâmetro de consulta `force`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Op {
    if_match: bool,
    force: bool,
}

/// Operações publicadas pelo servidor, por (método, caminho).
fn publicadas(doc: &Value) -> BTreeMap<(String, String), Op> {
    let mut mapa = BTreeMap::new();
    for (caminho, metodos) in doc["paths"].as_object().expect("paths da OpenAPI") {
        for metodo in ["get", "post", "put", "patch", "delete"] {
            let Some(operacao) = metodos.get(metodo) else {
                continue;
            };
            let parametros = operacao["parameters"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let tem = |nome: &str, em: &str| {
                parametros
                    .iter()
                    .any(|p| p["name"] == nome && p["in"] == em)
            };
            mapa.insert(
                (metodo.to_owned(), caminho.clone()),
                Op {
                    if_match: tem("If-Match", "header"),
                    force: tem("force", "query"),
                },
            );
        }
    }
    mapa
}

/// Confere a cobertura: cada operação de tenant publicada precisa de um caso, e cada caso precisa
/// de uma operação publicada. Devolve a lista de divergências (vazia = cobertura completa).
fn divergencias_de_cobertura(
    publicadas: &BTreeMap<(String, String), Op>,
    casos: &[Caso],
) -> Vec<String> {
    let mut erros = Vec::new();
    let tenant: BTreeSet<(String, String)> = publicadas
        .keys()
        .filter(|(_, caminho)| caminho.starts_with("/admin/v1/orgs"))
        .cloned()
        .collect();
    let nao_tenant: BTreeSet<(String, String)> = publicadas
        .keys()
        .filter(|(_, caminho)| !caminho.starts_with("/admin/v1/orgs"))
        .cloned()
        .collect();
    let esperadas_nao_tenant: BTreeSet<(String, String)> = NAO_TENANT
        .iter()
        .map(|(m, c)| ((*m).to_owned(), (*c).to_owned()))
        .collect();
    if nao_tenant != esperadas_nao_tenant {
        erros.push(format!(
            "operações fora de tenant mudaram: publicadas {nao_tenant:?}, esperadas {esperadas_nao_tenant:?}"
        ));
    }
    let com_caso: BTreeSet<(String, String)> = casos
        .iter()
        .map(|c| (c.metodo.to_owned(), c.modelo.clone()))
        .collect();
    for op in tenant.difference(&com_caso) {
        erros.push(format!(
            "operação publicada sem caso na suíte: {} {}",
            op.0, op.1
        ));
    }
    for op in com_caso.difference(&tenant) {
        erros.push(format!(
            "caso sem operação publicada na OpenAPI: {} {}",
            op.0, op.1
        ));
    }
    erros
}

fn metodo_http(metodo: &str) -> Method {
    Method::from_bytes(metodo.to_uppercase().as_bytes()).expect("método HTTP")
}

/// Caminho do caso montado com a org do `org_id` e os ids de `alvo`. Upload de arquivo leva a
/// query `filename` que a OpenAPI exige.
fn caminho_do_caso(caso: &Caso, alvo: &Org, org_id: &str) -> String {
    let caminho = alvo.caminho(&caso.modelo, org_id);
    match caso.tipo {
        Tipo::Arquivo => format!("{caminho}?filename=invadido.png"),
        _ => caminho,
    }
}

/// Corpo do pedido para o caso (ids da vítima já estão no caminho).
fn corpo_do_caso(tipo: Tipo) -> Corpo {
    match tipo {
        Tipo::Vazio => Corpo::Nenhum,
        Tipo::Json(texto) => Corpo::Json(serde_json::from_str(texto).expect("JSON do caso")),
        Tipo::Multipart => Corpo::Multipart("Invadido".to_owned()),
        Tipo::Arquivo => Corpo::Arquivo(b"invadido".to_vec()),
    }
}

/// Se a resposta vaza dado de `alvo`: marcador (nome) sempre; id só quando a resposta é sucesso
/// (um `404` pode ecoar o id que o próprio cliente mandou, o que não é dado).
fn vazamento(r: &Resposta, alvo: &Org) -> Option<String> {
    if r.texto.contains(alvo.marcador) {
        return Some(format!(
            "marcador `{}` da outra org aparece na resposta",
            alvo.marcador
        ));
    }
    if r.status.is_success() {
        for id in alvo.ids() {
            if r.texto.contains(&id) {
                return Some(format!(
                    "id `{id}` da outra org aparece numa resposta {}",
                    r.status
                ));
            }
        }
    }
    None
}

/// `404` de "rota não existe" (o caminho montado não bate com o roteador) ou `405`.
fn rota_nao_existe(r: &Resposta) -> bool {
    r.status == StatusCode::METHOD_NOT_ALLOWED
        || (r.status == StatusCode::NOT_FOUND && r.json["details"]["path"].is_string())
}

/// Status esperado do controle (o dono lendo o próprio recurso).
fn controle_esperado(metodo: &str, modelo: &str) -> Option<u16> {
    if metodo != "get" {
        return None;
    }
    if modelo.ends_with("/whatsapp/state") {
        // Integração de tipo `sms`: a rota responde que ela não é WhatsApp, o que prova o caminho.
        return Some(409);
    }
    if modelo.contains("/manifests/") {
        // Tipo sem plugin declarado: resposta de "não encontrado" do próprio manifesto.
        return Some(404);
    }
    if modelo.ends_with("/analytics/sla") {
        return Some(422);
    }
    if modelo.ends_with("/analytics/conversations") {
        return Some(422);
    }
    if modelo.ends_with("/usage/tokens") {
        return Some(422);
    }
    if modelo.ends_with("/analytics/operational-summary") {
        return Some(422);
    }
    Some(200)
}

/// Resultado de verificar uma operação: falhas encontradas e pedidos feitos.
struct Veredicto {
    falhas: Vec<String>,
    pedidos: usize,
}

/// Acrescenta uma query ao caminho (`?` ou `&`, conforme já tenha query).
fn com_query(caminho: String, query: &str) -> String {
    if query.is_empty() {
        caminho
    } else if caminho.contains('?') {
        format!("{caminho}&{query}")
    } else {
        format!("{caminho}?{query}")
    }
}

/// Operação com um id da vítima no caminho (o `kind` não conta: é o tipo de plugin).
fn tem_id_da_vitima(modelo: &str, vitima: &Org) -> bool {
    vitima
        .params()
        .iter()
        .any(|(nome, _)| *nome != "kind" && modelo.contains(&format!("{{{nome}}}")))
}

/// Verifica uma operação nas duas direções. Cada pedido é real; nada é pulado.
async fn verificar_operacao(s: &Servidor, caso: &Caso, op: Op, a: &Org, b: &Org) -> Veredicto {
    let mut falhas = Vec::new();
    let mut pedidos = 0usize;
    let metodo = metodo_http(caso.metodo);
    let rotulo = format!("{} {}", caso.metodo.to_uppercase(), caso.modelo);
    let if_match = op.if_match.then_some(1u64);
    // DELETE com `force` no OpenAPI é atacado também com `?force=true`, que é a variante que age.
    let variantes: Vec<&str> = if op.force && caso.metodo == "delete" {
        vec!["", "force=true"]
    } else {
        vec![""]
    };

    if !caso.modelo.contains("{org_id}") {
        // Operações de todas as orgs (listar e criar): o token de org não enxerga nem cria.
        if caso.metodo == "get" {
            let r = s
                .chamar(metodo.clone(), &caso.modelo, &a.token, Corpo::Nenhum, None)
                .await;
            pedidos += 1;
            if r.status != StatusCode::OK {
                falhas.push(format!(
                    "{rotulo} (token de A): esperado 200, veio {}",
                    r.status
                ));
            }
            let ids: Vec<String> = r.json["items"]
                .as_array()
                .map(|itens| {
                    itens
                        .iter()
                        .filter_map(|i| i["id"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if ids != vec![a.id.clone()] {
                falhas.push(format!(
                    "{rotulo}: token de A deveria ver só a própria org, viu {ids:?}"
                ));
            }
            if let Some(motivo) = vazamento(&r, b) {
                falhas.push(format!("{rotulo}: {motivo}"));
            }
        } else {
            let r = s
                .chamar(
                    metodo.clone(),
                    &caso.modelo,
                    &a.token,
                    corpo_do_caso(caso.tipo),
                    None,
                )
                .await;
            pedidos += 1;
            if r.status != StatusCode::FORBIDDEN {
                falhas.push(format!(
                    "{rotulo} (token de org): esperado 403, veio {} {}",
                    r.status, r.texto
                ));
            }
        }
        return Veredicto { falhas, pedidos };
    }

    // 1) Sem token: 401 em qualquer operação, nas duas orgs.
    for alvo in [b, a] {
        let caminho = caminho_do_caso(caso, alvo, &alvo.id);
        let r = s
            .chamar(metodo.clone(), &caminho, "", corpo_do_caso(caso.tipo), None)
            .await;
        pedidos += 1;
        if r.status != StatusCode::UNAUTHORIZED {
            falhas.push(format!(
                "{rotulo} sem token: esperado 401, veio {}",
                r.status
            ));
        }
    }

    // 2) Direções (atacante, vítima).
    for (atacante, vitima) in [(a, b), (b, a)] {
        for query in &variantes {
            // 2a) Org da vítima no caminho, ids da vítima, token do atacante: 403.
            let caminho = com_query(caminho_do_caso(caso, vitima, &vitima.id), query);
            let r = s
                .chamar(
                    metodo.clone(),
                    &caminho,
                    &atacante.token,
                    corpo_do_caso(caso.tipo),
                    if_match,
                )
                .await;
            pedidos += 1;
            if r.status != StatusCode::FORBIDDEN {
                falhas.push(format!(
                    "{rotulo} [{query}] [atacante {}, vítima {}, org da vítima]: esperado 403, veio {} {}",
                    atacante.marcador, vitima.marcador, r.status, r.texto
                ));
            }
            if let Some(motivo) = vazamento(&r, vitima) {
                falhas.push(format!("{rotulo} [{query}] [org da vítima]: {motivo}"));
            }

            // 2b) Org do próprio atacante no caminho, ids da vítima: 403, 404 ou 409 (conflito
            //     genérico, que não diz se o recurso existe). Nunca sucesso.
            if tem_id_da_vitima(&caso.modelo, vitima) {
                let caminho = com_query(caminho_do_caso(caso, vitima, &atacante.id), query);
                let r = s
                    .chamar(
                        metodo.clone(),
                        &caminho,
                        &atacante.token,
                        corpo_do_caso(caso.tipo),
                        if_match,
                    )
                    .await;
                pedidos += 1;
                if !matches!(r.status.as_u16(), 403 | 404 | 409) {
                    falhas.push(format!(
                        "{rotulo} [{query}] [org própria, ids de {}]: esperado 403/404/409, veio {} {}",
                        vitima.marcador, r.status, r.texto
                    ));
                }
                if rota_nao_existe(&r) {
                    falhas.push(format!(
                        "{rotulo} [{query}]: o caminho montado não existe no roteador"
                    ));
                }
                if let Some(motivo) = vazamento(&r, vitima) {
                    falhas.push(format!(
                        "{rotulo} [{query}] [org própria, ids da vítima]: {motivo}"
                    ));
                }
            }
        }

        // 2c) Lista do próprio atacante, sem id da vítima: nunca traz dado da vítima.
        if caso.metodo == "get" && !tem_id_da_vitima(&caso.modelo, vitima) {
            let caminho = caminho_do_caso(caso, atacante, &atacante.id);
            let r = s
                .chamar(
                    metodo.clone(),
                    &caminho,
                    &atacante.token,
                    Corpo::Nenhum,
                    None,
                )
                .await;
            pedidos += 1;
            if let Some(motivo) = vazamento(&r, vitima) {
                falhas.push(format!(
                    "{rotulo} [lista própria de {}]: {motivo}",
                    atacante.marcador
                ));
            }
        }
    }

    // 3) Controle: o dono lendo o próprio recurso. Só leituras, para não alterar nada. Prova que o
    //    caminho montado existe e que a resposta de sucesso é mesmo a esperada.
    if let Some(esperado) = controle_esperado(caso.metodo, &caso.modelo) {
        let caminho = caminho_do_caso(caso, a, &a.id);
        let r = s
            .chamar(metodo.clone(), &caminho, &a.token, Corpo::Nenhum, None)
            .await;
        pedidos += 1;
        if r.status.as_u16() != esperado {
            falhas.push(format!(
                "{rotulo} [controle do dono]: esperado {esperado}, veio {} {}",
                r.status, r.texto
            ));
        }
        if rota_nao_existe(&r) {
            falhas.push(format!(
                "{rotulo} [controle]: caminho não existe no roteador"
            ));
        }
    }
    Veredicto { falhas, pedidos }
}

/// Se a resposta vaza dado de `alvo` depois de tirar o texto que o próprio atacante mandou
/// (ex.: um nome de departamento que ele digitou e que volta numa mensagem de erro).
fn vazamento_sem_eco(r: &Resposta, alvo: &Org, eco: &str) -> Option<String> {
    let texto = r.texto.replace(eco, "");
    if texto.contains(alvo.marcador) {
        return Some(format!(
            "marcador `{}` da outra org aparece na resposta",
            alvo.marcador
        ));
    }
    None
}

/// A suíte inteira: cobertura, anônimo, ataque nas duas direções (com `force` quando a OpenAPI
/// declara), controle do dono, cruzamento por corpo e estado antes e depois.
#[tokio::test]
async fn isolamento_gerado_do_openapi() {
    let s = Servidor::novo("matriz", |store, config, _| {
        Estado::novo(Arc::clone(store), config.clone()).expect("estado do servidor")
    })
    .await;
    let doc = s
        .chamar(
            Method::GET,
            "/admin/v1/openapi.json",
            "",
            Corpo::Nenhum,
            None,
        )
        .await
        .json;
    let publicadas = publicadas(&doc);
    let casos = casos();
    let divergencias = divergencias_de_cobertura(&publicadas, &casos);
    assert!(
        divergencias.is_empty(),
        "cobertura da OpenAPI incompleta:\n{}",
        divergencias.join("\n")
    );

    let a = montar_org(&s, "isola-a", MARCADOR_A).await;
    let b = montar_org(&s, "isola-b", MARCADOR_B).await;
    let antes_a = s.foto(&a).await;
    let antes_b = s.foto(&b).await;
    let auditoria_antes_a = s.auditoria(&a);
    let auditoria_antes_b = s.auditoria(&b);
    assert!(
        !auditoria_antes_a.is_empty() && !auditoria_antes_b.is_empty(),
        "fixture sem auditoria: a comparação de auditoria não provaria nada"
    );
    // Os recursos da vítima existem de verdade: o dono lê cada um com 200. Sem isso, um 404 dos
    // ataques não provaria nada.
    for (nome, fotos) in [("A", &antes_a), ("B", &antes_b)] {
        for linha in fotos.iter() {
            if !linha.contains(" -> 200 ") {
                panic!("fixture da org {nome} incompleta, o dono não lê: {linha}");
            }
        }
    }

    // Casos em paralelo (a verificação de token é cara; o banco aceita leituras concorrentes).
    let s_ref = &s;
    let publicadas_ref = &publicadas;
    let veredictos: Vec<Veredicto> = futures::stream::iter(casos.iter())
        .map(|caso| {
            let op = publicadas_ref[&(caso.metodo.to_owned(), caso.modelo.clone())];
            verificar_operacao(s_ref, caso, op, &a, &b)
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut falhas: Vec<String> = Vec::new();
    let mut pedidos = 0usize;
    for v in veredictos {
        falhas.extend(v.falhas);
        pedidos += v.pedidos;
    }

    // 4) Cruzamento por corpo: a org própria não pode ligar um recurso seu a um id/nome da outra.
    let cruzadas: Vec<(&str, String, Value, Option<u64>, String)> = vec![
        (
            "POST",
            format!("/admin/v1/orgs/{}/products", a.id),
            json!({"name": "Cruzado", "price_cents": 100, "category_id": b.categoria}),
            None,
            String::new(),
        ),
        (
            "POST",
            format!("/admin/v1/orgs/{}/agents", a.id),
            json!({"name": "Cruzado", "slug": "cruzado-categoria", "product_category_ids": [b.categoria]}),
            None,
            String::new(),
        ),
        (
            "POST",
            format!("/admin/v1/orgs/{}/agents", a.id),
            json!({"name": "Cruzado", "slug": "cruzado-departamento", "department": format!("Departamento {MARCADOR_B}")}),
            None,
            format!("Departamento {MARCADOR_B}"),
        ),
        (
            "PATCH",
            format!("/admin/v1/orgs/{}/agents/{}", a.id, a.agente),
            json!({"product_category_ids": [b.categoria]}),
            Some(a.versao_agente),
            String::new(),
        ),
    ];
    for (metodo, caminho, corpo, if_match, eco) in cruzadas {
        let r = s
            .chamar(
                metodo_http(metodo),
                &caminho,
                &a.token,
                Corpo::Json(corpo.clone()),
                if_match,
            )
            .await;
        pedidos += 1;
        if !matches!(r.status.as_u16(), 400 | 404 | 409 | 422) {
            falhas.push(format!(
                "cruzamento {metodo} {caminho} com {corpo}: esperado erro de referência (400/404/409/422), veio {} {}",
                r.status, r.texto
            ));
        }
        let vazou = if eco.is_empty() {
            vazamento(&r, &b)
        } else {
            vazamento_sem_eco(&r, &b, &eco)
        };
        if let Some(motivo) = vazou {
            falhas.push(format!("cruzamento {metodo} {caminho}: {motivo}"));
        }
    }

    let caminho_conversa_b = format!("/admin/v1/orgs/{}/conversations/{}", b.id, b.conversa);
    let conversa_antes = s
        .chamar(
            Method::GET,
            &caminho_conversa_b,
            &b.token,
            Corpo::Nenhum,
            None,
        )
        .await
        .json;
    let transferencia = s
        .chamar(
            Method::POST,
            &format!(
                "/admin/v1/orgs/{}/conversations/{}/transfer",
                a.id, b.conversa
            ),
            &a.token,
            Corpo::Json(json!({"department_id": a.departamento, "reason": "atravessado"})),
            None,
        )
        .await;
    pedidos += 1;
    if transferencia.status != StatusCode::NOT_FOUND {
        falhas.push(format!(
            "transferência atravessada (conversa de {}, departamento de {}): esperado 404, veio {} {}",
            b.marcador, a.marcador, transferencia.status, transferencia.texto
        ));
    }
    let conversa_depois = s
        .chamar(
            Method::GET,
            &caminho_conversa_b,
            &b.token,
            Corpo::Nenhum,
            None,
        )
        .await
        .json;
    pedidos += 1;
    if conversa_antes != conversa_depois {
        falhas.push(format!(
            "conversa de {} mudou com a transferência atravessada:\n  antes: {conversa_antes}\n  depois: {conversa_depois}",
            b.marcador
        ));
    }

    // 5) Estado: nada da vítima mudou, lido pelo dono de cada org.
    let depois_a = s.foto(&a).await;
    let depois_b = s.foto(&b).await;
    for (antes, depois, nome) in [(&antes_a, &depois_a, "A"), (&antes_b, &depois_b, "B")] {
        if antes.len() != depois.len() {
            falhas.push(format!("estado da org {nome}: número de leituras mudou"));
        }
        for (x, y) in antes.iter().zip(depois.iter()) {
            if x != y {
                falhas.push(format!(
                    "estado da org {nome} mudou após os ataques:\n  antes: {x}\n  depois: {y}"
                ));
            }
        }
    }
    for (antes, depois, nome) in [
        (&auditoria_antes_a, &s.auditoria(&a), "A"),
        (&auditoria_antes_b, &s.auditoria(&b), "B"),
    ] {
        if antes != depois {
            falhas.push(format!(
                "auditoria da org {nome} mudou após os ataques: {} registro(s) antes, {} depois",
                antes.len(),
                depois.len()
            ));
        }
    }

    println!(
        "operações exercitadas: {}; pedidos de ataque, controle e cruzamento: {pedidos}",
        casos.len()
    );
    assert!(
        falhas.is_empty(),
        "{} falha(s) de isolamento:\n{}",
        falhas.len(),
        falhas.join("\n")
    );
}

/// O verificador de cobertura tem de reprovar operação sem caso e caso sem operação. Prova que a
/// reprovação da suíte principal não depende de sorte.
#[test]
fn caso_faltando_reprova_a_cobertura() {
    // OpenAPI com: uma operação sem caso (POST agents), uma rota nova sem caso (rota-nova), e
    // as três operações fora de tenant publicadas (que a suíte aceita).
    let doc = json!({
        "paths": {
            "/admin/v1/orgs/{org_id}/agents": {"get": {}, "post": {}},
            "/admin/v1/orgs/{org_id}/rota-nova": {
                "get": {"parameters": [{"name": "If-Match", "in": "header"}]}
            },
            "/healthz": {"get": {}},
            "/metrics": {"get": {}},
            "/admin/v1/cluster/reload": {"post": {}}
        }
    });
    let publicadas = publicadas(&doc);
    assert_eq!(
        publicadas.get(&(
            "get".to_owned(),
            "/admin/v1/orgs/{org_id}/rota-nova".to_owned()
        )),
        Some(&Op {
            if_match: true,
            force: false
        }),
        "If-Match declarado na OpenAPI tem de ser lido"
    );
    let casos = vec![
        caso("get", "/admin/v1/orgs/{org_id}/agents", Tipo::Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/fantasma", Tipo::Vazio),
    ];
    let erros = divergencias_de_cobertura(&publicadas, &casos);
    assert!(
        erros
            .iter()
            .any(|e| e.contains("sem caso") && e.contains("post /admin/v1/orgs/{org_id}/agents")),
        "operação publicada sem caso precisa reprovar: {erros:?}"
    );
    assert!(
        erros
            .iter()
            .any(|e| e.contains("sem caso") && e.contains("rota-nova")),
        "rota nova sem caso precisa reprovar: {erros:?}"
    );
    assert!(
        erros
            .iter()
            .any(|e| e.contains("caso sem operação") && e.contains("fantasma")),
        "caso sem operação precisa reprovar: {erros:?}"
    );
    assert_eq!(
        erros.len(),
        3,
        "exatamente as três divergências esperadas: {erros:?}"
    );

    // Cobertura completa: nenhuma divergência.
    let completos = vec![
        caso("get", "/admin/v1/orgs/{org_id}/agents", Tipo::Vazio),
        caso("post", "/admin/v1/orgs/{org_id}/agents", Tipo::Vazio),
        caso("get", "/admin/v1/orgs/{org_id}/rota-nova", Tipo::Vazio),
    ];
    assert!(
        divergencias_de_cobertura(&publicadas, &completos).is_empty(),
        "cobertura completa não pode dar divergência"
    );
}

// ─── Ferramentas do LLM e base de conhecimento: turno real, modelo falso ───

/// Provedor que repassa ao falso. Os roteiros precisam dos ids que a API gera, então o falso só
/// é ligado depois que o cenário existe.
struct Repassa(std::sync::OnceLock<Arc<ProvedorFake>>);

impl LlmProvider for Repassa {
    fn stream<'a>(&'a self, pedido: Pedido) -> BoxFuture<'a, Result<FluxoLlm, ErroLlm>> {
        Box::pin(async move {
            match self.0.get() {
                Some(falso) => falso.stream(pedido).await,
                None => Err(ErroLlm::Configuracao(
                    "provedor falso sem roteiro".to_owned(),
                )),
            }
        })
    }
}

/// Servidor com o turno montado como o `poph serve` monta (ferramentas de base, produto, arquivo
/// e carrinho), com índice de busca de um embedder falso (determinístico, sem modelo instalado).
async fn subir_com_turno(
    rotulo: &str,
) -> (
    Servidor,
    Arc<Repassa>,
    Arc<BaseConhecimento>,
    Arc<CatalogoProdutos>,
) {
    let repassa = Arc::new(Repassa(std::sync::OnceLock::new()));
    let provedor: Arc<dyn LlmProvider> = Arc::clone(&repassa) as Arc<dyn LlmProvider>;
    let mut saida: Option<(Arc<BaseConhecimento>, Arc<CatalogoProdutos>)> = None;
    let servidor = Servidor::novo(rotulo, |store, config, pasta| {
        let blobs = pasta.join("dados").join("blobs");
        let estado = Estado::novo(Arc::clone(store), config.clone())
            .expect("estado do servidor")
            .com_arquivos(ConfigArquivos::padrao(blobs.clone()));
        let config_quente = ConfigQuente::nova(
            Arc::clone(store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let motor = Motor::novo(
            Arc::clone(store),
            config_quente,
            limitador,
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_modelo_padrao("modelo-teste")
        .com_arquivos(ConfigArquivos::padrao(blobs));
        let indice = Indice::abrir(
            store.rag_path(),
            Arc::new(EmbedderFake::new()),
            ConfigIndice::default(),
        )
        .expect("índice de teste")
        .0;
        let indice = Arc::new(indice);
        let base = BaseConhecimento::nova(Arc::clone(store), Arc::clone(&indice));
        let catalogo = CatalogoProdutos::novo(Arc::clone(store), indice);
        let busca: Arc<dyn BuscadorBase> = Arc::clone(&base) as Arc<dyn BuscadorBase>;
        let registro =
            ConsultarBase::registro(Arc::clone(motor.config_quente()), Arc::clone(&busca));
        let registro = registro.com(BuscarProduto::nova(
            Arc::clone(motor.config_quente()),
            Arc::clone(&catalogo) as Arc<dyn BuscadorProdutos>,
        ));
        let registro = poph_agent::carrinho::registrar(registro, Arc::clone(store));
        let registro = poph_agent::biblioteca::registrar(registro, Arc::clone(store));
        let motor = Arc::new(motor.com_ferramentas(Arc::new(registro)).com_busca(busca));
        let processador = Arc::new(ProcessadorTurno::novo(Arc::clone(&motor)));
        let atores = Atores::novo(
            processador.clone(),
            OpcoesAtores::default(),
            MetricasMemoria::proprio(),
        );
        let fluxo = Fluxo::nova(Arc::clone(store), atores, OpcoesFluxo::default());
        processador.ligar_fluxo(fluxo.clone());
        saida = Some((base, catalogo));
        estado
            .com_motor(motor)
            .com_turno(fluxo, Duration::from_millis(20))
    })
    .await;
    let (base, catalogo) = saida.expect("base e catálogo montados");
    (servidor, repassa, base, catalogo)
}

fn chamada(id: &str, nome: &str, argumentos: Value) -> EventoLlm {
    EventoLlm::Ferramenta(ChamadaFerramenta {
        id: id.to_owned(),
        nome: nome.to_owned(),
        argumentos,
    })
}

/// Resultados das ferramentas (texto que o modelo lê), um por chamada: cada pedido repete o
/// histórico, então conta-se pelo id da chamada, mantendo a ordem da primeira aparição.
fn resultados(pedidos: &[Pedido], prefixo: &str) -> Vec<(String, String)> {
    let mut vistos: Vec<(String, String)> = Vec::new();
    for pedido in pedidos {
        for mensagem in &pedido.mensagens {
            if let Mensagem::Resultado { id, conteudo } = mensagem
                && id.starts_with(prefixo)
                && !vistos.iter().any(|(visto, _)| visto == id)
            {
                vistos.push((id.clone(), conteudo.clone()));
            }
        }
    }
    vistos
}

/// Quantos itens a caixa de saída da org tem, em qualquer estado (o console confirma cada item
/// logo depois do envio, então olhar só os pendentes não mostra nada).
fn itens_de_saida(s: &Servidor, org: poph_types::OrgId) -> u64 {
    [
        OutboxState::Pending,
        OutboxState::Sending,
        OutboxState::Sent,
        OutboxState::Failed,
    ]
    .into_iter()
    .map(|estado| {
        s.store
            .outbox()
            .contar_por_estado(org, estado)
            .expect("contagem da caixa de saída")
    })
    .sum()
}

/// Manda uma mensagem de texto ao chat de teste da org e espera o turno terminar (o SSE fecha).
async fn falar(s: &Servidor, org: &Org, texto: &str) -> Resposta {
    let caminho = format!(
        "/admin/v1/orgs/{}/agents/{}/test-chat/{}/messages",
        org.id, org.agente, org.conversa
    );
    s.chamar(
        Method::POST,
        &caminho,
        &org.token,
        Corpo::Multipart(texto.to_owned()),
        None,
    )
    .await
}

fn id<T: std::str::FromStr>(texto: &str) -> T
where
    T::Err: std::fmt::Debug,
{
    texto.parse().expect("id válido")
}

/// O modelo (falso) pede ferramentas com ids e termos da OUTRA org. Nada da outra org pode voltar
/// como resultado nem ser alterado. O controle: a mesma ferramenta, com os ids da própria org,
/// funciona, o que prova que o caminho testado é o caminho que de fato executa.
#[tokio::test]
async fn ferramentas_do_llm_nao_alcancam_outra_org() {
    let (s, repassa, base, catalogo) = subir_com_turno("ferramentas").await;
    let a = montar_org(&s, "ferr-a", MARCADOR_A).await;
    let b = montar_org(&s, "ferr-b", MARCADOR_B).await;
    let (oa, ob): (poph_types::OrgId, poph_types::OrgId) = (id(&a.id), id(&b.id));
    let (aa, ab): (poph_types::AgentId, poph_types::AgentId) = (id(&a.agente), id(&b.agente));
    base.sincronizar_agente(oa, aa)
        .await
        .expect("indexa base de A");
    base.sincronizar_agente(ob, ab)
        .await
        .expect("indexa base de B");
    catalogo
        .sincronizar_agente(oa, aa)
        .await
        .expect("indexa produtos de A");
    catalogo
        .sincronizar_agente(ob, ab)
        .await
        .expect("indexa produtos de B");

    // Turno de A (atacante): tudo com ids/termos de B. Turno de B (controle): os mesmos pedidos
    // com os próprios ids de B.
    let roteiros = vec![
        Roteiro::Eventos(vec![
            chamada("a1", "enviar_arquivo", json!({"id": b.arquivo})),
            EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
        ]),
        Roteiro::Eventos(vec![
            chamada(
                "a2",
                "cart_add_item",
                json!({"produto_id": b.produto, "quantidade": 1}),
            ),
            EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
        ]),
        Roteiro::Eventos(vec![
            chamada("a3", "consultar_base", json!({"consulta": MARCADOR_B})),
            EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
        ]),
        Roteiro::Eventos(vec![
            chamada("a4", "buscar_produto", json!({"termo": MARCADOR_B})),
            EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
        ]),
        Roteiro::Eventos(vec![
            EventoLlm::Texto("Não tenho nada para enviar.".to_owned()),
            EventoLlm::Fim(MotivoFim::Concluido),
        ]),
        Roteiro::Eventos(vec![
            chamada("b1", "enviar_arquivo", json!({"id": b.arquivo})),
            EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
        ]),
        Roteiro::Eventos(vec![
            chamada("b2", "consultar_base", json!({"consulta": MARCADOR_B})),
            EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
        ]),
        Roteiro::Eventos(vec![
            chamada("b3", "buscar_produto", json!({"termo": MARCADOR_B})),
            EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
        ]),
        Roteiro::Eventos(vec![
            EventoLlm::Texto("Pronto.".to_owned()),
            EventoLlm::Fim(MotivoFim::Concluido),
        ]),
    ];
    let falso = Arc::new(ProvedorFake::com_roteiros(roteiros));
    assert!(repassa.0.set(Arc::clone(&falso)).is_ok());

    let mut falhas: Vec<String> = Vec::new();
    let r_a = falar(&s, &a, "Quero o arquivo, o produto e o que diz a base.").await;
    if r_a.status != StatusCode::OK {
        falhas.push(format!(
            "turno de A: esperado 200 (SSE), veio {}: {}",
            r_a.status, r_a.texto
        ));
    }
    let pedidos = falso.recebidos();
    let res_a = resultados(&pedidos, "a");
    println!("resultados das ferramentas no turno de A (atacante):");
    for (id, texto) in &res_a {
        println!("  {id}: {}", texto.lines().next().unwrap_or(""));
    }
    for (id, texto) in &res_a {
        // O que A recebe da própria base (o item de A) é normal; o que não pode é trazer B.
        if texto.contains(MARCADOR_B) {
            falhas.push(format!(
                "ferramenta {id} do turno de A devolveu dado da outra org"
            ));
        }
        if texto.contains("enfileirado para envio") || texto.contains("adicionado") {
            falhas.push(format!(
                "ferramenta {id} do turno de A agiu com id da outra org: {texto}"
            ));
        }
    }
    if res_a.len() != 4 {
        falhas.push(format!(
            "turno de A: esperava 4 resultados de ferramenta, vieram {}",
            res_a.len()
        ));
    }
    let carrinho_a = s
        .store
        .carts()
        .resumo(oa, id(&a.conversa))
        .expect("carrinho de A");
    if !carrinho_a.itens.is_empty() {
        falhas.push(format!(
            "o carrinho de A recebeu item do produto de B: {:?}",
            carrinho_a.itens
        ));
    }
    // A caixa de saída de A: só a resposta de texto do turno (1 item). Arquivo de B não entra.
    let saida_a_antes = itens_de_saida(&s, oa);
    if saida_a_antes != 1 {
        falhas.push(format!(
            "caixa de saída de A deveria ter só a resposta de texto (1 item), tem {saida_a_antes}"
        ));
    }

    // Controle: o turno de B usa os próprios ids e as ferramentas devolvem o dado.
    let r_b = falar(&s, &b, "Quero o arquivo, o produto e o que diz a base.").await;
    if r_b.status != StatusCode::OK {
        falhas.push(format!(
            "turno de B (controle): esperado 200, veio {}",
            r_b.status
        ));
    }
    let pedidos = falso.recebidos();
    let res_b = resultados(&pedidos, "b");
    println!("resultados das ferramentas no turno de B (controle):");
    for (id, texto) in &res_b {
        println!("  {id}: {}", texto.lines().next().unwrap_or(""));
    }
    let controle = [
        (
            "enviar_arquivo acha o arquivo próprio",
            res_b
                .iter()
                .any(|(_, t)| t.contains("enfileirado para envio")),
        ),
        (
            "consultar_base acha o item próprio",
            res_b
                .iter()
                .any(|(_, t)| t.contains(&format!("Conteúdo sigiloso {MARCADOR_B}"))),
        ),
        (
            "buscar_produto acha o produto próprio",
            res_b
                .iter()
                .any(|(_, t)| t.contains(&format!("Produto {MARCADOR_B}"))),
        ),
    ];
    for (nome, ok) in controle {
        if !ok {
            falhas.push(format!(
                "controle falhou ({nome}): sem ele o teste do atacante não prova nada"
            ));
        }
    }
    if itens_de_saida(&s, ob) < 2 {
        falhas.push("controle: o arquivo de B não entrou na caixa de saída de B".to_owned());
    }
    if itens_de_saida(&s, oa) != saida_a_antes {
        falhas.push("a caixa de A mudou com o turno de B".to_owned());
    }

    assert!(
        falhas.is_empty(),
        "{} falha(s):\n{}",
        falhas.len(),
        falhas.join("\n")
    );
}

/// Busca da base e do catálogo: a org e o agente certos acham o próprio dado; trocar a org ou o
/// agente não devolve nada da outra org.
#[tokio::test]
async fn busca_da_base_nao_devolve_trecho_de_outra_org() {
    let (s, _repassa, base, catalogo) = subir_com_turno("rag").await;
    let a = montar_org(&s, "rag-a", MARCADOR_A).await;
    let b = montar_org(&s, "rag-b", MARCADOR_B).await;
    let (oa, ob): (poph_types::OrgId, poph_types::OrgId) = (id(&a.id), id(&b.id));
    let (aa, ab): (poph_types::AgentId, poph_types::AgentId) = (id(&a.agente), id(&b.agente));
    base.sincronizar_agente(oa, aa).await.expect("indexa A");
    base.sincronizar_agente(ob, ab).await.expect("indexa B");
    catalogo
        .sincronizar_agente(oa, aa)
        .await
        .expect("produtos A");
    catalogo
        .sincronizar_agente(ob, ab)
        .await
        .expect("produtos B");

    let params = ParametrosBusca {
        top_k: 10,
        corte: 0.0,
    };
    let mut falhas = Vec::new();

    // Controle: a org e o agente certos acham o próprio item.
    let proprio_b = base
        .buscar(ob, ab, MARCADOR_B.to_owned(), params)
        .await
        .expect("busca B");
    if !proprio_b.iter().any(|t| t.texto.contains(MARCADOR_B)) {
        falhas.push(
            "controle: a busca de B não acha o próprio item (o teste ficaria vazio)".to_owned(),
        );
    }
    let proprio_a = base
        .buscar(oa, aa, MARCADOR_A.to_owned(), params)
        .await
        .expect("busca A");
    if !proprio_a.iter().any(|t| t.texto.contains(MARCADOR_A)) {
        falhas.push("controle: a busca de A não acha o próprio item".to_owned());
    }

    // Atacante: perguntar pelo marcador de B com a org de A, ou com o agente de B, ou com a org de
    // B e o agente de A. Nada de B pode voltar.
    let cenarios = [
        ("org de A, agente de A, marcador de B", oa, aa),
        ("org de A, agente de B", oa, ab),
        ("org de B, agente de A", ob, aa),
    ];
    for (nome, org, agente) in cenarios {
        let trechos = base
            .buscar(org, agente, MARCADOR_B.to_owned(), params)
            .await
            .expect("busca");
        for t in &trechos {
            if t.texto.contains(MARCADOR_B) || t.titulo.contains(MARCADOR_B) {
                falhas.push(format!("{nome}: trecho de B voltou: {}", t.texto));
            }
        }
        if nome == "org de A, agente de B" && !trechos.is_empty() {
            falhas.push(format!(
                "{nome}: agente de B não deveria achar nada sob a org de A"
            ));
        }
    }
    // Produtos: o catálogo com a org de A não acha o produto de B.
    let produtos_a = BuscadorProdutos::buscar(
        catalogo.as_ref(),
        oa,
        aa,
        MARCADOR_B.to_owned(),
        None,
        ParametrosProdutos {
            top_k: 10,
            corte: 0.0,
        },
    )
    .await
    .expect("busca de produtos de A");
    if produtos_a.iter().any(|p| p.nome.contains(MARCADOR_B)) {
        falhas.push("catálogo de A devolveu produto de B".to_owned());
    }
    let produtos_b = BuscadorProdutos::buscar(
        catalogo.as_ref(),
        ob,
        ab,
        MARCADOR_B.to_owned(),
        None,
        ParametrosProdutos {
            top_k: 10,
            corte: 0.0,
        },
    )
    .await
    .expect("busca de produtos de B");
    if !produtos_b.iter().any(|p| p.nome.contains(MARCADOR_B)) {
        falhas.push("controle: o catálogo de B não acha o próprio produto".to_owned());
    }

    assert!(
        falhas.is_empty(),
        "{} falha(s):\n{}",
        falhas.len(),
        falhas.join("\n")
    );
}

// ─── Testes da própria suíte: o detector de vazamento e o de mudança de estado ───

#[test]
fn detector_de_vazamento_so_usa_id_em_sucesso() {
    let org = Org {
        id: "11111111-aaaa".to_owned(),
        token: String::new(),
        token_id: "22222222-bbbb".to_owned(),
        agente: "33333333-cccc".to_owned(),
        grupo: "44444444-dddd".to_owned(),
        item: "55555555-eeee".to_owned(),
        arquivo: "66666666-ffff".to_owned(),
        integracao: "77777777-0000".to_owned(),
        categoria: "88888888-1111".to_owned(),
        departamento: "99999999-2222".to_owned(),
        produto: "aaaaaaaa-3333".to_owned(),
        conversa: "bbbbbbbb-4444".to_owned(),
        fluxo: "eeeeeeee-7777".to_owned(),
        versao_agente: 1,
        ficha: "ffffffff-8888".to_owned(),
        execucao: "00000000-9999".to_owned(),
        mensagem: "cccccccc-5555".to_owned(),
        anexo: "dddddddd-6666".to_owned(),
        marcador: MARCADOR_B,
    };
    let resposta = |status: u16, texto: &str| Resposta {
        status: StatusCode::from_u16(status).unwrap(),
        texto: texto.to_owned(),
        json: Value::Null,
    };
    assert!(
        vazamento(
            &resposta(200, &format!("{{\"nome\":\"{MARCADOR_B}\"}}")),
            &org
        )
        .is_some(),
        "marcador em 200 é vazamento"
    );
    assert!(
        vazamento(&resposta(404, &format!("não existe: {MARCADOR_B}")), &org).is_some(),
        "marcador em 404 também é vazamento"
    );
    assert!(
        vazamento(&resposta(200, "{\"id\":\"33333333-cccc\"}"), &org).is_some(),
        "id da vítima em sucesso é vazamento"
    );
    assert!(
        vazamento(
            &resposta(404, "{\"message\":\"agente 33333333-cccc não existe\"}"),
            &org
        )
        .is_none(),
        "id ecoado num 404 não é dado"
    );
    assert!(
        vazamento(&resposta(403, "proibido"), &org).is_none(),
        "resposta limpa não é vazamento"
    );
}

/// A comparação de estado tem de acusar uma escrita do dono. Se não acusar, o "zero escrita" da
/// suíte principal não vale nada.
#[tokio::test]
async fn comparacao_de_estado_acusa_escrita() {
    let s = Servidor::novo("estado", |store, config, _| {
        Estado::novo(Arc::clone(store), config.clone()).expect("estado do servidor")
    })
    .await;
    let b = montar_org(&s, "estado-b", MARCADOR_B).await;
    let antes = s.foto(&b).await;
    let auditoria_antes = s.auditoria(&b);
    let r = s
        .chamar(
            Method::PATCH,
            &format!("/admin/v1/orgs/{}/agents/{}", b.id, b.agente),
            &b.token,
            Corpo::Json(json!({"description": "mudança legítima do dono"})),
            Some(b.versao_agente),
        )
        .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "o dono pode alterar o próprio agente: {}",
        r.texto
    );
    let depois = s.foto(&b).await;
    assert_ne!(
        antes, depois,
        "a comparação de estado tem de acusar uma escrita do dono"
    );
    assert_ne!(
        auditoria_antes,
        s.auditoria(&b),
        "a comparação de auditoria tem de acusar uma escrita do dono"
    );
}
