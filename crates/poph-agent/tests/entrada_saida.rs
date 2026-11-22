//! Testes da entrada e da saída (HR-12).
//!
//! Cobrem os seis grupos que a tarefa exige:
//! 1. a mesma mensagem entregue 2× é processada uma vez (e a segunda responde "duplicada");
//! 2. `SIGKILL` entre gravar na `inbox` e processar não perde nada — a retomada processa;
//! 3. `SIGKILL` entre `sending` e `sent` reenvia exatamente uma vez na retomada;
//! 4. falha repetida termina em `failed` com motivo e contador de tentativas;
//! 5. o backoff cresce e para no teto;
//! 6. nenhuma leitura de `inbox`/`outbox` cruza org.
//!
//! Os dois testes de `SIGKILL` matam um **processo filho de verdade**: o filho é este mesmo binário
//! de teste, rodado com `--exact <nome>`, que grava no banco, escreve um marcador e fica dormindo;
//! o pai espera o marcador e manda `SIGKILL`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use poph_agent::atores::{Atores, Futuro, OpcoesAtores, Processador, Tarefa};
use poph_agent::entrada_saida::{
    Backoff, EventoEntrada, FalhaEnvio, Fluxo, FuturoEnvio, MensagemRecebida, OpcoesFluxo,
    TransporteSaida,
};
use poph_agent::metricas::MetricasMemoria;
use poph_store::{
    Direcao, EstadoSaida, ItemSaida, NovaConversa, NovaEntrada, NovaMensagem, NovaOrg, NovaSaida,
    NovoAgente, Papel, Store,
};
use poph_types::{ConversationId, IntegrationId, MessageId, OrgId, TurnId};

// ───────────────────────────── Infraestrutura dos testes ─────────────────────────────

