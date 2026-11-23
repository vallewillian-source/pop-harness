//! Operação atômica de transição do fluxo e abertura de execução (FX-20, PRD §6.3, §6.4 e §7).
//!
//! O motor (`poph-fluxos`) é puro: ele diz **o que** aconteceu (estado novo, eventos, efeitos,
//! pedido de encerramento). Este repositório é quem **grava** — tudo numa escrita só, no
//! `SAVEPOINT` do escritor dedicado, que volta inteiro no primeiro erro.
//!
//! # Por que o store não recebe `poph_fluxos::Resultado`
//!
//! A tabela de camadas do `AGENTS.md` diz que `poph-store` importa `poph-types` e mais nada. Então
//! o desfecho do motor entra aqui como um **espelho** no vocabulário das colunas
//! ([`TransicaoDeFluxo`] com [`EstadoDaExecucao`], [`EventoDaTransicao`] e
//! [`EfeitoDaTransicao`]), e quem traduz é o crate que tem os dois (`poph-agent`/`poph-server`).
//! Nenhuma decisão do motor é refeita aqui: o store grava o que recebe, confere coerência e é
//! atômico.
//!
//! # O que uma transição grava (uma escrita)
//!
//! 1. **`fluxo_execucoes`** — `status`, `no_atual`, `dados_json`, `resultado`, `motivo_fim`,
//!    `finalizada_em` (só quando a execução deixa de ser `ativa`) e `version + 1`, conferindo a
//!    `version` esperada: versão velha é [`DomainError::ConflitoDeVersao`] (`412`).
//! 2. **`fluxo_execucao_etapas`** — a linha do tempo por etapa. Os eventos `etapa_concluida` e
//!    `etapa_entrou` do motor conduzem: concluir carimba `concluiu_em` na última entrada daquela
//!    etapa; entrar abre a entrada seguinte (`ordem + 1`). No fim, a entrada em que a execução
//!    parou recebe os campos e os atores do estado final (`parcial` quando houve progresso sem
//!    avanço, `pendente` quando não houve nada).
//! 3. **`ficha_eventos`** — um evento por evento do motor, na ordem, com `seq` calculado na mesma
//!    escrita. Cada um recebe a chave `fluxo:<execução>:<chave>#<índice>`: a primeira é o marcador
//!    de que a transição já foi aplicada (é o que faz repetir a chave não duplicar nada).
//! 4. **`fluxo_efeitos`** — os efeitos na fila durável, com a chave do motor (`nó + índice`), que o
//!    índice único `(org_id, execucao_id, chave)` protege: efeito repetido não é reenfileirado.
//! 5. **`conversations` + `fichas`** — quando o motor pede o encerramento
//!    (`fim(encerra_atendimento=True)`), a conversa fica `closed` e a Ficha fecha pela operação da
//!    FX-07, na mesma transação. Encaminhamento **não** é executado no banco: vai como efeito
//!    (`encaminhar`), porque a integração com o departamento é de quem entrega a fila (FX-21).
//!
//! # Ordem das recusas
//!
//! A idempotência vem **antes** da `version`: repetir a chave de uma transição que já entrou tem de
//! devolver o mesmo desfecho, não `412` (é o caso do agente que reenvia a ferramenta depois de uma
//! queda de rede). Depois, `version` (`412`), execução que não está `ativa` (`409`) e Ficha fechada
//! (`409 ficha_encerrada`). Só então a escrita.
//!
//! # O que esta camada não faz
//!
//! - **Rede**: nada aqui fala com Evolution, Synapse ou Jev. `encaminhar`, `mensagem_ao_cliente` e
//!   `notificar_whatsapp` entram na fila e saem por quem a consome.
//! - **`conversations.last_message_at`**: a transição não é mensagem do cliente (decisão da FX-20).
//! - **Auditoria (`audit_log`)**: é da API (FX-30), junto do `If-Match` do pedido.
//! - **Publicação de revisão**: `iniciar_execucao` recebe a revisão pronta (a publicação é FX-09).
//!
//! # Limites conhecidos
//!
//! - O nó (`no_atual`) é gravado pelo **id** (é o que a coluna guarda). Quem reconstrói o `Destino`
//!   tipado do motor precisa da IR em mãos para saber se aquele id é etapa, decisão ou fim.
//! - O mapa de chaves do motor (`Estado.chaves`) não é persistido: no banco a idempotência é o
//!   índice único de `ficha_eventos`, e o motor documenta isso.
//! - Transição sem evento nenhum não tem onde gravar a chave: o estado é gravado e a chave não fica
//!   marcada. O motor nunca devolve transição aceita sem evento (`acao_executada`,
//!   `execucao_abandonada` ou `execucao_falhou` sempre existe), então isto é defesa, não caminho.
//! - A `chave_idempotencia` é obrigatória (vazia é `400`): sem ela, duas transições diferentes
//!   colidiriam no mesmo marcador e a segunda seria lida como repetição.
//! - `fluxo_efeitos.tipo` não é conferido aqui: o `CHECK` da coluna recusa valor fora da lista
//!   fechada, e o valor vem de `EfeitoProduzido::tipo()`.

use poph_types::{
    AtorAcao, DomainError, EfeitoId, EstadoEfeito, ExecucaoId, FechadaPor, FichaId, FluxoId, OrgId,
    RevisaoId, StatusEtapa, StatusExecucao, StatusFicha, TipoEventoFicha,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Map, Value as Json};

