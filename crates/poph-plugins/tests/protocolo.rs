//! Testes do contrato de plugin (HR-25), com o plugin de fixtura de verdade como processo filho.
//!
//! Cada teste sobe o binário `plugin-fixtura` (compilado pelo cargo para os testes de integração),
//! conversa JSON-lines com ele e confere o que aconteceu — inclusive pelo que o próprio plugin
//! gravou em disco (`dump_dir`), que é a única forma de provar o que **chegou** nele.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use poph_plugins::{ColetorEventos, Espera, LimitesSupervisor, OpcoesPlugin, Supervisor};
use poph_types::{
    ErroIntegracao, EstadoIntegracao, EventoEstado, EventoIntegracao, Integration, IntegrationId,
    OrgId, PROTOCOLO_VERSAO, Presenca, Receptor, SendFile, SendText, SetPresence,
};

// ───────────────────────────── Apoio ─────────────────────────────

/// Binário de fixtura que o cargo compila antes de rodar este teste.
fn fixtura() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_plugin-fixtura"))
}

/// Prazos curtos para o teste não ficar esperando: o comportamento é o mesmo do padrão.
fn limites_rapidos() -> LimitesSupervisor {
    LimitesSupervisor {
        max_mensagem_bytes: 1024 * 1024,
        timeout_handshake: Duration::from_secs(2),
        timeout_comando: Duration::from_millis(300),
        timeout_saude: Duration::from_millis(300),
        espera: Espera {
            base: Duration::from_millis(30),
            fator: 2,
            teto: Duration::from_millis(200),
        },
        max_falhas: 3,
    }
}

/// Pasta temporária do cenário, apagada no fim.
struct Cenario {
    raiz: PathBuf,
}

impl Cenario {
    fn novo(nome: &str) -> Self {
        static CONTADOR: Mutex<u64> = Mutex::new(0);
        let mut contador = CONTADOR.lock().unwrap();
        *contador += 1;
        let raiz = std::env::temp_dir().join(format!(
            "poph-plugin-{nome}-{}-{}",
            std::process::id(),
            *contador
        ));
        let _ = std::fs::remove_dir_all(&raiz);
        std::fs::create_dir_all(&raiz).expect("criar a pasta do cenário");
        let raiz = raiz.canonicalize().unwrap_or(raiz);
        Self { raiz }
    }

    fn dump(&self) -> PathBuf {
        self.raiz.join("dump")
    }

    fn opcoes(&self, modo: &str) -> OpcoesPlugin {
        OpcoesPlugin::novo(fixtura(), IntegrationId::generate(), OrgId::generate())
            .com_kind("fixtura")
            .com_config(json!({
                "modo": modo,
                "dump_dir": self.dump().display().to_string(),
            }))
            .com_limites(limites_rapidos())
    }

    fn ler(&self, nome: &str) -> String {
        std::fs::read_to_string(self.dump().join(nome)).unwrap_or_default()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.raiz);
    }
}

fn enviar_texto(supervisor: &Supervisor, texto: &str) -> poph_types::FuturoComando {
    supervisor.enviar_texto(SendText {
        idempotencia: "turno-1:0".to_owned(),
        peer: "5511900000000".to_owned(),
        texto: texto.to_owned(),
    })
}

