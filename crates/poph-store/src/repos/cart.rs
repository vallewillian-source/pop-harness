use poph_types::{AgentId, CartId, ConversationId, OrgId, ProductId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::repos::{belongs, id_optional};

pub const LIMIT_QUANTITY: u32 = 999;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartItem {
    pub product: Option<ProductId>,
    pub nome: String,
    pub quantity: u32,
    pub price_unit_cents: i64,
    pub subtotal_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartSummary {
    pub cart: Option<CartId>,
    pub items: Vec<CartItem>,
    pub total_cents: i64,
}

impl CartSummary {
    fn empty() -> Self {
        Self {
            cart: None,
            items: Vec::new(),
            total_cents: 0,
        }
    }
}

pub struct Carts<'a>(pub(crate) &'a Store);

impl Carts<'_> {
    pub fn summary(&self, org: OrgId, conversation: ConversationId) -> Resultado<CartSummary> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        self.0
            .read(move |conn| summary_at(conn, &text_org, &text_conversation))
    }

    pub fn add(
        &self,
        org: OrgId,
        conversation: ConversationId,
        agent: AgentId,
        product: ProductId,
        quantity: u32,
    ) -> Resultado<CartSummary> {
        validate_quantity(quantity)?;
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let text_agent = agent.to_string();
        let text_product = product.to_string();
        let now = self.0.now();
        let new_id = CartId::generate().to_string();
        let new_item = uuid::Uuid::now_v7().to_string();
        self.0.write(move |conn| {
            belongs(
                conn,
                "conversations",
                "conversa",
                &text_org,
                &text_conversation,
            )?;
            let price: Option<i64> = conn
                .query_row(
                    "SELECT p.price_cents FROM products p
                      WHERE p.org_id = ?1 AND p.id = ?2 AND p.status = 'active'
                        AND (NOT EXISTS (SELECT 1 FROM agent_product_categories v
                                          WHERE v.org_id = ?1 AND v.agent_id = ?3)
                             OR p.category_id IN (SELECT v.category_id FROM agent_product_categories v
                                                   WHERE v.org_id = ?1 AND v.agent_id = ?3))",
                    params![text_org, text_product, text_agent],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)?;
            let Some(price) = price else {
                return Err(StoreError::Domain(poph_types::DomainError::NotFound {
                    entity: "produto disponível para este agente",
                    id: text_product,
                }));
            };

            let cart = match cart_open(conn, &text_org, &text_conversation)? {
                Some(id) => id,
                None => {
                    conn.execute(
                        "INSERT INTO carts
                            (id, org_id, conversation_id, status, version, created_at, updated_at)
                         VALUES (?1, ?2, ?3, 'open', 1, ?4, ?4)",
                        params![new_id, text_org, text_conversation, now],
                    )
                    .map_err(StoreError::from)?;
                    new_id.clone()
                }
            };

            let existing: Option<(String, i64)> = conn
                .query_row(
                    "SELECT id, quantity FROM cart_items
                      WHERE org_id = ?1 AND cart_id = ?2 AND product_id = ?3",
                    params![text_org, cart, text_product],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?;
            match existing {
                Some((row, current)) => {
                    let total = current + i64::from(quantity);
                    if total > i64::from(LIMIT_QUANTITY) {
                        return Err(limit_exceeded());
                    }
                    conn.execute(
                        "UPDATE cart_items SET quantity = ?1, unit_price_cents = ?2, updated_at = ?3
                          WHERE org_id = ?4 AND id = ?5",
                        params![total, price, now, text_org, row],
                    )
                    .map_err(StoreError::from)?;
                }
                None => {
                    conn.execute(
                        "INSERT INTO cart_items
                            (id, org_id, cart_id, product_id, quantity, unit_price_cents,
                             created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![
                            new_item,
                            text_org,
                            cart,
                            text_product,
                            i64::from(quantity),
                            price,
                            now
                        ],
                    )
                    .map_err(StoreError::from)?;
                }
            }
            touch_cart(conn, &text_org, &cart, now)?;
            summary_at(conn, &text_org, &text_conversation)
        })
    }

    pub fn remove(
        &self,
        org: OrgId,
        conversation: ConversationId,
        product: ProductId,
        quantity: Option<u32>,
    ) -> Resultado<CartSummary> {
        if let Some(quantity) = quantity {
            validate_quantity(quantity)?;
        }
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let text_product = product.to_string();
        let now = self.0.now();
        self.0.write(move |conn| {
            belongs(
                conn,
                "conversations",
                "conversa",
                &text_org,
                &text_conversation,
            )?;
            let cart = cart_open(conn, &text_org, &text_conversation)?.ok_or_else(|| {
                StoreError::Domain(poph_types::DomainError::NotFound {
                    entity: "item do carrinho",
                    id: text_product.clone(),
                })
            })?;
            let (row, current): (String, i64) = conn
                .query_row(
                    "SELECT id, quantity FROM cart_items
                      WHERE org_id = ?1 AND cart_id = ?2 AND product_id = ?3",
                    params![text_org, cart, text_product],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(StoreError::from)?
                .ok_or_else(|| {
                    StoreError::Domain(poph_types::DomainError::NotFound {
                        entity: "item do carrinho",
                        id: text_product.clone(),
                    })
                })?;
            match quantity {
                Some(tirar) if i64::from(tirar) < current => {
                    conn.execute(
                        "UPDATE cart_items SET quantity = ?1, updated_at = ?2
                          WHERE org_id = ?3 AND id = ?4",
                        params![current - i64::from(tirar), now, text_org, row],
                    )
                    .map_err(StoreError::from)?;
                }
                _ => {
                    conn.execute(
                        "DELETE FROM cart_items WHERE org_id = ?1 AND id = ?2",
                        params![text_org, row],
                    )
                    .map_err(StoreError::from)?;
                }
            }
            touch_cart(conn, &text_org, &cart, now)?;
            summary_at(conn, &text_org, &text_conversation)
        })
    }

    pub fn clean(&self, org: OrgId, conversation: ConversationId) -> Resultado<(u64, CartSummary)> {
        let text_org = org.to_string();
        let text_conversation = conversation.to_string();
        let now = self.0.now();
        self.0.write(move |conn| {
            belongs(
                conn,
                "conversations",
                "conversa",
                &text_org,
                &text_conversation,
            )?;
            let Some(cart) = cart_open(conn, &text_org, &text_conversation)? else {
                return Ok((0, CartSummary::empty()));
            };
            let saiu = conn
                .execute(
                    "DELETE FROM cart_items WHERE org_id = ?1 AND cart_id = ?2",
                    params![text_org, cart],
                )
                .map_err(StoreError::from)?;
            touch_cart(conn, &text_org, &cart, now)?;
            Ok((
                saiu as u64,
                summary_at(conn, &text_org, &text_conversation)?,
            ))
        })
    }
}

fn validate_quantity(quantity: u32) -> Resultado<()> {
    if (1..=LIMIT_QUANTITY).contains(&quantity) {
        return Ok(());
    }
    Err(StoreError::Domain(poph_types::DomainError::Validation {
        field: "quantidade",
        reason: format!("informe de 1 a {LIMIT_QUANTITY} unidades"),
    }))
}

fn limit_exceeded() -> StoreError {
    StoreError::Domain(poph_types::DomainError::Validation {
        field: "quantidade",
        reason: format!("o carrinho passaria de {LIMIT_QUANTITY} unidades deste produto"),
    })
}

fn cart_open(conn: &Connection, org: &str, conversation: &str) -> Resultado<Option<String>> {
    conn.query_row(
        "SELECT id FROM carts
          WHERE org_id = ?1 AND conversation_id = ?2 AND status = 'open'",
        params![org, conversation],
        |row| row.get(0),
    )
    .optional()
    .map_err(StoreError::from)
}

fn touch_cart(conn: &Connection, org: &str, cart: &str, now: i64) -> Resultado<()> {
    conn.execute(
        "UPDATE carts SET updated_at = ?1, version = version + 1
          WHERE org_id = ?2 AND id = ?3",
        params![now, org, cart],
    )
    .map_err(StoreError::from)?;
    Ok(())
}

fn summary_at(conn: &Connection, org: &str, conversation: &str) -> Resultado<CartSummary> {
    let Some(cart) = cart_open(conn, org, conversation)? else {
        return Ok(CartSummary::empty());
    };
    let mut statement = conn.prepare(
        "SELECT ci.product_id, p.name, ci.quantity, ci.unit_price_cents
           FROM cart_items ci
           LEFT JOIN products p ON p.org_id = ci.org_id AND p.id = ci.product_id
          WHERE ci.org_id = ?1 AND ci.cart_id = ?2
          ORDER BY ci.created_at, ci.id",
    )?;
    let rows = statement.query_map(params![org, cart], |row| {
        let quantity: i64 = row.get(2)?;
        let price: i64 = row.get(3)?;
        Ok((
            id_optional::<ProductId>(row, 0)?,
            row.get::<_, Option<String>>(1)?,
            quantity,
            price,
        ))
    })?;
    let mut items = Vec::new();
    let mut total: i64 = 0;
    for row in rows {
        let (product, nome, quantity, price) = row?;
        let subtotal = quantity
            .checked_mul(price)
            .ok_or_else(|| StoreError::Internal("subtotal do carrinho estourou".to_owned()))?;
        total = total
            .checked_add(subtotal)
            .ok_or_else(|| StoreError::Internal("total do carrinho estourou".to_owned()))?;
        items.push(CartItem {
            product,
            nome: nome.unwrap_or_else(|| "produto removido do catálogo".to_owned()),
            quantity: quantity.clamp(0, i64::from(u32::MAX)) as u32,
            price_unit_cents: price,
            subtotal_cents: subtotal,
        });
    }
    Ok(CartSummary {
        cart: Some(cart.parse::<CartId>().map_err(StoreError::from)?),
        items,
        total_cents: total,
    })
}