/// Banco de verdade em pasta temporária, com org, agente e conversa prontos.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
    org: OrgId,
    integracao: IntegrationId,
    conversa: ConversationId,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!("poph-hr12-{rotulo}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&pasta);
        std::fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store = Arc::new(
            Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
                .expect("abre o banco"),
        );
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: "Org HR-12".to_owned(),
                slug: format!("org-hr12-{rotulo}"),
                config_json: String::new(),
            })
            .expect("cria a org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: format!("Agente {rotulo}"),
                    slug: format!("agente-{rotulo}"),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente");
        let conversa = store
            .conversas()
            .criar(
                org,
                NovaConversa {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "cli".to_owned(),
                    peer_externo: None,
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa");
        Self {
            pasta,
            store,
            org,
            integracao: IntegrationId::generate(),
            conversa: conversa.id,
        }
    }

    /// Reabre o banco (o filho do teste de `SIGKILL` usa isto depois que o pai soltou o `Store`).
    fn reabrir(&mut self) {
        self.store = Arc::new(
            Store::abrir_em(&self.pasta.join("harness.db"), &self.pasta.join("rag.db"))
                .expect("reabre o banco"),
        );
    }

    fn evento(&self, mensagem: MessageId) -> String {
        EventoEntrada {
            conversa: self.conversa,
            mensagem,
        }
        .para_json()
        .expect("serializa o evento")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

/// Processador de teste: registra o que recebeu e conclui o item da `inbox`.
struct ProcessadorDeTeste {
    registro: Arc<Registro>,
    fluxo: Arc<OnceLock<Fluxo>>,
}

#[derive(Default)]
struct Registro {
    recebidas: Mutex<Vec<Tarefa>>,
    /// Espera antes de concluir (simula um turno lento).
    espera: Duration,
}

impl Registro {
    fn quantidade(&self) -> usize {
        self.recebidas.lock().expect("sem veneno").len()
    }
}

impl Processador for ProcessadorDeTeste {
    fn processar(&self, tarefa: Tarefa) -> Futuro {
        let registro = Arc::clone(&self.registro);
        let fluxo = Arc::clone(&self.fluxo);
        Box::pin(async move {
            if !registro.espera.is_zero() {
                tokio::time::sleep(registro.espera).await;
            }
            registro.recebidas.lock().expect("sem veneno").push(tarefa);
            if let (Some(entrada), Some(fluxo)) = (tarefa.entrada, fluxo.get()) {
                fluxo.concluir_entrada(tarefa.org, entrada, None)?;
            }
            Ok(())
        })
    }
}

/// Processador que não faz nada (filho do teste de `SIGKILL`).
struct SemProcessar;

impl Processador for SemProcessar {
    fn processar(&self, _tarefa: Tarefa) -> Futuro {
        Box::pin(async { Ok(()) })
    }
}

/// Monta um fluxo com o processador de teste. Devolve o fluxo, os atores e o registro.
fn montar_fluxo(
    cenario: &Cenario,
    opcoes: OpcoesFluxo,
    opcoes_atores: OpcoesAtores,
    espera: Duration,
) -> (Fluxo, Atores, Arc<Registro>) {
    let registro = Arc::new(Registro {
        recebidas: Mutex::new(Vec::new()),
        espera,
    });
    let fluxo_guardado: Arc<OnceLock<Fluxo>> = Arc::new(OnceLock::new());
    let atores = Atores::novo(
        Arc::new(ProcessadorDeTeste {
            registro: Arc::clone(&registro),
            fluxo: Arc::clone(&fluxo_guardado),
        }),
        opcoes_atores,
        MetricasMemoria::proprio(),
    );
    let fluxo = Fluxo::nova(Arc::clone(&cenario.store), atores.clone(), opcoes);
    let _ = fluxo_guardado.set(fluxo.clone());
    (fluxo, atores, registro)
}

fn opcoes_rapidas() -> OpcoesFluxo {
    OpcoesFluxo {
        backoff: Backoff {
            base: Duration::ZERO,
            fator: 2,
            teto: Duration::ZERO,
        },
        ..OpcoesFluxo::default()
    }
}

fn opcoes_atores(teto_fila: usize) -> OpcoesAtores {
    OpcoesAtores {
        ociosidade: Duration::from_secs(60),
        fila_por_conversa: teto_fila,
        max_atores: 32,
    }
}

/// Transporte que conta envios e responde com o roteiro dado.
struct TransporteDeTeste {
    envios: AtomicUsize,
    recebidos: Mutex<Vec<ItemSaida>>,
    falha: Mutex<Option<FalhaEnvio>>,
    espera: Duration,
}

impl TransporteDeTeste {
    fn novo() -> Arc<Self> {
        Arc::new(Self {
            envios: AtomicUsize::new(0),
            recebidos: Mutex::new(Vec::new()),
            falha: Mutex::new(None),
            espera: Duration::ZERO,
        })
    }

    fn com_espera(espera: Duration) -> Arc<Self> {
        let mut transporte = Self::novo();
        Arc::get_mut(&mut transporte).expect("recém-criado").espera = espera;
        transporte
    }

    fn com_falha(falha: FalhaEnvio) -> Arc<Self> {
        let transporte = Self::novo();
        *transporte.falha.lock().expect("sem veneno") = Some(falha);
        transporte
    }

    fn envios(&self) -> usize {
        self.envios.load(Ordering::SeqCst)
    }
}

impl TransporteSaida for TransporteDeTeste {
    fn enviar(&self, item: ItemSaida) -> FuturoEnvio {
        self.envios.fetch_add(1, Ordering::SeqCst);
        self.recebidos.lock().expect("sem veneno").push(item);
        let falha = self.falha.lock().expect("sem veneno").clone();
        let espera = self.espera;
        Box::pin(async move {
            if !espera.is_zero() {
                tokio::time::sleep(espera).await;
            }
            match falha {
                Some(falha) => Err(falha),
                None => Ok(()),
            }
        })
    }
}

/// Enfileira uma saída de texto simples para a conversa do cenário.
fn enfileirar_texto(fluxo: &Fluxo, cenario: &Cenario, turno: TurnId, seq: u32) -> ItemSaida {
    let (_novo, item) = fluxo
        .enfileirar_saida(
            cenario.org,
            NovaSaida {
                conversation_id: cenario.conversa,
                turn_id: turno,
                seq,
                tipo: "text".to_owned(),
                conteudo_json: format!("{{\"texto\":\"resposta {seq}\"}}"),
            },
        )
        .expect("enfileira a saída");
    item
}

/// Grava uma mensagem de cliente no banco (sem passar pela inbox) e devolve o id.
fn gravar_mensagem(cenario: &Cenario, turno: Option<TurnId>) -> MessageId {
    cenario
        .store
        .mensagens()
        .gravar(
            cenario.org,
            NovaMensagem {
                conversation_id: cenario.conversa,
                direcao: Direcao::Entrada,
                papel: Papel::Usuario,
                texto: "oi".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: turno,
                origem: poph_store::Origem::Agente,
                autor_nome: None,
            },
        )
        .expect("grava a mensagem")
        .id
}

// ───────────────────── 1. Duas entregas, um processamento ─────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mesma_mensagem_entregue_duas_vezes_e_processada_uma_vez() {
    let cenario = Cenario::novo("duplicada");
    let (fluxo, atores, registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);
    let mensagem = gravar_mensagem(&cenario, None);

    let primeira = fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: cenario.integracao,
                id_externo: "wamid-1".to_owned(),
                conversa: cenario.conversa,
                texto: "oi".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("primeira entrega");
    assert_eq!(
        primeira.situacao,
        poph_store::Recebido::Novo,
        "a primeira entrega é nova"
    );

    // Reentrega do webhook: mesmo par (integração, id externo).
    let segunda = fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: cenario.integracao,
                id_externo: "wamid-1".to_owned(),
                conversa: cenario.conversa,
                texto: "oi".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("reentrega não é erro");
    assert_eq!(
        segunda.situacao,
        poph_store::Recebido::Duplicado,
        "a segunda entrega é descartada como duplicada"
    );
    assert_eq!(
        segunda.item.id, primeira.item.id,
        "as duas entregas apontam para o mesmo item da inbox"
    );
    assert_eq!(
        segunda.mensagem.id, primeira.mensagem.id,
        "e para a mesma mensagem"
    );

    // O mesmo evento cru, pelo caminho de baixo, também é duplicado.
    let cru = fluxo
        .receber(
            cenario.org,
            NovaEntrada {
                integration_id: cenario.integracao,
                id_externo: "wamid-1".to_owned(),
                evento_json: cenario.evento(mensagem),
            },
        )
        .expect("entrega crua");
    assert_eq!(cru, poph_store::Recebido::Duplicado);

    for _ in 0..3 {
        fluxo.despachar(cenario.org).expect("despacha");
        assert!(atores.aguardar_filas_vazias(Duration::from_secs(5)).await);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(registro.quantidade(), 1, "um único processamento");
    assert_eq!(
        fluxo
            .estatisticas(cenario.org)
            .expect("estatísticas")
            .entrada_pendente,
        0,
        "o item saiu da fila depois de concluído"
    );
    assert_eq!(
        fluxo.metricas().entradas_duplicadas.valor(),
        2,
        "as duas reentregas contam como duplicadas"
    );
}

// ───────────────────── 2. SIGKILL entre gravar e processar ─────────────────────

/// Cenário do filho: grava a mensagem + inbox e para antes de processar.
#[test]
fn filho_grava_entrada_e_trava() {
    let Ok(rotulo) = std::env::var("POPH_HR12_FILHO") else {
        return;
    };
    if rotulo != "entrada" {
        return;
    }
    let pasta = PathBuf::from(std::env::var("POPH_HR12_PASTA").expect("pasta no ambiente"));
    let marcador = PathBuf::from(std::env::var("POPH_HR12_MARCADOR").expect("marcador"));
    let org: OrgId = std::env::var("POPH_HR12_ORG")
        .expect("org")
        .parse()
        .expect("org válida");
    let conversa: ConversationId = std::env::var("POPH_HR12_CONVERSA")
        .expect("conversa")
        .parse()
        .expect("conversa válida");
    let integracao: IntegrationId = std::env::var("POPH_HR12_INTEGRACAO")
        .expect("integração")
        .parse()
        .expect("integração válida");

    let store = Arc::new(
        Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre o banco"),
    );
    let atores = Atores::novo(
        Arc::new(SemProcessar),
        opcoes_atores(8),
        MetricasMemoria::proprio(),
    );
    let fluxo = Fluxo::nova(store, atores, opcoes_rapidas());
    let gravado = fluxo
        .receber_mensagem(
            org,
            MensagemRecebida {
                integration_id: integracao,
                id_externo: "wamid-sigkill".to_owned(),
                conversa,
                texto: "mensagem que não pode se perder".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava a entrada");
    assert_eq!(gravado.situacao, poph_store::Recebido::Novo);

    // Ponto controlado: já está no banco, ainda não foi processado.
    std::fs::write(&marcador, "gravado").expect("escreve o marcador");
    std::thread::sleep(Duration::from_secs(30));
    panic!("o pai devia ter matado o filho antes disso");
}

#[test]
fn sigkill_entre_gravar_e_processar_nao_perde_a_mensagem() {
    let mut cenario = Cenario::novo("sigkill-entrada");
    let org = cenario.org;
    let conversa = cenario.conversa;
    let integracao = cenario.integracao;
    let pasta = cenario.pasta.clone();
    let marcador = pasta.join("marcador-entrada");
    // Solta o Store antes de o filho abrir o banco (um escritor por processo de cada vez).
    cenario.store = Arc::new(
        Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("reabre"),
    );

    let filho = std::process::Command::new(std::env::current_exe().expect("binário de teste"))
        .args([
            "--exact",
            "filho_grava_entrada_e_trava",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("POPH_HR12_FILHO", "entrada")
        .env("POPH_HR12_PASTA", &pasta)
        .env("POPH_HR12_MARCADOR", &marcador)
        .env("POPH_HR12_ORG", org.to_string())
        .env("POPH_HR12_CONVERSA", conversa.to_string())
        .env("POPH_HR12_INTEGRACAO", integracao.to_string())
        .spawn()
        .expect("inicia o filho");

    esperar_marcador(&marcador);
    matar(filho);

    // O item ficou gravado e pendente: a queda não perdeu a mensagem.
    cenario.reabrir();
    assert_eq!(
        cenario
            .store
            .caixa_entrada()
            .contar_pendentes(org)
            .expect("conta"),
        1,
        "a entrada pendente sobreviveu à queda"
    );
    let pendentes = cenario
        .store
        .caixa_entrada()
        .listar_pendentes(org, 10)
        .expect("lista");
    assert_eq!(pendentes[0].id_externo, "wamid-sigkill");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (fluxo, atores, registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);
    let transporte = TransporteDeTeste::novo();
    let retomada = runtime
        .block_on(async {
            let retomada = fluxo.retomar(transporte.as_ref()).await?;
            assert!(atores.aguardar_filas_vazias(Duration::from_secs(10)).await);
            Ok::<_, poph_agent::ErroAgente>(retomada)
        })
        .expect("retomada");

    assert_eq!(
        retomada.entrada.despachados, 1,
        "a retomada despachou o item pendente"
    );
    assert_eq!(registro.quantidade(), 1, "processado uma vez na retomada");
    assert_eq!(
        cenario
            .store
            .caixa_entrada()
            .contar_pendentes(org)
            .expect("conta"),
        0,
        "depois de processado, a fila fica vazia"
    );
    // A mensagem do cliente também sobreviveu à queda.
    let mensagens = cenario
        .store
        .mensagens()
        .listar(org, conversa, poph_store::Pagina::com_limite(10))
        .expect("lista mensagens");
    assert!(
        mensagens
            .iter()
            .any(|mensagem| mensagem.texto == "mensagem que não pode se perder"),
        "a mensagem gravada na mesma transação continua lá"
    );
}

// ───────────────────── 3. SIGKILL entre sending e sent ─────────────────────

#[test]
fn filho_fica_em_sending_e_trava() {
    let Ok(rotulo) = std::env::var("POPH_HR12_FILHO") else {
        return;
    };
    if rotulo != "saida" {
        return;
    }
    let pasta = PathBuf::from(std::env::var("POPH_HR12_PASTA").expect("pasta"));
    let marcador = PathBuf::from(std::env::var("POPH_HR12_MARCADOR").expect("marcador"));
    let org: OrgId = std::env::var("POPH_HR12_ORG")
        .expect("org")
        .parse()
        .expect("org");
    let conversa: ConversationId = std::env::var("POPH_HR12_CONVERSA")
        .expect("conversa")
        .parse()
        .expect("conversa");
    let turno: TurnId = std::env::var("POPH_HR12_TURNO")
        .expect("turno")
        .parse()
        .expect("turno");

    let store =
        Arc::new(Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre"));
    let (_novo, item) = store
        .caixa_saida()
        .enfileirar(
            org,
            NovaSaida {
                conversation_id: conversa,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                conteudo_json: "{\"texto\":\"resposta\"}".to_owned(),
            },
        )
        .expect("enfileira");
    // Transição gravada antes do efeito externo; o filho morre antes de chamar o canal.
    store
        .caixa_saida()
        .iniciar_envio(org, item.id)
        .expect("entra em sending");
    std::fs::write(&marcador, "sending").expect("escreve o marcador");
    std::thread::sleep(Duration::from_secs(30));
    panic!("o pai devia ter matado o filho antes disso");
}

#[test]
fn sigkill_entre_sending_e_sent_reenvia_uma_vez() {
    let mut cenario = Cenario::novo("sigkill-saida");
    let org = cenario.org;
    let conversa = cenario.conversa;
    let turno = TurnId::generate();
    let pasta = cenario.pasta.clone();
    let marcador = pasta.join("marcador-saida");
    // Deixa o banco fechado para o filho.
    cenario.store = Arc::new(
        Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("reabre"),
    );

    let filho = std::process::Command::new(std::env::current_exe().expect("binário de teste"))
        .args([
            "--exact",
            "filho_fica_em_sending_e_trava",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("POPH_HR12_FILHO", "saida")
        .env("POPH_HR12_PASTA", &pasta)
        .env("POPH_HR12_MARCADOR", &marcador)
        .env("POPH_HR12_ORG", org.to_string())
        .env("POPH_HR12_CONVERSA", conversa.to_string())
        .env("POPH_HR12_TURNO", turno.to_string())
        .spawn()
        .expect("inicia o filho");

    esperar_marcador(&marcador);
    matar(filho);

    cenario.reabrir();
    assert_eq!(
        cenario
            .store
            .caixa_saida()
            .contar_por_estado(org, EstadoSaida::Enviando)
            .expect("conta"),
        1,
        "o item ficou em sending depois da queda"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (fluxo, atores, _registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);
    let transporte = TransporteDeTeste::novo();
    let retomada = runtime
        .block_on(fluxo.retomar(transporte.as_ref()))
        .expect("retomada");

    assert_eq!(retomada.reenvios, 1, "um item estava em sending");
    assert_eq!(transporte.envios(), 1, "reenviou exatamente uma vez");
    assert_eq!(
        fluxo.metricas().reenvios_retomada.valor(),
        1,
        "a métrica de reenvio conta a retomada"
    );
    let pendentes = cenario
        .store
        .caixa_saida()
        .listar_pendentes(org, 10)
        .expect("lista");
    assert!(pendentes.is_empty(), "nada sobrou na fila de saída");
    let itens = cenario
        .store
        .caixa_saida()
        .contar_por_estado(org, EstadoSaida::Enviado)
        .expect("conta");
    assert_eq!(itens, 1, "e o item terminou como enviado");

    // Rodar a retomada de novo não reenvia nada (o item já está sent).
    let segunda = runtime
        .block_on(fluxo.retomar(transporte.as_ref()))
        .expect("segunda retomada");
    assert_eq!(segunda.reenvios, 0);
    assert_eq!(transporte.envios(), 1, "nada duplicado na segunda subida");
    drop(atores);
}

// ───────────────────── 4. Falha repetida termina em failed ─────────────────────

#[tokio::test]
async fn falha_repetida_vai_para_failed_com_motivo_e_contador() {
    let cenario = Cenario::novo("falha-repetida");
    let (fluxo, _atores, _registro) = montar_fluxo(
        &cenario,
        OpcoesFluxo {
            max_tentativas: 3,
            ..opcoes_rapidas()
        },
        opcoes_atores(8),
        Duration::ZERO,
    );
    let transporte = TransporteDeTeste::com_falha(FalhaEnvio::transitoria("net::ERR_CONNECTION"));
    let item = enfileirar_texto(&fluxo, &cenario, TurnId::generate(), 0);

    for _ in 0..4 {
        fluxo
            .enviar_pendentes(cenario.org, transporte.as_ref())
            .await
            .expect("envia");
    }

    let gravado = cenario
        .store
        .caixa_saida()
        .obter(cenario.org, item.id)
        .expect("busca");
    assert_eq!(gravado.estado, EstadoSaida::Falhou);
    assert_eq!(
        gravado.tentativas, 3,
        "três tentativas reais antes de desistir"
    );
    assert_eq!(
        gravado.ultimo_erro.as_deref(),
        Some("tentativas de envio esgotadas"),
        "o motivo fica legível no banco"
    );
    assert_eq!(transporte.envios(), 3, "não tentou depois de esgotar");
    assert_eq!(fluxo.metricas().saidas_transitorias.valor(), 3);
    assert_eq!(fluxo.metricas().saidas_esgotadas.valor(), 1);
}

#[tokio::test]
async fn falha_definitiva_nao_retenta() {
    let cenario = Cenario::novo("falha-definitiva");
    let (fluxo, _atores, _registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);
    let transporte = TransporteDeTeste::com_falha(FalhaEnvio::definitiva("destinatário inválido"));
    let item = enfileirar_texto(&fluxo, &cenario, TurnId::generate(), 0);

    fluxo
        .enviar_pendentes(cenario.org, transporte.as_ref())
        .await
        .expect("envia");
    fluxo
        .enviar_pendentes(cenario.org, transporte.as_ref())
        .await
        .expect("envia de novo");

    let gravado = cenario
        .store
        .caixa_saida()
        .obter(cenario.org, item.id)
        .expect("busca");
    assert_eq!(gravado.estado, EstadoSaida::Falhou);
    assert_eq!(
        gravado.ultimo_erro.as_deref(),
        Some("destinatário inválido")
    );
    assert_eq!(gravado.tentativas, 1, "erro definitivo não repete");
    assert_eq!(transporte.envios(), 1);
    assert_eq!(fluxo.metricas().saidas_definitivas.valor(), 1);
}

#[tokio::test]
async fn saida_idempotente_por_turno_e_sequencia() {
    let cenario = Cenario::novo("saida-idempotente");
    let (fluxo, _atores, _registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);
    let turno = TurnId::generate();
    let primeira = enfileirar_texto(&fluxo, &cenario, turno, 0);
    let (_novo, segunda) = fluxo
        .enfileirar_saida(
            cenario.org,
            NovaSaida {
                conversation_id: cenario.conversa,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                conteudo_json: "{\"texto\":\"outro\"}".to_owned(),
            },
        )
        .expect("enfileira de novo");
    assert_eq!(primeira.id, segunda.id, "a mesma chave não duplica a linha");
    assert_eq!(
        segunda.conteudo_json, "{\"texto\":\"resposta 0\"}",
        "o conteúdo original é preservado"
    );
}

// ───────────────────── 5. Backoff cresce e para no teto ─────────────────────

#[test]
fn backoff_cresce_e_para_no_teto() {
    let backoff = Backoff {
        base: Duration::from_millis(100),
        fator: 2,
        teto: Duration::from_secs(1),
    };
    let esperas: Vec<u128> = (1..=8)
        .map(|tentativa| backoff.espera(tentativa).as_millis())
        .collect();
    assert_eq!(esperas, vec![100, 200, 400, 800, 1000, 1000, 1000, 1000]);
    assert!(
        esperas.windows(2).all(|par| par[0] <= par[1]),
        "nunca diminui"
    );
    assert_eq!(
        backoff.espera(0),
        Duration::from_millis(100),
        "tentativa 0 = base"
    );
}

#[tokio::test]
async fn item_em_backoff_nao_e_reenviado_antes_da_hora() {
    let cenario = Cenario::novo("backoff-virtual");
    let relogio = Arc::new(Mutex::new(Instant::now()));
    let relogio_para_o_fluxo = Arc::clone(&relogio);
    let (fluxo, _atores, _registro) = montar_fluxo(
        &cenario,
        OpcoesFluxo {
            backoff: Backoff {
                base: Duration::from_secs(2),
                fator: 2,
                teto: Duration::from_secs(30),
            },
            ..OpcoesFluxo::default()
        },
        opcoes_atores(8),
        Duration::ZERO,
    );
    fluxo.com_relogio(Arc::new(move || {
        *relogio_para_o_fluxo.lock().expect("sem veneno")
    }));

    let transporte = TransporteDeTeste::com_falha(FalhaEnvio::transitoria("429"));
    enfileirar_texto(&fluxo, &cenario, TurnId::generate(), 0);

    let primeiro = fluxo
        .enviar_pendentes(cenario.org, transporte.as_ref())
        .await
        .expect("envia");
    assert_eq!(primeiro.falharam, 1);
    assert_eq!(transporte.envios(), 1);

    // Sem andar o relógio, a repetição fica adiada.
    let adiado = fluxo
        .enviar_pendentes(cenario.org, transporte.as_ref())
        .await
        .expect("envia");
    assert_eq!(adiado.adiados, 1, "ainda dentro do backoff");
    assert_eq!(transporte.envios(), 1, "não chamou o canal de novo");

    // Avança 2 s (a espera da primeira tentativa) e a repetição acontece.
    *relogio.lock().expect("sem veneno") += Duration::from_secs(2);
    let segundo = fluxo
        .enviar_pendentes(cenario.org, transporte.as_ref())
        .await
        .expect("envia");
    assert_eq!(segundo.falharam, 1);
    assert_eq!(transporte.envios(), 2, "repetiu depois da espera");

    // A segunda espera é maior (4 s): 3 s depois ainda não é hora.
    *relogio.lock().expect("sem veneno") += Duration::from_secs(3);
    let ainda = fluxo
        .enviar_pendentes(cenario.org, transporte.as_ref())
        .await
        .expect("envia");
    assert_eq!(ainda.adiados, 1, "a espera dobrou");
    assert_eq!(transporte.envios(), 2);
}

// ───────────────────── 6. Isolamento por org ─────────────────────

#[tokio::test]
async fn caixas_nao_cruzam_org() {
    let cenario = Cenario::novo("isolamento");
    let outra = cenario
        .store
        .orgaos()
        .criar(NovaOrg {
            nome: "Outra org".to_owned(),
            slug: "outra-org-hr12".to_owned(),
            config_json: String::new(),
        })
        .expect("cria a outra org")
        .id;
    let (fluxo, _atores, _registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);

    let recebido = fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: cenario.integracao,
                id_externo: "wamid-org-a".to_owned(),
                conversa: cenario.conversa,
                texto: "da org A".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava na org A");
    let saida = enfileirar_texto(&fluxo, &cenario, TurnId::generate(), 0);

    assert_eq!(
        cenario
            .store
            .caixa_entrada()
            .contar_pendentes(outra)
            .expect("conta"),
        0,
        "a org B não vê a entrada da org A"
    );
    assert!(
        cenario
            .store
            .caixa_entrada()
            .listar_pendentes(outra, 10)
            .expect("lista")
            .is_empty()
    );
    assert!(
        cenario
            .store
            .caixa_entrada()
            .obter(outra, recebido.item.id)
            .is_err(),
        "buscar o item da org A com a org B falha"
    );
    assert!(
        cenario
            .store
            .caixa_saida()
            .listar_pendentes(outra, 10)
            .expect("lista")
            .is_empty()
    );
    assert!(
        cenario.store.caixa_saida().obter(outra, saida.id).is_err(),
        "buscar a saída da org A com a org B falha"
    );
    assert!(
        cenario
            .store
            .caixa_saida()
            .contar_por_estado(outra, EstadoSaida::Pendente)
            .expect("conta")
            == 0
    );
    assert!(
        cenario
            .store
            .caixa_saida()
            .iniciar_envio(outra, saida.id)
            .is_err(),
        "não dá para mexer no estado da saída de outra org"
    );
    assert_eq!(
        cenario
            .store
            .caixa_saida()
            .obter(cenario.org, saida.id)
            .expect("na org certa, existe")
            .estado,
        EstadoSaida::Pendente,
        "a tentativa da org errada não mudou nada"
    );
}

// ───────────────────── Extra: fila cheia não perde evento ─────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fila_cheia_adia_e_nao_perde_o_evento() {
    let cenario = Cenario::novo("fila-cheia");
    let (fluxo, atores, registro) = montar_fluxo(
        &cenario,
        opcoes_rapidas(),
        opcoes_atores(1),
        Duration::from_millis(120),
    );

    let mut itens = HashMap::new();
    for indice in 0..3 {
        let recebido = fluxo
            .receber_mensagem(
                cenario.org,
                MensagemRecebida {
                    integration_id: cenario.integracao,
                    id_externo: format!("wamid-fila-{indice}"),
                    conversa: cenario.conversa,
                    texto: format!("mensagem {indice}"),
                    anexos_json: "[]".to_owned(),
                },
            )
            .expect("grava");
        itens.insert(indice, recebido.item.id);
    }

    let primeiro = fluxo.despachar(cenario.org).expect("despacha");
    // A fila da conversa é de 1. O ator pode tirar o primeiro evento da fila entre um envio e o
    // seguinte (a janela é de microssegundos e depende do escalonador), então o que se cobra é
    // "nada se perdeu", não a divisão exata entre despachados e adiados.
    assert_eq!(
        primeiro.despachados + primeiro.adiados,
        3,
        "todos os eventos foram vistos no despacho"
    );
    assert!(primeiro.despachados >= 1, "ao menos um foi despachado");
    assert_eq!(
        cenario
            .store
            .caixa_entrada()
            .contar_pendentes(cenario.org)
            .expect("conta"),
        3,
        "nada foi marcado como processado no despacho"
    );

    // Deixa o turno lento terminar e o ator esvaziar; o próximo despacho pega os adiados.
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(60)).await;
        fluxo.despachar(cenario.org).expect("despacha");
        if registro.quantidade() == 3 {
            break;
        }
    }
    assert!(atores.aguardar_filas_vazias(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        registro.quantidade(),
        3,
        "as três mensagens foram processadas"
    );
    assert_eq!(
        cenario
            .store
            .caixa_entrada()
            .contar_pendentes(cenario.org)
            .expect("conta"),
        0
    );
    if primeiro.adiados > 0 {
        assert!(
            fluxo.metricas().entradas_adiadas.valor() >= 1,
            "evento adiado conta na métrica"
        );
    }
    let _ = itens;
}

// ───────────────────── Extra: lista lida antes da conclusão não repete o item ─────────────────────

/// Corrida que a intermitência de `fila_cheia_adia_e_nao_perde_o_evento` revelou: o despacho lê a
/// lista de pendentes, o turno conclui o item, e só então o despacho chega a ele com a lista antiga.
/// A reserva já foi solta, então sem conferir o banco o item sairia de novo e o turno rodaria duas
/// vezes (uma resposta duplicada para o cliente).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lista_lida_antes_da_conclusao_nao_repete_item_processado() {
    let cenario = Cenario::novo("lista-antiga");
    let (fluxo, atores, registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(4), Duration::ZERO);
    fluxo
        .receber_mensagem(
            cenario.org,
            MensagemRecebida {
                integration_id: cenario.integracao,
                id_externo: "wamid-lista-antiga".to_owned(),
                conversa: cenario.conversa,
                texto: "mensagem única".to_owned(),
                anexos_json: "[]".to_owned(),
            },
        )
        .expect("grava");

    // Lista lida pelo despacho antes de o turno existir (o item ainda está pendente).
    let antiga = cenario
        .store
        .caixa_entrada()
        .listar_pendentes(cenario.org, 10)
        .expect("lista");
    assert_eq!(antiga.len(), 1, "a lista antiga traz o item");

    let primeiro = fluxo.despachar(cenario.org).expect("despacha");
    assert_eq!(primeiro.despachados, 1);
    assert!(atores.aguardar_filas_vazias(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(registro.quantidade(), 1, "o turno rodou uma vez");
    assert_eq!(
        cenario
            .store
            .caixa_entrada()
            .contar_pendentes(cenario.org)
            .expect("conta"),
        0,
        "o item já está processado no banco"
    );

    // Agora chega o despacho com a lista antiga.
    let relato = fluxo
        .despachar_itens(cenario.org, antiga)
        .expect("despacha a lista antiga");
    assert_eq!(
        relato.despachados, 0,
        "item já processado não pode ser despachado de novo"
    );
    assert!(atores.aguardar_filas_vazias(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        registro.quantidade(),
        1,
        "o turno não pode rodar duas vezes para o mesmo item"
    );
}

// ───────────────────── Extra: ciclos concorrentes não duplicam ─────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dois_despachos_concorrentes_nao_duplicam_o_evento() {
    let cenario = Cenario::novo("concorrencia-entrada");
    let (fluxo, atores, registro) = montar_fluxo(
        &cenario,
        opcoes_rapidas(),
        opcoes_atores(64),
        Duration::ZERO,
    );
    for indice in 0..6 {
        fluxo
            .receber_mensagem(
                cenario.org,
                MensagemRecebida {
                    integration_id: cenario.integracao,
                    id_externo: format!("wamid-concorrente-{indice}"),
                    conversa: cenario.conversa,
                    texto: format!("concorrente {indice}"),
                    anexos_json: "[]".to_owned(),
                },
            )
            .expect("grava");
    }

    let handle = tokio::runtime::Handle::current();
    let resultados: Vec<poph_agent::entrada_saida::RelatoDespacho> = std::thread::scope(|escopo| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let fluxo = fluxo.clone();
            let org = cenario.org;
            let handle = handle.clone();
            handles.push(escopo.spawn(move || {
                let _contexto = handle.enter();
                fluxo.despachar(org).expect("despacha")
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("thread do despacho"))
            .collect()
    });

    let despachados: u32 = resultados.iter().map(|relato| relato.despachados).sum();
    assert_eq!(despachados, 6, "cada item foi reservado uma única vez");
    assert!(atores.aguardar_filas_vazias(Duration::from_secs(10)).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        registro.quantidade(),
        6,
        "seis processamentos, nem um a mais"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dois_envios_concorrentes_nao_duplicam_a_saida() {
    let cenario = Cenario::novo("concorrencia-saida");
    let (fluxo, _atores, _registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);
    for _ in 0..4 {
        enfileirar_texto(&fluxo, &cenario, TurnId::generate(), 0);
    }
    let transporte = TransporteDeTeste::com_espera(Duration::from_millis(40));

    let (primeiro, segundo) = tokio::join!(
        fluxo.enviar_pendentes(cenario.org, transporte.as_ref()),
        fluxo.enviar_pendentes(cenario.org, transporte.as_ref()),
    );
    let primeiro = primeiro.expect("envia");
    let segundo = segundo.expect("envia");

    assert_eq!(transporte.envios(), 4, "cada item foi ao canal uma vez só");
    assert_eq!(
        primeiro.enviados + segundo.enviados,
        4,
        "as duas passagens somam os quatro envios"
    );
    assert_eq!(
        fluxo
            .estatisticas(cenario.org)
            .expect("estatísticas")
            .saida_pendente,
        0
    );
}

// ───────────────────── Utilidades ─────────────────────

/// Espera o filho escrever o marcador (até 30 s).
fn esperar_marcador(marcador: &Path) {
    let inicio = Instant::now();
    while inicio.elapsed() < Duration::from_secs(30) {
        if marcador.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("o filho não escreveu o marcador em {} s", 30);
}

/// Mata o filho com `SIGKILL` e espera ele sair.
fn matar(mut filho: std::process::Child) {
    filho.kill().expect("SIGKILL no filho");
    let saida = filho.wait().expect("espera o filho");
    assert!(
        !saida.success(),
        "o filho devia ter morrido por sinal, não saído com sucesso"
    );
}

// ───────────────────── Medição (fora do CI) ─────────────────────

/// Mede a vazão e a latência do ACK da ingestão atômica (mensagem + inbox numa transação).
///
/// `cargo test --release -p poph-agent --test entrada_saida -- --ignored --nocapture medir_ingestao`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "medição, não checagem"]
async fn medir_ingestao() {
    const THREADS: usize = 8;
    const POR_THREAD: usize = 1500;

    let cenario = Cenario::novo("medir-ingestao");
    let (fluxo, _atores, _registro) =
        montar_fluxo(&cenario, opcoes_rapidas(), opcoes_atores(8), Duration::ZERO);

    let inicio = Instant::now();
    let mut latencias: Vec<u64> = std::thread::scope(|escopo| {
        let mut handles = Vec::new();
        for thread in 0..THREADS {
            let fluxo = fluxo.clone();
            let org = cenario.org;
            let integracao = cenario.integracao;
            let conversa = cenario.conversa;
            handles.push(escopo.spawn(move || {
                let mut micros = Vec::with_capacity(POR_THREAD);
                for indice in 0..POR_THREAD {
                    let id = format!("wamid-medir-{thread}-{indice}");
                    let comeco = Instant::now();
                    fluxo
                        .receber_mensagem(
                            org,
                            MensagemRecebida {
                                integration_id: integracao,
                                id_externo: id,
                                conversa,
                                texto: "mensagem de medição".to_owned(),
                                anexos_json: "[]".to_owned(),
                            },
                        )
                        .expect("ingere");
                    micros.push(u64::try_from(comeco.elapsed().as_micros()).unwrap_or(u64::MAX));
                }
                micros
            }));
        }
        let mut todas = Vec::with_capacity(THREADS * POR_THREAD);
        for handle in handles {
            todas.extend(handle.join().expect("thread de medição"));
        }
        todas
    });
    let total = inicio.elapsed();
    latencias.sort_unstable();
    let p = |fracao: f64| {
        latencias[((latencias.len() as f64 * fracao) as usize).min(latencias.len() - 1)]
    };
    let quantidade = latencias.len();
    println!(
        "ingestão atômica: {quantidade} mensagens em {:.2} s = {:.0} msg/s; ACK p50 {} µs, p95 {} µs, p99 {} µs",
        total.as_secs_f64(),
        quantidade as f64 / total.as_secs_f64(),
        p(0.50),
        p(0.95),
        p(0.99)
    );
    assert_eq!(
        cenario
            .store
            .caixa_entrada()
            .contar_pendentes(cenario.org)
            .expect("conta"),
        (THREADS * POR_THREAD) as u64,
        "todas as mensagens ficaram na fila, nenhuma duplicada nem perdida"
    );
}
