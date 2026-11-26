//! Latência da operação atômica de transição e do caminho de escrita que ganhou a Ficha (FX-36).
//!
//! Mede três coisas, com números que vão para a seção "Medidas" de `docs/FLUXOS.md`:
//!
//! 1. `aplicar_transicao` na etapa, sem avançar (estado + dois eventos + um efeito na mesma
//!    escrita) — é o caminho de cada ação do agente ou do atendente;
//! 2. `aplicar_transicao` avançando de etapa (estado + três eventos + a linha da nova etapa);
//! 3. `aplicar_transicao` no `fim` que **encerra o atendimento** (a transição mais cara: fecha
//!    conversa e Ficha na mesma escrita);
//! 4. `Conversas::criar`, que desde a FX-07 cria a Ficha na mesma escrita — o custo que o módulo
//!    acrescentou ao caminho de escrita de conversa, e **não** ao de mensagem.
//!
//! Rodar assim (medição, não regressão):
//!
//! ```text
//! cargo test --release -p poph-store --test transicao_latencia -- --ignored --nocapture
//! ```
//!
//! A vazão de escrita de mensagens é medida pelo `--test vazao` (que já existe); aqui a pergunta é
//! outra: quanto custa a transição de fluxo e a criação de conversa com Ficha.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use poph_store::{
    EfeitoDaTransicao, EstadoDaExecucao, EventoDaTransicao, ExecucaoGravada, ExtrasDaTransicao,
    InicioDeExecucao, NovaConversa, NovaOrg, NovoAgente, Org, Store, TransicaoDeFluxo,
};
use poph_types::{
    ActionActor, EventActor, ExecutionStatus, FlowId, RecordEventKind, RecordId, RevisionId,
};
use rusqlite::Connection;

