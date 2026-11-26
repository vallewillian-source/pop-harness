use poph_types::{AgentId, OrgId, ProductCategoryId, ProductId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::error::{Resultado, StoreError};
use crate::model::{NewProduct, Page, Product, ProductChange, ProductForIndex, Status, slugificar};
use crate::repos::{belongs, column_id, column_status, column_u64, conflict_or_missing};

const COLUMNS: &str = "id, org_id, category_id, name, slug, aliases_json, description, \
    price_text, price_cents, unit, includes, excludes, conditions, tags_json, status, version, \
    created_at, updated_at";

const FIELDS_PRODUCT: &[(&str, &str)] = &[
    ("name", "name"),
    ("price_text", "price_text"),
    ("price_cents", "price_cents"),
    ("status", "status"),
];

const LIMIT_NAME: usize = 160;

pub struct Products<'a>(pub(crate) &'a Store);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProductReferences {
    pub cart_items: u64,
    pub carts: u64,
}

impl Products<'_> {
    pub fn create(&self, org: OrgId, new: NewProduct) -> Resultado<Product> {
        let nome = validate_name(&new.name)?;
        let slug = validate_slug(new.slug.as_deref().unwrap_or(""), &nome)?;
        validate_price(new.price_cents)?;
        let Some(category_id) = new.category_id else {
            return Err(StoreError::Domain(poph_types::DomainError::Validation {
                field: "categoria do produto",
                reason: "informe a categoria (--category)".to_owned(),
            }));
        };
        let aliases = list_json(&new.aliases);
        let tags = list_json(&new.tags);
        let id = ProductId::generate();
        let now = self.0.now();
        let text_id = id.to_string();
        let text_org = org.to_string();
        let category = category_id.to_string();
        self.0.write(move |conn| {
            belongs(
                conn,
                "product_categories",
                "categoria de produto",
                &text_org,
                &category,
            )?;
            conn.execute(
                "INSERT INTO products
                    (id, org_id, category_id, name, slug, aliases_json, description, price_text,
                     price_cents, unit, includes, excludes, conditions, tags_json, status, version,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'active', 1,
                         ?15, ?15)",
                params![
                    text_id,
                    text_org,
                    category,
                    nome,
                    slug,
                    aliases,
                    new.description.trim(),
                    new.price_text.trim(),
                    new.price_cents,
                    text_or_null(&new.unit),
                    text_or_null(&new.includes),
                    text_or_null(&new.exclui),
                    text_or_null(&new.conditions),
                    tags,
                    now
                ],
            )
            .map_err(|error| StoreError::from_write_with("slug", FIELDS_PRODUCT, error))?;
            Ok(())
        })?;
        self.get(org, id)
    }

    pub fn get(&self, org: OrgId, id: ProductId) -> Resultado<Product> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM products WHERE id = ?1 AND org_id = ?2"),
                params![text_id, text_org],
                product_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "produto", &id.to_string())
    }

    pub fn by_slug(&self, org: OrgId, slug: &str) -> Resultado<Product> {
        let text_org = org.to_string();
        let slug = slug.trim().to_lowercase();
        let slug_error = slug.clone();
        let located = self.0.read(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM products WHERE org_id = ?1 AND slug = ?2"),
                params![text_org, slug],
                product_row,
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        super::require(located, "produto", &slug_error)
    }

    pub fn list(&self, org: OrgId, page: Page) -> Resultado<Vec<Product>> {
        self.list_with(org, page, false)
    }

    pub fn list_active(&self, org: OrgId, page: Page) -> Resultado<Vec<Product>> {
        self.list_with(org, page, true)
    }

    fn list_with(&self, org: OrgId, page: Page, only_active: bool) -> Resultado<Vec<Product>> {
        let text_org = org.to_string();
        let after = page.after.map(|id| id.to_string());
        let limit = i64::from(page.limit);
        self.0.read(move |conn| {
            let filter = if only_active {
                "AND status = 'active'"
            } else {
                ""
            };
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM products
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2) {filter}
                  ORDER BY id LIMIT ?3"
            ))?;
            let rows = statement.query_map(params![text_org, after, limit], product_row)?;
            let mut products = Vec::new();
            for row in rows {
                products.push(row?);
            }
            Ok(products)
        })
    }

    pub fn update(
        &self,
        org: OrgId,
        id: ProductId,
        expected: u64,
        change: ProductChange,
    ) -> Resultado<Product> {
        let nome = match &change.name {
            Some(nome) => Some(validate_name(nome)?),
            None => None,
        };
        let slug = match &change.slug {
            Some(slug) => Some(validate_slug(slug, nome.as_deref().unwrap_or("produto"))?),
            None => None,
        };
        if let Some(cents) = change.price_cents {
            validate_price(cents)?;
        }
        let aliases = change.aliases.as_ref().map(|list| list_json(list));
        let tags = change.tags.as_ref().map(|list| list_json(list));
        let category = change.category_id.map(|category| category.to_string());
        let status = change.status.map(Status::as_text);
        let now = self.0.now();
        let text_org = org.to_string();
        let text_id = id.to_string();
        let expected_i64 = expected as i64;
        let effect = self.0.write(move |conn| {
            if let Some(category) = &category {
                belongs(
                    conn,
                    "product_categories",
                    "categoria de produto",
                    &text_org,
                    category,
                )?;
            }
            conn.execute(
                "UPDATE products
                    SET category_id = COALESCE(?3, category_id),
                        name = COALESCE(?4, name),
                        slug = COALESCE(?5, slug),
                        aliases_json = COALESCE(?6, aliases_json),
                        description = COALESCE(?7, description),
                        price_text = COALESCE(?8, price_text),
                        price_cents = COALESCE(?9, price_cents),
                        unit = COALESCE(?10, unit),
                        includes = COALESCE(?11, includes),
                        excludes = COALESCE(?12, excludes),
                        conditions = COALESCE(?13, conditions),
                        tags_json = COALESCE(?14, tags_json),
                        status = COALESCE(?15, status),
                        version = version + 1,
                        updated_at = ?16
                  WHERE org_id = ?1 AND id = ?2 AND version = ?17",
                params![
                    text_org,
                    text_id,
                    category,
                    nome,
                    slug,
                    aliases,
                    change.description.map(|texto| texto.trim().to_owned()),
                    change.price_text.map(|texto| texto.trim().to_owned()),
                    change.price_cents,
                    change.unit.as_deref().map(text_or_null),
                    change.includes.as_deref().map(text_or_null),
                    change.exclui.as_deref().map(text_or_null),
                    change.conditions.as_deref().map(text_or_null),
                    tags,
                    status,
                    now,
                    expected_i64
                ],
            )
            .map_err(|error| StoreError::from_write_with("slug", FIELDS_PRODUCT, error))
        })?;
        if effect == 0 {
            return Err(self.conflict(org, id, expected));
        }
        self.get(org, id)
    }

    pub fn delete(&self, org: OrgId, id: ProductId) -> Resultado<()> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let deleted = self.0.write(move |conn| {
            conn.execute(
                "DELETE FROM products WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
            )
            .map_err(StoreError::from)
        })?;
        if deleted == 0 {
            return Err(not_found(&id.to_string()));
        }
        Ok(())
    }

    pub fn references(&self, org: OrgId, id: ProductId) -> Resultado<ProductReferences> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        self.0.read(move |conn| {
            let (items, carts): (i64, i64) = conn.query_row(
                "SELECT COUNT(*), COUNT(DISTINCT cart_id) FROM cart_items
                  WHERE org_id = ?1 AND product_id = ?2",
                params![text_org, text_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok(ProductReferences {
                cart_items: items as u64,
                carts: carts as u64,
            })
        })
    }

    pub fn categories_visible(
        &self,
        org: OrgId,
        agent: AgentId,
    ) -> Resultado<Option<Vec<ProductCategoryId>>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT category_id FROM agent_product_categories
                  WHERE org_id = ?1 AND agent_id = ?2 ORDER BY category_id",
            )?;
            let rows = statement.query_map(params![text_org, text_agent], |row| {
                column_id::<ProductCategoryId>(row, 0)
            })?;
            let mut categories = Vec::new();
            for row in rows {
                categories.push(row?);
            }
            Ok(if categories.is_empty() {
                None
            } else {
                Some(categories)
            })
        })
    }

    pub fn agent_visible(&self, org: OrgId, agent: AgentId) -> Resultado<Vec<Product>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM products p
                  WHERE p.org_id = ?1 AND p.status = 'active'
                    AND (NOT EXISTS (SELECT 1 FROM agent_product_categories v
                                      WHERE v.org_id = ?1 AND v.agent_id = ?2)
                         OR p.category_id IN (SELECT v.category_id FROM agent_product_categories v
                                               WHERE v.org_id = ?1 AND v.agent_id = ?2))
                  ORDER BY lower(p.name), p.id"
            ))?;
            let rows = statement.query_map(params![text_org, text_agent], product_row)?;
            let mut products = Vec::new();
            for row in rows {
                products.push(row?);
            }
            Ok(products)
        })
    }

    pub fn for_index(&self, org: OrgId, agent: AgentId) -> Resultado<Vec<ProductForIndex>> {
        let text_org = org.to_string();
        let text_agent = agent.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT p.id, p.name, c.name, p.aliases_json, p.description, p.tags_json, p.version
                   FROM products p
                   JOIN product_categories c ON c.org_id = p.org_id AND c.id = p.category_id
                  WHERE p.org_id = ?1 AND p.status = 'active'
                    AND (NOT EXISTS (SELECT 1 FROM agent_product_categories v
                                      WHERE v.org_id = ?1 AND v.agent_id = ?2)
                         OR p.category_id IN (SELECT v.category_id FROM agent_product_categories v
                                               WHERE v.org_id = ?1 AND v.agent_id = ?2))
                  ORDER BY lower(p.name), p.id",
            )?;
            let rows = statement.query_map(params![text_org, text_agent], |row| {
                let aliases: String = row.get(3)?;
                let descricao: String = row.get::<_, Option<String>>(4)?.unwrap_or_default();
                let tags: String = row.get(5)?;
                let nome: String = row.get(1)?;
                Ok(ProductForIndex {
                    id: column_id(row, 0)?,
                    name: nome.clone(),
                    category: row.get(2)?,
                    texto: index_text(
                        &nome,
                        &serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default(),
                        &descricao,
                        &serde_json::from_str::<Vec<String>>(&tags).unwrap_or_default(),
                    ),
                    version: column_u64(row, 6)?,
                })
            })?;
            let mut products = Vec::new();
            for row in rows {
                products.push(row?);
            }
            Ok(products)
        })
    }

    pub fn active_by_ids(&self, org: OrgId, ids: &[ProductId]) -> Resultado<Vec<Product>> {
        let mut saida = Vec::with_capacity(ids.len());
        for id in ids {
            let text_org = org.to_string();
            let text_id = id.to_string();
            let finding = self.0.read(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM products
                          WHERE org_id = ?1 AND id = ?2 AND status = 'active'"
                    ),
                    params![text_org, text_id],
                    product_row,
                )
                .optional()
                .map_err(StoreError::from)
            })?;
            if let Some(product) = finding {
                saida.push(product);
            }
        }
        Ok(saida)
    }

    pub fn active_of_category(
        &self,
        org: OrgId,
        category: ProductCategoryId,
    ) -> Resultado<Vec<ProductId>> {
        let text_org = org.to_string();
        let text_category = category.to_string();
        self.0.read(move |conn| {
            let mut statement = conn.prepare(
                "SELECT id FROM products
                  WHERE org_id = ?1 AND category_id = ?2 AND status = 'active'
                  ORDER BY id",
            )?;
            let rows = statement.query_map(params![text_org, text_category], |row| {
                column_id::<ProductId>(row, 0)
            })?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })
    }

    pub fn category_name(&self, org: OrgId, id: ProductCategoryId) -> Resultado<String> {
        let text_org = org.to_string();
        let text_id = id.to_string();
        let nome = self.0.read(move |conn| {
            conn.query_row(
                "SELECT name FROM product_categories WHERE org_id = ?1 AND id = ?2",
                params![text_org, text_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
        })?;
        Ok(nome.unwrap_or_else(|| "sem categoria".to_owned()))
    }

    fn conflict(&self, org: OrgId, id: ProductId, expected: u64) -> StoreError {
        self.0
            .read(move |conn| {
                Ok(conflict_or_missing(
                    conn,
                    "products",
                    "produto",
                    &id.to_string(),
                    &org.to_string(),
                    expected,
                ))
            })
            .unwrap_or_else(|error| error)
    }
}

pub fn index_text(nome: &str, aliases: &[String], descricao: &str, tags: &[String]) -> String {
    let mut texto = nome.to_owned();
    if !aliases.is_empty() {
        texto.push_str(&format!("\nTambém chamado de: {}", aliases.join(", ")));
    }
    if !descricao.trim().is_empty() {
        texto.push_str(&format!("\nDescrição: {}", descricao.trim()));
    }
    if !tags.is_empty() {
        texto.push_str(&format!("\nTags: {}", tags.join(", ")));
    }
    texto
}

fn validate_name(nome: &str) -> Resultado<String> {
    let clean_value = nome.trim();
    if clean_value.is_empty() {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: "nome do produto",
            reason: "não pode ficar em branco".to_owned(),
        }));
    }
    if clean_value.chars().count() > LIMIT_NAME {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: "nome do produto",
            reason: format!(
                "até {LIMIT_NAME} caracteres; vieram {}",
                clean_value.chars().count()
            ),
        }));
    }
    Ok(clean_value.to_owned())
}

