//! Testes HTTP das ações humanas do atendente sobre a Ficha (FX-30, PRD §7.3 e concorrência).
//!
//! Servidor de verdade (porta efêmera) sobre banco temporário, com cliente HTTP de verdade. O fluxo
//! é um script compilado de verdade (`poph_flows::compile`) e publicado pelo repositório (a
//! publicação pela API é a FX-27). O agente é simulado pelo próprio motor e pela operação atômica
//! (o turno do agente ainda não existe), para provar a corrida atendente × agente. Nomes e
//! telefones são fictícios.
//!
//! O que fica provado aqui:
//! - iniciar fluxo sem `If-Match` → `428`; chave vazia → `400`; versão velha → `412`;
//! - Ficha fechada → `409` ao iniciar, agir e abandonar; fluxo não liberado → `403` sem abrir execução;
//! - repetir a chave do início não abre outra execução, nem depois que a primeira terminou;
//! - atendente conclui a etapa que o critério `auto` só fecha com o agente (parcial até o agente);
//! - corrida atendente × agente: quem chega com versão velha leva `412` e nada é gravado;
//! - repetir a chave de uma ação devolve o mesmo desfecho, sem evento novo, mesmo com versão velha;
//! - ação restrita ao agente chamada pelo atendente → `409` (`acao_invalida`, a recusa do motor);
//! - abandono grava motivo e não gera efeito; motivo vazio → `400`;
//! - `close` fecha conversa e Ficha e abandona a execução; sem conversa, fecha só a Ficha;
//! - Ficha ou execução de outra org → `404` (token de outra org no caminho → `403`); execução de
//!   outra Ficha da mesma org → `404`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_flows::engine::{Context, Estado as EstadoMotor, execute_action};
use poph_flows::ir::Destination;
use poph_flows::{Ir, Limites, RuleEvaluator, compile};
use poph_server::Estado;
use poph_server::fluxos_transicao::transicao_do_motor;
use poph_store::{
    Execution, NewConversation, NewFlow, NewRecord, RevisionPublication, Store, TransitionExtras,
};
use poph_types::config::GlobalTokenConfig;
use poph_types::{
    ActionActor, AgentId, ExecutionId, FlowId, HarnessConfig, OrgId, RecordId, RevisionId,
};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado nos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-fx30";

/// Fluxo de teste: a etapa `coleta` é do agente **e** do atendente, e só avança quando os dois
/// agiram (critério `auto`); a etapa `revisao` é do atendente e conclui o fluxo.
const SCRIPT: &str = r#"fluxo(
    id = "agendar",
    nome = "Agendar visita",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer agendar uma visita.",
        nao_quando = "O cliente nao quer agendar.",
        exemplos = ["quero agendar"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    atores = ["agente", "atendente"],
    campos = [campo("nome", "Nome", obrigatorio = True)],
    acoes = [
        acao("registrar", "Registrar", ator = "agente", exige = ["nome"], vai_para = "revisao"),
        acao("confirmar", "Confirmar", ator = "atendente", vai_para = "revisao"),
    ],
)

etapa(
    id = "revisao",
    nome = "Revisão",
    atores = ["atendente"],
    acoes = [
        acao("concluir", "Concluir", ator = "atendente", vai_para = fim("concluido", resultado = "Agendado")),
    ],
)
"#;

fn hash_argon2(valor: &str) -> String {
    let sal = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(valor.as_bytes(), &sal)
        .expect("hash argon2")
        .to_string()
}

fn parse_org(id: &str) -> OrgId {
    id.parse().expect("id de org")
}

