use std::sync::{Arc, Barrier};
use std::thread;

use poph_fluxos::{Ir, Limites, LoadFailure, RevisionCache, RevisionKey, compile, entry_weight};

fn limites() -> Limites {
    Limites::default()
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
    let compiled = compile(fonte, &limites());
    compiled
        .ir()
        .unwrap_or_else(|diagnostics| panic!("script do teste não compila: {diagnostics:?}"))
        .clone()
}

fn load(cache: &RevisionCache, chave: &RevisionKey, ir: &Ir, fonte: &str) -> Arc<Ir> {
    cache
        .load(chave, ir, fonte, &limites())
        .unwrap_or_else(|failure| panic!("carga de {} falhou: {failure:?}", chave.fluxo_id()))
}

#[test]
fn hit_second_access_compiles_time_only() {
    let fonte = script("pedido", 3, 80);
    let ir = ir_do_script(&fonte);
    let chave = RevisionKey::new("pedido", 1, &fonte);
    let cache = RevisionCache::new(1);

    let first_pass = load(&cache, &chave, &ir, &fonte);
    let second_pass = load(&cache, &chave, &ir, &fonte);

    assert_eq!(*first_pass, ir, "a IR devolvida é a gravada");
    assert!(
        Arc::ptr_eq(&first_pass, &second_pass),
        "o segundo acesso devolve a mesma IR em memória, não uma cópia"
    );
    let metrics = cache.metrics();
    assert_eq!(metrics.loads, 1, "uma compilação para duas consultas");
    assert_eq!(metrics.misses, 1, "só a primeira consulta foi erro");
    assert_eq!(metrics.hits, 1);
    assert_eq!(metrics.waits, 0);
    assert_eq!(metrics.entries, 1);
    assert_eq!(metrics.bytes, entry_weight(&ir));
}

#[test]
fn key_separates_flow_number_and_text() {
    let fonte = script("pedido", 2, 40);
    let ir = ir_do_script(&fonte);
    let other_source = script("pedido", 2, 41);
    let other_ir = ir_do_script(&other_source);
    let cache = RevisionCache::new(1);

    let base = RevisionKey::new("pedido", 1, &fonte);
    let by_number = RevisionKey::new("pedido", 2, &fonte);
    let by_flow = RevisionKey::new("outro_pedido", 1, &fonte);
    let by_hash = RevisionKey::with_hash("pedido", 1, "0000");
    let other_text = RevisionKey::new("pedido", 1, &other_source);

    assert_ne!(base, by_number);
    assert_ne!(base, by_flow);
    assert_ne!(base, by_hash);
    assert_ne!(base, other_text);
    assert_eq!(
        base,
        RevisionKey::new("pedido", 1, &fonte),
        "a chave é estável"
    );

    for chave in [&base, &by_number, &by_flow, &by_hash] {
        load(&cache, chave, &ir, &fonte);
    }
    load(&cache, &other_text, &other_ir, &other_source);

    let metrics = cache.metrics();
    assert_eq!(metrics.entries, 5, "cinco identidades, cinco entradas");
    assert_eq!(metrics.loads, 5);
}

#[test]
fn cap_respected_with_more_flows_than_fit() {
    let fonte = script("pedido", 4, 200);
    let ir = ir_do_script(&fonte);
    let weight = entry_weight(&ir);
    let cap = weight * 3 + weight / 2;
    let cache = RevisionCache::with_byte_cap(cap);

    let mut loaded_count = 0;
    for numero in 1..=12u32 {
        let chave = RevisionKey::new("pedido", numero, &fonte);
        load(&cache, &chave, &ir, &fonte);
        loaded_count += 1;
        let metrics = cache.metrics();
        assert!(
            metrics.bytes <= cap,
            "depois de {loaded_count} revisões o cache está com {} bytes e o teto é {cap}",
            metrics.bytes
        );
    }

    let metrics = cache.metrics();
    assert_eq!(metrics.loads, 12);
    assert_eq!(
        metrics.entries, 3,
        "só três revisões cabem no teto de {cap} bytes (peso {weight})"
    );
    assert_eq!(metrics.evictions, 9, "as outras nove saíram");
    assert!(metrics.bytes <= cap);
}

#[test]
fn least_used_leaves_first() {
    let source_a = script("pedido_a", 3, 60);
    let source_b = script("pedido_b", 3, 60);
    let source_c = script("pedido_c", 3, 60);
    let ir_a = ir_do_script(&source_a);
    let ir_b = ir_do_script(&source_b);
    let ir_c = ir_do_script(&source_c);
    let key_a = RevisionKey::new("pedido_a", 1, &source_a);
    let key_b = RevisionKey::new("pedido_b", 1, &source_b);
    let key_c = RevisionKey::new("pedido_c", 1, &source_c);
    let cap = entry_weight(&ir_a) + entry_weight(&ir_b) + entry_weight(&ir_c) / 2;
    let cache = RevisionCache::with_byte_cap(cap);

    load(&cache, &key_a, &ir_a, &source_a);
    load(&cache, &key_b, &ir_b, &source_b);
    assert!(cache.get(&key_a).is_some(), "a está no cache");

    load(&cache, &key_c, &ir_c, &source_c);

    let metrics = cache.metrics();
    assert_eq!(metrics.evictions, 1);
    assert!(cache.contem(&key_a), "a mais usada ficou");
    assert!(!cache.contem(&key_b), "a menos usada saiu");
    assert!(cache.contem(&key_c), "a recém-carregada entrou");
    assert!(metrics.bytes <= cap);
}

