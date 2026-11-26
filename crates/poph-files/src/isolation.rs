pub fn limit_memory(bytes: u64) -> Result<(), String> {
    rlimit::Resource::CORE.set(0, 0).map_err(|error| {
        format!(
            "não consegui desligar o núcleo de memória do leitor ({})",
            error.kind()
        )
    })?;
    rlimit::Resource::AS.set(bytes, bytes).map_err(|error| {
        format!(
            "não consegui limitar a memória do leitor ({})",
            error.kind()
        )
    })
}