fn parse_agente(id: &str) -> AgentId {
    id.parse().expect("id de agente")
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
            "poph-acoes-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        config.server.port = 7483;
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
            let _ = poph_server::serve_at(listener, estado, async {
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

    /// Pedido HTTP. `versao` vira o `If-Match`; `None` não manda o cabeçalho.
    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        versao: Option<u64>,
        corpo: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base))
            .bearer_auth(token);
        if let Some(versao) = versao {
            pedido = pedido.header("If-Match", versao.to_string());
        }
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

    async fn criar_org(&self, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                None,
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
                None,
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
                None,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {corpo}");
        corpo["id"].as_str().expect("id do agente").to_owned()
    }

    /// Fluxo publicado (criado, compilado e publicado pelo repositório) e liberado ao agente.
    fn fluxo_liberado(&self, org: &str, agente: &str, slug: &str) -> (FlowId, RevisionId) {
        let (fluxo, revisao) = self.fluxo_publicado(org, slug);
        self.store
            .flows()
            .release(parse_org(org), parse_agente(agente), fluxo)
            .expect("libera o fluxo ao agente");
        (fluxo, revisao)
    }

    /// Fluxo publicado, sem liberar a agente nenhum.
    fn fluxo_publicado(&self, org: &str, slug: &str) -> (FlowId, RevisionId) {
        let org_id = parse_org(org);
        let fluxo = self
            .store
            .flows()
            .create(
                org_id,
                NewFlow {
                    slug: slug.to_owned(),
                    name: format!("Fluxo {slug}"),
                    description: String::new(),
                    rascunho_fonte: SCRIPT.to_owned(),
                },
            )
            .expect("cria o fluxo");
        let compilado = compile(SCRIPT, &Limites::default());
        let ir = match compilado.ir() {
            Ok(ir) => ir,
            Err(erros) => panic!("script de teste inválido: {erros:?}"),
        };
        let revisao = self
            .store
            .revisions()
            .publish_revision(
                org_id,
                fluxo.id,
                RevisionPublication {
                    expected: fluxo.version,
                    fonte: SCRIPT.to_owned(),
                    definicao_json: ir.to_json(),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "admin_token".to_owned(),
                    actor_id: None,
                },
            )
            .expect("publica a revisão");
        (fluxo.id, revisao.id)
    }

    /// Ficha sem conversa, do agente.
    fn ficha_avulsa(&self, org: &str, agente: &str) -> RecordId {
        self.store
            .cases()
            .create(
                parse_org(org),
                NewRecord {
                    conversation_id: None,
                    agent_id: parse_agente(agente),
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                },
            )
            .expect("cria a Ficha")
            .id
    }

    /// Ficha ligada a uma conversa nova (a Ficha nasce com ela).
    fn ficha_com_conversa(&self, org: &str, agente: &str) -> (poph_store::Conversa, RecordId) {
        let conversa = self
            .store
            .conversations()
            .create(
                parse_org(org),
                NewConversation {
                    agent_id: parse_agente(agente),
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                    target_department: None,
                },
            )
            .expect("cria a conversa");
        let ficha = self
            .store
            .cases()
            .by_conversation(parse_org(org), conversa.id)
            .expect("consulta a Ficha")
            .expect("a conversa tem Ficha")
            .id;
        (conversa, ficha)
    }

    fn versao_da_ficha(&self, org: &str, ficha: RecordId) -> u64 {
        self.store
            .cases()
            .get(parse_org(org), ficha)
            .expect("lê a Ficha")
            .version
    }

    fn execucao(&self, org: &str, run: &str) -> Execution {
        self.store
            .executions()
            .get(parse_org(org), run.parse().expect("id da execução"))
            .expect("lê a execução")
    }

    fn quantas_execucoes(&self, org: &str, ficha: RecordId) -> usize {
        self.store
            .executions()
            .from_case(parse_org(org), ficha, 100)
            .expect("lista as execuções")
            .len()
    }

    fn quantos_eventos(&self, org: &str, ficha: RecordId) -> usize {
        self.store
            .cases()
            .events(parse_org(org), ficha, None, 500)
            .expect("lista os eventos")
            .len()
    }

    /// O agente age pelo próprio motor e pela operação atômica, como o turno dele faria (o turno
    /// ainda não existe). Usa a mesma ponte de tradução da API.
    fn agente_age(&self, org: &str, run: &str, acao: &str, campos: Value, chave: &str) {
        let org_id = parse_org(org);
        let run_id: ExecutionId = run.parse().expect("id da execução");
        let execucao = self.execucao(org, run);
        let revisao = self
            .store
            .revisions()
            .get_by_id(org_id, execucao.revisao_id)
            .expect("lê a revisão");
        let ir = Ir::from_json(&revisao.definicao_json).expect("IR da revisão");
        let atores_que_agiram = self
            .store
            .executions()
            .stages(org_id, run_id)
            .expect("lê as etapas")
            .iter()
            .rev()
            .find(|etapa| etapa.no_id == execucao.no_atual)
            .map(|etapa| etapa.acting_actors.clone())
            .unwrap_or_default();
        let estado = EstadoMotor {
            fluxo_id: ir.fluxo.id.clone(),
            status: execucao.status,
            no_atual: Destination::Etapa {
                id: execucao.no_atual.clone(),
            },
            dados: serde_json::from_str(&execucao.dados_json).expect("dados da execução"),
            actors_who_acted: atores_que_agiram,
            resultado: None,
            motivo: None,
            keys: BTreeMap::new(),
        };
        let avaliador = RuleEvaluator::without_functions();
        let ficha_texto = execucao.ficha_id.to_string();
        let contexto = Context::new(&avaliador)
            .with_case(&ficha_texto)
            .with_client("cliente-ficticio");
        let campos = campos.as_object().cloned().unwrap_or_default();
        let resultado = execute_action(
            &ir,
            &estado,
            acao,
            &campos,
            ActionActor::Agent,
            Some(chave),
            &contexto,
        )
        .expect("o agente age na etapa");
        self.store
            .executions()
            .apply_transition(
                org_id,
                run_id,
                execucao.version,
                &transicao_do_motor(&resultado),
                chave,
                TransitionExtras {
                    ator_ref: Some("agente-de-teste"),
                    fechada_por: None,
                    close_reason: None,
                },
            )
            .expect("grava a ação do agente");
    }

    // ── Pedidos das rotas de ações ──

    async fn iniciar(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        versao: Option<u64>,
        fluxo: FlowId,
        chave: &str,
    ) -> (StatusCode, Value) {
        self.pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/flow-runs"),
            token,
            versao,
            Some(json!({ "fluxo_id": fluxo.to_string(), "chave_idempotencia": chave })),
        )
        .await
    }

    // Espelha a assinatura da rota (org, token, ficha, execução, versão, ação, campos, chave).
    #[allow(clippy::too_many_arguments)]
    async fn acao(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        run: &str,
        versao: Option<u64>,
        acao: &str,
        campos: Value,
        chave: &str,
    ) -> (StatusCode, Value) {
        self.pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/flow-runs/{run}/actions"),
            token,
            versao,
            Some(json!({ "acao": acao, "campos": campos, "chave_idempotencia": chave })),
        )
        .await
    }

    // Espelha a assinatura da rota (org, token, ficha, execução, versão, motivo, chave).
    #[allow(clippy::too_many_arguments)]
    async fn abandonar(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        run: &str,
        versao: Option<u64>,
        motivo: &str,
        chave: &str,
    ) -> (StatusCode, Value) {
        self.pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/flow-runs/{run}/abandon"),
            token,
            versao,
            Some(json!({ "motivo": motivo, "chave_idempotencia": chave })),
        )
        .await
    }

    async fn fechar(
        &self,
        org: &str,
        token: &str,
        ficha: RecordId,
        versao: Option<u64>,
    ) -> (StatusCode, Value) {
        self.pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{org}/fichas/{ficha}/close"),
            token,
            versao,
            None,
        )
        .await
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
    corpo["code"].as_str().unwrap_or("")
}

