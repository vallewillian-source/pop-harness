//! Carrinho de orçamento por conversa (HR-19).
//!
//! Regras que este repositório garante:
//! - **Um carrinho aberto por conversa.** Ele nasce na primeira adição; conversa nova não tem
//!   carrinho, então começa vazia.
//! - **Preço é do registro, não do pedido.** Quem chama passa só o id do produto e a quantidade.
//!   O preço é lido do catálogo dentro da mesma escrita e gravado na linha (`unit_price_cents`, o
//!   preço praticado). O total é a soma de quantidade × preço de cada linha, calculada aqui.
//! - **Mesmo produto soma na mesma linha** e o preço da linha passa a ser o do catálogo no momento
//!   da última adição (o que o cliente está vendo agora).
//! - **Só entra produto ativo e visível ao agente** (a mesma regra da busca: vínculo de categoria).
//! - **Quantidade de 1 a 999 por linha.**
//! - **`org_id` em toda assinatura**, e conversa de outra org não tem carrinho para quem não é dela.

use poph_types::{AgentId, CartId, ConversationId, OrgId, ProductId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::repos::{id_opcional, pertence};

/// Quantidade máxima por linha do carrinho.
pub const LIMITE_QUANTIDADE: u32 = 999;

/// Uma linha do carrinho, com o que o cliente precisa ver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemCarrinho {
    /// Produto da linha. `None` quando o produto foi apagado do catálogo depois de entrar no
    /// carrinho (a linha continua com o preço praticado).
    pub produto: Option<ProductId>,
    /// Nome do produto no catálogo (ou um aviso, quando ele saiu do catálogo).
    pub nome: String,
    /// Quantidade.
    pub quantidade: u32,
    /// Preço unitário praticado, em centavos.
    pub preco_unitario_centavos: i64,
    /// Quantidade × preço unitário, em centavos.
    pub subtotal_centavos: i64,
}

/// Estado do carrinho de uma conversa, com o total calculado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumoCarrinho {
    /// Carrinho aberto da conversa, quando existe.
    pub carrinho: Option<CartId>,
    /// Linhas, na ordem em que entraram.
    pub itens: Vec<ItemCarrinho>,
    /// Soma dos subtotais, em centavos.
    pub total_centavos: i64,
}

impl ResumoCarrinho {
    fn vazio() -> Self {
        Self {
            carrinho: None,
            itens: Vec::new(),
            total_centavos: 0,
        }
    }
}

/// Acesso ao carrinho de orçamento.
pub struct Carrinhos<'a>(pub(crate) &'a Store);

