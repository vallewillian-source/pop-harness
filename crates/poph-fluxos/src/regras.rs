use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Map, Value as Json};
use starlark::environment::{FrozenModule, Module};
use starlark::values::Value as StarValue;
use starlark::values::dict::AllocDict;
use starlark::values::list::AllocList;

use poph_types::DomainError;
use poph_types::config::FlowsConfig;

use crate::compilador::Limites;
use crate::ir::Rule;
use crate::language::{StarlarkLimits, load_module};

pub type RuleError = DomainError;

pub type Dados = Map<String, Json>;

pub const MAX_TICKS_REGRA: u64 = 10_000;
pub const MAX_HEAP_BYTES: usize = 1024 * 1024;
pub const MAX_CALL_DEPTH: usize = 128;
pub const RULE_DEADLINE: Duration = Duration::from_millis(200);
pub const DEADLINE_TOLERANCE: Duration = Duration::from_millis(25);
pub const MAX_NESTING: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleLimits {
    pub max_ticks: u64,
    pub max_heap_bytes: usize,
    pub max_callstack: usize,
    pub deadline: Duration,
}

impl Default for RuleLimits {
    fn default() -> Self {
        Self {
            max_ticks: MAX_TICKS_REGRA,
            max_heap_bytes: MAX_HEAP_BYTES,
            max_callstack: MAX_CALL_DEPTH,
            deadline: RULE_DEADLINE,
        }
    }
}

impl RuleLimits {
    pub fn from_config(fluxos: &FlowsConfig) -> Self {
        Self {
            max_ticks: fluxos.max_rule_ticks,
            max_heap_bytes: usize::try_from(fluxos.max_rule_heap).unwrap_or(usize::MAX),
            ..Self::default()
        }
    }
}

#[derive(Clone)]
pub struct RuleEvaluator {
    module: Option<Arc<FrozenModule>>,
    limites: RuleLimits,
}

impl std::fmt::Debug for RuleEvaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avaliador")
            .field("modulo", &self.module.is_some())
            .field("limites", &self.limites)
            .finish()
    }
}

impl RuleEvaluator {
    pub fn without_functions() -> Self {
        Self {
            module: None,
            limites: RuleLimits::default(),
        }
    }

    pub fn new(fonte: &str) -> Result<Self, RuleError> {
        Self::new_with(fonte, &Limites::default(), RuleLimits::default())
    }

    pub fn new_with(fonte: &str, load: &Limites, limites: RuleLimits) -> Result<Self, RuleError> {
        let starlark = StarlarkLimits {
            max_ticks: load.max_ticks,
            max_heap_bytes: load.max_heap_bytes,
            max_callstack: load.max_callstack,
            deadline: load.prazo_de_carga,
        };
        let module = load_with_deadline(fonte, starlark)?;
        Ok(Self {
            module: Some(Arc::new(module)),
            limites,
        })
    }

    pub fn limites(&self) -> &RuleLimits {
        &self.limites
    }

    pub fn evaluate(&self, rule: &Rule, dados: &Dados) -> Result<bool, RuleError> {
        self.evaluate_node(rule, dados, 0)
    }

