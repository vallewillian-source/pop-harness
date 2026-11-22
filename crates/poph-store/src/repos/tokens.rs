//! Repositório dos tokens de administração.
//!
//! Os tokens **globais** vêm da configuração (só o hash); os de org ficam nesta tabela.
//! O valor do token nunca é gravado: só o hash, e ele não volta na leitura.

use poph_types::{AdminTokenId, OrgId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{Pagina, TokenAdmin};
use crate::repos::{id_da_coluna, id_opcional};

const COLUNAS: &str = "id, org_id, label, token_hash, created_at, revoked_at";

/// Acesso aos tokens de administração.
pub struct Tokens<'a>(pub(crate) &'a Store);

impl Tokens<'_> {
    /// Cria um token de org (`org` = `None` cria token global).
    pub fn criar(&self, org: Option<OrgId>, rotulo: &str, hash: &str) -> Resultado<TokenAdmin> {
        let id = AdminTokenId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.map(|org| org.to_string());
        let rotulo = rotulo.to_owned();
        let hash = hash.to_owned();
        let guardado = rotulo.clone();
        let guardado_hash = hash.clone();
        self.0.escrita(move |conn| {
            conn.execute(
                "INSERT INTO admin_tokens (id, org_id, label, token_hash, created_at, revoked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![texto_id, texto_org, rotulo, hash, agora],
            )
            .map_err(|erro| ErroStore::da_escrita("token_hash", erro))?;
            Ok(())
        })?;
        Ok(TokenAdmin {
            id,
            org_id: org,
            rotulo: guardado,
            hash: guardado_hash,
            criado_em: agora,
            revogado_em: None,
        })
    }

    /// Busca pelo id **dentro da org** (token global não aparece aqui; ele vem da configuração).
    pub fn obter(&self, org: OrgId, id: AdminTokenId) -> Resultado<TokenAdmin> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM admin_tokens WHERE org_id = ?1 AND id = ?2"),
                params![texto_org, texto_id],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "token de administração", &id.to_string())
    }

    /// Busca pelo hash, **sem org**: é o passo de autenticação, que ainda não sabe de qual org é
    /// o token. Devolve `None` para token inexistente ou revogado.
    pub fn por_hash(&self, hash: &str) -> Resultado<Option<TokenAdmin>> {
        let hash = hash.to_owned();
        self.0.leitura(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT {COLUNAS} FROM admin_tokens
                      WHERE token_hash = ?1 AND revoked_at IS NULL"
                ),
                params![hash],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })
    }

    /// Busca pelo id **sem org**: é o passo de autenticação do token no formato
    /// `poph_<uuid>_<segredo>`, que chega antes de saber a qual org ele pertence.
    ///
    /// Devolve o registro mesmo revogado (o chamador decide: revogado = não autentica), e `None`
    /// só quando não existe.
    pub fn por_id(&self, id: AdminTokenId) -> Resultado<Option<TokenAdmin>> {
        let texto_id = id.to_string();
        self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM admin_tokens WHERE id = ?1"),
                params![texto_id],
                linha,
            )
            .optional()
            .map_err(ErroStore::from)
        })
    }

    /// Lista os tokens da org, mais novos primeiro.
    pub fn listar(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<TokenAdmin>> {
        let texto_org = org.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM admin_tokens
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2)
                  ORDER BY id LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha)?;
            let mut tokens = Vec::new();
            for linha in linhas {
                tokens.push(linha?);
            }
            Ok(tokens)
        })
    }

    /// Revoga o token da org. Revogar de novo não é erro (idempotente).
    pub fn revogar(&self, org: OrgId, id: AdminTokenId) -> Resultado<()> {
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE admin_tokens SET revoked_at = COALESCE(revoked_at, ?3)
                  WHERE id = ?1 AND org_id = ?2",
                params![texto_id, texto_org, agora],
            )
            .map(|_| ())
            .map_err(ErroStore::from)
        })?;
        Ok(())
    }
}

fn linha(linha: &Row<'_>) -> rusqlite::Result<TokenAdmin> {
    Ok(TokenAdmin {
        id: id_da_coluna(linha, 0)?,
        org_id: id_opcional(linha, 1)?,
        rotulo: linha.get(2)?,
        hash: linha.get(3)?,
        criado_em: linha.get(4)?,
        revogado_em: linha.get(5)?,
    })
}
