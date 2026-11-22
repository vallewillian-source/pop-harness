//! Testes da invariante conversa ↔ Ficha e do repositório de Fichas (FX-07).
//!
//! O que fica provado aqui:
//!
//! - **todo caminho que cria conversa cria a Ficha na mesma escrita** — `Conversas::criar` e a
//!   recepção do canal (`receber_do_canal`, o caminho do WhatsApp), com a mensagem seguinte na mesma
//!   conversa **não** criando Ficha nova;
//! - **todo caminho que fecha conversa fecha a Ficha** — `fechar_auditado` e `mudar_status`, com a
//!   execução de fluxo ativa abandonada e a linha do tempo registrando os dois fatos;
//! - **a escrita é atômica**: falha injetada no meio não deixa conversa fechada com Ficha aberta;
//! - **mensagem depois do fechamento abre conversa e Ficha novas** (o ciclo do PRD L5);
//! - **`reopen` de conversa `closed` é recusado** e o de `transferred` continua valendo (L13);
//! - **isolamento entre orgs** em leitura e escrita, e a lista de Fichas não cruza orgs;
//! - **eventos**: ordem por `seq`, paginação pelo cursor, idempotência por chave e recusa em Ficha
//!   fechada.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use poph_store::{
    Agente, Conversa, MOTIVO_ABANDONO_ATENDIMENTO, NovaConversa, NovaEntradaCanal, NovaFicha,
    NovaOrg, NovoAgente, NovoEventoFicha, Org, Pagina, Recebido, StatusConversa, Store,
};
use poph_types::{
    AgentId, AtorEvento, ConversationId, DomainError, FechadaPor, FichaId, OrgId, StatusFicha,
    TipoEventoFicha,
};
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
            std::env::temp_dir().join(format!("poph-fichas-{rotulo}-{}-{n}", std::process::id()));
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

    /// Conexão própria, para o que o repositório ainda não expõe (tabelas de fluxo são a FX-08) e
    /// para conferir o banco por SQL.
    fn conexao(&self) -> Connection {
        let conn = Connection::open(self.pasta.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
    }

    fn org(&self, slug: &str) -> Org {
        self.store()
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {slug}"),
                slug: slug.to_owned(),
                config_json: String::new(),
            })
            .expect("cria a org")
    }

    fn agente(&self, org: &Org, slug: &str) -> Agente {
        self.store()
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

    fn conversa(&self, org: &Org, agente: &Agente) -> Conversa {
        self.store()
            .conversas()
            .criar(
                org.id,
                NovaConversa {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some("cliente-1".to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa")
    }

    /// Fecha a conversa pelo caminho do administrador (`close` da API, MX-06).
    fn fechar(&self, org: &Org, conversa: &Conversa) -> Conversa {
        self.store()
            .conversas()
            .fechar_auditado(
                org.id,
                conversa.id,
                conversa.versao,
                "token-de-teste".to_owned(),
            )
            .expect("fecha a conversa")
    }

    /// Execução de fluxo ativa na Ficha, gravada direto (as tabelas de fluxo não têm repositório
    /// ainda; quem escreve nelas é a FX-08).
    fn execucao_ativa(&self, org: &Org, ficha: FichaId, agora: i64) -> String {
        let conn = self.conexao();
        let fluxo = uuid::Uuid::now_v7().to_string();
        let revisao = uuid::Uuid::now_v7().to_string();
        let execucao = uuid::Uuid::now_v7().to_string();
        conn.execute(
            "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                                 revisao_atual, version)
             VALUES (?1, ?2, 'agendar_visita', 'Agendar visita', '', 'active', '', 1, 1)",
            rusqlite::params![fluxo, org.id.to_string()],
        )
        .expect("fluxo");
        conn.execute(
            "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                        definicao_json, schema_versao, publicada_em, publicada_por,
                                        resolucoes_json)
             VALUES (?1, ?2, ?3, 1, '', 'sha', '{}', 1, ?4, 'admin_token', '{}')",
            rusqlite::params![revisao, org.id.to_string(), fluxo, agora],
        )
        .expect("revisão");
        conn.execute(
            "INSERT INTO fluxo_execucoes (id, org_id, ficha_id, fluxo_id, revisao_id, status,
                                         no_atual, dados_json, resultado, motivo_fim, iniciada_em,
                                         finalizada_em, version)
             VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', 'pedir_endereco', '{}', NULL, NULL, ?6, NULL, 1)",
            rusqlite::params![
                execucao,
                org.id.to_string(),
                ficha.to_string(),
                fluxo,
                revisao,
                agora
            ],
        )
        .expect("execução");
        execucao
    }

    /// Cria uma integração de WhatsApp para o agente (o caminho da recepção do canal exige uma).
    fn integracao(&self, org: &Org, agente: &Agente) -> poph_types::IntegrationId {
        self.store()
            .integracao_criar(org.id, agente.id, "whatsapp".into(), "active".into())
            .expect("integração")
            .id
            .parse()
            .expect("id da integração")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = self.store.take();
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn entrada(
    integracao: poph_types::IntegrationId,
    agente: AgentId,
    id_externo: &str,
) -> NovaEntradaCanal {
    NovaEntradaCanal {
        integration_id: integracao,
        agent_id: agente,
        canal: "whatsapp".to_owned(),
        id_externo: id_externo.to_owned(),
        peer: "5521999990000".to_owned(),
        evento_json: "{}".to_owned(),
        texto: "oi".to_owned(),
        anexos: Vec::new(),
        nome_cliente: None,
    }
}

// ─────────────────────────── Criação: toda conversa ganha Ficha ───────────────────────────

#[test]
fn criar_conversa_cria_a_ficha_na_mesma_escrita() {
    let cenario = Cenario::novo("criar");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);

    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("a conversa nova tem Ficha");

    assert_eq!(ficha.status, StatusFicha::Aberta);
    assert_eq!(ficha.agent_id, agente.id);
    assert_eq!(ficha.canal, "console");
    assert_eq!(ficha.peer_externo.as_deref(), Some("cliente-1"));
    assert_eq!(ficha.aberta_em, conversa.criado_em);
    assert_eq!(ficha.fechada_em, None);
    assert_eq!(ficha.fechada_por, None);
    assert_eq!(ficha.versao, 1);

    // A linha do tempo começa com a abertura do atendimento.
    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    assert_eq!(eventos.len(), 1);
    assert_eq!(eventos[0].tipo, TipoEventoFicha::FichaAberta);
    assert_eq!(eventos[0].ator_tipo, AtorEvento::Sistema);
    assert_eq!(eventos[0].seq, 1);
}

#[test]
fn recepcao_do_canal_cria_uma_ficha_por_conversa_e_nao_por_mensagem() {
    let cenario = Cenario::novo("canal");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let integracao = cenario.integracao(&org, &agente);

    let primeira = cenario
        .store()
        .caixa_entrada()
        .receber_do_canal(org.id, entrada(integracao, agente.id, "MSG-1"))
        .expect("recebe a primeira");
    assert!(matches!(primeira.situacao, Recebido::Novo));
    let conversa = primeira.mensagem.conversation_id;

    let segunda = cenario
        .store()
        .caixa_entrada()
        .receber_do_canal(org.id, entrada(integracao, agente.id, "MSG-2"))
        .expect("recebe a segunda");
    assert_eq!(
        segunda.mensagem.conversation_id, conversa,
        "a segunda mensagem entra na mesma conversa"
    );

    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa)
        .expect("consulta")
        .expect("a conversa do canal tem Ficha");
    assert_eq!(ficha.canal, "whatsapp");
    assert_eq!(ficha.peer_externo.as_deref(), Some("5521999990000"));
    assert_eq!(ficha.agent_id, agente.id);

    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    assert_eq!(
        eventos
            .iter()
            .filter(|evento| evento.tipo == TipoEventoFicha::FichaAberta)
            .count(),
        1,
        "uma Ficha tem exatamente um evento de abertura"
    );

    // E a conversa não ganhou uma segunda Ficha por fora.
    let todas = cenario
        .store()
        .fichas()
        .listar(org.id, &Default::default(), Pagina::default())
        .expect("lista");
    assert_eq!(todas.len(), 1);
}

#[test]
fn conversa_criada_pelo_repositorio_e_a_do_canal_convivem_cada_uma_com_sua_ficha() {
    let cenario = Cenario::novo("dois-caminhos");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let integracao = cenario.integracao(&org, &agente);

    let do_repositorio = cenario.conversa(&org, &agente);
    let do_canal = cenario
        .store()
        .caixa_entrada()
        .receber_do_canal(org.id, entrada(integracao, agente.id, "MSG-1"))
        .expect("recebe")
        .mensagem
        .conversation_id;

    for conversa in [do_repositorio.id, do_canal] {
        assert!(
            cenario
                .store()
                .fichas()
                .por_conversa(org.id, conversa)
                .expect("consulta")
                .is_some(),
            "toda conversa criada pelo sistema tem Ficha"
        );
    }
    assert_eq!(
        cenario
            .store()
            .fichas()
            .listar(org.id, &Default::default(), Pagina::default())
            .expect("lista")
            .len(),
        2
    );
}

// ─────────────────────────── Fechamento: fecha a Ficha junto ───────────────────────────

#[test]
fn fechar_conversa_fecha_a_ficha_e_registra_o_evento() {
    let cenario = Cenario::novo("fechar");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");

    let fechada = cenario.fechar(&org, &conversa);
    assert_eq!(fechada.status, StatusConversa::Fechada);

    let depois = cenario
        .store()
        .fichas()
        .obter(org.id, ficha.id)
        .expect("Ficha fechada continua legível");
    assert_eq!(depois.status, StatusFicha::Fechada);
    assert_eq!(depois.fechada_por, Some(FechadaPor::Admin));
    assert_eq!(depois.fechada_em, Some(fechada.atualizado_em));
    assert!(depois.fechada_motivo.is_some());
    assert_eq!(depois.versao, 2, "fechar a Ficha sobe a versão dela");

    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    let ultimo = eventos.last().expect("tem evento de fechamento");
    assert_eq!(ultimo.tipo, TipoEventoFicha::FichaFechada);
    assert_eq!(ultimo.ator_tipo, AtorEvento::Admin);
    assert_eq!(
        ultimo.seq, 2,
        "o fechamento é o segundo evento da linha do tempo"
    );
}

#[test]
fn mudar_status_para_fechada_tambem_fecha_a_ficha() {
    let cenario = Cenario::novo("mudar-status");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);

    cenario
        .store()
        .conversas()
        .mudar_status(
            org.id,
            conversa.id,
            conversa.versao,
            StatusConversa::Fechada,
            None,
        )
        .expect("fecha pela mudança de situação");

    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");
    assert_eq!(ficha.status, StatusFicha::Fechada);
    assert_eq!(ficha.fechada_por, Some(FechadaPor::Sistema));
}