use super::fichas::{
    evento_por_chave, fechar_ficha_por_id, ler_eventos_conn, registrar_evento_conn, ultimo_seq,
};
use super::{conflito_ou_ausente, id_da_coluna, u32_da_coluna, u64_da_coluna};
use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{
    EfeitoDaTransicao, EfeitoDoFluxo, EstadoDaExecucao, EtapaDaExecucao, EventoDaTransicao,
    Execucao, ExecucaoGravada, ExtrasDaTransicao, InicioDeExecucao, NovoEventoFicha,
    TransicaoDeFluxo,
};

const COLUNAS: &str = "id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, dados_json,
    resultado, motivo_fim, iniciada_em, finalizada_em, version";

const COLUNAS_ETAPA: &str = "id, org_id, execucao_id, no_id, ordem, status, dados_json,
    atores_que_agiram_json, entrou_em, concluiu_em";

pub(crate) const COLUNAS_EFEITO: &str =
    "id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, estado,
    tentativas, ultimo_erro, proxima_tentativa, enviado_em, created_at";

/// Quantas execuções a lista de uma Ficha devolve por padrão (histórico de fluxos do atendimento).
pub const LIMITE_EXECUCOES: u32 = 50;

/// Acesso às execuções de fluxo, às etapas percorridas e aos efeitos.
pub struct Execucoes<'a>(pub(crate) &'a Store);

