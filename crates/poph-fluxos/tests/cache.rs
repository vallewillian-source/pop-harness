//! Testes do cache de revisões compiladas (FX-19).
//!
//! Cada item do "pronto quando" do bloco FX-19 tem o seu teste aqui, e cada um tem de cair quando
//! o comportamento é revertido (as mutações estão no relatório de entrega). As revisões dos testes
//! são **scripts de verdade**, compilados pelo compilador da FX-10: é o mesmo caminho que a
//! execução usa, e a fonte é gerada para o teste poder fixar o tamanho da IR (e, com ele, o peso no
//! teto).
//!
//! O único teste que sai da suíte normal é `ram_por_revisao_medida` (`#[ignore]`): ele mede RSS de
//! verdade em `/proc/self/statm` e é material de medição, não de regressão.

use std::sync::{Arc, Barrier};
use std::thread;

use poph_fluxos::{
    CacheRevisoes, ChaveRevisao, FalhaCarregamento, Ir, Limites, compilar, peso_da_ir,
};

// ------------------------------------------------------------------ auxiliares

fn limites() -> Limites {
    Limites::default()
}

/// Script válido com `etapas` etapas em fila; cada instrução leva `enchimento` caracteres, que é o
/// que dá tamanho previsível à IR (e ao peso da entrada no cache).
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

/// Compila o script do teste e devolve a IR gravada, como a publicação a guardaria.
fn ir_do_script(fonte: &str) -> Ir {
    let compilado = compilar(fonte, &limites());
    compilado
        .ir()
        .unwrap_or_else(|diagnosticos| panic!("script do teste não compila: {diagnosticos:?}"))
        .clone()
}

/// Carrega uma revisão no cache pelo caminho normal e devolve a IR (falha do teste se não carregar).
fn carregar(cache: &CacheRevisoes, chave: &ChaveRevisao, ir: &Ir, fonte: &str) -> Arc<Ir> {
    cache
        .carregar(chave, ir, fonte, &limites())
        .unwrap_or_else(|falha| panic!("carga de {} falhou: {falha:?}", chave.fluxo_id()))
}

// ------------------------------------------------------------------ acerto e chave

#[test]
fn acerto_no_segundo_acesso_compila_uma_vez_so() {
    let fonte = script("pedido", 3, 80);
    let ir = ir_do_script(&fonte);
    let chave = ChaveRevisao::nova("pedido", 1, &fonte);
    let cache = CacheRevisoes::novo(1);

    let primeira = carregar(&cache, &chave, &ir, &fonte);
    let segunda = carregar(&cache, &chave, &ir, &fonte);

    assert_eq!(*primeira, ir, "a IR devolvida é a gravada");
    assert!(
        Arc::ptr_eq(&primeira, &segunda),
        "o segundo acesso devolve a mesma IR em memória, não uma cópia"
    );
    let metricas = cache.metricas();
    assert_eq!(metricas.cargas, 1, "uma compilação para duas consultas");
    assert_eq!(metricas.erros, 1, "só a primeira consulta foi erro");
    assert_eq!(metricas.acertos, 1);
    assert_eq!(metricas.esperas, 0);
    assert_eq!(metricas.entradas, 1);
    assert_eq!(metricas.bytes, peso_da_ir(&ir));
}

#[test]
fn a_chave_separa_fluxo_numero_e_texto() {
    let fonte = script("pedido", 2, 40);
    let ir = ir_do_script(&fonte);
    let outra_fonte = script("pedido", 2, 41);
    let outra_ir = ir_do_script(&outra_fonte);
    let cache = CacheRevisoes::novo(1);

    let base = ChaveRevisao::nova("pedido", 1, &fonte);
    let por_numero = ChaveRevisao::nova("pedido", 2, &fonte);
    let por_fluxo = ChaveRevisao::nova("outro_pedido", 1, &fonte);
    let por_hash = ChaveRevisao::com_hash("pedido", 1, "0000");
    let outro_texto = ChaveRevisao::nova("pedido", 1, &outra_fonte);

    assert_ne!(base, por_numero);
    assert_ne!(base, por_fluxo);
    assert_ne!(base, por_hash);
    assert_ne!(base, outro_texto);
    assert_eq!(
        base,
        ChaveRevisao::nova("pedido", 1, &fonte),
        "a chave é estável"
    );

    for chave in [&base, &por_numero, &por_fluxo, &por_hash] {
        carregar(&cache, chave, &ir, &fonte);
    }
    carregar(&cache, &outro_texto, &outra_ir, &outra_fonte);

    let metricas = cache.metricas();
    assert_eq!(metricas.entradas, 5, "cinco identidades, cinco entradas");
    assert_eq!(metricas.cargas, 5);
}

