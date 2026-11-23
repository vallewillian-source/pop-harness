//! Testes HTTP da API de fluxos (FX-27, PRD §6.2 e §7.3): validar, rascunho, publicar, revisões,
//! diff, preview, formatar e exclusão.
//!
//! Servidor de verdade (porta efêmera) sobre banco temporário e cliente HTTP de verdade. O que fica
//! provado:
//!
//! - `validate` devolve a IR e zero diagnósticos num script bom, e diagnóstico com linha e coluna
//!   num script ruim, sem gravar nada;
//! - `format` devolve a forma canônica (formatar duas vezes dá o mesmo texto);
//! - `POST /flows` cria a partir do script e recusa script inválido com `422` sem criar nada; o
//!   teto `fluxos.max_fluxos_por_org` vale na criação;
//! - publicar sem `If-Match` é `428`, com versão velha é `412` e com script inválido é `422`, nos
//!   três casos sem gravar revisão;
//! - a revisão 2 entra e a 1 continua idêntica byte a byte (texto, hash e estrutura);
//! - `diff` traz as mudanças do FX-17 e a contagem de execuções ativas por revisão;
//! - `preview` devolve o bloco de prompt da etapa e `404` para etapa inexistente;
//! - `DELETE` com execução ativa é `409` com a contagem; com `force=true` exclui, abandona a
//!   execução e audita;
//! - isolamento: token de outra org é `403`, sem token é `401` e fluxo de outra org é `404`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, SaltString};
use poph_server::Estado;
use poph_store::{NovoFluxo, Pagina, Store};
use poph_types::config::TokenGlobalConfig;
use poph_types::{
    AgentId, AtorEvento, FichaId, FluxoId, HarnessConfig, OrgId, RevisaoId, StatusExecucao,
};
use rand_core::OsRng;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Valor do token global usado nos testes (o servidor guarda só o hash).
const TOKEN_GLOBAL: &str = "pophg_teste-global-fx27";

