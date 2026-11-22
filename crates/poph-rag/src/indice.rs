//! Índice de trechos por agente (HR-16, recorte A): `rag.db`, indexação incremental, busca exata
//! em int8 e cache em memória com LRU.
//!
//! - [`Indice::indexar_agente`] recebe a lista completa dos itens de um agente. Compara o hash de
//!   origem de cada item: só reembeda item novo ou alterado, e apaga o que saiu da lista.
//! - [`Indice::buscar_vetor`] faz varredura exata sobre os vetores int8 do agente.
//! - [`Indice::buscar_palavras`] consulta o FTS5 (palavras, com acentos ignorados). Fica pronto
//!   para a busca híbrida da HR-17.
//! - Cada agente carregado ocupa memória. Quando o orçamento estoura, sai o agente ocioso há mais
//!   tempo, e ele volta do `rag.db` na próxima consulta.
//! - Toda operação recebe a org. Chaves do cache, linhas do banco e do FTS são filtradas por ela.
//!
//! Recorte: o comando `poph rag reindex` e a ligação com a base de conhecimento ficam na HR-17.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

use poph_types::{AgentId, KnowledgeItemId, OrgId, ProductId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::banco::{self, erro_banco};
use crate::embedder::{DIMENSAO, Embedder, IdentidadeModelo};
use crate::erro::ErroRag;
use crate::trechos::{dividir, hash_origem, texto_embutido};
use crate::vetor::{cosseno, norma, produto_interno, quantizar};

/// Quantos trechos vão de uma vez ao modelo na indexação. Só limita a memória transitória.
const LOTE_INDEXACAO: usize = 64;

/// Bytes por trecho no cache: vetor int8, norma (f32) e `rowid` (i64).
const BYTES_POR_TRECHO: usize = DIMENSAO + 4 + 8;

/// Configuração do índice em memória.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIndice {
    /// Teto de memória do cache de agentes, em bytes. Padrão 512 MB, o valor fixado na decisão
    /// D-07 (meta M5 do spike HR-01).
    pub orcamento_memoria_bytes: usize,
}

impl Default for ConfigIndice {
    fn default() -> Self {
        Self {
            orcamento_memoria_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Um item da base de conhecimento, como o índice recebe. O conteúdo vem de fora (HR-17 lê o
/// `harness.db`); o índice não sabe onde ele mora.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRag {
    pub id: KnowledgeItemId,
    pub titulo: String,
    pub texto: String,
    /// Versão do item no `harness.db`. Guardada para referência; a decisão de reindexar usa o
    /// hash do conteúdo.
    pub versao: u64,
}

/// Resultado de abrir o `rag.db`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatorioAbertura {
    /// `true` quando o `rag.db` tinha vetores de outro modelo ou divisor e foi limpo. Reindexar
    /// os agentes para voltar a ter resultados.
    pub limpo_por_divergencia: bool,
}

/// O que a indexação fez. Todos os números são de itens ou trechos, nunca de texto.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatorioIndexacao {
    pub itens_novos: usize,
    pub itens_alterados: usize,
    pub itens_inalterados: usize,
    pub itens_removidos: usize,
    /// Quantos textos foram ao modelo (um por trecho de item novo ou alterado).
    pub embeddings_gerados: usize,
    pub trechos_gravados: usize,
}

/// Um trecho gravado no índice, com a origem.
#[derive(Debug, Clone, PartialEq)]
pub struct Trecho {
    pub item: KnowledgeItemId,
    pub ordem: usize,
    pub titulo: String,
    pub texto: String,
}

/// Trecho devolvido por uma busca. `pontuacao` é o cosseno (busca de vetor) ou a relevância do
/// FTS5 com sinal invertido (busca de palavras). Nos dois casos, maior é melhor.
#[derive(Debug, Clone, PartialEq)]
pub struct Encontrado {
    pub trecho: Trecho,
    pub pontuacao: f32,
}

/// Configuração da busca híbrida (HR-17): top-k e corte de relevância do agente, mais a
/// profundidade de candidatos que cada lado (vetor e FTS) entrega à combinação por rank.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigBusca {
    /// Quantos trechos devolver.
    pub top_k: usize,
    /// Cosseno mínimo para o trecho ser considerado relevante. `0.0` mantém tudo (paridade com o
    /// POP Atende, que não tinha corte).
    pub corte: f32,
    /// Quantos candidatos de cada lado entram na combinação. Padrão 20 (6× o top-k padrão).
    pub profundidade: usize,
}

impl Default for ConfigBusca {
    fn default() -> Self {
        Self {
            top_k: 3,
            corte: 0.0,
            profundidade: 20,
        }
    }
}

/// Trecho achado pela busca híbrida, com as duas notas que explicam o resultado.
#[derive(Debug, Clone, PartialEq)]
pub struct Achado {
    /// O trecho e a origem.
    pub trecho: Trecho,
    /// Cosseno entre a consulta e o trecho (0 a 1 na prática, pode ser negativo).
    pub similaridade: f32,
    /// Nota da combinação por rank (RRF). Maior é melhor; não é probabilidade.
    pub relevancia: f32,
}

/// Produto como o índice recebe (HR-18). `texto` já vem montado com nome, aliases, descrição e
/// tags — **sem preço**: o preço é lido do registro na hora da resposta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProdutoRag {
    /// Id do produto.
    pub id: ProductId,
    /// Nome (a "fonte" mostrada ao modelo).
    pub nome: String,
    /// Texto a indexar.
    pub texto: String,
    /// Versão no `harness.db`. A decisão de reindexar usa o hash do conteúdo, não a versão.
    pub versao: u64,
}

/// Produto achado na busca híbrida de produtos, com o trecho que casou.
#[derive(Debug, Clone, PartialEq)]
pub struct AchadoProduto {
    /// Id do produto.
    pub produto: ProductId,
    /// Nome do produto.
    pub nome: String,
    /// Trecho do texto do produto que mais se aproximou da consulta.
    pub trecho: String,
    /// Cosseno entre a consulta e o trecho.
    pub similaridade: f32,
    /// Nota da combinação por rank (RRF).
    pub relevancia: f32,
}

/// Constante do RRF (k = 60 é o valor clássico; suaviza a diferença entre as primeiras posições).
const RRF_K: f32 = 60.0;

type Chave = (OrgId, AgentId);

/// Item que entra na gravação: posição na lista, hash novo, trechos e seus vetores int8.
struct Preparado {
    posicao: usize,
    hash: String,
    trechos: Vec<String>,
    vetores: Vec<(Vec<i8>, f32)>,
}

/// Produto prestes a ser gravado: posição na lista, hash de origem, trechos e seus vetores.
type ProdutoPreparado = (usize, String, Vec<String>, Vec<(Vec<i8>, f32)>);

/// Vetores de um agente, em colunas: `ids[i]`, `vetores[i*384..(i+1)*384]`, `normas[i]`.
#[derive(Debug, Default)]
struct IndiceAgente {
    ids: Vec<i64>,
    vetores: Vec<i8>,
    normas: Vec<f32>,
}