impl Carrinhos<'_> {
    /// Estado atual do carrinho da conversa (vazio quando não há carrinho aberto).
    pub fn resumo(&self, org: OrgId, conversa: ConversationId) -> Resultado<ResumoCarrinho> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        self.0
            .leitura(move |conn| resumo_em(conn, &texto_org, &texto_conversa))
    }

    /// Adiciona uma unidade ou mais de um produto ao carrinho da conversa e devolve o estado novo.
    ///
    /// Falha sem gravar nada quando: a quantidade está fora de 1..=999; a conversa não é da org; o
    /// produto não existe, está inativo ou não é visível ao agente; ou a soma passaria de 999.
    pub fn adicionar(
        &self,
        org: OrgId,
        conversa: ConversationId,
        agente: AgentId,
        produto: ProductId,
        quantidade: u32,
    ) -> Resultado<ResumoCarrinho> {
        validar_quantidade(quantidade)?;
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let texto_agente = agente.to_string();
        let texto_produto = produto.to_string();
        let agora = self.0.agora();
        let novo_id = CartId::generate().to_string();
        let novo_item = uuid::Uuid::now_v7().to_string();
        self.0.escrita(move |conn| {
            pertence(
                conn,
                "conversations",
                "conversa",
                &texto_org,
                &texto_conversa,
            )?;
            let preco: Option<i64> = conn
                .query_row(
                    "SELECT p.price_cents FROM products p
                      WHERE p.org_id = ?1 AND p.id = ?2 AND p.status = 'active'
                        AND (NOT EXISTS (SELECT 1 FROM agent_product_categories v
                                          WHERE v.org_id = ?1 AND v.agent_id = ?3)
                             OR p.category_id IN (SELECT v.category_id FROM agent_product_categories v
                                                   WHERE v.org_id = ?1 AND v.agent_id = ?3))",
                    params![texto_org, texto_produto, texto_agente],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?;
            let Some(preco) = preco else {
                return Err(ErroStore::Dominio(poph_types::DomainError::NotFound {
                    entity: "produto disponível para este agente",
                    id: texto_produto,
                }));
            };

            let carrinho = match carrinho_aberto(conn, &texto_org, &texto_conversa)? {
                Some(id) => id,
                None => {
                    conn.execute(
                        "INSERT INTO carts
                            (id, org_id, conversation_id, status, version, created_at, updated_at)
                         VALUES (?1, ?2, ?3, 'open', 1, ?4, ?4)",
                        params![novo_id, texto_org, texto_conversa, agora],
                    )
                    .map_err(ErroStore::from)?;
                    novo_id.clone()
                }
            };

            let existente: Option<(String, i64)> = conn
                .query_row(
                    "SELECT id, quantity FROM cart_items
                      WHERE org_id = ?1 AND cart_id = ?2 AND product_id = ?3",
                    params![texto_org, carrinho, texto_produto],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?;
            match existente {
                Some((linha, atual)) => {
                    let total = atual + i64::from(quantidade);
                    if total > i64::from(LIMITE_QUANTIDADE) {
                        return Err(limite_excedido());
                    }
                    conn.execute(
                        "UPDATE cart_items SET quantity = ?1, unit_price_cents = ?2, updated_at = ?3
                          WHERE org_id = ?4 AND id = ?5",
                        params![total, preco, agora, texto_org, linha],
                    )
                    .map_err(ErroStore::from)?;
                }
                None => {
                    conn.execute(
                        "INSERT INTO cart_items
                            (id, org_id, cart_id, product_id, quantity, unit_price_cents,
                             created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![
                            novo_item,
                            texto_org,
                            carrinho,
                            texto_produto,
                            i64::from(quantidade),
                            preco,
                            agora
                        ],
                    )
                    .map_err(ErroStore::from)?;
                }
            }
            tocar_carrinho(conn, &texto_org, &carrinho, agora)?;
            resumo_em(conn, &texto_org, &texto_conversa)
        })
    }

    /// Tira unidades de um produto do carrinho (sem quantidade, tira a linha inteira) e devolve o
    /// estado novo. Produto que não está no carrinho falha sem gravar nada.
    pub fn remover(
        &self,
        org: OrgId,
        conversa: ConversationId,
        produto: ProductId,
        quantidade: Option<u32>,
    ) -> Resultado<ResumoCarrinho> {
        if let Some(quantidade) = quantidade {
            validar_quantidade(quantidade)?;
        }
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let texto_produto = produto.to_string();
        let agora = self.0.agora();
        self.0.escrita(move |conn| {
            pertence(
                conn,
                "conversations",
                "conversa",
                &texto_org,
                &texto_conversa,
            )?;
            let carrinho =
                carrinho_aberto(conn, &texto_org, &texto_conversa)?.ok_or_else(|| {
                    ErroStore::Dominio(poph_types::DomainError::NotFound {
                        entity: "item do carrinho",
                        id: texto_produto.clone(),
                    })
                })?;
            let (linha, atual): (String, i64) = conn
                .query_row(
                    "SELECT id, quantity FROM cart_items
                      WHERE org_id = ?1 AND cart_id = ?2 AND product_id = ?3",
                    params![texto_org, carrinho, texto_produto],
                    |linha| Ok((linha.get(0)?, linha.get(1)?)),
                )
                .optional()
                .map_err(ErroStore::from)?
                .ok_or_else(|| {
                    ErroStore::Dominio(poph_types::DomainError::NotFound {
                        entity: "item do carrinho",
                        id: texto_produto.clone(),
                    })
                })?;
            match quantidade {
                Some(tirar) if i64::from(tirar) < atual => {
                    conn.execute(
                        "UPDATE cart_items SET quantity = ?1, updated_at = ?2
                          WHERE org_id = ?3 AND id = ?4",
                        params![atual - i64::from(tirar), agora, texto_org, linha],
                    )
                    .map_err(ErroStore::from)?;
                }
                _ => {
                    conn.execute(
                        "DELETE FROM cart_items WHERE org_id = ?1 AND id = ?2",
                        params![texto_org, linha],
                    )
                    .map_err(ErroStore::from)?;
                }
            }
            tocar_carrinho(conn, &texto_org, &carrinho, agora)?;
            resumo_em(conn, &texto_org, &texto_conversa)
        })
    }

    /// Esvazia o carrinho aberto da conversa. Devolve quantas linhas saíram e o estado novo (vazio).
    /// O carrinho continua aberto, pronto para a próxima adição.
    pub fn limpar(&self, org: OrgId, conversa: ConversationId) -> Resultado<(u64, ResumoCarrinho)> {
        let texto_org = org.to_string();
        let texto_conversa = conversa.to_string();
        let agora = self.0.agora();
        self.0.escrita(move |conn| {
            pertence(
                conn,
                "conversations",
                "conversa",
                &texto_org,
                &texto_conversa,
            )?;
            let Some(carrinho) = carrinho_aberto(conn, &texto_org, &texto_conversa)? else {
                return Ok((0, ResumoCarrinho::vazio()));
            };
            let saiu = conn
                .execute(
                    "DELETE FROM cart_items WHERE org_id = ?1 AND cart_id = ?2",
                    params![texto_org, carrinho],
                )
                .map_err(ErroStore::from)?;
            tocar_carrinho(conn, &texto_org, &carrinho, agora)?;
            Ok((saiu as u64, resumo_em(conn, &texto_org, &texto_conversa)?))
        })
    }
}

fn validar_quantidade(quantidade: u32) -> Resultado<()> {
    if (1..=LIMITE_QUANTIDADE).contains(&quantidade) {
        return Ok(());
    }
    Err(ErroStore::Dominio(poph_types::DomainError::Validation {
        field: "quantidade",
        reason: format!("informe de 1 a {LIMITE_QUANTIDADE} unidades"),
    }))
}

fn limite_excedido() -> ErroStore {
    ErroStore::Dominio(poph_types::DomainError::Validation {
        field: "quantidade",
        reason: format!("o carrinho passaria de {LIMITE_QUANTIDADE} unidades deste produto"),
    })
}

/// Id do carrinho aberto da conversa, como texto.
fn carrinho_aberto(conn: &Connection, org: &str, conversa: &str) -> Resultado<Option<String>> {
    conn.query_row(
        "SELECT id FROM carts
          WHERE org_id = ?1 AND conversation_id = ?2 AND status = 'open'",
        params![org, conversa],
        |linha| linha.get(0),
    )
    .optional()
    .map_err(ErroStore::from)
}

fn tocar_carrinho(conn: &Connection, org: &str, carrinho: &str, agora: i64) -> Resultado<()> {
    conn.execute(
        "UPDATE carts SET updated_at = ?1, version = version + 1
          WHERE org_id = ?2 AND id = ?3",
        params![agora, org, carrinho],
    )
    .map_err(ErroStore::from)?;
    Ok(())
}

/// Lê o carrinho aberto e as linhas, e calcula o total. Roda dentro da escrita (para devolver o
/// estado já gravado) e dentro da leitura.
fn resumo_em(conn: &Connection, org: &str, conversa: &str) -> Resultado<ResumoCarrinho> {
    let Some(carrinho) = carrinho_aberto(conn, org, conversa)? else {
        return Ok(ResumoCarrinho::vazio());
    };
    let mut consulta = conn.prepare(
        "SELECT ci.product_id, p.name, ci.quantity, ci.unit_price_cents
           FROM cart_items ci
           LEFT JOIN products p ON p.org_id = ci.org_id AND p.id = ci.product_id
          WHERE ci.org_id = ?1 AND ci.cart_id = ?2
          ORDER BY ci.created_at, ci.id",
    )?;
    let linhas = consulta.query_map(params![org, carrinho], |linha| {
        let quantidade: i64 = linha.get(2)?;
        let preco: i64 = linha.get(3)?;
        Ok((
            id_opcional::<ProductId>(linha, 0)?,
            linha.get::<_, Option<String>>(1)?,
            quantidade,
            preco,
        ))
    })?;
    let mut itens = Vec::new();
    let mut total: i64 = 0;
    for linha in linhas {
        let (produto, nome, quantidade, preco) = linha?;
        let subtotal = quantidade
            .checked_mul(preco)
            .ok_or_else(|| ErroStore::Interno("subtotal do carrinho estourou".to_owned()))?;
        total = total
            .checked_add(subtotal)
            .ok_or_else(|| ErroStore::Interno("total do carrinho estourou".to_owned()))?;
        itens.push(ItemCarrinho {
            produto,
            nome: nome.unwrap_or_else(|| "produto removido do catálogo".to_owned()),
            quantidade: quantidade.clamp(0, i64::from(u32::MAX)) as u32,
            preco_unitario_centavos: preco,
            subtotal_centavos: subtotal,
        });
    }
    Ok(ResumoCarrinho {
        carrinho: Some(carrinho.parse::<CartId>().map_err(ErroStore::from)?),
        itens,
        total_centavos: total,
    })
}
