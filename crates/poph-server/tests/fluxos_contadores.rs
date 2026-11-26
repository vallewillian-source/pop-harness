//! Contadores das execuções de fluxo (FX-40): `poph_fluxos_execucoes_*` e
//! `poph_fluxos_erros_regra_total` andando nos caminhos de escrita de verdade.
//!
//! O servidor é montado de verdade (`Estado::novo`), que publica os cinco handles no `Store` — é o
//! store quem escreve `fluxo_execucoes`. As operações usadas aqui são as mesmas que o turno (G9 e
//! reserva), as ferramentas do agente e a API de ações humanas chamam: `iniciar_execucao`,
//! `aplicar_transicao` (com o desfecho do motor), `fechar_auditado` da conversa e a exclusão do
//! fluxo com `force`. Nada de dado real: nomes, telefones e chaves são fictícios.
//!
//! O que fica provado:
//! - abertura conta uma vez, e repetir a chave de idempotência não conta de novo;
//! - conclusão, abandono e falha contam no contador da situação, nunca em dois;
//! - erro de regra conta em `poph_fluxos_erros_regra_total`, e a falha que **não** é de regra
//!   (teto de saltos) não conta lá;
//! - fechar o atendimento com execução ativa conta abandono;
//! - `/metrics` publica os números que o store contou.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use poph_fluxos::motor::{self, Campos, Contexto, Estado as EstadoMotor};
use poph_fluxos::{Avaliador, Limites, compilar};
use poph_server::Estado;
use poph_server::fluxos_transicao::transicao_do_motor;
use poph_store::metricas::MetricasExecucao;
use poph_store::{
    EstadoDaExecucao, ExtrasDaTransicao, InicioDeExecucao, NovaConversa, NovaOrg, NovoAgente,
    NovoFluxo, PublicacaoDeRevisao, Store,
};
use poph_types::{ActionActor, EventActor, ExecutionStatus, FlowId, HarnessConfig, RevisionId};

/// Fluxo simples: uma etapa do agente e um fim que conclui sem encerrar o atendimento.
const CONCLUSIVO: &str = r#"fluxo(
    id = "triagem_contadores",
    nome = "Triagem dos contadores",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente precisa de ajuda com o equipamento.",
        nao_quando = "O cliente quer apenas o preço.",
        exemplos = ["preciso de ajuda"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    campos = [campo("resumo", "Resumo", tipo = "texto")],
    acoes = [
        acao("concluir", "Concluir", ator = "agente", vai_para = fim("pronto", resultado = "Resolvido")),
    ],
)
"#;

/// Fluxo com decisão quebrada: a `def` devolve texto, então a regra **não avalia** em tempo de
/// execução (`FLXR01`). A publicação aceita (a `def` existe); a falha aparece ao executar.
const REGRA_QUEBRADA: &str = r#"fluxo(
    id = "quebra_de_regra",
    nome = "Quebra de regra",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente relata um defeito.",
        nao_quando = "O cliente quer comprar.",
        exemplos = ["deu defeito"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    acoes = [
        acao("ir", "Ir para a decisão", ator = "agente", vai_para = "escolher"),
    ],
)

decisao(
    id = "escolher",
    se = funcao("quebrada"),
    entao = "etapa_a",
    senao = "etapa_b",
)

