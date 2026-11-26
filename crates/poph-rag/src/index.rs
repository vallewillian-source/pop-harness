use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

use poph_types::{AgentId, KnowledgeItemId, OrgId, ProductId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::chunks::{embedding_text, source_hash, split};
use crate::database::{self, database_error};
use crate::embedder::{DIMENSION, Embedder, ModelIdentity};
use crate::error::RagError;
use crate::vector::{cosine, dot_product, norm, quantize};

const INDEXING_BATCH: usize = 64;

const BYTES_PER_CHUNK: usize = DIMENSION + 4 + 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexConfig {
    pub memory_budget_bytes: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            memory_budget_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRag {
    pub id: KnowledgeItemId,
    pub title: String,
    pub texto: String,
    pub versao: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenReport {
    pub cleaned_by_divergence: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexReport {
    pub new_items: usize,
    pub changed_items: usize,
    pub unchanged_items: usize,
    pub removed_items: usize,
    pub generated_embeddings: usize,
    pub stored_chunks: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub item: KnowledgeItemId,
    pub ordem: usize,
    pub title: String,
    pub texto: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub chunk: Chunk,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchConfig {
    pub top_k: usize,
    pub cutoff: f32,
    pub depth: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            top_k: 3,
            cutoff: 0.0,
            depth: 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub chunk: Chunk,
    pub similarity: f32,
    pub relevance: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductRag {
    pub id: ProductId,
    pub nome: String,
    pub texto: String,
    pub versao: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductHit {
    pub product: ProductId,
    pub nome: String,
    pub chunk: String,
    pub similarity: f32,
    pub relevance: f32,
}

const RRF_K: f32 = 60.0;

type Chave = (OrgId, AgentId);

struct PreparedItem {
    position: usize,
    hash: String,
    chunks: Vec<String>,
    vectors: Vec<(Vec<i8>, f32)>,
}

type PreparedProduct = (usize, String, Vec<String>, Vec<(Vec<i8>, f32)>);

#[derive(Debug, Default)]
struct AgentIndex {
    ids: Vec<i64>,
    vectors: Vec<i8>,
    norms: Vec<f32>,
}

impl AgentIndex {
    fn bytes(&self) -> usize {
        self.ids.len() * BYTES_PER_CHUNK
    }
}

struct Entry {
    index: Arc<AgentIndex>,
    last_used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<Chave, Entry>,
    bytes: usize,
    clock: u64,
    generations: HashMap<Chave, u64>,
}

impl Cache {
    fn generation(&self, chave: &Chave) -> u64 {
        self.generations.get(chave).copied().unwrap_or(0)
    }

    fn remove(&mut self, chave: &Chave) {
        if let Some(entry) = self.entries.remove(chave) {
            self.bytes -= entry.index.bytes();
        }
    }

    fn invalidate(&mut self, chave: Chave) {
        self.remove(&chave);
        *self.generations.entry(chave).or_insert(0) += 1;
    }
}

pub struct Index {
    conn: Mutex<Connection>,
    embedder: Arc<dyn Embedder>,
    cache: Mutex<Cache>,
    write_lock: tokio::sync::Mutex<()>,
    budget: usize,
}

fn lock<T>(target: &Mutex<T>) -> MutexGuard<'_, T> {
    target.lock().unwrap_or_else(|error| error.into_inner())
}

impl Index {
    pub fn open(
        path: &Path,
        embedder: Arc<dyn Embedder>,
        config: IndexConfig,
    ) -> Result<(Self, OpenReport), RagError> {
        let mut conn = database::open_database(path)?;
        let cleaned = {
            let tx = conn.transaction().map_err(database_error)?;
            let cleaned = database::check_identity(&tx, embedder.identity())?;
            tx.commit().map_err(database_error)?;
            cleaned
        };
        let index = Self {
            conn: Mutex::new(conn),
            embedder,
            cache: Mutex::new(Cache::default()),
            write_lock: tokio::sync::Mutex::new(()),
            budget: config.memory_budget_bytes,
        };
        Ok((
            index,
            OpenReport {
                cleaned_by_divergence: cleaned,
            },
        ))
    }

    pub async fn index_agent(
        &self,
        org: OrgId,
        agente: AgentId,
        items: &[ItemRag],
    ) -> Result<IndexReport, RagError> {
        let _turn = self.write_lock.lock().await;

        let mut seen = HashSet::new();
        for item in items {
            if !seen.insert(item.id) {
                return Err(RagError::Item(format!(
                    "item repetido na lista: {}",
                    item.id
                )));
            }
        }

        let identity = self.embedder.identity().clone();
        let current = {
            let conn = lock(&self.conn);
            read_hashes(&conn, org, agente)?
        };

        let mut report = IndexReport::default();
        let mut to_write: Vec<PreparedItem> = Vec::new();
        let mut to_delete: Vec<String> = Vec::new();
        for (position, item) in items.iter().enumerate() {
            let hash = source_hash(&identity, &item.title, &item.texto);
            let id = item.id.to_string();
            match current.get(&id) {
                Some(previous) if *previous == hash => {
                    report.unchanged_items += 1;
                    continue;
                }
                Some(_) => {
                    report.changed_items += 1;
                    to_delete.push(id);
                }
                None => report.new_items += 1,
            }
            to_write.push(PreparedItem {
                position,
                hash,
                chunks: split(&item.texto),
                vectors: Vec::new(),
            });
        }
        for id in current.keys() {
            if !items.iter().any(|item| item.id.to_string() == *id) {
                report.removed_items += 1;
                to_delete.push(id.clone());
            }
        }
        if to_write.is_empty() && to_delete.is_empty() {
            return Ok(report);
        }

        let mut texts: Vec<String> = Vec::new();
        for prepared in &to_write {
            let item = &items[prepared.position];
            texts.extend(
                prepared
                    .chunks
                    .iter()
                    .map(|chunk| embedding_text(&item.title, chunk)),
            );
        }
        report.generated_embeddings = texts.len();

        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for block in texts.chunks(INDEXING_BATCH) {
            let generated = self.embedder.embed_batch(block).await?;
            if generated.len() != block.len() {
                return Err(RagError::Inference(format!(
                    "o modelo devolveu {} vetores para {} textos",
                    generated.len(),
                    block.len()
                )));
            }
            vectors.extend(generated);
        }

        let mut remaining = vectors.into_iter();
        for prepared in to_write.iter_mut() {
            for _ in 0..prepared.chunks.len() {
                let vector = remaining.next().ok_or_else(|| {
                    RagError::Inference("faltam vetores para os trechos do item".to_owned())
                })?;
                if vector.len() != DIMENSION {
                    return Err(RagError::Inference(format!(
                        "vetor com {} posições; o esperado é {DIMENSION}",
                        vector.len()
                    )));
                }
                let q = quantize(&vector);
                let n = norm(&q);
                prepared.vectors.push((q, n));
            }
        }

        {
            let mut conn = lock(&self.conn);
            let tx = conn.transaction().map_err(database_error)?;
            for id in &to_delete {
                delete_item(&tx, org, agente, id)?;
            }
            for prepared in &to_write {
                let item = &items[prepared.position];
                tx.execute(
                    "INSERT INTO rag_itens (org_id, agente_id, item_id, titulo, versao, hash_origem, trechos)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        org.to_string(),
                        agente.to_string(),
                        item.id.to_string(),
                        item.title,
                        item.versao as i64,
                        prepared.hash,
                        prepared.chunks.len() as i64,
                    ],
                )
                .map_err(database_error)?;
                for (ordem, (texto, (vector, n))) in
                    prepared.chunks.iter().zip(&prepared.vectors).enumerate()
                {
                    let bytes: Vec<u8> = vector.iter().map(|x| *x as u8).collect();
                    tx.execute(
                        "INSERT INTO rag_trechos (org_id, agente_id, item_id, ordem, texto, vetor, norma)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            org.to_string(),
                            agente.to_string(),
                            item.id.to_string(),
                            ordem as i64,
                            texto,
                            bytes,
                            f64::from(*n),
                        ],
                    )
                    .map_err(database_error)?;
                    let rowid = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO rag_trechos_fts (rowid, texto_busca, org_id, agente_id)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            rowid,
                            embedding_text(&item.title, texto),
                            org.to_string(),
                            agente.to_string(),
                        ],
                    )
                    .map_err(database_error)?;
                    report.stored_chunks += 1;
                }
            }
            tx.commit().map_err(database_error)?;
        }

        lock(&self.cache).invalidate((org, agente));
        tracing::info!(
            new = report.new_items,
            changed = report.changed_items,
            unchanged = report.unchanged_items,
            removed = report.removed_items,
            chunks = report.stored_chunks,
            "índice do agente atualizado"
        );
        Ok(report)
    }

    pub async fn search_vector(
        &self,
        org: OrgId,
        agente: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Found>, RagError> {
        if limit == 0 {
            return Err(RagError::Configuration(
                "o limite de resultados deve ser maior que zero".to_owned(),
            ));
        }
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let texts = vec![query.to_owned()];
        let generated = self.embedder.embed_batch(&texts).await?;
        let vector = generated.into_iter().next().ok_or_else(|| {
            RagError::Inference("o modelo não devolveu vetor da consulta".to_owned())
        })?;
        if vector.len() != DIMENSION {
            return Err(RagError::Inference(format!(
                "vetor da consulta com {} posições; o esperado é {DIMENSION}",
                vector.len()
            )));
        }
        let q = quantize(&vector);
        let nq = norm(&q);

        let index = self.load(org, agente)?;
        let best = scan(&index, &q, nq, limit);

        let conn = lock(&self.conn);
        let mut saida = Vec::with_capacity(best.len());
        for (score, rowid) in best {
            if let Some(chunk) = detail(&conn, org, agente, rowid)? {
                saida.push(Found { chunk, score });
            }
        }
        Ok(saida)
    }

    pub fn search_words(
        &self,
        org: OrgId,
        agente: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Found>, RagError> {
        if limit == 0 {
            return Err(RagError::Configuration(
                "o limite de resultados deve ser maior que zero".to_owned(),
            ));
        }
        let matches = self.words_with_rowid(org, agente, query, limit)?;
        let conn = lock(&self.conn);
        let mut saida = Vec::with_capacity(matches.len());
        for (rowid, relevance) in matches {
            if let Some(chunk) = detail(&conn, org, agente, rowid)? {
                saida.push(Found {
                    chunk,
                    score: -(relevance as f32),
                });
            }
        }
        Ok(saida)
    }

    pub fn list_chunks(&self, org: OrgId, agente: AgentId) -> Result<Vec<Chunk>, RagError> {
        let conn = lock(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT t.item_id, t.ordem, i.titulo, t.texto
                 FROM rag_trechos t
                 JOIN rag_itens i
                   ON i.org_id = t.org_id AND i.agente_id = t.agente_id AND i.item_id = t.item_id
                 WHERE t.org_id = ?1 AND t.agente_id = ?2
                 ORDER BY t.item_id, t.ordem",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map(params![org.to_string(), agente.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(database_error)?;
        let mut saida = Vec::new();
        for row in rows {
            let (item, ordem, title, texto) = row.map_err(database_error)?;
            saida.push(Chunk {
                item: item_id(&item)?,
                ordem: ordem as usize,
                title,
                texto,
            });
        }
        Ok(saida)
    }

    pub async fn search_hybrid(
        &self,
        org: OrgId,
        agente: AgentId,
        query: &str,
        config: &SearchConfig,
    ) -> Result<Vec<Hit>, RagError> {
        if config.top_k == 0 {
            return Err(RagError::Configuration(
                "o top-k da busca deve ser maior que zero".to_owned(),
            ));
        }
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let depth = config.depth.max(config.top_k);
        let texts = vec![query.to_owned()];
        let generated = self.embedder.embed_batch(&texts).await?;
        let vector = generated.into_iter().next().ok_or_else(|| {
            RagError::Inference("o modelo não devolveu vetor da consulta".to_owned())
        })?;
        if vector.len() != DIMENSION {
            return Err(RagError::Inference(format!(
                "vetor da consulta com {} posições; o esperado é {DIMENSION}",
                vector.len()
            )));
        }
        let q = quantize(&vector);
        let nq = norm(&q);

        let index = self.load(org, agente)?;
        let words = self.words_with_rowid(org, agente, query, depth)?;
        let targets: HashSet<i64> = words.iter().map(|(rowid, _)| *rowid).collect();
        let (top_vectors, target_cosines) = scan_with_targets(&index, &q, nq, depth, &targets);

        let mut combined: HashMap<i64, (f32, f32)> = HashMap::new();
        for (position, (similarity, rowid)) in top_vectors.iter().enumerate() {
            if *similarity < config.cutoff {
                continue;
            }
            let rrf_score = 1.0 / (RRF_K + (position + 1) as f32);
            combined.insert(*rowid, (*similarity, rrf_score));
        }
        for (position, (rowid, _bm25)) in words.iter().enumerate() {
            let similarity = match target_cosines.get(rowid) {
                Some(value) => *value,
                None => continue,
            };
            if similarity < config.cutoff {
                continue;
            }
            let rrf_score = 1.0 / (RRF_K + (position + 1) as f32);
            let entry = combined.entry(*rowid).or_insert((similarity, 0.0));
            entry.0 = entry.0.max(similarity);
            entry.1 += rrf_score;
        }

        let mut ordered: Vec<(i64, f32, f32)> = combined
            .into_iter()
            .map(|(rowid, (similarity, relevance))| (rowid, similarity, relevance))
            .collect();
        ordered.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.0.cmp(&b.0))
        });
        ordered.truncate(config.top_k);

        let conn = lock(&self.conn);
        let mut saida = Vec::with_capacity(ordered.len());
        for (rowid, similarity, relevance) in ordered {
            if let Some(chunk) = detail(&conn, org, agente, rowid)? {
                saida.push(Hit {
                    chunk,
                    similarity,
                    relevance,
                });
            }
        }
        Ok(saida)
    }

    pub fn indexed_items(
        &self,
        org: OrgId,
        agente: AgentId,
    ) -> Result<HashMap<KnowledgeItemId, String>, RagError> {
        let conn = lock(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT item_id, hash_origem FROM rag_itens WHERE org_id = ?1 AND agente_id = ?2",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map(params![org.to_string(), agente.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error)?;
        let mut map = HashMap::new();
        for row in rows {
            let (id, hash) = row.map_err(database_error)?;
            map.insert(item_id(&id)?, hash);
        }
        Ok(map)
    }

    fn words_with_rowid(
        &self,
        org: OrgId,
        agente: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(i64, f64)>, RagError> {
        let Some(expression) = fts_expression(query) else {
            return Ok(Vec::new());
        };
        let conn = lock(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT rowid, bm25(rag_trechos_fts) FROM rag_trechos_fts
                 WHERE rag_trechos_fts MATCH ?1 AND org_id = ?2 AND agente_id = ?3
                 ORDER BY bm25(rag_trechos_fts), rowid
                 LIMIT ?4",
            )
            .map_err(database_error)?;
        let matches: Vec<(i64, f64)> = stmt
            .query_map(
                params![
                    expression,
                    org.to_string(),
                    agente.to_string(),
                    limit as i64
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(database_error)?
            .collect::<Result<_, _>>()
            .map_err(database_error)?;
        Ok(matches)
    }

    pub async fn index_products(
        &self,
        org: OrgId,
        agente: AgentId,
        products: &[ProductRag],
    ) -> Result<IndexReport, RagError> {
        let _turn = self.write_lock.lock().await;

        let mut seen = HashSet::new();
        for product in products {
            if !seen.insert(product.id) {
                return Err(RagError::Item(format!(
                    "produto repetido na lista: {}",
                    product.id
                )));
            }
        }

        let identity = self.embedder.identity().clone();
        let current = {
            let conn = lock(&self.conn);
            read_product_hashes(&conn, org, agente)?
        };

        let mut report = IndexReport::default();
        let mut to_write: Vec<PreparedProduct> = Vec::new();
        let mut to_delete: Vec<String> = Vec::new();
        for (position, product) in products.iter().enumerate() {
            let hash = source_hash(&identity, &product.nome, &product.texto);
            let id = product.id.to_string();
            match current.get(&id) {
                Some(previous) if *previous == hash => {
                    report.unchanged_items += 1;
                    continue;
                }
                Some(_) => {
                    report.changed_items += 1;
                    to_delete.push(id);
                }
                None => report.new_items += 1,
            }
            to_write.push((position, hash, split(&product.texto), Vec::new()));
        }
        for id in current.keys() {
            if !products.iter().any(|product| product.id.to_string() == *id) {
                report.removed_items += 1;
                to_delete.push(id.clone());
            }
        }
        if to_write.is_empty() && to_delete.is_empty() {
            return Ok(report);
        }

        let mut texts: Vec<String> = Vec::new();
        for (position, _, chunks, _) in &to_write {
            let product = &products[*position];
            texts.extend(
                chunks
                    .iter()
                    .map(|chunk| embedding_text(&product.nome, chunk)),
            );
        }
        report.generated_embeddings = texts.len();
        let mut vectors = self.embed_texts(&texts).await?.into_iter();
        for (_, _, chunks, target) in to_write.iter_mut() {
            target.clear();
            for _ in 0..chunks.len() {
                let vector = vectors.next().ok_or_else(|| {
                    RagError::Inference("faltam vetores para os trechos do produto".to_owned())
                })?;
                target.push(vector);
            }
        }

        {
            let mut conn = lock(&self.conn);
            let tx = conn.transaction().map_err(database_error)?;
            for id in &to_delete {
                delete_product(&tx, org, agente, id)?;
            }
            for (position, hash, chunks, vectors) in &to_write {
                let product = &products[*position];
                tx.execute(
                    "INSERT INTO rag_produtos
                        (org_id, agente_id, produto_id, nome, versao, hash_origem, trechos)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        org.to_string(),
                        agente.to_string(),
                        product.id.to_string(),
                        product.nome,
                        product.versao as i64,
                        hash,
                        chunks.len() as i64,
                    ],
                )
                .map_err(database_error)?;
                for (ordem, (texto, (vector, n))) in chunks.iter().zip(vectors).enumerate() {
                    let bytes: Vec<u8> = vector.iter().map(|x| *x as u8).collect();
                    tx.execute(
                        "INSERT INTO rag_produtos_trechos
                            (org_id, agente_id, produto_id, ordem, texto, vetor, norma)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            org.to_string(),
                            agente.to_string(),
                            product.id.to_string(),
                            ordem as i64,
                            texto,
                            bytes,
                            f64::from(*n),
                        ],
                    )
                    .map_err(database_error)?;
                    let rowid = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO rag_produtos_fts (rowid, texto_busca, org_id, agente_id)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            rowid,
                            embedding_text(&product.nome, texto),
                            org.to_string(),
                            agente.to_string(),
                        ],
                    )
                    .map_err(database_error)?;
                    report.stored_chunks += 1;
                }
            }
            tx.commit().map_err(database_error)?;
        }

        tracing::info!(
            new = report.new_items,
            changed = report.changed_items,
            unchanged = report.unchanged_items,
            removed = report.removed_items,
            chunks = report.stored_chunks,
            "índice de produtos do agente atualizado"
        );
        Ok(report)
    }

    pub async fn search_products(
        &self,
        org: OrgId,
        agente: AgentId,
        query: &str,
        config: &SearchConfig,
    ) -> Result<Vec<ProductHit>, RagError> {
        if config.top_k == 0 {
            return Err(RagError::Configuration(
                "o top-k da busca deve ser maior que zero".to_owned(),
            ));
        }
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let depth = config.depth.max(config.top_k);
        let (q, nq) = self.query_vector(query).await?;

        let index = {
            let conn = lock(&self.conn);
            read_product_index(&conn, org, agente)?
        };
        let words = {
            let conn = lock(&self.conn);
            product_words_with_rowid(&conn, org, agente, query, depth)?
        };
        let targets: HashSet<i64> = words.iter().map(|(rowid, _)| *rowid).collect();
        let (top_vectors, target_cosines) = scan_with_targets(&index, &q, nq, depth, &targets);

        let mut combined: HashMap<i64, (f32, f32)> = HashMap::new();
        for (position, (similarity, rowid)) in top_vectors.iter().enumerate() {
            if *similarity < config.cutoff {
                continue;
            }
            let rrf_score = 1.0 / (RRF_K + (position + 1) as f32);
            combined.insert(*rowid, (*similarity, rrf_score));
        }
        for (position, (rowid, _)) in words.iter().enumerate() {
            let similarity = match target_cosines.get(rowid) {
                Some(value) => *value,
                None => continue,
            };
            if similarity < config.cutoff {
                continue;
            }
            let rrf_score = 1.0 / (RRF_K + (position + 1) as f32);
            let entry = combined.entry(*rowid).or_insert((similarity, 0.0));
            entry.0 = entry.0.max(similarity);
            entry.1 += rrf_score;
        }

        let mut ordered: Vec<(i64, f32, f32)> = combined
            .into_iter()
            .map(|(rowid, (similarity, relevance))| (rowid, similarity, relevance))
            .collect();
        ordered.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.0.cmp(&b.0))
        });

        let conn = lock(&self.conn);
        let mut saida: Vec<ProductHit> = Vec::new();
        for (rowid, similarity, relevance) in ordered {
            let Some((product, nome, chunk)) = product_detail(&conn, org, agente, rowid)? else {
                continue;
            };
            if saida.iter().any(|hit| hit.product == product) {
                continue;
            }
            saida.push(ProductHit {
                product,
                nome,
                chunk,
                similarity,
                relevance,
            });
            if saida.len() == config.top_k {
                break;
            }
        }
        Ok(saida)
    }

    pub async fn search_products_vector(
        &self,
        org: OrgId,
        agente: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProductHit>, RagError> {
        if limit == 0 {
            return Err(RagError::Configuration(
                "o limite de resultados deve ser maior que zero".to_owned(),
            ));
        }
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let (q, nq) = self.query_vector(query).await?;
        let index = {
            let conn = lock(&self.conn);
            read_product_index(&conn, org, agente)?
        };
        let best = scan(&index, &q, nq, limit * 4);
        let conn = lock(&self.conn);
        let mut saida: Vec<ProductHit> = Vec::new();
        for (similarity, rowid) in best {
            let Some((product, nome, chunk)) = product_detail(&conn, org, agente, rowid)? else {
                continue;
            };
            if saida.iter().any(|hit| hit.product == product) {
                continue;
            }
            saida.push(ProductHit {
                product,
                nome,
                chunk,
                similarity,
                relevance: similarity,
            });
            if saida.len() == limit {
                break;
            }
        }
        Ok(saida)
    }

    pub fn indexed_products(
        &self,
        org: OrgId,
        agente: AgentId,
    ) -> Result<HashMap<ProductId, String>, RagError> {
        let conn = lock(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT produto_id, hash_origem FROM rag_produtos
                  WHERE org_id = ?1 AND agente_id = ?2",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map(params![org.to_string(), agente.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error)?;
        let mut map = HashMap::new();
        for row in rows {
            let (id, hash) = row.map_err(database_error)?;
            map.insert(
                ProductId::from_str(&id).map_err(|_| {
                    RagError::Database("id de produto inválido no rag.db".to_owned())
                })?,
                hash,
            );
        }
        Ok(map)
    }

    async fn query_vector(&self, query: &str) -> Result<(Vec<i8>, f32), RagError> {
        let texts = vec![query.to_owned()];
        let generated = self.embedder.embed_batch(&texts).await?;
        let vector = generated.into_iter().next().ok_or_else(|| {
            RagError::Inference("o modelo não devolveu vetor da consulta".to_owned())
        })?;
        if vector.len() != DIMENSION {
            return Err(RagError::Inference(format!(
                "vetor da consulta com {} posições; o esperado é {DIMENSION}",
                vector.len()
            )));
        }
        let q = quantize(&vector);
        let nq = norm(&q);
        Ok((q, nq))
    }

    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<(Vec<i8>, f32)>, RagError> {
        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for block in texts.chunks(INDEXING_BATCH) {
            let generated = self.embedder.embed_batch(block).await?;
            if generated.len() != block.len() {
                return Err(RagError::Inference(format!(
                    "o modelo devolveu {} vetores para {} textos",
                    generated.len(),
                    block.len()
                )));
            }
            vectors.extend(generated);
        }
        let mut saida = Vec::with_capacity(vectors.len());
        for vector in vectors {
            if vector.len() != DIMENSION {
                return Err(RagError::Inference(format!(
                    "vetor com {} posições; o esperado é {DIMENSION}",
                    vector.len()
                )));
            }
            let q = quantize(&vector);
            let n = norm(&q);
            saida.push((q, n));
        }
        Ok(saida)
    }

    pub fn identity(&self) -> &ModelIdentity {
        self.embedder.identity()
    }

    pub fn agents_in_memory(&self) -> usize {
        lock(&self.cache).entries.len()
    }

    pub fn bytes_in_memory(&self) -> usize {
        lock(&self.cache).bytes
    }

    pub fn memory_budget_bytes(&self) -> usize {
        self.budget
    }

    fn load(&self, org: OrgId, agente: AgentId) -> Result<Arc<AgentIndex>, RagError> {
        let chave = (org, agente);
        let generation = {
            let mut cache = lock(&self.cache);
            cache.clock += 1;
            let now = cache.clock;
            if let Some(entry) = cache.entries.get_mut(&chave) {
                entry.last_used = now;
                return Ok(Arc::clone(&entry.index));
            }
            cache.generation(&chave)
        };

        let loaded = Arc::new(self.read_index(org, agente)?);
        let bytes = loaded.bytes();

        let mut cache = lock(&self.cache);
        if cache.generation(&chave) != generation {
            return Ok(loaded);
        }
        if let Some(entry) = cache.entries.get(&chave) {
            return Ok(Arc::clone(&entry.index));
        }
        if bytes > self.budget {
            return Ok(loaded);
        }
        while cache.bytes + bytes > self.budget {
            let idle = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(chave, _)| *chave);
            match idle {
                Some(idle_key) => cache.remove(&idle_key),
                None => break,
            }
        }
        cache.clock += 1;
        let now = cache.clock;
        cache.bytes += bytes;
        cache.entries.insert(
            chave,
            Entry {
                index: Arc::clone(&loaded),
                last_used: now,
            },
        );
        Ok(loaded)
    }

    fn read_index(&self, org: OrgId, agente: AgentId) -> Result<AgentIndex, RagError> {
        let conn = lock(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, vetor, norma FROM rag_trechos
                 WHERE org_id = ?1 AND agente_id = ?2
                 ORDER BY id",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map(params![org.to_string(), agente.to_string()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .map_err(database_error)?;
        let mut index = AgentIndex::default();
        for row in rows {
            let (id, bytes, n) = row.map_err(database_error)?;
            if bytes.len() != DIMENSION {
                return Err(RagError::Database(format!(
                    "vetor gravado com {} posições; o esperado é {DIMENSION}. Reindexe o agente",
                    bytes.len()
                )));
            }
            index.ids.push(id);
            index.vectors.extend(bytes.iter().map(|b| *b as i8));
            index.norms.push(n as f32);
        }
        Ok(index)
    }
}

fn scan(index: &AgentIndex, q: &[i8], nq: f32, limit: usize) -> Vec<(f32, i64)> {
    let mut best: Vec<(f32, i64)> = Vec::with_capacity(limit + 1);
    for (position, vector) in index.vectors.as_chunks::<DIMENSION>().0.iter().enumerate() {
        let score = cosine(dot_product(q, vector), nq, index.norms[position]);
        if best.len() == limit && score <= best[limit - 1].0 {
            continue;
        }
        let slot = best
            .iter()
            .position(|(current, _)| score > *current)
            .unwrap_or(best.len());
        best.insert(slot, (score, index.ids[position]));
        if best.len() > limit {
            best.pop();
        }
    }
    best
}

fn scan_with_targets(
    index: &AgentIndex,
    q: &[i8],
    nq: f32,
    limit: usize,
    targets: &HashSet<i64>,
) -> (Vec<(f32, i64)>, HashMap<i64, f32>) {
    let mut best: Vec<(f32, i64)> = Vec::with_capacity(limit + 1);
    let mut target_cosines: HashMap<i64, f32> = HashMap::with_capacity(targets.len());
    for (position, vector) in index.vectors.as_chunks::<DIMENSION>().0.iter().enumerate() {
        let score = cosine(dot_product(q, vector), nq, index.norms[position]);
        let rowid = index.ids[position];
        if targets.contains(&rowid) {
            target_cosines.insert(rowid, score);
        }
        if best.len() == limit && score <= best[limit - 1].0 {
            continue;
        }
        let slot = best
            .iter()
            .position(|(current, _)| score > *current)
            .unwrap_or(best.len());
        best.insert(slot, (score, rowid));
        if best.len() > limit {
            best.pop();
        }
    }
    (best, target_cosines)
}

const STOPWORDS: &[&str] = &[
    "aos", "aquela", "aquele", "as", "ate", "com", "como", "da", "das", "de", "dela", "dele",
    "depois", "do", "dos", "e", "ela", "elas", "ele", "eles", "em", "entre", "era", "essa", "esse",
    "esta", "este", "eu", "foi", "for", "ha", "isso", "isto", "ja", "mais", "mas", "me", "mesmo",
    "meu", "minha", "muito", "na", "nas", "nem", "no", "nos", "nossa", "nosso", "num", "o", "os",
    "ou", "para", "pela", "pelo", "por", "pra", "qual", "que", "quem", "se", "sem", "ser", "seu",
    "sua", "suas", "seus", "so", "sou", "sua", "ta", "tem", "tenho", "ter", "teu", "tu", "tua",
    "um", "uma", "voce", "voces", "vos",
];

fn fts_expression(query: &str) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        let without_accent: String = word
            .chars()
            .map(|c| match c {
                'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'í' | 'ì' | 'î' | 'ï' => 'i',
                'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
                'ú' | 'ù' | 'û' | 'ü' => 'u',
                'ç' => 'c',
                other => other.to_ascii_lowercase(),
            })
            .collect();
        if without_accent.chars().count() < 3 || STOPWORDS.contains(&without_accent.as_str()) {
            continue;
        }
        let escaped = word.replace('"', "\"\"");
        let term = if word.chars().count() >= 4 {
            format!("\"{escaped}\"*")
        } else {
            format!("\"{escaped}\"")
        };
        if !words.contains(&term) {
            words.push(term);
        }
        if words.len() == 16 {
            break;
        }
    }
    if words.is_empty() {
        None
    } else {
        Some(words.join(" OR "))
    }
}

