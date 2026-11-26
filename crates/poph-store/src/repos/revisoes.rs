//! Publicação de revisões de fluxo e leitura delas (FX-09, PRD §6.2 e §7.1).
//!
//! O rascunho (`fluxos.rascunho_fonte`) é texto livre, pode estar inválido. O que o motor executa é
//! uma **revisão publicada**, imutável, com o texto de origem, o hash dele e a IR já validada. Este
//! arquivo é a única porta de entrada de revisão no banco: só existe publicação (acrescentar), não
//! existe alteração.
//!
//! # O que uma publicação grava (uma escrita só)
//!
//! 1. **`fluxos`** — `revisao_atual = n + 1`, `rascunho_fonte` = texto publicado e `version + 1`,
//!    conferindo a versão esperada no `WHERE`. É o passo que resolve a corrida: duas publicações com
//!    a mesma versão esperada chegam juntas, a primeira avança a versão e a segunda não casa o
//!    `WHERE` — recebe `412` ([`DomainError::ConflitoDeVersao`]) **antes** de gravar qualquer coisa,
//!    então não há revisão órfã. O número `n + 1` é calculado pela própria escrita
//!    (`COALESCE(revisao_atual, 0) + 1`), não por quem chamou.
//! 2. **`audit_log`** — `flow_publish` com o número, o id da revisão, o `schema_versao`, o hash do
//!    texto e o id do ator (o tipo vai também em `publicada_por`).
//! 3. **`fluxo_revisoes`** — a revisão nova, com `fonte`, `fonte_sha256`, `definicao_json`,
//!    `schema_versao`, `publicada_em`, `publicada_por` e `resolucoes_json`. É o último passo: se ele
//!    falhar, o `SAVEPOINT` do escritor devolve a auditoria e o fluxo junto (teste
//!    `falha_depois_da_auditoria_nao_deixa_revisao_nem_auditoria`).
//!
//! # Como a imutabilidade é garantida
//!
//! - **No código**: a revisão só é escrita por `INSERT` (`publicar_revisao`). Não há operação de
//!   alteração nem de exclusão neste repositório, e `Fluxo::atualizar` não toca em revisão.
//! - **No banco**: `tr_fluxo_revisoes_sem_update` recusa qualquer `UPDATE` e
//!   `tr_fluxo_revisoes_sem_delete` recusa `DELETE` de revisão de fluxo que continua de pé (a
//!   cascata da exclusão do fluxo e da org passa — o `WHEN` do gatilho existe para isso).
//! - **Na numeração**: `ux_fluxo_revisoes_numero` (org + fluxo + número) impede duas revisões com o
//!   mesmo número, mesmo que alguém escreva no banco por fora.
//!
//! # O que este repositório não faz
//!
//! - **Compilar ou validar fluxo**: recebe a IR já validada por quem chama (a API, FX-27) e não
//!   importa `poph-fluxos` (regra de camadas do `AGENTS.md`).
//! - **Resolver departamento**: `resolucoes_json` vem pronto de quem publica; aqui só se confere que
//!   é um objeto JSON, porque a linha é imutável e um valor torto não teria conserto depois.
//! - **Ler a IR**: `definicao_json` é guardado e devolvido como texto; interpretar é do motor.
//!
//! # Recusas que o repositório acrescenta
//!
//! A revisão é imutável, então o que entra errado não tem conserto — só uma revisão nova por cima.
//! Por isso a publicação recusa, com [`DomainError::Validacao`]: `fonte` em branco, `schema_versao`
//! ausente ou zero, `definicao_json` que não é objeto JSON, `definicao_json` que declara um
//! `schema_versao` diferente do informado, `resolucoes_json` que não é objeto JSON e `ator_tipo` fora
//! do vocabulário de `audit_log.actor_kind`.

use poph_types::{AuditId, DomainError, FlowId, OrgId, RevisionId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{conflito_ou_ausente, exigir, id_da_coluna, pertence, u32_da_coluna, u64_da_coluna};
use crate::Store;
use crate::backup_documento::sha256_hex;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{PublicacaoDeRevisao, RevisaoDeFluxo};

const COLUNAS: &str = "id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
    schema_versao, publicada_em, publicada_por, resolucoes_json";

/// Ação gravada em `audit_log.action` quando uma revisão é publicada.
pub const ACAO_PUBLICAR: &str = "flow_publish";

/// Vocabulário de `audit_log.actor_kind` (o `CHECK` da migração 0004). Quem publica é um destes.
pub const ATORES_DE_AUDITORIA: [&str; 5] =
    ["admin_token", "system", "plugin", "cli", "matrix_user"];

/// Quantas revisões a lista devolve quando o chamador não diz (o Console pede a página).
pub const LIMITE_REVISOES: u32 = 50;

/// Acesso às revisões publicadas do fluxo.
pub struct Revisoes<'a>(pub(crate) &'a Store);

