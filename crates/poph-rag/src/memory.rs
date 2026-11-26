pub fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}
