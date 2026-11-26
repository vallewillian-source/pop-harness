//! Registro das chamadas de ferramenta no turno (HR-47b).
//!
//! Prova o que o painel do por quê precisa e o que a tarefa exige:
//! 1. chamada executada grava argumentos, resultado, duração e rodada;
//! 2. chamada de ferramenta não habilitada grava recusa com o motivo, e o modelo recebe o erro;
//! 3. chamada de nome que o registro não tem grava recusa `desconhecida`;
//! 4. resumo de argumentos e de resultado respeita o teto, com marca de corte;
//! 5. argumento com nome de segredo é redigido no registro;
//! 6. turno sem ferramenta grava lista de chamadas vazia;
//! 7. `nomes` continua sendo só as chamadas executadas (contrato antigo de `tools`).
//!
//! Provedor falso e pausa que não dorme: nenhum teste toca a rede.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    ContextoFerramenta, FerramentaInterna, FuturoFerramenta, FuturoPausa, MARCA_DE_CORTE,
    MOTIVO_DESCONHECIDA, MOTIVO_NAO_HABILITADA, Motor, ObservadorNulo, OpcoesTurno, Pausa,
    RegistroPadrao, TETO_RESUMO_FERRAMENTA,
};
use poph_core::llm::{
    FakeProvider, FinishReason, LlmEvent, LlmProvider, Message, Script, ToolCall,
};
use poph_store::{NewAgent, NewConversation, NewMessage, NewOrg, Page, Store, Turno};
use poph_types::{AgentId, ConversationId, OrgId, StructuredInstructions, ToolConfig};

/// Pausa que não dorme: a janela de agrupamento é irrelevante aqui.
#[derive(Default, Clone)]
struct PausaFalsa;

impl Pausa for PausaFalsa {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// O que a ferramenta falsa faz quando o modelo a chama.
#[derive(Clone)]
enum Comportamento {
    /// Devolve este texto.
    Texto(String),
    /// Espera este tanto e devolve o texto.
    ComAtraso(String, u64),
    /// Devolve este erro ao modelo.
    Erro(String),
}

/// Ferramenta falsa: existe só para o turno exercitar o registro.
struct FerramentaFalsa {
    nome: &'static str,
    comportamento: Comportamento,
}

impl FerramentaInterna for FerramentaFalsa {
    fn nome(&self) -> &'static str {
        self.nome
    }

    fn descricao(&self) -> &'static str {
        "ferramenta de teste"
    }

    fn parametros(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"consulta": {"type": "string"}},
            "required": ["consulta"]
        })
    }

    fn executar(
        &self,
        _contexto: ContextoFerramenta,
        _argumentos: serde_json::Value,
    ) -> FuturoFerramenta {
        let comportamento = self.comportamento.clone();
        Box::pin(async move {
            match comportamento {
                Comportamento::Texto(texto) => Ok(texto),
                Comportamento::ComAtraso(texto, ms) => {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    Ok(texto)
                }
                Comportamento::Erro(erro) => Err(erro),
            }
        })
    }
}

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
}