impl Revisoes<'_> {
    /// Publica uma revisão nova do fluxo — tudo numa escrita só (ver o cabeçalho do módulo).
    ///
    /// `publicacao.esperada` é a `version` do fluxo que o chamador leu (`If-Match`). Versão diferente
    /// da gravada é [`DomainError::ConflitoDeVersao`] (`412`), e nada muda. Fluxo de outra org (ou
    /// inexistente) é `NaoEncontrado`. Devolve a revisão como ficou no banco.
    pub fn publicar_revisao(
        &self,
        org: OrgId,
        fluxo: FlowId,
        publicacao: PublicacaoDeRevisao,
    ) -> Resultado<RevisaoDeFluxo> {
        // 1. O que dá para recusar sem tocar no banco. A revisão é imutável: aqui é a última chance.
        if publicacao.fonte.trim().is_empty() {
            return Err(ErroStore::Dominio(DomainError::Validation {
                field: "fonte",
                reason: "o texto publicado não pode ficar vazio: é a origem que a revisão guarda"
                    .to_owned(),
            }));
        }
        let Some(schema_versao) = publicacao.schema_versao else {
            return Err(ErroStore::Dominio(DomainError::Validation {
                field: "schema_versao",
                reason:
                    "informe a versão do esquema da IR (`docs/FLUXOS_LINGUAGEM.md` §8): revisão \
                         sem ela não entra"
                        .to_owned(),
            }));
        };
        if schema_versao == 0 {
            return Err(ErroStore::Dominio(DomainError::Validation {
                field: "schema_versao",
                reason: "a versão do esquema começa em 1".to_owned(),
            }));
        }
        let definicao_json = conferir_ir(&publicacao.definicao_json, schema_versao)?;
        let resolucoes_json = conferir_resolucoes(&publicacao.resolucoes_json)?;
        let ator_tipo = conferir_ator(&publicacao.ator_tipo)?;
        let ator_id = publicacao
            .ator_id
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty());

        let fonte = publicacao.fonte;
        let fonte_sha256 = sha256_hex(fonte.as_bytes());
        let esperada = publicacao.esperada;
        let esperada_i64 = i64::try_from(esperada).unwrap_or(i64::MAX);
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_fluxo = fluxo.to_string();

        self.0.escrita(move |conn| {
            // 2. O fluxo: versão esperada, número novo da revisão, rascunho publicado. A escrita
            //    condicional é a corrida resolvida: quem chega depois não casa o `WHERE` e leva 412
            //    sem ter gravado nada.
            let avancou = conn
                .execute(
                    "UPDATE fluxos
                        SET revisao_atual = COALESCE(revisao_atual, 0) + 1,
                            rascunho_fonte = ?4,
                            version = version + 1
                      WHERE org_id = ?1 AND id = ?2 AND version = ?3",
                    params![texto_org, texto_fluxo, esperada_i64, fonte],
                )
                .map_err(|erro| ErroStore::da_escrita("rascunho_fonte", erro))?;
            if avancou == 0 {
                return Err(conflito_ou_ausente(
                    conn,
                    "fluxos",
                    "fluxo",
                    &texto_fluxo,
                    &texto_org,
                    esperada,
                ));
            }
            let numero: i64 = conn
                .query_row(
                    "SELECT revisao_atual FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_fluxo],
                    |linha| linha.get(0),
                )
                .map_err(ErroStore::from)?;

            // 3. A auditoria da publicação, na mesma transação.
            let revisao_id = RevisionId::generate();
            let detalhe = serde_json::json!({
                "revisao": numero,
                "revisao_id": revisao_id.to_string(),
                "schema_versao": schema_versao,
                "fonte_sha256": fonte_sha256,
            });
            conn.execute(
                "INSERT INTO audit_log
                    (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json,
                     created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'flow', ?6, ?7, ?8)",
                params![
                    AuditId::generate().to_string(),
                    texto_org,
                    ator_tipo,
                    ator_id,
                    ACAO_PUBLICAR,
                    texto_fluxo,
                    detalhe.to_string(),
                    agora
                ],
            )
            .map_err(|erro| {
                ErroStore::da_escrita_em("ator_tipo", &[("actor_kind", "ator_tipo")], erro)
            })?;

            // 4. A revisão nova. Último passo de propósito: se ele falhar, o `SAVEPOINT` do escritor
            //    devolve a auditoria e o fluxo junto.
            conn.execute(
                "INSERT INTO fluxo_revisoes
                    (id, org_id, fluxo_id, numero, fonte, fonte_sha256, definicao_json,
                     schema_versao, publicada_em, publicada_por, resolucoes_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    revisao_id.to_string(),
                    texto_org,
                    texto_fluxo,
                    numero,
                    fonte,
                    fonte_sha256,
                    definicao_json,
                    i64::from(schema_versao),
                    agora,
                    ator_tipo,
                    resolucoes_json
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("numero", erro))?;

            // 5. De volta como ficou no banco (é o que a API devolve).
            ler_por_numero(conn, &texto_org, &texto_fluxo, numero)?.ok_or_else(|| {
                ErroStore::Interno("a revisão foi gravada e não voltou na leitura".to_owned())
            })
        })
    }

    /// Revisão pelo número (1, 2, 3…), dentro da org. Fluxo de outra org → `NaoEncontrado`.
    pub fn obter(&self, org: OrgId, fluxo: FlowId, numero: u64) -> Resultado<RevisaoDeFluxo> {
        let texto_org = org.to_string();
        let texto_fluxo = fluxo.to_string();
        let numero_i64 = i64::try_from(numero).unwrap_or(i64::MAX);
        let encontrada = self.0.leitura(move |conn| {
            pertence(conn, "fluxos", "fluxo", &texto_org, &texto_fluxo)?;
            ler_por_numero(conn, &texto_org, &texto_fluxo, numero_i64)
        })?;
        exigir(encontrada, "revisão de fluxo", &numero.to_string())
    }

    /// Revisão pelo id: é a que uma execução segue (`fluxo_execucoes.revisao_id`), e a ação do
    /// atendente precisa da IR dela para conduzir o motor (FX-30). Revisão de outra org ou
    /// inexistente: `NaoEncontrado`.
    pub fn obter_por_id(&self, org: OrgId, revisao: RevisionId) -> Resultado<RevisaoDeFluxo> {
        let texto_org = org.to_string();
        let texto_revisao = revisao.to_string();
        let encontrada = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM fluxo_revisoes WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_revisao],
                linha_revisao,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        exigir(encontrada, "revisão de fluxo", &revisao.to_string())
    }

    /// A revisão que vale hoje (`fluxos.revisao_atual`), se o fluxo já foi publicado alguma vez.
    ///
    /// Fluxo sem publicação devolve `None` (é o estado de quem só tem rascunho); fluxo de outra org,
    /// `NaoEncontrado`.
    pub fn atual(&self, org: OrgId, fluxo: FlowId) -> Resultado<Option<RevisaoDeFluxo>> {
        let texto_org = org.to_string();
        let texto_fluxo = fluxo.to_string();
        self.0.leitura(move |conn| {
            let numero: Option<Option<i64>> = conn
                .query_row(
                    "SELECT revisao_atual FROM fluxos WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_fluxo],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            match numero {
                None => Err(ErroStore::Dominio(DomainError::NotFound {
                    entity: "fluxo",
                    id: texto_fluxo.clone(),
                })),
                Some(None) => Ok(None),
                Some(Some(numero)) => ler_por_numero(conn, &texto_org, &texto_fluxo, numero)?
                    .map_or_else(
                        || {
                            Err(ErroStore::Interno(format!(
                                "o fluxo {texto_fluxo} aponta para a revisão {numero}, que não \
                                 existe"
                            )))
                        },
                        |revisao| Ok(Some(revisao)),
                    ),
            }
        })
    }

    /// Revisões do fluxo em ordem de número, a partir de `apos` (exclusivo).
    ///
    /// `apos = None` começa na 1. O cursor é o número, não o id: a ordem que interessa ao diff e ao
    /// histórico é a da publicação.
    pub fn listar(
        &self,
        org: OrgId,
        fluxo: FlowId,
        apos: Option<u64>,
        limite: u32,
    ) -> Resultado<Vec<RevisaoDeFluxo>> {
        let texto_org = org.to_string();
        let texto_fluxo = fluxo.to_string();
        let apos = apos.map(|numero| i64::try_from(numero).unwrap_or(i64::MAX));
        let limite = i64::from(limite.clamp(1, 500));
        self.0.leitura(move |conn| {
            pertence(conn, "fluxos", "fluxo", &texto_org, &texto_fluxo)?;
            let mut consulta = conn
                .prepare(&format!(
                    "SELECT {COLUNAS} FROM fluxo_revisoes
                      WHERE org_id = ?1 AND fluxo_id = ?2 AND (?3 IS NULL OR numero > ?3)
                      ORDER BY numero LIMIT ?4"
                ))
                .map_err(ErroStore::from)?;
            let linhas = consulta
                .query_map(params![texto_org, texto_fluxo, apos, limite], linha_revisao)
                .map_err(ErroStore::from)?;
            let mut revisoes = Vec::new();
            for linha in linhas {
                revisoes.push(linha.map_err(ErroStore::from)?);
            }
            Ok(revisoes)
        })
    }
}