#[test]
fn fechar_com_execucao_ativa_abandona_a_execucao_na_mesma_escrita() {
    let cenario = Cenario::novo("abandona");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");
    let execucao = cenario.execucao_ativa(&org, ficha.id, cenario.store().agora());

    cenario.fechar(&org, &conversa);

    let conn = cenario.conexao();
    let (status, motivo, finalizada): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT status, motivo_fim, finalizada_em FROM fluxo_execucoes WHERE id = ?1",
            rusqlite::params![execucao],
            |linha| Ok((linha.get(0)?, linha.get(1)?, linha.get(2)?)),
        )
        .expect("execução");
    assert_eq!(status, "abandonada");
    assert_eq!(motivo.as_deref(), Some(MOTIVO_ABANDONO_ATENDIMENTO));
    assert!(
        finalizada.is_some(),
        "execução encerrada tem instante de fim"
    );

    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    let abandonos: Vec<_> = eventos
        .iter()
        .filter(|evento| evento.tipo == TipoEventoFicha::ExecucaoAbandonada)
        .collect();
    assert_eq!(abandonos.len(), 1);
    assert_eq!(
        abandonos[0].execucao_id.map(|id| id.to_string()),
        Some(execucao)
    );
    assert_eq!(
        eventos.last().map(|evento| evento.tipo),
        Some(TipoEventoFicha::FichaFechada),
        "o abandono vem antes do fechamento na linha do tempo"
    );
}

