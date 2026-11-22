//! Testes do laço de espelhamento (MX-14).
//!
//! Banco de verdade (`Store` numa pasta temporária, relógio controlado) e Synapse **falso** em
//! memória: o duplo deduplica por `txnId` como o Synapse faz (MX-02), então "repetir após queda"
//! é medido de forma honesta — a entrega repetida não cria evento a mais.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_matrix::erro::ErroChamada;
use poph_matrix::formato::Envio;
use poph_matrix::salas::{ConversaParaSala, ErroSala};
use poph_server::matrix_espelho::{
    DestinoMatrix, EspelhoMatrix, MAX_PAGINAS_POR_CICLO, PAGINA, TENTATIVAS_DEFINITIVAS,
    TETO_ATRASO,
};
use poph_store::{Direcao, NovaConversa, NovaMensagem, NovaOrg, NovoAgente, Origem, Papel, Store};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId};
use rusqlite::Connection;

// ───────────────────────────── cenário ─────────────────────────────

/// Relógio que anda 1 ms a cada leitura: ordem de criação sempre estrita.
struct Cenario {
    pasta: PathBuf,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-espelho-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let relogio = Arc::new(AtomicI64::new(1_000));
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .com_relogio(Arc::new(move || relogio.fetch_add(1, Ordering::SeqCst)));
        Self {
            pasta,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> Arc<Store> {
        Arc::clone(self.store.as_ref().expect("store viva"))
    }

    fn conexao(&self) -> Connection {
        Connection::open(self.store().caminho_harness()).expect("abre o banco para conferir")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn config() -> MatrixConfig {
    MatrixConfig {
        habilitado: true,
        homeserver_url: "http://127.0.0.1:1".to_owned(),
        server_name: "matrix-dev.local".to_owned(),
        prefixo: "ph".to_owned(),
        concorrencia: 8,
        intervalo_ms: 500,
        ..MatrixConfig::default()
    }
}

fn criar_org(store: &Store, slug: &str) -> OrgId {
    store
        .orgaos()
        .criar(NovaOrg {
            nome: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id
}

fn criar_agente(store: &Store, org: OrgId) -> AgentId {
    store
        .agentes()
        .criar(
            org,
            NovoAgente {
                nome: "Bia".to_owned(),
                slug: "bia".to_owned(),
                ..NovoAgente::default()
            },
        )
        .expect("cria o agente")
        .id
}

fn criar_conversa(store: &Store, org: OrgId, agente: AgentId, peer: &str) -> ConversationId {
    store
        .conversas()
        .criar(
            org,
            NovaConversa {
                agent_id: agente,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some(peer.to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("cria a conversa")
        .id
}

/// Mensagem do cliente (entrada, papel `user`).
fn msg_cliente(store: &Store, org: OrgId, conversa: ConversationId, texto: &str) -> String {
    gravar(
        store,
        org,
        conversa,
        Direcao::Entrada,
        Papel::Usuario,
        Origem::Agente,
        texto,
    )
}

/// Resposta do agente (saída, papel `assistant`).
fn msg_agente(store: &Store, org: OrgId, conversa: ConversationId, texto: &str) -> String {
    gravar(
        store,
        org,
        conversa,
        Direcao::Saida,
        Papel::Assistente,
        Origem::Agente,
        texto,
    )
}

fn gravar(
    store: &Store,
    org: OrgId,
    conversa: ConversationId,
    direcao: Direcao,
    papel: Papel,
    origem: Origem,
    texto: &str,
) -> String {
    store
        .mensagens()
        .gravar(
            org,
            NovaMensagem {
                conversation_id: conversa,
                direcao,
                papel,
                origem,
                autor_nome: None,
                texto: texto.to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
            },
        )
        .expect("grava a mensagem")
        .id
        .to_string()
}

/// Id do `txnId` que o espelho usa para uma mensagem (`poph-msg-` + id).
fn txn_de(id: &str) -> String {
    format!("poph-msg-{id}")
}

// ───────────────────────────── Synapse falso ─────────────────────────────

#[derive(Clone, Copy)]
enum Modo {
    /// Aceita tudo.
    Normal,
    /// Toda chamada falha de forma transitória (Synapse fora do ar).
    Fora,
    /// Corpo que contém o texto é recusado com 403 (definitivo); o resto é aceito.
    Venenoso(&'static str),
    /// A n-ésima chamada de envio falha de forma transitória, uma vez só.
    FalhaNaChamada(usize),
}

/// Um evento que o Synapse falso criou: (sala, txnId, corpo).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Criado {
    sala: String,
    txn: String,
    corpo: String,
}

struct Falso {
    modo: Mutex<Modo>,
    criados: Mutex<Vec<Criado>>,
    /// Quantas vezes cada `txnId` foi enviado (inclusive as recusadas e as repetidas).
    tentativas: Mutex<HashMap<String, u32>>,
    chamadas_sala: AtomicUsize,
    chamadas_envio: AtomicUsize,
}

impl Falso {
    fn novo(modo: Modo) -> Arc<Self> {
        Arc::new(Self {
            modo: Mutex::new(modo),
            criados: Mutex::new(Vec::new()),
            tentativas: Mutex::new(HashMap::new()),
            chamadas_sala: AtomicUsize::new(0),
            chamadas_envio: AtomicUsize::new(0),
        })
    }

    fn definir_modo(&self, modo: Modo) {
        *self.modo.lock().unwrap() = modo;
    }

    fn criados(&self) -> Vec<Criado> {
        self.criados.lock().unwrap().clone()
    }

    fn txns_da_sala(&self, sala: &str) -> Vec<String> {
        self.criados()
            .into_iter()
            .filter(|criado| criado.sala == sala)
            .map(|criado| criado.txn)
            .collect()
    }

    fn tentativas_de(&self, txn: &str) -> u32 {
        *self.tentativas.lock().unwrap().get(txn).unwrap_or(&0)
    }
}

fn transitorio() -> ErroChamada {
    ErroChamada::Resposta {
        operacao: "send",
        status: 503,
        errcode: Some("M_UNKNOWN".to_owned()),
        mensagem: "indisponível".to_owned(),
        retry_after_ms: None,
    }
}

fn recusado() -> ErroChamada {
    ErroChamada::Resposta {
        operacao: "send",
        status: 403,
        errcode: Some("M_FORBIDDEN".to_owned()),
        mensagem: "recusado".to_owned(),
        retry_after_ms: None,
    }
}

/// O duplo compartilhado com o teste: o espelho fica com uma cópia do `Arc`, o teste com outra.
struct Sinal(Arc<Falso>);

impl DestinoMatrix for Sinal {
    fn garantir_sala(
        &self,
        dados: &ConversaParaSala,
    ) -> impl std::future::Future<Output = Result<String, ErroSala>> + Send {
        let sala = format!("!{}:matrix-dev.local", dados.conversa);
        self.0.chamadas_sala.fetch_add(1, Ordering::SeqCst);
        async move { Ok(sala) }
    }

    fn enviar(
        &self,
        sala: &str,
        envio: &Envio,
    ) -> impl std::future::Future<Output = Result<(), ErroChamada>> + Send {
        let falso = &self.0;
        let n = falso.chamadas_envio.fetch_add(1, Ordering::SeqCst) + 1;
        let modo = *falso.modo.lock().unwrap();
        let sala = sala.to_owned();
        let txn = envio.txn_id.clone();
        let corpo = envio.corpo.clone();
        *falso
            .tentativas
            .lock()
            .unwrap()
            .entry(txn.clone())
            .or_insert(0) += 1;
        let resultado = match modo {
            Modo::Fora => Err(transitorio()),
            Modo::Venenoso(texto) if corpo.contains(texto) => Err(recusado()),
            Modo::FalhaNaChamada(k) if n == k => {
                // Uma vez só: depois da queda, o Synapse volta a aceitar.
                *falso.modo.lock().unwrap() = Modo::Normal;
                Err(transitorio())
            }
            _ => Ok(()),
        };
        let mut criados = falso.criados.lock().unwrap();
        if resultado.is_ok() && !criados.iter().any(|c| c.sala == sala && c.txn == txn) {
            // Como o Synapse: o mesmo `txnId` na mesma sala não cria outro evento.
            criados.push(Criado { sala, txn, corpo });
        }
        drop(criados);
        async move { resultado }
    }
}

fn espelho(store: Arc<Store>, destino: Arc<Falso>) -> EspelhoMatrix<Sinal> {
    EspelhoMatrix::novo(store, Sinal(destino), &config()).expect("monta o espelho")
}

fn sala_de(conversa: ConversationId) -> String {
    format!("!{conversa}:matrix-dev.local")
}

// ───────────────────────────── testes ─────────────────────────────

#[tokio::test]
async fn ordem_dentro_da_sala_e_paralelo_entre_salas() {
    let cenario = Cenario::novo("ordem");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let ana = criar_conversa(&store, org, agente, "+5511111");
    let bruno = criar_conversa(&store, org, agente, "+5522222");
    store.matrix().ligar(org).expect("liga");

    let mut esperado_ana = Vec::new();
    let mut esperado_bruno = Vec::new();
    for i in 0..6 {
        if i % 2 == 0 {
            esperado_ana.push(txn_de(&msg_cliente(&store, org, ana, &format!("ana {i}"))));
        } else {
            esperado_bruno.push(txn_de(&msg_cliente(
                &store,
                org,
                bruno,
                &format!("bruno {i}"),
            )));
        }
    }

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.enviadas, 6);
    assert_eq!(destino.txns_da_sala(&sala_de(ana)), esperado_ana);
    assert_eq!(destino.txns_da_sala(&sala_de(bruno)), esperado_bruno);
}

#[tokio::test]
async fn queda_no_meio_nao_pula_nem_duplica() {
    let cenario = Cenario::novo("queda");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let ana = criar_conversa(&store, org, agente, "+5511111");
    let bruno = criar_conversa(&store, org, agente, "+5522222");
    store.matrix().ligar(org).expect("liga");

    let mut todas = Vec::new();
    for i in 0..6 {
        let conversa = if i % 2 == 0 { ana } else { bruno };
        todas.push(txn_de(&msg_cliente(
            &store,
            org,
            conversa,
            &format!("msg {i}"),
        )));
    }

    let destino = Falso::novo(Modo::FalhaNaChamada(3));
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));

    let primeiro = espelho.ciclo().await.expect("ciclo 1");
    assert!(primeiro.pendentes >= 1, "a falha no meio deixa pendência");
    // O cursor não passou da primeira linha não resolvida.
    let cursor = store.matrix().ler_cursores(org).expect("cursor").mensagens;
    assert!(cursor < 6, "cursor não pode andar além da falha: {cursor}");

    espelho.ciclo().await.expect("ciclo 2");
    espelho.ciclo().await.expect("ciclo 3");

    let criados: Vec<String> = destino.criados().into_iter().map(|c| c.txn).collect();
    assert_eq!(criados.len(), 6, "todas entregues uma vez");
    for txn in &todas {
        assert!(criados.contains(txn), "nenhuma linha pulada: {txn}");
    }
    let unicos: std::collections::HashSet<_> = criados.iter().collect();
    assert_eq!(unicos.len(), 6, "nenhum evento duplicado");
    let repetida = todas
        .iter()
        .filter(|txn| destino.tentativas_de(txn) >= 2)
        .count();
    assert!(
        repetida >= 1,
        "a mensagem da queda foi reenviada com o mesmo txnId"
    );
}

#[tokio::test]
async fn mensagem_venenosa_nao_trava_a_fila() {
    let cenario = Cenario::novo("venenosa");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    let antes = txn_de(&msg_cliente(&store, org, conversa, "primeira"));
    let veneno = txn_de(&msg_cliente(&store, org, conversa, "VENENO aqui"));
    let depois = txn_de(&msg_cliente(&store, org, conversa, "terceira"));
    let ultima = txn_de(&msg_cliente(&store, org, conversa, "quarta"));

    let destino = Falso::novo(Modo::Venenoso("VENENO"));
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));

    // Até o limite de tentativas, a venenosa fica pendente e a fila espera.
    for _ in 0..(TENTATIVAS_DEFINITIVAS - 1) {
        espelho.ciclo().await.expect("ciclo");
    }
    assert!(!destino.txns_da_sala(&sala_de(conversa)).contains(&depois));

    // Na tentativa limite, ela é isolada e a fila anda.
    espelho.ciclo().await.expect("ciclo limite");
    espelho.ciclo().await.expect("ciclo seguinte");

    let enviadas = destino.txns_da_sala(&sala_de(conversa));
    assert!(enviadas.contains(&antes));
    assert!(enviadas.contains(&depois));
    assert!(enviadas.contains(&ultima));
    assert!(!enviadas.contains(&veneno));

    let falhas = store.matrix().listar_falhas(org, 10).expect("falhas");
    assert_eq!(falhas.len(), 1, "a venenosa foi isolada uma vez");
    assert_eq!(falhas[0].motivo, "M_FORBIDDEN");
    assert_eq!(falhas[0].tentativas, TENTATIVAS_DEFINITIVAS);
}

#[tokio::test]
async fn synapse_fora_acumula_sem_avancar_e_depois_drena() {
    let cenario = Cenario::novo("fora");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    let mut todas = Vec::new();
    for i in 0..5 {
        todas.push(txn_de(&msg_cliente(
            &store,
            org,
            conversa,
            &format!("m{i}"),
        )));
    }

    let destino = Falso::novo(Modo::Fora);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    for _ in 0..3 {
        let relatorio = espelho.ciclo().await.expect("ciclo");
        assert_eq!(relatorio.enviadas, 0);
        assert!(relatorio.pendentes >= 1);
    }
    assert_eq!(
        store.matrix().ler_cursores(org).expect("cursor").mensagens,
        0
    );
    assert!(
        espelho.atraso_apos(3) > espelho.atraso_apos(1),
        "o atraso cresce"
    );

    destino.definir_modo(Modo::Normal);
    espelho.ciclo().await.expect("ciclo de volta");
    let criados: Vec<String> = destino.criados().into_iter().map(|c| c.txn).collect();
    assert_eq!(criados, todas, "tudo entregue, na ordem, uma vez");
}

#[tokio::test]
async fn org_desligada_nao_gera_chamada_alguma() {
    let cenario = Cenario::novo("desligada");
    let store = cenario.store();
    let ligada = criar_org(&store, "ligada");
    let desligada = criar_org(&store, "desligada");
    let agente_l = criar_agente(&store, ligada);
    let agente_d = criar_agente(&store, desligada);
    let conv_l = criar_conversa(&store, ligada, agente_l, "+5511111");
    let conv_d = criar_conversa(&store, desligada, agente_d, "+5522222");
    store.matrix().ligar(ligada).expect("liga");

    msg_cliente(&store, ligada, conv_l, "da ligada");
    msg_cliente(&store, desligada, conv_d, "da desligada");
    msg_cliente(&store, desligada, conv_d, "de novo na desligada");

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.enviadas, 1);
    assert_eq!(relatorio.de_org_desligada, 2);
    assert_eq!(destino.chamadas_sala.load(Ordering::SeqCst), 1);
    assert!(destino.criados().iter().all(|c| c.sala == sala_de(conv_l)));
}

#[tokio::test]
async fn mensagem_anterior_ao_corte_nao_e_espelhada() {
    let cenario = Cenario::novo("corte");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");

    msg_cliente(&store, org, conversa, "antes de ligar");
    store.matrix().ligar(org).expect("liga");
    let depois = txn_de(&msg_cliente(&store, org, conversa, "depois de ligar"));

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.antes_do_corte, 1);
    assert_eq!(destino.txns_da_sala(&sala_de(conversa)), vec![depois]);
}

#[tokio::test]
async fn resposta_humana_e_ferramenta_nao_viram_evento() {
    let cenario = Cenario::novo("nao-espelha");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    gravar(
        &store,
        org,
        conversa,
        Direcao::Saida,
        Papel::Assistente,
        Origem::Humano,
        "vindo do Element",
    );
    gravar(
        &store,
        org,
        conversa,
        Direcao::Saida,
        Papel::Sistema,
        Origem::Agente,
        "instrução",
    );
    msg_cliente(&store, org, conversa, "   ");
    let normal = txn_de(&msg_agente(&store, org, conversa, "resposta do agente"));

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");

    assert_eq!(relatorio.enviadas, 1);
    assert_eq!(destino.txns_da_sala(&sala_de(conversa)), vec![normal]);
}

#[tokio::test]
async fn rowid_reaproveitado_nao_perde_a_mensagem_nova() {
    let cenario = Cenario::novo("rowid");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    msg_cliente(&store, org, conversa, "um");
    msg_cliente(&store, org, conversa, "dois");
    let ultima = msg_cliente(&store, org, conversa, "tres");

    let destino = Falso::novo(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    espelho.ciclo().await.expect("ciclo 1");
    assert_eq!(destino.criados().len(), 3);

    // Apaga a linha de maior rowid (já espelhada) e grava outra: o SQLite reaproveita o rowid.
    let rowid_antigo: i64 = cenario
        .conexao()
        .query_row(
            "SELECT rowid FROM messages WHERE id = ?1",
            [ultima.as_str()],
            |linha| linha.get(0),
        )
        .expect("rowid");
    cenario
        .conexao()
        .execute("DELETE FROM messages WHERE id = ?1", [ultima.as_str()])
        .expect("apaga");
    let nova = msg_cliente(&store, org, conversa, "quatro, com rowid antigo");
    let rowid_novo: i64 = cenario
        .conexao()
        .query_row(
            "SELECT rowid FROM messages WHERE id = ?1",
            [nova.as_str()],
            |linha| linha.get(0),
        )
        .expect("rowid novo");
    assert_eq!(rowid_novo, rowid_antigo, "o rowid foi reaproveitado");

    espelho.ciclo().await.expect("ciclo 2");
    assert!(
        destino
            .txns_da_sala(&sala_de(conversa))
            .contains(&txn_de(&nova)),
        "a mensagem com rowid reaproveitado sai"
    );
}

#[tokio::test]
async fn cursor_a_frente_do_maior_rowid_volta_ao_ponto_seguro() {
    let cenario = Cenario::novo("recuo");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");
    msg_cliente(&store, org, conversa, "um");
    msg_cliente(&store, org, conversa, "dois");

    // Cursor inventado, à frente de tudo que existe.
    store
        .matrix()
        .avancar_mensagens_com_id(org, 99, "00000000-0000-7000-8000-000000000000")
        .expect("cursor");

    let destino = Falso::novo(Modo::Normal);
    let espelho = espelho(Arc::clone(&store), Arc::clone(&destino));
    let relatorio = espelho.ciclo().await.expect("ciclo");
    assert_eq!(relatorio.recuos, 1);
    assert!(store.matrix().ler_cursores(org).expect("cursor").mensagens <= 2);

    // Mensagem nova depois do recuo é lida normalmente.
    let nova = txn_de(&msg_cliente(&store, org, conversa, "tres"));
    espelho.ciclo().await.expect("ciclo 2");
    assert!(destino.txns_da_sala(&sala_de(conversa)).contains(&nova));
}

#[tokio::test]
async fn memoria_do_ciclo_fica_limitada_a_uma_pagina() {
    let cenario = Cenario::novo("memoria");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");

    let total = PAGINA as usize * 3 + 17;
    for i in 0..total {
        msg_cliente(&store, org, conversa, &format!("m{i}"));
    }

    let destino = Falso::novo(Modo::Normal);
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");
    assert!(
        relatorio.maior_pagina <= PAGINA,
        "nunca mais que uma página em memória"
    );
    assert_eq!(
        relatorio.enviadas as usize,
        total.min(PAGINA as usize * MAX_PAGINAS_POR_CICLO as usize)
    );
}

#[tokio::test]
async fn atraso_cresce_com_falhas_e_para_no_teto() {
    let cenario = Cenario::novo("atraso");
    let espelho = espelho(cenario.store(), Falso::novo(Modo::Normal));
    assert_eq!(espelho.atraso_apos(0), Duration::from_millis(500));
    assert_eq!(espelho.atraso_apos(1), Duration::from_millis(1000));
    assert_eq!(espelho.atraso_apos(2), Duration::from_millis(2000));
    assert_eq!(espelho.atraso_apos(40), TETO_ATRASO);
}

#[tokio::test]
async fn parada_encerra_o_laco() {
    let cenario = Cenario::novo("parada");
    let store = cenario.store();
    let espelho = Arc::new(espelho(Arc::clone(&store), Falso::novo(Modo::Normal)));
    let tarefa = tokio::spawn(Arc::clone(&espelho).executar());
    tokio::time::sleep(Duration::from_millis(50)).await;
    espelho.pedir_parada();
    let resultado = tokio::time::timeout(Duration::from_secs(5), tarefa).await;
    assert!(resultado.is_ok(), "o laço sai depois do pedido de parada");
}

/// Cinquenta mil mensagens pendentes com o Synapse fora do ar: o ciclo lê uma página, para na
/// primeira falha e não carrega a fila inteira. Lento (grava 50 mil linhas): roda com `--ignored`.
#[tokio::test]
#[ignore = "grava 50 mil mensagens; rodar com --ignored"]
async fn memoria_com_50_mil_pendentes() {
    let cenario = Cenario::novo("cinquenta-mil");
    let store = cenario.store();
    let org = criar_org(&store, "a");
    let agente = criar_agente(&store, org);
    let conversa = criar_conversa(&store, org, agente, "+5511111");
    store.matrix().ligar(org).expect("liga");
    let inicio = std::time::Instant::now();
    for i in 0..50_000 {
        msg_cliente(&store, org, conversa, &format!("m{i}"));
    }
    eprintln!("gravação de 50 mil: {:?}", inicio.elapsed());

    let destino = Falso::novo(Modo::Fora);
    let inicio = std::time::Instant::now();
    let relatorio = espelho(Arc::clone(&store), Arc::clone(&destino))
        .ciclo()
        .await
        .expect("ciclo");
    eprintln!(
        "ciclo com 50 mil pendentes: {:?}, {relatorio:?}",
        inicio.elapsed()
    );
    assert_eq!(relatorio.enviadas, 0);
    assert!(relatorio.maior_pagina <= PAGINA);
    assert_eq!(relatorio.paginas, 1, "para na primeira página com falha");
    assert_eq!(
        store.matrix().ler_cursores(org).expect("cursor").mensagens,
        0
    );
}
