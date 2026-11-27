//! Regras: combinadores declarativos e funções avançadas (`def`) — FX-12.
//!
//! Uma regra é avaliada com o **snapshot dos dados de uma execução** (`dados[etapa][campo]`, o
//! mesmo formato de `fluxo_execucoes.dados_json`) e devolve `true`/`false`. Há dois caminhos, e a
//! diferença entre eles é a decisão D2 da especificação (`docs/FLUXOS_LINGUAGEM.md` §6):
//!
//! - **Combinadores** (`igual`, `diferente`, `maior`, `menor`, `contem`, `vazio`, `e`, `ou`, `nao`)
//!   são estrutura da IR e são avaliados **aqui, em Rust nativo**. Não passam pelo Starlark, não
//!   alocam valor de script e custam microssegundos.
//! - **`funcao("nome")`** roda a `def` correspondente no módulo Starlark **carregado uma vez**
//!   ([`Avaliador::novo`]) e exige `True`/`False` de volta.
//!
//! Tudo é puro: sem banco, sem rede, sem relógio de negócio e sem arquivo. O dado do cliente entra
//! **só como valor** (é montado como dicionário concreto), nunca como código: nada do que o cliente
//! escreveu é interpretado como Starlark.
//!
//! # Campo ausente
//!
//! Campo que a execução ainda não registrou (ou registrou como `null`) nunca dá pânico. A tabela
//! abaixo é contrato e está publicada em `FLUXOS_LINGUAGEM.md` §6.1:
//!
//! | Combinador | Campo ausente | Por quê |
//! |---|---|---|
//! | `igual` | falso | não há valor para comparar |
//! | `diferente` | verdadeiro | o que não existe é diferente do literal |
//! | `maior` / `menor` | falso | não há número |
//! | `contem` | falso | não há texto |
//! | `vazio` | **verdadeiro** | não preenchido é vazio (§5) |
//! | `e` / `ou` / `nao` | combinam os resultados acima | — |
//! | `funcao` | a `def` recebe os dados como estão e decide | — |
//!
//! # Prazo e limites
//!
//! Avaliar `def` sem prazo não é seguro: o Starlark confere heap e ticks a cada mil instruções, e
//! uma alocação única e enorme (`"x" * 1000000000`) acontece **antes** dessa conferência. Por isso
//! a `def` roda numa thread própria e o chamador espera no máximo `prazo` + [`TOLERANCIA_DO_PRAZO`]:
//! estourado o prazo, a regra responde `RegraLimite` na hora, sem depender de o script parar.
//! Laço e recursão são contidos pelo próprio Starlark (ticks, heap e profundidade de chamada).
//!
//! A **carga** do script ([`Avaliador::novo`]) tem a mesma proteção, pelo mesmo motivo: a soma ou
//! multiplicação de constantes é feita na compilação do script, então um cálculo constante enorme
//! dentro de uma `def` custa segundos na carga (medido: ~4 s para 10⁸, ~35 s para 10⁹) e o prazo
//! do Starlark só o percebe depois. A carga também roda em thread própria e o chamador recebe
//! `RegraLimite` no prazo da carga (1 s) mais a margem.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Map, Value as Json};
use starlark::environment::{FrozenModule, Module};
use starlark::eval::Evaluator;
use starlark::values::Value as StarValue;
use starlark::values::dict::AllocDict;
use starlark::values::list::AllocList;

use poph_types::DomainError;
use poph_types::config::FlowsConfig;

use crate::compilador::Limites;
use crate::ir::Rule;
use crate::language::{StarlarkLimits, load_module};

/// Erro de avaliação de regra.
///
/// São sempre dois, e os dois dizem de quem é o problema:
///
/// - [`DomainError::RegraInvalida`] (`regra_invalida`) — defeito do **autor** do fluxo: `funcao`
///   sem `def`, `def` que devolveu texto, `def` que estourou (chave ausente, divisão por zero),
///   comparação com literal que não é número, campo escrito fora do formato `etapa.campo`;
/// - [`DomainError::RegraLimite`] (`regra_limite`) — a regra passou de ticks, heap, profundidade de
///   chamada ou prazo.
///
/// Nenhum dos dois derruba o processo nem o turno: o motor (FX-13) marca a execução como `falha`
/// (`FLXR01`–`FLXR03`) e a Ficha continua aberta.
pub type ErroRegra = DomainError;

