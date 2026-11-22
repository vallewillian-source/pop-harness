//! Biblioteca de arquivos do agente (HR-24, decisão D13).
//!
//! É a única origem de arquivo que o agente pode enviar ao cliente: o modelo escolhe um id daqui,
//! nunca um caminho de disco.
//!
//! Regras deste repositório:
//! - **`org_id` e `agent_id` em toda assinatura.** Um arquivo do agente A não é alcançado com o id
//!   dele na org B nem pelo agente B da mesma org: a consulta sempre filtra os dois.
//! - **Sem versão (`If-Match`)**: a tabela `file_library` não é editável — para trocar o conteúdo, o
//!   arquivo é excluído e enviado de novo (o blob é deduplicado por hash).
//! - **O conteúdo não passa por aqui.** `blob_path` é só o endereço relativo à pasta de blobs;
//!   quem grava o arquivo é o crate `poph-files`.
//! - **Validação de campos** existe para o erro dizer qual campo e por quê (o banco só tem o `CHECK`
//!   de tamanho).

use poph_types::{AgentId, FileId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{ArquivoBiblioteca, NovoArquivoBiblioteca, Pagina};
use crate::repos::{exigir, id_da_coluna, pertence};

const COLUNAS: &str =
    "id, org_id, agent_id, name, description, mime, size_bytes, sha256, blob_path, created_at";

/// Entidade usada nas mensagens de "não encontrado".
const ENTIDADE: &str = "arquivo da biblioteca do agente";

/// Teto do nome mostrado ao cliente.
pub const LIMITE_NOME_ARQUIVO: usize = 200;
/// Teto da descrição que o modelo lê.
pub const LIMITE_DESCRICAO_ARQUIVO: usize = 500;

/// Acesso à biblioteca de arquivos do agente.
pub struct Biblioteca<'a>(pub(crate) &'a Store);

