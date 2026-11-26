//! O compilador de script de fluxo para a estrutura compilada (IR): FX-10.
//!
//! A porta de entrada do módulo é [`compilar`]: recebe o texto do `.star` e devolve a IR
//! (`schema_versao = 1`, [`crate::ir`]) mais a lista de diagnósticos de forma com linha e coluna.
//! [`carregar`] é o outro lado: recebe a IR **gravada** numa revisão e o texto, recompila e confere
//! que é a mesma coisa — se não for, falha fechado com `ir_divergente` em vez de adivinhar qual das
//! duas versões vale.
//!
//! O caminho é: tamanho da fonte → parse no dialeto (§3) → leitura da árvore, que monta a IR e os
//! diagnósticos ([`forma`]) → carga do módulo no Starlark com limites (só para provar que ele
//! carrega e que os limites estão ligados). O compilador é **puro**: sem banco, sem rede, sem
//! relógio, sem arquivo. Duas compilações do mesmo texto produzem a mesma IR, byte a byte.
//!
//! Nada do script é executado durante a leitura da árvore. É por isso que `"x" * 1000000000` é
//! recusado em microssegundos, e não depois de alocar 2 GB.

use std::time::Duration;

pub(crate) mod forma;

use poph_types::config::FlowsConfig;

use crate::diagnostico::{Codigo, Diagnostico};
use crate::ir::Ir;
use crate::linguagem::{LimitesStarlark, analisa, carregar_modulo};

/// Teto padrão de tamanho da fonte: 32 KB (32.768 bytes), `FLUXOS_LINGUAGEM.md` §3.4 e decisão D1.
/// É o padrão de `fluxos.max_fonte_bytes`; o PRD §9 ainda traz 65536 (pendência P14).
pub const MAX_FONTE_BYTES: usize = 32 * 1024;
/// Teto de ticks da carga.
pub const MAX_TICKS: u64 = 5_000_000;
/// Heap da carga: 64 MB.
pub const MAX_HEAP_BYTES: usize = 64 * 1024 * 1024;
/// Profundidade de chamada na carga.
pub const MAX_CALLSTACK: usize = 128;
/// Prazo de parede da carga de um script.
pub const PRAZO_DE_CARGA: Duration = Duration::from_secs(1);

/// Limites de uma compilação.
///
/// São fixos nesta versão (`FLUXOS_LINGUAGEM.md` §3.4) e existem como parâmetro para o teste poder
/// apertá-los e provar que estão ligados de verdade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limites {
    /// Tamanho máximo da fonte, em bytes; acima disso o script é recusado antes do parse.
    pub max_fonte_bytes: usize,
    /// Etapas + decisões + fins por fluxo; acima disso a publicação é recusada (`FLX27`).
    pub max_nos: usize,
    /// Ticks da carga (conta cada passo da avaliação do módulo).
    pub max_ticks: u64,
    /// Heap máximo da carga, em bytes.
    pub max_heap_bytes: usize,
    /// Profundidade máxima de chamada da carga.
    pub max_callstack: usize,
    /// Prazo de parede da carga.
    pub prazo_de_carga: Duration,
}

impl Default for Limites {
    fn default() -> Self {
        Self {
            max_fonte_bytes: MAX_FONTE_BYTES,
            max_nos: forma::MAX_NOS,
            max_ticks: MAX_TICKS,
            max_heap_bytes: MAX_HEAP_BYTES,
            max_callstack: MAX_CALLSTACK,
            prazo_de_carga: PRAZO_DE_CARGA,
        }
    }
}

impl Limites {
    /// Os limites de compilação que a configuração `[fluxos]` do processo define (FX-26b).
    ///
    /// É o único ponto que converte a configuração em limites do compilador: quem compila ou
    /// carrega revisão no servidor passa por aqui. Campos sem chave em `[fluxos]` (ticks e heap da
    /// carga do módulo, callstack, prazo) ficam nos valores de [`Limites::default`]. Os limites de
    /// avaliação de regra vêm de [`crate::regras::LimitesRegra::da_config`].
    pub fn da_config(fluxos: &FlowsConfig) -> Self {
        Self {
            max_fonte_bytes: usize::try_from(fluxos.max_source_bytes).unwrap_or(usize::MAX),
            max_nos: usize::try_from(fluxos.max_nodes).unwrap_or(usize::MAX),
            ..Self::default()
        }
    }

    fn starlark(&self) -> LimitesStarlark {
        LimitesStarlark {
            max_ticks: self.max_ticks,
            max_heap_bytes: self.max_heap_bytes,
            max_callstack: self.max_callstack,
            prazo: self.prazo_de_carga,
        }
    }
}

/// Resultado de compilar um script.
///
/// A IR só existe quando **não há erro**: aviso não impede publicar, erro impede. Quem publica
/// confere [`Compilado::tem_erro`] antes de gravar.
#[derive(Debug, Clone)]
pub struct Compilado {
    /// A estrutura compilada; `None` quando há pelo menos um erro.
    pub ir: Option<Ir>,
    /// Diagnósticos de forma, na ordem em que foram encontrados (erros e avisos juntos).
    pub diagnosticos: Vec<Diagnostico>,
}

