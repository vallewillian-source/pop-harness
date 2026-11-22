//! Medida de memória do processo, usada para registrar quanto o modelo ocupa na RAM.

/// Memória residente do processo (VmRSS), em bytes. `None` quando não dá para ler
/// (fora do Linux, ou `/proc` indisponível).
pub fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let linha = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
    let kb: u64 = linha.split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}
