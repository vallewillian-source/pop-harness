//! Testes da memória quente e da concorrência (HR-11).
//!
//! Cobrem os seis grupos que a tarefa exige:
//! 1. dois eventos da mesma conversa nunca rodam turnos em paralelo (ordem preservada);
//! 2. conversa ociosa é despejada e recarregada do banco sem perder mensagem nem ordem;
//! 3. mudança de versão do agente (ou das instruções) invalida o cache;
//! 4. org A saturando o limite não atrasa a org B (medido);
//! 5. teto de memória do cache respeitado com 10 mil entradas;
//! 6. janela de contexto respeita o orçamento de tokens do agente.
//!
//! E, de quebra, o isolamento por org do cache (agente de outra org não é servido pelo cache).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poph_agent::atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
use poph_agent::config_quente::{ConfigQuente, OpcoesConfigQuente};
use poph_agent::contexto::{estimar_tokens, montar};
use poph_agent::limites::{Limitador, OpcoesLimites};
use poph_agent::metricas::MetricasMemoria;
use poph_store::{Direction, Mensagem, NewAgent, NewConversation, NewMessage, NewOrg, Role, Store};
use poph_types::{AgentId, ConversationId, MessageId, OrgId};

/// Banco de verdade em pasta temporária, com uma org, um agente e uma conversa prontos.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    agente: AgentId,
    conversa: ConversationId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!("poph-hr11-{rotulo}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store = Arc::new(
            Store::open_at(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
        );
        let org = store
            .orgs()
            .create(NewOrg {
                name: "Org HR-11".to_owned(),
                slug: format!("org-hr11-{rotulo}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        Self {
            pasta,
            store,
            org,
            agente: AgentId::generate(),
            conversa: ConversationId::generate(),
        }
    }

    /// Cria um agente ativo com o slug dado.
    fn criar_agente(&mut self, slug: &str) -> AgentId {
        let agente = self
            .store
            .agents()
            .create(
                self.org,
                NewAgent {
                    name: format!("Agente {slug}"),
                    slug: slug.to_owned(),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente");
        self.agente = agente.id;
        self.abrir_conversa();
        agente.id
    }

    fn abrir_conversa(&mut self) {
        let conversa = self
            .store
            .conversations()
            .create(
                self.org,
                NewConversation {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    target_department: None,
                },
            )
            .expect("abre a conversa");
        self.conversa = conversa.id;
    }

    fn textos(&self) -> Vec<String> {
        self.store
            .messages()
            .list(self.org, self.conversa, poph_store::Page::with_limit(200))
            .expect("lista as mensagens")
            .into_iter()
            .map(|mensagem| mensagem.texto)
            .collect()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

// ─────────────────────────── 1. Um turno por vez e ordem preservada ───────────────────────────

#[derive(Default)]
struct DadosSerializacao {
    executando: AtomicUsize,
    maximo: AtomicUsize,
    ordem: Mutex<Vec<MessageId>>,
    atraso: Duration,
}

struct Serializador {
    dados: Arc<DadosSerializacao>,
}

impl Processador for Serializador {
    fn processar(&self, tarefa: Tarefa) -> Futuro {
        let dados = Arc::clone(&self.dados);
        Box::pin(async move {
            let atual = dados.executando.fetch_add(1, Ordering::SeqCst) + 1;
            dados.maximo.fetch_max(atual, Ordering::SeqCst);
            dados
                .ordem
                .lock()
                .expect("sem veneno")
                .push(tarefa.mensagem);
            tokio::time::sleep(dados.atraso).await;
            dados.executando.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dois_eventos_da_mesma_conversa_nunca_rodam_em_paralelo() {
    let cenario = Cenario::novo("serializacao");
    let dados = Arc::new(DadosSerializacao {
        atraso: Duration::from_millis(15),
        ..DadosSerializacao::default()
    });
    let atores = Atores::novo(
        Arc::new(Serializador {
            dados: Arc::clone(&dados),
        }),
        OpcoesAtores {
            ociosidade: Duration::from_secs(30),
            fila_por_conversa: 32,
            max_atores: 16,
        },
        MetricasMemoria::proprio(),
    );
    let enviados: Vec<MessageId> = (0..8).map(|_| MessageId::generate()).collect();
    for mensagem in &enviados {
        atores
            .enviar(Tarefa {
                org: cenario.org,
                conversa: cenario.conversa,
                mensagem: *mensagem,
                entrada: None,
            })
            .expect("enfileira");
    }
    assert!(
        atores.aguardar_filas_vazias(Duration::from_secs(10)).await,
        "as filas deviam esvaziar"
    );
    assert_eq!(
        dados.maximo.load(Ordering::SeqCst),
        1,
        "nunca pode haver dois turnos na mesma conversa"
    );
    assert_eq!(*dados.ordem.lock().expect("sem veneno"), enviados);
    assert_eq!(atores.atores_vivos(), 1);
}

// ────────────────────── 2. Despejo por ociosidade e recarga do banco ──────────────────────

struct ContadorDeEco {
    store: Arc<Store>,
    org: OrgId,
    conversa: ConversationId,
    acordadas: Arc<AtomicUsize>,
    recebidas: Arc<AtomicUsize>,
}

impl Processador for ContadorDeEco {
    fn processar(&self, _tarefa: Tarefa) -> Futuro {
        let store = Arc::clone(&self.store);
        let org = self.org;
        let conversa = self.conversa;
        let recebidas = Arc::clone(&self.recebidas);
        Box::pin(async move {
            let numero = recebidas.fetch_add(1, Ordering::SeqCst) + 1;
            store
                .messages()
                .write(
                    org,
                    NewMessage {
                        conversation_id: conversa,
                        direction: Direction::Outbound,
                        role: Role::Assistant,
                        texto: format!("eco {numero}"),
                        attachments_json: String::new(),
                        external_id: None,
                        turn_id: None,
                        origin: poph_store::Origin::Agente,
                        author_name: None,
                    },
                )
                .expect("grava a resposta");
            Ok(())
        })
    }

    fn ao_acordar(&self, _org: OrgId, _conversa: ConversationId) -> Futuro {
        // A recarga de verdade é ler o estado do banco: aqui basta contar que ela aconteceu.
        let acordadas = Arc::clone(&self.acordadas);
        let store = Arc::clone(&self.store);
        let org = self.org;
        let conversa = self.conversa;
        Box::pin(async move {
            store
                .messages()
                .last_set(org, conversa, 10)
                .expect("lê o histórico ao acordar");
            acordadas.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conversa_ociosa_e_despejada_e_recarregada_sem_perder_mensagem() {
    let mut cenario = Cenario::novo("despejo");
    cenario.criar_agente("despejo");
    let acordadas = Arc::new(AtomicUsize::new(0));
    let recebidas = Arc::new(AtomicUsize::new(0));
    let atores = Atores::novo(
        Arc::new(ContadorDeEco {
            store: Arc::clone(&cenario.store),
            org: cenario.org,
            conversa: cenario.conversa,
            acordadas: Arc::clone(&acordadas),
            recebidas: Arc::clone(&recebidas),
        }),
        OpcoesAtores {
            ociosidade: Duration::from_millis(80),
            fila_por_conversa: 32,
            max_atores: 16,
        },
        MetricasMemoria::proprio(),
    );

    for _ in 0..5 {
        atores
            .enviar(Tarefa {
                org: cenario.org,
                conversa: cenario.conversa,
                mensagem: MessageId::generate(),
                entrada: None,
            })
            .expect("enfileira");
    }
    assert!(atores.aguardar_filas_vazias(Duration::from_secs(5)).await);
    assert_eq!(atores.atores_vivos(), 1);

    assert!(
        atores.aguardar_despejo(Duration::from_secs(5)).await,
        "o ator ocioso devia ser despejado"
    );
    assert_eq!(acordadas.load(Ordering::SeqCst), 1, "acordou uma vez");

    // A conversa volta: ator novo (segunda recarga) e nada do que já estava no banco se perde.
    for _ in 0..3 {
        atores
            .enviar(Tarefa {
                org: cenario.org,
                conversa: cenario.conversa,
                mensagem: MessageId::generate(),
                entrada: None,
            })
            .expect("enfileira depois do despejo");
    }
    assert!(atores.aguardar_filas_vazias(Duration::from_secs(5)).await);
    assert_eq!(acordadas.load(Ordering::SeqCst), 2, "recarregou do banco");

    let textos = cenario.textos();
    let esperado: Vec<String> = (1..=8).map(|numero| format!("eco {numero}")).collect();
    assert_eq!(
        textos, esperado,
        "nenhuma mensagem pode faltar nem trocar de ordem"
    );
}

// ───────────────────────── 3. Invalidação do cache por versão ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mudanca_de_versao_invalida_o_cache() {
    let mut cenario = Cenario::novo("cache");
    let agente = cenario.criar_agente("cache");
    cenario
        .store
        .instructions()
        .save(
            cenario.org,
            agente,
            0,
            r#"{"identity":{"name":"Bia"}}"#,
            "v1 do prompt",
            true,
        )
        .expect("grava as instruções");

    let metricas = MetricasMemoria::proprio();
    let cache = ConfigQuente::nova(
        Arc::clone(&cenario.store),
        OpcoesConfigQuente::default(),
        metricas.clone(),
    );

    let primeira = cache
        .carregar_verificando(cenario.org, agente)
        .expect("carrega");
    assert_eq!(primeira.markdown, "v1 do prompt");
    assert_eq!(primeira.versao(), 1);
    assert_eq!(metricas.config_erros.value(), 1);

    let segunda = cache
        .carregar_verificando(cenario.org, agente)
        .expect("carrega do cache");
    assert_eq!(segunda.markdown, "v1 do prompt");
    assert_eq!(metricas.config_acertos.value(), 1);

    // Edita só o texto do prompt: a versão do agente não muda, a das instruções muda.
    let instrucoes = cenario
        .store
        .instructions()
        .get(cenario.org, agente)
        .expect("lê as instruções");
    cenario
        .store
        .instructions()
        .save(
            cenario.org,
            agente,
            instrucoes.version,
            r#"{"identity":{"name":"Bia v2"}}"#,
            "v2 do prompt",
            true,
        )
        .expect("regrava as instruções");

    let terceira = cache
        .carregar_verificando(cenario.org, agente)
        .expect("recarrega");
    assert_eq!(terceira.markdown, "v2 do prompt");
    assert_eq!(metricas.config_invalidadas.value(), 1);

    // Muda a versão do próprio agente (nome): também invalida.
    let atual = cenario
        .store
        .agents()
        .get(cenario.org, agente)
        .expect("lê o agente");
    cenario
        .store
        .agents()
        .update(
            cenario.org,
            agente,
            atual.version,
            poph_store::AgentChange {
                name: Some("Bia Editada".to_owned()),
                ..poph_store::AgentChange::default()
            },
        )
        .expect("edita o agente");
    let quarta = cache
        .carregar_verificando(cenario.org, agente)
        .expect("recarrega de novo");
    assert_eq!(quarta.agente.name, "Bia Editada");
    assert_eq!(quarta.markdown, "v2 do prompt");

    // Agente de outra org não é servido pelo cache do agente desta org.
    let outra_org = cenario
        .store
        .orgs()
        .create(NewOrg {
            name: "Outra".to_owned(),
            slug: "outra-org".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a outra org")
        .id;
    assert!(
        cache.carregar_verificando(outra_org, agente).is_err(),
        "agente não existe na outra org"
    );
}

// ────────────────────── 4. Org saturada não atrasa outra org ──────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn org_saturada_nao_atrasa_outra_org() {
    let limitador = Limitador::novo(
        OpcoesLimites {
            turnos_global: 16,
            turnos_por_org: 1,
            turnos_por_agente: 0,
            chamadas_por_provedor: 4,
            tetos_por_provedor: Default::default(),
            taxa_por_org_por_minuto: 0,
        },
        MetricasMemoria::proprio(),
    );
    let org_a = OrgId::generate();
    let org_b = OrgId::generate();
    let agente = AgentId::generate();

    // Org A ocupa a única vaga dela e segura por 400 ms.
    let vaga_a = limitador
        .adquirir_turno(org_a, agente, Duration::from_millis(200))
        .await
        .expect("org A pega a vaga");
    let segura = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        drop(vaga_a);
    });

    tokio::time::sleep(Duration::from_millis(30)).await;
    let inicio = Instant::now();
    let vaga_b = limitador
        .adquirir_turno(org_b, agente, Duration::from_secs(2))
        .await
        .expect("org B pega a vaga");
    let esperou = inicio.elapsed();
    drop(vaga_b);
    segura.await.expect("tarefa de apoio");
    assert!(
        esperou < Duration::from_millis(150),
        "org B esperou {esperou:?}: a saturação da org A vazou para ela"
    );

    // E a org A, com a vaga ocupada, realmente espera (o limite funciona).
    let primeira = limitador
        .adquirir_turno(org_a, agente, Duration::from_millis(50))
        .await
        .expect("org A pega a primeira vaga");
    let segunda = limitador
        .adquirir_turno(org_a, agente, Duration::from_millis(50))
        .await;
    assert!(
        segunda.is_err(),
        "org A não pode ter duas vagas com limite 1"
    );
    drop(primeira);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cota_por_minuto_da_org_e_consumida_e_recusada() {
    let limitador = Limitador::novo(
        OpcoesLimites {
            taxa_por_org_por_minuto: 2,
            ..OpcoesLimites::default()
        },
        MetricasMemoria::proprio(),
    );
    let org = OrgId::generate();
    limitador.consumir_cota(org).expect("1ª mensagem");
    limitador.consumir_cota(org).expect("2ª mensagem");
    let terceira = limitador.consumir_cota(org);
    assert!(terceira.is_err(), "a cota da org devia ter acabado");
    assert!(terceira.expect_err("erro de cota").eh_transitorio());
}

// ───────────────────────── 5. Teto de memória com 10 mil entradas ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_respeita_o_teto_de_memoria_com_dez_mil_agentes() {
    let mut cenario = Cenario::novo("teto");
    // O primeiro agente ida e volta para garantir esquema e org prontos.
    cenario.criar_agente("teto-0");
    let teto = 1024 * 1024; // 1 MB: muito menor que 10 mil configs.
    let cache = ConfigQuente::nova(
        Arc::clone(&cenario.store),
        OpcoesConfigQuente {
            validade: Duration::from_secs(600),
            teto_bytes: teto,
        },
        MetricasMemoria::proprio(),
    );

    let mut criados = 0u64;
    let mut ultimo = AgentId::generate();
    for indice in 0..10_000u32 {
        let agente = cenario
            .store
            .agents()
            .create(
                cenario.org,
                NewAgent {
                    name: format!("Agente de carga {indice}"),
                    slug: format!("carga-{indice}"),
                    description: "config sintética para medir o teto do cache".repeat(4),
                    ..NewAgent::default()
                },
            )
            .expect("cria o agente da carga");
        cache.carregar(cenario.org, agente.id).expect("carrega");
        ultimo = agente.id;
        criados += 1;
    }
    cache.sincronizar();
    assert_eq!(criados, 10_000);
    assert!(
        cache.bytes() <= teto,
        "o cache passou do teto: {} bytes > {teto}",
        cache.bytes()
    );
    assert!(
        cache.tamanho() < 10_000,
        "o cache guardou tudo ({} entradas): o teto não foi aplicado",
        cache.tamanho()
    );
    // A última entrada continua acessível (o cache não é um "perde tudo").
    assert!(cache.carregar(cenario.org, ultimo).is_ok());
}

// ─────────────────────── 6. Janela de contexto e orçamento de tokens ───────────────────────

fn mensagem_de(texto: &str) -> Mensagem {
    Mensagem {
        id: MessageId::generate(),
        org_id: OrgId::generate(),
        conversation_id: ConversationId::generate(),
        direction: Direction::Inbound,
        role: Role::User,
        origin: poph_store::Origin::Agente,
        author_name: None,
        texto: texto.to_owned(),
        attachments_json: String::new(),
        external_id: None,
        turn_id: None,
        created_at: 0,
    }
}

#[test]
fn janela_respeita_o_orcamento_e_guarda_as_recentes() {
    // Todas com exatamente 400 caracteres (100 tokens + 4 de custo fixo por mensagem).
    let mensagens: Vec<Mensagem> = (1..=10)
        .map(|indice| mensagem_de(&format!("{indice:02}-{}", "x".repeat(397))))
        .collect();
    let ultimo_texto = mensagens.last().expect("tem mensagens").texto.clone();

    // Cada mensagem custa ~100 tokens + 4; com 300 de orçamento cabem 2.
    let janela = montar(mensagens, 300, 0);
    assert_eq!(janela.mensagens.len(), 2);
    assert_eq!(janela.descartadas, 8);
    assert_eq!(janela.mensagens.last().expect("última").texto, ultimo_texto);
    assert!(janela.tokens_estimados <= 300, "estourou o orçamento");
    assert!(!janela.ultima_truncada);
}

#[test]
fn janela_respeita_o_teto_de_quantidade_de_mensagens() {
    let mensagens: Vec<Mensagem> = (0..5).map(|_| mensagem_de("curta")).collect();
    let janela = montar(mensagens, 10_000, 2);
    assert_eq!(janela.mensagens.len(), 2);
    assert_eq!(janela.descartadas, 3);
}

#[test]
fn mensagem_unica_maior_que_o_orcamento_e_cortada() {
    let mensagens = vec![mensagem_de(&"a".repeat(1000))];
    let janela = montar(mensagens, 50, 0);
    assert_eq!(janela.mensagens.len(), 1);
    assert!(janela.ultima_truncada);
    assert!(janela.tokens_estimados <= 50);
    assert!(janela.mensagens[0].texto.ends_with('…'));
    assert!(estimar_tokens(&janela.mensagens[0].texto) <= 50);
}

#[test]
fn janela_vazia_nao_quebra() {
    let janela = montar(Vec::new(), 100, 10);
    assert!(janela.mensagens.is_empty());
    assert_eq!(janela.descartadas, 0);
    assert_eq!(janela.tokens_estimados, 0);
}

// ───────────────────────────── Custo medido (rodar com --ignored) ─────────────────────────────

struct SemCusto;

impl Processador for SemCusto {
    fn processar(&self, _tarefa: Tarefa) -> Futuro {
        Box::pin(async { Ok(()) })
    }
}

/// Mede o custo do enfileiramento e do laço do ator, e o custo de uma leitura no cache quente.
///
/// Fora do CI normal (é medida, não regra): `cargo test -p poph-agent --test memoria_e_concorrencia
/// -- --ignored --nocapture custo`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "mede o custo; rodar com --ignored --nocapture"]
async fn custo_por_evento_e_por_leitura_de_cache() {
    const EVENTOS: usize = 8_000;
    let cenario = Cenario::novo("custo");
    let atores = Atores::novo(
        Arc::new(SemCusto),
        OpcoesAtores {
            ociosidade: Duration::from_secs(60),
            fila_por_conversa: EVENTOS,
            max_atores: 8,
        },
        MetricasMemoria::proprio(),
    );
    let inicio = Instant::now();
    for _ in 0..EVENTOS {
        atores
            .enviar(Tarefa {
                org: cenario.org,
                conversa: cenario.conversa,
                mensagem: MessageId::generate(),
                entrada: None,
            })
            .expect("enfileira");
    }
    let enfileirar = inicio.elapsed();
    assert!(atores.aguardar_filas_vazias(Duration::from_secs(60)).await);
    let total = inicio.elapsed();
    println!(
        "ator: {EVENTOS} eventos; enfileirar {:.3} µs/evento; laço completo {:.3} µs/evento",
        enfileirar.as_secs_f64() * 1e6 / EVENTOS as f64,
        total.as_secs_f64() * 1e6 / EVENTOS as f64
    );

    let mut cenario_cache = Cenario::novo("custo-cache");
    let agente = cenario_cache.criar_agente("cache");
    let cache = ConfigQuente::nova(
        Arc::clone(&cenario_cache.store),
        OpcoesConfigQuente::default(),
        MetricasMemoria::proprio(),
    );
    cache.carregar(cenario_cache.org, agente).expect("aquecer");
    let leituras = 50_000;
    let inicio = Instant::now();
    for _ in 0..leituras {
        cache
            .carregar(cenario_cache.org, agente)
            .expect("leitura quente");
    }
    let por_leitura = inicio.elapsed().as_secs_f64() * 1e6 / leituras as f64;
    println!("cache de config quente: {por_leitura:.3} µs por leitura (acerto)");
}
