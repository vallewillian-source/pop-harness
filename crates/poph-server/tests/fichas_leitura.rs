//! Testes HTTP da leitura das Fichas (FX-29, PRD §7.3 e W6).
//!
//! Servidor de verdade (porta efêmera) sobre banco temporário, com cliente HTTP de verdade. Fichas,
//! execuções e revisões são criadas direto no repositório: a publicação de revisão é a FX-09 e a
//! rota de ações é a FX-30. Nomes, telefones e textos são fictícios.
//!
//! O que fica provado aqui:
//! - filtros `status`, `agente`, `canal` e `desde`, cada um sozinho e combinados; valor inválido → 400;
//! - paginação por cursor: as páginas somadas são a lista inteira, na mesma ordem, sem repetir item;
//! - Ficha de outra org ou inexistente → 404 no detalhe e na linha do tempo, e a lista da org não
//!   traz a Ficha alheia; token de org em caminho de outra org → 403;
//! - eventos em ordem de `seq`, paginados por `seq`;
//! - execução com etapas e campos respondidos aparece dentro do detalhe da Ficha (W6);
//! - Ficha fechada continua legível, com a execução abandonada e o evento de fechamento.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_store::{
    Conversa, ExecutionStart, ExecutionState, FlowTransition, NewConversation, NewRecord, Store,
    StoredExecution, TransitionEvent, TransitionExtras,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    ActionActor, AgentId, EventActor, ExecutionStatus, FlowId, HarnessConfig, OrgId,
    RecordEventKind, RecordId, RevisionId,
};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado nos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-fx29";

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

/// Servidor no ar, com banco e pasta próprios. O `Drop` desliga e apaga a pasta.
struct Cenario {
    base: String,
    cliente: Client,
    pasta: PathBuf,
    store: Arc<Store>,
    desligar: Option<oneshot::Sender<()>>,
}

