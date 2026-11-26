//! Biblioteca de arquivos do agente e ferramenta `enviar_arquivo` (HR-24).
//!
//! Cobra o que a tarefa pede:
//! - id de outro agente/org é recusado, inclusive com org forjada no contexto, e o turno não cai;
//! - id inexistente devolve erro legível ao modelo, sem quebrar o turno;
//! - o envio enfileira `SendFile` (tipo `file`) na `outbox` com `blob_ref`, e repetir no mesmo turno
//!   não duplica;
//! - agente sem a ferramenta habilitada não a recebe; com ela, recebe e vê a descrição do arquivo.
//!
//! Provedor falso e pausa que não dorme: nenhum teste toca a rede nem espera de verdade.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_agent::atores::{Atores, OpcoesAtores};
use poph_agent::biblioteca::{self, NOME_FERRAMENTA};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::entrada_saida::{Fluxo, MensagemRecebida, OpcoesFluxo};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::{MetricasFluxo, MetricasMemoria, MetricasTurno};
use poph_agent::turno::{
    ContextoFerramenta, FerramentaInterna, FuturoPausa, Motor, ObservadorNulo, OpcoesTurno, Pausa,
    ProcessadorTurno, RegistroFerramentas, RegistroPadrao, UsoFerramentas,
};
use poph_core::llm::{
    ChamadaFerramenta, ErroLlm, EventoLlm, LlmProvider, MotivoFim, ProvedorFake, Roteiro,
};
use poph_store::{
    LibraryFile, NewAgent, NewConversation, NewLibraryFile, NewOrg, OutboxItem, Store,
};
use poph_types::{
    AgentId, ConversationId, FileId, OrgId, StructuredInstructions, ToolConfig, TurnId,
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

struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    outro_agente: AgentId,
    conversa: ConversationId,
    catalogo: FileId,
    proposta: FileId,
}

