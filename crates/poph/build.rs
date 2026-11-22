//! Grava no binário a versão do compilador usado, para `poph doctor` mostrar o toolchain real.
//! Sem dependências: só chama o `rustc` que o cargo já informa.

use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let versao = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|saida| saida.status.success())
        .map(|saida| String::from_utf8_lossy(&saida.stdout).trim().to_owned())
        .unwrap_or_else(|| "desconhecido".to_owned());
    println!("cargo:rustc-env=POPH_RUSTC_VERSAO={versao}");
    println!("cargo:rerun-if-changed=build.rs");
}