impl Cenario {
    async fn novo(rotulo: &str) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-fichas-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7482;
        config.admin.tokens_globais = vec![GlobalTokenConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::open(&config).expect("abre o banco"));
        let estado = Estado::novo(Arc::clone(&store), config).expect("estado do servidor");

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
                .timeout(Duration::from_secs(20))
                .build()
                .expect("cliente HTTP"),
            pasta,
            store,
            desligar: Some(desligar),
        };
        cenario.esperar_subir().await;
        cenario
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
                Ok(resposta) if resposta.status().is_success() => return,
                _ if inicio.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                outro => panic!("servidor não subiu: {outro:?}"),
            }
        }
    }

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base))
            .bearer_auth(token);
        if let Some(valor) = corpo {
            pedido = pedido.json(&valor);
        }
        let resposta = pedido.send().await.expect("pedido HTTP");
        let status = resposta.status();
        let texto = resposta.text().await.unwrap_or_default();
        let json = if texto.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&texto).unwrap_or(Value::String(texto))
        };
        (status, json)
    }

    async fn get(&self, caminho: &str, token: &str) -> (StatusCode, Value) {
        self.pedir(Method::GET, caminho, token, None).await
    }

    async fn criar_org(&self, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": format!("Org {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {corpo}");
        corpo["id"].as_str().expect("id da org").to_owned()
    }

    async fn criar_token(&self, org: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": "token de teste" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {corpo}");
        corpo["value"].as_str().expect("valor do token").to_owned()
    }

    async fn criar_agente(&self, org: &str, token: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {corpo}");
        corpo["id"].as_str().expect("id do agente").to_owned()
    }

    /// Conexão direta ao arquivo do banco, só para gravar a revisão publicada (a publicação é a FX-09).
    fn conexao(&self) -> rusqlite::Connection {
        let conn =
            rusqlite::Connection::open(self.store.harness_path()).expect("abre o banco do teste");
        conn.busy_timeout(Duration::from_secs(10))
            .expect("tempo de espera do banco");
        conn
    }

    /// Fluxo com uma revisão publicada, gravado direto (a publicação é a FX-09).
    fn fluxo_publicado(&self, org: &str, slug: &str) -> (FlowId, RevisionId) {
        let conn = self.conexao();
        let fluxo = uuid::Uuid::now_v7().to_string();
        let revisao = uuid::Uuid::now_v7().to_string();
        conn.execute(
            "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                                 revisao_atual, version)
             VALUES (?1, ?2, ?3, ?4, '', 'active', '', 1, 1)",
            rusqlite::params![fluxo, org, slug, format!("Fluxo {slug}")],
        )
        .expect("grava o fluxo");
        conn.execute(
            "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                        definicao_json, schema_versao, publicada_em, publicada_por,
                                        resolucoes_json)
             VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
            rusqlite::params![revisao, org, fluxo],
        )
        .expect("grava a revisão");
        (
            fluxo.parse().expect("id do fluxo"),
            revisao.parse().expect("id da revisão"),
        )
    }

    fn liberar(&self, org: &str, agente: &str, fluxo: FlowId) {
        self.store
            .fluxos()
            .liberar(parse_org(org), parse_agente(agente), fluxo)
            .expect("libera o fluxo ao agente");
    }

    /// Ficha sem conversa, do agente, no canal dado.
    fn ficha_avulsa(&self, org: &str, agente: &str, canal: &str) -> RecordId {
        self.store
            .fichas()
            .criar(
                parse_org(org),
                NewRecord {
                    conversation_id: None,
                    agent_id: parse_agente(agente),
                    canal: canal.to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                },
            )
            .expect("cria a Ficha")
            .id
    }

    /// Conversa nova, com a Ficha que o repositório cria junto. Devolve a conversa e a Ficha.
    fn conversa(&self, org: &str, agente: &str, canal: &str) -> (Conversa, RecordId) {
        let conversa = self
            .store
            .conversas()
            .criar(
                parse_org(org),
                NewConversation {
                    agent_id: parse_agente(agente),
                    integration_id: None,
                    canal: canal.to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                    target_department: None,
                },
            )
            .expect("cria a conversa");
        let ficha = self
            .store
            .fichas()
            .por_conversa(parse_org(org), conversa.id)
            .expect("consulta a Ficha")
            .expect("a conversa tem Ficha")
            .id;
        (conversa, ficha)
    }

    fn fechar_conversa(&self, org: &str, conversa: &Conversa) {
        self.store
            .conversas()
            .fechar_auditado(
                parse_org(org),
                conversa.id,
                conversa.version,
                "teste-fx29".to_owned(),
            )
            .expect("fecha a conversa");
    }

    /// Abre a execução do fluxo na Ficha, com o fluxo já liberado ao agente.
    fn abrir_execucao(
        &self,
        org: &str,
        agente: &str,
        ficha: RecordId,
        fluxo: FlowId,
        revisao: RevisionId,
    ) -> StoredExecution {
        self.liberar(org, agente, fluxo);
        self.store
            .executions()
            .iniciar_execucao(
                parse_org(org),
                ExecutionStart {
                    chave_idempotencia: None,
                    ficha,
                    fluxo,
                    revision: revisao,
                    actor: EventActor::Agent,
                    ator_ref: Some("agente-de-teste"),
                    jev_decision: None,
                    initial_state: ExecutionState {
                        status: ExecutionStatus::Active,
                        no_atual: "coleta".to_owned(),
                        dados_json: "{}".to_owned(),
                        acting_actors: Vec::new(),
                        result: None,
                        reason: None,
                    },
                    inbound_effects: Vec::new(),
                },
            )
            .expect("abre a execução")
    }

    /// O agente registra o nome na etapa `coleta` (campo respondido, W6).
    fn responder_nome(&self, org: &str, gravada: &StoredExecution, nome: &str) {
        let transicao = FlowTransition {
            new_state: ExecutionState {
                status: ExecutionStatus::Active,
                no_atual: "coleta".to_owned(),
                dados_json: json!({ "coleta": { "nome": nome } }).to_string(),
                acting_actors: vec![ActionActor::Agent],
                result: None,
                reason: None,
            },
            events: vec![TransitionEvent {
                tipo: RecordEventKind::FieldsFilled,
                ator_tipo: EventActor::Agent,
                payload_json: json!({ "etapa": "coleta", "campos": ["nome"] }).to_string(),
            }],
            effects: Vec::new(),
            closes_service: false,
            repeated: false,
            rule_error: false,
        };
        self.store
            .executions()
            .aplicar_transicao(
                parse_org(org),
                gravada.execution.id,
                gravada.execution.version,
                &transicao,
                &format!("coleta-{}", uuid::Uuid::now_v7()),
                TransitionExtras {
                    ator_ref: None,
                    fechada_por: None,
                    close_reason: None,
                },
            )
            .expect("registra o campo respondido");
    }

    /// Lista da org, paginada pelo cursor até o fim. Devolve os ids na ordem em que vieram.
    async fn paginar(&self, org: &str, token: &str, filtros: &str, limite: u32) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let mut caminho = format!("/admin/v1/orgs/{org}/fichas?limit={limite}{filtros}");
            if let Some(atual) = &cursor {
                caminho.push_str(&format!("&cursor={atual}"));
            }
            let (status, corpo) = self.get(&caminho, token).await;
            assert_eq!(status, StatusCode::OK, "listar: {corpo}");
            let itens = corpo["items"].as_array().expect("items");
            assert!(
                itens.len() <= limite as usize,
                "página maior que o limite: {} itens",
                itens.len()
            );
            ids.extend(itens.iter().map(id_de));
            cursor = corpo["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                return ids;
            }
        }
        panic!("paginação não terminou em 100 páginas");
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