/// Org com agente, token e fluxo liberado: o cenário mínimo de quase todos os testes.
struct Base {
    org: String,
    token: String,
    agente: String,
    fluxo: FlowId,
}

async fn base(c: &Cenario, slug: &str) -> Base {
    let org = c.criar_org(slug).await;
    let token = c.criar_token(&org).await;
    let agente = c
        .criar_agente(&org, &token, &format!("agente-{slug}"))
        .await;
    let (fluxo, _revisao) = c.fluxo_liberado(
        &org,
        &agente,
        &format!("agendar_{}", slug.replace('-', "_")),
    );
    Base {
        org,
        token,
        agente,
        fluxo,
    }
}

// ───────────────────────────────────── Testes ─────────────────────────────────────

#[tokio::test]
async fn iniciar_exige_if_match_chave_e_versao_certa() {
    let c = Cenario::novo("inicio-exige").await;
    let b = base(&c, "inicio-exige").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);

    let (status, corpo) = c
        .iniciar(&b.org, &b.token, ficha, None, b.fluxo, "chave-1")
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{corpo}");
    assert_eq!(codigo(&corpo), "precondition_required");

    let (status, corpo) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), b.fluxo, "   ")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{corpo}");
    assert_eq!(codigo(&corpo), "validation_error");

    let (status, corpo) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha,
            Some(versao + 7),
            b.fluxo,
            "chave-1",
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{corpo}");
    assert_eq!(codigo(&corpo), "version_conflict");
    assert_eq!(c.quantas_execucoes(&b.org, ficha), 0, "nada foi aberto");
}

