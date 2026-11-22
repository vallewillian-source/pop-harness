//! Produtos do catálogo e a visão que o agente tem deles (HR-18).
//!
//! Regras que este repositório garante:
//! - **`org_id` em toda assinatura.** Produto e categoria são lidos sempre dentro da org; a chave
//!   estrangeira composta `(org_id, category_id)` impede produto apontando para categoria de outra
//!   org, e o repositório confere a categoria antes de gravar (erro de "não encontrado" em vez de
//!   erro cru de SQLite).
//! - **D-05 aplicado a produto:** inativo ou arquivado fica fora da busca, da ferramenta e da
//!   resposta. Continua listado e editável pela CLI/Console.
//! - **Visibilidade por categoria (`agent_product_categories`):** agente com categorias vinculadas
//!   só vê os produtos ativos delas; agente sem vínculo vê todos os produtos ativos da org. É a
//!   mesma regra do POP Atende (`core/skills/tools/product_tools.py`).
//! - **Versão para `If-Match`** e slug único por org.

use poph_types::{AgentId, OrgId, ProductCategoryId, ProductId};
use rusqlite::{OptionalExtension, Row, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::modelo::{
    MudancaProduto, NovoProduto, Pagina, Produto, ProdutoParaIndice, Status, slugificar,
};
use crate::repos::{conflito_ou_ausente, id_da_coluna, pertence, status_da_coluna, u64_da_coluna};

const COLUNAS: &str = "id, org_id, category_id, name, slug, aliases_json, description, \
    price_text, price_cents, unit, includes, excludes, conditions, tags_json, status, version, \
    created_at, updated_at";

/// Limite do nome do produto (o POP Atende não tem limite; aqui o erro precisa ser legível).
/// Colunas do produto que podem recusar um valor e o campo que a API expõe para cada uma.
const CAMPOS_PRODUTO: &[(&str, &str)] = &[
    ("name", "name"),
    ("price_text", "price_text"),
    ("price_cents", "price_cents"),
    ("status", "status"),
];

const LIMITE_NOME: usize = 160;

/// Acesso ao catálogo de produtos.
pub struct Produtos<'a>(pub(crate) &'a Store);

/// Quantas referências de carrinho um produto tem (ver `Produtos::referencias`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReferenciasProduto {
    /// Linhas de carrinho que apontam para o produto.
    pub itens_de_carrinho: u64,
    /// Carrinhos distintos que têm pelo menos uma dessas linhas.
    pub carrinhos: u64,
}