/// Espera a condição virar verdadeira, sem dormir mais que o prazo.
async fn esperar(condicao: impl Fn() -> bool, prazo: Duration) -> bool {
    let limite = Instant::now() + prazo;
    while Instant::now() < limite {
        if condicao() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    condicao()
}

/// Subscriber global de log que guarda tudo em memória, para o teste do segredo.
fn captura_log() -> &'static Arc<Mutex<Vec<u8>>> {
    struct Escritor(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Escritor {
        fn write(&mut self, dados: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(dados);
            Ok(dados.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    static LOG: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let para_escritor = Arc::clone(&buffer);
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_ansi(false)
                .with_writer(move || Escritor(Arc::clone(&para_escritor)))
                .finish(),
        );
        buffer
    })
}

fn log_atual() -> String {
    String::from_utf8_lossy(&captura_log().lock().unwrap()).into_owned()
}

// ───────────────────────────── Handshake e contrato ─────────────────────────────

#[tokio::test]
async fn handshake_manifesto_saude_texto_arquivo_e_presenca() {
    let cenario = Cenario::novo("contrato");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let integracao = IntegrationId::generate();
    let org = OrgId::generate();
    let opcoes = OpcoesPlugin::novo(fixtura(), integracao, org)
        .com_kind("fixtura")
        .com_config(json!({
            "modo": "normal",
            "dump_dir": cenario.dump().display().to_string(),
        }))
        .com_limites(limites_rapidos());
    let supervisor = Supervisor::iniciar(opcoes, receptor.clone())
        .await
        .expect("handshake");
    let (id_integracao, org_do_supervisor, kind) = supervisor.integracao();
    assert_eq!(id_integracao, integracao);
    assert_eq!(org_do_supervisor, org);
    assert_eq!(kind, "fixtura");

    let manifesto = supervisor.manifesto().expect("manifesto no handshake");
    assert_eq!(manifesto.kind, "fixtura");
    assert_eq!(manifesto.protocolo, PROTOCOLO_VERSAO);
    assert!(manifesto.capacidades.text);
    assert!(manifesto.capacidades.file_out);
    assert!(!manifesto.capacidades.read_receipt);
    assert_eq!(manifesto.limites.max_anexo_bytes, 16_777_216);
    assert_eq!(supervisor.estado(), EstadoIntegracao::Pronto);

    let saude = supervisor.saude().await.expect("saúde");
    assert!(saude.ok);
    assert_eq!(saude.detalhe.as_deref(), Some("fixtura"));

    enviar_texto(&supervisor, "olá")
        .await
        .expect("texto entregue");
    supervisor
        .enviar_arquivo(SendFile {
            idempotencia: "turno-1:1".to_owned(),
            peer: "5511900000000".to_owned(),
            nome: "manual.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            tamanho_bytes: 1_234,
            sha256: "a".repeat(64),
            blob_ref: "01a1f6bd-0000-7000-8000-000000000000/aaaaaaaa".to_owned(),
            legenda: Some("segue o manual".to_owned()),
        })
        .await
        .expect("arquivo entregue");
    supervisor
        .definir_presenca(SetPresence {
            peer: "5511900000000".to_owned(),
            estado: Presenca::Digitando,
        })
        .await
        .expect("presença");

    // O plugin gravou o que recebeu: é o que prova que o comando chegou com o conteúdo certo.
    let comandos: Vec<Value> = cenario
        .ler("comandos.jsonl")
        .lines()
        .map(|linha| serde_json::from_str(linha).expect("linha de comando em JSON"))
        .collect();
    assert_eq!(comandos.len(), 3, "três comandos: {comandos:?}");
    assert_eq!(comandos[0]["comando"]["tipo"], "enviar_texto");
    assert_eq!(comandos[0]["comando"]["idempotencia"], "turno-1:0");
    assert_eq!(comandos[0]["comando"]["texto"], "olá");
    assert_eq!(comandos[1]["comando"]["tipo"], "enviar_arquivo");
    assert_eq!(comandos[1]["comando"]["mime"], "application/pdf");
    assert_eq!(comandos[1]["comando"]["legenda"], "segue o manual");
    assert_eq!(comandos[2]["comando"]["tipo"], "definir_presenca");
    assert_eq!(comandos[2]["comando"]["estado"], "digitando");

    let contadores = supervisor.contadores();
    assert_eq!(contadores.handshakes, 1);
    assert_eq!(contadores.comandos, 3);
    assert!(contadores.saudes >= 1, "{contadores:?}");
    assert_eq!(contadores.lixo, 0, "{contadores:?}");

    supervisor.encerrar().await;
    assert_eq!(supervisor.estado(), EstadoIntegracao::Encerrada);
    // O `encerrar` chega ao plugin, que sai sozinho e deixa o registro.
    assert_eq!(cenario.ler("saiu.txt").trim(), "sim");
}

#[tokio::test]
async fn evento_de_entrada_e_estado_chegam_normalizados() {
    let cenario = Cenario::novo("evento");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let opcoes = cenario.opcoes("normal").com_config(json!({
        "modo": "normal",
        "dump_dir": cenario.dump().display().to_string(),
        "emitir_evento": true,
        "emitir_estado": "conectado",
        "id_externo": "msg-fixtura-7",
        "peer": "5511988887777",
        "texto": "quero um orçamento",
    }));
    let supervisor = Supervisor::iniciar(opcoes, receptor.clone())
        .await
        .expect("handshake");
    let (integracao, _, _) = supervisor.integracao();

    let eventos = coletor.esperar(2, Duration::from_secs(2)).await;
    assert_eq!(eventos.len(), 2, "entrada + estado: {eventos:?}");

    let entrada = eventos
        .iter()
        .find_map(|evento| match evento {
            EventoIntegracao::Entrada(entrada) => Some(entrada.clone()),
            EventoIntegracao::Estado(_) => None,
        })
        .expect("evento de entrada");
    assert_eq!(entrada.integracao, integracao);
    assert_eq!(entrada.id_externo, "msg-fixtura-7");
    assert_eq!(entrada.peer, "5511988887777");
    assert_eq!(entrada.texto, "quero um orçamento");
    assert_eq!(entrada.ocorrido_em, 1_795_140_000_000);
    assert!(entrada.anexos.is_empty());
    // O evento entregue passa pela validação do contrato.
    assert_eq!(entrada.validar(), Ok(()));

    let estados = coletor.estados();
    assert_eq!(estados.len(), 1, "{estados:?}");
    assert!(matches!(estados[0], EventoEstado::Conectado { .. }));

    let contadores = supervisor.contadores();
    assert_eq!(contadores.eventos, 1);
    assert_eq!(contadores.estados, 1);
    supervisor.encerrar().await;
}

#[tokio::test]
async fn plugin_que_trava_no_handshake_e_abortado_pelo_prazo() {
    let cenario = Cenario::novo("trava-handshake");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let inicio = Instant::now();
    let erro = Supervisor::iniciar(cenario.opcoes("trava_no_handshake"), receptor.clone())
        .await
        .expect_err("o handshake tinha de estourar");
    let decorrido = inicio.elapsed();
    assert!(
        matches!(erro, ErroIntegracao::TempoEsgotado { .. }),
        "erro: {erro}"
    );
    assert!(
        decorrido < Duration::from_secs(3),
        "demorou {decorrido:?} para desistir"
    );
}

#[tokio::test]
async fn plugin_que_trava_no_comando_estoura_o_prazo_e_volta_depois_do_reinicio() {
    let cenario = Cenario::novo("trava-comando");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let supervisor = Supervisor::iniciar(
        cenario.opcoes("trava_no_primeiro_comando"),
        receptor.clone(),
    )
    .await
    .expect("handshake");

    let inicio = Instant::now();
    let erro = enviar_texto(&supervisor, "primeira")
        .await
        .expect_err("o comando tinha de estourar");
    assert!(
        matches!(erro, ErroIntegracao::TempoEsgotado { .. }),
        "erro: {erro}"
    );
    assert!(inicio.elapsed() < Duration::from_secs(2));
    assert!(erro.eh_transitorio());
    assert!(supervisor.contadores().tempo_esgotado >= 1);

    // O supervisor não fica preso: o processo travado é morto e o plugin volta, respondendo.
    assert!(
        esperar(
            || supervisor.contadores().reinicios >= 1,
            Duration::from_secs(3)
        )
        .await,
        "o plugin não foi reiniciado: {:?}",
        supervisor.contadores()
    );
    supervisor
        .aguardar_pronto(Duration::from_secs(2))
        .await
        .expect("pronto depois do reinício");
    enviar_texto(&supervisor, "segunda")
        .await
        .expect("depois do reinício responde");
    supervisor.encerrar().await;
}

#[tokio::test]
async fn plugin_que_cai_e_reiniciado_com_backoff_e_volta_a_responder() {
    let cenario = Cenario::novo("cai");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let supervisor =
        Supervisor::iniciar(cenario.opcoes("cai_no_primeiro_comando"), receptor.clone())
            .await
            .expect("handshake");

    let erro = enviar_texto(&supervisor, "derruba")
        .await
        .expect_err("o plugin sai antes de responder");
    assert!(erro.eh_transitorio(), "erro: {erro}");

    assert!(
        esperar(
            || supervisor.contadores().reinicios >= 1,
            Duration::from_secs(3)
        )
        .await,
        "não houve reinício: {:?}",
        supervisor.contadores()
    );
    assert!(supervisor.contadores().falhas >= 1);
    supervisor
        .aguardar_pronto(Duration::from_secs(2))
        .await
        .expect("pronto depois do reinício");
    enviar_texto(&supervisor, "depois da queda")
        .await
        .expect("voltou a responder");
    assert_eq!(supervisor.contadores().handshakes, 2);
    supervisor.encerrar().await;
}

#[tokio::test]
async fn lixo_e_ignorado_e_contado_sem_afetar_outra_integracao() {
    let cenario_a = Cenario::novo("lixo");
    let cenario_b = Cenario::novo("limpo");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let com_lixo = Supervisor::iniciar(cenario_a.opcoes("lixo"), receptor.clone())
        .await
        .expect("handshake do plugin com lixo");
    let limpo = Supervisor::iniciar(cenario_b.opcoes("normal"), receptor.clone())
        .await
        .expect("handshake do plugin normal");

    // A linha inválida e a linha gigante (2 MiB, acima do teto de 1 MiB) são descartadas e contadas.
    assert!(
        esperar(|| com_lixo.contadores().lixo >= 2, Duration::from_secs(2)).await,
        "lixo não contado: {:?}",
        com_lixo.contadores()
    );
    assert_eq!(limpo.contadores().lixo, 0);

    // Nenhuma das duas integrações parou por causa do lixo da outra.
    enviar_texto(&com_lixo, "ainda funciona")
        .await
        .expect("a integração com lixo continua respondendo");
    enviar_texto(&limpo, "também funciona")
        .await
        .expect("a outra integração não foi afetada");
    assert_eq!(com_lixo.estado(), EstadoIntegracao::Pronto);
    assert_eq!(limpo.estado(), EstadoIntegracao::Pronto);

    com_lixo.encerrar().await;
    limpo.encerrar().await;
}

#[tokio::test]
async fn segredo_vai_so_no_handshake_e_nao_vaza() {
    const SEGREDO: &str = "sk-fixtura-nao-vaza-42";
    let cenario = Cenario::novo("segredo");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let opcoes = cenario
        .opcoes("normal")
        .com_config(json!({
            "modo": "normal",
            "dump_dir": cenario.dump().display().to_string(),
            "emitir_evento": true,
        }))
        .com_segredo("token", SEGREDO)
        .com_ambiente("POPH_TESTE_AMBIENTE", "visivel");

    // Nem o Debug das opções mostra o valor.
    let depuracao = format!("{opcoes:?} {opcoes:#?}");
    assert!(!depuracao.contains(SEGREDO), "Debug vazou: {depuracao}");

    let supervisor = Supervisor::iniciar(opcoes, receptor.clone())
        .await
        .expect("handshake");

    // Chegou ao plugin pelo handshake (nome + tamanho, nunca o valor em disco).
    let segredos = cenario.ler("segredos.txt");
    assert_eq!(segredos.trim(), format!("token={}", SEGREDO.len()));

    // Não foi por linha de comando nem por ambiente.
    let argumentos = cenario.ler("args.txt");
    assert!(!argumentos.contains(SEGREDO), "argv vazou: {argumentos}");
    let ambiente = cenario.ler("env.txt");
    assert!(!ambiente.contains(SEGREDO), "ambiente vazou");
    // O que é explícito chega; o ambiente do Harness não é herdado.
    assert!(
        ambiente.contains("POPH_TESTE_AMBIENTE=visivel"),
        "a variável explícita não chegou: {ambiente}"
    );
    assert!(
        ambiente.contains("PATH="),
        "o mínimo (PATH) tem de ir: {ambiente}"
    );
    for (nome, _) in std::env::vars() {
        if matches!(
            nome.as_str(),
            "PATH" | "HOME" | "LANG" | "LC_ALL" | "TZ" | "TMPDIR" | "POPH_TESTE_AMBIENTE"
        ) {
            continue;
        }
        assert!(
            !ambiente.contains(&format!("{nome}=")),
            "o plugin herdou `{nome}` do ambiente do Harness"
        );
    }

    // Não aparece nos eventos entregues.
    let eventos = coletor.esperar(1, Duration::from_secs(2)).await;
    assert!(!eventos.is_empty(), "nenhum evento chegou");
    for evento in &eventos {
        let json = match evento {
            EventoIntegracao::Entrada(entrada) => serde_json::to_string(entrada),
            EventoIntegracao::Estado(estado) => serde_json::to_string(estado),
        }
        .expect("evento serializável");
        assert!(!json.contains(SEGREDO), "evento vazou: {json}");
        assert!(!format!("{evento:?}").contains(SEGREDO));
    }

    // Não aparece em erro, nem numa recusa do plugin, nem no log.
    let texto_grande = "x".repeat(2 * 1024 * 1024);
    let erro = enviar_texto(&supervisor, &texto_grande)
        .await
        .expect_err("mensagem acima do teto");
    assert!(!erro.to_string().contains(SEGREDO));
    let erro_do_protocolo = ErroIntegracao::Recusado {
        operacao: "enviar_texto".to_owned(),
        codigo: "401".to_owned(),
        mensagem: "sessão caiu".to_owned(),
        transitoria: false,
    };
    assert!(!format!("{erro_do_protocolo:?}").contains(SEGREDO));
    assert!(
        !log_atual().contains(SEGREDO),
        "o segredo apareceu no log:\n{}",
        log_atual()
    );

    supervisor.encerrar().await;
}

#[tokio::test]
async fn mensagem_acima_do_teto_e_recusada_sem_derrubar_o_plugin() {
    let cenario = Cenario::novo("teto");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let limites = LimitesSupervisor {
        max_mensagem_bytes: 1024,
        ..limites_rapidos()
    };
    let opcoes = cenario.opcoes("normal").com_limites(limites);
    let supervisor = Supervisor::iniciar(opcoes, receptor.clone())
        .await
        .expect("handshake");

    let erro = enviar_texto(&supervisor, &"y".repeat(4096))
        .await
        .expect_err("mensagem acima do teto");
    assert!(
        matches!(
            erro,
            ErroIntegracao::MensagemGrandeDemais { teto: 1024, .. }
        ),
        "erro: {erro}"
    );
    // A recusa é do pedido, não do plugin: ele continua pronto e atende o próximo.
    assert_eq!(supervisor.estado(), EstadoIntegracao::Pronto);
    assert_eq!(supervisor.contadores().falhas, 0);
    enviar_texto(&supervisor, "curto")
        .await
        .expect("comando curto continua passando");
    supervisor.encerrar().await;
}

#[tokio::test]
async fn versao_de_protocolo_diferente_e_recusada() {
    let cenario = Cenario::novo("protocolo");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let erro = Supervisor::iniciar(cenario.opcoes("protocolo_errado"), receptor.clone())
        .await
        .expect_err("versão diferente tinha de falhar");
    match erro {
        ErroIntegracao::ProtocoloIncompativel { esperado, recebido } => {
            assert_eq!(esperado, PROTOCOLO_VERSAO);
            assert_eq!(recebido, 999);
        }
        outro => panic!("erro inesperado: {outro}"),
    }
}

#[tokio::test]
async fn manifesto_invalido_e_recusado_no_handshake() {
    let cenario = Cenario::novo("manifesto");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let erro = Supervisor::iniciar(cenario.opcoes("manifesto_invalido"), receptor.clone())
        .await
        .expect_err("manifesto inválido tinha de falhar");
    assert!(
        matches!(erro, ErroIntegracao::HandshakeRecusado { .. }),
        "erro: {erro}"
    );
}

#[tokio::test]
async fn depois_de_encerrar_nao_ha_mais_processo_nem_comando() {
    let cenario = Cenario::novo("encerrar");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let supervisor = Supervisor::iniciar(cenario.opcoes("normal"), receptor.clone())
        .await
        .expect("handshake");
    supervisor.encerrar().await;
    assert_eq!(supervisor.estado(), EstadoIntegracao::Encerrada);
    let erro = enviar_texto(&supervisor, "depois de encerrar")
        .await
        .expect_err("não há mais processo");
    assert!(matches!(erro, ErroIntegracao::Encerrada), "{erro}");
}

#[tokio::test]
async fn integracao_marcada_como_caida_para_de_reiniciar() {
    let cenario = Cenario::novo("caida");
    let coletor = Arc::new(ColetorEventos::novo());
    let receptor: Arc<dyn Receptor> = coletor.clone();
    let supervisor = Supervisor::iniciar(cenario.opcoes("cai_em_todo_comando"), receptor.clone())
        .await
        .expect("handshake");

    // Cada comando derruba o plugin; com teto de 3 falhas seguidas ele para de voltar.
    for _ in 0..8 {
        if supervisor.estado().esta_encerrada() {
            break;
        }
        // Espera o processo estar de pé: mandar durante o reinício só devolveria "indisponível".
        let _ = supervisor.aguardar_pronto(Duration::from_secs(2)).await;
        if supervisor.estado().esta_encerrada() {
            break;
        }
        let _ = enviar_texto(&supervisor, "derruba de novo").await;
        let _ = esperar(
            || !supervisor.estado().esta_pronta(),
            Duration::from_millis(500),
        )
        .await;
    }
    assert!(
        esperar(
            || supervisor.estado().esta_encerrada(),
            Duration::from_secs(3)
        )
        .await,
        "não foi marcada como caída: {:?}",
        supervisor.estado()
    );
    assert!(matches!(
        supervisor.estado(),
        EstadoIntegracao::Caida { .. }
    ));
    let erro = enviar_texto(&supervisor, "não volta mais")
        .await
        .expect_err("caída não envia");
    assert!(matches!(erro, ErroIntegracao::Caida { .. }), "{erro}");
}