#[tokio::test]
async fn iniciar_em_ficha_fechada_da_409() {
    let c = Cenario::novo("ficha-fechada").await;
    let b = base(&c, "ficha-fechada").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    c.store
        .cases()
        .close_without_conversation(parse_org(&b.org), ficha, 1, "encerrada no teste")
        .expect("fecha a Ficha");
    let versao = c.versao_da_ficha(&b.org, ficha);

    let (status, corpo) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), b.fluxo, "chave-1")
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{corpo}");
    assert_eq!(codigo(&corpo), "ficha_encerrada");
    assert_eq!(c.quantas_execucoes(&b.org, ficha), 0);
}

#[tokio::test]
async fn fluxo_nao_liberado_da_403_e_nao_abre_execucao() {
    let c = Cenario::novo("nao-liberado").await;
    let b = base(&c, "nao-liberado").await;
    // Segundo fluxo, publicado mas não liberado a este agente.
    let (outro, _) = c.fluxo_publicado(&b.org, "sem_permissao");
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);

    let (status, corpo) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), outro, "chave-1")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{corpo}");
    assert_eq!(codigo(&corpo), "fluxo_nao_liberado");
    assert_eq!(c.quantas_execucoes(&b.org, ficha), 0);
}

#[tokio::test]
async fn repetir_a_chave_do_inicio_nao_duplica_nem_depois_de_terminar() {
    let c = Cenario::novo("inicio-repetido").await;
    let b = base(&c, "inicio-repetido").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);

    let (status, primeira) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{primeira}");
    let run = primeira["id"].as_str().expect("id da execução").to_owned();
    let eventos = c.quantos_eventos(&b.org, ficha);

    // Repetição com a mesma chave, enquanto a execução ainda está ativa.
    let (status, repetida) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repetida}");
    assert_eq!(repetida["id"], primeira["id"]);
    assert_eq!(repetida["repetida"], json!(true));
    assert_eq!(c.quantas_execucoes(&b.org, ficha), 1);
    assert_eq!(c.quantos_eventos(&b.org, ficha), eventos, "sem evento novo");

    // Depois que a execução termina, a mesma chave ainda devolve a primeira, não abre outra.
    let (status, _) = c
        .abandonar(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(c.execucao(&b.org, &run).version),
            "cliente desistiu",
            "parar-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, depois) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{depois}");
    assert_eq!(depois["id"], primeira["id"]);
    assert_eq!(
        c.quantas_execucoes(&b.org, ficha),
        1,
        "nenhuma execução nova"
    );
}