fn item_id(texto: &str) -> Result<KnowledgeItemId, RagError> {
    KnowledgeItemId::from_str(texto)
        .map_err(|_| RagError::Database("id de item inválido no rag.db".to_owned()))
}

fn read_hashes(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
) -> Result<HashMap<String, String>, RagError> {
    let mut stmt = conn
        .prepare("SELECT item_id, hash_origem FROM rag_itens WHERE org_id = ?1 AND agente_id = ?2")
        .map_err(database_error)?;
    let rows = stmt
        .query_map(params![org.to_string(), agente.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, hash) = row.map_err(database_error)?;
        map.insert(id, hash);
    }
    Ok(map)
}

fn delete_item(
    tx: &rusqlite::Transaction<'_>,
    org: OrgId,
    agente: AgentId,
    id: &str,
) -> Result<(), RagError> {
    let (org, agente) = (org.to_string(), agente.to_string());
    let rowids: Vec<i64> = {
        let mut stmt = tx
            .prepare(
                "SELECT id FROM rag_trechos WHERE org_id = ?1 AND agente_id = ?2 AND item_id = ?3",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map(params![org, agente, id], |row| row.get(0))
            .map_err(database_error)?;
        rows.collect::<Result<_, _>>().map_err(database_error)?
    };
    for rowid in rowids {
        tx.execute(
            "DELETE FROM rag_trechos_fts WHERE rowid = ?1",
            params![rowid],
        )
        .map_err(database_error)?;
    }
    tx.execute(
        "DELETE FROM rag_trechos WHERE org_id = ?1 AND agente_id = ?2 AND item_id = ?3",
        params![org, agente, id],
    )
    .map_err(database_error)?;
    tx.execute(
        "DELETE FROM rag_itens WHERE org_id = ?1 AND agente_id = ?2 AND item_id = ?3",
        params![org, agente, id],
    )
    .map_err(database_error)?;
    Ok(())
}

fn detail(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
    rowid: i64,
) -> Result<Option<Chunk>, RagError> {
    let row = conn
        .query_row(
            "SELECT t.item_id, t.ordem, i.titulo, t.texto
             FROM rag_trechos t
             JOIN rag_itens i
               ON i.org_id = t.org_id AND i.agente_id = t.agente_id AND i.item_id = t.item_id
             WHERE t.id = ?1 AND t.org_id = ?2 AND t.agente_id = ?3",
            params![rowid, org.to_string(), agente.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    match row {
        Some((item, ordem, title, texto)) => Ok(Some(Chunk {
            item: item_id(&item)?,
            ordem: ordem as usize,
            title,
            texto,
        })),
        None => Ok(None),
    }
}

fn read_product_hashes(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
) -> Result<HashMap<String, String>, RagError> {
    let mut stmt = conn
        .prepare(
            "SELECT produto_id, hash_origem FROM rag_produtos
              WHERE org_id = ?1 AND agente_id = ?2",
        )
        .map_err(database_error)?;
    let rows = stmt
        .query_map(params![org.to_string(), agente.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error)?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, hash) = row.map_err(database_error)?;
        map.insert(id, hash);
    }
    Ok(map)
}

fn delete_product(
    tx: &rusqlite::Transaction<'_>,
    org: OrgId,
    agente: AgentId,
    id: &str,
) -> Result<(), RagError> {
    let (org, agente) = (org.to_string(), agente.to_string());
    let rowids: Vec<i64> = {
        let mut stmt = tx
            .prepare(
                "SELECT id FROM rag_produtos_trechos
                  WHERE org_id = ?1 AND agente_id = ?2 AND produto_id = ?3",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map(params![org, agente, id], |row| row.get(0))
            .map_err(database_error)?;
        rows.collect::<Result<_, _>>().map_err(database_error)?
    };
    for rowid in rowids {
        tx.execute(
            "DELETE FROM rag_produtos_fts WHERE rowid = ?1",
            params![rowid],
        )
        .map_err(database_error)?;
    }
    tx.execute(
        "DELETE FROM rag_produtos_trechos WHERE org_id = ?1 AND agente_id = ?2 AND produto_id = ?3",
        params![org, agente, id],
    )
    .map_err(database_error)?;
    tx.execute(
        "DELETE FROM rag_produtos WHERE org_id = ?1 AND agente_id = ?2 AND produto_id = ?3",
        params![org, agente, id],
    )
    .map_err(database_error)?;
    Ok(())
}

fn read_product_index(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
) -> Result<AgentIndex, RagError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, vetor, norma FROM rag_produtos_trechos
              WHERE org_id = ?1 AND agente_id = ?2
              ORDER BY id",
        )
        .map_err(database_error)?;
    let rows = stmt
        .query_map(params![org.to_string(), agente.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(database_error)?;
    let mut index = AgentIndex::default();
    for row in rows {
        let (id, bytes, n) = row.map_err(database_error)?;
        if bytes.len() != DIMENSION {
            return Err(RagError::Database(format!(
                "vetor de produto gravado com {} posições; o esperado é {DIMENSION}. Reindexe o agente",
                bytes.len()
            )));
        }
        index.ids.push(id);
        index.vectors.extend(bytes.iter().map(|b| *b as i8));
        index.norms.push(n as f32);
    }
    Ok(index)
}

fn product_words_with_rowid(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
    query: &str,
    limit: usize,
) -> Result<Vec<(i64, f64)>, RagError> {
    let Some(expression) = fts_expression(query) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn
        .prepare(
            "SELECT rowid, bm25(rag_produtos_fts) FROM rag_produtos_fts
              WHERE rag_produtos_fts MATCH ?1 AND org_id = ?2 AND agente_id = ?3
              ORDER BY bm25(rag_produtos_fts), rowid
              LIMIT ?4",
        )
        .map_err(database_error)?;
    let matches: Vec<(i64, f64)> = stmt
        .query_map(
            params![
                expression,
                org.to_string(),
                agente.to_string(),
                limit as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(database_error)?
        .collect::<Result<_, _>>()
        .map_err(database_error)?;
    Ok(matches)
}

fn product_detail(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
    rowid: i64,
) -> Result<Option<(ProductId, String, String)>, RagError> {
    let row = conn
        .query_row(
            "SELECT t.produto_id, p.nome, t.texto
             FROM rag_produtos_trechos t
             JOIN rag_produtos p
               ON p.org_id = t.org_id AND p.agente_id = t.agente_id AND p.produto_id = t.produto_id
             WHERE t.id = ?1 AND t.org_id = ?2 AND t.agente_id = ?3",
            params![rowid, org.to_string(), agente.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    match row {
        Some((product, nome, texto)) => {
            let id = ProductId::from_str(&product)
                .map_err(|_| RagError::Database("id de produto inválido no rag.db".to_owned()))?;
            Ok(Some((id, nome, texto)))
        }
        None => Ok(None),
    }
}
