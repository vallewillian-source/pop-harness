//! Carrinho de orçamento (HR-19).
//!
//! Cobre o que a tarefa pede: totais corretos em 10 casos (quantidade, preço zero, item repetido,
//! item removido, carrinho vazio), o modelo não consegue gravar preço, agente sem a ferramenta não a
//! recebe, o carrinho é por conversa, e a org de outro token não mexe nele.
//!
//! Provedor falso e pausa que não dorme: nenhum teste toca a rede nem espera de verdade.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::carrinho::formatar_reais;
use poph_agent::carrinho::{self, NOME_ADICIONAR, NOME_LIMPAR, NOME_REMOVER, NOME_RESUMO};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    ContextoFerramenta, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa,
    RegistroFerramentas, RegistroPadrao, UsoFerramentas,
};
use poph_core::llm::{ChamadaFerramenta, EventoLlm, LlmProvider, MotivoFim, ProvedorFake, Roteiro};
use poph_store::{NewAgent, NewCategory, NewConversation, NewOrg, NewProduct, Page, Store};
use poph_types::{
    AgentId, ConversationId, OrgId, ProductId, StructuredInstructions, ToolConfig, TurnId,
};
use serde_json::{Value, json};

/// Pausa que não dorme.
#[derive(Default, Clone)]
struct PausaFalsa;

impl Pausa for PausaFalsa {
    fn esperar(&self, _duracao: Duration) -> FuturoPausa {
        Box::pin(async {})
    }
}

/// Catálogo de teste: quatro produtos com preços conhecidos e um fora das categorias do agente.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
    /// Dedetização R$ 320,00; desentupimento R$ 90,00; visita grátis R$ 0,00; projeto R$ 1.234,56.
    produtos: Vec<ProductId>,
    /// Produto da categoria "Jardim", que o agente não enxerga quando ele tem vínculo com "Pragas".
    jardim: ProductId,
}