impl Cenario {
    fn novo(rotulo: &str, ferramentas: &[&str]) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-biblioteca-{rotulo}-{}-{}",
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
        let agente = Self::criar_agente(&store, org, rotulo, ferramentas);
        let outro_agente = Self::criar_agente(&store, org, &format!("{rotulo}-2"), &[]);
        let catalogo = Self::arquivo(&store, org, agente, "catalogo.pdf", "catálogo de serviços");
        let proposta = Self::arquivo(&store, org, agente, "proposta.pdf", "modelo de proposta");
        let conversa = Self::abrir_conversa(&store, org, agente);
        Self {
            pasta,
            store,
            org,
            agente,
            outro_agente,
            conversa,
            catalogo,
            proposta,
        }
    }

    fn criar_agente(store: &Store, org: OrgId, rotulo: &str, ferramentas: &[&str]) -> AgentId {
        let sufixo = AgentId::generate().to_string();
        let agente = store
            .agents()
            .create(
                org,
                NewAgent {
                    name: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}-{}", &sufixo[..8]),
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
            .save(
                org,
                agente,
                0,
                &estruturado.to_json(),
                "# Bia\n\nVocê é a atendente da FixProLar.",
                true,
            )
            .expect("grava as instruções");
        agente
    }

    fn arquivo(store: &Store, org: OrgId, agente: AgentId, nome: &str, descricao: &str) -> FileId {
        store
            .library()
            .create(
                org,
                NewLibraryFile {
                    agent_id: agente,
                    name: nome.to_owned(),
                    description: descricao.to_owned(),
                    mime: "application/pdf".to_owned(),
                    size_bytes: 4_096,
                    sha256: "b".repeat(64),
                    blob_path: format!("{org}/{nome}"),
                },
            )
            .expect("põe o arquivo na biblioteca")
            .id
    }

    fn abrir_conversa(store: &Store, org: OrgId, agente: AgentId) -> ConversationId {
        store
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
            .id
    }

    fn contexto(&self, turno: TurnId) -> ContextoFerramenta {
        ContextoFerramenta {
            org: self.org,
            conversa: self.conversa,
            agente: self.agente,
            turno,
            uso: UsoFerramentas::nova(),
        }
    }

    fn registro(&self) -> RegistroPadrao {
        biblioteca::registrar(RegistroPadrao::novo(), Arc::clone(&self.store))
    }

    fn ferramenta(&self) -> Arc<dyn FerramentaInterna> {
        self.registro()
            .ferramenta(NOME_FERRAMENTA)
            .expect("ferramenta registrada")
    }

    /// Chama a ferramenta direto, como o motor faria.
    async fn chamar(&self, turno: TurnId, argumentos: Value) -> Result<String, String> {
        self.ferramenta()
            .executar(self.contexto(turno), argumentos)
            .await
    }

    /// Itens da `outbox` do turno que ainda estão pendentes (nenhum transporte roda nestes testes).
    fn saidas(&self, turno: TurnId) -> Vec<OutboxItem> {
        self.store
            .outbox()
            .list_pending(self.org, 50)
            .expect("lê a outbox")
            .into_iter()
            .filter(|item| item.turn_id == turno)
            .collect()
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
        let motor =
            motor.com_ferramentas(Arc::new(self.registro()) as Arc<dyn RegistroFerramentas>);
        (motor, fake)
    }

    fn falar(&self, texto: &str) -> poph_types::MessageId {
        self.store
            .messages()
            .write(
                self.org,
                poph_store::NewMessage {
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

    async fn rodar_turno(&self, motor: &Motor) -> poph_agent::turno::ResumoTurno {
        let mensagem = self.falar("manda o catálogo por favor");
        motor
            .executar(
                poph_agent::turno::Entrada::nova(self.org, self.conversa, mensagem),
                &mut ObservadorNulo,
            )
            .await
            .expect("o turno roda")
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

fn chamada(id: &str, argumentos: Value) -> Roteiro {
    Roteiro::Eventos(vec![
        EventoLlm::Ferramenta(ChamadaFerramenta {
            id: id.to_owned(),
            nome: NOME_FERRAMENTA.to_owned(),
            argumentos,
        }),
        EventoLlm::Fim(MotivoFim::ChamadasFerramenta),
    ])
}

fn arquivo_do_item(item: &OutboxItem) -> Option<Value> {
    serde_json::from_str(&item.content_json).ok()
}

#[tokio::test]
async fn id_de_outro_agente_da_mesma_org_e_recusado() {
    let cenario = Cenario::novo("outro-agente", &[NOME_FERRAMENTA]);
    // Arquivo do outro agente: o id existe, mas não na biblioteca deste agente.
    let alheio = Cenario::arquivo(
        &cenario.store,
        cenario.org,
        cenario.outro_agente,
        "interno.pdf",
        "documento interno",
    );
    let turno = TurnId::generate();
    let erro = cenario
        .chamar(turno, json!({"id": alheio.to_string()}))
        .await
        .expect_err("id de outro agente é recusado");
    assert!(erro.contains("não existe arquivo"), "{erro}");
    assert!(cenario.saidas(turno).is_empty(), "nada foi enfileirado");
}

#[tokio::test]
async fn id_de_outra_org_e_recusado_mesmo_com_org_forjada() {
    let cenario = Cenario::novo("outra-org", &[NOME_FERRAMENTA]);
    // Org e agente de fora, com um arquivo dentro.
    let outra_org = cenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: format!("outra-{}", AgentId::generate()),
            config_json: "{}".to_owned(),
        })
        .expect("cria a outra org");
    let agente_alheio = Cenario::criar_agente(&cenario.store, outra_org.id, "fora", &[]);
    let arquivo_alheio = Cenario::arquivo(
        &cenario.store,
        outra_org.id,
        agente_alheio,
        "segredo.pdf",
        "não é desta org",
    );

    // 1) coexistência: cada org enxerga só o seu.
    let erro = cenario
        .chamar(
            TurnId::generate(),
            json!({"id": arquivo_alheio.to_string()}),
        )
        .await
        .expect_err("arquivo de outra org é recusado");
    assert!(erro.contains("não existe arquivo"), "{erro}");

    // 2) org forjada no contexto: o mesmo id do próprio agente, mas alegando a outra org.
    let turno = TurnId::generate();
    let ferramenta = cenario.ferramenta();
    let mut contexto = cenario.contexto(turno);
    contexto.org = outra_org.id;
    let erro = ferramenta
        .executar(contexto, json!({"id": cenario.catalogo.to_string()}))
        .await
        .expect_err("id do próprio agente não vale com a org trocada");
    assert!(erro.contains("não existe arquivo"), "{erro}");
    assert!(cenario.saidas(turno).is_empty());
}

#[tokio::test]
async fn id_inexistente_devolve_erro_legivel_e_o_turno_segue() {
    let cenario = Cenario::novo("inexistente", &[NOME_FERRAMENTA]);
    // Erro direto na ferramenta, com texto que o modelo entende.
    let erro = cenario
        .chamar(
            TurnId::generate(),
            json!({"id": FileId::generate().to_string()}),
        )
        .await
        .expect_err("id inexistente é recusado");
    assert!(erro.starts_with("erro:"), "{erro}");
    assert!(erro.contains("lista de arquivos"), "{erro}");

    // No turno: o modelo recebe o erro como resultado da ferramenta e a resposta sai.
    let (motor, _fake) = cenario.motor(vec![
        chamada("c1", json!({"id": FileId::generate().to_string()})),
        resposta("Não achei esse arquivo; posso te mandar o catálogo."),
    ]);
    let resumo = cenario.rodar_turno(&motor).await;
    assert!(resumo.texto.contains("catálogo"), "{}", resumo.texto);
    let saidas = cenario.saidas(resumo.turno);
    assert!(
        saidas.iter().all(|item| item.tipo == "text"),
        "nenhum arquivo enfileirado: {saidas:?}"
    );
}

#[tokio::test]
async fn envio_enfileira_arquivo_na_outbox_e_repetir_nao_duplica() {
    let cenario = Cenario::novo("envio", &[NOME_FERRAMENTA]);
    let turno = TurnId::generate();
    let primeiro = cenario
        .chamar(turno, json!({"id": cenario.catalogo.to_string()}))
        .await
        .expect("enfileira");
    assert!(primeiro.contains("enfileirado"), "{primeiro}");

    let saidas = cenario.saidas(turno);
    assert_eq!(saidas.len(), 1);
    let item = &saidas[0];
    assert_eq!(item.tipo, "file");
    let payload = arquivo_do_item(item).expect("conteúdo em JSON");
    assert_eq!(
        payload["blob_ref"],
        json!(format!("{}/catalogo.pdf", cenario.org))
    );
    assert_eq!(payload["arquivo_id"], json!(cenario.catalogo.to_string()));
    assert_eq!(payload["mime"], json!("application/pdf"));

    // Repetir a chamada no mesmo turno não cria item novo (idempotência por turno + seq).
    let segundo = cenario
        .chamar(turno, json!({"id": cenario.catalogo.to_string()}))
        .await
        .expect("repetição é aceita");
    assert!(segundo.contains("já estava enfileirado"), "{segundo}");
    assert_eq!(cenario.saidas(turno).len(), 1);

    // Outro arquivo no mesmo turno entra em item separado.
    cenario
        .chamar(turno, json!({"id": cenario.proposta.to_string()}))
        .await
        .expect("enfileira o segundo");
    assert_eq!(cenario.saidas(turno).len(), 2);
}

#[tokio::test]
async fn turno_completo_envia_arquivo_e_responde() {
    let cenario = Cenario::novo("turno", &[NOME_FERRAMENTA]);
    let (motor, _fake) = cenario.motor(vec![
        chamada("c1", json!({"id": cenario.catalogo.to_string()})),
        resposta("Estou enviando o catálogo."),
    ]);
    let resumo = cenario.rodar_turno(&motor).await;
    let saidas = cenario.saidas(resumo.turno);
    let tipos: Vec<&str> = saidas.iter().map(|item| item.tipo.as_str()).collect();
    assert!(tipos.contains(&"file"), "{tipos:?}");
    assert!(tipos.contains(&"text"), "{tipos:?}");
}

#[tokio::test]
async fn agente_sem_a_ferramenta_nao_a_recebe() {
    let cenario = Cenario::novo("sem-ferramenta", &[]);
    let (motor, fake) = cenario.motor(vec![resposta("ok")]);
    cenario.rodar_turno(&motor).await;
    let nomes: Vec<String> = fake.recebidos()[0]
        .ferramentas
        .iter()
        .map(|ferramenta| ferramenta.nome.clone())
        .collect();
    assert!(nomes.is_empty(), "{nomes:?}");
}

#[tokio::test]
async fn agente_com_a_ferramenta_ve_a_descricao_dos_arquivos() {
    let cenario = Cenario::novo("com-ferramenta", &[NOME_FERRAMENTA]);
    let (motor, fake) = cenario.motor(vec![resposta("ok")]);
    cenario.rodar_turno(&motor).await;
    let pedido = &fake.recebidos()[0];
    let ferramenta = pedido
        .ferramentas
        .iter()
        .find(|ferramenta| ferramenta.nome == NOME_FERRAMENTA)
        .expect("a ferramenta vai no pedido");
    assert!(
        ferramenta.descricao.contains(&cenario.catalogo.to_string()),
        "a descrição traz o id do arquivo: {}",
        ferramenta.descricao
    );
    assert!(
        ferramenta.descricao.contains("catálogo de serviços"),
        "{}",
        ferramenta.descricao
    );
    assert!(
        ferramenta.descricao.contains("modelo de proposta"),
        "{}",
        ferramenta.descricao
    );
    // O esquema aceita só `id`.
    assert_eq!(
        ferramenta.parametros["properties"]
            .as_object()
            .map(|mapa| mapa.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["id".to_owned()])
    );
}

#[tokio::test]
async fn biblioteca_vazia_aparece_como_tal_e_nao_quebra_o_turno() {
    let cenario = Cenario::novo("vazia", &[NOME_FERRAMENTA]);
    for arquivo in [cenario.catalogo, cenario.proposta] {
        cenario
            .store
            .library()
            .delete(cenario.org, cenario.agente, arquivo)
            .expect("apaga o arquivo");
    }
    let (motor, fake) = cenario.motor(vec![resposta("ok")]);
    cenario.rodar_turno(&motor).await;
    let recebidos = fake.recebidos();
    let ferramenta = recebidos[0]
        .ferramentas
        .iter()
        .find(|ferramenta| ferramenta.nome == NOME_FERRAMENTA)
        .expect("a ferramenta continua no pedido");
    assert!(
        ferramenta
            .descricao
            .contains("biblioteca de arquivos está vazia"),
        "{}",
        ferramenta.descricao
    );
}

#[tokio::test]
async fn argumento_alem_do_id_e_recusado() {
    let cenario = Cenario::novo("argumentos", &[NOME_FERRAMENTA]);
    let turno = TurnId::generate();
    // O modelo não manda caminho nem conteúdo (D13).
    let erro = cenario
        .chamar(
            turno,
            json!({"id": cenario.catalogo.to_string(), "caminho": "/etc/passwd"}),
        )
        .await
        .expect_err("caminho não é aceito");
    assert!(erro.contains("não existe nesta ferramenta"), "{erro}");
    assert!(cenario.saidas(turno).is_empty());
    // Nem id vazio.
    assert!(cenario.chamar(turno, json!({"id": "  "})).await.is_err());
}

#[test]
fn arquivo_da_biblioteca_traz_o_caminho_relativo_do_blob() {
    // O caminho gravado é relativo à pasta de blobs (`<org>/<sha256>`), como a retenção espera.
    let arquivo = LibraryFile {
        id: FileId::generate(),
        org_id: OrgId::generate(),
        agent_id: AgentId::generate(),
        name: "x.pdf".to_owned(),
        description: String::new(),
        mime: "application/pdf".to_owned(),
        size_bytes: 1,
        sha256: "c".repeat(64),
        blob_path: "org/hash".to_owned(),
        created_at: 0,
    };
    assert!(!arquivo.blob_path.starts_with('/'));
}

/// HR-26i: a primeira tentativa já mandou o arquivo para a saída e depois o modelo falhou. Repetir
/// o turno mandaria o arquivo de novo, então a falha é definitiva: uma saída só, uma chamada a
/// mais ao modelo (a da falha), e a entrada sai da fila.
#[tokio::test]
async fn falha_do_modelo_depois_de_arquivo_enfileirado_nao_repete_o_envio() {
    let cenario = Cenario::novo("falha-arquivo", &[NOME_FERRAMENTA]);
    let falha = Roteiro::Falha(ErroLlm::Indisponivel {
        tentativas: 3,
        ultimo: "503".to_owned(),
    });
    let (motor, fake) = cenario.motor(vec![
        chamada("c1", json!({"id": cenario.catalogo.to_string()})),
        falha,
        resposta("não deveria sair"),
    ]);
    let motor = Arc::new(motor);
    let processador = Arc::new(ProcessadorTurno::novo(Arc::clone(&motor)));
    let atores = Atores::novo(
        processador.clone(),
        OpcoesAtores::default(),
        MetricasMemoria::proprio(),
    );
    // Repetição curta (50 ms), para que o teste espere de verdade só o necessário. Se o turno
    // repetisse, a segunda rodada chamaria o modelo de novo e mandaria o arquivo outra vez.
    let fluxo = Fluxo::com_metricas(
        Arc::clone(&cenario.store),
        atores,
        OpcoesFluxo::default().com_falha_modelo(3, Duration::from_millis(50)),
        MetricasFluxo::proprio(),
    );
    processador.ligar_fluxo(fluxo.clone());

    fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: poph_types::IntegrationId::generate(),
                id_externo: "ext-arquivo-falha-1".to_owned(),
                conversa: cenario.conversa,
                texto: "manda o catálogo".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    fluxo.despachar(cenario.org).expect("despacha");

    for _ in 0..500 {
        let pendentes = cenario
            .store
            .inbox()
            .count_pending(cenario.org)
            .expect("pendentes");
        let rodando = cenario
            .store
            .turns()
            .list(
                cenario.org,
                cenario.conversa,
                poph_store::Page::with_limit(10),
            )
            .expect("turnos")
            .iter()
            .any(|turno| turno.state == poph_store::TurnState::Running);
        if pendentes == 0 && !rodando {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Espera que a repetição (se existisse) tivesse tempo de rodar.
    tokio::time::sleep(Duration::from_millis(150)).await;
    fluxo.despachar(cenario.org).expect("despacha de novo");
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Duas chamadas ao modelo (a do arquivo e a da falha), um turno só, e um arquivo só na saída.
    assert_eq!(fake.recebidos().len(), 2, "sem repetição do turno");
    let turnos = cenario
        .store
        .turns()
        .list(
            cenario.org,
            cenario.conversa,
            poph_store::Page::with_limit(10),
        )
        .expect("turnos");
    assert_eq!(turnos.len(), 1, "um turno só");
    let saidas = cenario
        .store
        .outbox()
        .list_pending(cenario.org, 50)
        .expect("saída");
    let arquivos = saidas.iter().filter(|item| item.tipo == "file").count();
    assert_eq!(arquivos, 1, "o arquivo sai uma vez");
    assert_eq!(
        cenario
            .store
            .inbox()
            .count_pending(cenario.org)
            .expect("pendentes"),
        0,
        "a entrada saiu da fila (concluída com o motivo)"
    );
}