#[test]
fn falha_no_meio_do_fechamento_nao_deixa_conversa_fechada_com_ficha_aberta() {
    let cenario = Cenario::novo("atomico");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");

    // Falha injetada no último passo do fechamento: o evento de fechamento da Ficha não entra.
    // Se a transação não fosse atômica, a conversa ficaria fechada com a Ficha aberta.
    {
        let conn = cenario.conexao();
        conn.execute_batch(
            "CREATE TRIGGER falha_injetada BEFORE INSERT ON ficha_eventos
             WHEN NEW.tipo = 'ficha_fechada'
             BEGIN SELECT RAISE(ABORT, 'falha injetada no teste'); END;",
        )
        .expect("cria o gatilho da falha");
    }

    let erro = cenario
        .store()
        .conversas()
        .fechar_auditado(org.id, conversa.id, conversa.versao, "token".to_owned())
        .expect_err("o fechamento falha no meio");
    assert!(
        format!("{erro}").contains("falha injetada"),
        "o erro do gatilho aparece: {erro}"
    );

    cenario
        .conexao()
        .execute_batch("DROP TRIGGER falha_injetada;")
        .expect("remove o gatilho");

    let conversa_depois = cenario
        .store()
        .conversas()
        .obter(org.id, conversa.id)
        .expect("lê a conversa");
    assert_eq!(
        conversa_depois.status,
        StatusConversa::Aberta,
        "a conversa não pode ficar fechada se a Ficha não fechou"
    );
    assert_eq!(
        conversa_depois.versao, conversa.versao,
        "nem a versão subiu"
    );

    let ficha_depois = cenario
        .store()
        .fichas()
        .obter(org.id, ficha.id)
        .expect("lê a Ficha");
    assert_eq!(ficha_depois.status, StatusFicha::Aberta);
    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    assert!(
        eventos
            .iter()
            .all(|evento| evento.tipo != TipoEventoFicha::FichaFechada),
        "nenhum evento parcial do fechamento sobrou"
    );
}