// ------------------------------------------------------------------ teto e expulsão

#[test]
fn o_teto_e_respeitado_com_mais_fluxos_do_que_cabem() {
    let fonte = script("pedido", 4, 200);
    let ir = ir_do_script(&fonte);
    let peso = peso_da_ir(&ir);
    // Espaço para três revisões do tamanho do exemplo, com sobra de meio peso.
    let teto = peso * 3 + peso / 2;
    let cache = CacheRevisoes::com_teto_bytes(teto);

    let mut carregadas = 0;
    for numero in 1..=12u32 {
        let chave = ChaveRevisao::nova("pedido", numero, &fonte);
        carregar(&cache, &chave, &ir, &fonte);
        carregadas += 1;
        let metricas = cache.metricas();
        assert!(
            metricas.bytes <= teto,
            "depois de {carregadas} revisões o cache está com {} bytes e o teto é {teto}",
            metricas.bytes
        );
    }

    let metricas = cache.metricas();
    assert_eq!(metricas.cargas, 12);
    assert_eq!(
        metricas.entradas, 3,
        "só três revisões cabem no teto de {teto} bytes (peso {peso})"
    );
    assert_eq!(metricas.expulsoes, 9, "as outras nove saíram");
    assert!(metricas.bytes <= teto);
}

#[test]
fn o_menos_usado_sai_primeiro() {
    let fonte_a = script("pedido_a", 3, 60);
    let fonte_b = script("pedido_b", 3, 60);
    let fonte_c = script("pedido_c", 3, 60);
    let ir_a = ir_do_script(&fonte_a);
    let ir_b = ir_do_script(&fonte_b);
    let ir_c = ir_do_script(&fonte_c);
    let chave_a = ChaveRevisao::nova("pedido_a", 1, &fonte_a);
    let chave_b = ChaveRevisao::nova("pedido_b", 1, &fonte_b);
    let chave_c = ChaveRevisao::nova("pedido_c", 1, &fonte_c);
    let teto = peso_da_ir(&ir_a) + peso_da_ir(&ir_b) + peso_da_ir(&ir_c) / 2;
    let cache = CacheRevisoes::com_teto_bytes(teto);

    carregar(&cache, &chave_a, &ir_a, &fonte_a);
    carregar(&cache, &chave_b, &ir_b, &fonte_b);
    // `a` volta a ser usada: agora a menos recente é a `b`.
    assert!(cache.obter(&chave_a).is_some(), "a está no cache");

    carregar(&cache, &chave_c, &ir_c, &fonte_c);

    let metricas = cache.metricas();
    assert_eq!(metricas.expulsoes, 1);
    assert!(cache.contem(&chave_a), "a mais usada ficou");
    assert!(!cache.contem(&chave_b), "a menos usada saiu");
    assert!(cache.contem(&chave_c), "a recém-carregada entrou");
    assert!(metricas.bytes <= teto);
}

#[test]
fn a_revisao_em_uso_nao_e_expulsa() {
    let fonte_a = script("pedido_a", 3, 60);
    let fonte_b = script("pedido_b", 3, 60);
    let fonte_c = script("pedido_c", 3, 60);
    let ir_a = ir_do_script(&fonte_a);
    let ir_b = ir_do_script(&fonte_b);
    let ir_c = ir_do_script(&fonte_c);
    let chave_a = ChaveRevisao::nova("pedido_a", 1, &fonte_a);
    let chave_b = ChaveRevisao::nova("pedido_b", 1, &fonte_b);
    let chave_c = ChaveRevisao::nova("pedido_c", 1, &fonte_c);
    let teto = peso_da_ir(&ir_a) + peso_da_ir(&ir_b) + peso_da_ir(&ir_c) / 2;
    let cache = CacheRevisoes::com_teto_bytes(teto);

    // Duas revisões carregadas e as duas em uso (cada uma com um `Arc` na mão).
    let em_uso_a = carregar(&cache, &chave_a, &ir_a, &fonte_a);
    let em_uso_b = carregar(&cache, &chave_b, &ir_b, &fonte_b);

    // A terceira não cabe e não há o que expulsar: entra sem cache.
    let terceira = carregar(&cache, &chave_c, &ir_c, &fonte_c);
    assert_eq!(*terceira, ir_c, "a execução recebe a IR do mesmo jeito");

    let metricas = cache.metricas();
    assert!(cache.contem(&chave_a) && cache.contem(&chave_b));
    assert!(!cache.contem(&chave_c), "c não entra com o teto ocupado");
    assert_eq!(metricas.nao_cacheadas, 1);
    assert_eq!(
        metricas.expulsoes, 0,
        "nada é expulso para não guardar nada no lugar"
    );
    assert_eq!(metricas.em_uso, 2);
    assert!(metricas.bytes <= teto);

    // Soltando a `b`, o espaço volta a existir e a expulsão escolhe a menos recente — poupando a
    // `a`, que continua em uso.
    drop(em_uso_b);
    carregar(&cache, &chave_c, &ir_c, &fonte_c);
    assert!(cache.contem(&chave_c));
    assert!(!cache.contem(&chave_b), "b, agora livre, saiu");
    assert!(cache.contem(&chave_a), "a continua em uso");
    assert_eq!(*em_uso_a, ir_a);
    let metricas = cache.metricas();
    assert_eq!(metricas.expulsoes, 1);
    assert!(
        metricas.em_uso_poupadas >= 1,
        "a entrada em uso foi poupada da expulsão"
    );
    assert!(metricas.bytes <= teto);
}

