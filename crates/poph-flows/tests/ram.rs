use poph_flows::{Ir, Limites, RevisionCache, RevisionKey, compile, entry_weight};

const CAP_DEFAULT_MB: usize = 16;

const REVISIONS: usize = 1_000;

fn rss_kb() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").expect("/proc/self/statm");
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .expect("segundo campo do statm")
        .parse()
        .expect("páginas residentes");
    pages * 4
}

fn script(fluxo: &str, steps: usize, enchimento: usize) -> String {
    let recheio = "x".repeat(enchimento.max(1));
    let mut texto = format!(
        "fluxo(\n    id = \"{fluxo}\",\n    nome = \"Fluxo {fluxo}\",\n    inicio = \"e0\",\n    gatilho = gatilho(\n        quando = \"O cliente quer testar o fluxo.\",\n        exemplos = [\"quero testar\"],\n    ),\n)\n\n"
    );
    for index in 0..steps {
        let target = if index + 1 < steps {
            format!("\"e{}\"", index + 1)
        } else {
            "fim(\"pronto\", resultado = \"Feito\")".to_owned()
        };
        texto.push_str(&format!(
            "etapa(\n    id = \"e{index}\",\n    nome = \"Etapa {index}\",\n    instrucao = \"{recheio}\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = {target})],\n)\n\n"
        ));
    }
    texto
}

fn ir_do_script(fonte: &str) -> Ir {
    compile(fonte, &Limites::default())
        .ir()
        .cloned()
        .unwrap_or_else(|diagnostics| panic!("script do teste não compila: {diagnostics:?}"))
}

#[test]
#[ignore = "mede RSS real (/proc/self/statm): rodar à mão com --ignored --nocapture"]
fn ram_with_0_100_and_1000_revisions_loaded() {
    let cap_mb = std::env::var("POPH_RAM_TETO_MB")
        .ok()
        .and_then(|texto| texto.trim().parse().ok())
        .unwrap_or(CAP_DEFAULT_MB);

    let first_pass = script("pedido0", 8, 200);
    let weight = entry_weight(&ir_do_script(&first_pass));

    let cache = RevisionCache::new(cap_mb);
    let without_revision = rss_kb();

    let mut rss_100 = without_revision;
    for numero in 1..=REVISIONS {
        let fonte = script(&format!("pedido{numero}"), 8, 200);
        let ir = ir_do_script(&fonte);
        let chave = RevisionKey::new(format!("fluxo{numero}"), 1, &fonte);
        cache
            .load(&chave, &ir, &fonte, &Limites::default())
            .unwrap_or_else(|failure| panic!("revisão {numero} não carregou: {failure:?}"));
        if numero == 100 {
            rss_100 = rss_kb();
        }
    }
    let with_thousand = rss_kb();
    let metrics = cache.metrics();

    println!(
        "teto {cap_mb} MB · peso estimado {weight} B por revisão · {REVISIONS} revisões carregadas, \
         {} guardadas, {} expulsas, {} não cacheadas, {} B guardados",
        metrics.entries, metrics.evictions, metrics.not_cached, metrics.bytes
    );
    println!("RSS com 0 revisões: {without_revision} KB");
    println!(
        "RSS com 100 revisões: {rss_100} KB (+{} KB, {:.0} B por revisão)",
        rss_100.saturating_sub(without_revision),
        rss_100.saturating_sub(without_revision) as f64 * 1024.0 / 100.0
    );
    println!(
        "RSS com {REVISIONS} revisões: {with_thousand} KB (+{} KB, {:.0} B por revisão)",
        with_thousand.saturating_sub(without_revision),
        with_thousand.saturating_sub(without_revision) as f64 * 1024.0 / REVISIONS as f64
    );

    assert!(
        metrics.bytes <= cap_mb * 1024 * 1024,
        "o cache diz ter {} B guardados e o teto é {cap_mb} MB",
        metrics.bytes
    );
    assert!(
        weight < 300 * 1024,
        "a revisão de 8 etapas pesa {weight} B e a meta da FX-01 é 300 KB"
    );
}
