use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "desconhecido".to_owned());
    println!("cargo:rustc-env=POPH_RUSTC_VERSAO={version}");
    println!("cargo:rerun-if-changed=build.rs");
}
