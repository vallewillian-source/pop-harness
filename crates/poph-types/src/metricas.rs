//! Registro de métricas em memória (PRD §15), sem HTTP.
//!
//! A HR-20 pendura [`Registro::renderizar`] na rota `/metrics`. Aqui só há contadores,
//! medidores e histogramas com estado atômico; nada é lido do sistema.
//! Rótulos (ex.: por org) ainda não existem: entram quando a HR-20 precisar deles.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{DomainError, DomainResult};

/// Conjunto de métricas. Cada nome é registrado uma vez; repetir o nome é erro.
#[derive(Debug, Default)]
pub struct Registro {
    itens: Mutex<Vec<Item>>,
}

#[derive(Debug)]
struct Item {
    nome: &'static str,
    ajuda: &'static str,
    tipo: Tipo,
}

#[derive(Debug)]
enum Tipo {
    Contador(Arc<AtomicU64>),
    Medidor(Arc<AtomicI64>),
    Histograma(Arc<Mutex<Estado>>, Vec<f64>),
}

#[derive(Debug, Default)]
struct Estado {
    /// Uma posição por limite, mais a última para acima do maior limite (+Inf).
    contagens: Vec<u64>,
    soma: f64,
    total: u64,
}

/// Valor que só cresce (ex.: chamadas ao LLM).
#[derive(Debug, Clone)]
pub struct Contador(Arc<AtomicU64>);

/// Valor que sobe e desce (ex.: conversas ativas).
#[derive(Debug, Clone)]
pub struct Medidor(Arc<AtomicI64>);

/// Distribuição de valores (ex.: latência) em faixas fixas.
#[derive(Debug, Clone)]
pub struct Histograma {
    estado: Arc<Mutex<Estado>>,
    limites: Vec<f64>,
}