fn parse_org(id: &str) -> OrgId {
    id.parse().expect("id de org")
}

fn parse_agente(id: &str) -> AgentId {
    id.parse().expect("id de agente")
}

fn id_de(item: &Value) -> String {
    item["id"].as_str().expect("id no item").to_owned()
}

fn codigo(corpo: &Value) -> &str {
    corpo["code"].as_str().unwrap_or("")
}

fn conjunto(ids: &[String]) -> BTreeSet<String> {
    ids.iter().cloned().collect()
}

#[tokio::test]
async fn filtros_isolados_e_combinados() {
    let c = Cenario::novo("filtros").await;
    let org = c.criar_org("filtros-a").await;
    let token = c.criar_token(&org).await;
    let x = c.criar_agente(&org, &token, "agente-x").await;
    let y = c.criar_agente(&org, &token, "agente-y").await;

    // f1: aberta, X, whatsapp. f2: fechada, X, console. f3: aberta, Y, whatsapp. f4: fechada, Y, whatsapp.
    let f1 = c.ficha_avulsa(&org, &x, "whatsapp");
    let (conversa2, f2) = c.conversa(&org, &x, "console");
    c.fechar_conversa(&org, &conversa2);
    let f3 = c.ficha_avulsa(&org, &y, "whatsapp");
    let (conversa4, f4) = c.conversa(&org, &y, "whatsapp");
    c.fechar_conversa(&org, &conversa4);

    // Ficha de outra org: não pode aparecer na lista de A.
    let org_b = c.criar_org("filtros-b").await;
    let token_b = c.criar_token(&org_b).await;
    let xb = c.criar_agente(&org_b, &token_b, "agente-b").await;
    c.ficha_avulsa(&org_b, &xb, "whatsapp");

    let (f1, f2, f3, f4) = (
        f1.to_string(),
        f2.to_string(),
        f3.to_string(),
        f4.to_string(),
    );
    // (nome do caso, filtros na URL, Fichas que devem voltar)
    let casos: Vec<(&str, String, Vec<String>)> = vec![
        (
            "sem filtro",
            String::new(),
            vec![f1.clone(), f2.clone(), f3.clone(), f4.clone()],
        ),
        (
            "status=open",
            "&status=open".to_owned(),
            vec![f1.clone(), f3.clone()],
        ),
        (
            "status=closed",
            "&status=closed".to_owned(),
            vec![f2.clone(), f4.clone()],
        ),
        (
            "agente X",
            format!("&agente={x}"),
            vec![f1.clone(), f2.clone()],
        ),
        (
            "agente Y",
            format!("&agente={y}"),
            vec![f3.clone(), f4.clone()],
        ),
        (
            "canal=whatsapp",
            "&canal=whatsapp".to_owned(),
            vec![f1.clone(), f3.clone(), f4.clone()],
        ),
        (
            "canal=console",
            "&canal=console".to_owned(),
            vec![f2.clone()],
        ),
        (
            "status=closed + agente Y + canal=whatsapp",
            format!("&status=closed&agente={y}&canal=whatsapp"),
            vec![f4.clone()],
        ),
        (
            "status=open + canal=console",
            "&status=open&canal=console".to_owned(),
            vec![],
        ),
        (
            "desde=0",
            "&desde=0".to_owned(),
            vec![f1.clone(), f2.clone(), f3.clone(), f4.clone()],
        ),
        (
            "desde no futuro",
            "&desde=9223372036854775807".to_owned(),
            vec![],
        ),
    ];
    for (nome, filtros, esperado) in &casos {
        let (status, corpo) = c
            .get(
                &format!("/admin/v1/orgs/{org}/fichas?limit=200{filtros}"),
                &token,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "caso {nome}: {corpo}");
        let vistos: Vec<String> = corpo["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(id_de)
            .collect();
        assert_eq!(
            conjunto(&vistos),
            conjunto(esperado),
            "caso `{nome}` devolveu conjunto diferente"
        );
        assert_eq!(vistos.len(), esperado.len(), "caso `{nome}` repetiu item");
    }

    // Valores fora do vocabulário ou vazios: 400 com o campo, nunca filtro ignorado.
    for (filtro, campo) in [
        ("status=aberta", "status"),
        ("agente=nao-e-um-id", ""),
        ("canal=", "canal"),
        ("desde=ontem", "desde"),
    ] {
        let (status, corpo) = c
            .get(&format!("/admin/v1/orgs/{org}/fichas?{filtro}"), &token)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{filtro}: {corpo}");
        assert!(
            matches!(codigo(&corpo), "validation_error" | "invalid_id"),
            "{filtro} tem de sair com código de validação: {corpo}"
        );
        if !campo.is_empty() {
            assert_eq!(corpo["details"]["field"], campo, "{filtro}: {corpo}");
        }
    }
}

#[tokio::test]
async fn paginacao_por_cursor_nao_repete_nem_pula() {
    let c = Cenario::novo("paginacao").await;
    let org = c.criar_org("paginacao-a").await;
    let token = c.criar_token(&org).await;
    let x = c.criar_agente(&org, &token, "agente-x").await;
    let y = c.criar_agente(&org, &token, "agente-y").await;
    for _ in 0..5 {
        c.ficha_avulsa(&org, &x, "whatsapp");
    }
    for _ in 0..2 {
        c.ficha_avulsa(&org, &y, "whatsapp");
    }

    // Referência: a lista inteira numa página só.
    let (status, corpo) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas?limit=200"), &token)
        .await;
    assert_eq!(status, StatusCode::OK);
    let inteira: Vec<String> = corpo["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(id_de)
        .collect();
    assert_eq!(inteira.len(), 7);

    // Páginas de 2: somadas dão a mesma lista, na mesma ordem, sem repetir na fronteira.
    let paginada = c.paginar(&org, &token, "", 2).await;
    assert_eq!(paginada, inteira, "páginas de 2 diferem da lista inteira");
    assert_eq!(conjunto(&paginada).len(), 7, "item repetido entre páginas");

    // Com filtro: mesma regra.
    let (_, so_x) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?limit=200&agente={x}"),
            &token,
        )
        .await;
    let so_x: Vec<String> = so_x["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(id_de)
        .collect();
    assert_eq!(so_x.len(), 5);
    let paginada_x = c.paginar(&org, &token, &format!("&agente={x}"), 2).await;
    assert_eq!(paginada_x, so_x, "paginação com filtro diferente");

    // Página que cabe exatamente: a próxima vem vazia e sem cursor.
    let (_, cheia) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas?limit=7"), &token)
        .await;
    assert_eq!(cheia["items"].as_array().unwrap().len(), 7);
    let cursor = cheia["next_cursor"]
        .as_str()
        .expect("cursor da página cheia");
    let (_, depois) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?limit=7&cursor={cursor}"),
            &token,
        )
        .await;
    assert!(depois["items"].as_array().unwrap().is_empty());
    assert!(depois["next_cursor"].is_null());

    // Cursor que não é de Ficha: 400 com o campo.
    let (status, erro) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?cursor=nao-e-cursor"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{erro}");
    assert_eq!(codigo(&erro), "invalid_request");
    assert_eq!(erro["details"]["field"], "cursor");
}