    fn evaluate_node(&self, rule: &Rule, dados: &Dados, depth: usize) -> Result<bool, RuleError> {
        if depth > MAX_NESTING {
            return Err(invalid(format!(
                "a regra tem mais de {MAX_NESTING} níveis de aninhamento"
            )));
        }
        match rule {
            Rule::Igual { campo, value } => Ok(igual(field_value(dados, campo)?, value)),
            Rule::Diferente { campo, value } => Ok(!igual(field_value(dados, campo)?, value)),
            Rule::Maior { campo, value } => compare(
                field_value(dados, campo)?,
                value,
                campo,
                std::cmp::Ordering::Greater,
            ),
            Rule::Menor { campo, value } => compare(
                field_value(dados, campo)?,
                value,
                campo,
                std::cmp::Ordering::Less,
            ),
            Rule::Contem { campo, value } => Ok(contem(field_value(dados, campo)?, value)),
            Rule::Vazio { campo } => Ok(vazio(field_value(dados, campo)?)),
            Rule::E { args } => {
                for arg in args {
                    if !self.evaluate_node(arg, dados, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Rule::Ou { args } => {
                for arg in args {
                    if self.evaluate_node(arg, dados, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Rule::Nao { args } => match args.as_slice() {
                [single] => Ok(!self.evaluate_node(single, dados, depth + 1)?),
                _ => Err(invalid(format!(
                    "`nao` recebe exatamente 1 condição; a regra tem {}",
                    args.len()
                ))),
            },
            Rule::Funcao { nome } => self.evaluate_function(nome, dados),
        }
    }

    fn evaluate_function(&self, nome: &str, dados: &Dados) -> Result<bool, RuleError> {
        let module = self.module.clone().ok_or_else(|| {
            invalid(format!(
                "a regra chama `funcao(\"{nome}\")`, mas este avaliador não carregou o script: \
                 use `Avaliador::novo`"
            ))
        })?;
        let limites = self.limites.clone();
        let deadline = limites.deadline;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (output_tx, output_rx) = mpsc::channel::<FunctionOutput>();

        let thread_name = nome.to_owned();
        let module_name = thread_name.clone();
        let dados = dados.clone();
        let flag = Arc::clone(&cancelled);
        let thread_limits = limites.clone();
        let created = std::thread::Builder::new()
            .name(format!("regra-{nome}"))
            .spawn(move || {
                let saida = run_function(
                    &module,
                    &module_name,
                    &dados,
                    &thread_limits,
                    flag,
                    deadline,
                );
                let _ = output_tx.send(saida);
            });
        if created.is_err() {
            return Err(limit(format!(
                "não foi possível abrir a thread da função `{nome}`"
            )));
        }

        match output_rx.recv_timeout(deadline + DEADLINE_TOLERANCE) {
            Ok(FunctionOutput::Boolean(value)) => Ok(value),
            Ok(FunctionOutput::NotBoolean) => Err(invalid(format!(
                "a função `{nome}` devolveu algo que não é `True`/`False`; uma regra precisa \
                 decidir sim ou não"
            ))),
            Ok(FunctionOutput::Failure(motivo)) => {
                Err(invalid(format!("a função `{nome}` falhou: {motivo}")))
            }
            Ok(FunctionOutput::Limit(motivo)) => Err(limit(motivo)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(limit(format!(
                "a função `{nome}` passou do prazo de {} ms",
                deadline.as_millis()
            ))),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(invalid(format!(
                "a função `{nome}` terminou sem devolver resposta"
            ))),
        }
    }
}

fn load_with_deadline(fonte: &str, limites: StarlarkLimits) -> Result<FrozenModule, RuleError> {
    let deadline = limites.deadline;
    let (sent, recepcao) = mpsc::channel();
    let fonte = fonte.to_owned();
    std::thread::Builder::new()
        .name("fluxo-carga".to_owned())
        .spawn(move || {
            let _ = sent.send(load_module(&fonte, "fluxo", &limites));
        })
        .map_err(|error| limit(format!("não foi possível abrir a thread da carga: {error}")))?;

    match recepcao.recv_timeout(deadline + DEADLINE_TOLERANCE) {
        Ok(Ok(module)) => Ok(module),
        Ok(Err(error)) => Err(classify_load(&error)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(limit(format!(
            "o script não carregou em {} ms e a carga foi abandonada; cálculo constante enorme \
             escrito direto no script (por exemplo `\"x\" * 1000000000` dentro de uma `def`) é \
             feito na carga e não pode ser interrompido",
            deadline.as_millis()
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(invalid(
            "a carga do script terminou sem responder".to_owned(),
        )),
    }
}

fn classify_load(error: &starlark::Error) -> RuleError {
    let texto = first_line(&error.to_string());
    if is_limit(&texto) || matches!(error.kind(), starlark::ErrorKind::StackOverflow(_)) {
        return limit(format!(
            "o script do fluxo passou dos limites da carga: {texto}"
        ));
    }
    invalid(format!("o script do fluxo não carregou: {texto}"))
}

fn is_limit(texto: &str) -> bool {
    texto.contains("Heap memory limit")
        || texto.contains("ticks has been exceeded")
        || texto.contains("Evaluation cancelled")
        || texto.contains("call stack")
        || texto.contains("Starlark call stack")
}

pub fn evaluate(rule: &Rule, dados: &Dados) -> Result<bool, RuleError> {
    RuleEvaluator::without_functions().evaluate(rule, dados)
}

pub fn snapshot_from_json(texto: &str) -> Result<Dados, RuleError> {
    let raw: Json = serde_json::from_str(texto)
        .map_err(|error| invalid(format!("os dados da execução não são JSON: {error}")))?;
    match raw {
        Json::Object(map) => Ok(map),
        _ => Err(invalid(
            "os dados da execução precisam ser um objeto `{\"etapa\": {\"campo\": valor}}`"
                .to_owned(),
        )),
    }
}

fn igual(value: Option<&Json>, literal: &str) -> bool {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return false;
    };
    if let Json::Bool(b) = value {
        return normalize_boolean(literal) == Some(*b);
    }
    match canonical_text(value) {
        Some(texto) => texto == literal.trim(),
        None => false,
    }
}

fn compare(
    value: Option<&Json>,
    literal: &str,
    campo: &str,
    expected: std::cmp::Ordering,
) -> Result<bool, RuleError> {
    let limit = literal.trim().parse::<f64>().map_err(|_| {
        invalid(format!(
            "a regra compara `{campo}` com `{literal}`, que não é número"
        ))
    })?;
    if !limit.is_finite() {
        return Err(invalid(format!(
            "a regra compara `{campo}` com `{literal}`, que não é número"
        )));
    }
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(false);
    };
    let Some(numero) = number_of(value) else {
        return Ok(false);
    };
    Ok(numero.partial_cmp(&limit) == Some(expected))
}

fn contem(value: Option<&Json>, excerpt: &str) -> bool {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return false;
    };
    let target = dobra(excerpt.trim());
    if let Json::Array(items) = value {
        return items
            .iter()
            .any(|item| canonical_text(item).is_some_and(|texto| dobra(&texto).contains(&target)));
    }
    canonical_text(value).is_some_and(|texto| dobra(&texto).contains(&target))
}

pub fn preenchido(value: Option<&Json>) -> bool {
    !vazio(value)
}

fn vazio(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => true,
        Some(Json::String(texto)) => texto.trim().is_empty(),
        Some(Json::Array(items)) => items.is_empty(),
        Some(_) => false,
    }
}

pub(crate) fn canonical_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::String(texto) => Some(texto.trim().to_owned()),
        Json::Bool(b) => Some(if *b { "sim" } else { "nao" }.to_owned()),
        Json::Number(numero) => Some(canonical_number(numero)),
        Json::Array(_) => None,
        Json::Object(map) => match map.get("nome") {
            Some(Json::String(nome)) => Some(nome.trim().to_owned()),
            _ => Some(Json::Object(map.clone()).to_string()),
        },
    }
}

fn canonical_number(numero: &serde_json::Number) -> String {
    if let Some(integer) = numero.as_i64() {
        return integer.to_string();
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

fn number_of(value: &Json) -> Option<f64> {
    match value {
        Json::Number(numero) => numero.as_f64(),
        Json::String(texto) => texto.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn normalize_boolean(literal: &str) -> Option<bool> {
    match literal.trim().to_lowercase().as_str() {
        "sim" | "true" | "1" => Some(true),
        "nao" | "não" | "false" | "0" => Some(false),
        _ => None,
    }
}

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
                other => other,
            };
            base.to_lowercase()
        })
        .collect()
}

fn field_value<'a>(dados: &'a Dados, campo: &str) -> Result<Option<&'a Json>, RuleError> {
    let (etapa, nome) = campo.split_once('.').ok_or_else(|| {
        invalid(format!(
            "a regra cita `{campo}`; o campo precisa ser escrito como `etapa.campo`"
        ))
    })?;
    if etapa.trim().is_empty() || nome.trim().is_empty() {
        return Err(invalid(format!(
            "a regra cita `{campo}`; o campo precisa ser escrito como `etapa.campo`"
        )));
    }
    Ok(dados
        .get(etapa)
        .and_then(|of_step| of_step.as_object())
        .and_then(|of_step| of_step.get(nome)))
}

enum FunctionOutput {
    Boolean(bool),
    NotBoolean,
    Failure(String),
    Limit(String),
}

fn run_function(
    module: &FrozenModule,
    nome: &str,
    dados: &Dados,
    limites: &RuleLimits,
    cancelled: Arc<AtomicBool>,
    deadline: Duration,
) -> FunctionOutput {
    let funcao = match module.get(nome) {
        Ok(funcao) => funcao,
        Err(_) => {
            return FunctionOutput::Failure(format!(
                "não existe `def {nome}(dados)` no script do fluxo"
            ));
        }
    };
    let resultado = Module::with_temp_heap(|module| -> Result<Option<bool>, FunctionOutput> {
        let mut evaluator = starlark::eval::Evaluator::new(&module);
        evaluator
            .set_max_tick_count(limites.max_ticks)
            .map_err(|error| {
                FunctionOutput::Failure(format!("limite de ticks inválido: {error}"))
            })?;
        evaluator
            .set_max_heap_size(limites.max_heap_bytes)
            .map_err(|error| {
                FunctionOutput::Failure(format!("limite de heap inválido: {error}"))
            })?;
        evaluator
            .set_max_callstack_size(limites.max_callstack)
            .map_err(|error| {
                FunctionOutput::Failure(format!("limite de pilha inválido: {error}"))
            })?;

        let start = Instant::now();
        let flag = Arc::clone(&cancelled);
        evaluator.set_check_cancelled(Box::new(move || {
            if flag.load(Ordering::Relaxed) {
                return true;
            }
            let elapsed = start.elapsed() > deadline;
            if elapsed {
                flag.store(true, Ordering::Relaxed);
            }
            elapsed
        }));

        let value = module.heap().access_owned_frozen_value(&funcao);
        let raw = Json::Object(dados.clone());
        let argument = starlark_value(module.heap(), &raw, 0)?;
        let saida = evaluator
            .eval_function(value, &[argument], &[])
            .map_err(classify)?;
        Ok(saida.unpack_bool())
    });

    match resultado {
        Ok(Some(value)) => FunctionOutput::Boolean(value),
        Ok(None) => FunctionOutput::NotBoolean,
        Err(saida) => saida,
    }
}

fn classify(error: starlark::Error) -> FunctionOutput {
    let texto = first_line(&error.to_string());
    if matches!(error.kind(), starlark::ErrorKind::StackOverflow(_)) {
        return FunctionOutput::Limit(format!(
            "passou do limite de {MAX_CALL_DEPTH} chamadas aninhadas (recursão?)"
        ));
    }
    if is_limit(&texto) {
        return FunctionOutput::Limit(texto);
    }
    FunctionOutput::Failure(texto)
}

fn starlark_value<'v>(
    heap: starlark::values::Heap<'v>,
    value: &Json,
    depth: usize,
) -> Result<StarValue<'v>, FunctionOutput> {
    if depth > MAX_NESTING {
        return Err(FunctionOutput::Failure(format!(
            "os dados da execução têm mais de {MAX_NESTING} níveis de aninhamento"
        )));
    }
    Ok(match value {
        Json::Null => StarValue::new_none(),
        Json::Bool(b) => heap.alloc(*b),
        Json::Number(numero) => match numero.as_i64() {
            Some(integer) => heap.alloc(integer),
            None => heap.alloc(numero.as_f64().unwrap_or_default()),
        },
        Json::String(texto) => heap.alloc(texto.as_str()),
        Json::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(starlark_value(heap, item, depth + 1)?);
            }
            heap.alloc(AllocList(values))
        }
        Json::Object(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (chave, item) in map {
                pairs.push((
                    heap.alloc(chave.as_str()),
                    starlark_value(heap, item, depth + 1)?,
                ));
            }
            heap.alloc(AllocDict(pairs))
        }
    })
}

