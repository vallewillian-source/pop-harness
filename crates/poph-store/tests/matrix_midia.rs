//! Testes da tabela `matrix_midia` (MX-46): o que a rotina de expiração lê e grava.
//!
//! Banco de verdade em pasta temporária e relógio controlado. O que se prova:
//!
//! - a migração nova entra num banco vazio e num banco que já existia na versão anterior;
//! - gravar a mesma mídia duas vezes não duplica (a reentrega do Synapse é o caminho normal);
//! - a fila devolve só o que já venceu e só o que ainda tem etapa pendente;
//! - a espera da falha tira o item da fila até a hora marcada — nada de laço apertado;
//! - cada etapa tem carimbo próprio: redigir não apaga, apagar não redige, e avançar numa delas
//!   zera a espera da outra;
//! - apagar a org leva as linhas do módulo junto (cascade);
//! - nenhuma coluna aceita texto de cliente fora do nome do arquivo, e o código do erro cabe na
//!   coluna.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::migrations::migrate;
use poph_store::{
    CategoriaMidia, Conversa, HARNESS_MIGRATIONS, MidiaDaSala, NewAgent, NewConversation, NewOrg,
    NovaMidia, Org, Store, StoreError, file_version, harness_target_version,
};
use poph_types::{AttachmentId, DomainError};

/// Relógio controlado pelo teste.
#[derive(Clone)]
struct Relogio(Arc<AtomicI64>);

impl Relogio {
    fn novo(inicial: i64) -> Self {
        Self(Arc::new(AtomicI64::new(inicial)))
    }

    fn definir(&self, valor: i64) {
        self.0.store(valor, Ordering::SeqCst);
    }
}