impl Produtos<'_> {
    /// Cria o produto na org. O slug vazio é derivado do nome.
    pub fn criar(&self, org: OrgId, novo: NovoProduto) -> Resultado<Produto> {
        let nome = validar_nome(&novo.nome)?;
        let slug = validar_slug(novo.slug.as_deref().unwrap_or(""), &nome)?;
        validar_preco(novo.preco_centavos)?;
        let Some(categoria_id) = novo.categoria_id else {
            return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
                campo: "categoria do produto",
                motivo: "informe a categoria (--category)".to_owned(),
            }));
        };
        let aliases = lista_json(&novo.aliases);
        let tags = lista_json(&novo.tags);
        let id = ProductId::generate();
        let agora = self.0.agora();
        let texto_id = id.to_string();
        let texto_org = org.to_string();
        let categoria = categoria_id.to_string();
        self.0.escrita(move |conn| {
            pertence(
                conn,
                "product_categories",
                "categoria de produto",
                &texto_org,
                &categoria,
            )?;
            conn.execute(
                "INSERT INTO products
                    (id, org_id, category_id, name, slug, aliases_json, description, price_text,
                     price_cents, unit, includes, excludes, conditions, tags_json, status, version,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'active', 1,
                         ?15, ?15)",
                params![
                    texto_id,
                    texto_org,
                    categoria,
                    nome,
                    slug,
                    aliases,
                    novo.descricao.trim(),
                    novo.preco_texto.trim(),
                    novo.preco_centavos,
                    texto_ou_nulo(&novo.unidade),
                    texto_ou_nulo(&novo.inclui),
                    texto_ou_nulo(&novo.exclui),
                    texto_ou_nulo(&novo.condicoes),
                    tags,
                    agora
                ],
            )
            .map_err(|erro| ErroStore::da_escrita_em("slug", CAMPOS_PRODUTO, erro))?;
            Ok(())
        })?;
        self.obter(org, id)
    }

    /// Busca o produto pelo id dentro da org (qualquer situação).
    pub fn obter(&self, org: OrgId, id: ProductId) -> Resultado<Produto> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM products WHERE id = ?1 AND org_id = ?2"),
                params![texto_id, texto_org],
                linha_produto,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "produto", &id.to_string())
    }

    /// Busca pelo slug dentro da org.
    pub fn por_slug(&self, org: OrgId, slug: &str) -> Resultado<Produto> {
        let texto_org = org.to_string();
        let slug = slug.trim().to_lowercase();
        let slug_erro = slug.clone();
        let encontrado = self.0.leitura(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUNAS} FROM products WHERE org_id = ?1 AND slug = ?2"),
                params![texto_org, slug],
                linha_produto,
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        super::exigir(encontrado, "produto", &slug_erro)
    }

    /// Lista os produtos da org em qualquer situação (a CLI precisa ver o inativo também).
    pub fn listar(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<Produto>> {
        self.listar_com(org, pagina, false)
    }

    /// Lista só os produtos ativos (o que a busca e a ferramenta enxergam).
    pub fn listar_ativos(&self, org: OrgId, pagina: Pagina) -> Resultado<Vec<Produto>> {
        self.listar_com(org, pagina, true)
    }

    fn listar_com(&self, org: OrgId, pagina: Pagina, so_ativos: bool) -> Resultado<Vec<Produto>> {
        let texto_org = org.to_string();
        let apos = pagina.apos.map(|id| id.to_string());
        let limite = i64::from(pagina.limite);
        self.0.leitura(move |conn| {
            let filtro = if so_ativos {
                "AND status = 'active'"
            } else {
                ""
            };
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM products
                  WHERE org_id = ?1 AND (?2 IS NULL OR id > ?2) {filtro}
                  ORDER BY id LIMIT ?3"
            ))?;
            let linhas = consulta.query_map(params![texto_org, apos, limite], linha_produto)?;
            let mut produtos = Vec::new();
            for linha in linhas {
                produtos.push(linha?);
            }
            Ok(produtos)
        })
    }

    /// Altera o produto com `If-Match`.
    pub fn atualizar(
        &self,
        org: OrgId,
        id: ProductId,
        esperada: u64,
        mudanca: MudancaProduto,
    ) -> Resultado<Produto> {
        let nome = match &mudanca.nome {
            Some(nome) => Some(validar_nome(nome)?),
            None => None,
        };
        let slug = match &mudanca.slug {
            Some(slug) => Some(validar_slug(slug, nome.as_deref().unwrap_or("produto"))?),
            None => None,
        };
        if let Some(centavos) = mudanca.preco_centavos {
            validar_preco(centavos)?;
        }
        let aliases = mudanca.aliases.as_ref().map(|lista| lista_json(lista));
        let tags = mudanca.tags.as_ref().map(|lista| lista_json(lista));
        let categoria = mudanca.categoria_id.map(|categoria| categoria.to_string());
        let status = mudanca.status.map(Status::como_texto);
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let esperada_i64 = esperada as i64;
        let efeito = self.0.escrita(move |conn| {
            if let Some(categoria) = &categoria {
                pertence(
                    conn,
                    "product_categories",
                    "categoria de produto",
                    &texto_org,
                    categoria,
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
                    texto_org,
                    texto_id,
                    categoria,
                    nome,
                    slug,
                    aliases,
                    mudanca.descricao.map(|texto| texto.trim().to_owned()),
                    mudanca.preco_texto.map(|texto| texto.trim().to_owned()),
                    mudanca.preco_centavos,
                    mudanca.unidade.as_deref().map(texto_ou_nulo),
                    mudanca.inclui.as_deref().map(texto_ou_nulo),
                    mudanca.exclui.as_deref().map(texto_ou_nulo),
                    mudanca.condicoes.as_deref().map(texto_ou_nulo),
                    tags,
                    status,
                    agora,
                    esperada_i64
                ],
            )
            .map_err(|erro| ErroStore::da_escrita_em("slug", CAMPOS_PRODUTO, erro))
        })?;
        if efeito == 0 {
            return Err(self.conflito(org, id, esperada));
        }
        self.obter(org, id)
    }

    /// Exclui o produto.
    pub fn excluir(&self, org: OrgId, id: ProductId) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let apagou = self.0.escrita(move |conn| {
            conn.execute(
                "DELETE FROM products WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
            )
            .map_err(ErroStore::from)
        })?;
        if apagou == 0 {
            return Err(nao_encontrado(&id.to_string()));
        }
        Ok(())
    }

    /// Quantos itens de carrinho (de qualquer carrinho, aberto ou fechado) apontam para o produto.
    ///
    /// Excluir o produto zera essa referência (`cart_items.product_id` tem `ON DELETE SET NULL`):
    /// o item fica sem nome e com o preço que já estava gravado. Por isso a exclusão precisa
    /// mostrar a contagem antes.
    pub fn referencias(&self, org: OrgId, id: ProductId) -> Resultado<ReferenciasProduto> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.leitura(move |conn| {
            let (itens, carrinhos): (i64, i64) = conn.query_row(
                "SELECT COUNT(*), COUNT(DISTINCT cart_id) FROM cart_items
                  WHERE org_id = ?1 AND product_id = ?2",
                params![texto_org, texto_id],
                |linha| Ok((linha.get(0)?, linha.get(1)?)),
            )?;
            Ok(ReferenciasProduto {
                itens_de_carrinho: itens as u64,
                carrinhos: carrinhos as u64,
            })
        })
    }

    // ───────────────────────── Visão do agente ─────────────────────────

    /// Categorias que o agente enxerga. `None` = nenhuma vinculada, ou seja, todas as da org.
    ///
    /// É a mesma regra do POP Atende: vínculo manda; sem vínculo, o agente vê o catálogo inteiro.
    pub fn categorias_visiveis(
        &self,
        org: OrgId,
        agente: AgentId,
    ) -> Resultado<Option<Vec<ProductCategoryId>>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT category_id FROM agent_product_categories
                  WHERE org_id = ?1 AND agent_id = ?2 ORDER BY category_id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_agente], |linha| {
                id_da_coluna::<ProductCategoryId>(linha, 0)
            })?;
            let mut categorias = Vec::new();
            for linha in linhas {
                categorias.push(linha?);
            }
            Ok(if categorias.is_empty() {
                None
            } else {
                Some(categorias)
            })
        })
    }

    /// Produtos ativos que o agente enxerga, em ordem de nome (o que entra no índice dele).
    pub fn visiveis_do_agente(&self, org: OrgId, agente: AgentId) -> Resultado<Vec<Produto>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(&format!(
                "SELECT {COLUNAS} FROM products p
                  WHERE p.org_id = ?1 AND p.status = 'active'
                    AND (NOT EXISTS (SELECT 1 FROM agent_product_categories v
                                      WHERE v.org_id = ?1 AND v.agent_id = ?2)
                         OR p.category_id IN (SELECT v.category_id FROM agent_product_categories v
                                               WHERE v.org_id = ?1 AND v.agent_id = ?2))
                  ORDER BY lower(p.name), p.id"
            ))?;
            let linhas = consulta.query_map(params![texto_org, texto_agente], linha_produto)?;
            let mut produtos = Vec::new();
            for linha in linhas {
                produtos.push(linha?);
            }
            Ok(produtos)
        })
    }

    /// Produtos ativos que o agente enxerga, já com o texto que vai ao índice (sem preço).
    pub fn para_indice(&self, org: OrgId, agente: AgentId) -> Resultado<Vec<ProdutoParaIndice>> {
        let texto_org = org.to_string();
        let texto_agente = agente.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
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
            let linhas = consulta.query_map(params![texto_org, texto_agente], |linha| {
                let aliases: String = linha.get(3)?;
                let descricao: String = linha.get::<_, Option<String>>(4)?.unwrap_or_default();
                let tags: String = linha.get(5)?;
                let nome: String = linha.get(1)?;
                Ok(ProdutoParaIndice {
                    id: id_da_coluna(linha, 0)?,
                    nome: nome.clone(),
                    categoria: linha.get(2)?,
                    texto: texto_para_indice(
                        &nome,
                        &serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default(),
                        &descricao,
                        &serde_json::from_str::<Vec<String>>(&tags).unwrap_or_default(),
                    ),
                    versao: u64_da_coluna(linha, 6)?,
                })
            })?;
            let mut produtos = Vec::new();
            for linha in linhas {
                produtos.push(linha?);
            }
            Ok(produtos)
        })
    }

    /// Produtos ativos com esses ids, na ordem pedida. Id de outra org ou inativo simplesmente não
    /// volta — é o que a ferramenta usa para buscar o preço atual do registro.
    pub fn ativos_por_ids(&self, org: OrgId, ids: &[ProductId]) -> Resultado<Vec<Produto>> {
        let mut saida = Vec::with_capacity(ids.len());
        for id in ids {
            let texto_org = org.to_string();
            let texto_id = id.to_string();
            let achado = self.0.leitura(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {COLUNAS} FROM products
                          WHERE org_id = ?1 AND id = ?2 AND status = 'active'"
                    ),
                    params![texto_org, texto_id],
                    linha_produto,
                )
                .optional()
                .map_err(ErroStore::from)
            })?;
            if let Some(produto) = achado {
                saida.push(produto);
            }
        }
        Ok(saida)
    }

    /// Ids dos produtos ativos de uma categoria (o que muda quando a categoria muda).
    pub fn ativos_da_categoria(
        &self,
        org: OrgId,
        categoria: ProductCategoryId,
    ) -> Resultado<Vec<ProductId>> {
        let texto_org = org.to_string();
        let texto_categoria = categoria.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT id FROM products
                  WHERE org_id = ?1 AND category_id = ?2 AND status = 'active'
                  ORDER BY id",
            )?;
            let linhas = consulta.query_map(params![texto_org, texto_categoria], |linha| {
                id_da_coluna::<ProductId>(linha, 0)
            })?;
            let mut ids = Vec::new();
            for linha in linhas {
                ids.push(linha?);
            }
            Ok(ids)
        })
    }

    /// Nome da categoria do produto (rótulo de exibição na resposta ao modelo).
    pub fn nome_da_categoria(&self, org: OrgId, id: ProductCategoryId) -> Resultado<String> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        let nome = self.0.leitura(move |conn| {
            conn.query_row(
                "SELECT name FROM product_categories WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id],
                |linha| linha.get(0),
            )
            .optional()
            .map_err(ErroStore::from)
        })?;
        Ok(nome.unwrap_or_else(|| "sem categoria".to_owned()))
    }

    fn conflito(&self, org: OrgId, id: ProductId, esperada: u64) -> ErroStore {
        self.0
            .leitura(move |conn| {
                Ok(conflito_ou_ausente(
                    conn,
                    "products",
                    "produto",
                    &id.to_string(),
                    &org.to_string(),
                    esperada,
                ))
            })
            .unwrap_or_else(|erro| erro)
    }
}