fn invalid(motivo: String) -> RuleError {
    DomainError::InvalidRule { reason: motivo }
}

fn limit(motivo: String) -> RuleError {
    DomainError::RuleLimit { reason: motivo }
}

fn first_line(texto: &str) -> String {
    texto.lines().next().unwrap_or(texto).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dados(json: &str) -> Dados {
        snapshot_from_json(json).expect("dados de teste válidos")
    }

    #[test]
    fn canonical_text_follows_the_documented_table() {
        assert_eq!(
            canonical_text(&Json::String("  oi  ".into())).as_deref(),
            Some("oi")
        );
        assert_eq!(canonical_text(&Json::Bool(true)).as_deref(), Some("sim"));
        assert_eq!(canonical_text(&Json::Bool(false)).as_deref(), Some("nao"));
        assert_eq!(
            canonical_text(&serde_json::json!(12)).as_deref(),
            Some("12")
        );
        assert_eq!(
            canonical_text(&serde_json::json!(12.5)).as_deref(),
            Some("12.5")
        );
        assert_eq!(
            canonical_text(&serde_json::json!(12.0)).as_deref(),
            Some("12")
        );
        assert_eq!(canonical_text(&Json::Null), None);
        assert_eq!(canonical_text(&serde_json::json!([1, 2])), None);
        assert_eq!(
            canonical_text(&serde_json::json!({"nome": "foto.jpg", "tipo": "image/jpeg"}))
                .as_deref(),
            Some("foto.jpg")
        );
    }

    #[test]
    fn fold_strips_accents_and_case() {
        assert_eq!(dobra("Não ÁBRe"), "nao abre");
        assert_eq!(dobra("Logística"), "logistica");
    }

    #[test]
    fn canonical_number_keeps_no_trailing_zero() {
        assert_eq!(
            canonical_number(&serde_json::json!(10).as_number().unwrap().clone()),
            "10"
        );
        assert_eq!(
            canonical_number(&serde_json::json!(10.50).as_number().unwrap().clone()),
            "10.5"
        );
    }

    #[test]
    fn field_outside_format_is_invalid_rule() {
        for campo in ["periodo", ".periodo", "relato."] {
            let error = evaluate(
                &Rule::Vazio {
                    campo: campo.into(),
                },
                &dados("{}"),
            )
            .expect_err("campo fora do formato `etapa.campo` tem de ser recusado");
            assert_eq!(error.api_code(), "regra_invalida", "campo {campo}");
            assert!(
                error.to_string().contains("etapa.campo"),
                "{campo}: {error}"
            );
        }
    }

    #[test]
    fn snapshot_from_json_rejects_non_object() {
        assert!(snapshot_from_json("[]").is_err());
        assert!(snapshot_from_json("nao é json").is_err());
        assert!(snapshot_from_json("{\"etapa\": {\"campo\": \"x\"}}").is_ok());
    }
}