#[test]
fn revisao_maior_que_o_teto_inteiro_nao_e_cacheada() {
    let fonte = script("pedido", 6, 400);
    let ir = ir_do_script(&fonte);
    let peso = peso_da_ir(&ir);
    let cache = CacheRevisoes::com_teto_bytes(peso / 2);
    let chave = ChaveRevisao::nova("pedido", 1, &fonte);

    let devolvida = carregar(&cache, &chave, &ir, &fonte);

    assert_eq!(*devolvida, ir);
    let metricas = cache.metricas();
    assert_eq!(metricas.entradas, 0);
    assert_eq!(metricas.nao_cacheadas, 1);
    assert_eq!(metricas.bytes, 0);
}

#[test]
fn limpar_esvazia_o_cache_sem_perder_as_metricas() {
    let fonte = script("pedido", 2, 40);
    let ir = ir_do_script(&fonte);
    let cache = CacheRevisoes::novo(1);
    let chave_1 = ChaveRevisao::nova("pedido", 1, &fonte);
    let chave_2 = ChaveRevisao::nova("pedido", 2, &fonte);
    carregar(&cache, &chave_1, &ir, &fonte);
    carregar(&cache, &chave_2, &ir, &fonte);

    assert_eq!(cache.limpar(), 2);
    let metricas = cache.metricas();
    assert_eq!(metricas.entradas, 0);
    assert_eq!(metricas.bytes, 0);
    assert_eq!(metricas.cargas, 2, "o histórico de cargas continua");
    assert!(!cache.contem(&chave_1));

    carregar(&cache, &chave_1, &ir, &fonte);
    assert_eq!(
        cache.metricas().cargas,
        3,
        "depois de limpar, carrega de novo"
    );
}

// ------------------------------------------------------------------ concorrência

#[test]
fn carga_concorrente_da_mesma_revisao_compila_uma_vez_so() {
    const THREADS: usize = 8;
    const RODADAS: usize = 20;
    // Script maior de propósito: a compilação leva alguns milissegundos, que é a janela em que as
    // outras threads encontram a carga em curso.
    let fonte = script("pedido", 20, 120);
    let ir = Arc::new(ir_do_script(&fonte));
    let mut esperas = 0u64;

    for rodada in 0..RODADAS {
        let cache = CacheRevisoes::novo(4);
        let chave = ChaveRevisao::nova("pedido", rodada as u32 + 1, &fonte);
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
                    .carregar(&chave, ir.as_ref(), &fonte, &Limites::default())
                    .map(|revisao| revisao.as_ref() == ir.as_ref())
            }));
        }
        for mao in maos {
            let resultado = mao.join().expect("thread do teste");
            match resultado {
                Ok(igual) => assert!(igual, "rodada {rodada}: IR diferente da gravada"),
                Err(falha) => panic!("rodada {rodada}: carga falhou: {falha:?}"),
            }
        }

        let metricas = cache.metricas();
        assert_eq!(
            metricas.cargas, 1,
            "rodada {rodada}: uma compilação por revisão"
        );
        assert_eq!(metricas.entradas, 1, "rodada {rodada}");
        assert_eq!(
            metricas.acertos + metricas.esperas,
            (THREADS - 1) as u64,
            "rodada {rodada}: as outras sete foram servidas pela carga da primeira"
        );
        esperas += metricas.esperas;
    }

    println!(
        "{RODADAS} rodadas de {THREADS} threads: {RODADAS} compilações, {esperas} esperas pelo lote"
    );
    assert!(
        esperas >= 1,
        "nenhuma chamada esperou a carga da outra em {RODADAS} rodadas: o lote não foi exercitado"
    );
}