impl Contador {
    /// Soma `n` ao contador.
    pub fn incrementar(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    /// Valor atual.
    pub fn valor(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Medidor {
    /// Define o valor atual.
    pub fn definir(&self, valor: i64) {
        self.0.store(valor, Ordering::Relaxed);
    }

    /// Valor atual.
    pub fn valor(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Histograma {
    /// Registra uma observação. Ela cai na primeira faixa cujo limite seja maior ou igual a ela.
    pub fn observar(&self, valor: f64) {
        let mut estado = match self.estado.lock() {
            Ok(guarda) => guarda,
            Err(veneno) => veneno.into_inner(),
        };
        let posicao = self
            .limites
            .iter()
            .position(|limite| valor <= *limite)
            .unwrap_or(self.limites.len());
        estado.contagens[posicao] += 1;
        estado.soma += valor;
        estado.total += 1;
    }
}

impl Registro {
    /// Cria um registro vazio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra um contador.
    pub fn contador(&self, nome: &'static str, ajuda: &'static str) -> DomainResult<Contador> {
        let valor = Arc::new(AtomicU64::new(0));
        self.inserir(nome, ajuda, Tipo::Contador(Arc::clone(&valor)))?;
        Ok(Contador(valor))
    }

    /// Registra um medidor.
    pub fn medidor(&self, nome: &'static str, ajuda: &'static str) -> DomainResult<Medidor> {
        let valor = Arc::new(AtomicI64::new(0));
        self.inserir(nome, ajuda, Tipo::Medidor(Arc::clone(&valor)))?;
        Ok(Medidor(valor))
    }

    /// Registra um histograma. `limites` são os limites superiores das faixas, em ordem crescente.
    pub fn histograma(
        &self,
        nome: &'static str,
        ajuda: &'static str,
        limites: &[f64],
    ) -> DomainResult<Histograma> {
        if limites.is_empty() || limites.windows(2).any(|par| par[0] >= par[1]) {
            return Err(validacao(nome, "limites devem ser não vazios e crescentes"));
        }
        let estado = Arc::new(Mutex::new(Estado {
            contagens: vec![0; limites.len() + 1],
            soma: 0.0,
            total: 0,
        }));
        self.inserir(
            nome,
            ajuda,
            Tipo::Histograma(Arc::clone(&estado), limites.to_vec()),
        )?;
        Ok(Histograma {
            estado,
            limites: limites.to_vec(),
        })
    }

    /// Texto no formato de exposição do Prometheus (o que a rota `/metrics` devolve).
    pub fn renderizar(&self) -> String {
        let itens = match self.itens.lock() {
            Ok(guarda) => guarda,
            Err(veneno) => veneno.into_inner(),
        };
        let mut saida = String::new();
        for item in itens.iter() {
            let _ = writeln!(saida, "# HELP {} {}", item.nome, item.ajuda);
            match &item.tipo {
                Tipo::Contador(valor) => {
                    let _ = writeln!(saida, "# TYPE {} counter", item.nome);
                    let _ = writeln!(saida, "{} {}", item.nome, valor.load(Ordering::Relaxed));
                }
                Tipo::Medidor(valor) => {
                    let _ = writeln!(saida, "# TYPE {} gauge", item.nome);
                    let _ = writeln!(saida, "{} {}", item.nome, valor.load(Ordering::Relaxed));
                }
                Tipo::Histograma(estado, limites) => {
                    let _ = writeln!(saida, "# TYPE {} histogram", item.nome);
                    let estado = match estado.lock() {
                        Ok(guarda) => guarda,
                        Err(veneno) => veneno.into_inner(),
                    };
                    let mut acumulado = 0u64;
                    for (posicao, limite) in limites.iter().enumerate() {
                        acumulado += estado.contagens[posicao];
                        let _ = writeln!(
                            saida,
                            "{}_bucket{{le=\"{}\"}} {}",
                            item.nome, limite, acumulado
                        );
                    }
                    acumulado += estado.contagens[limites.len()];
                    let _ = writeln!(saida, "{}_bucket{{le=\"+Inf\"}} {}", item.nome, acumulado);
                    let _ = writeln!(saida, "{}_sum {}", item.nome, estado.soma);
                    let _ = writeln!(saida, "{}_count {}", item.nome, estado.total);
                }
            }
        }
        saida
    }

    fn inserir(&self, nome: &'static str, ajuda: &'static str, tipo: Tipo) -> DomainResult<()> {
        if !nome_valido(nome) {
            return Err(validacao(
                nome,
                "use o prefixo poph_ e só letras minúsculas, números e _",
            ));
        }
        let mut itens = match self.itens.lock() {
            Ok(guarda) => guarda,
            Err(veneno) => veneno.into_inner(),
        };
        if itens.iter().any(|item| item.nome == nome) {
            return Err(validacao(nome, "métrica já registrada"));
        }
        itens.push(Item { nome, ajuda, tipo });
        Ok(())
    }
}

/// Métricas base do PRD §15, registradas com nomes fixos. Devolve os handles para os módulos
/// que vão medir (turno, LLM, Jev, Evolution); os que dependem de nomes de org entram depois, com rótulos.
#[derive(Debug, Clone)]
pub struct MetricasBase {
    /// RAM do processo, em bytes.
    pub memoria_processo_bytes: Medidor,
    /// Agentes com configuração carregada na memória.
    pub agentes_carregados: Medidor,
    /// Conversas com ator ativo.
    pub conversas_ativas: Medidor,
    /// Turnos esperando na fila.
    pub fila_turnos: Medidor,
    /// Mensagens esperando envio.
    pub fila_saida: Medidor,
    /// Latência de cada etapa do turno, em segundos.
    pub latencia_etapa_segundos: Histograma,
    /// Chamadas ao LLM.
    pub chamadas_llm: Contador,
    /// Erros nas chamadas ao LLM.
    pub erros_llm: Contador,
    /// Chamadas ao Jev.
    pub chamadas_jev: Contador,
    /// Erros nas chamadas ao Jev.
    pub erros_jev: Contador,
    /// Chamadas à Evolution.
    pub chamadas_evolution: Contador,
    /// Erros nas chamadas à Evolution.
    pub erros_evolution: Contador,
    /// Tokens consumidos (entrada e saída somadas), sem divisão por org ainda.
    pub tokens_total: Contador,
    /// Tamanho do `harness.db`, em bytes.
    pub tamanho_harness_db_bytes: Medidor,
    /// Tamanho do `rag.db`, em bytes.
    pub tamanho_rag_db_bytes: Medidor,
}

impl MetricasBase {
    /// Registra o catálogo base no registro dado.
    pub fn registrar(registro: &Registro) -> DomainResult<Self> {
        Ok(Self {
            memoria_processo_bytes: registro.medidor(
                "poph_memoria_processo_bytes",
                "Memória residente do processo, em bytes.",
            )?,
            agentes_carregados: registro.medidor(
                "poph_agentes_carregados",
                "Agentes com configuração carregada na memória.",
            )?,
            conversas_ativas: registro
                .medidor("poph_conversas_ativas", "Conversas com ator ativo.")?,
            fila_turnos: registro.medidor("poph_fila_turnos", "Turnos esperando na fila.")?,
            fila_saida: registro.medidor("poph_fila_saida", "Mensagens esperando envio.")?,
            latencia_etapa_segundos: registro.histograma(
                "poph_latencia_etapa_segundos",
                "Latência de cada etapa do turno, em segundos.",
                &[0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0],
            )?,
            chamadas_llm: registro.contador("poph_chamadas_llm_total", "Chamadas ao LLM.")?,
            erros_llm: registro.contador("poph_erros_llm_total", "Erros nas chamadas ao LLM.")?,
            chamadas_jev: registro.contador("poph_chamadas_jev_total", "Chamadas ao Jev.")?,
            erros_jev: registro.contador("poph_erros_jev_total", "Erros nas chamadas ao Jev.")?,
            chamadas_evolution: registro
                .contador("poph_chamadas_evolution_total", "Chamadas à Evolution.")?,
            erros_evolution: registro.contador(
                "poph_erros_evolution_total",
                "Erros nas chamadas à Evolution.",
            )?,
            tokens_total: registro
                .contador("poph_tokens_total", "Tokens consumidos (entrada e saída).")?,
            tamanho_harness_db_bytes: registro
                .medidor("poph_harness_db_bytes", "Tamanho do harness.db, em bytes.")?,
            tamanho_rag_db_bytes: registro
                .medidor("poph_rag_db_bytes", "Tamanho do rag.db, em bytes.")?,
        })
    }
}

fn nome_valido(nome: &str) -> bool {
    nome.starts_with("poph_")
        && nome
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn validacao(nome: &'static str, motivo: &str) -> DomainError {
    DomainError::Validacao {
        campo: "metrica",
        motivo: format!("{nome}: {motivo}"),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn contador_e_medidor_aparecem_no_texto() {
        let registro = Registro::new();
        let chamadas = registro
            .contador("poph_chamadas_llm_total", "Chamadas ao LLM.")
            .expect("registra");
        let conversas = registro
            .medidor("poph_conversas_ativas", "Conversas ativas.")
            .expect("registra");
        chamadas.incrementar(2);
        chamadas.incrementar(1);
        conversas.definir(7);
        conversas.definir(5);
        let texto = registro.renderizar();
        assert!(
            texto.contains("# TYPE poph_chamadas_llm_total counter\npoph_chamadas_llm_total 3\n")
        );
        assert!(texto.contains("# TYPE poph_conversas_ativas gauge\npoph_conversas_ativas 5\n"));
    }

    #[test]
    fn histograma_acumula_faixas_no_formato_prometheus() {
        let registro = Registro::new();
        let latencia = registro
            .histograma("poph_latencia_teste_segundos", "Teste.", &[0.1, 1.0])
            .expect("registra");
        latencia.observar(0.05);
        latencia.observar(0.5);
        latencia.observar(3.0);
        let texto = registro.renderizar();
        assert!(texto.contains("poph_latencia_teste_segundos_bucket{le=\"0.1\"} 1\n"));
        assert!(texto.contains("poph_latencia_teste_segundos_bucket{le=\"1\"} 2\n"));
        assert!(texto.contains("poph_latencia_teste_segundos_bucket{le=\"+Inf\"} 3\n"));
        assert!(texto.contains("poph_latencia_teste_segundos_count 3\n"));
    }

    #[test]
    fn nome_repetido_ou_fora_do_padrao_e_recusado() {
        let registro = Registro::new();
        registro
            .contador("poph_x_total", "Teste.")
            .expect("primeiro registro");
        assert!(registro.contador("poph_x_total", "Outro.").is_err());
        assert!(registro.contador("chamadas", "Sem prefixo.").is_err());
        assert!(registro.contador("poph_Maiuscula", "Não pode.").is_err());
    }

    #[test]
    fn catalogo_base_registra_sem_conflito() {
        let registro = Registro::new();
        let base = MetricasBase::registrar(&registro).expect("catálogo registra");
        base.chamadas_llm.incrementar(1);
        base.latencia_etapa_segundos.observar(0.002);
        let texto = registro.renderizar();
        assert!(texto.contains("poph_chamadas_llm_total 1"));
        assert!(texto.contains("poph_latencia_etapa_segundos_count 1"));
    }
}