// ─────────────────────── Ciclo: mensagem nova depois do fechamento ───────────────────────

#[test]
fn mensagem_depois_do_fechamento_abre_conversa_e_ficha_novas() {
    let cenario = Cenario::novo("ciclo");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let integracao = cenario.integracao(&org, &agente);

    let primeira = cenario
        .store()
        .caixa_entrada()
        .receber_do_canal(org.id, entrada(integracao, agente.id, "MSG-1"))
        .expect("recebe");
    let conversa_1 = primeira.mensagem.conversation_id;
    let ficha_1 = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa_1)
        .expect("consulta")
        .expect("Ficha da primeira");

    let versao = cenario
        .store()
        .conversas()
        .obter(org.id, conversa_1)
        .expect("conversa")
        .versao;
    cenario
        .store()
        .conversas()
        .fechar_auditado(org.id, conversa_1, versao, "token".to_owned())
        .expect("fecha a primeira");

    let segunda = cenario
        .store()
        .caixa_entrada()
        .receber_do_canal(org.id, entrada(integracao, agente.id, "MSG-2"))
        .expect("recebe depois do fechamento");
    let conversa_2 = segunda.mensagem.conversation_id;
    assert_ne!(conversa_2, conversa_1, "mensagem nova abre conversa nova");

    let ficha_2 = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa_2)
        .expect("consulta")
        .expect("a conversa nova tem Ficha");
    assert_ne!(ficha_2.id, ficha_1.id);
    assert_eq!(ficha_2.status, StatusFicha::Aberta);

    let ficha_1_depois = cenario
        .store()
        .fichas()
        .obter(org.id, ficha_1.id)
        .expect("lê a primeira");
    assert_eq!(ficha_1_depois.status, StatusFicha::Fechada);

    // A Ficha fechada recusa evento novo (PRD L5): a ação nova vive na Ficha nova.
    let erro = cenario
        .store()
        .fichas()
        .registrar_evento(
            org.id,
            ficha_1.id,
            NovoEventoFicha {
                execucao_id: None,
                tipo: TipoEventoFicha::EtapaEntrou,
                ator_tipo: AtorEvento::Agente,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("Ficha fechada não recebe evento");
    assert!(matches!(
        erro,
        poph_store::ErroStore::Dominio(DomainError::FichaFechada { .. })
    ));
}

// ──────────────────────────────── Reabrir (PRD L13) ────────────────────────────────

#[test]
fn reabrir_conversa_fechada_e_recusado_e_transferida_continua_reabrindo() {
    let cenario = Cenario::novo("reabrir");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");

    // `transferred` reabre.
    let transferida = cenario.conversa(&org, &agente);
    cenario
        .store()
        .conversas()
        .mudar_status(
            org.id,
            transferida.id,
            transferida.versao,
            StatusConversa::Transferida,
            None,
        )
        .expect("transfere");
    let versao = cenario
        .store()
        .conversas()
        .obter(org.id, transferida.id)
        .expect("conversa")
        .versao;
    let reaberta = cenario
        .store()
        .conversas()
        .reabrir_auditado(org.id, transferida.id, versao, "token".to_owned())
        .expect("conversa transferida reabre");
    assert_eq!(reaberta.status, StatusConversa::Aberta);

    // `closed` não volta.
    let fechada = cenario.conversa(&org, &agente);
    let depois_de_fechar = cenario.fechar(&org, &fechada);
    let erro = cenario
        .store()
        .conversas()
        .reabrir_auditado(
            org.id,
            fechada.id,
            depois_de_fechar.versao,
            "token".to_owned(),
        )
        .expect_err("conversa encerrada não reabre");
    assert!(
        matches!(
            erro,
            poph_store::ErroStore::Dominio(DomainError::ConversaEncerrada { .. })
        ),
        "veio {erro:?}"
    );

    // E o caminho genérico de mudança de situação também não reabre conversa encerrada.
    let erro = cenario
        .store()
        .conversas()
        .mudar_status(
            org.id,
            fechada.id,
            depois_de_fechar.versao,
            StatusConversa::Aberta,
            None,
        )
        .expect_err("mudar_status não reabre conversa encerrada");
    assert!(matches!(
        erro,
        poph_store::ErroStore::Dominio(DomainError::ConversaEncerrada { .. })
    ));

    // A conversa e a Ficha continuam como estavam.
    let conversa_depois = cenario
        .store()
        .conversas()
        .obter(org.id, fechada.id)
        .expect("conversa");
    assert_eq!(conversa_depois.status, StatusConversa::Fechada);
    assert_eq!(conversa_depois.versao, depois_de_fechar.versao);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, fechada.id)
        .expect("consulta")
        .expect("Ficha");
    assert_eq!(ficha.status, StatusFicha::Fechada);
}