#[tokio::test]
async fn atendente_confirma_o_agente_completa_e_o_atendente_conclui() {
    let c = Cenario::novo("criterio-auto").await;
    let b = base(&c, "criterio-auto").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);
    let (_, aberta) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = aberta["id"].as_str().expect("id").to_owned();

    // O atendente confirma primeiro: a etapa não avança sozinha (o agente também age ali).
    let (status, parcial) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(c.execucao(&b.org, &run).version),
            "confirmar",
            json!({}),
            "atendente-confirma",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{parcial}");
    assert_eq!(parcial["no_atual"], json!("coleta"), "{parcial}");
    assert_eq!(
        parcial["etapas"][0]["status"],
        json!("parcial"),
        "{parcial}"
    );

    // O agente registra o nome: agora os dois agiram e a etapa avança para a revisão.
    c.agente_age(
        &b.org,
        &run,
        "registrar",
        json!({ "nome": "Ana Teste" }),
        "agente-registra",
    );
    assert_eq!(c.execucao(&b.org, &run).no_atual, "revisao");

    // O atendente conclui: a execução termina com o resultado do fim.
    let (status, concluida) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(c.execucao(&b.org, &run).version),
            "concluir",
            json!({}),
            "atendente-conclui",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{concluida}");
    assert_eq!(concluida["status"], json!("concluida"), "{concluida}");
    assert_eq!(concluida["resultado"], json!("Agendado"));
}

#[tokio::test]
async fn corrida_atendente_com_agente_da_412_e_nao_grava_nada() {
    let c = Cenario::novo("corrida").await;
    let b = base(&c, "corrida").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);
    let (_, aberta) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = aberta["id"].as_str().expect("id").to_owned();
    let versao_lida = c.execucao(&b.org, &run).version;

    // O agente grava primeiro: a versão que o atendente leu ficou velha.
    c.agente_age(
        &b.org,
        &run,
        "registrar",
        json!({ "nome": "Bia Teste" }),
        "agente-primeiro",
    );
    let eventos = c.quantos_eventos(&b.org, ficha);

    let (status, corpo) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(versao_lida),
            "confirmar",
            json!({}),
            "atendente-atrasado",
        )
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{corpo}");
    assert_eq!(codigo(&corpo), "version_conflict");
    assert_eq!(
        c.quantos_eventos(&b.org, ficha),
        eventos,
        "a recusa não gravou evento"
    );
    // Com o agente sozinho a etapa não avança: o critério `auto` também espera o atendente.
    assert_eq!(c.execucao(&b.org, &run).no_atual, "coleta");
}

#[tokio::test]
async fn repetir_a_chave_da_acao_devolve_o_mesmo_sem_evento_novo() {
    let c = Cenario::novo("acao-repetida").await;
    let b = base(&c, "acao-repetida").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);
    let (_, aberta) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = aberta["id"].as_str().expect("id").to_owned();
    let versao_lida = c.execucao(&b.org, &run).version;

    let (status, primeira) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(versao_lida),
            "confirmar",
            json!({}),
            "confirma-uma-vez",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{primeira}");
    let eventos = c.quantos_eventos(&b.org, ficha);
    let versao_depois = c.execucao(&b.org, &run).version;

    // Reenvio da mesma chave com a versão antiga (queda de rede no meio): mesmo desfecho.
    let (status, repetida) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(versao_lida),
            "confirmar",
            json!({}),
            "confirma-uma-vez",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repetida}");
    assert_eq!(repetida["repetida"], json!(true));
    assert_eq!(c.quantos_eventos(&b.org, ficha), eventos, "sem evento novo");
    assert_eq!(
        c.execucao(&b.org, &run).version,
        versao_depois,
        "versão não andou"
    );
}