fn validate_slug(slug: &str, nome: &str) -> Resultado<String> {
    let derived = if slug.trim().is_empty() {
        slugificar(nome)
    } else {
        slugificar(slug)
    };
    let size = derived.chars().count();
    if !(3..=64).contains(&size) {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: "slug do produto",
            reason: format!(
                "precisa de 3 a 64 caracteres (letras sem acento, números e hífen); o derivado \
                 ficou com {size}"
            ),
        }));
    }
    Ok(derived)
}

fn validate_price(cents: i64) -> Resultado<()> {
    if cents < 0 {
        return Err(StoreError::Domain(poph_types::DomainError::Validation {
            field: "preço do produto",
            reason: "não pode ser negativo (o valor é em centavos)".to_owned(),
        }));
    }
    Ok(())
}

fn list_json(items: &[String]) -> String {
    let clean_rows: Vec<String> = items
        .iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect();
    serde_json::to_string(&clean_rows).unwrap_or_else(|_| "[]".to_owned())
}

fn text_or_null(texto: &str) -> Option<String> {
    let clean_value = texto.trim();
    (!clean_value.is_empty()).then(|| clean_value.to_owned())
}

fn not_found(id: &str) -> StoreError {
    StoreError::Domain(poph_types::DomainError::NotFound {
        entity: "produto",
        id: id.to_owned(),
    })
}

fn product_row(row: &Row<'_>) -> rusqlite::Result<Product> {
    let aliases: String = row.get(5)?;
    let tags: String = row.get(13)?;
    Ok(Product {
        id: column_id(row, 0)?,
        org_id: column_id(row, 1)?,
        category_id: column_id(row, 2)?,
        name: row.get(3)?,
        slug: row.get(4)?,
        aliases: serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default(),
        description: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        price_text: row.get(7)?,
        price_cents: row.get(8)?,
        unit: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        includes: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        exclui: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
        conditions: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
        tags: serde_json::from_str::<Vec<String>>(&tags).unwrap_or_default(),
        status: column_status(row, 14)?,
        version: column_u64(row, 15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}