#[test]
fn reabrir_conversa_com_versao_velha_devolve_conflito_de_versao() {
    let cenario = Cenario::novo("reabrir-versao");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);

    let erro = cenario
        .store()
        .conversas()
        .reabrir_auditado(org.id, conversa.id, conversa.versao + 7, "token".to_owned())
        .expect_err("versão velha não reabre");
    assert!(matches!(
        erro,
        poph_store::ErroStore::Dominio(DomainError::ConflitoDeVersao { .. })
    ));
}

// ─────────────────────────────── Repositório de Fichas ───────────────────────────────

#[test]
fn lista_filtra_por_situacao_agente_canal_e_instante() {
    let cenario = Cenario::novo("lista");
    let org = cenario.org("a");
    let agente_a = cenario.agente(&org, "a");
    let agente_b = cenario.agente(&org, "b");

    let conversa_a = cenario.conversa(&org, &agente_a);
    let conversa_b = cenario
        .store()
        .conversas()
        .criar(
            org.id,
            NovaConversa {
                agent_id: agente_b.id,
                integration_id: None,
                canal: "whatsapp".to_owned(),
                peer_externo: Some("5521988887777".to_owned()),
                departamento_alvo: None,
            },
        )
        .expect("segunda conversa");
    cenario.fechar(&org, &conversa_a);

    let aberta: Vec<_> = cenario
        .store()
        .fichas()
        .listar(
            org.id,
            &poph_store::FiltroFichas {
                status: Some(StatusFicha::Aberta),
                ..Default::default()
            },
            Pagina::default(),
        )
        .expect("lista abertas");
    assert_eq!(aberta.len(), 1);
    assert_eq!(
        aberta[0].conversation_id.map(|id| id.to_string()),
        Some(conversa_b.id.to_string())
    );

    let do_agente: Vec<_> = cenario
        .store()
        .fichas()
        .listar(
            org.id,
            &poph_store::FiltroFichas {
                agente: Some(agente_b.id),
                ..Default::default()
            },
            Pagina::default(),
        )
        .expect("lista do agente");
    assert_eq!(do_agente.len(), 1);
    assert_eq!(do_agente[0].agent_id, agente_b.id);

    let do_canal: Vec<_> = cenario
        .store()
        .fichas()
        .listar(
            org.id,
            &poph_store::FiltroFichas {
                canal: Some("console".to_owned()),
                ..Default::default()
            },
            Pagina::default(),
        )
        .expect("lista do canal");
    assert_eq!(do_canal.len(), 1);
    assert_eq!(do_canal[0].canal, "console");

    let depois_de_tudo = cenario.store().agora() + 1;
    let nenhuma = cenario
        .store()
        .fichas()
        .listar(
            org.id,
            &poph_store::FiltroFichas {
                desde: Some(depois_de_tudo),
                ..Default::default()
            },
            Pagina::default(),
        )
        .expect("lista desde o futuro");
    assert!(nenhuma.is_empty());

    // Paginação por cursor: uma por página, sem repetir.
    let primeira_pagina = cenario
        .store()
        .fichas()
        .listar(org.id, &Default::default(), Pagina::com_limite(1))
        .expect("primeira página");
    assert_eq!(primeira_pagina.len(), 1);
    let segunda_pagina = cenario
        .store()
        .fichas()
        .listar(
            org.id,
            &Default::default(),
            Pagina {
                apos: Some(primeira_pagina[0].id.as_uuid()),
                limite: 1,
            },
        )
        .expect("segunda página");
    assert_eq!(segunda_pagina.len(), 1);
    assert_ne!(segunda_pagina[0].id, primeira_pagina[0].id);
}