#[test]
fn revision_in_use_is_not_evicted() {
    let source_a = script("pedido_a", 3, 60);
    let source_b = script("pedido_b", 3, 60);
    let source_c = script("pedido_c", 3, 60);
    let ir_a = ir_do_script(&source_a);
    let ir_b = ir_do_script(&source_b);
    let ir_c = ir_do_script(&source_c);
    let key_a = RevisionKey::new("pedido_a", 1, &source_a);
    let key_b = RevisionKey::new("pedido_b", 1, &source_b);
    let key_c = RevisionKey::new("pedido_c", 1, &source_c);
    let cap = entry_weight(&ir_a) + entry_weight(&ir_b) + entry_weight(&ir_c) / 2;
    let cache = RevisionCache::with_byte_cap(cap);

    let in_use_a = load(&cache, &key_a, &ir_a, &source_a);
    let in_use_b = load(&cache, &key_b, &ir_b, &source_b);

    let third = load(&cache, &key_c, &ir_c, &source_c);
    assert_eq!(*third, ir_c, "a execução recebe a IR do mesmo jeito");

    let metrics = cache.metrics();
    assert!(cache.contem(&key_a) && cache.contem(&key_b));
    assert!(!cache.contem(&key_c), "c não entra com o teto ocupado");
    assert_eq!(metrics.not_cached, 1);
    assert_eq!(
        metrics.evictions, 0,
        "nada é expulso para não guardar nada no lugar"
    );
    assert_eq!(metrics.in_use, 2);
    assert!(metrics.bytes <= cap);

    drop(in_use_b);
    load(&cache, &key_c, &ir_c, &source_c);
    assert!(cache.contem(&key_c));
    assert!(!cache.contem(&key_b), "b, agora livre, saiu");
    assert!(cache.contem(&key_a), "a continua em uso");
    assert_eq!(*in_use_a, ir_a);
    let metrics = cache.metrics();
    assert_eq!(metrics.evictions, 1);
    assert!(
        metrics.in_use_spared >= 1,
        "a entrada em uso foi poupada da expulsão"
    );
    assert!(metrics.bytes <= cap);
}

#[test]
fn revision_larger_than_whole_cap_is_not_cached() {
    let fonte = script("pedido", 6, 400);
    let ir = ir_do_script(&fonte);
    let weight = entry_weight(&ir);
    let cache = RevisionCache::with_byte_cap(weight / 2);
    let chave = RevisionKey::new("pedido", 1, &fonte);

    let devolvida = load(&cache, &chave, &ir, &fonte);

    assert_eq!(*devolvida, ir);
    let metrics = cache.metrics();
    assert_eq!(metrics.entries, 0);
    assert_eq!(metrics.not_cached, 1);
    assert_eq!(metrics.bytes, 0);
}

#[test]
fn clear_empties_cache_without_losing_metrics() {
    let fonte = script("pedido", 2, 40);
    let ir = ir_do_script(&fonte);
    let cache = RevisionCache::new(1);
    let key_1 = RevisionKey::new("pedido", 1, &fonte);
    let key_2 = RevisionKey::new("pedido", 2, &fonte);
    load(&cache, &key_1, &ir, &fonte);
    load(&cache, &key_2, &ir, &fonte);

    assert_eq!(cache.clear(), 2);
    let metrics = cache.metrics();
    assert_eq!(metrics.entries, 0);
    assert_eq!(metrics.bytes, 0);
    assert_eq!(metrics.loads, 2, "o histórico de cargas continua");
    assert!(!cache.contem(&key_1));

    load(&cache, &key_1, &ir, &fonte);
    assert_eq!(
        cache.metrics().loads,
        3,
        "depois de limpar, carrega de novo"
    );
}