impl IndiceAgente {
    fn bytes(&self) -> usize {
        self.ids.len() * BYTES_POR_TRECHO
    }
}

struct Entrada {
    indice: Arc<IndiceAgente>,
    ultimo_uso: u64,
}

#[derive(Default)]
struct Cache {
    entradas: HashMap<Chave, Entrada>,
    bytes: usize,
    relogio: u64,
    /// Sobe a cada escrita do agente. Uma leitura que começou antes de uma escrita não guarda o
    /// resultado, para não deixar índice antigo no cache.
    geracoes: HashMap<Chave, u64>,
}

impl Cache {
    fn geracao(&self, chave: &Chave) -> u64 {
        self.geracoes.get(chave).copied().unwrap_or(0)
    }

    fn remover(&mut self, chave: &Chave) {
        if let Some(entrada) = self.entradas.remove(chave) {
            self.bytes -= entrada.indice.bytes();
        }
    }

    fn invalidar(&mut self, chave: Chave) {
        self.remover(&chave);
        *self.geracoes.entry(chave).or_insert(0) += 1;
    }
}

/// Índice de trechos. Guarde em `Arc` e compartilhe: as operações são seguras entre tarefas.
pub struct Indice {
    conn: Mutex<Connection>,
    embedder: Arc<dyn Embedder>,
    cache: Mutex<Cache>,
    /// Uma indexação por vez: a lista de itens e a gravação precisam ver o mesmo estado.
    escrita: tokio::sync::Mutex<()>,
    orcamento: usize,
}

fn trava<T>(alvo: &Mutex<T>) -> MutexGuard<'_, T> {
    // Veneno de um pânico anterior não impede a leitura: o estado guardado continua válido.
    alvo.lock().unwrap_or_else(|erro| erro.into_inner())
}

impl Indice {
    /// Abre (ou cria) o `rag.db` em `caminho`. `embedder` define a identidade do índice.
    pub fn abrir(
        caminho: &Path,
        embedder: Arc<dyn Embedder>,
        config: ConfigIndice,
    ) -> Result<(Self, RelatorioAbertura), ErroRag> {
        let mut conn = banco::abrir(caminho)?;
        let limpo = {
            let tx = conn.transaction().map_err(erro_banco)?;
            let limpo = banco::conferir_identidade(&tx, embedder.identidade())?;
            tx.commit().map_err(erro_banco)?;
            limpo
        };
        let indice = Self {
            conn: Mutex::new(conn),
            embedder,
            cache: Mutex::new(Cache::default()),
            escrita: tokio::sync::Mutex::new(()),
            orcamento: config.orcamento_memoria_bytes,
        };
        Ok((
            indice,
            RelatorioAbertura {
                limpo_por_divergencia: limpo,
            },
        ))
    }