impl Cenario {
    fn novo(rotulo: &str, ferramentas: &[&str]) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-carrinho-{rotulo}-{}-{}",
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
            .criar(NewOrg {
                name: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agents()
            .criar(
                org,
                NewAgent {
                    name: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}"),
                    grouping_window_ms: 0,
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente")
            .id;
        let estruturado = StructuredInstructions {
            tools: ferramentas
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
            .salvar(
                org,
                agente,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        let pragas = store
            .categories()
            .criar(
                org,
                NewCategory {
                    name: "Pragas".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        let jardim_categoria = store
            .categories()
            .criar(
                org,
                NewCategory {
                    name: "Jardim".to_owned(),
                    description: String::new(),
                },
            )
            .expect("cria a categoria")
            .id;
        let mut produtos = Vec::new();
        for (nome, centavos) in [
            ("Dedetização de baratas", 32_000),
            ("Desentupimento", 9_000),
            ("Visita técnica gratuita", 0),
            ("Projeto completo", 123_456),
        ] {
            let produto = store
                .products()
                .criar(
                    org,
                    NewProduct {
                        category_id: Some(pragas),
                        name: nome.to_owned(),
                        price_text: String::new(),
                        price_cents: centavos,
                        unit: "por serviço".to_owned(),
                        ..NewProduct::default()
                    },
                )
                .expect("cria o produto");
            produtos.push(produto.id);
        }
        let jardim = store
            .products()
            .criar(
                org,
                NewProduct {
                    category_id: Some(jardim_categoria),
                    name: "Poda de árvores".to_owned(),
                    price_cents: 15_000,
                    ..NewProduct::default()
                },
            )
            .expect("cria o produto de jardim")
            .id;
        let conversa = Self::abrir_conversa(&store, org, agente);
        Self {
            pasta,
            store,
            org,
            agente,
            conversa,
            produtos,
            jardim,
        }
    }

    fn abrir_conversa(store: &Store, org: OrgId, agente: AgentId) -> ConversationId {
        store
            .conversas()
            .criar(
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
            .id
    }

    fn contexto(&self, conversa: ConversationId) -> ContextoFerramenta {
        ContextoFerramenta {
            org: self.org,
            conversa,
            agente: self.agente,
            turno: TurnId::generate(),
            uso: UsoFerramentas::nova(),
        }
    }

    /// Chama uma ferramenta do carrinho direto, como o motor faria.
    async fn chamar(
        &self,
        conversa: ConversationId,
        nome: &str,
        argumentos: Value,
    ) -> Result<String, String> {
        let registro = carrinho::registrar(RegistroPadrao::novo(), Arc::clone(&self.store));
        let ferramenta = registro.ferramenta(nome).expect("ferramenta registrada");
        ferramenta
            .executar(self.contexto(conversa), argumentos)
            .await
    }

    fn total(&self, conversa: ConversationId) -> (i64, usize) {
        let resumo = self
            .store
            .carts()
            .resumo(self.org, conversa)
            .expect("lê o carrinho");
        (resumo.total_centavos, resumo.itens.len())
    }

    fn motor(&self, roteiros: Vec<Roteiro>) -> (Motor, Arc<ProvedorFake>) {
        let fake = Arc::new(ProvedorFake::com_roteiros(roteiros));
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
        .com_pausa(Arc::new(PausaFalsa));
        let registro = carrinho::registrar(RegistroPadrao::novo(), Arc::clone(&self.store));
        let motor = motor.com_ferramentas(Arc::new(registro) as Arc<dyn RegistroFerramentas>);
        (motor, fake)
    }

    fn falar(&self, conversa: ConversationId, texto: &str) -> poph_types::MessageId {
        self.store
            .messages()
            .gravar(
                self.org,
                poph_store::NewMessage {
                    conversation_id: conversa,
                    direction: poph_store::Direction::Inbound,
                    role: poph_store::Role::User,
                    texto: texto.to_owned(),
                    attachments_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                    origin: poph_store::Origin::Agente,
                    author_name: None,
                },
            )
            .expect("grava a mensagem")
            .id
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn resposta(texto: &str) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Texto(texto.to_owned()),
        EventoLlm::Fim(MotivoFim::Concluido),
    ])
}

fn chamada(nome: &str, id: &str, argumentos: Value) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Ferramenta(ChamadaFerramenta {
            id: id.to_owned(),
            nome: nome.to_owned(),
            argumentos,
        }),
        EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
    ])
}

/// Operação de um caso de total.
enum Op {
    Add(usize, Option<u32>),
    Rem(usize, Option<u32>),
}

#[tokio::test]
async fn dez_casos_de_total_batem_com_o_servidor() {
    // (nome, operações, total esperado em centavos, linhas esperadas)
    let casos: Vec<(&str, Vec<Op>, i64, usize)> = vec![
        (
            "um item, quantidade padrão",
            vec![Op::Add(0, None)],
            32_000,
            1,
        ),
        (
            "quantidade maior que 1",
            vec![Op::Add(0, Some(3))],
            96_000,
            1,
        ),
        ("preço zero", vec![Op::Add(2, Some(2))], 0, 1),
        (
            "produto repetido soma na mesma linha",
            vec![Op::Add(0, Some(1)), Op::Add(0, Some(2))],
            96_000,
            1,
        ),
        (
            "dois produtos diferentes",
            vec![Op::Add(0, Some(2)), Op::Add(1, Some(1))],
            73_000,
            2,
        ),
        (
            "remove parte da quantidade",
            vec![Op::Add(0, Some(3)), Op::Rem(0, Some(1))],
            64_000,
            1,
        ),
        (
            "remove a linha inteira",
            vec![Op::Add(0, Some(3)), Op::Rem(0, None)],
            0,
            0,
        ),
        (
            "item removido e outro fica",
            vec![Op::Add(0, None), Op::Add(1, None), Op::Rem(0, None)],
            9_000,
            1,
        ),
        (
            "valor alto com milhar",
            vec![Op::Add(3, Some(2))],
            246_912,
            1,
        ),
        ("carrinho vazio", vec![], 0, 0),
    ];
    assert_eq!(casos.len(), 10);

    let cenario = Cenario::novo("totais", &[NOME_ADICIONAR, NOME_REMOVER, NOME_RESUMO]);
    for (nome, operacoes, total_esperado, linhas_esperadas) in casos {
        let conversa = Cenario::abrir_conversa(&cenario.store, cenario.org, cenario.agente);
        for operacao in operacoes {
            let (nome_ferramenta, argumentos) = match operacao {
                Op::Add(indice, quantidade) => (
                    NOME_ADICIONAR,
                    produto_e_quantidade(cenario.produtos[indice], quantidade),
                ),
                Op::Rem(indice, quantidade) => (
                    NOME_REMOVER,
                    produto_e_quantidade(cenario.produtos[indice], quantidade),
                ),
            };
            cenario
                .chamar(conversa, nome_ferramenta, argumentos)
                .await
                .unwrap_or_else(|erro| panic!("{nome}: a ferramenta recusou: {erro}"));
        }
        let (total, linhas) = cenario.total(conversa);
        assert_eq!(total, total_esperado, "caso: {nome} (total)");
        assert_eq!(linhas, linhas_esperadas, "caso: {nome} (linhas)");

        // O texto que vai ao modelo traz o mesmo total que o banco calculou.
        let texto = cenario
            .chamar(conversa, NOME_RESUMO, json!({}))
            .await
            .expect("resumo");
        assert!(
            texto.contains(&format!("Total: {}", formatar_reais(total_esperado))),
            "caso: {nome}: texto sem o total certo: {texto}"
        );
    }
}

fn produto_e_quantidade(produto: ProductId, quantidade: Option<u32>) -> Value {
    match quantidade {
        Some(quantidade) => json!({"produto_id": produto.to_string(), "quantidade": quantidade}),
        None => json!({"produto_id": produto.to_string()}),
    }
}

#[tokio::test]
async fn ferramenta_recusa_preco_e_nao_grava_nada() {
    let cenario = Cenario::novo("recusa", &[NOME_ADICIONAR]);
    let conversa = cenario.conversa;
    let produto = cenario.produtos[0].to_string();

    for argumentos in [
        json!({"produto_id": produto, "preco_centavos": 1}),
        json!({"produto_id": produto, "quantidade": 1, "price": 1}),
        json!({"produto_id": produto, "total": 1}),
    ] {
        let erro = cenario
            .chamar(conversa, NOME_ADICIONAR, argumentos)
            .await
            .expect_err("preço não pode entrar");
        assert!(erro.contains("não é aceito"), "{erro}");
        assert!(erro.contains("catálogo"), "{erro}");
    }
    assert_eq!(cenario.total(conversa), (0, 0), "nada foi gravado");
}

#[test]
fn esquema_da_ferramenta_so_tem_id_e_quantidade() {
    let registro = carrinho::registrar(RegistroPadrao::novo(), Arc::new(armazem_vazio()));
    let ferramenta = registro.ferramenta(NOME_ADICIONAR).expect("registrada");
    let propriedades: Vec<String> = ferramenta.parametros()["properties"]
        .as_object()
        .expect("objeto")
        .keys()
        .cloned()
        .collect();
    assert_eq!(propriedades.len(), 2);
    assert!(propriedades.contains(&"produto_id".to_owned()));
    assert!(propriedades.contains(&"quantidade".to_owned()));
    assert_eq!(
        ferramenta.parametros()["additionalProperties"],
        json!(false)
    );
}

/// Armazém vazio só para olhar o esquema da ferramenta.
fn armazem_vazio() -> Store {
    let pasta = std::env::temp_dir().join(format!("poph-carrinho-esquema-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).expect("pasta");
    Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("banco")
}

#[tokio::test]
async fn llm_manda_preco_no_turno_e_o_total_continua_do_catalogo() {
    let cenario = Cenario::novo("turno-preco", &[NOME_ADICIONAR, NOME_RESUMO]);
    let produto = cenario.produtos[0].to_string();
    let (motor, fake) = cenario.motor(vec![
        // 1ª tentativa: o modelo tenta mandar o preço junto.
        chamada(
            NOME_ADICIONAR,
            "c1",
            json!({"produto_id": produto, "quantidade": 1, "preco_centavos": 1}),
        ),
        // 2ª tentativa: sem preço.
        chamada(
            NOME_ADICIONAR,
            "c2",
            json!({"produto_id": produto, "quantidade": 1}),
        ),
        resposta("Anotado, a dedetização está no orçamento."),
    ]);
    let mensagem = cenario.falar(cenario.conversa, "quero a dedetização");
    let resumo = motor
        .executar(
            poph_agent::turno::Entrada::nova(cenario.org, cenario.conversa, mensagem),
            &mut ObservadorNulo,
        )
        .await
        .expect("o turno roda");
    assert_eq!(resumo.estado.as_text(), "done");

    // O erro da primeira tentativa volta ao modelo como resultado de ferramenta.
    let pedidos = fake.recebidos();
    let resultado = pedidos[1]
        .mensagens
        .iter()
        .find_map(|mensagem| match mensagem {
            poph_core::llm::Mensagem::Resultado { id, conteudo } if id == "c1" => {
                Some(conteudo.clone())
            }
            _ => None,
        })
        .expect("resultado da primeira chamada");
    assert!(resultado.contains("não é aceito"), "{resultado}");

    // O carrinho tem uma unidade pelo preço do catálogo, não o que o modelo mandou.
    assert_eq!(cenario.total(cenario.conversa), (32_000, 1));
}

#[tokio::test]
async fn agente_sem_a_ferramenta_nao_a_recebe() {
    // Só o resumo está habilitado: o modelo não deve ver a de adicionar.
    let cenario = Cenario::novo("habilitacao", &[NOME_RESUMO]);
    let (motor, fake) = cenario.motor(vec![resposta("Seu carrinho está vazio.")]);
    let mensagem = cenario.falar(cenario.conversa, "quanto deu até agora?");
    motor
        .executar(
            poph_agent::turno::Entrada::nova(cenario.org, cenario.conversa, mensagem),
            &mut ObservadorNulo,
        )
        .await
        .expect("o turno roda");
    let nomes: Vec<String> = fake.recebidos()[0]
        .ferramentas
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect();
    assert_eq!(nomes, vec![NOME_RESUMO.to_owned()]);
}

#[tokio::test]
async fn agente_com_as_quatro_recebe_as_quatro() {
    let cenario = Cenario::novo(
        "quatro",
        &[NOME_ADICIONAR, NOME_REMOVER, NOME_RESUMO, NOME_LIMPAR],
    );
    let (motor, fake) = cenario.motor(vec![resposta("ok")]);
    let mensagem = cenario.falar(cenario.conversa, "oi");
    motor
        .executar(
            poph_agent::turno::Entrada::nova(cenario.org, cenario.conversa, mensagem),
            &mut ObservadorNulo,
        )
        .await
        .expect("o turno roda");
    let mut nomes: Vec<String> = fake.recebidos()[0]
        .ferramentas
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect();
    nomes.sort();
    assert_eq!(
        nomes,
        vec![NOME_ADICIONAR, NOME_LIMPAR, NOME_REMOVER, NOME_RESUMO]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn carrinho_e_por_conversa_e_conversa_nova_comeca_vazia() {
    let cenario = Cenario::novo("conversas", &[NOME_ADICIONAR, NOME_RESUMO]);
    let primeira = cenario.conversa;
    cenario
        .chamar(
            primeira,
            NOME_ADICIONAR,
            produto_e_quantidade(cenario.produtos[1], Some(2)),
        )
        .await
        .expect("adiciona");
    assert_eq!(cenario.total(primeira), (18_000, 1));

    let nova = Cenario::abrir_conversa(&cenario.store, cenario.org, cenario.agente);
    assert_eq!(cenario.total(nova), (0, 0), "conversa nova começa vazia");
    let texto = cenario
        .chamar(nova, NOME_RESUMO, json!({}))
        .await
        .expect("resumo");
    assert!(texto.contains("vazio"), "{texto}");
    assert_eq!(
        cenario.total(primeira),
        (18_000, 1),
        "a antiga continua com o item"
    );
}

#[tokio::test]
async fn org_de_outro_token_nao_mexe_no_carrinho() {
    let cenario = Cenario::novo("isolamento", &[NOME_ADICIONAR]);
    let conversa = cenario.conversa;
    cenario
        .chamar(
            conversa,
            NOME_ADICIONAR,
            produto_e_quantidade(cenario.produtos[0], None),
        )
        .await
        .expect("adiciona");

    let outra = cenario
        .store
        .orgs()
        .criar(NewOrg {
            name: "Outra".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org")
        .id;
    // Conversa da primeira org não é lida nem alterada por quem tem a segunda.
    let resumo = cenario.store.carts().resumo(outra, conversa).expect("lê");
    assert!(resumo.itens.is_empty());
    assert!(resumo.carrinho.is_none());
    let erro = cenario
        .store
        .carts()
        .adicionar(outra, conversa, cenario.agente, cenario.produtos[1], 1)
        .expect_err("org alheia não adiciona");
    assert!(erro.is_not_found(), "{erro}");
    assert_eq!(
        cenario.total(conversa),
        (32_000, 1),
        "o carrinho da dona não mudou"
    );
}

#[tokio::test]
async fn produto_fora_das_categorias_do_agente_nao_entra() {
    let cenario = Cenario::novo("visibilidade", &[NOME_ADICIONAR]);
    let categorias = cenario
        .store
        .categories()
        .listar(cenario.org, Page::default())
        .expect("lista")
        .into_iter()
        .filter(|categoria| categoria.name == "Pragas")
        .map(|categoria| categoria.id)
        .collect::<Vec<_>>();
    let agente = cenario
        .store
        .agents()
        .get(cenario.org, cenario.agente)
        .expect("agente");
    cenario
        .store
        .agents()
        .definir_categorias(cenario.org, cenario.agente, agente.version, &categorias)
        .expect("vincula o agente a Pragas");

    let erro = cenario
        .chamar(
            cenario.conversa,
            NOME_ADICIONAR,
            produto_e_quantidade(cenario.jardim, Some(1)),
        )
        .await
        .expect_err("produto de Jardim não é visível");
    assert!(erro.contains("não está disponível"), "{erro}");
    assert_eq!(cenario.total(cenario.conversa), (0, 0));

    cenario
        .chamar(
            cenario.conversa,
            NOME_ADICIONAR,
            produto_e_quantidade(cenario.produtos[0], Some(1)),
        )
        .await
        .expect("produto de Pragas entra");
    assert_eq!(cenario.total(cenario.conversa), (32_000, 1));
}

#[tokio::test]
async fn remover_o_que_nao_esta_no_carrinho_falha_sem_mudar_nada() {
    let cenario = Cenario::novo("remover", &[NOME_REMOVER]);
    cenario
        .chamar(
            cenario.conversa,
            NOME_ADICIONAR,
            produto_e_quantidade(cenario.produtos[0], Some(2)),
        )
        .await
        .expect("adiciona");
    let erro = cenario
        .chamar(
            cenario.conversa,
            NOME_REMOVER,
            produto_e_quantidade(cenario.produtos[1], None),
        )
        .await
        .expect_err("não estava no carrinho");
    assert!(erro.contains("não está no carrinho"), "{erro}");
    assert_eq!(cenario.total(cenario.conversa), (64_000, 1));
}

#[tokio::test]
async fn quantidade_acumulada_acima_de_999_e_recusada() {
    let cenario = Cenario::novo("limite", &[NOME_ADICIONAR]);
    cenario
        .chamar(
            cenario.conversa,
            NOME_ADICIONAR,
            produto_e_quantidade(cenario.produtos[2], Some(999)),
        )
        .await
        .expect("999 cabem");
    let erro = cenario
        .chamar(
            cenario.conversa,
            NOME_ADICIONAR,
            produto_e_quantidade(cenario.produtos[2], Some(1)),
        )
        .await
        .expect_err("passa de 999");
    assert!(erro.contains("999"), "{erro}");
    // Quantidade fora do intervalo nem chega ao banco.
    assert!(
        cenario
            .chamar(
                cenario.conversa,
                NOME_ADICIONAR,
                json!({"produto_id": cenario.produtos[0].to_string(), "quantidade": 0}),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn limpar_esvazia_e_conta_as_linhas() {
    let cenario = Cenario::novo("limpar", &[NOME_LIMPAR]);
    for indice in [0, 1, 3] {
        cenario
            .store
            .carts()
            .adicionar(
                cenario.org,
                cenario.conversa,
                cenario.agente,
                cenario.produtos[indice],
                1,
            )
            .expect("adiciona");
    }
    let texto = cenario
        .chamar(cenario.conversa, NOME_LIMPAR, json!({}))
        .await
        .expect("limpa");
    assert!(texto.contains("3 item(ns) removido(s)"), "{texto}");
    assert_eq!(cenario.total(cenario.conversa), (0, 0));
    // O carrinho continua aberto: a próxima adição não precisa criar outro.
    cenario
        .store
        .carts()
        .adicionar(
            cenario.org,
            cenario.conversa,
            cenario.agente,
            cenario.produtos[0],
            1,
        )
        .expect("volta a adicionar");
    let resumo = cenario
        .store
        .carts()
        .resumo(cenario.org, cenario.conversa)
        .expect("lê");
    assert!(
        resumo.carrinho.is_some(),
        "o carrinho aberto continua o mesmo"
    );
}