#[test]
fn ficha_de_outra_org_nao_aparece_nem_aceita_evento() {
    let cenario = Cenario::novo("isolamento");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let agente_a = cenario.agente(&org_a, "a");
    let agente_b = cenario.agente(&org_b, "b");
    let conversa_a = cenario.conversa(&org_a, &agente_a);
    cenario.conversa(&org_b, &agente_b);
    let ficha_a = cenario
        .store()
        .fichas()
        .por_conversa(org_a.id, conversa_a.id)
        .expect("consulta")
        .expect("Ficha A");

    let erro = cenario
        .store()
        .fichas()
        .obter(org_b.id, ficha_a.id)
        .expect_err("Ficha de outra org não é legível");
    assert!(
        matches!(
            erro,
            poph_store::ErroStore::Dominio(DomainError::NaoEncontrado { .. })
        ),
        "veio {erro:?}"
    );

    let erro = cenario
        .store()
        .fichas()
        .registrar_evento(
            org_b.id,
            ficha_a.id,
            NovoEventoFicha {
                execucao_id: None,
                tipo: TipoEventoFicha::EtapaEntrou,
                ator_tipo: AtorEvento::Agente,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("evento em Ficha de outra org é recusado");
    assert!(matches!(
        erro,
        poph_store::ErroStore::Dominio(DomainError::NaoEncontrado { .. })
    ));

    let da_org_b: Vec<_> = cenario
        .store()
        .fichas()
        .listar(org_b.id, &Default::default(), Pagina::default())
        .expect("lista da org B");
    assert_eq!(da_org_b.len(), 1);
    assert!(
        da_org_b.iter().all(|ficha| ficha.id != ficha_a.id),
        "a lista não cruza orgs"
    );

    let erro = cenario
        .store()
        .fichas()
        .eventos(org_b.id, ficha_a.id, None, 50)
        .expect_err("linha do tempo de outra org não é legível");
    assert!(matches!(
        erro,
        poph_store::ErroStore::Dominio(DomainError::NaoEncontrado { .. })
    ));
}

#[test]
fn eventos_saem_em_ordem_e_paginam_pelo_seq() {
    let cenario = Cenario::novo("eventos");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");

    for tipo in [
        TipoEventoFicha::FluxoIniciado,
        TipoEventoFicha::EtapaEntrou,
        TipoEventoFicha::CamposPreenchidos,
    ] {
        cenario
            .store()
            .fichas()
            .registrar_evento(
                org.id,
                ficha.id,
                NovoEventoFicha {
                    execucao_id: None,
                    tipo,
                    ator_tipo: AtorEvento::Agente,
                    ator_ref: Some("agente-1"),
                    payload_json: "{}".to_owned(),
                    chave_idempotencia: None,
                },
            )
            .expect("registra evento");
    }

    let todos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    let seqs: Vec<i64> = todos.iter().map(|evento| evento.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4], "abertura + três eventos, em ordem");

    let segunda_pagina = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, Some(2), 50)
        .expect("eventos depois do seq 2");
    assert_eq!(
        segunda_pagina
            .iter()
            .map(|evento| evento.tipo)
            .collect::<Vec<_>>(),
        vec![
            TipoEventoFicha::CamposPreenchidos,
            TipoEventoFicha::EtapaEntrou,
        ]
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
    );

    let com_limite = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 2)
        .expect("eventos com limite");
    assert_eq!(com_limite.len(), 2);
}