    /// Deixa o índice do agente igual à lista `itens`: indexa o que é novo ou mudou, apaga o que
    /// saiu da lista e não mexe no que não mudou.
    pub async fn indexar_agente(
        &self,
        org: OrgId,
        agente: AgentId,
        itens: &[ItemRag],
    ) -> Result<RelatorioIndexacao, ErroRag> {
        let _vez = self.escrita.lock().await;

        let mut vistos = HashSet::new();
        for item in itens {
            if !vistos.insert(item.id) {
                return Err(ErroRag::Item(format!(
                    "item repetido na lista: {}",
                    item.id
                )));
            }
        }

        let identidade = self.embedder.identidade().clone();
        let atuais = {
            let conn = trava(&self.conn);
            ler_hashes(&conn, org, agente)?
        };

        let mut relatorio = RelatorioIndexacao::default();
        let mut para_gravar: Vec<Preparado> = Vec::new();
        let mut para_apagar: Vec<String> = Vec::new();
        for (posicao, item) in itens.iter().enumerate() {
            let hash = hash_origem(&identidade, &item.titulo, &item.texto);
            let id = item.id.to_string();
            match atuais.get(&id) {
                Some(anterior) if *anterior == hash => {
                    relatorio.itens_inalterados += 1;
                    continue;
                }
                Some(_) => {
                    relatorio.itens_alterados += 1;
                    para_apagar.push(id);
                }
                None => relatorio.itens_novos += 1,
            }
            para_gravar.push(Preparado {
                posicao,
                hash,
                trechos: dividir(&item.texto),
                vetores: Vec::new(),
            });
        }
        for id in atuais.keys() {
            if !itens.iter().any(|item| item.id.to_string() == *id) {
                relatorio.itens_removidos += 1;
                para_apagar.push(id.clone());
            }
        }
        if para_gravar.is_empty() && para_apagar.is_empty() {
            return Ok(relatorio);
        }

        // Textos a embedar: um por trecho, na ordem dos itens e dos trechos.
        let mut textos: Vec<String> = Vec::new();
        for preparado in &para_gravar {
            let item = &itens[preparado.posicao];
            textos.extend(
                preparado
                    .trechos
                    .iter()
                    .map(|trecho| texto_embutido(&item.titulo, trecho)),
            );
        }
        relatorio.embeddings_gerados = textos.len();

        let mut vetores: Vec<Vec<f32>> = Vec::with_capacity(textos.len());
        for bloco in textos.chunks(LOTE_INDEXACAO) {
            let gerados = self.embedder.embedir_lote(bloco).await?;
            if gerados.len() != bloco.len() {
                return Err(ErroRag::Inferencia(format!(
                    "o modelo devolveu {} vetores para {} textos",
                    gerados.len(),
                    bloco.len()
                )));
            }
            vetores.extend(gerados);
        }

        // Devolve os vetores a cada item, na mesma ordem em que foram pedidos.
        let mut restantes = vetores.into_iter();
        for preparado in para_gravar.iter_mut() {
            for _ in 0..preparado.trechos.len() {
                let vetor = restantes.next().ok_or_else(|| {
                    ErroRag::Inferencia("faltam vetores para os trechos do item".to_owned())
                })?;
                if vetor.len() != DIMENSAO {
                    return Err(ErroRag::Inferencia(format!(
                        "vetor com {} posições; o esperado é {DIMENSAO}",
                        vetor.len()
                    )));
                }
                let q = quantizar(&vetor);
                let n = norma(&q);
                preparado.vetores.push((q, n));
            }
        }

        {
            let mut conn = trava(&self.conn);
            let tx = conn.transaction().map_err(erro_banco)?;
            for id in &para_apagar {
                apagar_item(&tx, org, agente, id)?;
            }
            for preparado in &para_gravar {
                let item = &itens[preparado.posicao];
                tx.execute(
                    "INSERT INTO rag_itens (org_id, agente_id, item_id, titulo, versao, hash_origem, trechos)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        org.to_string(),
                        agente.to_string(),
                        item.id.to_string(),
                        item.titulo,
                        item.versao as i64,
                        preparado.hash,
                        preparado.trechos.len() as i64,
                    ],
                )
                .map_err(erro_banco)?;
                for (ordem, (texto, (vetor, n))) in
                    preparado.trechos.iter().zip(&preparado.vetores).enumerate()
                {
                    let bytes: Vec<u8> = vetor.iter().map(|x| *x as u8).collect();
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
                    .map_err(erro_banco)?;
                    let rowid = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO rag_trechos_fts (rowid, texto_busca, org_id, agente_id)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            rowid,
                            texto_embutido(&item.titulo, texto),
                            org.to_string(),
                            agente.to_string(),
                        ],
                    )
                    .map_err(erro_banco)?;
                    relatorio.trechos_gravados += 1;
                }
            }
            tx.commit().map_err(erro_banco)?;
        }

        trava(&self.cache).invalidar((org, agente));
        tracing::info!(
            novos = relatorio.itens_novos,
            alterados = relatorio.itens_alterados,
            inalterados = relatorio.itens_inalterados,
            removidos = relatorio.itens_removidos,
            trechos = relatorio.trechos_gravados,
            "índice do agente atualizado"
        );
        Ok(relatorio)
    }

    /// Os `limite` trechos mais parecidos com `consulta`, por cosseno sobre os vetores int8 do
    /// agente. Varredura exata: compara com todos os trechos do agente.
    pub async fn buscar_vetor(
        &self,
        org: OrgId,
        agente: AgentId,
        consulta: &str,
        limite: usize,
    ) -> Result<Vec<Encontrado>, ErroRag> {
        if limite == 0 {
            return Err(ErroRag::Configuracao(
                "o limite de resultados deve ser maior que zero".to_owned(),
            ));
        }
        if consulta.trim().is_empty() {
            return Ok(Vec::new());
        }
        let textos = vec![consulta.to_owned()];
        let geradas = self.embedder.embedir_lote(&textos).await?;
        let vetor = geradas.into_iter().next().ok_or_else(|| {
            ErroRag::Inferencia("o modelo não devolveu vetor da consulta".to_owned())
        })?;
        if vetor.len() != DIMENSAO {
            return Err(ErroRag::Inferencia(format!(
                "vetor da consulta com {} posições; o esperado é {DIMENSAO}",
                vetor.len()
            )));
        }
        let q = quantizar(&vetor);
        let nq = norma(&q);

        let indice = self.carregar(org, agente)?;
        let melhores = varrer(&indice, &q, nq, limite);

        let conn = trava(&self.conn);
        let mut saida = Vec::with_capacity(melhores.len());
        for (pontuacao, rowid) in melhores {
            if let Some(trecho) = detalhe(&conn, org, agente, rowid)? {
                saida.push(Encontrado { trecho, pontuacao });
            }
        }
        Ok(saida)
    }

    /// Os `limite` trechos com mais palavras em comum com `consulta` (FTS5, por BM25). Acentos e
    /// maiúsculas não contam. Vazio quando a consulta não tem palavras.
    pub fn buscar_palavras(
        &self,
        org: OrgId,
        agente: AgentId,
        consulta: &str,
        limite: usize,
    ) -> Result<Vec<Encontrado>, ErroRag> {
        if limite == 0 {
            return Err(ErroRag::Configuracao(
                "o limite de resultados deve ser maior que zero".to_owned(),
            ));
        }
        let achados = self.palavras_com_rowid(org, agente, consulta, limite)?;
        let conn = trava(&self.conn);
        let mut saida = Vec::with_capacity(achados.len());
        for (rowid, relevancia) in achados {
            if let Some(trecho) = detalhe(&conn, org, agente, rowid)? {
                saida.push(Encontrado {
                    trecho,
                    pontuacao: -(relevancia as f32),
                });
            }
        }
        Ok(saida)
    }

    /// Todos os trechos gravados do agente, em ordem de item e de posição. Serve para conferir o
    /// que foi indexado.
    pub fn listar_trechos(&self, org: OrgId, agente: AgentId) -> Result<Vec<Trecho>, ErroRag> {
        let conn = trava(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT t.item_id, t.ordem, i.titulo, t.texto
                 FROM rag_trechos t
                 JOIN rag_itens i
                   ON i.org_id = t.org_id AND i.agente_id = t.agente_id AND i.item_id = t.item_id
                 WHERE t.org_id = ?1 AND t.agente_id = ?2
                 ORDER BY t.item_id, t.ordem",
            )
            .map_err(erro_banco)?;
        let linhas = stmt
            .query_map(params![org.to_string(), agente.to_string()], |linha| {
                Ok((
                    linha.get::<_, String>(0)?,
                    linha.get::<_, i64>(1)?,
                    linha.get::<_, String>(2)?,
                    linha.get::<_, String>(3)?,
                ))
            })
            .map_err(erro_banco)?;
        let mut saida = Vec::new();
        for linha in linhas {
            let (item, ordem, titulo, texto) = linha.map_err(erro_banco)?;
            saida.push(Trecho {
                item: item_id(&item)?,
                ordem: ordem as usize,
                titulo,
                texto,
            });
        }
        Ok(saida)
    }

    /// Busca híbrida: vetor (cosseno, varredura exata) + FTS5 (palavras), combinados por rank
    /// (RRF) e filtrados pelo corte de relevância do agente (PRD §9, HR-17).
    ///
    /// - Os dois lados trazem até `profundidade` candidatos.
    /// - O corte é aplicado no **cosseno**: trecho abaixo dele não volta, venha do vetor ou das
    ///   palavras. É o que impede injetar trecho irrelevante quando a base não tem a resposta.
    /// - A ordenação final é pela nota RRF; empate desempata por similaridade e depois por rowid
    ///   (resultado determinístico).
    pub async fn buscar_hibrida(
        &self,
        org: OrgId,
        agente: AgentId,
        consulta: &str,
        config: &ConfigBusca,
    ) -> Result<Vec<Achado>, ErroRag> {
        if config.top_k == 0 {
            return Err(ErroRag::Configuracao(
                "o top-k da busca deve ser maior que zero".to_owned(),
            ));
        }
        if consulta.trim().is_empty() {
            return Ok(Vec::new());
        }
        let profundidade = config.profundidade.max(config.top_k);
        let textos = vec![consulta.to_owned()];
        let geradas = self.embedder.embedir_lote(&textos).await?;
        let vetor = geradas.into_iter().next().ok_or_else(|| {
            ErroRag::Inferencia("o modelo não devolveu vetor da consulta".to_owned())
        })?;
        if vetor.len() != DIMENSAO {
            return Err(ErroRag::Inferencia(format!(
                "vetor da consulta com {} posições; o esperado é {DIMENSAO}",
                vetor.len()
            )));
        }
        let q = quantizar(&vetor);
        let nq = norma(&q);

        // As duas listas de candidatos: vetor (ordem do cosseno) e palavras (ordem do BM25).
        let indice = self.carregar(org, agente)?;
        let palavras = self.palavras_com_rowid(org, agente, consulta, profundidade)?;
        let alvos: HashSet<i64> = palavras.iter().map(|(rowid, _)| *rowid).collect();
        let (vetor_top, cos_alvos) = varrer_com_alvos(&indice, &q, nq, profundidade, &alvos);

        // Combinação por rank: 1/(k + posição) em cada lista, somado.
        let mut combinado: HashMap<i64, (f32, f32)> = HashMap::new();
        for (posicao, (similaridade, rowid)) in vetor_top.iter().enumerate() {
            if *similaridade < config.corte {
                continue;
            }
            let nota = 1.0 / (RRF_K + (posicao + 1) as f32);
            combinado.insert(*rowid, (*similaridade, nota));
        }
        for (posicao, (rowid, _bm25)) in palavras.iter().enumerate() {
            let similaridade = match cos_alvos.get(rowid) {
                Some(valor) => *valor,
                // Sem vetor do trecho não há como aplicar o corte; fica de fora.
                None => continue,
            };
            if similaridade < config.corte {
                continue;
            }
            let nota = 1.0 / (RRF_K + (posicao + 1) as f32);
            let entrada = combinado.entry(*rowid).or_insert((similaridade, 0.0));
            entrada.0 = entrada.0.max(similaridade);
            entrada.1 += nota;
        }

        let mut ordenado: Vec<(i64, f32, f32)> = combinado
            .into_iter()
            .map(|(rowid, (similaridade, relevancia))| (rowid, similaridade, relevancia))
            .collect();
        ordenado.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.0.cmp(&b.0))
        });
        ordenado.truncate(config.top_k);

        let conn = trava(&self.conn);
        let mut saida = Vec::with_capacity(ordenado.len());
        for (rowid, similaridade, relevancia) in ordenado {
            if let Some(trecho) = detalhe(&conn, org, agente, rowid)? {
                saida.push(Achado {
                    trecho,
                    similaridade,
                    relevancia,
                });
            }
        }
        Ok(saida)
    }

    /// Itens indexados do agente e o hash de origem de cada um. A CLI usa para dizer se o item
    /// está indexado ou pendente (comparando com o hash que o conteúdo atual produziria).
    pub fn itens_indexados(
        &self,
        org: OrgId,
        agente: AgentId,
    ) -> Result<HashMap<KnowledgeItemId, String>, ErroRag> {
        let conn = trava(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT item_id, hash_origem FROM rag_itens WHERE org_id = ?1 AND agente_id = ?2",
            )
            .map_err(erro_banco)?;
        let linhas = stmt
            .query_map(params![org.to_string(), agente.to_string()], |linha| {
                Ok((linha.get::<_, String>(0)?, linha.get::<_, String>(1)?))
            })
            .map_err(erro_banco)?;
        let mut mapa = HashMap::new();
        for linha in linhas {
            let (id, hash) = linha.map_err(erro_banco)?;
            mapa.insert(item_id(&id)?, hash);
        }
        Ok(mapa)
    }

    /// `rowid` e relevância BM25 (menor é melhor) dos trechos com mais palavras em comum.
    fn palavras_com_rowid(
        &self,
        org: OrgId,
        agente: AgentId,
        consulta: &str,
        limite: usize,
    ) -> Result<Vec<(i64, f64)>, ErroRag> {
        let Some(expressao) = expressao_fts(consulta) else {
            return Ok(Vec::new());
        };
        let conn = trava(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT rowid, bm25(rag_trechos_fts) FROM rag_trechos_fts
                 WHERE rag_trechos_fts MATCH ?1 AND org_id = ?2 AND agente_id = ?3
                 ORDER BY bm25(rag_trechos_fts), rowid
                 LIMIT ?4",
            )
            .map_err(erro_banco)?;
        let achados: Vec<(i64, f64)> = stmt
            .query_map(
                params![
                    expressao,
                    org.to_string(),
                    agente.to_string(),
                    limite as i64
                ],
                |linha| Ok((linha.get(0)?, linha.get(1)?)),
            )
            .map_err(erro_banco)?
            .collect::<Result<_, _>>()
            .map_err(erro_banco)?;
        Ok(achados)
    }

    // ───────────────────────────── Produtos (HR-18) ─────────────────────────────

    /// Deixa o índice de produtos do agente igual à lista `produtos`: indexa o novo e o alterado,
    /// apaga o que saiu da lista (produto inativo, de outra categoria, apagado) e não mexe no resto.
    ///
    /// O hash de origem é calculado sobre o texto indexável (nome, aliases, descrição, tags). Mudar
    /// **preço** não muda o hash, então não reembeda nada — o preço é lido do `harness.db`.
    pub async fn indexar_produtos(
        &self,
        org: OrgId,
        agente: AgentId,
        produtos: &[ProdutoRag],
    ) -> Result<RelatorioIndexacao, ErroRag> {
        let _vez = self.escrita.lock().await;

        let mut vistos = HashSet::new();
        for produto in produtos {
            if !vistos.insert(produto.id) {
                return Err(ErroRag::Item(format!(
                    "produto repetido na lista: {}",
                    produto.id
                )));
            }
        }

        let identidade = self.embedder.identidade().clone();
        let atuais = {
            let conn = trava(&self.conn);
            ler_hashes_produtos(&conn, org, agente)?
        };

        let mut relatorio = RelatorioIndexacao::default();
        let mut para_gravar: Vec<ProdutoPreparado> = Vec::new();
        let mut para_apagar: Vec<String> = Vec::new();
        for (posicao, produto) in produtos.iter().enumerate() {
            let hash = hash_origem(&identidade, &produto.nome, &produto.texto);
            let id = produto.id.to_string();
            match atuais.get(&id) {
                Some(anterior) if *anterior == hash => {
                    relatorio.itens_inalterados += 1;
                    continue;
                }
                Some(_) => {
                    relatorio.itens_alterados += 1;
                    para_apagar.push(id);
                }
                None => relatorio.itens_novos += 1,
            }
            para_gravar.push((posicao, hash, dividir(&produto.texto), Vec::new()));
        }
        for id in atuais.keys() {
            if !produtos.iter().any(|produto| produto.id.to_string() == *id) {
                relatorio.itens_removidos += 1;
                para_apagar.push(id.clone());
            }
        }
        if para_gravar.is_empty() && para_apagar.is_empty() {
            return Ok(relatorio);
        }

        let mut textos: Vec<String> = Vec::new();
        for (posicao, _, trechos, _) in &para_gravar {
            let produto = &produtos[*posicao];
            textos.extend(
                trechos
                    .iter()
                    .map(|trecho| texto_embutido(&produto.nome, trecho)),
            );
        }
        relatorio.embeddings_gerados = textos.len();
        let mut vetores = self.embedar_textos(&textos).await?.into_iter();
        for (_, _, trechos, alvo) in para_gravar.iter_mut() {
            alvo.clear();
            for _ in 0..trechos.len() {
                let vetor = vetores.next().ok_or_else(|| {
                    ErroRag::Inferencia("faltam vetores para os trechos do produto".to_owned())
                })?;
                alvo.push(vetor);
            }
        }

        {
            let mut conn = trava(&self.conn);
            let tx = conn.transaction().map_err(erro_banco)?;
            for id in &para_apagar {
                apagar_produto(&tx, org, agente, id)?;
            }
            for (posicao, hash, trechos, vetores) in &para_gravar {
                let produto = &produtos[*posicao];
                tx.execute(
                    "INSERT INTO rag_produtos
                        (org_id, agente_id, produto_id, nome, versao, hash_origem, trechos)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        org.to_string(),
                        agente.to_string(),
                        produto.id.to_string(),
                        produto.nome,
                        produto.versao as i64,
                        hash,
                        trechos.len() as i64,
                    ],
                )
                .map_err(erro_banco)?;
                for (ordem, (texto, (vetor, n))) in trechos.iter().zip(vetores).enumerate() {
                    let bytes: Vec<u8> = vetor.iter().map(|x| *x as u8).collect();
                    tx.execute(
                        "INSERT INTO rag_produtos_trechos
                            (org_id, agente_id, produto_id, ordem, texto, vetor, norma)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            org.to_string(),
                            agente.to_string(),
                            produto.id.to_string(),
                            ordem as i64,
                            texto,
                            bytes,
                            f64::from(*n),
                        ],
                    )
                    .map_err(erro_banco)?;
                    let rowid = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO rag_produtos_fts (rowid, texto_busca, org_id, agente_id)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            rowid,
                            texto_embutido(&produto.nome, texto),
                            org.to_string(),
                            agente.to_string(),
                        ],
                    )
                    .map_err(erro_banco)?;
                    relatorio.trechos_gravados += 1;
                }
            }
            tx.commit().map_err(erro_banco)?;
        }

        tracing::info!(
            novos = relatorio.itens_novos,
            alterados = relatorio.itens_alterados,
            inalterados = relatorio.itens_inalterados,
            removidos = relatorio.itens_removidos,
            trechos = relatorio.trechos_gravados,
            "índice de produtos do agente atualizado"
        );
        Ok(relatorio)
    }

    /// Busca híbrida de produtos (vetor + FTS5, combinados por RRF), com corte no cosseno.
    ///
    /// Devolve **um resultado por produto** (o trecho que melhor casou), no máximo `top_k`.
    pub async fn buscar_produtos(
        &self,
        org: OrgId,
        agente: AgentId,
        consulta: &str,
        config: &ConfigBusca,
    ) -> Result<Vec<AchadoProduto>, ErroRag> {
        if config.top_k == 0 {
            return Err(ErroRag::Configuracao(
                "o top-k da busca deve ser maior que zero".to_owned(),
            ));
        }
        if consulta.trim().is_empty() {
            return Ok(Vec::new());
        }
        let profundidade = config.profundidade.max(config.top_k);
        let (q, nq) = self.vetor_da_consulta(consulta).await?;

        let indice = {
            let conn = trava(&self.conn);
            ler_indice_produtos(&conn, org, agente)?
        };
        let palavras = {
            let conn = trava(&self.conn);
            palavras_produtos_com_rowid(&conn, org, agente, consulta, profundidade)?
        };
        let alvos: HashSet<i64> = palavras.iter().map(|(rowid, _)| *rowid).collect();
        let (vetor_top, cos_alvos) = varrer_com_alvos(&indice, &q, nq, profundidade, &alvos);

        let mut combinado: HashMap<i64, (f32, f32)> = HashMap::new();
        for (posicao, (similaridade, rowid)) in vetor_top.iter().enumerate() {
            if *similaridade < config.corte {
                continue;
            }
            let nota = 1.0 / (RRF_K + (posicao + 1) as f32);
            combinado.insert(*rowid, (*similaridade, nota));
        }
        for (posicao, (rowid, _)) in palavras.iter().enumerate() {
            let similaridade = match cos_alvos.get(rowid) {
                Some(valor) => *valor,
                None => continue,
            };
            if similaridade < config.corte {
                continue;
            }
            let nota = 1.0 / (RRF_K + (posicao + 1) as f32);
            let entrada = combinado.entry(*rowid).or_insert((similaridade, 0.0));
            entrada.0 = entrada.0.max(similaridade);
            entrada.1 += nota;
        }

        let mut ordenado: Vec<(i64, f32, f32)> = combinado
            .into_iter()
            .map(|(rowid, (similaridade, relevancia))| (rowid, similaridade, relevancia))
            .collect();
        ordenado.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.0.cmp(&b.0))
        });

        let conn = trava(&self.conn);
        let mut saida: Vec<AchadoProduto> = Vec::new();
        for (rowid, similaridade, relevancia) in ordenado {
            let Some((produto, nome, trecho)) = detalhe_produto(&conn, org, agente, rowid)? else {
                continue;
            };
            // Um resultado por produto: o primeiro (melhor colocado) manda.
            if saida.iter().any(|achado| achado.produto == produto) {
                continue;
            }
            saida.push(AchadoProduto {
                produto,
                nome,
                trecho,
                similaridade,
                relevancia,
            });
            if saida.len() == config.top_k {
                break;
            }
        }
        Ok(saida)
    }

    /// Busca **só vetorial** de produtos: os `limite` produtos mais próximos por cosseno.
    ///
    /// Serve de régua para a híbrida (medição da fixtura) e para diagnóstico. A busca que o agente
    /// usa é a [`Indice::buscar_produtos`].
    pub async fn buscar_produtos_vetor(
        &self,
        org: OrgId,
        agente: AgentId,
        consulta: &str,
        limite: usize,
    ) -> Result<Vec<AchadoProduto>, ErroRag> {
        if limite == 0 {
            return Err(ErroRag::Configuracao(
                "o limite de resultados deve ser maior que zero".to_owned(),
            ));
        }
        if consulta.trim().is_empty() {
            return Ok(Vec::new());
        }
        let (q, nq) = self.vetor_da_consulta(consulta).await?;
        let indice = {
            let conn = trava(&self.conn);
            ler_indice_produtos(&conn, org, agente)?
        };
        let melhores = varrer(&indice, &q, nq, limite * 4);
        let conn = trava(&self.conn);
        let mut saida: Vec<AchadoProduto> = Vec::new();
        for (similaridade, rowid) in melhores {
            let Some((produto, nome, trecho)) = detalhe_produto(&conn, org, agente, rowid)? else {
                continue;
            };
            if saida.iter().any(|achado| achado.produto == produto) {
                continue;
            }
            saida.push(AchadoProduto {
                produto,
                nome,
                trecho,
                similaridade,
                relevancia: similaridade,
            });
            if saida.len() == limite {
                break;
            }
        }
        Ok(saida)
    }

    /// Produtos indexados do agente e o hash de origem de cada um (a CLI compara com o hash que o
    /// conteúdo atual produziria para dizer se está indexado ou pendente).
    pub fn produtos_indexados(
        &self,
        org: OrgId,
        agente: AgentId,
    ) -> Result<HashMap<ProductId, String>, ErroRag> {
        let conn = trava(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT produto_id, hash_origem FROM rag_produtos
                  WHERE org_id = ?1 AND agente_id = ?2",
            )
            .map_err(erro_banco)?;
        let linhas = stmt
            .query_map(params![org.to_string(), agente.to_string()], |linha| {
                Ok((linha.get::<_, String>(0)?, linha.get::<_, String>(1)?))
            })
            .map_err(erro_banco)?;
        let mut mapa = HashMap::new();
        for linha in linhas {
            let (id, hash) = linha.map_err(erro_banco)?;
            mapa.insert(
                ProductId::from_str(&id)
                    .map_err(|_| ErroRag::Banco("id de produto inválido no rag.db".to_owned()))?,
                hash,
            );
        }
        Ok(mapa)
    }

    /// Vetor (int8) e norma da consulta.
    async fn vetor_da_consulta(&self, consulta: &str) -> Result<(Vec<i8>, f32), ErroRag> {
        let textos = vec![consulta.to_owned()];
        let geradas = self.embedder.embedir_lote(&textos).await?;
        let vetor = geradas.into_iter().next().ok_or_else(|| {
            ErroRag::Inferencia("o modelo não devolveu vetor da consulta".to_owned())
        })?;
        if vetor.len() != DIMENSAO {
            return Err(ErroRag::Inferencia(format!(
                "vetor da consulta com {} posições; o esperado é {DIMENSAO}",
                vetor.len()
            )));
        }
        let q = quantizar(&vetor);
        let nq = norma(&q);
        Ok((q, nq))
    }

    /// Embeda os textos em lotes e devolve (vetor int8, norma) na ordem pedida.
    async fn embedar_textos(&self, textos: &[String]) -> Result<Vec<(Vec<i8>, f32)>, ErroRag> {
        let mut vetores: Vec<Vec<f32>> = Vec::with_capacity(textos.len());
        for bloco in textos.chunks(LOTE_INDEXACAO) {
            let gerados = self.embedder.embedir_lote(bloco).await?;
            if gerados.len() != bloco.len() {
                return Err(ErroRag::Inferencia(format!(
                    "o modelo devolveu {} vetores para {} textos",
                    gerados.len(),
                    bloco.len()
                )));
            }
            vetores.extend(gerados);
        }
        let mut saida = Vec::with_capacity(vetores.len());
        for vetor in vetores {
            if vetor.len() != DIMENSAO {
                return Err(ErroRag::Inferencia(format!(
                    "vetor com {} posições; o esperado é {DIMENSAO}",
                    vetor.len()
                )));
            }
            let q = quantizar(&vetor);
            let n = norma(&q);
            saida.push((q, n));
        }
        Ok(saida)
    }

    /// Identidade do modelo que gerou os vetores. A CLI usa para calcular o hash que o conteúdo
    /// atual de um item produziria e dizer se ele está indexado ou pendente.
    pub fn identidade(&self) -> &IdentidadeModelo {
        self.embedder.identidade()
    }

    /// Quantos agentes estão carregados na memória agora.
    pub fn agentes_em_memoria(&self) -> usize {
        trava(&self.cache).entradas.len()
    }

    /// Bytes ocupados pelo cache de agentes (estimativa pelo tamanho dos vetores).
    pub fn bytes_em_memoria(&self) -> usize {
        trava(&self.cache).bytes
    }

    /// Orçamento de memória configurado para o cache.
    pub fn orcamento_memoria_bytes(&self) -> usize {
        self.orcamento
    }

    /// Carrega o índice do agente, do cache ou do `rag.db`. Quando o orçamento estoura, tira o
    /// agente ocioso há mais tempo.
    fn carregar(&self, org: OrgId, agente: AgentId) -> Result<Arc<IndiceAgente>, ErroRag> {
        let chave = (org, agente);
        let geracao = {
            let mut cache = trava(&self.cache);
            cache.relogio += 1;
            let agora = cache.relogio;
            if let Some(entrada) = cache.entradas.get_mut(&chave) {
                entrada.ultimo_uso = agora;
                return Ok(Arc::clone(&entrada.indice));
            }
            cache.geracao(&chave)
        };

        let lido = Arc::new(self.ler_indice(org, agente)?);
        let bytes = lido.bytes();

        let mut cache = trava(&self.cache);
        if cache.geracao(&chave) != geracao {
            return Ok(lido);
        }
        if let Some(entrada) = cache.entradas.get(&chave) {
            return Ok(Arc::clone(&entrada.indice));
        }
        if bytes > self.orcamento {
            // Não cabe nem sozinho: responde sem guardar.
            return Ok(lido);
        }
        while cache.bytes + bytes > self.orcamento {
            let ocioso = cache
                .entradas
                .iter()
                .min_by_key(|(_, entrada)| entrada.ultimo_uso)
                .map(|(chave, _)| *chave);
            match ocioso {
                Some(chave_ociosa) => cache.remover(&chave_ociosa),
                None => break,
            }
        }
        cache.relogio += 1;
        let agora = cache.relogio;
        cache.bytes += bytes;
        cache.entradas.insert(
            chave,
            Entrada {
                indice: Arc::clone(&lido),
                ultimo_uso: agora,
            },
        );
        Ok(lido)
    }

    fn ler_indice(&self, org: OrgId, agente: AgentId) -> Result<IndiceAgente, ErroRag> {
        let conn = trava(&self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, vetor, norma FROM rag_trechos
                 WHERE org_id = ?1 AND agente_id = ?2
                 ORDER BY id",
            )
            .map_err(erro_banco)?;
        let linhas = stmt
            .query_map(params![org.to_string(), agente.to_string()], |linha| {
                Ok((
                    linha.get::<_, i64>(0)?,
                    linha.get::<_, Vec<u8>>(1)?,
                    linha.get::<_, f64>(2)?,
                ))
            })
            .map_err(erro_banco)?;
        let mut indice = IndiceAgente::default();
        for linha in linhas {
            let (id, bytes, n) = linha.map_err(erro_banco)?;
            if bytes.len() != DIMENSAO {
                return Err(ErroRag::Banco(format!(
                    "vetor gravado com {} posições; o esperado é {DIMENSAO}. Reindexe o agente",
                    bytes.len()
                )));
            }
            indice.ids.push(id);
            indice.vetores.extend(bytes.iter().map(|b| *b as i8));
            indice.normas.push(n as f32);
        }
        Ok(indice)
    }
}