impl Compilado {
    /// Se algum diagnóstico bloqueia a publicação.
    pub fn tem_erro(&self) -> bool {
        self.diagnosticos.iter().any(Diagnostico::e_erro)
    }

    /// Só os erros.
    pub fn erros(&self) -> impl Iterator<Item = &Diagnostico> {
        self.diagnosticos.iter().filter(|d| d.e_erro())
    }

    /// Só os avisos.
    pub fn avisos(&self) -> impl Iterator<Item = &Diagnostico> {
        self.diagnosticos.iter().filter(|d| !d.e_erro())
    }

    /// A IR, ou os diagnósticos que impediram de gerá-la.
    pub fn ir(&self) -> Result<&Ir, &[Diagnostico]> {
        match &self.ir {
            Some(ir) => Ok(ir),
            None => Err(&self.diagnosticos),
        }
    }
}

/// O que falhou ao carregar a IR gravada de uma revisão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalhaCarregamento {
    /// O texto não compila: os diagnósticos explicam o que corrigir.
    Forma(Vec<Diagnostico>),
    /// O texto compila, mas o resultado não é a IR gravada (`FLXR06`): o fluxo falha fechado.
    Divergente {
        /// Id do fluxo da IR gravada.
        fluxo: String,
        /// O que divergiu, em linguagem comum.
        motivo: String,
    },
}

impl FalhaCarregamento {
    /// Código estável da falha, para log e API.
    pub fn codigo(&self) -> &'static str {
        match self {
            Self::Forma(_) => "fluxo_invalido",
            Self::Divergente { .. } => "ir_divergente",
        }
    }
}

/// Compila o texto de um fluxo.
///
/// Devolve sempre um resultado: problema no script é diagnóstico, não erro de função. Duas
/// compilações do mesmo texto dão a mesma IR (`Ir::para_json` é estável).
pub fn compilar(fonte: &str, limites: &Limites) -> Compilado {
    let mut diagnosticos = Vec::new();

    if fonte.len() > limites.max_fonte_bytes {
        diagnosticos.push(Diagnostico::erro(
            Codigo::FonteGrande,
            format!(
                "O script tem {} bytes e o limite é {} bytes ({} KB). Tire textos longos das instruções ou divida o fluxo em dois.",
                fonte.len(),
                limites.max_fonte_bytes,
                limites.max_fonte_bytes / 1024
            ),
            1,
            1,
        ));
        return Compilado {
            ir: None,
            diagnosticos,
        };
    }

    let modulo = match analisa(fonte, "fluxo.star") {
        Ok(modulo) => modulo,
        Err(erro) => {
            diagnosticos.push(diagnostico_do_parse(&erro));
            return Compilado {
                ir: None,
                diagnosticos,
            };
        }
    };

    let (ir, mut de_forma) = forma::compila(fonte, &modulo);
    diagnosticos.append(&mut de_forma);

    // Teto de nós da configuração (`fluxos.max_nos`). O teto de forma (60) já foi conferido pela
    // leitura da árvore; aqui vale o limite do processo, que pode ser menor.
    let ir = match ir {
        Some(ir) if ir.etapas.len() + ir.decisoes.len() + ir.fins.len() > limites.max_nos => {
            let nos = ir.etapas.len() + ir.decisoes.len() + ir.fins.len();
            diagnosticos.push(Diagnostico::erro(
                Codigo::NosDemais,
                format!(
                    "O fluxo tem {nos} nós e o limite é {}. Junte etapas parecidas ou divida o fluxo.",
                    limites.max_nos
                ),
                1,
                1,
            ));
            None
        }
        outro => outro,
    };

    // Última barreira: com a forma válida, o módulo tem de carregar de verdade (é o que o motor
    // faz na execução). Aqui os limites estão ligados; sem erro de forma, o único erro possível é
    // limite estourado, que vira `FLX04`.
    if ir.is_some()
        && let Err(erro) = carregar_modulo(fonte, "fluxo.star", &limites.starlark())
    {
        diagnosticos.push(diagnostico_da_carga(&erro));
        return Compilado {
            ir: None,
            diagnosticos,
        };
    }

    Compilado { ir, diagnosticos }
}

/// Carrega a IR gravada de uma revisão: recompila o texto e confere que dá a mesma estrutura.
///
/// É o que o motor chama antes de executar (com cache na FX-19). Divergência **não** derruba o
/// processo: vira [`FalhaCarregamento::Divergente`] e a execução falha fechada (`FLXR06`).
pub fn carregar(ir_gravada: &Ir, fonte: &str, limites: &Limites) -> Result<Ir, FalhaCarregamento> {
    let compilado = compilar(fonte, limites);
    let Some(recompilada) = compilado.ir else {
        return Err(FalhaCarregamento::Forma(compilado.diagnosticos));
    };
    if recompilada == *ir_gravada {
        return Ok(recompilada);
    }
    Err(FalhaCarregamento::Divergente {
        fluxo: ir_gravada.fluxo.id.clone(),
        motivo: diferencas(ir_gravada, &recompilada),
    })
}