/// Script bom (o exemplo do repositório, sem os comentários): compila sem nenhum diagnóstico.
const SCRIPT_V1: &str = r#"fluxo(
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
        campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"),
        acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu")),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmar visita",
    instrucao = "Repita o endereço e o período combinados e peça a confirmação do cliente.",
    acoes = [
        acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

/// O mesmo fluxo com outra instrução na etapa `confirmacao`: a revisão 2 muda uma frase (o `diff`
/// tem de enxergar `instrucao_mudou`) sem mexer no resto.
const SCRIPT_V2: &str = r#"fluxo(
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
        campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"),
        acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu")),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmar visita",
    instrucao = "Confirme o endereço e o período e pergunte se o cliente está no local.",
    acoes = [
        acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
"#;

/// O mesmo fluxo de [`SCRIPT_V1`] escrito numa linha por declaração: compila igual e tem de
/// formatar para o mesmo texto canônico.
const SCRIPT_TORTO: &str = r#"fluxo(id = "agendar_visita", nome = "Agendar visita", descricao = "Marca uma visita técnica no endereço do cliente.", inicio = "coleta", gatilho = gatilho(quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.", nao_quando = "Só quer saber preço ou prazo, sem pedir visita.", exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"]))

etapa(id = "coleta", nome = "Coletar dados", instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.", campos = [campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True), campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True)], acoes = [acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"), acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu"))])

etapa(id = "confirmacao", nome = "Confirmar visita", instrucao = "Repita o endereço e o período combinados e peça a confirmação do cliente.", acoes = [acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada"))])
"#;

/// Outro fluxo, válido e limpo: serve para a criação já com a primeira publicação.
const SCRIPT_ENTREGA: &str = r#"fluxo(
    id = "confirmar_entrega",
    nome = "Confirmar entrega",
    descricao = "Confirma o horário da entrega com o cliente.",
    inicio = "confirma",
    gatilho = gatilho(
        quando = "O cliente quer confirmar o horário de uma entrega.",
        nao_quando = "O cliente quer remarcar a entrega para outro dia.",
        exemplos = ["a entrega sai hoje?", "que horas chega?"],
    ),
)

etapa(
    id = "confirma",
    nome = "Confirmar",
    instrucao = "Confirme o horário previsto e pergunte se alguém recebe.",
    acoes = [
        acao("ok", "Confirmado", vai_para = fim("confirmada", resultado = "Entrega confirmada")),
    ],
)
"#;

/// Script com erro de sintaxe na linha 3 (o valor do `nome` não existe).
const SCRIPT_COM_ERRO: &str = "fluxo(\n    id = \"quebrado\",\n    nome = ,\n)\n";

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
        Self::novo_com(rotulo, |_| {}).await
    }

    /// Como [`Cenario::novo`], com um ajuste na configuração do processo (teto de fluxos, etc.).
    async fn novo_com(rotulo: &str, ajuste: impl FnOnce(&mut HarnessConfig)) -> Self {
        let carimbo = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|duracao| duracao.as_nanos())
            .unwrap_or(0);
        let pasta = std::env::temp_dir().join(format!(
            "poph-server-{rotulo}-{}-{carimbo}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");

        let mut config = HarnessConfig::default();
        config.dados.diretorio = pasta.join("dados");
        config.servidor.porta = 7481;
        config.admin.tokens_globais = vec![TokenGlobalConfig {
            label: "teste".to_owned(),
            hash: hash_argon2(TOKEN_GLOBAL),
        }];
        ajuste(&mut config);
        config.validar().expect("configuração válida");

        let store = Arc::new(Store::abrir(&config).expect("abre o banco"));
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

    fn store(&self) -> &Store {
        &self.store
    }

    async fn pedir(
        &self,
        metodo: Method,
        caminho: &str,
        token: &str,
        corpo: Option<Value>,
        if_match: Option<u64>,
    ) -> (StatusCode, Value) {
        let mut pedido = self
            .cliente
            .request(metodo, format!("{}{caminho}", self.base))
            .bearer_auth(token);
        if let Some(versao) = if_match {
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

    async fn criar_org(&self, nome: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                "/admin/v1/orgs",
                TOKEN_GLOBAL,
                Some(json!({ "name": nome, "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar org: {corpo}");
        corpo["id"].as_str().expect("id da org").to_owned()
    }

    async fn criar_token(&self, org_id: &str, rotulo: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/tokens"),
                TOKEN_GLOBAL,
                Some(json!({ "label": rotulo })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar token: {corpo}");
        corpo["value"].as_str().expect("valor do token").to_owned()
    }

    async fn criar_agente(&self, org_id: &str, token: &str, slug: &str) -> String {
        let (status, corpo) = self
            .pedir(
                Method::POST,
                &format!("/admin/v1/orgs/{org_id}/agents"),
                token,
                Some(json!({ "name": format!("Agente {slug}"), "slug": slug })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "criar agente: {corpo}");
        corpo["id"].as_str().expect("id do agente").to_owned()
    }

    /// Ficha (atendimento) aberta direto no repositório: a rota de Ficha é a FX-29/FX-30.
    fn criar_ficha(&self, org_id: &str, agente_id: &str) -> FichaId {
        let org: OrgId = org_id.parse().expect("org válida");
        let agente: AgentId = agente_id.parse().expect("agente válido");
        self.store()
            .fichas()
            .criar(
                org,
                poph_store::NovaFicha {
                    conversation_id: None,
                    agent_id: agente,
                    canal: "console".to_owned(),
                    peer_externo: Some(format!("cliente-ficticio-{}", uuid::Uuid::now_v7())),
                },
            )
            .expect("cria a Ficha")
            .id
    }

    /// Abre a execução da revisão na Ficha, com o fluxo liberado ao agente (é o atendimento em
    /// curso que o `diff` e o `DELETE` precisam ver).
    fn abrir_execucao(
        &self,
        org_id: &str,
        agente_id: &str,
        ficha: FichaId,
        fluxo_id: &str,
        revisao_id: &str,
    ) {
        let org: OrgId = org_id.parse().expect("org válida");
        let agente: AgentId = agente_id.parse().expect("agente válido");
        let fluxo: FluxoId = fluxo_id.parse().expect("fluxo válido");
        let revisao: RevisaoId = revisao_id.parse().expect("revisão válida");
        self.store()
            .fluxos()
            .liberar(org, agente, fluxo)
            .expect("libera o fluxo ao agente");
        self.store()
            .execucoes()
            .iniciar_execucao(
                org,
                poph_store::InicioDeExecucao {
                    chave_idempotencia: None,
                    ficha,
                    fluxo,
                    revisao,
                    ator: AtorEvento::Agente,
                    ator_ref: Some("atendente-de-teste"),
                    decisao_jev: None,
                    estado_inicial: poph_store::EstadoDaExecucao {
                        status: StatusExecucao::Ativa,
                        no_atual: "coleta".to_owned(),
                        dados_json: "{}".to_owned(),
                        atores_que_agiram: Vec::new(),
                        resultado: None,
                        motivo: None,
                    },
                    efeitos_de_entrada: Vec::new(),
                },
            )
            .expect("abre a execução");
    }

    /// Fluxo criado direto no repositório (para o teto não precisar de rota).
    fn criar_fluxo(&self, org_id: &str, slug: &str) -> FluxoId {
        let org: OrgId = org_id.parse().expect("org válida");
        self.store()
            .fluxos()
            .criar(
                org,
                NovoFluxo {
                    slug: slug.to_owned(),
                    nome: format!("Fluxo {slug}"),
                    descricao: String::new(),
                    rascunho_fonte: String::new(),
                },
            )
            .expect("cria o fluxo")
            .id
    }

    fn auditoria(&self, org_id: &str) -> Vec<poph_store::RegistroAuditoria> {
        let org: OrgId = org_id.parse().expect("org válida");
        self.store()
            .auditoria()
            .listar(org, Pagina::default())
            .expect("lê a auditoria")
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

/// Primeiro diagnóstico da resposta (`details.diagnostics[0]`).
fn primeiro_diagnostico(corpo: &Value) -> &Value {
    &corpo["details"]["diagnostics"][0]
}

#[tokio::test]
async fn validar_formatar_criar_e_isolamento() {
    let c = Cenario::novo("fluxos-fx27-validar").await;
    let oa = c.criar_org("Org A", "fx27-org-a").await;
    let ob = c.criar_org("Org B", "fx27-org-b").await;
    let ta = c.criar_token(&oa, "a").await;
    let tb = c.criar_token(&ob, "b").await;

    // Sem token: 401.
    let (s, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            "",
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Token de outra org no caminho: 403.
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &tb,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{corpo}");
    assert_eq!(codigo(&corpo), "forbidden");

    // Script bom: IR e nenhum diagnóstico.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/validate"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["valid"], json!(true), "{corpo}");
    assert_eq!(corpo["diagnostics"], json!([]), "{corpo}");
    assert_eq!(corpo["ir"]["schema_versao"], json!(1));
    assert_eq!(corpo["ir"]["fluxo"]["id"], json!("agendar_visita"));
    assert_eq!(corpo["ir"]["inicio"], json!("coleta"));

    // Script ruim: diagnóstico com linha e coluna, e nenhuma IR.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/validate"),
            &ta,
            Some(json!({ "source": SCRIPT_COM_ERRO })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["valid"], json!(false), "{corpo}");
    assert_eq!(corpo["ir"], Value::Null);
    let diagnostico = &corpo["diagnostics"][0];
    assert_eq!(diagnostico["codigo"], json!("FLX01"), "{corpo}");
    assert_eq!(diagnostico["severidade"], json!("erro"));
    assert_eq!(diagnostico["linha"], json!(3), "{corpo}");
    assert!(
        diagnostico["coluna"].as_u64().unwrap_or(0) >= 1,
        "coluna do diagnóstico: {diagnostico}"
    );

    // Formatar: forma canônica, e formatar de novo não muda nada.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    let canonico = corpo["source"].as_str().expect("texto canônico").to_owned();
    assert!(canonico.contains("etapa("), "{canonico}");
    // O mesmo fluxo escrito torto formata exatamente para o texto canônico.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": SCRIPT_TORTO })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_ne!(
        corpo["source"],
        json!(SCRIPT_TORTO),
        "formatar não fez nada"
    );
    assert_eq!(
        corpo["source"],
        json!(canonico),
        "formas canônicas diferentes"
    );
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": canonico })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(
        corpo["source"],
        json!(canonico),
        "formatar duas vezes difere"
    );
    // Script inválido: 422 com a lista, sem texto parcial.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows/format"),
            &ta,
            Some(json!({ "source": SCRIPT_COM_ERRO })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{corpo}");
    assert_eq!(codigo(&corpo), "fluxo_invalido");
    assert_eq!(primeiro_diagnostico(&corpo)["linha"], json!(3));

    // Criar com script inválido: 422 e nada gravado.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_COM_ERRO })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{corpo}");
    assert_eq!(codigo(&corpo), "fluxo_invalido");
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(corpo["items"], json!([]), "{corpo}");

    // Criar com o script bom: o fluxo nasce do `fluxo(...)` do script.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{corpo}");
    let fluxo = corpo["id"].as_str().expect("id do fluxo").to_owned();
    assert_eq!(corpo["slug"], json!("agendar_visita"));
    assert_eq!(corpo["name"], json!("Agendar visita"));
    assert_eq!(corpo["current_revision"], Value::Null);
    assert_eq!(corpo["version"], json!(1));
    assert_eq!(corpo["draft_source"], json!(SCRIPT_V1));

    // A lista traz o fluxo e o detalhe também.
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(corpo["items"].as_array().expect("items").len(), 1);
    assert_eq!(corpo["items"][0]["id"], json!(fluxo));
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows/{fluxo}"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["revision"], Value::Null);

    // PATCH: 428 sem If-Match, 412 com versão velha, 200 com a versão certa.
    let rota = format!("/admin/v1/orgs/{oa}/flows/{fluxo}");
    let (s, corpo) = c
        .pedir(
            Method::PATCH,
            &rota,
            &ta,
            Some(json!({ "name": "Outro nome" })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_REQUIRED, "{corpo}");
    let (s, corpo) = c
        .pedir(
            Method::PATCH,
            &rota,
            &ta,
            Some(json!({ "name": "Outro nome" })),
            Some(9),
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_FAILED, "{corpo}");
    assert_eq!(codigo(&corpo), "version_conflict");
    let (s, corpo) = c
        .pedir(
            Method::PATCH,
            &rota,
            &ta,
            Some(json!({ "name": "Visita técnica", "source": SCRIPT_V2 })),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["name"], json!("Visita técnica"));
    assert_eq!(corpo["version"], json!(2));
    assert_eq!(corpo["draft_source"], json!(SCRIPT_V2));

    // Isolamento do detalhe: fluxo da org A pela org B (token B) é 403; pela própria org B, 404.
    let (s, _) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows/{fluxo}"),
            &tb,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{ob}/flows/{fluxo}"),
            &tb,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{corpo}");
}

#[tokio::test]
async fn publicar_recusas_revisoes_imutaveis_e_diff() {
    let c = Cenario::novo("fluxos-fx27-publicar").await;
    let oa = c.criar_org("Org A", "fx27-pub-a").await;
    let ob = c.criar_org("Org B", "fx27-pub-b").await;
    let ta = c.criar_token(&oa, "a").await;
    let tb = c.criar_token(&ob, "b").await;
    let agente = c.criar_agente(&oa, &ta, "atendente").await;

    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{corpo}");
    let fluxo = corpo["id"].as_str().expect("id do fluxo").to_owned();
    let rota = format!("/admin/v1/orgs/{oa}/flows/{fluxo}");
    let revisoes = format!("{rota}/revisions");

    // Sem If-Match: 428. Versão velha: 412. Nenhuma revisão gravada nos dois.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("{rota}/publish"),
            &ta,
            Some(Value::Null),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_REQUIRED, "{corpo}");
    assert_eq!(codigo(&corpo), "precondition_required");
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("{rota}/publish"),
            &ta,
            Some(Value::Null),
            Some(7),
        )
        .await;
    assert_eq!(s, StatusCode::PRECONDITION_FAILED, "{corpo}");
    assert_eq!(codigo(&corpo), "version_conflict");
    let (s, corpo) = c.pedir(Method::GET, &revisoes, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(corpo["items"], json!([]), "{corpo}");

    // Script inválido no corpo: 422 com a lista, nada gravado, versão e rascunho intactos.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("{rota}/publish"),
            &ta,
            Some(json!({ "source": SCRIPT_COM_ERRO })),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{corpo}");
    assert_eq!(codigo(&corpo), "fluxo_invalido");
    assert_eq!(primeiro_diagnostico(&corpo)["linha"], json!(3));
    let (s, corpo) = c.pedir(Method::GET, &revisoes, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(corpo["items"], json!([]), "revisão órfã: {corpo}");
    let (s, corpo) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(corpo["version"], json!(1), "{corpo}");

    // Publica a revisão 1 a partir do rascunho.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("{rota}/publish"),
            &ta,
            Some(Value::Null),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{corpo}");
    assert_eq!(corpo["revision"]["number"], json!(1));
    assert_eq!(corpo["revision"]["schema_version"], json!(1));
    assert_eq!(corpo["version"], json!(2));
    let revisao_um_id = corpo["revision"]["id"]
        .as_str()
        .expect("id da revisão")
        .to_owned();

    // A revisão 1 lida por número traz o texto e a estrutura.
    let (s, um_antes) = c
        .pedir(Method::GET, &format!("{revisoes}/1"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{um_antes}");
    assert_eq!(um_antes["number"], json!(1));
    assert_eq!(um_antes["source"], json!(SCRIPT_V1));
    assert_eq!(
        um_antes["definition"]["fluxo"]["id"],
        json!("agendar_visita")
    );
    assert!(um_antes["source_sha256"].as_str().unwrap_or("").len() == 64);

    // Publica a revisão 2 com outro texto no corpo (sem mexer no rascunho antes).
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("{rota}/publish"),
            &ta,
            Some(json!({ "source": SCRIPT_V2 })),
            Some(2),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{corpo}");
    assert_eq!(corpo["revision"]["number"], json!(2));
    assert_eq!(corpo["version"], json!(3));

    // A revisão 1 continua idêntica: mesmo texto, mesmo hash, mesma estrutura, mesmo id.
    let (s, um_depois) = c
        .pedir(Method::GET, &format!("{revisoes}/1"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(um_depois, um_antes, "a revisão 1 mudou depois da 2");
    assert_eq!(um_depois["id"], json!(revisao_um_id));
    let (s, lista) = c.pedir(Method::GET, &revisoes, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(lista["items"].as_array().expect("items").len(), 2);
    assert_eq!(lista["items"][0]["number"], json!(1));
    assert_eq!(lista["items"][1]["number"], json!(2));
    assert_eq!(lista["next_cursor"], Value::Null);

    // Revisão inexistente: 404.
    let (s, corpo) = c
        .pedir(Method::GET, &format!("{revisoes}/9"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{corpo}");

    // Diff entre 1 e 2: a instrução mudou, e as duas revisões estão com zero atendimento ativo.
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("{rota}/diff?de=1&para=2"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["from"]["revision"], json!(1));
    assert_eq!(corpo["to"]["revision"], json!(2));
    assert_eq!(corpo["from"]["active_runs"], json!(0));
    assert_eq!(corpo["to"]["active_runs"], json!(0));
    let mudancas = corpo["changes"].as_array().expect("changes");
    assert!(!mudancas.is_empty(), "diff vazio: {corpo}");
    let instrucao = mudancas
        .iter()
        .find(|m| m["tipo"] == json!("instrucao_mudou"))
        .unwrap_or_else(|| panic!("sem `instrucao_mudou` no diff: {corpo}"));
    assert_eq!(instrucao["alvo"], json!("etapa:confirmacao"));
    assert!(
        instrucao["texto"]
            .as_str()
            .unwrap_or("")
            .contains("confirmação"),
        "frase do diff: {instrucao}"
    );

    // Preview da etapa: o bloco do prompt (FX-18) com a instrução daquela etapa.
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("{rota}/preview?etapa=coleta"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["stage"], json!("coleta"));
    assert_eq!(corpo["revision"], json!(2));
    let prompt = corpo["prompt"].as_str().expect("bloco do prompt");
    assert!(prompt.contains("Pergunte o endereço"), "{prompt}");
    assert!(prompt.contains("Coletar dados"), "{prompt}");
    // Sem `etapa`, mostra a etapa de início; etapa inexistente é 404.
    let (s, corpo) = c
        .pedir(Method::GET, &format!("{rota}/preview"), &ta, None, None)
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["stage"], json!("coleta"));
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("{rota}/preview?etapa=nao_existe"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{corpo}");

    // Atendimento em curso na revisão 1: o diff da 1 para a 2 passa a contar 1 ativa na 1.
    let ficha = c.criar_ficha(&oa, &agente);
    let (s, detalhe) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK, "{detalhe}");
    assert_eq!(detalhe["revision"]["number"], json!(2));
    c.abrir_execucao(&oa, &agente, ficha, &fluxo, &revisao_um_id);
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("{rota}/diff?de=1&para=2"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["from"]["active_runs"], json!(1), "{corpo}");
    assert_eq!(corpo["to"]["active_runs"], json!(0), "{corpo}");

    // Isolamento: fluxo da org A pela org B.
    let (s, corpo) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{ob}/flows/{fluxo}/revisions"),
            &tb,
            None,
            None,
        )
        .await;
    assert!(!s.is_success(), "{corpo}");
}

#[tokio::test]
async fn excluir_com_execucao_ativa_exige_force_e_audita() {
    let c = Cenario::novo("fluxos-fx27-excluir").await;
    let oa = c.criar_org("Org A", "fx27-del-a").await;
    let ta = c.criar_token(&oa, "a").await;
    let agente = c.criar_agente(&oa, &ta, "atendente").await;

    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{corpo}");
    let fluxo = corpo["id"].as_str().expect("id do fluxo").to_owned();
    let rota = format!("/admin/v1/orgs/{oa}/flows/{fluxo}");

    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("{rota}/publish"),
            &ta,
            Some(Value::Null),
            Some(1),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{corpo}");
    let revisao = corpo["revision"]["id"]
        .as_str()
        .expect("id da revisão")
        .to_owned();

    let ficha = c.criar_ficha(&oa, &agente);
    c.abrir_execucao(&oa, &agente, ficha, &fluxo, &revisao);

    // Sem `force`: 409 com a contagem, e o fluxo continua lá.
    let (s, corpo) = c.pedir(Method::DELETE, &rota, &ta, None, None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{corpo}");
    assert_eq!(corpo["details"]["active_runs"], json!(1), "{corpo}");
    let (s, _) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(s, StatusCode::OK);

    // `force` com valor inválido: 400.
    let (s, corpo) = c
        .pedir(
            Method::DELETE,
            &format!("{rota}?force=talvez"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{corpo}");

    // Com `force=true`: exclui, abandona a execução e audita.
    let (s, corpo) = c
        .pedir(
            Method::DELETE,
            &format!("{rota}?force=true"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corpo}");
    assert_eq!(corpo["deleted"], json!(true));
    assert_eq!(corpo["abandoned_runs"], json!(1), "{corpo}");
    let (s, corpo) = c.pedir(Method::GET, &rota, &ta, None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{corpo}");
    let exclusoes: Vec<_> = c
        .auditoria(&oa)
        .into_iter()
        .filter(|registro| registro.acao == "flow_delete")
        .collect();
    assert_eq!(exclusoes.len(), 1, "a exclusão tem de ser auditada");
    assert_eq!(exclusoes[0].entidade, "flow");
    assert_eq!(exclusoes[0].entidade_id.as_deref(), Some(fluxo.as_str()));

    // A auditoria da publicação também ficou.
    let publicacoes: Vec<_> = c
        .auditoria(&oa)
        .into_iter()
        .filter(|registro| registro.acao == "flow_publish")
        .collect();
    assert_eq!(publicacoes.len(), 1);
}

#[tokio::test]
async fn teto_de_fluxos_por_org_vale_na_criacao() {
    let c = Cenario::novo_com("fluxos-fx27-teto", |config| {
        config.fluxos.max_fluxos_por_org = 1;
    })
    .await;
    let oa = c.criar_org("Org A", "fx27-teto-a").await;
    let ta = c.criar_token(&oa, "a").await;

    // Um fluxo já cadastrado (pelo repositório, para o teste não depender da rota de criação).
    c.criar_fluxo(&oa, "fluxo_existente");

    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_V1 })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{corpo}");
    assert_eq!(codigo(&corpo), "teto_de_fluxos_por_org");
    assert_eq!(corpo["details"]["flows"], json!(1));
    assert_eq!(corpo["details"]["limit"], json!(1));
}

#[tokio::test]
async fn criar_com_a_primeira_publicacao() {
    let c = Cenario::novo("fluxos-fx27-criar-pub").await;
    let oa = c.criar_org("Org A", "fx27-criar-a").await;
    let ta = c.criar_token(&oa, "a").await;

    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/fluxos"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "rota de fluxo não é a de Ficha: {corpo}"
    );

    // Criação com `publish: true`: nasce com a revisão 1 e a versão já avançada pela publicação.
    let (s, corpo) = c
        .pedir(
            Method::POST,
            &format!("/admin/v1/orgs/{oa}/flows"),
            &ta,
            Some(json!({ "source": SCRIPT_ENTREGA, "publish": true })),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{corpo}");
    let fluxo = corpo["id"].as_str().expect("id do fluxo").to_owned();
    assert_eq!(corpo["slug"], json!("confirmar_entrega"));
    assert_eq!(corpo["current_revision"], json!(1));
    assert_eq!(corpo["version"], json!(2));
    assert_eq!(corpo["revision"]["number"], json!(1));

    // A revisão 1 existe e traz o texto publicado.
    let (s, revisao) = c
        .pedir(
            Method::GET,
            &format!("/admin/v1/orgs/{oa}/flows/{fluxo}/revisions/1"),
            &ta,
            None,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{revisao}");
    assert_eq!(revisao["source"], json!(SCRIPT_ENTREGA));
    assert_eq!(
        revisao["definition"]["fluxo"]["id"],
        json!("confirmar_entrega")
    );
    assert_eq!(revisao["published_by"], json!("admin_token"));
}