/// Snapshot dos dados de uma execução: `{"etapa": {"campo": valor}}`.
///
/// É o mesmo formato de `fluxo_execucoes.dados_json` (`FLUXOS_LINGUAGEM.md` §5) e o dicionário que
/// a `def` recebe. Ordenado por chave (mapa de `BTreeMap`), então a avaliação é determinística.
pub type Dados = Map<String, Json>;

/// Teto de ticks de uma regra (`FLUXOS_LINGUAGEM.md` §3.4).
pub const MAX_TICKS_REGRA: u64 = 10_000;
/// Heap de uma regra: 1 MiB.
pub const MAX_HEAP_REGRA_BYTES: usize = 1024 * 1024;
/// Profundidade de chamada de uma regra.
pub const MAX_CALLSTACK_REGRA: usize = 128;
/// Prazo de parede de uma regra.
pub const PRAZO_DA_REGRA: Duration = Duration::from_millis(200);
/// Margem que o chamador espera além do prazo antes de considerar a `def` perdida.
pub const TOLERANCIA_DO_PRAZO: Duration = Duration::from_millis(25);
/// Profundidade máxima de aninhamento aceita numa regra.
///
/// A IR publicada tem no máximo `MAX_NOS` nós, mas uma IR gravada à mão pode vir aninhada até
/// estourar a pilha nativa: a avaliação para em 32 níveis com erro tipado, em vez de derrubar o
/// processo.
pub const MAX_PROFUNDIDADE: usize = 32;

/// Limites da avaliação de uma regra.
///
/// Os valores de [`Default`] são os da especificação (§3.4). Existe como parâmetro para o teste
/// apertar cada limite e provar que está ligado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitesRegra {
    /// Ticks antes de considerar laço infinito.
    pub max_ticks: u64,
    /// Heap máximo, em bytes.
    pub max_heap_bytes: usize,
    /// Profundidade máxima de chamada.
    pub max_callstack: usize,
    /// Prazo de parede.
    pub prazo: Duration,
}

impl Default for LimitesRegra {
    fn default() -> Self {
        Self {
            max_ticks: MAX_TICKS_REGRA,
            max_heap_bytes: MAX_HEAP_REGRA_BYTES,
            max_callstack: MAX_CALLSTACK_REGRA,
            prazo: PRAZO_DA_REGRA,
        }
    }
}

impl LimitesRegra {
    /// Os limites de uma avaliação de regra que a configuração `[fluxos]` define (FX-26b):
    /// `fluxos.max_ticks_regra` e `fluxos.max_heap_regra`. Callstack e prazo ficam no padrão.
    pub fn da_config(fluxos: &FlowsConfig) -> Self {
        Self {
            max_ticks: fluxos.max_rule_ticks,
            max_heap_bytes: usize::try_from(fluxos.max_rule_heap).unwrap_or(usize::MAX),
            ..Self::default()
        }
    }
}

/// Avaliador de regras de um fluxo.
///
/// O script é carregado **uma vez** ([`Avaliador::novo`]) e o módulo congelado serve todas as
/// regras daquela passagem: é o que a FX-19 vai guardar no cache de revisões. Regra que não chama
/// `def` dispensa o módulo ([`Avaliador::sem_funcoes`]) e não paga nada por ele.
#[derive(Clone)]
pub struct Avaliador {
    modulo: Option<Arc<FrozenModule>>,
    limites: LimitesRegra,
}

impl std::fmt::Debug for Avaliador {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avaliador")
            .field("modulo", &self.modulo.is_some())
            .field("limites", &self.limites)
            .finish()
    }
}

impl Avaliador {
    /// Avaliador só de combinadores: `funcao("nome")` responde `RegraInvalida`.
    pub fn sem_funcoes() -> Self {
        Self {
            modulo: None,
            limites: LimitesRegra::default(),
        }
    }