impl Execucoes<'_> {
    /// Abre a execução de um fluxo na Ficha: uma escrita com a linha da execução, a entrada na
    /// primeira etapa, o evento `fluxo_iniciado` e os efeitos do `ao_entrar` da etapa de entrada.
    ///
    /// Recusas, nesta ordem, todas antes de gravar:
    ///
    /// | Situação | Erro |
    /// |---|---|
    /// | Ficha de outra org, ou inexistente | `404` [`DomainError::NaoEncontrado`] |
    /// | Ficha já fechada | `409` [`DomainError::FichaFechada`] |
    /// | Fluxo de outra org, ou inexistente | `404` |
    /// | Fluxo `inactive` | `409` [`DomainError::Conflito`] |
    /// | Fluxo não liberado ao agente da Ficha | `403` [`DomainError::FluxoNaoLiberado`] |
    /// | Ficha já com execução ativa (PRD L6) | `409` [`DomainError::ExecucaoJaAtiva`] |
    /// | Revisão que não é daquele fluxo | `404` |
    /// | Estado inicial que não nasce `ativa` | `400` [`DomainError::Validacao`] |
    ///
    /// A permissão é conferida no agente **da Ficha** (o atendimento manda), não em quem chamou.
    pub fn iniciar_execucao(
        &self,
        org: OrgId,
        inicio: InicioDeExecucao<'_>,
    ) -> Resultado<ExecucaoGravada> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_ficha = inicio.ficha.to_string();
        let texto_fluxo = inicio.fluxo.to_string();
        let texto_revisao = inicio.revisao.to_string();
        let ator = inicio.ator;
        let ator_ref = inicio.ator_ref.map(str::to_owned);
        let decisao = inicio
            .decisao_jev
            .as_ref()
            .map(|decisao| (decisao.escolha.to_owned(), decisao.confianca));
        let estado_inicial = inicio.estado_inicial;
        let efeitos_entrada = inicio.efeitos_de_entrada;
        // Marcador da abertura: prefixo próprio, para não colidir com as chaves das transições.
        let chave_inicio = match inicio.chave_idempotencia.map(str::trim) {
            None => None,
            Some("") => {
                return Err(ErroStore::Dominio(DomainError::Validacao {
                    campo: "chave_idempotencia",
                    motivo: "a abertura precisa de uma chave: repetir a mesma tem de devolver a \
                             mesma execução"
                        .to_owned(),
                }));
            }
            Some(chave) => Some(format!("inicio:{chave}")),
        };

        self.0.escrita(move |conn| {
            // Idempotência antes de tudo (como nas transições): a chave já abriu uma execução na
            // Ficha, então devolve essa execução, mesmo que ela já tenha terminado. Não grava nada.
            if let Some(marcador) = chave_inicio.as_deref()
                && let Some(evento) = evento_por_chave(conn, &texto_org, &texto_ficha, marcador)?
            {
                let Some(execucao_id) = evento.execucao_id else {
                    return Err(ErroStore::Interno(
                        "a abertura marcada não tem execução ligada".to_owned(),
                    ));
                };
                let execucao = ler_execucao(conn, &texto_org, &execucao_id.to_string())?
                    .ok_or_else(|| {
                        ErroStore::Interno("a execução da abertura marcada sumiu".to_owned())
                    })?;
                if execucao.fluxo_id.to_string() != texto_fluxo {
                    return Err(ErroStore::Dominio(DomainError::Conflito {
                        entidade: "execução de fluxo",
                        id: execucao.id.to_string(),
                        motivo: "esta chave de idempotência já abriu outro fluxo nesta Ficha"
                            .to_owned(),
                    }));
                }
                return Ok(ExecucaoGravada {
                    execucao,
                    eventos: Vec::new(),
                    efeitos: Vec::new(),
                    fechou_atendimento: false,
                    repetida: true,
                });
            }

            let linha_ficha: Option<(String, String)> = conn
                .query_row(
                    "SELECT status, agent_id FROM fichas WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_ficha],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some((situacao_ficha, agent_id)) = linha_ficha else {
                return Err(nao_encontrado("ficha", &texto_ficha));
            };
            if StatusFicha::de_texto(&situacao_ficha).map_err(ErroStore::from)?
                == StatusFicha::Fechada
            {
                return Err(ErroStore::Dominio(DomainError::FichaFechada {
                    id: texto_ficha.clone(),
                }));
            }

            let linha_fluxo: Option<(String, String)> = conn
                .query_row(
                    "SELECT status, slug FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_fluxo],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some((situacao_fluxo, slug)) = linha_fluxo else {
                return Err(nao_encontrado("fluxo", &texto_fluxo));
            };
            if situacao_fluxo != "active" {
                return Err(ErroStore::Dominio(DomainError::Conflito {
                    entidade: "fluxo",
                    id: texto_fluxo.clone(),
                    motivo: "o fluxo está inativo: não inicia execução nova".to_owned(),
                }));
            }

            let liberado: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM agente_fluxos
                      WHERE org_id = ?1 AND agent_id = ?2 AND fluxo_id = ?3",
                    params![texto_org, agent_id, texto_fluxo],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            if liberado.is_none() {
                return Err(ErroStore::Dominio(DomainError::FluxoNaoLiberado {
                    fluxo_id: texto_fluxo.clone(),
                    agent_id,
                }));
            }

            let ativa: Option<String> = conn
                .query_row(
                    "SELECT id FROM fluxo_execucoes
                      WHERE org_id = ?1 AND ficha_id = ?2 AND status = 'ativa'",
                    params![texto_org, texto_ficha],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            if let Some(ativa) = ativa {
                return Err(ErroStore::Dominio(DomainError::ExecucaoJaAtiva {
                    ficha_id: texto_ficha.clone(),
                    execucao_id: ativa,
                }));
            }

            let revisao_do_fluxo: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM fluxo_revisoes WHERE org_id = ?1 AND id = ?2 AND fluxo_id = ?3",
                    params![texto_org, texto_revisao, texto_fluxo],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            if revisao_do_fluxo.is_none() {
                return Err(nao_encontrado("revisão de fluxo", &texto_revisao));
            }

            if estado_inicial.status != StatusExecucao::Ativa {
                return Err(ErroStore::Dominio(DomainError::Validacao {
                    campo: "estado_inicial",
                    motivo: "a execução nasce `ativa`; estado de abertura recusado".to_owned(),
                }));
            }

            let execucao = ExecucaoId::generate().to_string();
            conn.execute(
                "INSERT INTO fluxo_execucoes
                    (id, org_id, ficha_id, fluxo_id, revisao_id, status, no_atual, dados_json,
                     resultado, motivo_fim, iniciada_em, finalizada_em, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ativa', ?6, ?7, NULL, NULL, ?8, NULL, 1)",
                params![
                    execucao,
                    texto_org,
                    texto_ficha,
                    texto_fluxo,
                    texto_revisao,
                    estado_inicial.no_atual,
                    estado_inicial.dados_json,
                    agora
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("fluxo_execucoes", erro))?;

            // Primeira entrada da linha do tempo por etapa (W6).
            conn.execute(
                "INSERT INTO fluxo_execucao_etapas
                    (id, org_id, execucao_id, no_id, ordem, status, dados_json,
                     atores_que_agiram_json, entrou_em, concluiu_em)
                 VALUES (?1, ?2, ?3, ?4, 1, 'pendente', ?5, '[]', ?6, NULL)",
                params![
                    uuid::Uuid::now_v7().to_string(),
                    texto_org,
                    execucao,
                    estado_inicial.no_atual,
                    dados_da_etapa(&estado_inicial.dados_json, &estado_inicial.no_atual),
                    agora
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("fluxo_execucao_etapas", erro))?;

            let mut detalhe = Map::new();
            detalhe.insert("fluxo".to_owned(), Json::String(slug));
            detalhe.insert(
                "etapa".to_owned(),
                Json::String(estado_inicial.no_atual.clone()),
            );
            detalhe.insert(
                "ator".to_owned(),
                Json::String(ator.como_texto().to_owned()),
            );
            if let Some((escolha, confianca)) = decisao {
                let mut jev = Map::new();
                jev.insert("escolha".to_owned(), Json::String(escolha));
                if let Some(confianca) = confianca {
                    jev.insert(
                        "confianca".to_owned(),
                        serde_json::Number::from_f64(confianca)
                            .map(Json::Number)
                            .unwrap_or(Json::Null),
                    );
                }
                detalhe.insert("jev".to_owned(), Json::Object(jev));
            }

            let seq_antes = ultimo_seq(conn, &texto_org, &texto_ficha)?;
            registrar_evento_conn(
                conn,
                &texto_org,
                &texto_ficha,
                &NovoEventoFicha {
                    execucao_id: Some(
                        execucao
                            .parse()
                            .map_err(|erro: DomainError| ErroStore::Dominio(erro))?,
                    ),
                    tipo: TipoEventoFicha::FluxoIniciado,
                    ator_tipo: ator,
                    ator_ref: ator_ref.as_deref(),
                    payload_json: Json::Object(detalhe).to_string(),
                    chave_idempotencia: chave_inicio.as_deref(),
                },
                agora,
            )?;
            let eventos = ler_eventos_conn(conn, &texto_org, &texto_ficha, seq_antes)?;

            let efeitos = gravar_efeitos(
                conn,
                &texto_org,
                &texto_ficha,
                &execucao,
                &efeitos_entrada,
                agora,
            )?;

            let gravada = ler_execucao(conn, &texto_org, &execucao)?.ok_or_else(|| {
                ErroStore::Interno("a execução foi gravada e não voltou".to_owned())
            })?;
            Ok(ExecucaoGravada {
                execucao: gravada,
                eventos,
                efeitos,
                fechou_atendimento: false,
                repetida: false,
            })
        })
    }

    /// Diz se a chave de idempotência já aplicou uma transição nesta execução (FX-30).
    ///
    /// A API usa isso **antes** de rodar o motor: sem essa leitura, repetir uma ação já aplicada
    /// cairia no motor sobre o estado novo, que recusaria a ação antes de chegar ao repositório.
    /// Só lê; quem grava continua sendo [`Execucoes::aplicar_transicao`], que também confere a chave.
    pub fn transicao_ja_aplicada(
        &self,
        org: OrgId,
        execucao: ExecucaoId,
        chave_idempotencia: &str,
    ) -> Resultado<bool> {
        let chave = chave_idempotencia.trim();
        if chave.is_empty() {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "chave_idempotencia",
                motivo: "informe a chave de idempotência da operação".to_owned(),
            }));
        }
        let marcador = chave_do_evento(&execucao.to_string(), chave, 0);
        let texto_org = org.to_string();
        self.0.leitura(move |conn| {
            let achado: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM ficha_eventos WHERE org_id = ?1 AND chave_idempotencia = ?2",
                    params![texto_org, marcador],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            Ok(achado.is_some())
        })
    }

    /// Qual execução a chave de abertura já abriu nesta Ficha, se alguma (FX-30).
    ///
    /// A API usa isso **antes** de conferir Ficha fechada e `If-Match`: repetir a abertura (reenvio
    /// depois de queda de conexão) devolve a execução da primeira chamada, mesmo que a Ficha já tenha
    /// fechado. Mesma chave para outro fluxo → `Conflito`. Só lê; quem grava é `iniciar_execucao`.
    pub fn abertura_ja_feita(
        &self,
        org: OrgId,
        ficha: FichaId,
        fluxo: FluxoId,
        chave_idempotencia: &str,
    ) -> Resultado<Option<Execucao>> {
        let chave = chave_idempotencia.trim();
        if chave.is_empty() {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "chave_idempotencia",
                motivo: "a abertura precisa de uma chave".to_owned(),
            }));
        }
        let marcador = format!("inicio:{chave}");
        let texto_org = org.to_string();
        let texto_ficha = ficha.to_string();
        let texto_fluxo = fluxo.to_string();
        self.0.leitura(move |conn| {
            let Some(evento) = evento_por_chave(conn, &texto_org, &texto_ficha, &marcador)? else {
                return Ok(None);
            };
            let Some(execucao_id) = evento.execucao_id else {
                return Err(ErroStore::Interno(
                    "a abertura marcada não tem execução ligada".to_owned(),
                ));
            };
            let execucao =
                ler_execucao(conn, &texto_org, &execucao_id.to_string())?.ok_or_else(|| {
                    ErroStore::Interno("a execução da abertura marcada sumiu".to_owned())
                })?;
            if execucao.fluxo_id.to_string() != texto_fluxo {
                return Err(ErroStore::Dominio(DomainError::Conflito {
                    entidade: "execução de fluxo",
                    id: execucao.id.to_string(),
                    motivo: "esta chave de idempotência já abriu outro fluxo nesta Ficha"
                        .to_owned(),
                }));
            }
            Ok(Some(execucao))
        })
    }

    /// Aplica o desfecho do motor numa escrita só: estado, etapas, eventos, efeitos e — quando o
    /// fluxo pede — o encerramento da conversa e da Ficha.
    ///
    /// `esperada` é a `version` que o chamador leu. Repetir a `chave_idempotencia` devolve o estado
    /// como está, com `repetida = true`, sem evento nem efeito novo.
    pub fn aplicar_transicao(
        &self,
        org: OrgId,
        execucao: ExecucaoId,
        esperada: u64,
        transicao: &TransicaoDeFluxo,
        chave_idempotencia: &str,
        extras: ExtrasDaTransicao<'_>,
    ) -> Resultado<ExecucaoGravada> {
        // Chave vazia faria duas transições diferentes colidirem no mesmo evento `#0` — a segunda
        // seria lida como repetição e não gravaria nada. O chamador sintetiza a chave quando o
        // agente não manda nenhuma; aqui ela é obrigatória.
        let chave = chave_idempotencia.trim();
        if chave.is_empty() {
            return Err(ErroStore::Dominio(DomainError::Validacao {
                campo: "chave_idempotencia",
                motivo:
                    "a transição precisa de uma chave: repetir a mesma tem de devolver o mesmo \
                         desfecho, não gravar outra coisa"
                        .to_owned(),
            }));
        }
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_execucao = execucao.to_string();
        let transicao = transicao.clone();
        let chave = chave.to_owned();
        let ator_ref = extras.ator_ref.map(str::to_owned);
        let fechada_por = extras.fechada_por.unwrap_or(FechadaPor::Fluxo);
        let motivo_pedido = extras.motivo_encerramento.map(str::to_owned);
        let esperada_i64 = i64::try_from(esperada).unwrap_or(i64::MAX);

        self.0.escrita(move |conn| {
            let Some(atual) = ler_execucao(conn, &texto_org, &texto_execucao)? else {
                return Err(nao_encontrado("execução de fluxo", &texto_execucao));
            };
            let texto_ficha = atual.ficha_id.to_string();
            let novo = &transicao.novo_estado;

            // 1. Idempotência antes da versão: repetir a chave devolve o desfecho, não `412`.
            if transicao.repetida {
                return Ok(ExecucaoGravada {
                    execucao: atual,
                    eventos: Vec::new(),
                    efeitos: Vec::new(),
                    fechou_atendimento: false,
                    repetida: true,
                });
            }
            let marcador = chave_do_evento(&texto_execucao, &chave, 0);
            let aplicada: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM ficha_eventos
                      WHERE org_id = ?1 AND ficha_id = ?2 AND chave_idempotencia = ?3",
                    params![texto_org, texto_ficha, marcador],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            if aplicada.is_some() {
                return Ok(ExecucaoGravada {
                    execucao: atual,
                    eventos: Vec::new(),
                    efeitos: Vec::new(),
                    fechou_atendimento: false,
                    repetida: true,
                });
            }

            // 2. Versão (`If-Match` da execução).
            if atual.versao != esperada {
                return Err(ErroStore::Dominio(DomainError::ConflitoDeVersao {
                    entidade: "execução de fluxo",
                    id: texto_execucao.clone(),
                    esperada,
                    atual: atual.versao,
                }));
            }

            // 3. Só execução ativa aceita transição.
            if atual.status != StatusExecucao::Ativa {
                return Err(ErroStore::Dominio(DomainError::Conflito {
                    entidade: "execução de fluxo",
                    id: texto_execucao.clone(),
                    motivo: format!("a execução está {} e não aceita transição", atual.status),
                }));
            }

            // 4. A Ficha do atendimento precisa estar aberta (PRD L5).
            let linha_ficha: Option<(String, Option<String>)> = conn
                .query_row(
                    "SELECT status, conversation_id FROM fichas WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_ficha],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some((situacao_ficha, conversa)) = linha_ficha else {
                return Err(nao_encontrado("ficha", &texto_ficha));
            };
            if StatusFicha::de_texto(&situacao_ficha).map_err(ErroStore::from)?
                == StatusFicha::Fechada
            {
                return Err(ErroStore::Dominio(DomainError::FichaFechada {
                    id: texto_ficha.clone(),
                }));
            }

            // 5. Coerência do desfecho: encerrar o atendimento exige execução concluída.
            if transicao.encerra_atendimento && novo.status != StatusExecucao::Concluida {
                return Err(ErroStore::Dominio(DomainError::Validacao {
                    campo: "encerra_atendimento",
                    motivo: "só execução concluída num fim pode encerrar o atendimento".to_owned(),
                }));
            }

            // 6. A linha da execução.
            let finalizada = match novo.status {
                StatusExecucao::Ativa => None,
                _ => Some(agora),
            };
            let alteradas = conn
                .execute(
                    "UPDATE fluxo_execucoes
                        SET status = ?3, no_atual = ?4, dados_json = ?5, resultado = ?6,
                            motivo_fim = ?7, finalizada_em = ?8, version = version + 1
                      WHERE org_id = ?1 AND id = ?2 AND version = ?9",
                    params![
                        texto_org,
                        texto_execucao,
                        novo.status.como_texto(),
                        novo.no_atual,
                        novo.dados_json,
                        novo.resultado,
                        novo.motivo,
                        finalizada,
                        esperada_i64
                    ],
                )
                .map_err(ErroStore::from)?;
            if alteradas == 0 {
                return Err(conflito_ou_ausente(
                    conn,
                    "fluxo_execucoes",
                    "execução de fluxo",
                    &texto_execucao,
                    &texto_org,
                    esperada,
                ));
            }

            // 7. Etapas percorridas (concluir e entrar, guiadas pelos eventos do motor).
            gravar_etapas(
                conn,
                &texto_org,
                &texto_execucao,
                novo,
                &transicao.eventos,
                agora,
            )?;

            // 8. Eventos da Ficha, com a chave de idempotência de cada um.
            let seq_antes = ultimo_seq(conn, &texto_org, &texto_ficha)?;
            for (indice, evento) in transicao.eventos.iter().enumerate() {
                registrar_evento_conn(
                    conn,
                    &texto_org,
                    &texto_ficha,
                    &NovoEventoFicha {
                        execucao_id: Some(execucao),
                        tipo: evento.tipo,
                        ator_tipo: evento.ator_tipo,
                        ator_ref: ator_ref.as_deref(),
                        payload_json: evento.payload_json.clone(),
                        chave_idempotencia: Some(&chave_do_evento(&texto_execucao, &chave, indice)),
                    },
                    agora,
                )?;
            }
            let eventos = ler_eventos_conn(conn, &texto_org, &texto_ficha, seq_antes)?;

            // 9. Efeitos na fila durável.
            let efeitos = gravar_efeitos(
                conn,
                &texto_org,
                &texto_ficha,
                &texto_execucao,
                &transicao.efeitos,
                agora,
            )?;

            // 10. Encerramento do atendimento, na mesma transação (PRD L5 e §6.4).
            let mut fechou = false;
            if transicao.encerra_atendimento {
                if let Some(conversa) = conversa {
                    conn.execute(
                        "UPDATE conversations
                            SET status = 'closed', version = version + 1, updated_at = ?3
                          WHERE org_id = ?1 AND id = ?2 AND status <> 'closed'",
                        params![texto_org, conversa, agora],
                    )
                    .map_err(|erro| ErroStore::da_escrita("conversa", erro))?;
                }
                let motivo = motivo_pedido
                    .or_else(|| novo.resultado.clone())
                    .unwrap_or_else(|| "fim do fluxo".to_owned());
                fechou = fechar_ficha_por_id(
                    conn,
                    &texto_org,
                    &texto_ficha,
                    fechada_por,
                    &motivo,
                    agora,
                )?;
            }

            let gravada = ler_execucao(conn, &texto_org, &texto_execucao)?.ok_or_else(|| {
                ErroStore::Interno("a execução foi gravada e não voltou".to_owned())
            })?;
            Ok(ExecucaoGravada {
                execucao: gravada,
                eventos,
                efeitos,
                fechou_atendimento: fechou,
                repetida: false,
            })
        })
    }

    /// Execução pelo id, dentro da org.
    pub fn obter(&self, org: OrgId, id: ExecucaoId) -> Resultado<Execucao> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrada = self
            .0
            .leitura(move |conn| ler_execucao(conn, &texto_org, &texto_id))?;
        match encontrada {
            Some(execucao) => Ok(execucao),
            None => Err(nao_encontrado("execução de fluxo", &id.to_string())),
        }
    }

    /// Execução ativa da Ficha, se houver (no máximo uma, pelo índice único parcial).
    pub fn ativa_da_ficha(&self, org: OrgId, ficha: FichaId) -> Resultado<Option<Execucao>> {
        let texto_org = org.to_string();
        let texto_ficha = ficha.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS} FROM fluxo_execucoes
                      WHERE org_id = ?1 AND ficha_id = ?2 AND status = 'ativa'"
                ),
                params![texto_org, texto_ficha],
                linha_execucao,
            )
            .optional()
            .map_err(ErroStore::from)
        })
    }

    /// Execuções da Ficha, da mais antiga para a mais nova (o histórico de fluxos do atendimento).
    pub fn da_ficha(&self, org: OrgId, ficha: FichaId, limite: u32) -> Resultado<Vec<Execucao>> {
        let texto_org = org.to_string();
        let texto_ficha = ficha.to_string();
        let limite = i64::from(limite.clamp(1, 500));
        self.0.leitura(move |conn| {
            let mut consulta = conn
                .prepare(&format!(
                    "SELECT {COLUNAS} FROM fluxo_execucoes
                      WHERE org_id = ?1 AND ficha_id = ?2
                      ORDER BY iniciada_em, id LIMIT ?3"
                ))
                .map_err(ErroStore::from)?;
            let linhas = consulta
                .query_map(params![texto_org, texto_ficha, limite], linha_execucao)
                .map_err(ErroStore::from)?;
            let mut execucoes = Vec::new();
            for linha in linhas {
                execucoes.push(linha.map_err(ErroStore::from)?);
            }
            Ok(execucoes)
        })
    }

    /// Quantas execuções **ativas** cada revisão do fluxo tem agora (o `diff` da API mostra isso,
    /// PRD §6.2: quem está num atendimento continua na revisão em que começou).
    ///
    /// Só as revisões com pelo menos uma execução ativa aparecem; revisão sem atendimento em curso
    /// não está na lista. Fluxo de outra org (ou inexistente) devolve lista vazia — quem chama
    /// confere o fluxo antes (`Fluxos::obter`), como a API faz.
    pub fn ativas_por_revisao(
        &self,
        org: OrgId,
        fluxo: FluxoId,
    ) -> Resultado<Vec<(RevisaoId, u64)>> {
        let texto_org = org.to_string();
        let texto_fluxo = fluxo.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn
                .prepare(
                    "SELECT revisao_id, COUNT(*) FROM fluxo_execucoes
                      WHERE org_id = ?1 AND fluxo_id = ?2 AND status = 'ativa'
                      GROUP BY revisao_id ORDER BY revisao_id",
                )
                .map_err(ErroStore::from)?;
            let linhas = consulta
                .query_map(params![texto_org, texto_fluxo], |linha| {
                    Ok((
                        id_da_coluna::<RevisaoId>(linha, 0)?,
                        u64_da_coluna(linha, 1)?,
                    ))
                })
                .map_err(ErroStore::from)?;
            let mut contagens = Vec::new();
            for linha in linhas {
                contagens.push(linha.map_err(ErroStore::from)?);
            }
            Ok(contagens)
        })
    }

    /// Etapas percorridas pela execução, em ordem de entrada.
    pub fn etapas(&self, org: OrgId, execucao: ExecucaoId) -> Resultado<Vec<EtapaDaExecucao>> {
        self.obter(org, execucao)?;
        let texto_org = org.to_string();
        let texto_execucao = execucao.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn
                .prepare(&format!(
                    "SELECT {COLUNAS_ETAPA} FROM fluxo_execucao_etapas
                      WHERE org_id = ?1 AND execucao_id = ?2 ORDER BY ordem"
                ))
                .map_err(ErroStore::from)?;
            let linhas = consulta
                .query_map(params![texto_org, texto_execucao], linha_etapa)
                .map_err(ErroStore::from)?;
            let mut etapas = Vec::new();
            for linha in linhas {
                etapas.push(linha.map_err(ErroStore::from)?);
            }
            Ok(etapas)
        })
    }

    /// Efeitos enfileirados pela execução, em ordem de criação.
    pub fn efeitos(&self, org: OrgId, execucao: ExecucaoId) -> Resultado<Vec<EfeitoDoFluxo>> {
        self.obter(org, execucao)?;
        let texto_org = org.to_string();
        let texto_execucao = execucao.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn
                .prepare(&format!(
                    "SELECT {COLUNAS_EFEITO} FROM fluxo_efeitos
                      WHERE org_id = ?1 AND execucao_id = ?2 ORDER BY created_at, id"
                ))
                .map_err(ErroStore::from)?;
            let linhas = consulta
                .query_map(params![texto_org, texto_execucao], linha_efeito)
                .map_err(ErroStore::from)?;
            let mut efeitos = Vec::new();
            for linha in linhas {
                efeitos.push(linha.map_err(ErroStore::from)?);
            }
            Ok(efeitos)
        })
    }
}

