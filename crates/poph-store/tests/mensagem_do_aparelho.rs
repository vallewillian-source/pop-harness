//! Testes da MX-41: a operação `receber_do_aparelho` (mensagem que o cliente digitou no celular e
//! a Evolution devolve como `fromMe`) e a consulta `enviado_com_id_externo` (o eco do nosso envio).
//!
//! Cobrem: gravação feliz numa escrita só (mensagem `out`/`device` sem turno, conversa
//! `transferred`, versão subindo uma vez e auditoria `device_takeover`), **sem** linha na `inbox`
//! e **sem** linha na `outbox`; reentrega do mesmo id externo devolve `Duplicado` e não grava nada
//! de novo; conversa já transferida não sobe a versão outra vez; conversa fechada abre outra e a
//! nova nasce transferida; a Ficha nasce com a conversa aberta por aqui; falha no meio (auditoria
//! recusada por gatilho) não deixa nada gravado; e o eco é reconhecido só pelo id que a MX-40
//! guardou no envio confirmado.
//!
//! Todos usam banco de verdade em pasta temporária própria.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{
    NovaMensagemDoAparelho, NovaOrg, NovoAgente, Origem, Recebido, StatusConversa, Store,
};
use poph_types::{AgentId, IntegrationId, OrgId, TurnId};
use rusqlite::Connection;

/// Banco de verdade numa pasta temporária, apagada no fim.
struct Cenario {
    pasta: PathBuf,
    store: Option<Store>,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        static CONTADOR: AtomicUsize = AtomicUsize::new(0);
        let n = CONTADOR.fetch_add(1, Ordering::Relaxed);
        let pasta =
            std::env::temp_dir().join(format!("poph-aparelho-{rotulo}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco");
        Self {
            pasta,
            store: Some(store),
        }
    }

