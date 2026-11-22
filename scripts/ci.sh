#!/usr/bin/env bash
# CI local do POP Harness (HR-04). Roda todas as checagens e devolve um código de saída agregado:
# 0 só se todas passarem. Uso: apps/pop-harness/scripts/ci.sh
set -uo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RAIZ" || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

falhas=0

executa() {
    local nome="$1"
    shift
    echo
    echo "==> ${nome}: $*"
    if "$@"; then
        echo "<== OK: ${nome}"
    else
        local rc=$?
        echo "<== FALHOU: ${nome} (código ${rc})"
        falhas=$((falhas + 1))
    fi
}

# Console (HR-38): o bundle embutido no binário e o cliente do OpenAPI. Cada etapa se declara
# pulada quando `console/node_modules` não existe, então o CI offline de um clone limpo continua
# funcionando; a instalação (`pnpm install` em `console/`) é a única etapa que exige rede, e é
# manual de propósito.
#
# HR-60: desde o Cargo 1.100 o build-dir guarda um diretório por unidade (perfil, features
# unificadas, alvo e hashes das dependências) e nunca apaga os antigos; cada binário de teste com
# debuginfo passa de 100 MB. Sem poda, o `target/` cresce a cada forma de comando diferente até
# encher o disco (chegou a 313 GB em 21/11/2026). A poda remove o que não é tocado há 3 dias,
# preserva diretório de build recente e não falha quando há outro cargo rodando na máquina.
executa "cache de compilação (poda do que não é usado há 3 dias)" bash scripts/limpar-cache.sh --dias 3
executa "console (bundle do Console embutido no binário)" bash console/scripts/build.sh
executa "console (cliente do OpenAPI em dia)" bash console/scripts/verificar-api.sh
# Testes de componente do Console (HR-39a). Também se declara pulada sem `console/node_modules`.
executa "console (testes de componente)" bash console/scripts/testar.sh

executa "formatação" cargo fmt --all -- --check
executa "clippy" cargo clippy --workspace --all-targets --locked -- -D warnings
executa "build" cargo build --workspace --locked
executa "testes" cargo test --workspace --locked
executa "ram (medição do modelo, poph-rag sozinho)" cargo test --locked -p poph-rag --test ram -- --ignored --nocapture
# aarch64: sem compilador C cruzado nesta estação (D-02), o `rusqlite` (SQLite em C) não compila
# para ARM. Checam-se os crates sem C; o binário ARM é compilado na máquina ARM64 de testes na HR-56.
# `poph-server` saiu desta lista na HR-20: passou a depender do `poph-store` (SQLite em C), então
# também precisa do compilador cruzado. Ele é coberto na máquina ARM64 de testes, junto com o `poph` e o `poph-store`.
# `poph-core` saiu na HR-10b: passou a usar TLS (reqwest com rustls), e o `aws-lc-sys` é C — quebra
# na estação por falta do compilador cruzado (D-02/D-16). Também é coberto na máquina ARM64 de testes.
# poph-fluxos (FX-04) entra aqui com `blake3` na feature `pure`: sem ela o check ARM pede compilador C.
executa "aarch64 (check, crates sem C)" cargo check --locked --target aarch64-unknown-linux-gnu -p poph-types -p poph-files -p poph-plugins -p poph-fluxos
# Plugin de exemplo (HR-27): só `poph-types` + `serde_json`, então também é código puro para o ARM.
executa "aarch64 (check, plugin de exemplo)" cargo check --locked --target aarch64-unknown-linux-gnu -p poph-plugin-eco
# `poph-jev` saiu do check aarch64 na HR-10b pelo mesmo motivo do `poph-core`: com TLS (rustls via
# reqwest) ele puxa `aws-lc-sys`, que compila C. A verificação ARM dele também é na máquina ARM64 de testes (HR-56).
executa "deny (licenças, bans, fontes, advisories)" cargo deny check
executa "audit (vulnerabilidades)" cargo audit

echo
if [ "$falhas" -eq 0 ]; then
    echo "CI LOCAL: OK (todas as checagens passaram)"
    exit 0
fi
echo "CI LOCAL: FALHOU (${falhas} checagem(ns) com erro)"
exit 1