/// Banco de verdade numa pasta temporária, apagado no fim.
struct Cenario {
    pasta: PathBuf,
    relogio: Relogio,
    store: Option<Arc<Store>>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        Self::com_versao(rotulo, None)
    }

    /// `ate = Some(n)` cria o banco com as migrações 1..=n (simula instalação antiga).
    fn com_versao(rotulo: &str, ate: Option<usize>) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-matrix-midia-{rotulo}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let harness = pasta.join("harness.db");
        let rag = pasta.join("rag.db");
        if let Some(ate) = ate {
            let conn = rusqlite::Connection::open(&harness).expect("abre o banco antigo");
            migrate(&conn, "harness.db", &harness, &HARNESS_MIGRATIONS[..ate], 1)
                .expect("migra até a versão antiga");
        }
        let relogio = Relogio::novo(1_000);
        let agora = Arc::clone(&relogio.0);
        let store = Store::open_at(&harness, &rag)
            .expect("abre o banco")
            .with_clock(Arc::new(move || agora.load(Ordering::SeqCst)));
        Self {
            pasta,
            relogio,
            store: Some(Arc::new(store)),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_deref().expect("store viva")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn criar_org(store: &Store, slug: &str) -> Org {
    store
        .orgs()
        .criar(NewOrg {
            name: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

fn criar_conversa(store: &Store, org: &Org) -> Conversa {
    let agente = store
        .agents()
        .criar(
            org.id,
            NewAgent {
                name: "Agente".to_owned(),
                slug: "agente".to_owned(),
                ..NewAgent::default()
            },
        )
        .expect("cria o agente");
    store
        .conversas()
        .criar(
            org.id,
            NewConversation {
                agent_id: agente.id,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some("5511999990000".to_owned()),
                target_department: None,
            },
        )
        .expect("cria a conversa")
}

/// Uma mídia publicada, com o carimbo que o teste escolher.
fn midia(conversa: &Conversa, evento: &str, enviada_em: i64) -> NovaMidia {
    NovaMidia {
        id: AttachmentId::generate(),
        conversa: conversa.id,
        agente: "agt_teste".to_owned(),
        sala: "!sala:matrix-dev.local".to_owned(),
        evento: evento.to_owned(),
        mxc: format!("mxc://matrix-dev.local/{evento}"),
        nome: "laudo.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
        categoria: CategoriaMidia::Documento,
        tamanho_bytes: 2_048,
        enviada_em,
    }
}

fn ida(midias: &[MidiaDaSala]) -> Vec<String> {
    midias.iter().map(|m| m.id.to_string()).collect()
}

// ───────────────────────────── migração ─────────────────────────────

#[test]
fn migracao_nova_entra_num_banco_vazio_e_num_banco_antigo() {
    let novo = Cenario::novo("migracao-nova");
    assert_eq!(
        file_version(&novo.pasta.join("harness.db")).expect("versão"),
        Some(harness_target_version())
    );

    // Banco parado na versão anterior (sem `matrix_midia`): a 0016 entra na abertura.
    let antigo = Cenario::com_versao("migracao-antiga", Some(HARNESS_MIGRATIONS.len() - 1));
    let org = criar_org(antigo.store(), "antiga");
    let conversa = criar_conversa(antigo.store(), &org);
    let uma = midia(&conversa, "$e1:matrix-dev.local", 5_000);
    assert!(
        antigo
            .store()
            .matrix_midia()
            .registrar(org.id, &uma)
            .expect("grava depois de migrate")
    );
}

// ───────────────────────────── gravação ─────────────────────────────

#[test]
fn registrar_a_mesma_midia_duas_vezes_nao_duplica_nem_move_o_prazo() {
    let cenario = Cenario::novo("idempotente");
    let store = cenario.store();
    let org = criar_org(store, "a");
    let conversa = criar_conversa(store, &org);

    let primeira = midia(&conversa, "$e1:matrix-dev.local", 5_000);
    assert!(
        store
            .matrix_midia()
            .registrar(org.id, &primeira)
            .expect("grava")
    );

    // Reentrega do Synapse: mesmo anexo, mesmo evento, carimbo diferente.
    cenario.relogio.definir(9_000);
    let mut segunda = primeira.clone();
    segunda.enviada_em = 9_000;
    assert!(
        !store
            .matrix_midia()
            .registrar(org.id, &segunda)
            .expect("regrava")
    );

    let pendentes = store
        .matrix_midia()
        .pendentes(org.id, 10_000, 10_000, 50)
        .expect("lê a fila");
    assert_eq!(pendentes.len(), 1, "uma linha só: {pendentes:?}");
    assert_eq!(
        pendentes[0].enviada_em, 5_000,
        "o prazo conta da primeira publicação"
    );
}

#[test]
fn recusa_midia_sem_sala_ou_sem_agente() {
    let cenario = Cenario::novo("validacao");
    let store = cenario.store();
    let org = criar_org(store, "a");
    let conversa = criar_conversa(store, &org);
    let completa = midia(&conversa, "$e1:matrix-dev.local", 5_000);

    for (rotulo, ajuste) in [
        (
            "sala",
            Box::new(|m: &mut NovaMidia| m.sala.clear()) as Box<dyn Fn(&mut NovaMidia)>,
        ),
        (
            "evento",
            Box::new(|m: &mut NovaMidia| m.evento = "  ".to_owned()),
        ),
        ("agente", Box::new(|m: &mut NovaMidia| m.agente.clear())),
        ("mxc", Box::new(|m: &mut NovaMidia| m.mxc.clear())),
    ] {
        let mut defeituosa = completa.clone();
        ajuste(&mut defeituosa);
        let erro = store
            .matrix_midia()
            .registrar(org.id, &defeituosa)
            .expect_err("mídia incompleta deve ser recusada");
        match erro {
            StoreError::Domain(DomainError::Validation { field: campo, .. }) => {
                assert_eq!(campo, rotulo)
            }
            outro => panic!("esperado erro de validação em {rotulo}, veio {outro:?}"),
        }
    }
}

// ───────────────────────────── a fila do prazo ─────────────────────────────

#[test]
fn a_fila_so_devolve_o_que_venceu_e_o_que_ainda_tem_etapa_pendente() {
    let cenario = Cenario::novo("prazo");
    let store = cenario.store();
    let org = criar_org(store, "a");
    let conversa = criar_conversa(store, &org);

    let antiga = midia(&conversa, "$velha:matrix-dev.local", 1_000);
    let nova = midia(&conversa, "$nova:matrix-dev.local", 9_000);
    for uma in [&antiga, &nova] {
        store
            .matrix_midia()
            .registrar(org.id, uma)
            .expect("grava a mídia");
    }

    // Corte em 5.000: só a antiga venceu.
    let vencidas = store
        .matrix_midia()
        .pendentes(org.id, 5_000, 10_000, 50)
        .expect("lê a fila");
    assert_eq!(ida(&vencidas), vec![antiga.id.to_string()]);

    // Concluída nas duas etapas, sai da fila mesmo vencida.
    store
        .matrix_midia()
        .marcar_redigida(org.id, antiga.id, 10_000)
        .expect("redigida");
    assert_eq!(
        store
            .matrix_midia()
            .pendentes(org.id, 5_000, 10_000, 50)
            .expect("lê a fila")
            .len(),
        1,
        "ainda falta apagar o arquivo"
    );
    store
        .matrix_midia()
        .marcar_apagada(org.id, antiga.id, 10_000)
        .expect("apagada");
    assert!(
        store
            .matrix_midia()
            .pendentes(org.id, 5_000, 10_000, 50)
            .expect("lê a fila")
            .is_empty(),
        "mídia completa não volta"
    );
    assert_eq!(
        store
            .matrix_midia()
            .pendentes_da_org(org.id)
            .expect("conta"),
        1,
        "só a nova segue pendente"
    );
}

#[test]
fn org_nao_ve_a_midia_de_outra_org() {
    let cenario = Cenario::novo("isolamento");
    let store = cenario.store();
    let minha = criar_org(store, "minha");
    let alheia = criar_org(store, "alheia");
    let conversa_minha = criar_conversa(store, &minha);
    let conversa_alheia = criar_conversa(store, &alheia);

    let da_minha = midia(&conversa_minha, "$a:matrix-dev.local", 1_000);
    let da_alheia = midia(&conversa_alheia, "$b:matrix-dev.local", 1_000);
    store
        .matrix_midia()
        .registrar(minha.id, &da_minha)
        .expect("grava a minha");
    store
        .matrix_midia()
        .registrar(alheia.id, &da_alheia)
        .expect("grava a alheia");

    let vencidas = store
        .matrix_midia()
        .pendentes(minha.id, 5_000, 10_000, 50)
        .expect("lê a fila");
    assert_eq!(ida(&vencidas), vec![da_minha.id.to_string()]);

    // Marcar na org errada não toca a linha da outra.
    store
        .matrix_midia()
        .marcar_apagada(alheia.id, da_minha.id, 10_000)
        .expect("marca na org errada");
    let ainda_pendente = store
        .matrix_midia()
        .pendentes(minha.id, 5_000, 10_000, 50)
        .expect("lê a fila");
    assert!(
        ainda_pendente[0].falta_apagar(),
        "a org errada não podia mexer nesta linha"
    );
}

// ───────────────────────────── espera da falha ─────────────────────────────

#[test]
fn a_espera_da_falha_tira_o_item_da_fila_ate_a_hora_marcada() {
    let cenario = Cenario::novo("espera");
    let store = cenario.store();
    let org = criar_org(store, "a");
    let conversa = criar_conversa(store, &org);
    let uma = midia(&conversa, "$e1:matrix-dev.local", 1_000);
    store
        .matrix_midia()
        .registrar(org.id, &uma)
        .expect("grava a mídia");

    // Primeira falha: só volta daqui a 60 s e o código é curto.
    let tentativas = store
        .matrix_midia()
        .adiar(org.id, uma.id, 61_000, "M_UNKNOWN", 1_000)
        .expect("adia");
    assert_eq!(tentativas, 1);

    assert!(
        store
            .matrix_midia()
            .pendentes(org.id, 5_000, 60_000, 50)
            .expect("lê a fila")
            .is_empty(),
        "antes da hora marcada o item não volta"
    );
    let na_hora = store
        .matrix_midia()
        .pendentes(org.id, 5_000, 61_000, 50)
        .expect("lê a fila");
    assert_eq!(na_hora.len(), 1);
    assert_eq!(na_hora[0].tentativas, 1);
    assert_eq!(na_hora[0].ultimo_erro.as_deref(), Some("M_UNKNOWN"));

    // A etapa que anda zera a espera das duas (o item fez progresso).
    store
        .matrix_midia()
        .marcar_redigida(org.id, uma.id, 62_000)
        .expect("redigida");
    let depois = store
        .matrix_midia()
        .pendentes(org.id, 5_000, 62_000, 50)
        .expect("lê a fila");
    assert_eq!(depois[0].tentativas, 0);
    assert_eq!(depois[0].proxima_tentativa_em, 0);
    assert_eq!(depois[0].ultimo_erro, None);
    assert!(depois[0].falta_apagar());
}

#[test]
fn o_codigo_do_erro_cabe_na_coluna() {
    let cenario = Cenario::novo("codigo");
    let store = cenario.store();
    let org = criar_org(store, "a");
    let conversa = criar_conversa(store, &org);
    let uma = midia(&conversa, "$e1:matrix-dev.local", 1_000);
    store
        .matrix_midia()
        .registrar(org.id, &uma)
        .expect("grava a mídia");

    let enorme = "X".repeat(200);
    store
        .matrix_midia()
        .adiar(org.id, uma.id, 0, &enorme, 1_000)
        .expect("adia com código enorme");
    let linha = store
        .matrix_midia()
        .pendentes(org.id, 5_000, 1_000, 50)
        .expect("lê a fila")
        .remove(0);
    assert_eq!(
        linha.ultimo_erro.as_deref().map(str::len),
        Some(poph_store::LIMITE_ERRO_MIDIA),
        "o código é recortado no limite da coluna"
    );
}

// ───────────────────────────── cascade e contagem ─────────────────────────────

#[test]
fn apagar_a_org_leva_as_midias_do_modulo_junto() {
    let cenario = Cenario::novo("cascade");
    let store = cenario.store();
    let org = criar_org(store, "a");
    let conversa = criar_conversa(store, &org);
    let uma = midia(&conversa, "$e1:matrix-dev.local", 1_000);
    store
        .matrix_midia()
        .registrar(org.id, &uma)
        .expect("grava a mídia");

    store
        .orgs()
        .excluir_forcado(org.id)
        .expect("apaga a org com os agentes dela");
    assert_eq!(
        store
            .matrix_midia()
            .pendentes(org.id, 5_000, 10_000, 50)
            .expect("lê a fila")
            .len(),
        0,
        "as linhas da org foram junto"
    );
}
