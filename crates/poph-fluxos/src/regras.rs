//! Regras: combinadores declarativos (`igual`, `diferente`, `maior`, `menor`, `contem`, `vazio`,
//! `e`, `ou`, `nao`) e funções avançadas (`def`) guardadas como texto e recompiladas na carga.
//!
//! Implementado na FX-12. Os combinadores são avaliados em Rust, sem passar pelo Starlark; só a
//! `def` roda no Starlark, em tempo de execução. Meta de latência: 20 µs p95 por regra, medida em
//! 1,7 µs no spike da FX-01.