    /// Carrega o script (limites da FX-10) e deixa pronto para avaliar regras com as `def` dele.
    pub fn novo(fonte: &str) -> Result<Self, ErroRegra> {
        Self::novo_com(fonte, &Limites::default(), LimitesRegra::default())
    }

    /// Como [`Avaliador::novo`], com limites escolhidos (o teste aperta; a produção usa o padrão).
    pub fn novo_com(
        fonte: &str,
        carga: &Limites,
        limites: LimitesRegra,
    ) -> Result<Self, ErroRegra> {
        let starlark = StarlarkLimits {
            max_ticks: carga.max_ticks,
            max_heap_bytes: carga.max_heap_bytes,
            max_callstack: carga.max_callstack,
            deadline: carga.prazo_de_carga,
        };
        let modulo = carregar_com_prazo(fonte, starlark)?;
        Ok(Self {
            modulo: Some(Arc::new(modulo)),
            limites,
        })
    }

    /// Os limites em vigor.
    pub fn limites(&self) -> &LimitesRegra {
        &self.limites
    }

    /// Avalia a regra com os dados da execução.
    ///
    /// Sempre devolve booleano ou erro tipado: campo ausente é tratado pela tabela do topo do
    /// módulo, nunca com pânico.
    pub fn avaliar(&self, regra: &Rule, dados: &Dados) -> Result<bool, ErroRegra> {
        self.avaliar_no(regra, dados, 0)
    }