#[test]
fn load_concurrent_same_revision_compiles_time_only() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 20;
    let fonte = script("pedido", 20, 120);
    let ir = Arc::new(ir_do_script(&fonte));
    let mut waits = 0u64;

    for round in 0..ROUNDS {
        let cache = RevisionCache::new(4);
        let chave = RevisionKey::new("pedido", round as u32 + 1, &fonte);
        let barreira = Arc::new(Barrier::new(THREADS));
        let mut maos = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            let cache = cache.clone();
            let barreira = Arc::clone(&barreira);
            let chave = chave.clone();
            let ir = Arc::clone(&ir);
            let fonte = fonte.clone();
            maos.push(thread::spawn(move || {
                barreira.wait();
                cache
                    .load(&chave, ir.as_ref(), &fonte, &Limites::default())
                    .map(|revision| revision.as_ref() == ir.as_ref())
            }));
        }
        for mao in maos {
            let resultado = mao.join().expect("thread do teste");
            match resultado {
                Ok(igual) => assert!(igual, "rodada {round}: IR diferente da gravada"),
                Err(failure) => panic!("rodada {round}: carga falhou: {failure:?}"),
            }
        }

        let metrics = cache.metrics();
        assert_eq!(
            metrics.loads, 1,
            "rodada {round}: uma compilação por revisão"
        );
        assert_eq!(metrics.entries, 1, "rodada {round}");
        assert_eq!(
            metrics.hits + metrics.waits,
            (THREADS - 1) as u64,
            "rodada {round}: as outras sete foram servidas pela carga da primeira"
        );
        waits += metrics.waits;
    }

    println!(
        "{ROUNDS} rodadas de {THREADS} threads: {ROUNDS} compilações, {waits} esperas pelo lote"
    );
    assert!(
        waits >= 1,
        "nenhuma chamada esperou a carga da outra em {ROUNDS} rodadas: o lote não foi exercitado"
    );
}

#[test]
fn cache_is_shared_between_threads_and_by_clone() {
    fn requires<T: Send + Sync + 'static>() {}
    requires::<RevisionCache>();

    let fonte = script("pedido", 2, 40);
    let ir = ir_do_script(&fonte);
    let chave = RevisionKey::new("pedido", 1, &fonte);
    let original = RevisionCache::new(1);
    let clone = original.clone();

    let of_thread = {
        let cache = clone.clone();
        let chave = chave.clone();
        let ir = ir.clone();
        let fonte = fonte.clone();
        thread::spawn(move || load(&cache, &chave, &ir, &fonte))
    }
    .join()
    .expect("thread do teste");
    drop(of_thread);

    assert!(
        original.contem(&chave),
        "o clone e o original são o mesmo cache"
    );
    assert_eq!(clone.metrics().loads, 1);
}

#[test]
fn load_failure_is_typed_and_cache_remains_usable() {
    let fonte = script("pedido", 3, 60);
    let ir = ir_do_script(&fonte);
    let cache = RevisionCache::new(1);

    let adulterada = fonte.replace("Etapa 1", "Etapa Um");
    assert_ne!(adulterada, fonte);
    let key_tampered = RevisionKey::new("pedido", 2, &adulterada);
    let failure = cache
        .load(&key_tampered, &ir, &adulterada, &limites())
        .expect_err("a IR gravada não corresponde ao texto adulterado");
    assert!(
        matches!(failure, LoadFailure::Divergent { .. }),
        "{failure:?}"
    );
    assert_eq!(failure.code(), "ir_divergente");
    assert!(
        !cache.contem(&key_tampered),
        "o que diverge não entra no cache"
    );

    let quebrada = "fluxo(id = \"pedido\",\n";
    let key_broken = RevisionKey::new("pedido", 3, quebrada);
    let failure = cache
        .load(&key_broken, &ir, quebrada, &limites())
        .expect_err("texto quebrado não carrega");
    assert!(matches!(failure, LoadFailure::Form(_)), "{failure:?}");
    assert_eq!(failure.code(), "fluxo_invalido");
    assert!(!cache.contem(&key_broken));

    let chave = RevisionKey::new("pedido", 1, &fonte);
    let loaded = load(&cache, &chave, &ir, &fonte);
    assert_eq!(*loaded, ir);
    let metrics = cache.metrics();
    assert_eq!(metrics.loads, 3, "três cargas tentadas");
    assert_eq!(metrics.entries, 1, "só a que deu certo ficou");
    assert_eq!(load(&cache, &chave, &ir, &fonte).as_ref(), &ir);
    assert_eq!(cache.metrics().hits, 1);
}

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

#[test]
#[ignore = "mede RSS real (/proc/self/statm): rodar à mão com --ignored"]
fn ram_per_revision_measured() {
    const REVISIONS: usize = 200;
    let fonte = script("pedido", 8, 200);
    let ir = ir_do_script(&fonte);
    let weight = entry_weight(&ir);

    let aquecimento = Arc::new(ir.clone());
    let before = rss_kb();
    let mut guarded = Vec::with_capacity(REVISIONS);
    for _ in 0..REVISIONS {
        guarded.push(Arc::new(ir.clone()));
    }
    let after = rss_kb();
    let cresceu_kb = after.saturating_sub(before);
    let by_revision = cresceu_kb * 1024 / REVISIONS as u64;
    println!(
        "RSS: {before} KB → {after} KB com {REVISIONS} revisões (+{cresceu_kb} KB); \
         por revisão ≈ {by_revision} B; peso estimado {weight} B; fator ≈ {:.2}",
        by_revision as f64 / weight as f64
    );

    assert!(guarded.len() == REVISIONS && !guarded.is_empty());
    drop(aquecimento);
    drop(guarded);

    assert!(
        by_revision < 300 * 1024,
        "a revisão de 8 etapas ocupou {by_revision} B e a meta da FX-01 é 300 KB"
    );
}
