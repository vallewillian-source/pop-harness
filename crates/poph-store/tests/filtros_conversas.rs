//! Listagem de conversas com filtros e ordem (RP-02), no banco de verdade numa pasta temporária.
//!
//! O que se prova:
//! - `external_peer` e `status` filtram sozinhos e juntos, sempre dentro da org; conversa de outra
//!   org com o mesmo cliente não aparece;
//! - sem filtro e sem `sort`, a listagem é a de id com paginação por cursor, como antes;
//! - `sort=recent` ordena por `COALESCE(last_message_at, created_at)` decrescente, com desempate
//!   por id decrescente, e respeita o limite;
//! - o plano da consulta com `external_peer` usa o índice `ix_conversations_org_peer_id`
//!   (e não varre a tabela);
//! - o identificador do cliente (`client_identifier` do metadata da sala) é o `external_peer` sem
//!   espaço nas pontas, ou o id da conversa quando ele está vazio.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use poph_store::{
    Agente, Conversa, FiltroConversas, NovaConversa, NovaOrg, NovoAgente, OrdemConversas, Org,
    Pagina, StatusConversa, Store,
};
use poph_types::{ConversationId, OrgId};

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
            "poph-filtros-conversas-{rotulo}-{}-{n}",
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

    fn org(&self, slug: &str) -> Org {
        self.store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agente(&self, org: &Org, slug: &str) -> Agente {
        self.store
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

    fn conversa(&self, org: &Org, agente: &Agente, peer: Option<&str>) -> Conversa {
        self.store
            .conversas()
            .criar(
                org.id,
                NovaConversa {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: peer.map(str::to_owned),
                    departamento_alvo: None,
                },
            )
            .expect("cria a conversa")
    }

    fn listar(&self, org: OrgId, filtro: &FiltroConversas) -> Vec<ConversationId> {
        self.store
            .conversas()
            .listar_filtrado(org, filtro, Pagina::com_limite(200))
            .expect("lista com filtro")
            .into_iter()
            .map(|c| c.id)
            .collect()
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn filtro_peer(peer: &str) -> FiltroConversas {
    FiltroConversas {
        external_peer: Some(peer.to_owned()),
        status: None,
        ..Default::default()
    }
}

#[test]
fn filtro_por_cliente_acha_so_as_conversas_dele_na_org() {
    let c = Cenario::novo("peer");
    let a = c.org("a");
    let b = c.org("b");
    let agente_a = c.agente(&a, "a");
    let agente_b = c.agente(&b, "b");

    let c1 = c.conversa(&a, &agente_a, Some("5511"));
    let c2 = c.conversa(&a, &agente_a, Some("5511"));
    let _c3 = c.conversa(&a, &agente_a, Some("5522"));
    let d1 = c.conversa(&b, &agente_b, Some("5511"));

    let da_a = c.listar(a.id, &filtro_peer("5511"));
    assert_eq!(
        da_a,
        vec![c1.id, c2.id],
        "só as duas conversas do cliente, na org A"
    );

    let da_b = c.listar(b.id, &filtro_peer("5511"));
    assert_eq!(
        da_b,
        vec![d1.id],
        "o cliente de outra org não aparece no filtro desta org"
    );
    assert!(
        c.listar(a.id, &filtro_peer("9999")).is_empty(),
        "cliente sem conversa devolve lista vazia"
    );
}

#[test]
fn filtro_por_situacao_e_os_dois_juntos() {
    let c = Cenario::novo("situacao");
    let org = c.org("a");
    let agente = c.agente(&org, "a");
    let aberta = c.conversa(&org, &agente, Some("5511"));
    let transferida = c.conversa(&org, &agente, Some("5511"));
    let fechada = c.conversa(&org, &agente, Some("5522"));
    let transferida_de_outro = c.conversa(&org, &agente, Some("5522"));

    c.store
        .conversas()
        .transferir(org.id, transferida.id, None, None, "{}".to_owned())
        .expect("transfere");
    c.store
        .conversas()
        .mudar_status(org.id, fechada.id, 1, StatusConversa::Fechada, None)
        .expect("fecha");
    c.store
        .conversas()
        .transferir(org.id, transferida_de_outro.id, None, None, "{}".to_owned())
        .expect("transfere a quarta");

    let so_transferidas = FiltroConversas {
        status: Some(StatusConversa::Transferida),
        external_peer: None,
        ..Default::default()
    };
    assert_eq!(
        c.listar(org.id, &so_transferidas),
        vec![transferida.id, transferida_de_outro.id]
    );

    let so_fechadas = FiltroConversas {
        status: Some(StatusConversa::Fechada),
        external_peer: None,
        ..Default::default()
    };
    assert_eq!(c.listar(org.id, &so_fechadas), vec![fechada.id]);

    let so_abertas = FiltroConversas {
        status: Some(StatusConversa::Aberta),
        external_peer: None,
        ..Default::default()
    };
    assert_eq!(c.listar(org.id, &so_abertas), vec![aberta.id]);

    let cliente_e_transferida = FiltroConversas {
        status: Some(StatusConversa::Transferida),
        external_peer: Some("5511".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        c.listar(org.id, &cliente_e_transferida),
        vec![transferida.id],
        "cliente e situação juntos devolvem só a conversa que atende os dois"
    );
}

#[test]
fn sem_filtro_a_listagem_e_por_id_com_cursor() {
    let c = Cenario::novo("sem-filtro");
    let org = c.org("a");
    let agente = c.agente(&org, "a");
    let ids: Vec<ConversationId> = (0..5)
        .map(|n| {
            c.conversa(&org, &agente, Some(format!("55{n}").as_str()))
                .id
        })
        .collect();

    let vazio = FiltroConversas::default();
    let primeira = c
        .store
        .conversas()
        .listar_filtrado(org.id, &vazio, Pagina::com_limite(2))
        .expect("página 1");
    assert_eq!(primeira.iter().map(|c| c.id).collect::<Vec<_>>(), ids[..2]);

    let segunda = c
        .store
        .conversas()
        .listar_filtrado(
            org.id,
            &vazio,
            Pagina {
                apos: Some(ids[1].as_uuid()),
                limite: 2,
            },
        )
        .expect("página 2");
    assert_eq!(
        segunda.iter().map(|c| c.id).collect::<Vec<_>>(),
        ids[2..4],
        "o cursor é o último id da página anterior"
    );
}

#[test]
fn ordem_recente_usa_atividade_desempata_por_id_e_respeita_o_limite() {
    let c = Cenario::novo("recente");
    let org = c.org("a");
    let agente = c.agente(&org, "a");
    let c1 = c.conversa(&org, &agente, Some("5511"));
    let c2 = c.conversa(&org, &agente, Some("5522"));
    let c3 = c.conversa(&org, &agente, Some("5533"));

    // c1 e c3 com a mesma atividade mais recente; c2 só com a data de criação, que é a mais antiga.
    c.store
        .conversas()
        .tocar(org.id, c1.id, 9_000_000)
        .expect("toca c1");
    c.store
        .conversas()
        .tocar(org.id, c3.id, 9_000_000)
        .expect("toca c3");

    let sem = FiltroConversas::default();
    let ordenadas: Vec<ConversationId> = c
        .store
        .conversas()
        .listar_recentes(org.id, &sem, 50)
        .expect("recentes")
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(
        ordenadas,
        vec![c3.id, c1.id, c2.id],
        "atividade decrescente; empate na mesma atividade vai pelo id maior primeiro"
    );

    let dois = c
        .store
        .conversas()
        .listar_recentes(org.id, &sem, 2)
        .expect("limite 2");
    assert_eq!(
        dois.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![c3.id, c1.id]
    );

    let so_cliente = c
        .store
        .conversas()
        .listar_recentes(org.id, &filtro_peer("5522"), 50)
        .expect("recentes do cliente");
    assert_eq!(
        so_cliente.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![c2.id],
        "a ordem recente também respeita o filtro por cliente"
    );
}

#[test]
fn plano_da_consulta_por_cliente_usa_o_indice_novo_e_nao_varre_a_tabela() {
    let c = Cenario::novo("plano");
    let org = c.org("a");
    let agente = c.agente(&org, "a");
    for n in 0..3 {
        c.conversa(&org, &agente, Some(format!("55{n}").as_str()));
    }

    let por_cliente = FiltroConversas {
        external_peer: Some("5511".to_owned()),
        status: Some(StatusConversa::Aberta),
        ..Default::default()
    };
    for ordem in [OrdemConversas::Id, OrdemConversas::Recente] {
        let plano = c
            .store
            .conversas()
            .plano_da_listagem(org.id, &por_cliente, ordem)
            .expect("plano");
        let texto = plano.join(" | ");
        assert!(
            texto.contains("ix_conversations_org_peer_id"),
            "o plano não usa o índice novo ({ordem:?}): {texto}"
        );
        assert!(
            texto.contains("external_peer=?"),
            "a busca não usa a coluna do cliente ({ordem:?}): {texto}"
        );
        if ordem == OrdemConversas::Id {
            assert!(
                !texto.contains("TEMP B-TREE"),
                "a ordem por id sai do índice, sem ordenação temporária: {texto}"
            );
        }
        assert!(
            !texto.contains("SCAN conversations"),
            "a consulta varre a tabela inteira ({ordem:?}): {texto}"
        );
    }
}

#[test]
fn identificador_do_cliente_e_o_peer_sem_espaco_ou_o_id_quando_vazio() {
    let c = Cenario::novo("identificador");
    let org = c.org("a");
    let agente = c.agente(&org, "a");

    let normal = c.conversa(&org, &agente, Some("5511999999999"));
    assert_eq!(normal.identificador_cliente(), "5511999999999");

    let com_espacos = c.conversa(&org, &agente, Some("  5511999999999  "));
    assert_eq!(
        com_espacos.identificador_cliente(),
        "5511999999999",
        "o identificador sai sem espaço nas pontas"
    );

    let vazio = c.conversa(&org, &agente, Some("   "));
    assert_eq!(
        vazio.identificador_cliente(),
        vazio.id.to_string(),
        "peer só com espaço cai no id da conversa"
    );

    let sem_peer = c.conversa(&org, &agente, None);
    assert_eq!(sem_peer.identificador_cliente(), sem_peer.id.to_string());

    // Para o cliente canônico (o que o canal grava: só dígitos), o identificador da sala é o
    // mesmo valor que o filtro procura, e a conversa aparece no histórico dele.
    let achadas = c.listar(org.id, &filtro_peer(&normal.identificador_cliente()));
    assert!(
        achadas.contains(&normal.id),
        "o identificador da sala acha a conversa pelo filtro de cliente"
    );
}

#[test]
fn filtro_por_ultima_atividade_traz_so_as_conversas_paradas_da_org() {
    let c = Cenario::novo("inatividade");
    let org = c.org("a");
    let outra = c.org("b");
    let agente = c.agente(&org, "a");
    let agente_b = c.agente(&outra, "b");
    let primeira = c.conversa(&org, &agente, Some("5511"));
    let segunda = c.conversa(&org, &agente, Some("5522"));
    let terceira = c.conversa(&org, &agente, Some("5533"));
    let alheia = c.conversa(&outra, &agente_b, Some("5511"));
    assert!(alheia.criado_em >= terceira.criado_em);

    let paradas_ate = FiltroConversas {
        last_activity_before: Some(segunda.criado_em + 1),
        ..Default::default()
    };
    assert_eq!(
        c.listar(org.id, &paradas_ate),
        vec![primeira.id, segunda.id]
    );

    let nada = FiltroConversas {
        last_activity_before: Some(primeira.criado_em),
        ..Default::default()
    };
    assert!(c.listar(org.id, &nada).is_empty());

    let abertas_paradas = FiltroConversas {
        status: Some(StatusConversa::Aberta),
        external_peer: None,
        last_activity_before: Some(segunda.criado_em + 1),
    };
    assert_eq!(
        c.listar(org.id, &abertas_paradas),
        vec![primeira.id, segunda.id]
    );

    c.store
        .conversas()
        .mudar_status(org.id, primeira.id, 1, StatusConversa::Fechada, None)
        .expect("fecha a primeira");
    assert_eq!(c.listar(org.id, &abertas_paradas), vec![segunda.id]);
}