impl Biblioteca<'_> {
    /// Exclui e registra o ator da API na mesma transação.
    pub fn excluir_auditado(
        &self,
        org: OrgId,
        agente: AgentId,
        id: FileId,
        ator: String,
    ) -> Resultado<()> {
        let org = org.to_string();
        let agente = agente.to_string();
        let id = id.to_string();
        let audit_id = poph_types::AuditId::generate().to_string();
        let agora = self.0.agora();
        self.0.escrita(move |conn| {
            let apagou = conn.execute(
                "DELETE FROM file_library WHERE org_id = ?1 AND agent_id = ?2 AND id = ?3",
                params![org, agente, id],
            )?;
            if apagou == 0 {
                return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
                    entidade: ENTIDADE,
                    id,
                }));
            }
            conn.execute(
                "INSERT INTO audit_log (id, org_id, actor_kind, actor_id, action, entity, entity_id, detail_json, created_at)
                 VALUES (?1, ?2, 'admin_token', ?3, 'delete', 'file_library', ?4, '{}', ?5)",
                params![audit_id, org, ator, id, agora],
            )?;
            Ok(())
        })
    }

    /// Põe um arquivo na biblioteca do agente.
    ///
    /// O agente precisa existir na org (erro de "não encontrado" quando não existe), e o blob já
    /// deve ter sido gravado por quem chamou (`poph-files`).
    pub fn criar(&self, org: OrgId, novo: NovoArquivoBiblioteca) -> Resultado<ArquivoBiblioteca> {
        let nome = validar_texto("nome", &novo.nome, LIMITE_NOME_ARQUIVO)?;
        let descricao = novo.descricao.trim().to_owned();
        if descricao.chars().count() > LIMITE_DESCRICAO_ARQUIVO {
            return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
                campo: "descrição",
                motivo: format!("use no máximo {LIMITE_DESCRICAO_ARQUIVO} caracteres"),
            }));
        }
        let mime = novo.mime.trim().to_owned();
        if mime.is_empty() {
            return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
                campo: "mime",
                motivo: "informe o tipo real do arquivo".to_owned(),
            }));
        }
        let sha256 = validar_sha256(&novo.sha256)?;
        let blob_path = novo.blob_path.trim().to_owned();
        if blob_path.is_empty() {
            return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
                campo: "blob_path",
                motivo: "informe onde o conteúdo foi gravado".to_owned(),
            }));
        }

        let id = FileId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        let texto_agente = novo.agent_id.to_string();
        let tamanho = i64::try_from(novo.tamanho_bytes).unwrap_or(i64::MAX);
        self.0.escrita(move |conn| {
            pertence(conn, "agents", "agente", &texto_org, &texto_agente)?;
            conn.execute(
                "INSERT INTO file_library
                    (id, org_id, agent_id, name, description, mime, size_bytes, sha256, blob_path,
                     created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    texto_id,
                    texto_org,
                    texto_agente,
                    nome,
                    descricao,
                    mime,
                    tamanho,
                    sha256,
                    blob_path,
                    agora
                ],
            )
            .map_err(|erro| ErroStore::da_escrita("nome", erro))?;
            Ok(())
        })?;
        self.obter(org, novo.agent_id, id)
    }

    /// Busca o arquivo pelo id, **dentro da org e do agente**.
    ///
    /// Id de outro agente ou de outra org devolve "não encontrado" — nunca o registro alheio.
    pub fn obter(&self, org: OrgId, agente: AgentId, id: FileId) -> Resultado<ArquivoBiblioteca> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS} FROM file_library
                      WHERE org_id = ?1 AND agent_id = ?2 AND id = ?3"
                ),
                params![texto_org, texto_agente, texto_id],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        exigir(encontrado, ENTIDADE, &id.to_string())
    }

    /// Lista a biblioteca do agente, em ordem de criação (id cresce com o tempo, UUID v7).
    pub fn listar(
        &self,
        org: OrgId,
        agente: AgentId,
        pagina: Pagina,
    ) -> Resultado<Vec<ArquivoBiblioteca>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM file_library
                  WHERE org_id = ?1 AND agent_id = ?2 AND (?3 IS NULL OR id > ?3)
                  ORDER BY id LIMIT ?4"
            ))?;
            let linhas =
                consulta.query_map(params![texto_org, texto_agente, apos, limite], linha)?;
            let mut arquivos = Vec::new();
            for linha in linhas {
                arquivos.push(linha?);
            }
            Ok(arquivos)
        })
    }

    /// Quantos arquivos o agente tem.
    pub fn contar(&self, org: OrgId, agente: AgentId) -> Resultado<u64> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        self.0.leitura(move |conn| {
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM file_library WHERE org_id = ?1 AND agent_id = ?2",
                params![texto_org, texto_agente],
                |linha| linha.get(0),
            )?;
            Ok(total.max(0) as u64)
        })
    }

    /// Tira o arquivo da biblioteca do agente. O blob fica; quem apaga órfão é a retenção (HR-51).
    pub fn excluir(&self, org: OrgId, agente: AgentId, id: FileId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        let texto_id = id.to_string();
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM file_library WHERE org_id = ?1 AND agent_id = ?2 AND id = ?3",
                params![texto_org, texto_agente, texto_id],
            )
            .map_err(ErroStore::from)
        })?;
        if apagou == 0 {
            return Err(ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
                entidade: ENTIDADE,
                id: id.to_string(),
            }));
        }
        Ok(())
    }
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<ArquivoBiblioteca> {
    Ok(ArquivoBiblioteca {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        agent_id: id_da_coluna(linha, 2)?,
        nome: linha.get(3)?,
        descricao: linha.get(4)?,
        mime: linha.get(5)?,
        tamanho_bytes: crate::repos::u64_da_coluna(linha, 6)?,
        sha256: linha.get(7)?,
        blob_path: linha.get(8)?,
        criado_em: linha.get(9)?,
    })
}

fn validar_texto(campo: &'static str, valor: &str, limite: usize) -> Resultado<String> {
    let limpo = valor.trim().to_owned();
    if limpo.is_empty() {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
            campo,
            motivo: "não pode ficar vazio".to_owned(),
        }));
    }
    if limpo.chars().count() > limite {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
            campo,
            motivo: format!("use no máximo {limite} caracteres"),
        }));
    }
    Ok(limpo)
}

fn validar_sha256(valor: &str) -> Resultado<String> {
    let texto = valor.trim().to_lowercase();
    let valido = texto.len() == 64 && texto.chars().all(|c| c.is_ascii_hexdigit());
    if !valido {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
            campo: "sha256",
            motivo: "precisa ser o hash em hexadecimal (64 caracteres)".to_owned(),
        }));
    }
    Ok(texto)
}