    fn store(&self) -> &Store {
        self.store.as_ref().expect("store viva")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        // Fecha o escritor antes de apagar a pasta.
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// Org, agente e integração do WhatsApp — o que o webhook tem em mãos.
struct OrgPronta {
    org: OrgId,
    agente: AgentId,
    integracao: IntegrationId,
}

fn org_pronta(store: &Store, slug: &str) -> OrgPronta {
    let org = store
        .orgaos()
        .criar(NovaOrg {
            nome: format!("Org {slug}"),
            slug: slug.to_owned(),
            config_json: String::new(),
        })
        .expect("cria a org")
        .id;
    let agente = store
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
        .id;
    let integracao: IntegrationId = store
        .integracao_criar(org, agente, "whatsapp".to_owned(), "active".to_owned())
        .expect("cria a integração")
        .id
        .parse()
        .expect("id da integração");
    OrgPronta {
        org,
        agente,
        integracao,
    }
}

fn nova(pronta: &OrgPronta, id_externo: &str, texto: &str) -> NovaMensagemDoAparelho {
    NovaMensagemDoAparelho {
        integration_id: pronta.integracao,
        agent_id: pronta.agente,
        canal: "whatsapp".to_owned(),
        peer: "5511999990000".to_owned(),
        id_externo: id_externo.to_owned(),
        texto: texto.to_owned(),
    }
}

fn conexao(store: &Store) -> Connection {
    Connection::open(store.caminho_harness()).expect("abre o banco para conferir")
}

/// Quantas linhas uma consulta de contagem devolve, lidas direto da tabela.
fn contar(store: &Store, sql: &str) -> i64 {
    conexao(store)
        .query_row(sql, [], |linha| linha.get(0))
        .expect("conta")
}

/// Linha única de uma consulta (`SELECT x, y, ... WHERE ...`), como texto (nulo vira `None`,
/// número vira o número em texto).
fn uma_linha(store: &Store, sql: &str, parametro: &str) -> Vec<Option<String>> {
    conexao(store)
        .query_row(sql, [parametro], |linha| {
            let mut valores = Vec::new();
            for i in 0..linha.as_ref().column_count() {
                valores.push(match linha.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => None,
                    rusqlite::types::ValueRef::Integer(n) => Some(n.to_string()),
                    rusqlite::types::ValueRef::Real(n) => Some(n.to_string()),
                    rusqlite::types::ValueRef::Text(t) => {
                        Some(String::from_utf8_lossy(t).into_owned())
                    }
                    rusqlite::types::ValueRef::Blob(_) => Some("<blob>".to_owned()),
                });
            }
            Ok(valores)
        })
        .expect("lê a linha")
}

/// Texto do SQL para a mensagem de um id externo.
const SQL_MENSAGEM: &str =
    "SELECT direction, role, origin, turn_id FROM messages WHERE external_id = ?1";

// ───────────────────────── gravação feliz ─────────────────────────

/// Aceite: a mensagem do aparelho entra uma vez, como saída do dispositivo, sem turno e sem fila,
/// e a conversa fica transferida com a auditoria `device_takeover`.
#[test]
fn grava_a_mensagem_do_aparelho_e_pausa_o_agente() {
    let cenario = Cenario::novo("feliz");
    let store = cenario.store();
    let pronta = org_pronta(store, "aparelho-feliz");

    let recebido = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-1", "já enviei, obrigado"))
        .expect("grava a mensagem do aparelho");

    assert_eq!(recebido.situacao, Recebido::Novo);
    assert!(recebido.transferiu_agora, "a conversa nasceu aqui e pausou");
    assert_eq!(recebido.mensagem.direcao.como_texto(), "out");
    assert_eq!(recebido.mensagem.origem, Origem::Dispositivo);
    assert!(recebido.mensagem.turn_id.is_none(), "sem turno");
    assert_eq!(recebido.mensagem.texto, "já enviei, obrigado");

    // A conversa do contato nasceu aberta e passou a transferida, com a versão subindo uma vez.
    let linha = uma_linha(
        store,
        "SELECT status, version FROM conversations WHERE id = ?1",
        &recebido.conversa.to_string(),
    );
    assert_eq!(linha[0].as_deref(), Some("transferred"));
    assert_eq!(linha[1].as_deref(), Some("2"));

    // A mensagem gravada é saída do dispositivo, sem turno.
    let mensagem = uma_linha(store, SQL_MENSAGEM, "MSG-AP-1");
    assert_eq!(mensagem[0].as_deref(), Some("out"));
    assert_eq!(mensagem[1].as_deref(), Some("assistant"));
    assert_eq!(mensagem[2].as_deref(), Some("device"));
    assert_eq!(mensagem[3], None, "sem turno");

    // Nada na inbox (o turno não tem o que pegar) e nada na outbox (já saiu pelo celular).
    assert_eq!(contar(store, "SELECT COUNT(*) FROM inbox"), 0, "inbox");
    assert_eq!(contar(store, "SELECT COUNT(*) FROM outbox"), 0, "outbox");

    // Auditoria `device_takeover` pelo sistema, sem ator humano.
    let auditoria = uma_linha(
        store,
        "SELECT actor_kind, action, entity FROM audit_log WHERE entity_id = ?1",
        &recebido.conversa.to_string(),
    );
    assert_eq!(auditoria[0].as_deref(), Some("system"));
    assert_eq!(auditoria[1].as_deref(), Some("device_takeover"));
    assert_eq!(auditoria[2].as_deref(), Some("conversation"));

    // A Ficha do atendimento nasce com a conversa aberta por este caminho.
    assert_eq!(
        contar(store, "SELECT COUNT(*) FROM fichas"),
        1,
        "ficha da conversa nova"
    );
}

/// Aceite: reentrega do mesmo id externo não duplica mensagem nem auditoria.
#[test]
fn reentrega_do_mesmo_id_externo_nao_duplica() {
    let cenario = Cenario::novo("reentrega");
    let store = cenario.store();
    let pronta = org_pronta(store, "aparelho-reentrega");

    let primeira = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-2", "oi"))
        .expect("primeira entrega");
    let segunda = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-2", "oi"))
        .expect("reentrega");

    assert_eq!(segunda.situacao, Recebido::Duplicado);
    assert_eq!(segunda.mensagem.id, primeira.mensagem.id);
    assert_eq!(segunda.conversa, primeira.conversa);
    assert!(!segunda.transferiu_agora, "a segunda não transfere de novo");
    assert_eq!(contar(store, "SELECT COUNT(*) FROM messages"), 1);
    assert_eq!(
        contar(
            store,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"
        ),
        1
    );
    assert_eq!(contar(store, "SELECT COUNT(*) FROM conversations"), 1);

    // A versão subiu uma vez só.
    let versao = uma_linha(
        store,
        "SELECT version FROM conversations WHERE id = ?1",
        &primeira.conversa.to_string(),
    );
    assert_eq!(versao[0].as_deref(), Some("2"));
}

/// A conversa que já está transferida continua transferida e não sobe a versão de novo.
#[test]
fn conversa_ja_transferida_nao_sobe_a_versao() {
    let cenario = Cenario::novo("ja-transferida");
    let store = cenario.store();
    let pronta = org_pronta(store, "aparelho-ja-transferida");

    let primeira = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-3", "primeira"))
        .expect("primeira");
    let segunda = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-4", "segunda"))
        .expect("segunda");

    assert_eq!(segunda.situacao, Recebido::Novo);
    assert!(!segunda.transferiu_agora);
    assert_eq!(segunda.conversa, primeira.conversa, "mesma conversa");
    let linha = uma_linha(
        store,
        "SELECT status, version FROM conversations WHERE id = ?1",
        &primeira.conversa.to_string(),
    );
    assert_eq!(linha[0].as_deref(), Some("transferred"));
    assert_eq!(linha[1].as_deref(), Some("2"), "a versão não sobe de novo");
    assert_eq!(
        contar(
            store,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'device_takeover'"
        ),
        2,
        "cada mensagem do aparelho deixa a sua auditoria"
    );
}

/// Conversa fechada não reabre: a mensagem do aparelho abre outra e a nova nasce transferida.
#[test]
fn conversa_fechada_abre_outra() {
    let cenario = Cenario::novo("fechada");
    let store = cenario.store();
    let pronta = org_pronta(store, "aparelho-fechada");

    let primeira = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-5", "antes de fechar"))
        .expect("primeira");
    let atual = store
        .conversas()
        .obter(pronta.org, primeira.conversa)
        .expect("lê a conversa");
    store
        .conversas()
        .mudar_status(
            pronta.org,
            primeira.conversa,
            atual.versao,
            StatusConversa::Fechada,
            None,
        )
        .expect("fecha a conversa");

    let depois = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-6", "depois de fechar"))
        .expect("mensagem depois do fechamento");

    assert_ne!(depois.conversa, primeira.conversa, "conversa nova");
    assert!(depois.transferiu_agora);
    let linha = uma_linha(
        store,
        "SELECT status FROM conversations WHERE id = ?1",
        &depois.conversa.to_string(),
    );
    assert_eq!(linha[0].as_deref(), Some("transferred"));
}

/// Falha no meio (auditoria recusada por gatilho) não deixa mensagem, conversa nem auditoria.
#[test]
fn falha_na_auditoria_nao_deixa_nada_gravado() {
    let cenario = Cenario::novo("falha");
    let store = cenario.store();
    let pronta = org_pronta(store, "aparelho-falha");

    conexao(store)
        .execute_batch(
            "CREATE TRIGGER recusa_device_takeover BEFORE INSERT ON audit_log
             WHEN NEW.action = 'device_takeover'
             BEGIN SELECT RAISE(ABORT, 'falha simulada'); END;",
        )
        .expect("cria o gatilho");

    let erro = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-7", "vai falhar"))
        .expect_err("a gravação tem de falhar");
    assert!(!format!("{erro}").is_empty());

    assert_eq!(
        contar(store, "SELECT COUNT(*) FROM messages"),
        0,
        "mensagem"
    );
    assert_eq!(
        contar(store, "SELECT COUNT(*) FROM conversations"),
        0,
        "conversa"
    );
    assert_eq!(
        contar(store, "SELECT COUNT(*) FROM audit_log"),
        0,
        "auditoria"
    );
}

// ───────────────────────── eco do nosso envio (MX-40) ─────────────────────────

/// Só o id guardado no envio confirmado é reconhecido como eco, e só na org dona.
#[test]
fn enviado_com_id_externo_reconhece_so_o_id_do_envio_confirmado() {
    let cenario = Cenario::novo("eco");
    let store = cenario.store();
    let pronta = org_pronta(store, "aparelho-eco");
    let outra = org_pronta(store, "aparelho-eco-outra");

    let recebido = store
        .caixa_entrada()
        .receber_do_aparelho(pronta.org, nova(&pronta, "MSG-AP-8", "abre a conversa"))
        .expect("mensagem do aparelho");

    // Envio nosso, na mesma conversa, ainda sem confirmação: nenhum id gravado.
    let turno = TurnId::generate();
    let saida = store
        .mensagens()
        .gravar_com_saida(
            pronta.org,
            poph_store::NovaMensagem {
                conversation_id: recebido.conversa,
                direcao: poph_store::Direcao::Saida,
                papel: poph_store::Papel::Assistente,
                origem: Origem::Agente,
                autor_nome: None,
                texto: "resposta do agente".to_owned(),
                anexos_json: "[]".to_owned(),
                id_externo: None,
                turn_id: Some(turno),
            },
            poph_store::NovaSaida {
                conversation_id: recebido.conversa,
                turn_id: turno,
                seq: 0,
                tipo: "text".to_owned(),
                conteudo_json: "{\"texto\":\"oi\"}".to_owned(),
            },
        )
        .expect("grava a saída")
        .1;

    assert!(
        !store
            .caixa_saida()
            .enviado_com_id_externo(pronta.org, "EVO-XYZ")
            .expect("consulta o eco"),
        "antes da confirmação o id não existe"
    );

    store
        .caixa_saida()
        .confirmar_envio_com_id(pronta.org, saida.id, Some("EVO-XYZ"))
        .expect("confirma o envio com id");

    assert!(
        store
            .caixa_saida()
            .enviado_com_id_externo(pronta.org, "EVO-XYZ")
            .expect("consulta o eco"),
        "o id devolvido pela Evolution é o eco"
    );
    assert!(
        !store
            .caixa_saida()
            .enviado_com_id_externo(outra.org, "EVO-XYZ")
            .expect("consulta o eco da outra org"),
        "o eco não atravessa org"
    );
    assert!(
        !store
            .caixa_saida()
            .enviado_com_id_externo(pronta.org, "EVO-OUTRO")
            .expect("consulta outro id"),
        "id desconhecido não é eco"
    );
}