#[tokio::test]
async fn ficha_de_outra_org_da_404_nas_rotas_de_leitura() {
    let c = Cenario::novo("isolamento").await;
    let org_a = c.criar_org("isolamento-a").await;
    let token_a = c.criar_token(&org_a).await;
    let agente_a = c.criar_agente(&org_a, &token_a, "agente-a").await;
    let ficha_a = c.ficha_avulsa(&org_a, &agente_a, "whatsapp");

    let org_b = c.criar_org("isolamento-b").await;
    let token_b = c.criar_token(&org_b).await;
    let agente_b = c.criar_agente(&org_b, &token_b, "agente-b").await;
    let ficha_b = c.ficha_avulsa(&org_b, &agente_b, "whatsapp");

    let ficha_b_texto = ficha_b.to_string();
    let inexistente = uuid::Uuid::now_v7().to_string();

    // Detalhe e linha do tempo: 404 com o mesmo código, de Ficha alheia ou inexistente.
    for caminho in [
        format!("/admin/v1/orgs/{org_a}/fichas/{ficha_b_texto}"),
        format!("/admin/v1/orgs/{org_a}/fichas/{ficha_b_texto}/events"),
        format!("/admin/v1/orgs/{org_a}/fichas/{inexistente}"),
        format!("/admin/v1/orgs/{org_a}/fichas/{inexistente}/events"),
    ] {
        let (status, corpo) = c.get(&caminho, &token_a).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{caminho}: {corpo}");
        assert_eq!(codigo(&corpo), "not_found", "{caminho}: {corpo}");
    }
    // O token global também não vê a Ficha de B pelo caminho de A.
    let (status, corpo) = c
        .get(
            &format!("/admin/v1/orgs/{org_a}/fichas/{ficha_b_texto}"),
            TOKEN_GLOBAL,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{corpo}");

    // A lista de A traz só a Ficha de A.
    let (status, lista) = c
        .get(&format!("/admin/v1/orgs/{org_a}/fichas"), &token_a)
        .await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<String> = lista["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(id_de)
        .collect();
    assert_eq!(ids, vec![ficha_a.to_string()]);

    // Token de A em caminho de B: 403 (escopo da org, como no resto da API).
    for caminho in [
        format!("/admin/v1/orgs/{org_b}/fichas"),
        format!("/admin/v1/orgs/{org_b}/fichas/{ficha_b_texto}"),
        format!("/admin/v1/orgs/{org_b}/fichas/{ficha_b_texto}/events"),
    ] {
        let (status, corpo) = c.get(&caminho, &token_a).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caminho}: {corpo}");
    }

    // Controle: o dono da Ficha lê normalmente.
    let (status, _) = c
        .get(
            &format!("/admin/v1/orgs/{org_b}/fichas/{ficha_b_texto}"),
            &token_b,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn eventos_em_ordem_de_seq_com_paginacao() {
    let c = Cenario::novo("eventos").await;
    let org = c.criar_org("eventos-a").await;
    let token = c.criar_token(&org).await;
    let agente = c.criar_agente(&org, &token, "agente-x").await;
    let (fluxo, revisao) = c.fluxo_publicado(&org, "agendar-visita");
    let ficha = c.ficha_avulsa(&org, &agente, "whatsapp");
    let gravada = c.abrir_execucao(&org, &agente, ficha, fluxo, revisao);
    c.responder_nome(&org, &gravada, "Ana Teste");

    let caminho = format!("/admin/v1/orgs/{org}/fichas/{ficha}/events");
    let (status, todos) = c.get(&format!("{caminho}?limit=200"), &token).await;
    assert_eq!(status, StatusCode::OK, "{todos}");
    let itens = todos["items"].as_array().unwrap();
    let seqs: Vec<i64> = itens.iter().map(|i| i["seq"].as_i64().unwrap()).collect();
    assert!(
        seqs.len() >= 3,
        "esperava abertura, início e campo: {seqs:?}"
    );
    let esperados: Vec<i64> = (1..=seqs.len() as i64).collect();
    assert_eq!(seqs, esperados, "seq fora de ordem ou com buraco");
    let tipos: Vec<&str> = itens.iter().map(|i| i["tipo"].as_str().unwrap()).collect();
    assert_eq!(tipos.first(), Some(&"ficha_aberta"));
    assert!(tipos.contains(&"fluxo_iniciado"), "{tipos:?}");
    assert_eq!(tipos.last(), Some(&"campos_preenchidos"), "{tipos:?}");

    // Paginado de 1 em 1: a mesma sequência, sem repetir nem pular.
    let mut paginados: Vec<i64> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let mut atual = format!("{caminho}?limit=1");
        if let Some(valor) = &cursor {
            atual.push_str(&format!("&cursor={valor}"));
        }
        let (status, pagina) = c.get(&atual, &token).await;
        assert_eq!(status, StatusCode::OK, "{pagina}");
        assert!(pagina["items"].as_array().unwrap().len() <= 1);
        paginados.extend(
            pagina["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["seq"].as_i64().unwrap()),
        );
        cursor = pagina["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        paginados, seqs,
        "paginação por seq diferente da lista inteira"
    );

    // Cursor é o seq: começar depois do segundo evento devolve do terceiro em diante.
    let (_, depois_do_segundo) = c
        .get(&format!("{caminho}?cursor=2&limit=200"), &token)
        .await;
    let a_partir_do_terceiro: Vec<i64> = depois_do_segundo["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(a_partir_do_terceiro, seqs[2..].to_vec());

    let (status, erro) = c.get(&format!("{caminho}?cursor=abc"), &token).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{erro}");
    assert_eq!(codigo(&erro), "invalid_request");
}

#[tokio::test]
async fn execucao_com_etapas_e_campos_aparece_dentro_da_ficha() {
    let c = Cenario::novo("w6").await;
    let org = c.criar_org("w6-a").await;
    let token = c.criar_token(&org).await;
    let agente = c.criar_agente(&org, &token, "agente-x").await;
    let (fluxo, revisao) = c.fluxo_publicado(&org, "agendar-visita");
    let ficha = c.ficha_avulsa(&org, &agente, "whatsapp");
    let gravada = c.abrir_execucao(&org, &agente, ficha, fluxo, revisao);
    c.responder_nome(&org, &gravada, "Ana Teste");

    let (status, corpo) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas/{ficha}"), &token)
        .await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["id"], ficha.to_string());
    assert_eq!(corpo["status"], "open");

    let execucoes = corpo["execucoes"].as_array().expect("execucoes na Ficha");
    assert_eq!(execucoes.len(), 1, "{corpo}");
    let execucao = &execucoes[0];
    assert_eq!(execucao["id"], gravada.execution.id.to_string());
    assert_eq!(execucao["fluxo_id"], fluxo.to_string());
    assert_eq!(execucao["revisao_id"], revisao.to_string());
    assert_eq!(execucao["status"], "ativa");
    assert_eq!(execucao["no_atual"], "coleta");

    let etapas = execucao["etapas"].as_array().expect("etapas na execução");
    assert_eq!(etapas.len(), 1, "{execucao}");
    assert_eq!(etapas[0]["no_id"], "coleta");
    assert_eq!(etapas[0]["ordem"], 1);
    assert_eq!(etapas[0]["status"], "parcial");
    assert_eq!(
        etapas[0]["dados"]["nome"], "Ana Teste",
        "o campo respondido tem de estar na etapa dentro da Ficha: {etapas:?}"
    );
    assert_eq!(etapas[0]["atores_que_agiram"], json!(["agente"]));

    let liberados = corpo["permissoes"]["fluxos_liberados_ao_agente"]
        .as_array()
        .expect("permissões");
    assert_eq!(liberados.len(), 1, "{corpo}");
    assert_eq!(liberados[0]["slug"], "agendar-visita");
    assert_eq!(liberados[0]["id"], fluxo.to_string());

    // Nada de texto de mensagem na Ficha: só campos e metadados.
    assert!(!corpo.to_string().contains("mensagem_ao_cliente"));
}

#[tokio::test]
async fn ficha_fechada_continua_legivel() {
    let c = Cenario::novo("fechada").await;
    let org = c.criar_org("fechada-a").await;
    let token = c.criar_token(&org).await;
    let agente = c.criar_agente(&org, &token, "agente-x").await;
    let (fluxo, revisao) = c.fluxo_publicado(&org, "agendar-visita");
    let (conversa, ficha) = c.conversa(&org, &agente, "whatsapp");
    let gravada = c.abrir_execucao(&org, &agente, ficha, fluxo, revisao);
    c.responder_nome(&org, &gravada, "Ana Teste");
    c.fechar_conversa(&org, &conversa);

    let (status, corpo) = c
        .get(&format!("/admin/v1/orgs/{org}/fichas/{ficha}"), &token)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Ficha fechada tem de ser legível: {corpo}"
    );
    assert_eq!(corpo["status"], "closed");
    assert!(corpo["fechada_em"].is_i64(), "{corpo}");
    assert_eq!(
        corpo["fechada_por"], "admin",
        "fechar pela conversa grava o autor: {corpo}"
    );
    assert!(corpo["fechada_motivo"].is_string(), "{corpo}");
    let execucao = &corpo["execucoes"][0];
    assert_eq!(execucao["status"], "abandonada", "{execucao}");
    assert_eq!(execucao["motivo_fim"], "atendimento_encerrado");
    assert_eq!(execucao["etapas"][0]["dados"]["nome"], "Ana Teste");

    let (status, eventos) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/events?limit=200"),
            &token,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{eventos}");
    let tipos: Vec<&str> = eventos["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["tipo"].as_str().unwrap())
        .collect();
    assert!(tipos.contains(&"execucao_abandonada"), "{tipos:?}");
    assert_eq!(tipos.last(), Some(&"ficha_fechada"), "{tipos:?}");

    // A lista também mostra a Ficha fechada com o filtro de status.
    let (_, fechadas) = c
        .get(
            &format!("/admin/v1/orgs/{org}/fichas?status=closed"),
            &token,
        )
        .await;
    let ids: Vec<String> = fechadas["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(id_de)
        .collect();
    assert_eq!(ids, vec![ficha.to_string()]);
}