/// Diz, em linguagem comum, o que divergiu entre a IR gravada e a recompilada.
fn diferencas(gravada: &Ir, recompilada: &Ir) -> String {
    let mut partes: Vec<&str> = Vec::new();
    if gravada.schema_versao != recompilada.schema_versao {
        partes.push("a versão do esquema");
    }
    if gravada.fluxo != recompilada.fluxo {
        partes.push("o cabeçalho do fluxo");
    }
    if gravada.gatilho != recompilada.gatilho {
        partes.push("o gatilho");
    }
    if gravada.inicio != recompilada.inicio {
        partes.push("a etapa de início");
    }
    if gravada.etapas != recompilada.etapas {
        partes.push("as etapas");
    }
    if gravada.decisoes != recompilada.decisoes {
        partes.push("as decisões");
    }
    if gravada.fins != recompilada.fins {
        partes.push("os fins");
    }
    if gravada.defs != recompilada.defs {
        partes.push("as funções de regra");
    }
    if partes.is_empty() {
        // Não deveria acontecer (a comparação acima já teria dado igual): a mensagem é honesta.
        return "a estrutura gravada difere da recompilação do texto".to_owned();
    }
    format!(
        "a estrutura gravada difere da recompilação do texto em {}",
        partes.join(", ")
    )
}

/// Traduz o erro do parser em um dos códigos da lista fechada.
fn diagnostico_do_parse(erro: &starlark::Error) -> Diagnostico {
    let (linha, coluna) = posicao_do_erro(erro);
    let texto = erro.to_string();
    let minusculo = texto.to_lowercase();
    let (codigo, mensagem) = if minusculo.contains("not allowed in this dialect")
        || minusculo.contains("reserved keyword")
        || minusculo.contains("cannot be used outside")
        || minusculo.contains("must only occur at the top")
        || minusculo.contains("not supported")
    {
        let qual = if minusculo.contains("load") {
            "`load`"
        } else if minusculo.contains("lambda") {
            "`lambda`"
        } else if minusculo.contains("f-string") || minusculo.contains("f_string") {
            "f-string"
        } else if minusculo.contains("for") {
            "`for`"
        } else if minusculo.contains("if") {
            "`if`"
        } else if minusculo.contains("while") {
            "`while`"
        } else if minusculo.contains("class") {
            "`class`"
        } else if minusculo.contains("try") {
            "`try`"
        } else {
            "esta construção"
        };
        (
            Codigo::Dialeto,
            format!(
                "{qual} não faz parte da linguagem de fluxos. Um fluxo descreve etapas, campos, ações e regras; cálculo livre só dentro de `def {}(dados)`.",
                "minha_regra"
            ),
        )
    } else {
        // A primeira linha do erro do parser já diz o que ele esperava; o resto é o desenho do
        // trecho, que não cabe numa mensagem de API.
        let resumo = texto.lines().next().unwrap_or(&texto).trim().to_owned();
        (
            Codigo::Sintaxe,
            format!("Erro de sintaxe no script: {resumo}"),
        )
    };
    Diagnostico::erro(codigo, mensagem, linha, coluna)
}

/// Traduz o erro da carga em um dos códigos da lista fechada.
fn diagnostico_da_carga(erro: &starlark::Error) -> Diagnostico {
    let (linha, coluna) = posicao_do_erro(erro);
    let texto = erro.to_string();
    let minusculo = texto.to_lowercase();
    if minusculo.contains("tick")
        || minusculo.contains("heap")
        || minusculo.contains("callstack")
        || minusculo.contains("cancelled")
        || minusculo.contains("stack overflow")
        || minusculo.contains("recursion")
    {
        return Diagnostico::erro(
            Codigo::LimiteDeCarga,
            "O script executou demais ao ser carregado (ticks, memória, profundidade ou prazo de 1 s). No topo do arquivo deve haver só declarações; confira se alguma `def` tem laço longo.",
            linha,
            coluna,
        );
    }
    if minusculo.contains("not found") {
        return Diagnostico::erro(
            Codigo::NomeInexistente,
            format!("O script usa um nome que não existe na linguagem de fluxos: {texto}"),
            linha,
            coluna,
        );
    }
    let resumo = texto.lines().next().unwrap_or(&texto).trim().to_owned();
    Diagnostico::erro(
        Codigo::Sintaxe,
        format!("O script não carregou: {resumo}"),
        linha,
        coluna,
    )
}

/// Linha e coluna (1-based) de um erro do Starlark; `1:1` quando ele não traz posição.
fn posicao_do_erro(erro: &starlark::Error) -> (u32, u32) {
    match erro.span() {
        Some(span) => {
            let resolvido = span.resolve_span();
            (
                resolvido.begin.line as u32 + 1,
                resolvido.begin.column as u32 + 1,
            )
        }
        None => (1, 1),
    }
}