/// Varredura exata: cosseno contra cada trecho do agente. Devolve até `limite` pares
/// (pontuação, rowid), do maior para o menor. Em empate, fica o trecho que veio antes.
fn varrer(indice: &IndiceAgente, q: &[i8], nq: f32, limite: usize) -> Vec<(f32, i64)> {
    let mut melhores: Vec<(f32, i64)> = Vec::with_capacity(limite + 1);
    for (posicao, vetor) in indice.vetores.as_chunks::<DIMENSAO>().0.iter().enumerate() {
        let pontuacao = cosseno(produto_interno(q, vetor), nq, indice.normas[posicao]);
        if melhores.len() == limite && pontuacao <= melhores[limite - 1].0 {
            continue;
        }
        let onde = melhores
            .iter()
            .position(|(atual, _)| pontuacao > *atual)
            .unwrap_or(melhores.len());
        melhores.insert(onde, (pontuacao, indice.ids[posicao]));
        if melhores.len() > limite {
            melhores.pop();
        }
    }
    melhores
}

/// Varredura única que devolve os `limite` melhores pelo cosseno **e** o cosseno dos `rowid`
/// pedidos em `alvos` (os candidatos vindos do FTS, que não estão necessariamente no topo).
fn varrer_com_alvos(
    indice: &IndiceAgente,
    q: &[i8],
    nq: f32,
    limite: usize,
    alvos: &HashSet<i64>,
) -> (Vec<(f32, i64)>, HashMap<i64, f32>) {
    let mut melhores: Vec<(f32, i64)> = Vec::with_capacity(limite + 1);
    let mut cos_alvos: HashMap<i64, f32> = HashMap::with_capacity(alvos.len());
    for (posicao, vetor) in indice.vetores.as_chunks::<DIMENSAO>().0.iter().enumerate() {
        let pontuacao = cosseno(produto_interno(q, vetor), nq, indice.normas[posicao]);
        let rowid = indice.ids[posicao];
        if alvos.contains(&rowid) {
            cos_alvos.insert(rowid, pontuacao);
        }
        if melhores.len() == limite && pontuacao <= melhores[limite - 1].0 {
            continue;
        }
        let onde = melhores
            .iter()
            .position(|(atual, _)| pontuacao > *atual)
            .unwrap_or(melhores.len());
        melhores.insert(onde, (pontuacao, rowid));
        if melhores.len() > limite {
            melhores.pop();
        }
    }
    (melhores, cos_alvos)
}