#[test]
fn evento_com_a_mesma_chave_nao_duplica() {
    let cenario = Cenario::novo("idempotencia");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");

    let novo = || NovoEventoFicha {
        execucao_id: None,
        tipo: TipoEventoFicha::AcaoExecutada,
        ator_tipo: AtorEvento::Atendente,
        ator_ref: Some("@atendente:matrix"),
        payload_json: "{\"acao\":\"informar_endereco\"}".to_owned(),
        chave_idempotencia: Some("acao-1"),
    };
    let primeiro = cenario
        .store()
        .fichas()
        .registrar_evento(org.id, ficha.id, novo())
        .expect("primeiro evento");
    let segundo = cenario
        .store()
        .fichas()
        .registrar_evento(org.id, ficha.id, novo())
        .expect("repetição devolve o mesmo evento");

    assert_eq!(primeiro.id, segundo.id);
    assert_eq!(primeiro.seq, segundo.seq);
    assert_eq!(segundo.ator_ref.as_deref(), Some("@atendente:matrix"));

    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    assert_eq!(eventos.len(), 2, "abertura + uma ação, sem duplicata");
}

#[test]
fn evento_de_execucao_de_outra_org_e_recusado() {
    let cenario = Cenario::novo("evento-execucao");
    let org_a = cenario.org("a");
    let org_b = cenario.org("b");
    let agente_a = cenario.agente(&org_a, "a");
    let agente_b = cenario.agente(&org_b, "b");
    let conversa_b = cenario.conversa(&org_b, &agente_b);
    let ficha_b = cenario
        .store()
        .fichas()
        .por_conversa(org_b.id, conversa_b.id)
        .expect("consulta")
        .expect("Ficha B");
    let execucao_de_b = cenario.execucao_ativa(&org_b, ficha_b.id, cenario.store().agora());
    let execucao_de_b: poph_types::ExecucaoId = execucao_de_b.parse().expect("id");

    let conversa_a = cenario.conversa(&org_a, &agente_a);
    let ficha_a = cenario
        .store()
        .fichas()
        .por_conversa(org_a.id, conversa_a.id)
        .expect("consulta")
        .expect("Ficha A");

    let erro = cenario
        .store()
        .fichas()
        .registrar_evento(
            org_a.id,
            ficha_a.id,
            NovoEventoFicha {
                execucao_id: Some(execucao_de_b),
                tipo: TipoEventoFicha::EtapaEntrou,
                ator_tipo: AtorEvento::Agente,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("execução de outra org não entra no evento");
    assert!(
        matches!(
            erro,
            poph_store::ErroStore::Dominio(DomainError::NaoEncontrado { .. })
        ),
        "veio {erro:?}"
    );
}

#[test]
fn ficha_criada_pelo_repositorio_nasce_aberta_e_sem_conversa() {
    let cenario = Cenario::novo("ficha-solta");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");

    let ficha = cenario
        .store()
        .fichas()
        .criar(
            org.id,
            NovaFicha {
                conversation_id: None,
                agent_id: agente.id,
                canal: "whatsapp".to_owned(),
                peer_externo: None,
            },
        )
        .expect("cria a Ficha sem conversa");
    assert_eq!(ficha.status, StatusFicha::Aberta);
    assert_eq!(ficha.conversation_id, None);
    assert!(
        cenario
            .store()
            .fichas()
            .por_conversa(org.id, ConversationId::generate())
            .expect("consulta")
            .is_none()
    );
}

#[test]
fn ficha_sobrevive_a_conversa_apagada() {
    let cenario = Cenario::novo("retencao");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");

    // A retenção apaga conversas antigas; a Ficha fica como registro do atendimento (PRD L12).
    cenario
        .conexao()
        .execute(
            "DELETE FROM conversations WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), conversa.id.to_string()],
        )
        .expect("apaga a conversa");

    let depois = cenario
        .store()
        .fichas()
        .obter(org.id, ficha.id)
        .expect("a Ficha continua legível");
    assert_eq!(depois.conversation_id, None);
    assert_eq!(depois.agent_id, agente.id, "o retrato do atendimento fica");

    let lista = cenario
        .store()
        .fichas()
        .listar(org.id, &Default::default(), Pagina::default())
        .expect("lista");
    assert_eq!(lista.len(), 1, "Ficha órfã continua na lista");
}

/// O id do tipo da Ficha precisa continuar sendo UUID v7, como todos os ids do banco.
#[test]
fn id_da_ficha_e_uuid_v7() {
    let cenario = Cenario::novo("id");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");

    let texto = ficha.id.to_string();
    let uuid = uuid::Uuid::parse_str(&texto).expect("id é UUID");
    assert_eq!(uuid.get_version_num(), 7);
    let _: OrgId = org.id;
    let _: AgentId = agente.id;
}

#[test]
fn evento_de_execucao_de_outra_ficha_da_mesma_org_e_recusado() {
    let cenario = Cenario::novo("evento-execucao-ficha");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");

    // Ficha com execução ativa.
    let conversa_1 = cenario.conversa(&org, &agente);
    let ficha_1 = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa_1.id)
        .expect("consulta")
        .expect("Ficha 1");
    let execucao = cenario.execucao_ativa(&org, ficha_1.id, cenario.store().agora());
    let execucao: poph_types::ExecucaoId = execucao.parse().expect("id");

    // Outra Ficha da mesma org.
    let conversa_2 = cenario.conversa(&org, &agente);
    let ficha_2 = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa_2.id)
        .expect("consulta")
        .expect("Ficha 2");

    let mesmo_atendimento = cenario
        .store()
        .fichas()
        .registrar_evento(
            org.id,
            ficha_1.id,
            NovoEventoFicha {
                execucao_id: Some(execucao),
                tipo: TipoEventoFicha::EtapaEntrou,
                ator_tipo: AtorEvento::Agente,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect("evento na própria Ficha da execução");
    assert_eq!(
        mesmo_atendimento.execucao_id.map(|id| id.to_string()),
        Some(execucao.to_string())
    );

    let erro = cenario
        .store()
        .fichas()
        .registrar_evento(
            org.id,
            ficha_2.id,
            NovoEventoFicha {
                execucao_id: Some(execucao),
                tipo: TipoEventoFicha::EtapaEntrou,
                ator_tipo: AtorEvento::Agente,
                ator_ref: None,
                payload_json: "{}".to_owned(),
                chave_idempotencia: None,
            },
        )
        .expect_err("execução de outro atendimento não entra no evento");
    assert!(
        matches!(
            erro,
            poph_store::ErroStore::Dominio(DomainError::NaoEncontrado { .. })
        ),
        "veio {erro:?}"
    );
}

#[test]
fn conversa_sem_ficha_que_e_fechada_ganha_ficha_fechada() {
    // Caminho de exceção: conversa que entrou no banco por fora do aplicativo (importação manual,
    // banco antigo que escapou do backfill da 0012). Fechá-la **cria** a Ficha já fechada, em vez de
    // deixar a conversa sem registro.
    let cenario = Cenario::novo("sem-ficha");
    let org = cenario.org("a");
    let agente = cenario.agente(&org, "a");
    let conversa = cenario.conversa(&org, &agente);
    let ficha_original = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("Ficha");

    // Tira a Ficha do caminho, como se a conversa tivesse vindo de fora.
    cenario
        .conexao()
        .execute(
            "DELETE FROM fichas WHERE org_id = ?1 AND id = ?2",
            rusqlite::params![org.id.to_string(), ficha_original.id.to_string()],
        )
        .expect("remove a Ficha");

    let fechada = cenario.fechar(&org, &conversa);
    assert_eq!(fechada.status, StatusConversa::Fechada);

    let ficha = cenario
        .store()
        .fichas()
        .por_conversa(org.id, conversa.id)
        .expect("consulta")
        .expect("a conversa fechada ganhou Ficha");
    assert_eq!(ficha.status, StatusFicha::Fechada);
    assert_eq!(ficha.fechada_por, Some(FechadaPor::Admin));
    assert_eq!(ficha.aberta_em, conversa.criado_em);

    let eventos = cenario
        .store()
        .fichas()
        .eventos(org.id, ficha.id, None, 50)
        .expect("eventos");
    assert_eq!(
        eventos.last().map(|evento| evento.tipo),
        Some(TipoEventoFicha::FichaFechada)
    );
}