/// Texto do produto que entra no índice: nome, aliases, descrição e tags.
///
/// **Preço, unidade, inclui/exclui e condições ficam de fora de propósito**: eles são lidos do
/// registro na hora da resposta, e mudar preço não pode exigir reindexar.
pub fn texto_para_indice(
    nome: &str,
    aliases: &[String],
    descricao: &str,
    tags: &[String],
) -> String {
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

fn validar_nome(nome: &str) -> Resultado<String> {
    let limpo = nome.trim();
    if limpo.is_empty() {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
            campo: "nome do produto",
            motivo: "não pode ficar em branco".to_owned(),
        }));
    }
    if limpo.chars().count() > LIMITE_NOME {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
            campo: "nome do produto",
            motivo: format!(
                "até {LIMITE_NOME} caracteres; vieram {}",
                limpo.chars().count()
            ),
        }));
    }
    Ok(limpo.to_owned())
}

fn validar_slug(slug: &str, nome: &str) -> Resultado<String> {
    let derivado = if slug.trim().is_empty() {
        slugificar(nome)
    } else {
        slugificar(slug)
    };
    let tamanho = derivado.chars().count();
    if !(3..=64).contains(&tamanho) {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
            campo: "slug do produto",
            motivo: format!(
                "precisa de 3 a 64 caracteres (letras sem acento, números e hífen); o derivado \
                 ficou com {tamanho}"
            ),
        }));
    }
    Ok(derivado)
}

