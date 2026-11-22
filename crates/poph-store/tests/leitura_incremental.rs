//! Testes da MX-05: leitura incremental de mensagens (`Mensagens::desde`) e de conversas
//! (`Conversas::alteradas_desde`), a base do espelhamento para o Matrix (PRD §7 N3).
//!
//! Cobrem: ordem estável por `rowid`; paginação até o fim sem perder nem repetir linha, mesmo com
//! mensagens gravadas entre as páginas; `tool` e `system` fora da leitura; origem, autor e id
//! externo chegando intactos; e, para conversas, paginação por `(updated_at, id)` com empate de
//! milissegundo. Usam banco de verdade em pasta temporária própria.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::migracoes::{MIGRACOES_HARNESS, migrar};
use poph_store::{
    Direcao, Mensagem, NovaConversa, NovaMensagem, NovaOrg, NovoAgente, Origem, Papel, Store,
};
use poph_types::{ConversationId, OrgId};

static CONTADOR: AtomicUsize = AtomicUsize::new(0);

/// Banco temporário com uma org e um agente. A pasta é apagada ao sair.
struct Ambiente {
    pasta: PathBuf,
    store: Store,
    org: OrgId,
    agente: poph_types::AgentId,
}

impl Ambiente {
    fn novo(rotulo: &str) -> Self {
        let n = CONTADOR.fetch_add(1, Ordering::SeqCst);
        let pasta =
            std::env::temp_dir().join(format!("poph-mx05-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria pasta");
        let store =
            Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db")).expect("abre banco");
        let org = store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {rotulo}"),
                slug: format!("mx05-{rotulo}-{n}"),
                config_json: String::new(),
            })
            .expect("org")
            .id;
        let agente = store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: "Agente".to_owned(),
                    slug: format!("agente-{n}"),
                    ..NovoAgente::default()
                },
            )
            .expect("agente")
            .id;
        Self {
            pasta,
            store,
            org,
            agente,
        }
    }

    fn conversa(&self, peer: &str) -> ConversationId {
        self.store
            .conversas()
            .criar(
                self.org,
                NovaConversa {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "whatsapp".to_owned(),
                    peer_externo: Some(peer.to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("conversa")
            .id
    }

    /// Grava uma mensagem com o papel dado. `Usuario` entra; os demais saem.
    fn gravar(&self, conversa: ConversationId, papel: Papel, texto: &str) -> Mensagem {
        let direcao = match papel {
            Papel::Usuario => Direcao::Entrada,
            _ => Direcao::Saida,
        };
        self.store
            .mensagens()
            .gravar(
                self.org,
                NovaMensagem {
                    conversation_id: conversa,
                    direcao,
                    papel,
                    origem: Origem::Agente,
                    autor_nome: None,
                    texto: texto.to_owned(),
                    anexos_json: "[]".to_owned(),
                    id_externo: None,
                    turn_id: None,
                },
            )
            .expect("grava mensagem")
    }

    /// Lê tudo a partir de `apos`, pagando de `limite` em `limite`, e devolve as linhas na ordem.
    fn ler_tudo(&self, apos: u64, limite: u32) -> Vec<poph_store::MensagemDesde> {
        let mut cursor = apos;
        let mut todas = Vec::new();
        loop {
            let pagina = self.store.mensagens().desde(cursor, limite).expect("desde");
            if pagina.is_empty() {
                break;
            }
            cursor = pagina.last().expect("ultima").rowid;
            todas.extend(pagina);
        }
        todas
    }
}

impl Drop for Ambiente {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

#[test]
fn desde_devolve_em_ordem_crescente_de_rowid() {
    let amb = Ambiente::novo("ordem");
    let conversa = amb.conversa("5511900000001");
    let textos = ["um", "dois", "tres", "quatro", "cinco"];
    for (i, texto) in textos.iter().enumerate() {
        let papel = if i % 2 == 0 {
            Papel::Usuario
        } else {
            Papel::Assistente
        };
        amb.gravar(conversa, papel, texto);
    }

    let pagina = amb.store.mensagens().desde(0, 200).expect("desde");
    let lidos: Vec<&str> = pagina.iter().map(|m| m.texto.as_str()).collect();
    assert_eq!(lidos, textos);
    assert!(
        pagina.windows(2).all(|par| par[0].rowid < par[1].rowid),
        "rowid precisa crescer estritamente"
    );
    assert!(pagina.iter().all(|m| m.conversation_id == conversa));
    assert!(pagina.iter().all(|m| m.org_id == amb.org));
}

#[test]
fn paginar_ate_o_fim_nao_perde_nem_repete_linha() {
    let amb = Ambiente::novo("paginas");
    let conversa = amb.conversa("5511900000002");
    let gravadas: Vec<_> = (0..25)
        .map(|i| {
            let papel = if i % 3 == 0 {
                Papel::Assistente
            } else {
                Papel::Usuario
            };
            amb.gravar(conversa, papel, &format!("msg {i}"))
        })
        .collect();

    // Página de 7: 25 linhas viram 7, 7, 7, 4.
    let lidas = amb.ler_tudo(0, 7);

    assert_eq!(lidas.len(), gravadas.len(), "nenhuma linha pode faltar");
    let ids_lidos: Vec<_> = lidas.iter().map(|m| m.id).collect();
    let ids_gravados: Vec<_> = gravadas.iter().map(|m| m.id).collect();
    assert_eq!(ids_lidos, ids_gravados, "mesma sequência, sem repetição");
    // Depois do fim, o cursor da última linha não devolve mais nada.
    let fim = lidas.last().expect("ultima").rowid;
    assert!(
        amb.store
            .mensagens()
            .desde(fim, 7)
            .expect("desde")
            .is_empty()
    );
}

#[test]
fn mensagens_gravadas_entre_paginas_aparecem_depois_sem_perder_as_outras() {
    let amb = Ambiente::novo("entre-paginas");
    let conversa = amb.conversa("5511900000003");
    let inicio: Vec<_> = (0..10)
        .map(|i| amb.gravar(conversa, Papel::Usuario, &format!("antes {i}")))
        .collect();

    let primeira = amb.store.mensagens().desde(0, 4).expect("primeira");
    assert_eq!(primeira.len(), 4);
    let cursor = primeira.last().expect("ultima").rowid;

    // Chegam mais mensagens enquanto o laço ainda está no meio.
    let depois: Vec<_> = (0..3)
        .map(|i| amb.gravar(conversa, Papel::Assistente, &format!("depois {i}")))
        .collect();

    let resto = amb.ler_tudo(cursor, 4);
    let ids_resto: Vec<_> = resto.iter().map(|m| m.id).collect();
    let esperado: Vec<_> = inicio[4..]
        .iter()
        .chain(depois.iter())
        .map(|m| m.id)
        .collect();
    assert_eq!(ids_resto, esperado);

    let ids_primeira: Vec<_> = primeira.iter().map(|m| m.id).collect();
    let ids_inicio: Vec<_> = inicio[..4].iter().map(|m| m.id).collect();
    assert_eq!(ids_primeira, ids_inicio);
}

#[test]
fn tool_e_system_nunca_aparecem_na_leitura() {
    let amb = Ambiente::novo("papeis");
    let conversa = amb.conversa("5511900000004");
    amb.gravar(conversa, Papel::Usuario, "oi");
    amb.gravar(conversa, Papel::Sistema, "instrucao interna");
    amb.gravar(conversa, Papel::Ferramenta, "resultado da ferramenta");
    amb.gravar(conversa, Papel::Assistente, "tudo bem");
    amb.gravar(conversa, Papel::Ferramenta, "outra ferramenta");

    // Com limite 1, as linhas proibidas no meio não podem travar nem sumir a paginação.
    let lidas = amb.ler_tudo(0, 1);
    let textos: Vec<&str> = lidas.iter().map(|m| m.texto.as_str()).collect();
    assert_eq!(textos, ["oi", "tudo bem"]);
    assert!(
        lidas
            .iter()
            .all(|m| matches!(m.papel, Papel::Usuario | Papel::Assistente)),
        "só user e assistant"
    );
}

#[test]
fn origem_autor_e_id_externo_chegam_intactos() {
    let amb = Ambiente::novo("origem");
    let conversa = amb.conversa("5511900000005");
    let humana = amb
        .store
        .mensagens()
        .gravar(
            amb.org,
            NovaMensagem {
                conversation_id: conversa,
                direcao: Direcao::Saida,
                papel: Papel::Assistente,
                origem: Origem::Humano,
                autor_nome: Some("Ana".to_owned()),
                texto: "posso ajudar".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: Some("$evento-1".to_owned()),
                turn_id: None,
            },
        )
        .expect("grava humana");

    let lidas = amb.store.mensagens().desde(0, 10).expect("desde");
    assert_eq!(lidas.len(), 1);
    let m = &lidas[0];
    assert_eq!(m.id, humana.id);
    assert_eq!(m.origem, Origem::Humano);
    assert_eq!(m.autor_nome.as_deref(), Some("Ana"));
    assert_eq!(m.id_externo.as_deref(), Some("$evento-1"));
    assert_eq!(m.direcao, Direcao::Saida);
    assert_eq!(m.papel, Papel::Assistente);
}

#[test]
fn leitura_traz_mensagens_de_mais_de_uma_org_com_a_org_de_cada_linha() {
    let amb = Ambiente::novo("duas-orgs-a");
    let outra = amb
        .store
        .orgaos()
        .criar(NovaOrg {
            nome: "Outra".to_owned(),
            slug: format!("mx05-outra-{}", std::process::id()),
            config_json: String::new(),
        })
        .expect("outra org")
        .id;
    let agente_outra = amb
        .store
        .agentes()
        .criar(
            outra,
            NovoAgente {
                nome: "Outro".to_owned(),
                slug: "outro".to_owned(),
                ..NovoAgente::default()
            },
        )
        .expect("agente outra")
        .id;
    let conversa_a = amb.conversa("a");
    let conversa_b = amb
        .store
        .conversas()
        .criar(
            outra,
            NovaConversa {
                agent_id: agente_outra,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some("b".to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("conversa outra")
        .id;
    amb.gravar(conversa_a, Papel::Usuario, "da A");
    amb.store
        .mensagens()
        .gravar(
            outra,
            NovaMensagem {
                conversation_id: conversa_b,
                direcao: Direcao::Entrada,
                papel: Papel::Usuario,
                origem: Origem::Agente,
                autor_nome: None,
                texto: "da B".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: None,
            },
        )
        .expect("grava B");

    // A leitura é global: quem espelha filtra pela org da linha.
    let lidas = amb.store.mensagens().desde(0, 10).expect("desde");
    assert_eq!(lidas.len(), 2);
    assert_eq!(lidas[0].org_id, amb.org);
    assert_eq!(lidas[1].org_id, outra);
    assert_eq!(lidas[1].conversation_id, conversa_b);
}

#[test]
fn alteradas_desde_pagina_por_instante_e_id_sem_perder_empate() {
    let amb = Ambiente::novo("conversas");
    let ids: Vec<ConversationId> = (0..5).map(|i| amb.conversa(&format!("peer-{i}"))).collect();
    // Três conversas com o mesmo instante (empate de milissegundo) e duas em outros instantes.
    amb.store
        .conversas()
        .tocar(amb.org, ids[0], 1000)
        .expect("tocar");
    amb.store
        .conversas()
        .tocar(amb.org, ids[1], 1000)
        .expect("tocar");
    amb.store
        .conversas()
        .tocar(amb.org, ids[2], 1000)
        .expect("tocar");
    amb.store
        .conversas()
        .tocar(amb.org, ids[3], 500)
        .expect("tocar");
    amb.store
        .conversas()
        .tocar(amb.org, ids[4], 2000)
        .expect("tocar");

    let mut cursor: Option<(i64, ConversationId)> = None;
    let mut vistas: Vec<ConversationId> = Vec::new();
    loop {
        let pagina = amb
            .store
            .conversas()
            .alteradas_desde(cursor, 1)
            .expect("alteradas");
        let Some(ultima) = pagina.last() else { break };
        cursor = Some((ultima.atualizado_em, ultima.id));
        vistas.extend(pagina.iter().map(|c| c.id));
    }

    assert_eq!(vistas.len(), 5, "nenhuma conversa pode faltar nem repetir");
    // Ordem esperada: 500 ms, depois os três empates de 1000 ms (desempatados pelo id), depois 2000.
    let mut empatadas = vec![ids[0], ids[1], ids[2]];
    empatadas.sort_by_key(|id| id.to_string());
    let mut esperado_ids: Vec<ConversationId> = vec![ids[3]];
    esperado_ids.extend(empatadas);
    esperado_ids.push(ids[4]);
    assert_eq!(vistas, esperado_ids);
}

#[test]
fn alteradas_desde_none_comeca_do_inicio_e_depois_do_cursor_nao_repete() {
    let amb = Ambiente::novo("conversas-inicio");
    let a = amb.conversa("x");
    let b = amb.conversa("y");
    amb.store.conversas().tocar(amb.org, a, 10).expect("tocar");
    amb.store.conversas().tocar(amb.org, b, 20).expect("tocar");

    let todas = amb
        .store
        .conversas()
        .alteradas_desde(None, 1000)
        .expect("todas");
    let ids: Vec<_> = todas.iter().map(|c| c.id).collect();
    assert!(ids.contains(&a) && ids.contains(&b));

    let depois_de_a = amb
        .store
        .conversas()
        .alteradas_desde(Some((10, a)), 1000)
        .expect("depois");
    assert!(
        depois_de_a.iter().all(|c| c.id != a),
        "o cursor não repete a própria conversa"
    );
    assert!(depois_de_a.iter().any(|c| c.id == b));
}

#[test]
fn migracao_0006_sobre_banco_com_dados_preserva_linhas_e_leitura_funciona() {
    static CONTADOR: AtomicUsize = AtomicUsize::new(0);
    let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
    let pasta = std::env::temp_dir().join(format!("poph-mx05-migracao-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&pasta);
    fs::create_dir_all(&pasta).expect("cria a pasta");
    let caminho = pasta.join("harness.db");
    let conn = rusqlite::Connection::open(&caminho).expect("abre o banco");

    // Banco no estado anterior (migrações 1 a 5), com conversas e mensagens gravadas.
    migrar(&conn, "harness.db", &caminho, &MIGRACOES_HARNESS[..5], 1).expect("migra até a 5");
    // Org e agente de verdade: o backfill das Fichas (FX-05, migração 0012) referencia a org, então
    // uma conversa órfã aqui faria a migração falhar por chave estrangeira. A checagem continua
    // desligada nesta conexão, como estava.
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .expect("desliga");
    conn.execute_batch(
        "INSERT INTO orgs (id, name, slug, created_at, updated_at)
            VALUES ('0190a000-0000-7000-8000-00000000a001', 'Org da migração', 'org-migracao-0006', 1, 1);
         INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
            VALUES ('0190a000-0000-7000-8000-00000000a002',
                    '0190a000-0000-7000-8000-00000000a001', 'Agente da migração',
                    'agente-migracao-0006', 'antigravity2api', 'gemini', 1, 1);
         INSERT INTO conversations (id, org_id, agent_id, channel, status, version, created_at, updated_at)
            VALUES ('0190a000-0000-7000-8000-00000000c001', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000a002', 'whatsapp', 'open', 1, 10, 30),
                   ('0190a000-0000-7000-8000-00000000c002', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000a002', 'whatsapp', 'open', 1, 10, 20);
         INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at)
            VALUES ('0190a000-0000-7000-8000-00000000d001', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000c001', 'in', 'user', 'oi', 10),
                   ('0190a000-0000-7000-8000-00000000d002', '0190a000-0000-7000-8000-00000000a001', '0190a000-0000-7000-8000-00000000c002', 'out', 'assistant', 'tudo bem', 20);",
    )
    .expect("grava dados no formato anterior");

    let relatorio =
        migrar(&conn, "harness.db", &caminho, &MIGRACOES_HARNESS[..6], 2).expect("até a 6");
    assert_eq!(relatorio.versao_antes, 5);
    assert_eq!(relatorio.versao_depois, 6);

    let contagem = |sql: &str| -> i64 { conn.query_row(sql, [], |l| l.get(0)).expect("conta") };
    assert_eq!(
        contagem("SELECT count(*) FROM messages"),
        2,
        "nenhuma mensagem perdida"
    );
    assert_eq!(
        contagem("SELECT count(*) FROM conversations"),
        2,
        "nenhuma conversa perdida"
    );
    assert_eq!(
        contagem(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'ix_conversations_atualizada'"
        ),
        1,
        "o índice novo existe"
    );
    let integridade: String = conn
        .query_row("PRAGMA integrity_check", [], |l| l.get(0))
        .expect("integridade");
    assert_eq!(integridade, "ok");
    drop(conn);

    // A leitura nova enxerga as linhas que já existiam, na ordem certa.
    let store = Store::abrir_em(&caminho, &pasta.join("rag.db")).expect("abre o store");
    let mensagens = store.mensagens().desde(0, 10).expect("desde");
    let textos: Vec<&str> = mensagens.iter().map(|m| m.texto.as_str()).collect();
    assert_eq!(textos, ["oi", "tudo bem"]);
    let conversas = store
        .conversas()
        .alteradas_desde(None, 10)
        .expect("alteradas");
    assert_eq!(conversas.len(), 2);
    assert_eq!(conversas[0].atualizado_em, 20, "a mais antiga primeiro");
    assert_eq!(conversas[1].atualizado_em, 30);

    drop(store);
    let _ = fs::remove_dir_all(&pasta);
}

#[test]
fn plano_da_leitura_de_conversas_usa_o_indice_e_nao_varre_a_tabela() {
    // A consulta é a mesma de `Conversas::alteradas_desde`. Se ela mudar, este teste precisa
    // acompanhar, porque é ele que segura o índice `ix_conversations_atualizada`.
    let amb = Ambiente::novo("plano");
    let caminho = amb.store.caminho_harness().to_path_buf();
    let con = rusqlite::Connection::open(&caminho).expect("abre para ler o plano");
    let detalhes: Vec<String> = {
        let mut consulta = con
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM conversations
                  WHERE (updated_at, id) > (?1, ?2) ORDER BY updated_at, id LIMIT ?3",
            )
            .expect("prepara");
        consulta
            .query_map(rusqlite::params![0_i64, "", 10_i64], |l| {
                l.get::<_, String>(3)
            })
            .expect("plano")
            .map(|l| l.expect("linha"))
            .collect()
    };
    let texto = detalhes.join(" | ");
    assert!(
        texto.contains("ix_conversations_atualizada"),
        "plano: {texto}"
    );
    assert!(!texto.contains("SCAN conversations"), "plano: {texto}");
    assert!(!texto.contains("TEMP B-TREE"), "plano: {texto}");
}
