//! RAM do processo com o cache de revisões carregado (FX-36).
//!
//! Arquivo próprio de propósito, no mesmo padrão de `poph-rag --test ram`: cada arquivo de teste de
//! integração roda num processo, e o RSS só significa algo se o processo não carregou outra coisa
//! antes. Mede a meta da FX-01 (`≤ 300 KB` por fluxo de 40 nós; aqui um fluxo de 8 etapas) e o
//! comportamento do cache no teto (`fluxos.cache_revisoes_mb`, padrão 16 MB).
//!
//! Rodar assim (a medição é material de relatório, não de regressão):
//!
//! ```text
//! POPH_RAM_TETO_MB=16 cargo test --release -p poph-fluxos --test ram -- --ignored --nocapture
//! ```
//!
//! O teto vem de `POPH_RAM_TETO_MB` (padrão 16) e cada valor roda num processo próprio: o alocador
//! não devolve ao sistema operacional o que uma fase liberou, então medir dois tetos no mesmo
//! processo mediria a sobra da primeira fase. Para o relatório da FX-36 o teste rodou duas vezes,
//! com 4 (o cache enche e expulsa) e com 16 (as 1.000 revisões cabem).
//!
//! O que sai impresso: RSS com **0**, **100** e **1.000** revisões carregadas, o peso estimado por
//! entrada ([`peso_da_ir`]), o que o cache diz ter guardado (entradas, expulsões, bytes) e o custo
//! real por revisão guardada.

use poph_fluxos::{Ir, Limites, RevisionCache, RevisionKey, compile, entry_weight};

/// Teto padrão de `fluxos.cache_revisoes_mb` (PRD §9).
const TETO_PADRAO_MB: usize = 16;

/// Quantas revisões distintas a medição carrega.
const REVISOES: usize = 1_000;

/// Páginas residentes do processo, em KB (`/proc/self/statm`).
fn rss_kb() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").expect("/proc/self/statm");
    let paginas: u64 = statm
        .split_whitespace()
        .nth(1)
        .expect("segundo campo do statm")
        .parse()
        .expect("páginas residentes");
    paginas * 4
}

/// Script válido com `etapas` etapas em fila; `enchimento` caracteres por instrução dão à IR um
/// tamanho previsível (o mesmo gerador do teste `cache.rs`, para o peso ser comparável).
fn script(fluxo: &str, etapas: usize, enchimento: usize) -> String {
    let recheio = "x".repeat(enchimento.max(1));
    let mut texto = format!(
        "fluxo(\n    id = \"{fluxo}\",\n    nome = \"Fluxo {fluxo}\",\n    inicio = \"e0\",\n    gatilho = gatilho(\n        quando = \"O cliente quer testar o fluxo.\",\n        exemplos = [\"quero testar\"],\n    ),\n)\n\n"
    );
    for indice in 0..etapas {
        let destino = if indice + 1 < etapas {
            format!("\"e{}\"", indice + 1)
        } else {
            "fim(\"pronto\", resultado = \"Feito\")".to_owned()
        };
        texto.push_str(&format!(
            "etapa(\n    id = \"e{indice}\",\n    nome = \"Etapa {indice}\",\n    instrucao = \"{recheio}\",\n    acoes = [acao(\"seguir\", \"Seguir\", vai_para = {destino})],\n)\n\n"
        ));
    }
    texto
}

fn ir_do_script(fonte: &str) -> Ir {
    compile(fonte, &Limites::default())
        .ir()
        .cloned()
        .unwrap_or_else(|diagnosticos| panic!("script do teste não compila: {diagnosticos:?}"))
}

/// RAM com 0, 100 e 1.000 revisões carregadas, no teto de `POPH_RAM_TETO_MB`.
#[test]
#[ignore = "mede RSS real (/proc/self/statm): rodar à mão com --ignored --nocapture"]
fn ram_com_0_100_e_1000_revisoes_carregadas() {
    let teto_mb = std::env::var("POPH_RAM_TETO_MB")
        .ok()
        .and_then(|texto| texto.trim().parse().ok())
        .unwrap_or(TETO_PADRAO_MB);

    // O primeiro script compila antes da medida: o trabalho de carga da biblioteca não entra na
    // conta das revisões guardadas.
    let primeira = script("pedido0", 8, 200);
    let peso = entry_weight(&ir_do_script(&primeira));

    let cache = RevisionCache::new(teto_mb);
    let sem_revisao = rss_kb();

    let mut rss_100 = sem_revisao;
    for numero in 1..=REVISOES {
        let fonte = script(&format!("pedido{numero}"), 8, 200);
        let ir = ir_do_script(&fonte);
        let chave = RevisionKey::new(format!("fluxo{numero}"), 1, &fonte);
        cache
            .load(&chave, &ir, &fonte, &Limites::default())
            .unwrap_or_else(|falha| panic!("revisão {numero} não carregou: {falha:?}"));
        if numero == 100 {
            rss_100 = rss_kb();
        }
    }
    let com_mil = rss_kb();
    let metricas = cache.metrics();

    println!(
        "teto {teto_mb} MB · peso estimado {peso} B por revisão · {REVISOES} revisões carregadas, \
         {} guardadas, {} expulsas, {} não cacheadas, {} B guardados",
        metricas.entries, metricas.evictions, metricas.not_cached, metricas.bytes
    );
    println!("RSS com 0 revisões: {sem_revisao} KB");
    println!(
        "RSS com 100 revisões: {rss_100} KB (+{} KB, {:.0} B por revisão)",
        rss_100.saturating_sub(sem_revisao),
        rss_100.saturating_sub(sem_revisao) as f64 * 1024.0 / 100.0
    );
    println!(
        "RSS com {REVISOES} revisões: {com_mil} KB (+{} KB, {:.0} B por revisão)",
        com_mil.saturating_sub(sem_revisao),
        com_mil.saturating_sub(sem_revisao) as f64 * 1024.0 / REVISOES as f64
    );

    // O teto é uma promessa do cache: o que está guardado nunca passa dele.
    assert!(
        metricas.bytes <= teto_mb * 1024 * 1024,
        "o cache diz ter {} B guardados e o teto é {teto_mb} MB",
        metricas.bytes
    );
    // A meta da FX-01 é por fluxo carregado: 300 KB.
    assert!(
        peso < 300 * 1024,
        "a revisão de 8 etapas pesa {peso} B e a meta da FX-01 é 300 KB"
    );
}