/// Palavras que não ajudam a buscar: se entrassem na consulta do FTS5, qualquer item casaria e a
/// combinação por rank premiasse ruído. Lista curta de português, só o que aparece em pergunta.
const PALAVRAS_VAZIAS: &[&str] = &[
    "aos", "aquela", "aquele", "as", "ate", "com", "como", "da", "das", "de", "dela", "dele",
    "depois", "do", "dos", "e", "ela", "elas", "ele", "eles", "em", "entre", "era", "essa", "esse",
    "esta", "este", "eu", "foi", "for", "ha", "isso", "isto", "ja", "mais", "mas", "me", "mesmo",
    "meu", "minha", "muito", "na", "nas", "nem", "no", "nos", "nossa", "nosso", "num", "o", "os",
    "ou", "para", "pela", "pelo", "por", "pra", "qual", "que", "quem", "se", "sem", "ser", "seu",
    "sua", "suas", "seus", "so", "sou", "sua", "ta", "tem", "tenho", "ter", "teu", "tu", "tua",
    "um", "uma", "voce", "voces", "vos",
];

/// Consulta do FTS5: cada palavra entre aspas, unidas por OU. Aspas dentro da palavra são
/// escapadas, palavras vazias (artigo, preposição) saem e palavra com quatro letras ou mais vira
/// prefixo — é o que faz "rato" achar "ratos" e "barata" achar "baratas".
///
/// Sem palavra que sirva, devolve `None` (a busca híbrida fica só com o vetor).
fn expressao_fts(consulta: &str) -> Option<String> {
    let mut palavras: Vec<String> = Vec::new();
    for palavra in consulta.split(|c: char| !c.is_alphanumeric()) {
        // Acentos não contam para a lista: o tokenizador do FTS5 já os ignora.
        let sem_acento: String = palavra
            .chars()
            .map(|c| match c {
                'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'í' | 'ì' | 'î' | 'ï' => 'i',
                'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
                'ú' | 'ù' | 'û' | 'ü' => 'u',
                'ç' => 'c',
                outro => outro.to_ascii_lowercase(),
            })
            .collect();
        if sem_acento.chars().count() < 3 || PALAVRAS_VAZIAS.contains(&sem_acento.as_str()) {
            continue;
        }
        let escapada = palavra.replace('"', "\"\"");
        let termo = if palavra.chars().count() >= 4 {
            format!("\"{escapada}\"*")
        } else {
            format!("\"{escapada}\"")
        };
        if !palavras.contains(&termo) {
            palavras.push(termo);
        }
        if palavras.len() == 16 {
            break;
        }
    }
    if palavras.is_empty() {
        None
    } else {
        Some(palavras.join(" OR "))
    }
}