#[test]
fn o_cache_e_compartilhado_entre_threads_e_por_clone() {
    fn exige<T: Send + Sync + 'static>() {}
    exige::<CacheRevisoes>();

    let fonte = script("pedido", 2, 40);
    let ir = ir_do_script(&fonte);
    let chave = ChaveRevisao::nova("pedido", 1, &fonte);
    let original = CacheRevisoes::novo(1);
    let clone = original.clone();

    let da_thread = {
        let cache = clone.clone();
        let chave = chave.clone();
        let ir = ir.clone();
        let fonte = fonte.clone();
        thread::spawn(move || carregar(&cache, &chave, &ir, &fonte))
    }
    .join()
    .expect("thread do teste");
    drop(da_thread);

    assert!(
        original.contem(&chave),
        "o clone e o original são o mesmo cache"
    );
    assert_eq!(clone.metricas().cargas, 1);
}

// ------------------------------------------------------------------ falha fechada

#[test]
fn falha_de_carga_e_tipada_e_o_cache_continua_usavel() {
    let fonte = script("pedido", 3, 60);
    let ir = ir_do_script(&fonte);
    let cache = CacheRevisoes::novo(1);

    // 1. Texto que compila, mas não gera a IR gravada: `ir_divergente`.
    let adulterada = fonte.replace("Etapa 1", "Etapa Um");
    assert_ne!(adulterada, fonte);
    let chave_adulterada = ChaveRevisao::nova("pedido", 2, &adulterada);
    let falha = cache
        .carregar(&chave_adulterada, &ir, &adulterada, &limites())
        .expect_err("a IR gravada não corresponde ao texto adulterado");
    assert!(
        matches!(falha, FalhaCarregamento::Divergente { .. }),
        "{falha:?}"
    );
    assert_eq!(falha.codigo(), "ir_divergente");
    assert!(
        !cache.contem(&chave_adulterada),
        "o que diverge não entra no cache"
    );

    // 2. Texto que não compila: `fluxo_invalido`.
    let quebrada = "fluxo(id = \"pedido\",\n";
    let chave_quebrada = ChaveRevisao::nova("pedido", 3, quebrada);
    let falha = cache
        .carregar(&chave_quebrada, &ir, quebrada, &limites())
        .expect_err("texto quebrado não carrega");
    assert!(matches!(falha, FalhaCarregamento::Forma(_)), "{falha:?}");
    assert_eq!(falha.codigo(), "fluxo_invalido");
    assert!(!cache.contem(&chave_quebrada));

    // 3. O cache continua utilizável depois das duas falhas.
    let chave = ChaveRevisao::nova("pedido", 1, &fonte);
    let carregada = carregar(&cache, &chave, &ir, &fonte);
    assert_eq!(*carregada, ir);
    let metricas = cache.metricas();
    assert_eq!(metricas.cargas, 3, "três cargas tentadas");
    assert_eq!(metricas.entradas, 1, "só a que deu certo ficou");
    assert_eq!(carregar(&cache, &chave, &ir, &fonte).as_ref(), &ir);
    assert_eq!(cache.metricas().acertos, 1);
}

// ------------------------------------------------------------------ memória

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

/// RAM por revisão carregada, medida no processo (meta proposta na FX-01: ≤ 300 KB por fluxo).
///
/// Fica fora da suíte normal porque depende de `/proc` e do estado de memória da máquina: é
/// medição para o relatório, não regressão.
#[test]
#[ignore = "mede RSS real (/proc/self/statm): rodar à mão com --ignored"]
fn ram_por_revisao_medida() {
    const REVISOES: usize = 200;
    let fonte = script("pedido", 8, 200);
    let ir = ir_do_script(&fonte);
    let peso = peso_da_ir(&ir);

    // Aquecimento: o primeiro clone aloca o que é alocação única (nada, mas garante página tocada).
    let aquecimento = Arc::new(ir.clone());
    let antes = rss_kb();
    let mut guardadas = Vec::with_capacity(REVISOES);
    for _ in 0..REVISOES {
        guardadas.push(Arc::new(ir.clone()));
    }
    let depois = rss_kb();
    let cresceu_kb = depois.saturating_sub(antes);
    let por_revisao = cresceu_kb * 1024 / REVISOES as u64;
    println!(
        "RSS: {antes} KB → {depois} KB com {REVISOES} revisões (+{cresceu_kb} KB); \
         por revisão ≈ {por_revisao} B; peso estimado {peso} B; fator ≈ {:.2}",
        por_revisao as f64 / peso as f64
    );

    assert!(guardadas.len() == REVISOES && !guardadas.is_empty());
    drop(aquecimento);
    drop(guardadas);

    // O teto usa o peso estimado; aqui a medida real precisa caber na meta da FX-01.
    assert!(
        por_revisao < 300 * 1024,
        "a revisão de 8 etapas ocupou {por_revisao} B e a meta da FX-01 é 300 KB"
    );
}