impl Cenario {
    fn novo(rotulo: &str, habilitadas: &[&str]) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-registro-ferramentas-{rotulo}-{}-{}",
            std::process::id(),
            AgentId::generate()
        ));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Atendente {rotulo}"),
                    slug: format!("atendente-{rotulo}"),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let estruturado = StructuredInstructions {
            tools: habilitadas
                .iter()
                .map(|nome| ToolConfig {
                    name: (*nome).to_owned(),
                    enabled: true,
                    when_to_use: String::new(),
                })
                .collect(),
            ..StructuredInstructions::default()
        };
        store
            .instructions()
            .save(
                org,
                agente,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        let conversa = store
            .conversations()
            .create(
                org,
                NewConversation {
                    agent_id: agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa")
            .id;
        Self {
            pasta,
            store,
            org,
            conversa,
        }
    }

    fn motor(&self, roteiros: Vec<Script>, registro: RegistroPadrao) -> (Motor, Arc<FakeProvider>) {
        let fake = Arc::new(FakeProvider::with_scripts(roteiros));
        let provedor: Arc<dyn LlmProvider> = fake.clone();
        let config = ConfigQuente::nova(
            Arc::clone(&self.store),
            OpcoesConfigQuente::default(),
            MetricasMemoria::proprio(),
        );
        let limitador = Arc::new(Limitador::novo(
            OpcoesLimites::default(),
            MetricasMemoria::proprio(),
        ));
        let motor = Motor::novo(
            Arc::clone(&self.store),
            config,
            limitador,
            provedor,
            OpcoesTurno::default(),
            MetricasTurno::proprio(),
        )
        .com_pausa(Arc::new(PausaFalsa))
        .com_ferramentas(Arc::new(registro));
        (motor, fake)
    }

    fn falar(&self, texto: &str) -> poph_types::MessageId {
        self.store
            .messages()
            .write(
                self.org,
                NewMessage {
                    conversation_id: self.conversa,
                    direction: poph_store::Direction::Inbound,
                    role: poph_store::Role::User,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    external_id: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
            )
            .expect("grava a mensagem")
            .id
    }

    fn turno(&self) -> Turno {
        let turnos = self
            .store
            .turns()
            .list(self.org, self.conversa, Page::default())
            .expect("lista os turnos");
        assert_eq!(turnos.len(), 1, "um turno por mensagem");
        turnos.into_iter().next().expect("o turno")
    }

    async fn rodar(&self, motor: &Motor, fala: &str) -> serde_json::Value {
        let mensagem = self.falar(fala);
        motor
            .executar(
                poph_agent::turno::Entrada::nova(self.org, self.conversa, mensagem),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda");
        let turno = self.turno();
        serde_json::from_str(&turno.tools_json).expect("JSON do registro de ferramentas")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn resposta(texto: &str) -> Script {
    Script::Events(vec![
        LlmEvent::Texto(texto.to_owned()),
        LlmEvent::Fim(FinishReason::Completed),
    ])
}

fn chamada(nome: &str, argumentos: serde_json::Value) -> Script {
    Script::Events(vec![
        LlmEvent::Tool(ToolCall {
            id: "c1".to_owned(),
            nome: nome.to_owned(),
            arguments: argumentos,
        }),
        LlmEvent::Fim(FinishReason::ToolCalls),
    ])
}

fn registro_consultar_base(comportamento: Comportamento) -> RegistroPadrao {
    RegistroPadrao::novo().com(Arc::new(FerramentaFalsa {
        nome: "consultar_base",
        comportamento,
    }))
}

fn registro_consultar_base_e_carrinho() -> RegistroPadrao {
    RegistroPadrao::novo()
        .com(Arc::new(FerramentaFalsa {
            nome: "consultar_base",
            comportamento: Comportamento::Texto("ok".to_owned()),
        }))
        .com(Arc::new(FerramentaFalsa {
            nome: "cart_add_item",
            comportamento: Comportamento::Texto("ok".to_owned()),
        }))
}

#[tokio::test]
async fn chamada_executada_grava_argumentos_resultado_duracao_e_rodada() {
    let cenario = Cenario::novo("executada", &["consultar_base"]);
    let (motor, _) = cenario.motor(
        vec![
            chamada(
                "consultar_base",
                serde_json::json!({"consulta": "prazo de entrega"}),
            ),
            resposta("O prazo é de cinco dias úteis."),
        ],
        registro_consultar_base(Comportamento::ComAtraso(
            "Prazo de 5 dias úteis.".to_owned(),
            20,
        )),
    );

    let registro = cenario.rodar(&motor, "qual o prazo?").await;
    assert_eq!(registro["nomes"], serde_json::json!(["consultar_base"]));
    let chamadas = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(chamadas.len(), 1);
    let item = &chamadas[0];
    assert_eq!(item["nome"], "consultar_base");
    assert_eq!(item["estado"], "executada");
    assert!(item["motivo"].is_null(), "executada não tem motivo");
    assert_eq!(item["rodada"], 0);
    assert_eq!(item["ordem"], 1);
    assert!(
        item["duracao_ms"].as_u64().expect("duração") >= 5,
        "a duração mede a execução: {item}"
    );
    assert!(
        item["argumentos"]
            .as_str()
            .expect("argumentos")
            .contains("prazo de entrega")
    );
    assert_eq!(item["resultado"], "Prazo de 5 dias úteis.");
}

#[tokio::test]
async fn ferramenta_nao_habilitada_grava_recusa_e_o_modelo_recebe_o_erro() {
    let cenario = Cenario::novo("nao-habilitada", &["consultar_base"]);
    let (motor, fake) = cenario.motor(
        vec![
            chamada(
                "cart_add_item",
                serde_json::json!({"produto_id": "qualquer", "quantidade": 1}),
            ),
            resposta("Não consigo montar o carrinho."),
        ],
        registro_consultar_base_e_carrinho(),
    );

    let registro = cenario.rodar(&motor, "põe no carrinho").await;
    let chamadas = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(chamadas.len(), 1);
    assert_eq!(chamadas[0]["estado"], "recusada");
    assert_eq!(chamadas[0]["motivo"], MOTIVO_NAO_HABILITADA);
    assert_eq!(chamadas[0]["duracao_ms"], 0);
    // `nomes` mantém o comportamento anterior: lista toda chamada que chegou ao laço, inclusive a
    // recusada. Quem separa executada de recusada é `tool_calls`.
    assert_eq!(registro["nomes"], serde_json::json!(["cart_add_item"]));

    // O modelo recebeu o erro como resultado da chamada, não a execução.
    let pedidos = fake.received();
    assert_eq!(pedidos.len(), 2);
    let erro = pedidos[1]
        .messages
        .iter()
        .find_map(|mensagem| match mensagem {
            Message::Resultado {
                content: conteudo, ..
            } => Some(conteudo.clone()),
            _ => None,
        })
        .expect("o resultado da chamada vai para o modelo");
    assert!(
        erro.contains("não está habilitada"),
        "erro ao modelo: {erro}"
    );
}

#[tokio::test]
async fn ferramenta_desconhecida_grava_recusa_com_motivo_proprio() {
    let cenario = Cenario::novo("desconhecida", &["cart_add_item"]);
    // O agente habilita `cart_add_item`, mas o registro do processo não a tem.
    let (motor, _) = cenario.motor(
        vec![
            chamada("cart_add_item", serde_json::json!({"produto_id": "x"})),
            resposta("Sigo sem o carrinho."),
        ],
        registro_consultar_base(Comportamento::Texto("ok".to_owned())),
    );

    let registro = cenario.rodar(&motor, "põe no carrinho").await;
    let chamadas = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(chamadas.len(), 1);
    assert_eq!(chamadas[0]["estado"], "recusada");
    assert_eq!(chamadas[0]["motivo"], MOTIVO_DESCONHECIDA);
}

#[tokio::test]
async fn erro_de_argumento_da_ferramenta_vira_motivo_argumento_invalido() {
    let cenario = Cenario::novo("argumento", &["consultar_base"]);
    let (motor, _) = cenario.motor(
        vec![
            chamada("consultar_base", serde_json::json!({})),
            resposta("Não encontrei."),
        ],
        registro_consultar_base(Comportamento::Erro(
            "erro: o argumento `consulta` é obrigatório".to_owned(),
        )),
    );

    let registro = cenario.rodar(&motor, "consulta").await;
    let chamadas = registro["chamadas"].as_array().expect("lista de chamadas");
    assert_eq!(chamadas[0]["estado"], "recusada");
    assert_eq!(chamadas[0]["motivo"], "argumento_invalido");
    assert!(
        chamadas[0]["resultado"]
            .as_str()
            .expect("resultado")
            .contains("obrigatório")
    );
}

#[tokio::test]
async fn argumentos_e_resultado_respeitam_o_teto_de_tamanho() {
    let cenario = Cenario::novo("teto", &["consultar_base"]);
    let argumento_grande = "consulta".repeat(500);
    let resultado_grande = "retorno".repeat(500);
    let (motor, _) = cenario.motor(
        vec![
            chamada(
                "consultar_base",
                serde_json::json!({"consulta": argumento_grande}),
            ),
            resposta("Certo."),
        ],
        registro_consultar_base(Comportamento::Texto(resultado_grande)),
    );

    let registro = cenario.rodar(&motor, "consulta grande").await;
    let chamadas = registro["chamadas"].as_array().expect("lista de chamadas");
    let argumentos = chamadas[0]["argumentos"].as_str().expect("argumentos");
    let resultado = chamadas[0]["resultado"].as_str().expect("resultado");
    assert!(argumentos.ends_with(MARCA_DE_CORTE), "argumentos cortados");
    assert!(resultado.ends_with(MARCA_DE_CORTE), "resultado cortado");
    assert_eq!(
        argumentos.chars().count(),
        TETO_RESUMO_FERRAMENTA + MARCA_DE_CORTE.chars().count()
    );
    assert_eq!(
        resultado.chars().count(),
        TETO_RESUMO_FERRAMENTA + MARCA_DE_CORTE.chars().count()
    );
    // O conteúdo inteiro não fica no registro do turno.
    assert!(!registro.to_string().contains(&"consulta".repeat(500)));
}

#[tokio::test]
async fn argumento_com_nome_de_segredo_nao_vai_para_o_registro() {
    let cenario = Cenario::novo("segredo", &["consultar_base"]);
    let (motor, _) = cenario.motor(
        vec![
            chamada(
                "consultar_base",
                serde_json::json!({
                    "consulta": "prazo",
                    "token": "segredo-que-nao-pode-vazar",
                    "api_key": "chave-que-nao-pode-vazar",
                }),
            ),
            resposta("Prazo de 5 dias."),
        ],
        registro_consultar_base(Comportamento::Texto("ok".to_owned())),
    );

    let registro = cenario.rodar(&motor, "qual o prazo?").await;
    let texto = registro.to_string();
    assert!(texto.contains("prazo"));
    assert!(texto.contains("[redigido]"));
    for proibido in ["segredo-que-nao-pode-vazar", "chave-que-nao-pode-vazar"] {
        assert!(!texto.contains(proibido), "vazou `{proibido}`: {texto}");
    }
}

#[tokio::test]
async fn turno_sem_ferramenta_grava_lista_de_chamadas_vazia() {
    let cenario = Cenario::novo("sem-ferramenta", &["consultar_base"]);
    let (motor, _) = cenario.motor(
        vec![resposta("Olá! Como posso ajudar?")],
        registro_consultar_base(Comportamento::Texto("ok".to_owned())),
    );

    let registro = cenario.rodar(&motor, "oi").await;
    assert_eq!(registro["nomes"], serde_json::json!([]));
    assert_eq!(registro["chamadas"], serde_json::json!([]));
}