etapa(id = "etapa_a", nome = "A", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_a"))])
etapa(id = "etapa_b", nome = "B", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_b"))])

def quebrada(dados):
    return "sim"
"#;

/// Fluxo com duas decisões encadeadas: com `max_saltos_decisao = 1` a segunda estoura o teto e a
/// execução falha **sem** ser erro de regra.
const DUAS_DECISOES: &str = r#"fluxo(
    id = "laco_de_decisoes",
    nome = "Laço de decisões",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer falar sobre a fatura.",
        nao_quando = "O cliente quer marcar visita.",
        exemplos = ["fatura"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coleta",
    campos = [campo("marca", "Marca", tipo = "texto")],
    acoes = [
        acao("ir", "Ir para a decisão", ator = "agente", vai_para = "primeira"),
    ],
)

decisao(id = "primeira", se = igual("coleta.marca", "x"), entao = "segunda", senao = "segunda")

decisao(id = "segunda", se = igual("coleta.marca", "x"), entao = "etapa_a", senao = "etapa_b")

etapa(id = "etapa_a", nome = "A", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_a"))])
etapa(id = "etapa_b", nome = "B", acoes = [acao("ok", "Ok", ator = "agente", vai_para = fim("fim_b"))])
"#;

/// Um cenário: banco temporário, org, agente e um `Estado` do servidor (que publica os contadores
/// no store).
struct Cenario {
    estado: Option<Estado>,
    org: poph_types::OrgId,
    agente: poph_types::AgentId,
    pasta: PathBuf,
}

impl Cenario {
    fn novo(rotulo: &str) -> Self {
        let pasta = std::env::temp_dir().join(format!(
            "poph-fx40-{rotulo}-{}-{}",
            std::process::id(),
            poph_types::AgentId::generate()
        ));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
        let mut config = HarnessConfig::default();
        config.data.directory = pasta.join("dados");
        let store = Arc::new(Store::abrir(&config).expect("abre o banco"));
        let estado = Estado::novo(store, config).expect("estado do servidor");
        assert!(
            estado.metricas_fluxos().is_some(),
            "o motor de fluxos ligado declara os contadores"
        );
        let org = estado
            .store
            .orgaos()
            .criar(NovaOrg {
                nome: format!("Org {rotulo}"),
                slug: format!("org-{rotulo}-{}", poph_types::AgentId::generate()),
                config_json: "{}".to_owned(),
            })
            .expect("cria a org")
            .id;
        let agente = estado
            .store
            .agentes()
            .criar(
                org,
                NovoAgente {
                    nome: format!("Bia {rotulo}"),
                    slug: format!("bia-{rotulo}"),
                    ..NovoAgente::default()
                },
            )
            .expect("cria o agente")
            .id;
        Self {
            estado: Some(estado),
            org,
            agente,
            pasta,
        }
    }

    fn estado(&self) -> &Estado {
        self.estado.as_ref().expect("o estado ainda está aqui")
    }

    /// Retira o `Estado` do cenário (o teste do `/metrics` entrega ele ao servidor).
    fn tirar_estado(&mut self) -> Estado {
        self.estado.take().expect("o estado ainda está aqui")
    }

    fn store(&self) -> &Store {
        &self.estado().store
    }

    /// Cria o fluxo, publica a revisão compilada e libera ao agente.
    fn fluxo(&self, script: &str, slug: &str) -> (FlowId, RevisionId) {
        let fluxo = self
            .store()
            .fluxos()
            .criar(
                self.org,
                NovoFluxo {
                    slug: slug.to_owned(),
                    nome: format!("Fluxo {slug}"),
                    descricao: String::new(),
                    rascunho_fonte: script.to_owned(),
                },
            )
            .expect("cria o fluxo");
        let compilado = compilar(script, &Limites::default());
        let ir = compilado
            .ir()
            .unwrap_or_else(|diagnosticos| panic!("script do teste não compila: {diagnosticos:?}"));
        let revisao = self
            .store()
            .revisoes()
            .publicar_revisao(
                self.org,
                fluxo.id,
                PublicacaoDeRevisao {
                    esperada: fluxo.versao,
                    fonte: script.to_owned(),
                    definicao_json: serde_json::to_string(ir).expect("IR em JSON"),
                    schema_versao: Some(ir.schema_versao),
                    resolucoes_json: "{}".to_owned(),
                    ator_tipo: "system".to_owned(),
                    ator_id: None,
                },
            )
            .expect("publica a revisão");
        self.store()
            .fluxos()
            .liberar(self.org, self.agente, fluxo.id)
            .expect("libera o fluxo");
        (fluxo.id, revisao.id)
    }

    /// Ficha de um atendimento novo (o caminho normal: a conversa nasce e a Ficha vem com ela).
    fn ficha(&self) -> (poph_types::ConversationId, poph_types::RecordId) {
        let conversa = self
            .store()
            .conversas()
            .criar(
                self.org,
                NovaConversa {
                    agent_id: self.agente,
                    integration_id: None,
                    canal: "console".to_owned(),
                    peer_externo: Some(format!(
                        "cliente-ficticio-{}",
                        poph_types::AgentId::generate()
                    )),
                    departamento_alvo: None,
                },
            )
            .expect("abre a conversa");
        let ficha = self
            .store()
            .fichas()
            .por_conversa(self.org, conversa.id)
            .expect("lê a Ficha")
            .expect("a conversa tem Ficha");
        (conversa.id, ficha.id)
    }

    /// Abre a execução, como o turno e a API de ações fazem.
    fn abrir(
        &self,
        ficha: poph_types::RecordId,
        fluxo: FlowId,
        revisao: RevisionId,
        chave: &str,
    ) -> poph_store::Execucao {
        self.store()
            .execucoes()
            .iniciar_execucao(
                self.org,
                InicioDeExecucao {
                    ficha,
                    fluxo,
                    revisao,
                    ator: EventActor::Agent,
                    ator_ref: Some("atendente-de-teste"),
                    decisao_jev: None,
                    estado_inicial: EstadoDaExecucao {
                        status: ExecutionStatus::Active,
                        no_atual: "coleta".to_owned(),
                        dados_json: "{}".to_owned(),
                        atores_que_agiram: Vec::new(),
                        resultado: None,
                        motivo: None,
                    },
                    efeitos_de_entrada: Vec::new(),
                    chave_idempotencia: Some(chave),
                },
            )
            .expect("abre a execução")
            .execucao
    }

    /// Roda o motor e grava o desfecho, como a API de ações e a ferramenta do agente fazem.
    fn agir(
        &self,
        script: &str,
        execucao: &poph_store::Execucao,
        acao: &str,
        saltos: Option<usize>,
        chave: &str,
    ) -> poph_store::ExecucaoGravada {
        let compilado = compilar(script, &Limites::default());
        let ir = compilado
            .ir()
            .unwrap_or_else(|diagnosticos| panic!("script do teste não compila: {diagnosticos:?}"));
        let avaliador = Avaliador::novo(script).expect("as regras do script carregam");
        let estado = estado_do_banco(execucao);
        let mut contexto = Contexto::novo(&avaliador);
        if let Some(saltos) = saltos {
            contexto = contexto.com_limite_de_saltos(saltos);
        }
        let resultado = motor::executar_acao(
            ir,
            &estado,
            acao,
            &Campos::new(),
            ActionActor::Agent,
            Some(chave),
            &contexto,
        )
        .unwrap_or_else(|erro| panic!("a passagem devia voltar desfecho: {erro}"));
        self.store()
            .execucoes()
            .aplicar_transicao(
                self.org,
                execucao.id,
                execucao.versao,
                &transicao_do_motor(&resultado),
                chave,
                ExtrasDaTransicao {
                    ator_ref: Some("atendente-de-teste"),
                    ..ExtrasDaTransicao::default()
                },
            )
            .expect("grava a transição")
    }

    /// Contadores publicados (os mesmos handles do `/metrics`).
    fn contadores(&self) -> MetricasExecucao {
        self.estado()
            .metricas_fluxos()
            .expect("motor ligado")
            .das_execucoes()
    }

    fn valores(&self) -> (u64, u64, u64, u64, u64) {
        let c = self.contadores();
        (
            c.iniciadas.value(),
            c.concluidas.value(),
            c.abandonadas.value(),
            c.falhas.value(),
            c.erros_regra.value(),
        )
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

/// A execução do banco no vocabulário puro do motor (o que a API e o turno montam antes de agir).
fn estado_do_banco(execucao: &poph_store::Execucao) -> EstadoMotor {
    let ir = {
        // O estado puro só precisa do nó e dos dados; `fluxo_id` é o slug da IR.
        &execucao.no_atual
    };
    EstadoMotor {
        fluxo_id: execucao.fluxo_id.to_string(),
        status: execucao.status,
        no_atual: poph_fluxos::ir::Destino::Etapa { id: ir.clone() },
        dados: poph_fluxos::Dados::new(),
        atores_que_agiram: Vec::new(),
        resultado: execucao.resultado.clone(),
        motivo: execucao.motivo_fim.clone(),
        chaves: Default::default(),
    }
}

#[test]
fn abertura_conta_uma_vez_e_repetir_a_chave_nao_conta_de_novo() {
    let cenario = Cenario::novo("abertura");
    let (fluxo, revisao) = cenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = cenario.ficha();

    let primeira = cenario.abrir(ficha, fluxo, revisao, "abertura-1");
    assert_eq!(
        cenario.valores().0,
        1,
        "a abertura conta uma execução iniciada"
    );

    // Repetir a chave devolve a mesma execução e não soma nada.
    let repetida = cenario
        .store()
        .execucoes()
        .iniciar_execucao(
            cenario.org,
            InicioDeExecucao {
                ficha,
                fluxo,
                revisao,
                ator: EventActor::Agent,
                ator_ref: Some("atendente-de-teste"),
                decisao_jev: None,
                estado_inicial: EstadoDaExecucao {
                    status: ExecutionStatus::Active,
                    no_atual: "coleta".to_owned(),
                    dados_json: "{}".to_owned(),
                    atores_que_agiram: Vec::new(),
                    resultado: None,
                    motivo: None,
                },
                efeitos_de_entrada: Vec::new(),
                chave_idempotencia: Some("abertura-1"),
            },
        )
        .expect("repetir a chave devolve a execução");
    assert!(repetida.repetida, "a segunda abertura é repetição");
    assert_eq!(repetida.execucao.id, primeira.id);
    assert_eq!(cenario.valores().0, 1, "repetir não soma outra iniciada");
}

#[test]
fn conclusao_conta_concluida_e_repetir_a_chave_nao_soma() {
    let cenario = Cenario::novo("conclusao");
    let (fluxo, revisao) = cenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = cenario.ficha();
    let execucao = cenario.abrir(ficha, fluxo, revisao, "conclusao-1");

    let gravada = cenario.agir(CONCLUSIVO, &execucao, "concluir", None, "acao-1");
    assert_eq!(gravada.execucao.status, ExecutionStatus::Completed);
    assert_eq!(
        cenario.valores(),
        (1, 1, 0, 0, 0),
        "iniciada e concluída, nada em abandono nem em falha"
    );

    // Repetir a chave da ação não conta de novo (nem concluída, nem falha).
    let de_novo = cenario
        .store()
        .execucoes()
        .aplicar_transicao(
            cenario.org,
            gravada.execucao.id,
            gravada.execucao.versao,
            &transicao_do_motor(&motor::Resultado {
                novo_estado: estado_do_banco(&gravada.execucao),
                eventos: Vec::new(),
                efeitos: Vec::new(),
                aguardando: Vec::new(),
                repetida: true,
                encerra_atendimento: false,
                erro_de_regra: false,
            }),
            "acao-1",
            ExtrasDaTransicao::default(),
        )
        .expect("repetir devolve o desfecho");
    assert!(de_novo.repetida, "a repetição é marcada");
    assert_eq!(cenario.valores(), (1, 1, 0, 0, 0), "nada somou");
}

#[test]
fn abandono_pelo_agente_conta_abandonada() {
    let cenario = Cenario::novo("abandono");
    let (fluxo, revisao) = cenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = cenario.ficha();
    let execucao = cenario.abrir(ficha, fluxo, revisao, "abandono-1");

    let estado = estado_do_banco(&execucao);
    let resultado = motor::abandonar(
        &estado,
        "cliente desistiu",
        ActionActor::Agent,
        Some("ab-1"),
    )
    .expect("o abandono volta com o desfecho");
    cenario
        .store()
        .execucoes()
        .aplicar_transicao(
            cenario.org,
            execucao.id,
            execucao.versao,
            &transicao_do_motor(&resultado),
            "ab-1",
            ExtrasDaTransicao::default(),
        )
        .expect("grava o abandono");

    assert_eq!(
        cenario.valores(),
        (1, 0, 1, 0, 0),
        "uma iniciada, uma abandonada"
    );
}

#[test]
fn fechar_o_atendimento_com_execucao_ativa_conta_abandono() {
    let cenario = Cenario::novo("fechamento");
    let (fluxo, revisao) = cenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (conversa, ficha) = cenario.ficha();
    cenario.abrir(ficha, fluxo, revisao, "fechamento-1");
    assert_eq!(cenario.valores().0, 1);

    // `close` da conversa: é o caminho do encerramento (G8, API e atendente), que abandona a
    // execução ativa da Ficha na mesma escrita.
    let versao = cenario
        .store()
        .conversas()
        .obter(cenario.org, conversa)
        .expect("lê a conversa")
        .versao;
    cenario
        .store()
        .conversas()
        .fechar_auditado(
            cenario.org,
            conversa,
            versao,
            "atendente-de-teste".to_owned(),
        )
        .expect("fecha o atendimento");

    assert_eq!(
        cenario.valores(),
        (1, 0, 1, 0, 0),
        "o fechamento abandonou a execução ativa"
    );
}

#[test]
fn falha_de_regra_conta_falha_e_erro_de_regra() {
    let cenario = Cenario::novo("regra");
    let (fluxo, revisao) = cenario.fluxo(REGRA_QUEBRADA, "quebra_de_regra");
    let (_, ficha) = cenario.ficha();
    let execucao = cenario.abrir(ficha, fluxo, revisao, "regra-1");

    let gravada = cenario.agir(REGRA_QUEBRADA, &execucao, "ir", None, "acao-regra");
    assert_eq!(gravada.execucao.status, ExecutionStatus::Failed);
    assert_eq!(
        cenario.valores(),
        (1, 0, 0, 1, 1),
        "a falha conta em falhas e a avaliação que não deu conta em erro de regra"
    );
}

#[test]
fn falha_que_nao_e_de_regra_nao_conta_erro_de_regra() {
    let cenario = Cenario::novo("saltos");
    let (fluxo, revisao) = cenario.fluxo(DUAS_DECISOES, "laco_de_decisoes");
    let (_, ficha) = cenario.ficha();
    let execucao = cenario.abrir(ficha, fluxo, revisao, "saltos-1");

    // Teto de uma decisão encadeada: a segunda estoura e a execução para em `falha`.
    let gravada = cenario.agir(DUAS_DECISOES, &execucao, "ir", Some(1), "acao-saltos");
    assert_eq!(gravada.execucao.status, ExecutionStatus::Failed);
    assert_eq!(
        cenario.valores(),
        (1, 0, 0, 1, 0),
        "falha conta em falhas; erro de regra fica zerado"
    );
}

#[tokio::test]
async fn metrics_publica_os_contadores_das_execucoes() {
    let mut cenario = Cenario::novo("metrics");
    let (fluxo, revisao) = cenario.fluxo(CONCLUSIVO, "triagem_contadores");
    let (_, ficha) = cenario.ficha();
    let execucao = cenario.abrir(ficha, fluxo, revisao, "metrics-1");
    cenario.agir(CONCLUSIVO, &execucao, "concluir", None, "acao-metrics");
    let (_, outra) = cenario.ficha();
    cenario.abrir(outra, fluxo, revisao, "metrics-2");
    assert_eq!(cenario.valores(), (2, 1, 0, 0, 0));

    let estado = cenario.tirar_estado();
    let texto = metricas_http(estado).await;
    let _ = fs::remove_dir_all(&cenario.pasta);
    assert!(
        texto.contains("poph_fluxos_execucoes_iniciadas_total 2\n"),
        "{texto}"
    );
    assert!(
        texto.contains("poph_fluxos_execucoes_concluidas_total 1\n"),
        "{texto}"
    );
    assert!(
        texto.contains("poph_fluxos_execucoes_abandonadas_total 0\n")
            && texto.contains("poph_fluxos_execucoes_falhas_total 0\n")
            && texto.contains("poph_fluxos_erros_regra_total 0\n"),
        "os contadores sem uso aparecem zerados: {texto}"
    );
}

/// Sobe o servidor com o estado dado e devolve o texto de `/metrics`.
async fn metricas_http(estado: Estado) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("porta efêmera");
    let endereco = listener.local_addr().expect("endereço");
    let (desligar, sinal) = tokio::sync::oneshot::channel::<()>();
    let servidor = tokio::spawn(async move {
        let _ = poph_server::servir_em(listener, estado, async {
            let _ = sinal.await;
        })
        .await;
    });
    let cliente = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("cliente HTTP");
    let mut corpo = None;
    for _ in 0..100 {
        if let Ok(resposta) = cliente
            .get(format!("http://{endereco}/metrics"))
            .send()
            .await
        {
            corpo = Some(resposta.text().await.expect("corpo do /metrics"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = desligar.send(());
    let _ = servidor.await;
    corpo.expect("o /metrics respondeu")
}
