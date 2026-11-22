//! Leituras em lote que a API de conversas usa para o Element (RP-01).
//!
//! Banco de verdade numa pasta temporária. Cobrem: o nome de vários agentes numa consulta só,
//! filtrado por org; a sala `ready` de várias conversas numa consulta só, sem trazer sala ainda
//! em `creating` nem sala de outra org; e que cada chamada conta como uma leitura (é o que o teste
//! de API usa para provar que a lista não faz uma consulta por linha).

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::{Agente, Conversa, NovaConversa, NovaOrg, NovoAgente, Org, Store};
use poph_types::ConversationId;

/// Banco de verdade numa pasta temporária, apagada no fim.
struct Cenario {
    pasta: PathBuf,
    store: Arc<Store>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta = std::env::temp_dir().join(format!(
            "poph-dados-element-{rotulo}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let relogio = Arc::new(AtomicI64::new(1_000));
        let agora = Arc::clone(&relogio);
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco")
            .com_relogio(Arc::new(move || agora.fetch_add(10, Ordering::SeqCst)));
        Self {
            pasta,
            store: Arc::new(store),
        }
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn criar_org(store: &Store, slug: &str) -> Org {
    store
        .orgaos()
        .criar(NovaOrg {
            nome: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
}

fn criar_agente(store: &Store, org: &Org, slug: &str) -> Agente {
    store
        .agentes()
        .criar(
            org.id,
            NovoAgente {
                nome: format!("Agente {slug}"),
                slug: slug.to_owned(),
                ..NovoAgente::default()
            },
        )
        .expect("cria o agente")
}

fn criar_conversa(store: &Store, org: &Org, agente: &Agente) -> Conversa {
    store
        .conversas()
        .criar(
            org.id,
            NovaConversa {
                agent_id: agente.id,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: Some("teste".to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("cria a conversa")
}

#[test]
fn nomes_de_agentes_saem_numa_consulta_so_e_filtrados_por_org() {
    let cenario = Cenario::novo("nomes");
    let store = &cenario.store;
    let a = criar_org(store, "a");
    let b = criar_org(store, "b");
    let a1 = criar_agente(store, &a, "a1");
    let a2 = criar_agente(store, &a, "a2");
    let b1 = criar_agente(store, &b, "b1");

    let antes = store.leituras_feitas();
    let nomes = store
        .agentes()
        .nomes(a.id, &[a1.id, a2.id, b1.id])
        .expect("nomes");
    assert_eq!(
        store.leituras_feitas() - antes,
        1,
        "os nomes de vários agentes saem numa leitura só"
    );
    assert_eq!(nomes.get(&a1.id).map(String::as_str), Some("Agente a1"));
    assert_eq!(nomes.get(&a2.id).map(String::as_str), Some("Agente a2"));
    assert!(
        !nomes.contains_key(&b1.id),
        "agente de outra org não entra no resultado"
    );

    let vazio = store.agentes().nomes(a.id, &[]).expect("lista vazia");
    assert!(vazio.is_empty());
}

#[test]
fn salas_prontas_saem_numa_consulta_so_so_com_state_ready_e_por_org() {
    let cenario = Cenario::novo("salas");
    let store = &cenario.store;
    let a = criar_org(store, "a");
    let b = criar_org(store, "b");
    let agente_a = criar_agente(store, &a, "a");
    let agente_b = criar_agente(store, &b, "b");

    let pronta = criar_conversa(store, &a, &agente_a);
    let criando = criar_conversa(store, &a, &agente_a);
    let sem_sala = criar_conversa(store, &a, &agente_a);
    let de_outra_org = criar_conversa(store, &b, &agente_b);

    store
        .matrix()
        .garantir_sala(a.id, pronta.id, "!pronta:teste", None)
        .expect("sala");
    store
        .matrix()
        .marcar_sala_pronta(a.id, pronta.id)
        .expect("pronta");
    store
        .matrix()
        .garantir_sala(a.id, criando.id, "!criando:teste", None)
        .expect("sala em criação");
    store
        .matrix()
        .garantir_sala(b.id, de_outra_org.id, "!outra:teste", None)
        .expect("sala da outra org");
    store
        .matrix()
        .marcar_sala_pronta(b.id, de_outra_org.id)
        .expect("pronta da outra org");

    let pedidas: Vec<ConversationId> = vec![pronta.id, criando.id, sem_sala.id, de_outra_org.id];
    let antes = store.leituras_feitas();
    let salas = store.matrix().salas_prontas(a.id, &pedidas).expect("salas");
    assert_eq!(
        store.leituras_feitas() - antes,
        1,
        "as salas de várias conversas saem numa leitura só"
    );
    assert_eq!(
        salas.get(&pronta.id).map(String::as_str),
        Some("!pronta:teste"),
        "sala ready aparece"
    );
    assert_eq!(salas.len(), 1, "só a sala pronta da própria org aparece");
    assert!(
        !salas.contains_key(&criando.id),
        "sala em creating não aparece"
    );
    assert!(
        !salas.contains_key(&sem_sala.id),
        "conversa sem sala não aparece"
    );
    assert!(
        !salas.contains_key(&de_outra_org.id),
        "sala de conversa de outra org não aparece, mesmo com state ready"
    );

    let vazio = store
        .matrix()
        .salas_prontas(a.id, &[])
        .expect("lista vazia");
    assert!(vazio.is_empty());
}