#[tokio::test]
async fn repetir_a_abertura_depois_de_fechar_a_ficha_devolve_a_mesma() {
    let c = Cenario::novo("inicio-depois-fechar").await;
    let b = base(&c, "inicio-depois-fechar").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);

    let (status, primeira) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{primeira}");

    // Fecha a Ficha: a execução ativa é abandonada pelo fechamento.
    let (status, corpo) = c.fechar(&b.org, &b.token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    let eventos = c.quantos_eventos(&b.org, ficha);

    // Reenvio com a mesma chave e com o If-Match de antes: devolve a primeira execução, sem nada novo.
    let (status, repetida) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "abrir-visita",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repetida}");
    assert_eq!(repetida["id"], primeira["id"]);
    assert_eq!(repetida["repetida"], json!(true));
    assert_eq!(c.quantas_execucoes(&b.org, ficha), 1);
    assert_eq!(c.quantos_eventos(&b.org, ficha), eventos, "sem evento novo");

    // Chave nova numa Ficha fechada continua recusada com 409.
    let (status, corpo) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha,
            Some(versao),
            b.fluxo,
            "outra-abertura",
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{corpo}");
}

#[tokio::test]
async fn acao_restrita_ao_agente_chamada_pelo_atendente_da_409() {
    let c = Cenario::novo("acao-restrita").await;
    let b = base(&c, "acao-restrita").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);
    let (_, aberta) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = aberta["id"].as_str().expect("id").to_owned();
    let versao_lida = c.execucao(&b.org, &run).version;

    let (status, corpo) = c
        .acao(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(versao_lida),
            "registrar",
            json!({ "nome": "Carla Teste" }),
            "atendente-tenta-agente",
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{corpo}");
    assert_eq!(codigo(&corpo), "acao_invalida");
    assert_eq!(c.execucao(&b.org, &run).version, versao_lida, "nada mudou");
}