    fn avaliar_no(
        &self,
        regra: &Rule,
        dados: &Dados,
        profundidade: usize,
    ) -> Result<bool, ErroRegra> {
        if profundidade > MAX_PROFUNDIDADE {
            return Err(invalida(format!(
                "a regra tem mais de {MAX_PROFUNDIDADE} níveis de aninhamento"
            )));
        }
        match regra {
            Rule::Igual {
                campo,
                value: valor,
            } => Ok(igual(valor_do_campo(dados, campo)?, valor)),
            Rule::Diferente {
                campo,
                value: valor,
            } => Ok(!igual(valor_do_campo(dados, campo)?, valor)),
            Rule::Maior {
                campo,
                value: valor,
            } => comparar(
                valor_do_campo(dados, campo)?,
                valor,
                campo,
                std::cmp::Ordering::Greater,
            ),
            Rule::Menor {
                campo,
                value: valor,
            } => comparar(
                valor_do_campo(dados, campo)?,
                valor,
                campo,
                std::cmp::Ordering::Less,
            ),
            Rule::Contem {
                campo,
                value: valor,
            } => Ok(contem(valor_do_campo(dados, campo)?, valor)),
            Rule::Vazio { campo } => Ok(vazio(valor_do_campo(dados, campo)?)),
            Rule::E { args } => {
                for arg in args {
                    if !self.avaliar_no(arg, dados, profundidade + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Rule::Ou { args } => {
                for arg in args {
                    if self.avaliar_no(arg, dados, profundidade + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Rule::Nao { args } => match args.as_slice() {
                [unico] => Ok(!self.avaliar_no(unico, dados, profundidade + 1)?),
                _ => Err(invalida(format!(
                    "`nao` recebe exatamente 1 condição; a regra tem {}",
                    args.len()
                ))),
            },
            Rule::Funcao { nome } => self.avaliar_funcao(nome, dados),
        }
    }

    /// Roda a `def` no módulo carregado, com prazo.
    fn avaliar_funcao(&self, nome: &str, dados: &Dados) -> Result<bool, ErroRegra> {
        let modulo = self.modulo.clone().ok_or_else(|| {
            invalida(format!(
                "a regra chama `funcao(\"{nome}\")`, mas este avaliador não carregou o script: \
                 use `Avaliador::novo`"
            ))
        })?;
        let limites = self.limites.clone();
        let prazo = limites.prazo;
        let cancelado = Arc::new(AtomicBool::new(false));
        let (saida_tx, saida_rx) = mpsc::channel::<SaidaFuncao>();

        let nome_thread = nome.to_owned();
        let nome_modulo = nome_thread.clone();
        let dados = dados.clone();
        let flag = Arc::clone(&cancelado);
        let limites_thread = limites.clone();
        let criada = std::thread::Builder::new()
            .name(format!("regra-{nome}"))
            .spawn(move || {
                let saida =
                    executar_funcao(&modulo, &nome_modulo, &dados, &limites_thread, flag, prazo);
                // O chamador pode já ter desistido (prazo estourado): o envio falhar não é erro.
                let _ = saida_tx.send(saida);
            });
        if criada.is_err() {
            return Err(limite(format!(
                "não foi possível abrir a thread da função `{nome}`"
            )));
        }

        match saida_rx.recv_timeout(prazo + TOLERANCIA_DO_PRAZO) {
            Ok(SaidaFuncao::Booleano(valor)) => Ok(valor),
            Ok(SaidaFuncao::NaoBooleano) => Err(invalida(format!(
                "a função `{nome}` devolveu algo que não é `True`/`False`; uma regra precisa \
                 decidir sim ou não"
            ))),
            Ok(SaidaFuncao::Falha(motivo)) => {
                Err(invalida(format!("a função `{nome}` falhou: {motivo}")))
            }
            Ok(SaidaFuncao::Limite(motivo)) => Err(limite(motivo)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(limite(format!(
                "a função `{nome}` passou do prazo de {} ms",
                prazo.as_millis()
            ))),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(invalida(format!(
                "a função `{nome}` terminou sem devolver resposta"
            ))),
        }
    }
}

/// Carrega o script numa thread própria e espera, no máximo, o prazo da carga mais a margem.
///
/// O prazo do Starlark **não é preemptivo**. Um cálculo constante enorme escrito dentro de uma
/// `def` (`"x" * 1000000000`) é feito quando o script é compilado, e o Starlark só percebe o prazo
/// depois de segundos (medido: ~4 s para 10⁸, ~35 s para 10⁹). Por isso a carga roda em thread
/// separada: o chamador recebe `RegraLimite` no prazo combinado, e a thread abandonada termina
/// sozinha. Sem isso, uma revisão publicada assim travaria o turno do atendimento.
fn carregar_com_prazo(fonte: &str, limites: StarlarkLimits) -> Result<FrozenModule, ErroRegra> {
    let prazo = limites.deadline;
    let (envio, recepcao) = mpsc::channel();
    let fonte = fonte.to_owned();
    std::thread::Builder::new()
        .name("fluxo-carga".to_owned())
        .spawn(move || {
            // Se o chamador já desistiu, o envio falha e não é erro.
            let _ = envio.send(load_module(&fonte, "fluxo", &limites));
        })
        .map_err(|erro| limite(format!("não foi possível abrir a thread da carga: {erro}")))?;

    match recepcao.recv_timeout(prazo + TOLERANCIA_DO_PRAZO) {
        Ok(Ok(modulo)) => Ok(modulo),
        Ok(Err(erro)) => Err(classificar_carga(&erro)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(limite(format!(
            "o script não carregou em {} ms e a carga foi abandonada; cálculo constante enorme \
             escrito direto no script (por exemplo `\"x\" * 1000000000` dentro de uma `def`) é \
             feito na carga e não pode ser interrompido",
            prazo.as_millis()
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(invalida(
            "a carga do script terminou sem responder".to_owned(),
        )),
    }
}

/// Erro da carga do script → erro de regra: limite estourado é `RegraLimite`, o resto é
/// `RegraInvalida` (script com defeito de autor).
fn classificar_carga(erro: &starlark::Error) -> ErroRegra {
    let texto = primeira_linha(&erro.to_string());
    if eh_limite(&texto) || matches!(erro.kind(), starlark::ErrorKind::StackOverflow(_)) {
        return limite(format!(
            "o script do fluxo passou dos limites da carga: {texto}"
        ));
    }
    invalida(format!("o script do fluxo não carregou: {texto}"))
}

/// Mensagem de erro do Starlark que quer dizer "passou dos limites".
///
/// O Starlark não devolve um tipo de erro próprio para tick, heap e prazo; o critério é o texto
/// que ele publica. Está coberto por teste (`limites_da_carga_viram_regra_limite`).
fn eh_limite(texto: &str) -> bool {
    texto.contains("Heap memory limit")
        || texto.contains("ticks has been exceeded")
        || texto.contains("Evaluation cancelled")
        || texto.contains("call stack")
        || texto.contains("Starlark call stack")
}

/// Avalia uma regra **sem** `def`: combinadores apenas.
///
/// Regra que chama `funcao("nome")` devolve [`DomainError::RegraInvalida`] — para usar `def` é
/// preciso o módulo carregado de [`Avaliador::novo`].
pub fn avaliar(regra: &Rule, dados: &Dados) -> Result<bool, ErroRegra> {
    Avaliador::sem_funcoes().avaliar(regra, dados)
}

/// Lê `dados_json` no formato `{"etapa": {"campo": valor}}`.
///
/// Texto ilegível ou JSON que não é objeto é erro de regra (`FLXR05` no motor): nada é avaliado com
/// dado pela metade.
pub fn dados_de_json(texto: &str) -> Result<Dados, ErroRegra> {
    let bruto: Json = serde_json::from_str(texto)
        .map_err(|erro| invalida(format!("os dados da execução não são JSON: {erro}")))?;
    match bruto {
        Json::Object(mapa) => Ok(mapa),
        _ => Err(invalida(
            "os dados da execução precisam ser um objeto `{\"etapa\": {\"campo\": valor}}`"
                .to_owned(),
        )),
    }
}

// ---------------------------------------------------------------------------------------------
// Combinadores (Rust nativo)
// ---------------------------------------------------------------------------------------------

/// `igual`: texto canônico do campo igual ao literal, sem espaços nas pontas.
///
/// Booleano aceita `sim`/`nao` e também `true`/`false`/`1`/`0` (§5). Campo ausente ou de tipo sem
/// texto único (lista) é falso — e por consequência `diferente` é verdadeiro.
fn igual(valor: Option<&Json>, literal: &str) -> bool {
    let Some(valor) = valor.filter(|v| !v.is_null()) else {
        return false;
    };
    if let Json::Bool(b) = valor {
        return normaliza_booleano(literal) == Some(*b);
    }
    match texto_canonico(valor) {
        Some(texto) => texto == literal.trim(),
        None => false,
    }
}

/// `maior`/`menor`: comparação numérica.
///
/// Valor numérico compara direto; texto que é número compara pelo número (`"12"` > `"9"`); texto
/// que não é número, lista, objeto ou booleano dá falso — é o `FLX60` avisado na publicação.
/// Literal que não é número é [`DomainError::RegraInvalida`]: a regra nunca poderia ser verdadeira.
fn comparar(
    valor: Option<&Json>,
    literal: &str,
    campo: &str,
    esperado: std::cmp::Ordering,
) -> Result<bool, ErroRegra> {
    let limite = literal.trim().parse::<f64>().map_err(|_| {
        invalida(format!(
            "a regra compara `{campo}` com `{literal}`, que não é número"
        ))
    })?;
    if !limite.is_finite() {
        return Err(invalida(format!(
            "a regra compara `{campo}` com `{literal}`, que não é número"
        )));
    }
    let Some(valor) = valor.filter(|v| !v.is_null()) else {
        return Ok(false);
    };
    let Some(numero) = numero_de(valor) else {
        return Ok(false);
    };
    Ok(numero.partial_cmp(&limite) == Some(esperado))
}

/// `contem`: o campo contém o trecho, sem diferenciar maiúscula nem acento.
///
/// Em lista, é verdadeiro quando **algum item** contém o trecho. Campo ausente é falso.
fn contem(valor: Option<&Json>, trecho: &str) -> bool {
    let Some(valor) = valor.filter(|v| !v.is_null()) else {
        return false;
    };
    let alvo = dobra(trecho.trim());
    if let Json::Array(itens) = valor {
        return itens
            .iter()
            .any(|item| texto_canonico(item).is_some_and(|texto| dobra(&texto).contains(&alvo)));
    }
    texto_canonico(valor).is_some_and(|texto| dobra(&texto).contains(&alvo))
}

/// Se um valor de campo conta como **preenchido** (`FLUXOS_LINGUAGEM.md` §5): `null`, texto só com
/// espaços e lista vazia não contam.
///
/// É a mesma régua do combinador `vazio`, exposta para o motor (FX-13) conferir `exige` da ação e
/// `obrigatorio` do campo sem uma segunda definição que pudesse divergir.
pub fn preenchido(valor: Option<&Json>) -> bool {
    !vazio(valor)
}

/// `vazio`: não preenchido, `null`, só espaços ou lista sem itens.
fn vazio(valor: Option<&Json>) -> bool {
    match valor {
        None | Some(Json::Null) => true,
        Some(Json::String(texto)) => texto.trim().is_empty(),
        Some(Json::Array(itens)) => itens.is_empty(),
        Some(_) => false,
    }
}

/// Texto canônico de um valor de campo (`FLUXOS_LINGUAGEM.md` §5).
///
/// `None` quando o valor não tem texto único — lista (use `contem`) ou `null`. É a mesma forma que
/// a variável de efeito usa (`{etapa.campo}`), por isso o motor a lê daqui.
pub(crate) fn texto_canonico(valor: &Json) -> Option<String> {
    match valor {
        Json::Null => None,
        Json::String(texto) => Some(texto.trim().to_owned()),
        Json::Bool(b) => Some(if *b { "sim" } else { "nao" }.to_owned()),
        Json::Number(numero) => Some(numero_canonico(numero)),
        Json::Array(_) => None,
        // Anexo é objeto `{nome, tipo, tamanho}`: o texto é o nome do arquivo (§5).
        Json::Object(mapa) => match mapa.get("nome") {
            Some(Json::String(nome)) => Some(nome.trim().to_owned()),
            _ => Some(Json::Object(mapa.clone()).to_string()),
        },
    }
}

/// Número em texto canônico: `123`, `12.5` (ponto decimal, sem zero à direita).
fn numero_canonico(numero: &serde_json::Number) -> String {
    if let Some(inteiro) = numero.as_i64() {
        return inteiro.to_string();
    }
    let real = numero.as_f64().unwrap_or_default();
    if real == real.trunc() && real.abs() < 1e15 {
        return format!("{}", real as i64);
    }
    let texto = format!("{real}");
    if texto.contains('.') {
        texto.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        texto
    }
}

/// Valor numérico de um campo: número direto, ou texto que é número.
fn numero_de(valor: &Json) -> Option<f64> {
    match valor {
        Json::Number(numero) => numero.as_f64(),
        Json::String(texto) => texto.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Normaliza o literal comparado com um booleano (§5): `sim`/`true`/`1` e `nao`/`false`/`0`.
fn normaliza_booleano(literal: &str) -> Option<bool> {
    match literal.trim().to_lowercase().as_str() {
        "sim" | "true" | "1" => Some(true),
        "nao" | "não" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// Minúsculas e sem acento, para `contem` (busca em fala de cliente).
fn dobra(texto: &str) -> String {
    texto
        .chars()
        .flat_map(|c| {
            let base = match c {
                'á' | 'à' | 'â' | 'ã' | 'ä' | 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'a',
                'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
                'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
                'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
                'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
                'ç' | 'Ç' => 'c',
                'ñ' | 'Ñ' => 'n',
                outro => outro,
            };
            base.to_lowercase()
        })
        .collect()
}

/// O valor de `etapa.campo` nos dados, ou `None` quando ainda não foi preenchido.
fn valor_do_campo<'a>(dados: &'a Dados, campo: &str) -> Result<Option<&'a Json>, ErroRegra> {
    let (etapa, nome) = campo.split_once('.').ok_or_else(|| {
        invalida(format!(
            "a regra cita `{campo}`; o campo precisa ser escrito como `etapa.campo`"
        ))
    })?;
    if etapa.trim().is_empty() || nome.trim().is_empty() {
        return Err(invalida(format!(
            "a regra cita `{campo}`; o campo precisa ser escrito como `etapa.campo`"
        )));
    }
    Ok(dados
        .get(etapa)
        .and_then(|da_etapa| da_etapa.as_object())
        .and_then(|da_etapa| da_etapa.get(nome)))
}

// ---------------------------------------------------------------------------------------------
// A `def` no Starlark
// ---------------------------------------------------------------------------------------------

/// O que a thread da regra devolve.
enum SaidaFuncao {
    /// A `def` decidiu.
    Booleano(bool),
    /// Devolveu texto, número, `None`…: uma regra precisa decidir sim ou não.
    NaoBooleano,
    /// A `def` lançou erro (chave ausente, tipo inesperado, divisão por zero).
    Falha(String),
    /// Passou de ticks, heap, profundidade de chamada ou prazo.
    Limite(String),
}

/// Roda a `def` num heap temporário próprio, com os limites da regra ligados.
fn executar_funcao(
    modulo: &FrozenModule,
    nome: &str,
    dados: &Dados,
    limites: &LimitesRegra,
    cancelado: Arc<AtomicBool>,
    prazo: Duration,
) -> SaidaFuncao {
    // A `def` é resolvida no módulo congelado **antes** de abrir o heap temporário: o valor
    // pertence ao módulo, não ao heap da chamada.
    let funcao = match modulo.get(nome) {
        Ok(funcao) => funcao,
        Err(_) => {
            return SaidaFuncao::Falha(format!(
                "não existe `def {nome}(dados)` no script do fluxo"
            ));
        }
    };
    let resultado = Module::with_temp_heap(|module| -> Result<Option<bool>, SaidaFuncao> {
        let mut avaliador = Evaluator::new(&module);
        avaliador
            .set_max_tick_count(limites.max_ticks)
            .map_err(|erro| SaidaFuncao::Falha(format!("limite de ticks inválido: {erro}")))?;
        avaliador
            .set_max_heap_size(limites.max_heap_bytes)
            .map_err(|erro| SaidaFuncao::Falha(format!("limite de heap inválido: {erro}")))?;
        avaliador
            .set_max_callstack_size(limites.max_callstack)
            .map_err(|erro| SaidaFuncao::Falha(format!("limite de pilha inválido: {erro}")))?;

        let inicio = Instant::now();
        let flag = Arc::clone(&cancelado);
        avaliador.set_check_cancelled(Box::new(move || {
            if flag.load(Ordering::Relaxed) {
                return true;
            }
            let passou = inicio.elapsed() > prazo;
            if passou {
                flag.store(true, Ordering::Relaxed);
            }
            passou
        }));

        let valor = module.heap().access_owned_frozen_value(&funcao);
        let bruto = Json::Object(dados.clone());
        let argumento = valor_starlark(module.heap(), &bruto, 0)?;
        let saida = avaliador
            .eval_function(valor, &[argumento], &[])
            .map_err(classificar)?;
        Ok(saida.unpack_bool())
    });

    match resultado {
        Ok(Some(valor)) => SaidaFuncao::Booleano(valor),
        Ok(None) => SaidaFuncao::NaoBooleano,
        Err(saida) => saida,
    }
}

/// Erro do Starlark → erro de regra: limite estourado é `RegraLimite`, o resto é `RegraInvalida`.
fn classificar(erro: starlark::Error) -> SaidaFuncao {
    let texto = primeira_linha(&erro.to_string());
    if matches!(erro.kind(), starlark::ErrorKind::StackOverflow(_)) {
        return SaidaFuncao::Limite(format!(
            "passou do limite de {MAX_CALLSTACK_REGRA} chamadas aninhadas (recursão?)"
        ));
    }
    if eh_limite(&texto) {
        return SaidaFuncao::Limite(texto);
    }
    SaidaFuncao::Falha(texto)
}

/// Converte um valor JSON em valor Starlark: o dado do cliente entra como **valor**, nunca código.
fn valor_starlark<'v>(
    heap: starlark::values::Heap<'v>,
    valor: &Json,
    profundidade: usize,
) -> Result<StarValue<'v>, SaidaFuncao> {
    if profundidade > MAX_PROFUNDIDADE {
        return Err(SaidaFuncao::Falha(format!(
            "os dados da execução têm mais de {MAX_PROFUNDIDADE} níveis de aninhamento"
        )));
    }
    Ok(match valor {
        Json::Null => StarValue::new_none(),
        Json::Bool(b) => heap.alloc(*b),
        Json::Number(numero) => match numero.as_i64() {
            Some(inteiro) => heap.alloc(inteiro),
            None => heap.alloc(numero.as_f64().unwrap_or_default()),
        },
        Json::String(texto) => heap.alloc(texto.as_str()),
        Json::Array(itens) => {
            let mut valores = Vec::with_capacity(itens.len());
            for item in itens {
                valores.push(valor_starlark(heap, item, profundidade + 1)?);
            }
            heap.alloc(AllocList(valores))
        }
        Json::Object(mapa) => {
            let mut pares = Vec::with_capacity(mapa.len());
            for (chave, item) in mapa {
                pares.push((
                    heap.alloc(chave.as_str()),
                    valor_starlark(heap, item, profundidade + 1)?,
                ));
            }
            heap.alloc(AllocDict(pares))
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Apoio
// ---------------------------------------------------------------------------------------------

/// Erro de regra por defeito do fluxo (`regra_invalida`).
fn invalida(motivo: String) -> ErroRegra {
    DomainError::InvalidRule { reason: motivo }
}

/// Erro de regra por limite estourado (`regra_limite`).
fn limite(motivo: String) -> ErroRegra {
    DomainError::RuleLimit { reason: motivo }
}

/// Primeira linha de uma mensagem de erro, sem o rastro de chamadas do Starlark.
fn primeira_linha(texto: &str) -> String {
    texto.lines().next().unwrap_or(texto).trim().to_owned()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn dados(json: &str) -> Dados {
        dados_de_json(json).expect("dados de teste válidos")
    }

    #[test]
    fn texto_canonico_segue_a_tabela_do_documento() {
        assert_eq!(
            texto_canonico(&Json::String("  oi  ".into())).as_deref(),
            Some("oi")
        );
        assert_eq!(texto_canonico(&Json::Bool(true)).as_deref(), Some("sim"));
        assert_eq!(texto_canonico(&Json::Bool(false)).as_deref(), Some("nao"));
        assert_eq!(
            texto_canonico(&serde_json::json!(12)).as_deref(),
            Some("12")
        );
        assert_eq!(
            texto_canonico(&serde_json::json!(12.5)).as_deref(),
            Some("12.5")
        );
        assert_eq!(
            texto_canonico(&serde_json::json!(12.0)).as_deref(),
            Some("12")
        );
        assert_eq!(texto_canonico(&Json::Null), None);
        assert_eq!(texto_canonico(&serde_json::json!([1, 2])), None);
        assert_eq!(
            texto_canonico(&serde_json::json!({"nome": "foto.jpg", "tipo": "image/jpeg"}))
                .as_deref(),
            Some("foto.jpg")
        );
    }

    #[test]
    fn dobra_tira_acento_e_maiuscula() {
        assert_eq!(dobra("Não ÁBRe"), "nao abre");
        assert_eq!(dobra("Logística"), "logistica");
    }

    #[test]
    fn numero_canonico_nao_deixa_zero_a_direita() {
        assert_eq!(
            numero_canonico(&serde_json::json!(10).as_number().unwrap().clone()),
            "10"
        );
        assert_eq!(
            numero_canonico(&serde_json::json!(10.50).as_number().unwrap().clone()),
            "10.5"
        );
    }

    #[test]
    fn campo_fora_do_formato_etapa_campo_e_regra_invalida() {
        // Sem ponto (`periodo`), com etapa vazia (`.periodo`) e com campo vazio (`relato.`): os
        // três são defeito do autor, não "campo ausente" — avaliar como ausente esconderia o erro
        // (o `FLX12` do compilador já recusa a forma, mas a IR pode vir de fora).
        for campo in ["periodo", ".periodo", "relato."] {
            let erro = avaliar(
                &Rule::Vazio {
                    campo: campo.into(),
                },
                &dados("{}"),
            )
            .expect_err("campo fora do formato `etapa.campo` tem de ser recusado");
            assert_eq!(erro.api_code(), "regra_invalida", "campo {campo}");
            assert!(erro.to_string().contains("etapa.campo"), "{campo}: {erro}");
        }
    }

    #[test]
    fn dados_de_json_recusa_o_que_nao_e_objeto() {
        assert!(dados_de_json("[]").is_err());
        assert!(dados_de_json("nao é json").is_err());
        assert!(dados_de_json("{\"etapa\": {\"campo\": \"x\"}}").is_ok());
    }
}