fn validar_preco(centavos: i64) -> Resultado<()> {
    if centavos < 0 {
        return Err(ErroStore::Dominio(poph_types::DomainError::Validacao {
            campo: "preço do produto",
            motivo: "não pode ser negativo (o valor é em centavos)".to_owned(),
        }));
    }
    Ok(())
}

fn lista_json(itens: &[String]) -> String {
    let limpos: Vec<String> = itens
        .iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect();
    serde_json::to_string(&limpos).unwrap_or_else(|_| "[]".to_owned())
}

fn texto_ou_nulo(texto: &str) -> Option<String> {
    let limpo = texto.trim();
    (!limpo.is_empty()).then(|| limpo.to_owned())
}

fn nao_encontrado(id: &str) -> ErroStore {
    ErroStore::Dominio(poph_types::DomainError::NaoEncontrado {
        entidade: "produto",
        id: id.to_owned(),
    })
}

fn linha_produto(linha: &Row<'_>) -> rusqlite::Result<Produto> {
    let aliases: String = linha.get(5)?;
    let tags: String = linha.get(13)?;
    Ok(Produto {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        categoria_id: id_da_coluna(linha, 2)?,
        nome: linha.get(3)?,
        slug: linha.get(4)?,
        aliases: serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default(),
        descricao: linha.get::<_, Option<String>>(6)?.unwrap_or_default(),
        preco_texto: linha.get(7)?,
        preco_centavos: linha.get(8)?,
        unidade: linha.get::<_, Option<String>>(9)?.unwrap_or_default(),
        inclui: linha.get::<_, Option<String>>(10)?.unwrap_or_default(),
        exclui: linha.get::<_, Option<String>>(11)?.unwrap_or_default(),
        condicoes: linha.get::<_, Option<String>>(12)?.unwrap_or_default(),
        tags: serde_json::from_str::<Vec<String>>(&tags).unwrap_or_default(),
        status: status_da_coluna(linha, 14)?,
        versao: u64_da_coluna(linha, 15)?,
        criado_em: linha.get(16)?,
        atualizado_em: linha.get(17)?,
    })
}