#[tokio::test]
async fn abandonar_grava_motivo_e_repetir_a_chave_nao_repete_o_evento() {
    let c = Cenario::novo("abandono").await;
    let b = base(&c, "abandono").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);
    let (_, aberta) = c
        .iniciar(&b.org, &b.token, ficha, Some(versao), b.fluxo, "abrir")
        .await;
    let run = aberta["id"].as_str().expect("id").to_owned();
    let versao_lida = c.execucao(&b.org, &run).version;

    let (status, corpo) = c
        .abandonar(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(versao_lida),
            "",
            "sem-motivo",
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{corpo}");

    let (status, corpo) = c
        .abandonar(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(versao_lida),
            "cliente desistiu",
            "parar",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["status"], json!("abandonada"));
    assert_eq!(corpo["motivo_fim"], json!("cliente desistiu"));
    let eventos = c.quantos_eventos(&b.org, ficha);

    let (status, repetida) = c
        .abandonar(
            &b.org,
            &b.token,
            ficha,
            &run,
            Some(versao_lida),
            "cliente desistiu",
            "parar",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{repetida}");
    assert_eq!(repetida["repetida"], json!(true));
    assert_eq!(c.quantos_eventos(&b.org, ficha), eventos, "sem evento novo");
}

#[tokio::test]
async fn close_fecha_conversa_ficha_e_abandona_a_execucao() {
    let c = Cenario::novo("close-conversa").await;
    let org = c.criar_org("close-conversa").await;
    let token = c.criar_token(&org).await;
    let agente = c.criar_agente(&org, &token, "agente-close").await;
    let (fluxo, _) = c.fluxo_liberado(&org, &agente, "agendar_close");
    let (conversa, ficha) = c.ficha_com_conversa(&org, &agente);
    let versao = c.versao_da_ficha(&org, ficha);
    let (status, aberta) = c
        .iniciar(&org, &token, ficha, Some(versao), fluxo, "abrir")
        .await;
    assert_eq!(status, StatusCode::CREATED, "{aberta}");
    let run = aberta["id"].as_str().expect("id").to_owned();

    // Versão velha da Ficha: `412`, nada fecha.
    let (status, corpo) = c.fechar(&org, &token, ficha, Some(versao + 3)).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{corpo}");
    assert_eq!(c.execucao(&org, &run).status.as_text(), "ativa");

    let (status, corpo) = c.fechar(&org, &token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["status"], json!("closed"));

    let conversa_depois = c
        .store
        .conversations()
        .get(parse_org(&org), conversa.id)
        .expect("lê a conversa");
    assert_eq!(conversa_depois.status.as_text(), "closed");
    let execucao = c.execucao(&org, &run);
    assert_eq!(execucao.status.as_text(), "abandonada");
    assert_eq!(
        execucao.motivo_fim.as_deref(),
        Some("atendimento_encerrado")
    );

    // Fechar de novo recusa: `409`, sem evento novo.
    let eventos = c.quantos_eventos(&org, ficha);
    let (status, corpo) = c.fechar(&org, &token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{corpo}");
    assert_eq!(c.quantos_eventos(&org, ficha), eventos);
}

#[tokio::test]
async fn close_sem_conversa_fecha_so_a_ficha() {
    let c = Cenario::novo("close-avulsa").await;
    let b = base(&c, "close-avulsa").await;
    let ficha = c.ficha_avulsa(&b.org, &b.agente);
    let versao = c.versao_da_ficha(&b.org, ficha);

    let (status, corpo) = c.fechar(&b.org, &b.token, ficha, Some(versao + 3)).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{corpo}");

    let (status, corpo) = c.fechar(&b.org, &b.token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["status"], json!("closed"));
    assert_eq!(
        corpo["fechada_motivo"],
        json!("encerrada pelo administrador")
    );

    let (status, corpo) = c.fechar(&b.org, &b.token, ficha, Some(versao)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{corpo}");
}

#[tokio::test]
async fn isolamento_das_rotas_de_acao_por_org_e_por_ficha() {
    let c = Cenario::novo("isolamento").await;
    let a = base(&c, "iso-a").await;
    let b = base(&c, "iso-b").await;
    let ficha_a = c.ficha_avulsa(&a.org, &a.agente);
    let ficha_a2 = c.ficha_avulsa(&a.org, &a.agente);
    let versao_a = c.versao_da_ficha(&a.org, ficha_a);
    let (_, aberta) = c
        .iniciar(&a.org, &a.token, ficha_a, Some(versao_a), a.fluxo, "abrir")
        .await;
    let run_a = aberta["id"].as_str().expect("id").to_owned();
    let versao_exec = c.execucao(&a.org, &run_a).version;

    // Token da org B lendo a Ficha da org A: `404`, sem dizer que a Ficha existe.
    let (status, corpo) = c
        .iniciar(
            &b.org,
            &b.token,
            ficha_a,
            Some(versao_a),
            a.fluxo,
            "outra-org",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{corpo}");
    let (status, corpo) = c.fechar(&b.org, &b.token, ficha_a, Some(versao_a)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{corpo}");
    let (status, corpo) = c
        .acao(
            &b.org,
            &b.token,
            ficha_a,
            &run_a,
            Some(versao_exec),
            "confirmar",
            json!({}),
            "outra-org-acao",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{corpo}");
    let (status, corpo) = c
        .abandonar(
            &b.org,
            &b.token,
            ficha_a,
            &run_a,
            Some(versao_exec),
            "x",
            "outra-org-ab",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{corpo}");

    // Token da org B no caminho da org A: `403`, como no resto da API.
    let (status, corpo) = c
        .iniciar(
            &a.org,
            &b.token,
            ficha_a,
            Some(versao_a),
            a.fluxo,
            "caminho-alheio",
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{corpo}");

    // Mesma org, outra Ficha: a execução da Ficha A não age pela Ficha A2.
    let versao_a2 = c.versao_da_ficha(&a.org, ficha_a2);
    let (status, corpo) = c
        .acao(
            &a.org,
            &a.token,
            ficha_a2,
            &run_a,
            Some(versao_exec),
            "confirmar",
            json!({}),
            "outra-ficha",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{corpo}");
    assert_eq!(c.execucao(&a.org, &run_a).version, versao_exec);
    let _ = versao_a2;
}