/// Chave do evento que marca a transição: `fluxo:<execução>:<chave>#<índice>`.
///
/// O primeiro evento da transição (`#0`) é o **marcador**: se ele existe, a transição já entrou e a
/// repetição devolve o estado sem gravar nada. O prefixo com a execução deixa a mesma chave do
/// cliente valer em outra execução da mesma Ficha sem colidir no índice único de `ficha_eventos`.
fn chave_do_evento(execucao: &str, chave: &str, indice: usize) -> String {
    format!("fluxo:{execucao}:{chave}#{indice}")
}

fn nao_encontrado(entidade: &'static str, id: &str) -> ErroStore {
    ErroStore::Dominio(DomainError::NaoEncontrado {
        entidade,
        id: id.to_owned(),
    })
}

/// Grava a linha do tempo por etapa guiada pelos eventos do motor.
///
/// - `etapa_concluida` carimba `concluiu_em` na última entrada daquela etapa (a que está aberta);
/// - `etapa_entrou` abre a entrada seguinte (`ordem + 1`), com os campos daquela etapa;
/// - no fim, a entrada em que a execução parou recebe os campos e os atores do estado final.
///
/// Status da entrada aberta: `parcial` quando já houve progresso nela (campo ou ator), `pendente`
/// quando não houve. `concluida` é só a etapa que o motor concluiu, e ela é a que tem
/// `concluiu_em` — o `CHECK` da tabela anda junto.
fn gravar_etapas(
    conn: &Connection,
    org: &str,
    execucao: &str,
    novo: &EstadoDaExecucao,
    eventos: &[EventoDaTransicao],
    agora: i64,
) -> Resultado<()> {
    for evento in eventos {
        match evento.tipo {
            TipoEventoFicha::EtapaConcluida => {
                let Some(etapa) = etapa_do_payload(&evento.payload_json) else {
                    continue;
                };
                conn.execute(
                    "UPDATE fluxo_execucao_etapas
                        SET status = 'concluida', concluiu_em = ?4
                      WHERE org_id = ?1 AND execucao_id = ?2 AND no_id = ?3
                        AND status <> 'concluida'
                        AND ordem = (SELECT MAX(ordem) FROM fluxo_execucao_etapas
                                      WHERE org_id = ?1 AND execucao_id = ?2 AND no_id = ?3)",
                    params![org, execucao, etapa, agora],
                )
                .map_err(ErroStore::from)?;
            }
            TipoEventoFicha::EtapaEntrou => {
                let Some(etapa) = etapa_do_payload(&evento.payload_json) else {
                    continue;
                };
                let ordem: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(ordem), 0) + 1 FROM fluxo_execucao_etapas
                          WHERE org_id = ?1 AND execucao_id = ?2",
                        params![org, execucao],
                        |linha| linha.get(0),
                    )
                    .map_err(ErroStore::from)?;
                conn.execute(
                    "INSERT INTO fluxo_execucao_etapas
                        (id, org_id, execucao_id, no_id, ordem, status, dados_json,
                         atores_que_agiram_json, entrou_em, concluiu_em)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'pendente', ?6, '[]', ?7, NULL)",
                    params![
                        uuid::Uuid::now_v7().to_string(),
                        org,
                        execucao,
                        etapa,
                        ordem,
                        dados_da_etapa(&novo.dados_json, &etapa),
                        agora
                    ],
                )
                .map_err(|erro| ErroStore::da_escrita("fluxo_execucao_etapas", erro))?;
            }
            _ => {}
        }
    }

    if novo.status != StatusExecucao::Ativa {
        return Ok(());
    }
    let dados = dados_da_etapa(&novo.dados_json, &novo.no_atual);
    let atores = serde_json::to_string(&novo.atores_que_agiram).unwrap_or_else(|_| "[]".to_owned());
    let status = if dados == "{}" && novo.atores_que_agiram.is_empty() {
        StatusEtapa::Pendente
    } else {
        StatusEtapa::Parcial
    };
    conn.execute(
        "UPDATE fluxo_execucao_etapas
            SET dados_json = ?3, atores_que_agiram_json = ?4, status = ?5
          WHERE org_id = ?1 AND execucao_id = ?2 AND no_id = ?6 AND status <> 'concluida'
            AND ordem = (SELECT MAX(ordem) FROM fluxo_execucao_etapas
                          WHERE org_id = ?1 AND execucao_id = ?2)",
        params![
            org,
            execucao,
            dados,
            atores,
            status.como_texto(),
            novo.no_atual
        ],
    )
    .map_err(ErroStore::from)?;
    Ok(())
}