/// Revisão pelo número na conexão dada, sem erro quando não existe.
fn ler_por_numero(
    conn: &Connection,
    org: &str,
    fluxo: &str,
    numero: i64,
) -> Resultado<Option<RevisaoDeFluxo>> {
    conn.query_row(
        &format!(
            "SELECT {COLUNAS} FROM fluxo_revisoes
              WHERE org_id = ?1 AND fluxo_id = ?2 AND numero = ?3"
        ),
        params![org, fluxo, numero],
        linha_revisao,
    )
    .optional()
    .map_err(ErroStore::from)
}

/// Confere que a IR é um objeto JSON e que, quando ela declara a versão do esquema, é a informada.
///
/// Não é compilar nem validar fluxo (isso é da API): é só garantir que a coluna imutável guarda JSON
/// com a mesma versão que a coluna `schema_versao` diz, para o motor não encontrar duas verdades.
fn conferir_ir(definicao_json: &str, schema_versao: u32) -> Resultado<String> {
    let Ok(serde_json::Value::Object(mapa)) = serde_json::from_str(definicao_json) else {
        return Err(ErroStore::Dominio(DomainError::Validation {
            field: "definicao_json",
            reason: "a IR entra como objeto JSON já validado por quem chama".to_owned(),
        }));
    };
    if let Some(declarada) = mapa.get("schema_versao") {
        let declarada = declarada.as_u64();
        if declarada != Some(u64::from(schema_versao)) {
            return Err(ErroStore::Dominio(DomainError::Validation {
                field: "schema_versao",
                reason: format!(
                    "a IR declara {declarada:?} e a publicação informa {schema_versao}: os dois têm \
                     de ser a mesma versão"
                ),
            }));
        }
    }
    Ok(definicao_json.to_owned())
}