fn item_id(texto: &str) -> Result<KnowledgeItemId, ErroRag> {
    KnowledgeItemId::from_str(texto)
        .map_err(|_| ErroRag::Banco("id de item inválido no rag.db".to_owned()))
}

/// Hashes de origem dos itens já indexados do agente, por id do item.
fn ler_hashes(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
) -> Result<HashMap<String, String>, ErroRag> {
    let mut stmt = conn
        .prepare("SELECT item_id, hash_origem FROM rag_itens WHERE org_id = ?1 AND agente_id = ?2")
        .map_err(erro_banco)?;
    let linhas = stmt
        .query_map(params![org.to_string(), agente.to_string()], |linha| {
            Ok((linha.get::<_, String>(0)?, linha.get::<_, String>(1)?))
        })
        .map_err(erro_banco)?;
    let mut mapa = HashMap::new();
    for linha in linhas {
        let (id, hash) = linha.map_err(erro_banco)?;
        mapa.insert(id, hash);
    }
    Ok(mapa)
}

/// Apaga um item e os trechos e entradas de FTS dele, nesta ordem (chave estrangeira).
fn apagar_item(
    tx: &rusqlite::Transaction<'_>,
    org: OrgId,
    agente: AgentId,
    id: &str,
) -> Result<(), ErroRag> {
    let (org, agente) = (org.to_string(), agente.to_string());
    let rowids: Vec<i64> = {
        let mut stmt = tx
            .prepare(
                "SELECT id FROM rag_trechos WHERE org_id = ?1 AND agente_id = ?2 AND item_id = ?3",
            )
            .map_err(erro_banco)?;
        let linhas = stmt
            .query_map(params![org, agente, id], |linha| linha.get(0))
            .map_err(erro_banco)?;
        linhas.collect::<Result<_, _>>().map_err(erro_banco)?
    };
    for rowid in rowids {
        tx.execute(
            "DELETE FROM rag_trechos_fts WHERE rowid = ?1",
            params![rowid],
        )
        .map_err(erro_banco)?;
    }
    tx.execute(
        "DELETE FROM rag_trechos WHERE org_id = ?1 AND agente_id = ?2 AND item_id = ?3",
        params![org, agente, id],
    )
    .map_err(erro_banco)?;
    tx.execute(
        "DELETE FROM rag_itens WHERE org_id = ?1 AND agente_id = ?2 AND item_id = ?3",
        params![org, agente, id],
    )
    .map_err(erro_banco)?;
    Ok(())
}