/// Enfileira os efeitos, ignorando a chave que já existe (o índice único é a garantia).
///
/// Devolve só os que entraram agora: repetir a mesma transição não devolve efeito duplicado.
fn gravar_efeitos(
    conn: &Connection,
    org: &str,
    ficha: &str,
    execucao: &str,
    efeitos: &[EfeitoDaTransicao],
    agora: i64,
) -> Resultado<Vec<EfeitoDoFluxo>> {
    let ficha_id: FichaId = ficha.parse().map_err(ErroStore::Dominio)?;
    let execucao_id: ExecucaoId = execucao.parse().map_err(ErroStore::Dominio)?;
    let org_id: OrgId = org.parse().map_err(ErroStore::Dominio)?;
    let mut gravados = Vec::new();
    for efeito in efeitos {
        let id = EfeitoId::generate();
        let inseridas = conn
            .execute(
                "INSERT INTO fluxo_efeitos
                    (id, org_id, ficha_id, execucao_id, chave, tipo, payload_json, estado,
                     tentativas, ultimo_erro, proxima_tentativa, enviado_em, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pendente', 0, NULL, NULL, NULL, ?8)
                 ON CONFLICT (org_id, execucao_id, chave) DO NOTHING",
                params![
                    id.to_string(),
                    org,
                    ficha,
                    execucao,
                    efeito.chave,
                    efeito.tipo,
                    efeito.payload_json,
                    agora
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("fluxo_efeitos", erro))?;
        if inseridas == 1 {
            gravados.push(EfeitoDoFluxo {
                id,
                org_id,
                ficha_id,
                execucao_id,
                chave: efeito.chave.clone(),
                tipo: efeito.tipo.clone(),
                payload_json: efeito.payload_json.clone(),
                estado: EstadoEfeito::Pendente,
                tentativas: 0,
                ultimo_erro: None,
                proxima_tentativa: None,
                enviado_em: None,
                criado_em: agora,
            });
        }
    }
    Ok(gravados)
}

/// Execução pelo id na org, sem erro quando não existe.
fn ler_execucao(conn: &Connection, org: &str, execucao: &str) -> Resultado<Option<Execucao>> {
    conn.query_row(
        &format!("SELECT {COLUNAS} FROM fluxo_execucoes WHERE org_id = ?1 AND id = ?2"),
        params![org, execucao],
        linha_execucao,
    )
    .optional()
    .map_err(ErroStore::from)
}

/// Recorte `dados[etapa]` do JSON da execução (`{}` quando a etapa ainda não tem campo).
fn dados_da_etapa(dados_json: &str, no: &str) -> String {
    let Ok(Json::Object(mapa)) = serde_json::from_str::<Json>(dados_json) else {
        return "{}".to_owned();
    };
    match mapa.get(no) {
        Some(Json::Object(_)) => {
            serde_json::to_string(&mapa[no]).unwrap_or_else(|_| "{}".to_owned())
        }
        _ => "{}".to_owned(),
    }
}

/// Campo `etapa` do payload de um evento do motor.
fn etapa_do_payload(payload_json: &str) -> Option<String> {
    let valor: Json = serde_json::from_str(payload_json).ok()?;
    valor.get("etapa")?.as_str().map(str::to_owned)
}

fn linha_execucao(linha: &Row<'_>) -> rusqlite::Result<Execucao> {
    Ok(Execucao {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        ficha_id: id_da_coluna(linha, 2)?,
        fluxo_id: id_da_coluna(linha, 3)?,
        revisao_id: id_da_coluna(linha, 4)?,
        status: StatusExecucao::de_texto(&linha.get::<_, String>(5)?)
            .map_err(|erro| conversao(5, erro))?,
        no_atual: linha.get(6)?,
        dados_json: linha.get(7)?,
        resultado: linha.get(8)?,
        motivo_fim: linha.get(9)?,
        iniciada_em: linha.get(10)?,
        finalizada_em: linha.get(11)?,
        versao: u64_da_coluna(linha, 12)?,
    })
}

fn linha_etapa(linha: &Row<'_>) -> rusqlite::Result<EtapaDaExecucao> {
    let atores_json: String = linha.get(7)?;
    let atores: Vec<AtorAcao> = serde_json::from_str(&atores_json).map_err(|erro| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(erro))
    })?;
    Ok(EtapaDaExecucao {
        id: linha.get(0)?,
        org_id: id_da_coluna(linha, 1)?,
        execucao_id: id_da_coluna(linha, 2)?,
        no_id: linha.get(3)?,
        ordem: linha.get(4)?,
        status: StatusEtapa::de_texto(&linha.get::<_, String>(5)?)
            .map_err(|erro| conversao(5, erro))?,
        dados_json: linha.get(6)?,
        atores_que_agiram: atores,
        entrou_em: linha.get(8)?,
        concluiu_em: linha.get(9)?,
    })
}

pub(crate) fn linha_efeito(linha: &Row<'_>) -> rusqlite::Result<EfeitoDoFluxo> {
    Ok(EfeitoDoFluxo {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        ficha_id: id_da_coluna(linha, 2)?,
        execucao_id: id_da_coluna(linha, 3)?,
        chave: linha.get(4)?,
        tipo: linha.get(5)?,
        payload_json: linha.get(6)?,
        estado: EstadoEfeito::de_texto(&linha.get::<_, String>(7)?)
            .map_err(|erro| conversao(7, erro))?,
        tentativas: u32_da_coluna(linha, 8)?,
        ultimo_erro: linha.get(9)?,
        proxima_tentativa: linha.get(10)?,
        enviado_em: linha.get(11)?,
        criado_em: linha.get(12)?,
    })
}

/// Texto de coluna que não é valor do vocabulário → erro de conversão do SQLite.
fn conversao(indice: usize, erro: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(indice, rusqlite::types::Type::Text, Box::new(erro))
}