/// Resoluções: objeto JSON; texto vazio vira `{}` (a coluna tem esse padrão).
fn conferir_resolucoes(resolucoes_json: &str) -> Resultado<String> {
    let texto = resolucoes_json.trim();
    if texto.is_empty() {
        return Ok("{}".to_owned());
    }
    if !matches!(
        serde_json::from_str::<serde_json::Value>(texto),
        Ok(serde_json::Value::Object(_))
    ) {
        return Err(ErroStore::Dominio(DomainError::Validation {
            field: "resolucoes_json",
            reason: "as resoluções entram como objeto JSON (ex.: nome de departamento → id)"
                .to_owned(),
        }));
    }
    Ok(texto.to_owned())
}

/// Tipo do ator no vocabulário de `audit_log.actor_kind`.
fn conferir_ator(ator_tipo: &str) -> Resultado<String> {
    let ator = ator_tipo.trim();
    if ATORES_DE_AUDITORIA.contains(&ator) {
        return Ok(ator.to_owned());
    }
    Err(ErroStore::Dominio(DomainError::Validation {
        field: "ator_tipo",
        reason: format!(
            "use um destes tipos de ator: {}",
            ATORES_DE_AUDITORIA.join(", ")
        ),
    }))
}

/// Lê uma revisão da linha do resultado.
fn linha_revisao(linha: &Row<'_>) -> rusqlite::Result<RevisaoDeFluxo> {
    Ok(RevisaoDeFluxo {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        fluxo_id: id_da_coluna(linha, 2)?,
        numero: u64_da_coluna(linha, 3)?,
        fonte: linha.get(4)?,
        fonte_sha256: linha.get(5)?,
        definicao_json: linha.get(6)?,
        schema_versao: u32_da_coluna(linha, 7)?,
        publicada_em: linha.get(8)?,
        publicada_por: linha.get(9)?,
        resolucoes_json: linha.get(10)?,
    })
}