/// Um trecho pelo `rowid`, só se for da org e do agente pedidos.
fn detalhe(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
    rowid: i64,
) -> Result<Option<Trecho>, ErroRag> {
    let linha = conn
        .query_row(
            "SELECT t.item_id, t.ordem, i.titulo, t.texto
             FROM rag_trechos t
             JOIN rag_itens i
               ON i.org_id = t.org_id AND i.agente_id = t.agente_id AND i.item_id = t.item_id
             WHERE t.id = ?1 AND t.org_id = ?2 AND t.agente_id = ?3",
            params![rowid, org.to_string(), agente.to_string()],
            |linha| {
                Ok((
                    linha.get::<_, String>(0)?,
                    linha.get::<_, i64>(1)?,
                    linha.get::<_, String>(2)?,
                    linha.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(erro_banco)?;
    match linha {
        Some((item, ordem, titulo, texto)) => Ok(Some(Trecho {
            item: item_id(&item)?,
            ordem: ordem as usize,
            titulo,
            texto,
        })),
        None => Ok(None),
    }
}

// ───────────────────────────── Apoio da busca de produtos (HR-18) ─────────────────────────────

/// Hashes de origem dos produtos já indexados do agente, por id do produto.
fn ler_hashes_produtos(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
) -> Result<HashMap<String, String>, ErroRag> {
    let mut stmt = conn
        .prepare(
            "SELECT produto_id, hash_origem FROM rag_produtos
              WHERE org_id = ?1 AND agente_id = ?2",
        )
        .map_err(erro_banco)?;
    let linhas = stmt
        .query_map(params![org.to_string(), agente.to_string()], |linha| {
            Ok((linha.get::<_, String>(0)?, linha.get::<_, String>(1)?))
        })
        .map_err(erro_banco)?;
    let mut mapa = HashMap::new();
    for linha in linhas {
        let (id, hash) = linha.map_err(erro_banco)?;
        mapa.insert(id, hash);
    }
    Ok(mapa)
}

/// Apaga um produto e os trechos e entradas de FTS dele, nesta ordem (chave estrangeira).
fn apagar_produto(
    tx: &rusqlite::Transaction<'_>,
    org: OrgId,
    agente: AgentId,
    id: &str,
) -> Result<(), ErroRag> {
    let (org, agente) = (org.to_string(), agente.to_string());
    let rowids: Vec<i64> = {
        let mut stmt = tx
            .prepare(
                "SELECT id FROM rag_produtos_trechos
                  WHERE org_id = ?1 AND agente_id = ?2 AND produto_id = ?3",
            )
            .map_err(erro_banco)?;
        let linhas = stmt
            .query_map(params![org, agente, id], |linha| linha.get(0))
            .map_err(erro_banco)?;
        linhas.collect::<Result<_, _>>().map_err(erro_banco)?
    };
    for rowid in rowids {
        tx.execute(
            "DELETE FROM rag_produtos_fts WHERE rowid = ?1",
            params![rowid],
        )
        .map_err(erro_banco)?;
    }
    tx.execute(
        "DELETE FROM rag_produtos_trechos WHERE org_id = ?1 AND agente_id = ?2 AND produto_id = ?3",
        params![org, agente, id],
    )
    .map_err(erro_banco)?;
    tx.execute(
        "DELETE FROM rag_produtos WHERE org_id = ?1 AND agente_id = ?2 AND produto_id = ?3",
        params![org, agente, id],
    )
    .map_err(erro_banco)?;
    Ok(())
}

/// Vetores dos produtos do agente, lidos do `rag.db` na hora (o catálogo é pequeno; ver o limite
/// documentado em `docs/PRODUTOS.md`).
fn ler_indice_produtos(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
) -> Result<IndiceAgente, ErroRag> {
    let mut stmt = conn
        .prepare(
            "SELECT id, vetor, norma FROM rag_produtos_trechos
              WHERE org_id = ?1 AND agente_id = ?2
              ORDER BY id",
        )
        .map_err(erro_banco)?;
    let linhas = stmt
        .query_map(params![org.to_string(), agente.to_string()], |linha| {
            Ok((
                linha.get::<_, i64>(0)?,
                linha.get::<_, Vec<u8>>(1)?,
                linha.get::<_, f64>(2)?,
            ))
        })
        .map_err(erro_banco)?;
    let mut indice = IndiceAgente::default();
    for linha in linhas {
        let (id, bytes, n) = linha.map_err(erro_banco)?;
        if bytes.len() != DIMENSAO {
            return Err(ErroRag::Banco(format!(
                "vetor de produto gravado com {} posições; o esperado é {DIMENSAO}. Reindexe o agente",
                bytes.len()
            )));
        }
        indice.ids.push(id);
        indice.vetores.extend(bytes.iter().map(|b| *b as i8));
        indice.normas.push(n as f32);
    }
    Ok(indice)
}

/// `rowid` e relevância BM25 dos trechos de produto com mais palavras em comum.
fn palavras_produtos_com_rowid(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
    consulta: &str,
    limite: usize,
) -> Result<Vec<(i64, f64)>, ErroRag> {
    let Some(expressao) = expressao_fts(consulta) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn
        .prepare(
            "SELECT rowid, bm25(rag_produtos_fts) FROM rag_produtos_fts
              WHERE rag_produtos_fts MATCH ?1 AND org_id = ?2 AND agente_id = ?3
              ORDER BY bm25(rag_produtos_fts), rowid
              LIMIT ?4",
        )
        .map_err(erro_banco)?;
    let achados: Vec<(i64, f64)> = stmt
        .query_map(
            params![
                expressao,
                org.to_string(),
                agente.to_string(),
                limite as i64
            ],
            |linha| Ok((linha.get(0)?, linha.get(1)?)),
        )
        .map_err(erro_banco)?
        .collect::<Result<_, _>>()
        .map_err(erro_banco)?;
    Ok(achados)
}

/// Trecho de produto pelo `rowid`, só se for da org e do agente pedidos.
fn detalhe_produto(
    conn: &Connection,
    org: OrgId,
    agente: AgentId,
    rowid: i64,
) -> Result<Option<(ProductId, String, String)>, ErroRag> {
    let linha = conn
        .query_row(
            "SELECT t.produto_id, p.nome, t.texto
             FROM rag_produtos_trechos t
             JOIN rag_produtos p
               ON p.org_id = t.org_id AND p.agente_id = t.agente_id AND p.produto_id = t.produto_id
             WHERE t.id = ?1 AND t.org_id = ?2 AND t.agente_id = ?3",
            params![rowid, org.to_string(), agente.to_string()],
            |linha| {
                Ok((
                    linha.get::<_, String>(0)?,
                    linha.get::<_, String>(1)?,
                    linha.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(erro_banco)?;
    match linha {
        Some((produto, nome, texto)) => {
            let id = ProductId::from_str(&produto)
                .map_err(|_| ErroRag::Banco("id de produto inválido no rag.db".to_owned()))?;
            Ok(Some((id, nome, texto)))
        }
        None => Ok(None),
    }
}