/// Banco de verdade numa pasta temporária, apagada no fim.
struct Cenario {
    pasta: PathBuf,
    store: Store,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-transicao-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(&pasta).expect("cria a pasta do teste");
        let store = Store::abrir_em(&pasta.join("harness.db"), &pasta.join("rag.db"))
            .expect("abre o banco");
        Self { pasta, store }
    }

    fn conexao(&self) -> Connection {
        let conn = Connection::open(self.pasta.join("harness.db")).expect("abre para conferir");
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .expect("pragmas");
        conn
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

    fn agente(&self, org: &Org) -> poph_store::Agente {
        self.store
            .agentes()
            .criar(
                org.id,
                NovoAgente {
                    nome: "Agente de medição".to_owned(),
                    slug: "medicao".to_owned(),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
    }

    /// Conversa (que já nasce com a Ficha) e o id da Ficha.
    fn ficha(
        &self,
        org: &Org,
        agente: &poph_store::Agente,
        peer: &str,
    ) -> (RecordId, poph_types::ConversationId) {
        let conversa = self
            .store
            .conversas()
            .criar(
                org.id,
                NovaConversa {
                    agent_id: agente.id,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some(peer.to_owned()),
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa");
        let ficha = self
            .store
            .fichas()
            .por_conversa(org.id, conversa.id)
            .expect("consulta")
            .expect("a conversa tem Ficha")
            .id;
        (ficha, conversa.id)
    }

    /// Fluxo publicado (revisão 1) gravado direto: a publicação é a FX-09.
    fn fluxo(&self, org: &Org, slug: &str) -> (FlowId, RevisionId) {
        let conn = self.conexao();
        let fluxo: FlowId = uuid::Uuid::now_v7().to_string().parse().expect("fluxo");
        let revisao: RevisionId = uuid::Uuid::now_v7().to_string().parse().expect("revisão");
        conn.execute(
            "INSERT INTO fluxos (id, org_id, slug, nome, descricao, status, rascunho_fonte,
                                 revisao_atual, version)
             VALUES (?1, ?2, ?3, ?4, '', 'active', '', 1, 1)",
            rusqlite::params![
                fluxo.to_string(),
                org.id.to_string(),
                slug,
                format!("Fluxo {slug}")
            ],
        )
        .expect("fluxo");
        conn.execute(
            "INSERT INTO fluxo_revisoes (id, org_id, fluxo_id, numero, fonte, fonte_sha256,
                                        definicao_json, schema_versao, publicada_em, publicada_por,
                                        resolucoes_json)
             VALUES (?1, ?2, ?3, 1, 'fluxo()', 'sha', '{}', 1, 1, 'admin_token', '{}')",
            rusqlite::params![revisao.to_string(), org.id.to_string(), fluxo.to_string()],
        )
        .expect("revisão");
        (fluxo, revisao)
    }

    fn abrir(
        &self,
        org: &Org,
        agente: &poph_store::Agente,
        ficha: RecordId,
        (fluxo, revisao): (FlowId, RevisionId),
    ) -> ExecucaoGravada {
        self.store
            .fluxos()
            .liberar(org.id, agente.id, fluxo)
            .expect("libera o fluxo");
        self.store
            .execucoes()
            .iniciar_execucao(
                org.id,
                InicioDeExecucao {
                    chave_idempotencia: None,
                    ficha,
                    fluxo,
                    revisao,
                    ator: EventActor::Agent,
                    ator_ref: Some("agente-de-teste"),
                    decisao_jev: None,
                    estado_inicial: estado_inicial("coleta"),
                    efeitos_de_entrada: Vec::new(),
                },
            )
            .expect("abre a execução")
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn estado_inicial(etapa: &str) -> EstadoDaExecucao {
    EstadoDaExecucao {
        status: ExecutionStatus::Active,
        no_atual: etapa.to_owned(),
        dados_json: "{}".to_owned(),
        atores_que_agiram: Vec::new(),
        resultado: None,
        motivo: None,
    }
}

/// Transição que fica na etapa: dois eventos e um efeito (o caso do agente que ainda espera).
fn transicao_parcial(etapa: &str) -> TransicaoDeFluxo {
    TransicaoDeFluxo {
        novo_estado: EstadoDaExecucao {
            status: ExecutionStatus::Active,
            no_atual: etapa.to_owned(),
            dados_json: format!("{{\"{etapa}\":{{\"endereco\":\"Rua das Flores, 10\"}}}}"),
            atores_que_agiram: vec![ActionActor::Agent],
            resultado: None,
            motivo: None,
        },
        eventos: vec![
            EventoDaTransicao {
                tipo: RecordEventKind::FieldsFilled,
                ator_tipo: EventActor::Agent,
                payload_json: format!(
                    "{{\"etapa\":\"{etapa}\",\"campos\":{{\"endereco\":\"x\"}}}}"
                ),
            },
            EventoDaTransicao {
                tipo: RecordEventKind::ActionExecuted,
                ator_tipo: EventActor::Agent,
                payload_json: format!(
                    "{{\"etapa\":\"{etapa}\",\"acao\":\"registrar\",\"parcial\":true}}"
                ),
            },
        ],
        efeitos: vec![EfeitoDaTransicao {
            chave: format!("acao:{etapa}.registrar/0"),
            tipo: "mensagem_ao_cliente".to_owned(),
            payload_json: "{\"texto\":\"Recebi seu endereço\"}".to_owned(),
        }],
        encerra_atendimento: false,
        repetida: false,
        erro_de_regra: false,
    }
}

/// Transição que conclui a etapa e entra na seguinte: três eventos e a linha da etapa nova.
fn transicao_avanca(de: &str, para: &str) -> TransicaoDeFluxo {
    TransicaoDeFluxo {
        novo_estado: EstadoDaExecucao {
            status: ExecutionStatus::Active,
            no_atual: para.to_owned(),
            dados_json: format!(
                "{{\"{de}\":{{\"endereco\":\"Rua das Flores, 10\"}},\"{para}\":{{}}}}"
            ),
            atores_que_agiram: Vec::new(),
            resultado: None,
            motivo: None,
        },
        eventos: vec![
            EventoDaTransicao {
                tipo: RecordEventKind::ActionExecuted,
                ator_tipo: EventActor::Attendant,
                payload_json: format!("{{\"etapa\":\"{de}\",\"acao\":\"confirmar\"}}"),
            },
            EventoDaTransicao {
                tipo: RecordEventKind::StepCompleted,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{de}\",\"acao\":\"confirmar\"}}"),
            },
            EventoDaTransicao {
                tipo: RecordEventKind::StepEntered,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{para}\"}}"),
            },
        ],
        efeitos: Vec::new(),
        encerra_atendimento: false,
        repetida: false,
        erro_de_regra: false,
    }
}

/// Transição do `fim` que pede o encerramento do atendimento.
fn transicao_encerra(de: &str) -> TransicaoDeFluxo {
    TransicaoDeFluxo {
        novo_estado: EstadoDaExecucao {
            status: ExecutionStatus::Completed,
            no_atual: "concluida".to_owned(),
            dados_json: format!("{{\"{de}\":{{\"endereco\":\"Rua das Flores, 10\"}}}}"),
            atores_que_agiram: Vec::new(),
            resultado: Some("Visita agendada".to_owned()),
            motivo: None,
        },
        eventos: vec![
            EventoDaTransicao {
                tipo: RecordEventKind::StepCompleted,
                ator_tipo: EventActor::System,
                payload_json: format!("{{\"etapa\":\"{de}\"}}"),
            },
            EventoDaTransicao {
                tipo: RecordEventKind::ExecutionCompleted,
                ator_tipo: EventActor::System,
                payload_json: "{\"resultado\":\"Visita agendada\"}".to_owned(),
            },
        ],
        efeitos: Vec::new(),
        encerra_atendimento: true,
        repetida: false,
        erro_de_regra: false,
    }
}

fn percentil(ordenadas: &[u128], p: f64) -> u128 {
    if ordenadas.is_empty() {
        return 0;
    }
    let indice = ((ordenadas.len() as f64 - 1.0) * p).round() as usize;
    ordenadas[indice]
}

/// Imprime p50/p95/p99 e a vazão da amostra, com o rótulo do cenário.
fn relatar(rotulo: &str, mut micros: Vec<u128>) {
    let amostras = micros.len();
    let total: u128 = micros.iter().sum();
    micros.sort_unstable();
    let media = total as f64 / amostras as f64;
    let por_segundo = if total == 0 {
        0.0
    } else {
        amostras as f64 * 1_000_000.0 / total as f64
    };
    println!(
        "{rotulo}: {amostras} amostras, p50 {} µs, p95 {} µs, p99 {} µs, média {media:.0} µs ({por_segundo:.0}/s)",
        percentil(&micros, 0.50),
        percentil(&micros, 0.95),
        percentil(&micros, 0.99),
    );
}

#[test]
#[ignore = "medição de latência: rodar à mão com --ignored --nocapture"]
fn latencia_da_transicao_e_da_conversa_com_ficha() {
    let cenario = Cenario::novo("medicao");
    let org = cenario.org("medicao");
    let agente = cenario.agente(&org);
    let fluxo = cenario.fluxo(&org, "medicao");

    // 1. Na etapa, sem avançar: estado + dois eventos + um efeito na mesma escrita.
    {
        let (ficha, _) = cenario.ficha(&org, &agente, "cliente-parcial");
        let aberta = cenario.abrir(&org, &agente, ficha, fluxo);
        let mut versao = aberta.execucao.versao;
        let mut amostras = Vec::with_capacity(500);
        for indice in 0..500u32 {
            let inicio = Instant::now();
            let gravada = cenario
                .store
                .execucoes()
                .aplicar_transicao(
                    org.id,
                    aberta.execucao.id,
                    versao,
                    &transicao_parcial("coleta"),
                    &format!("medicao-parcial-{indice}"),
                    ExtrasDaTransicao::default(),
                )
                .expect("aplica a parcial");
            amostras.push(inicio.elapsed().as_micros());
            versao = gravada.execucao.versao;
        }
        relatar(
            "transição na etapa (estado + 2 eventos + 1 efeito)",
            amostras,
        );
    }

    // 2. Avançando de etapa: estado + três eventos + a linha da etapa nova.
    {
        let (ficha, _) = cenario.ficha(&org, &agente, "cliente-avanca");
        let aberta = cenario.abrir(&org, &agente, ficha, fluxo);
        let mut versao = aberta.execucao.versao;
        let mut amostras = Vec::with_capacity(300);
        for indice in 0..300u32 {
            let (de, para) = if indice % 2 == 0 {
                ("coleta", "confirmacao")
            } else {
                ("confirmacao", "coleta")
            };
            let inicio = Instant::now();
            let gravada = cenario
                .store
                .execucoes()
                .aplicar_transicao(
                    org.id,
                    aberta.execucao.id,
                    versao,
                    &transicao_avanca(de, para),
                    &format!("medicao-avanca-{indice}"),
                    ExtrasDaTransicao::default(),
                )
                .expect("aplica o avanço");
            amostras.push(inicio.elapsed().as_micros());
            versao = gravada.execucao.versao;
        }
        relatar(
            "transição avançando de etapa (estado + 3 eventos + etapa nova)",
            amostras,
        );
    }

    // 3. `fim` que encerra o atendimento: fecha conversa e Ficha na mesma escrita.
    {
        let mut amostras = Vec::with_capacity(100);
        for indice in 0..100u32 {
            let (ficha, _) = cenario.ficha(&org, &agente, &format!("cliente-fim-{indice}"));
            let aberta = cenario.abrir(&org, &agente, ficha, fluxo);
            let inicio = Instant::now();
            let gravada = cenario
                .store
                .execucoes()
                .aplicar_transicao(
                    org.id,
                    aberta.execucao.id,
                    aberta.execucao.versao,
                    &transicao_encerra("coleta"),
                    &format!("medicao-fim-{indice}"),
                    ExtrasDaTransicao::default(),
                )
                .expect("aplica o fim");
            amostras.push(inicio.elapsed().as_micros());
            assert!(
                gravada.fechou_atendimento,
                "o fim de teste encerra o atendimento"
            );
        }
        relatar(
            "transição do fim que encerra o atendimento (fecha conversa e Ficha)",
            amostras,
        );
    }

    // 4. Criação de conversa, que desde a FX-07 cria a Ficha na mesma escrita.
    {
        let mut amostras = Vec::with_capacity(200);
        for indice in 0..200u32 {
            let inicio = Instant::now();
            cenario
                .store
                .conversas()
                .criar(
                    org.id,
                    NovaConversa {
                        agent_id: agente.id,
                        integration_id: None,
                        canal: "console".to_owned(),
                        peer_externo: Some(format!("cliente-criacao-{indice}")),
                        departamento_alvo: None,
                    },
                )
                .expect("cria a conversa");
            amostras.push(inicio.elapsed().as_micros());
        }
        relatar("Conversas::criar com a Ficha na mesma escrita", amostras);
    }
}
