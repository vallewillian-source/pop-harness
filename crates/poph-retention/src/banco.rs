//! Adaptador da retenção sobre o `poph-store`: é o que a CLI e o servidor usam.
//!
//! A retenção não conhece SQL: quem apaga é o repositório `retencao()` do `poph-store`, numa
//! transação por org. Aqui só se traduz tipo e erro, e se acha a pasta de blobs da org.

use std::path::{Path, PathBuf};

use poph_store::{NewAuditEntry, Page, Store, StoreError};
use poph_types::OrgId;

use crate::armazem::{Armazem, OrgRetencao, VarreduraBanco};
use crate::erro::ErroRetencao;

/// Tamanho da página ao listar as orgs.
const PAGINA: u32 = 200;

/// `Armazem` sobre um `Store` aberto e a pasta de blobs da instância.
pub struct BancoHarness<'a> {
    store: &'a Store,
    blobs: PathBuf,
}

impl<'a> BancoHarness<'a> {
    /// `blobs` é a pasta de arquivos (`dados.blobs`); cada org fica numa subpasta com o id dela.
    pub fn novo(store: &'a Store, blobs: impl AsRef<Path>) -> Self {
        Self {
            store,
            blobs: blobs.as_ref().to_path_buf(),
        }
    }
}

impl Armazem for BancoHarness<'_> {
    fn orgs(&self) -> Result<Vec<OrgRetencao>, ErroRetencao> {
        let mut todas = Vec::new();
        let mut apos = None;
        loop {
            let pagina = self
                .store
                .orgs()
                .list(Page {
                    after: apos,
                    limit: PAGINA,
                })
                .map_err(banco)?;
            let cheia = pagina.len() as u32 == PAGINA;
            apos = pagina.last().map(|org| org.id.as_uuid());
            todas.extend(pagina.into_iter().map(|org| OrgRetencao {
                id: org.id,
                slug: org.slug,
                config_json: org.config_json,
            }));
            if !cheia {
                break;
            }
        }
        Ok(todas)
    }

    fn org(&self, id: OrgId) -> Result<OrgRetencao, ErroRetencao> {
        match self.store.orgs().get(id) {
            Ok(org) => Ok(OrgRetencao {
                id: org.id,
                slug: org.slug,
                config_json: org.config_json,
            }),
            Err(erro) if erro.is_not_found() => Err(ErroRetencao::OrgInexistente(id.to_string())),
            Err(erro) => Err(banco(erro)),
        }
    }

    fn varrer(
        &self,
        org: OrgId,
        corte_ms: i64,
        simular: bool,
    ) -> Result<VarreduraBanco, ErroRetencao> {
        let varredura = self
            .store
            .retention()
            .scan_table(org, corte_ms, simular)
            .map_err(banco)?;
        Ok(VarreduraBanco {
            mensagens: varredura.messages,
            anexos: varredura.attachments,
            turnos: varredura.turns,
            inbox: varredura.inbox,
            outbox: varredura.outbox,
            conversas: varredura.conversations,
            ids_conversas: varredura.conversation_ids,
            hashes_referenciados: varredura.hashes_referenciados,
        })
    }

    fn pasta_blobs(&self, org: OrgId) -> PathBuf {
        self.blobs.join(org.to_string())
    }

    fn registrar_auditoria(
        &self,
        org: OrgId,
        ator_tipo: &str,
        detalhe_json: &str,
    ) -> Result<(), ErroRetencao> {
        self.store
            .audit()
            .register(
                Some(org),
                NewAuditEntry {
                    ator_tipo: ator_tipo.to_owned(),
                    actor_id: None,
                    acao: "retention".to_owned(),
                    entity: "org".to_owned(),
                    entity_id: Some(org.to_string()),
                    detail_json: detalhe_json.to_owned(),
                },
            )
            .map(|_| ())
            .map_err(banco)
    }
}

fn banco(erro: StoreError) -> ErroRetencao {
    ErroRetencao::Banco(erro.to_string())
}
