//! Cache de revisões compiladas: FX-19.
//!
//! O motor recebe a IR da revisão em que a execução começou, e a revisão publicada é **imutável**:
//! `fluxo_revisoes.definicao_json` nunca muda para a mesma chave. Isso permite guardar a IR já
//! carregada em memória e não recompilar o texto a cada turno — a compilação é a única parte cara
//! do caminho (parse em Starlark), e o resto do motor é barato.
//!
//! ## A chave
//!
//! [`ChaveRevisao`] = `fluxo_id` + `numero` + hash do texto. O número da revisão sozinho não basta
//! (o mesmo número de dois fluxos diferentes existe), e o número com o fluxo também não basta
//! sozinho para provar que a IR guardada corresponde ao texto — o hash é o que fecha isso. A
//! publicação grava o hash em `fluxo_revisoes.fonte_sha256`; [`ChaveRevisao::nova`] calcula o
//! mesmo sha256 a partir do texto, e [`ChaveRevisao::com_hash`] aceita o valor que já está no
//! banco. [`sha256_da_fonte`] expõe o cálculo para quem publica.
//!
//! ## A carga e o lote (single-flight)
//!
//! [`CacheRevisoes::carregar`] é a única porta de entrada: acerto devolve a IR guardada; na falta
//! dela, **uma** chamada carrega (`crate::compilador::carregar`) e as demais que pedem a mesma
//! chave no mesmo instante esperam o resultado dela, em vez de compilar cada uma. A espera tem
//! limite ([`ESPERA_MAXIMA_DO_LOTE`]): se quem carregava sumir (pânico), quem espera carrega
//! sozinho depois do limite, em vez de travar.
//!
//! A falha de carga **não derruba nada**: `ir_divergente` (texto que não gera a IR gravada) ou
//! `fluxo_invalido` (texto que não compila) voltam como [`FalhaCarregamento`], não entram no cache
//! e o processo — e o próprio cache — segue utilizável. É a falha fechada da `FLXR06`: a execução
//! termina em falha, o resto do módulo continua.
//!
//! ## O teto e a expulsão
//!
//! O teto é em bytes ([`CacheRevisoes::com_teto_bytes`]; [`CacheRevisoes::novo`] recebe MB, que é a
//! unidade de `fluxos.cache_revisoes_mb`). Cada entrada pesa [`peso_da_ir`] — o JSON canônico da IR
//! vezes [`FATOR_DO_PESO`] mais [`CUSTO_FIXO_DA_ENTRADA`] —, uma estimativa conservadora do custo
//! real em memória (a medida de RSS está no relatório da tarefa e no teste `#[ignore]` do crate).
//!
//! Quando o teto é alcançado, sai a entrada **menos recentemente usada entre as que não estão em
//! uso**. "Em uso" é concreto, não um sinalizador: quem chamou recebe um `Arc<Ir>` e, enquanto
//! aquele `Arc` existir, a entrada não é escolhida para sair (`Arc::strong_count`). Se todas as
//! entradas estiverem em uso e ainda faltar espaço, a revisão nova **não entra** no cache (conta em
//! `Metricas::nao_cacheadas`) e a chamada devolve a IR do mesmo jeito: cache cheio nunca faz uma
//! execução falhar, só deixa de acelerar.
//!
//! A escolha da vítima varre as entradas (`O(n)`), e não mantém estrutura intrusiva: o teto padrão
//! ([`TETO_PADRAO_MB`] = 16 MB) comporta dezenas de revisões, e a varredura só acontece quando o
//! teto é atingido. Um cache por processo é o que a FX-20 vai usar; a instância é compartilhável
//! entre threads (`Clone` compartilha o mesmo estado, `Send + Sync`) e o `Mutex` é **tolerante a
//! envenenamento**: pânico de uma thread não impede as outras de usar o cache.
//!
//! ## O que este módulo não faz
//!
//! Não lê configuração (a chave `fluxos.cache_revisoes_mb` é da FX-26, que vai construir o cache
//! com o valor do `poph.toml`), não fala com banco nem com relógio (o tempo não entra na decisão de
//! expulsão: a ordem de uso é um contador), e não guarda "para sempre" nada: toda entrada é
//! candidata a sair.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::compilador::{FalhaCarregamento, Limites, carregar as carregar_revisao};
use crate::ir::Ir;

/// Teto padrão do cache em MB, o mesmo de `fluxos.cache_revisoes_mb` (PRD §9).
pub const TETO_PADRAO_MB: usize = 16;

/// Custo fixo atribuído a cada entrada: chave, `Arc`, carimbos e o nó do mapa.
pub const CUSTO_FIXO_DA_ENTRADA: usize = 256;

/// Fator aplicado ao tamanho do JSON da IR para estimar o custo em memória.
///
/// A IR em memória não é o JSON: cada texto é um `String` com capacidade própria e há a estrutura
/// dos `Vec`/`Box`/`Arc`/chave. A medição de RSS da FX-19 (teste `ram_por_revisao_medida`) deu
/// **5.857 a 6.389 bytes** reais por revisão contra 3.666 bytes de JSON — fator **1,60 a 1,74**. O
/// teto usa 2 para errar pelo lado seguro, e é por isso que o cache fica um pouco abaixo do teto em
/// RAM real.
pub const FATOR_DO_PESO: usize = 2;

/// Quanto uma chamada espera pelo carregamento de outra antes de carregar sozinha.
///
/// Um carregamento é de milissegundos; o limite existe só para o caso de quem carregava ter
/// desaparecido (pânico), e é generoso para não competir com a carga real.
pub const ESPERA_MAXIMA_DO_LOTE: Duration = Duration::from_secs(30);

/// O sha256 do texto de uma revisão, em hexadecimal minúsculo (64 caracteres).
///
/// É o mesmo valor de `fluxo_revisoes.fonte_sha256`: quem publica guarda este e o cache usa este,
/// então não há duas contas de hash para a mesma revisão.
pub fn sha256_da_fonte(fonte: &str) -> String {
    let digest = Sha256::digest(fonte.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Identidade de uma revisão no cache: fluxo, número e hash do texto.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChaveRevisao {
    fluxo_id: String,
    numero: u32,
    hash: String,
}

impl ChaveRevisao {
    /// Chave de uma revisão, com o hash calculado do texto ([`sha256_da_fonte`]).
    pub fn nova(fluxo_id: impl Into<String>, numero: u32, fonte: &str) -> Self {
        Self {
            fluxo_id: fluxo_id.into(),
            numero,
            hash: sha256_da_fonte(fonte),
        }
    }

    /// Chave de uma revisão cujo hash já é conhecido (o `fonte_sha256` gravado na publicação).
    pub fn com_hash(fluxo_id: impl Into<String>, numero: u32, hash: impl Into<String>) -> Self {
        Self {
            fluxo_id: fluxo_id.into(),
            numero,
            hash: hash.into(),
        }
    }

    /// Id do fluxo (`fluxos.slug`).
    pub fn fluxo_id(&self) -> &str {
        &self.fluxo_id
    }

    /// Número da revisão dentro do fluxo (a partir de 1, como `fluxo_revisoes.numero`).
    pub fn numero(&self) -> u32 {
        self.numero
    }

    /// sha256 do texto da revisão.
    pub fn hash(&self) -> &str {
        &self.hash
    }
}

/// Quanto uma IR ocupa no teto do cache, em bytes (estimativa conservadora).
///
/// Base: o JSON canônico da IR (`Ir::para_json`, o mesmo que vai ao banco), vezes
/// [`FATOR_DO_PESO`], mais [`CUSTO_FIXO_DA_ENTRADA`].
pub fn peso_da_ir(ir: &Ir) -> usize {
    ir.para_json()
        .len()
        .saturating_mul(FATOR_DO_PESO)
        .saturating_add(CUSTO_FIXO_DA_ENTRADA)
}

/// Retrato das contas do cache: um número por acontecimento, mais o estado de agora.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Metricas {
    /// Consultas servidas pela entrada já carregada.
    pub acertos: u64,
    /// Consultas que não acharam a revisão pronta (= `cargas` + `esperas`).
    pub erros: u64,
    /// Consultas que carregaram a revisão (uma compilação cada).
    pub cargas: u64,
    /// Consultas que esperaram a carga de outra chamada da mesma chave.
    pub esperas: u64,
    /// Entradas que saíram por falta de espaço.
    pub expulsoes: u64,
    /// Entradas em uso que deixaram de ser expulsas (ficaram no cache).
    pub em_uso_poupadas: u64,
    /// Revisões carregadas que não couberam no teto (a execução seguiu sem cache).
    pub nao_cacheadas: u64,
    /// Entradas no cache agora.
    pub entradas: usize,
    /// Entradas em uso agora (alguém segura o `Arc` desta revisão).
    pub em_uso: usize,
    /// Soma do peso das entradas, em bytes.
    pub bytes: usize,
}

/// Estado de uma revisão que está sendo carregada, para as outras chamadas esperarem.
struct EmCurso {
    resultado: Mutex<Option<Result<Arc<Ir>, FalhaCarregamento>>>,
    aviso: Condvar,
}

impl EmCurso {
    fn novo() -> Self {
        Self {
            resultado: Mutex::new(None),
            aviso: Condvar::new(),
        }
    }

    /// Publica o desfecho e acorda quem espera.
    fn publicar(&self, resultado: Result<Arc<Ir>, FalhaCarregamento>) {
        let mut guarda = self.trava();
        *guarda = Some(resultado);
        drop(guarda);
        self.aviso.notify_all();
    }

    /// Espera o desfecho; `None` quando o limite estoura (quem carregava desapareceu).
    fn esperar(&self) -> Option<Result<Arc<Ir>, FalhaCarregamento>> {
        let inicio = Instant::now();
        let mut guarda = self.trava();
        loop {
            if let Some(resultado) = guarda.as_ref() {
                return Some(resultado.clone());
            }
            let restante = ESPERA_MAXIMA_DO_LOTE.checked_sub(inicio.elapsed())?;
            guarda = match self.aviso.wait_timeout(guarda, restante) {
                Ok((novo, _)) => novo,
                Err(envenenado) => envenenado.into_inner().0,
            };
        }
    }

    fn trava(&self) -> MutexGuard<'_, Option<Result<Arc<Ir>, FalhaCarregamento>>> {
        self.resultado
            .lock()
            .unwrap_or_else(|envenenado| envenenado.into_inner())
    }
}

/// Uma revisão guardada.
struct Entrada {
    ir: Arc<Ir>,
    bytes: usize,
    /// Carimbo de uso: quanto maior, mais recente.
    usado_em: u64,
}

#[derive(Default)]
struct Contadores {
    acertos: u64,
    erros: u64,
    cargas: u64,
    esperas: u64,
    expulsoes: u64,
    em_uso_poupadas: u64,
    nao_cacheadas: u64,
}

struct Interno {
    entradas: HashMap<ChaveRevisao, Entrada>,
    /// Chaves que estão sendo carregadas agora, com o ponto de encontro de quem espera.
    em_curso: HashMap<ChaveRevisao, Arc<EmCurso>>,
    bytes: usize,
    relogio: u64,
    contadores: Contadores,
}

impl Interno {
    /// Marca a entrada como a mais recente e devolve a IR; `None` na falta dela.
    fn tocar(&mut self, chave: &ChaveRevisao) -> Option<Arc<Ir>> {
        self.relogio += 1;
        let carimbo = self.relogio;
        let entrada = self.entradas.get_mut(chave)?;
        entrada.usado_em = carimbo;
        Some(Arc::clone(&entrada.ir))
    }

    /// Guarda a revisão se couber, expulsando o que não está em uso.
    ///
    /// A conferência de espaço vem **antes** da expulsão: sem ela, uma revisão que não caberia de
    /// jeito (teto ocupado só por entradas em uso) ainda tiraria do cache o que não está em uso,
    /// sem nada guardar no lugar. Com todas as entradas em uso, a revisão nova simplesmente não
    /// entra e a chamada segue sem cache.
    fn guardar(&mut self, chave: &ChaveRevisao, ir: Arc<Ir>, teto: usize) {
        if self.entradas.contains_key(chave) {
            return;
        }
        let bytes = peso_da_ir(&ir);
        if bytes > teto || !self.ha_espaco(bytes, teto) {
            self.contadores.nao_cacheadas += 1;
            return;
        }
        self.abrir_espaco(bytes, teto);
        self.relogio += 1;
        let usado_em = self.relogio;
        self.entradas.insert(
            chave.clone(),
            Entrada {
                ir,
                bytes,
                usado_em,
            },
        );
        self.bytes += bytes;
    }

    /// Se a revisão cabe no teto contando só o que pode sair (o que não está em uso).
    fn ha_espaco(&self, precisa: usize, teto: usize) -> bool {
        let livre = teto.saturating_sub(self.bytes);
        if livre >= precisa {
            return true;
        }
        let expulsavel: usize = self
            .entradas
            .values()
            .filter(|entrada| Arc::strong_count(&entrada.ir) == 1)
            .map(|entrada| entrada.bytes)
            .sum();
        livre + expulsavel >= precisa
    }

    /// Expulsa entradas até a nova caber; para quando só restam entradas em uso.
    fn abrir_espaco(&mut self, precisa: usize, teto: usize) {
        while self.bytes + precisa > teto {
            let Some(vitima) = self.vitima() else {
                return;
            };
            if let Some(entrada) = self.entradas.remove(&vitima) {
                self.bytes = self.bytes.saturating_sub(entrada.bytes);
                self.contadores.expulsoes += 1;
            } else {
                return;
            }
        }
    }

    /// A entrada menos recentemente usada entre as que ninguém está segurando.
    fn vitima(&mut self) -> Option<ChaveRevisao> {
        let mut escolhida: Option<(ChaveRevisao, u64)> = None;
        let mut poupadas = 0u64;
        for (chave, entrada) in &self.entradas {
            // `strong_count` 1 é só a do cache: mais que isso, alguém está com a IR na mão.
            if Arc::strong_count(&entrada.ir) > 1 {
                poupadas += 1;
                continue;
            }
            let melhor = escolhida
                .as_ref()
                .is_none_or(|(_, usado)| entrada.usado_em < *usado);
            if melhor {
                escolhida = Some((chave.clone(), entrada.usado_em));
            }
        }
        self.contadores.em_uso_poupadas += poupadas;
        escolhida.map(|(chave, _)| chave)
    }

    fn metricas(&self) -> Metricas {
        Metricas {
            acertos: self.contadores.acertos,
            erros: self.contadores.erros,
            cargas: self.contadores.cargas,
            esperas: self.contadores.esperas,
            expulsoes: self.contadores.expulsoes,
            em_uso_poupadas: self.contadores.em_uso_poupadas,
            nao_cacheadas: self.contadores.nao_cacheadas,
            entradas: self.entradas.len(),
            em_uso: self
                .entradas
                .values()
                .filter(|entrada| Arc::strong_count(&entrada.ir) > 1)
                .count(),
            bytes: self.bytes,
        }
    }
}

/// Cache de revisões compiladas, com teto em bytes e expulsão do menos usado.
///
/// `Clone` compartilha o mesmo estado (é um `Arc` por dentro): um cache por processo, entregue às
/// threads que precisarem.
#[derive(Clone)]
pub struct CacheRevisoes {
    interno: Arc<Mutex<Interno>>,
    teto_bytes: usize,
}

impl CacheRevisoes {
    /// Cache com teto em MB (a unidade de `fluxos.cache_revisoes_mb`).
    pub fn novo(teto_mb: usize) -> Self {
        Self::com_teto_bytes(teto_mb.saturating_mul(1024 * 1024))
    }

    /// Cache com teto em bytes. Teto zero significa "não guarda nada" (toda carga conta em
    /// `Metricas::nao_cacheadas`), não erro.
    pub fn com_teto_bytes(teto_bytes: usize) -> Self {
        Self {
            interno: Arc::new(Mutex::new(Interno {
                entradas: HashMap::new(),
                em_curso: HashMap::new(),
                bytes: 0,
                relogio: 0,
                contadores: Contadores::default(),
            })),
            teto_bytes,
        }
    }

    /// Teto configurado, em bytes.
    pub fn teto_bytes(&self) -> usize {
        self.teto_bytes
    }

    /// Retrato das contas e do estado de agora.
    pub fn metricas(&self) -> Metricas {
        self.trava().metricas()
    }

    /// Se a revisão está no cache agora (não conta acerto nem mexe na ordem de uso).
    pub fn contem(&self, chave: &ChaveRevisao) -> bool {
        self.trava().entradas.contains_key(chave)
    }

    /// A revisão já carregada, se estiver no cache.
    ///
    /// Conta como **uso** (a entrada passa a ser a mais recente), mas não mexe nas métricas de
    /// acerto/erro: elas são contadas em [`CacheRevisoes::carregar`], que é a porta de entrada de
    /// quem vai executar.
    pub fn obter(&self, chave: &ChaveRevisao) -> Option<Arc<Ir>> {
        self.trava().tocar(chave)
    }

    /// Esvazia o cache e devolve quantas entradas saíram. As métricas não são zeradas: o histórico
    /// de acertos e cargas continua valendo.
    pub fn limpar(&self) -> usize {
        let mut interno = self.trava();
        let saidas = interno.entradas.len();
        interno.entradas.clear();
        interno.bytes = 0;
        saidas
    }

    /// Devolve a IR da revisão, carregando-a se ainda não estiver no cache.
    ///
    /// `ir_gravada` é a IR publicada (`fluxo_revisoes.definicao_json`) e `fonte` o texto da mesma
    /// revisão: a carga recompila o texto e confere que dá a IR gravada
    /// ([`crate::compilador::carregar`]) — se não der, a revisão **não** entra no cache e a falha
    /// volta tipada.
    ///
    /// Chamadas concorrentes da mesma chave compilam **uma** vez: as outras esperam o resultado
    /// daquela carga.
    pub fn carregar(
        &self,
        chave: &ChaveRevisao,
        ir_gravada: &Ir,
        fonte: &str,
        limites: &Limites,
    ) -> Result<Arc<Ir>, FalhaCarregamento> {
        let espera = {
            let mut interno = self.trava();
            if let Some(ir) = interno.tocar(chave) {
                interno.contadores.acertos += 1;
                return Ok(ir);
            }
            interno.contadores.erros += 1;
            let em_curso = interno.em_curso.get(chave).map(Arc::clone);
            match em_curso {
                Some(em_curso) => {
                    interno.contadores.esperas += 1;
                    Some(em_curso)
                }
                None => {
                    interno
                        .em_curso
                        .insert(chave.clone(), Arc::new(EmCurso::novo()));
                    None
                }
            }
        };

        if let Some(em_curso) = espera {
            if let Some(resultado) = em_curso.esperar() {
                return resultado;
            }
            // Quem carregava sumiu (pânico): assume o lote — carrega, guarda e acorda quem ainda
            // espera, para a chave não ficar presa em "em curso".
            let resultado = self.carregar_sozinho(ir_gravada, fonte, limites);
            self.concluir(chave, &resultado);
            return resultado;
        }

        let resultado = self.carregar_sozinho(ir_gravada, fonte, limites);
        self.concluir(chave, &resultado);
        resultado
    }

    /// Compila uma vez e conta a carga.
    fn carregar_sozinho(
        &self,
        ir_gravada: &Ir,
        fonte: &str,
        limites: &Limites,
    ) -> Result<Arc<Ir>, FalhaCarregamento> {
        {
            let mut interno = self.trava();
            interno.contadores.cargas += 1;
        }
        carregar_revisao(ir_gravada, fonte, limites).map(Arc::new)
    }

    /// Fecha um lote: guarda o que deu certo, tira a chave de "em curso" e acorda quem espera.
    fn concluir(&self, chave: &ChaveRevisao, resultado: &Result<Arc<Ir>, FalhaCarregamento>) {
        let em_curso = {
            let mut interno = self.trava();
            if let Ok(ir) = resultado {
                interno.guardar(chave, Arc::clone(ir), self.teto_bytes);
            }
            interno.em_curso.remove(chave)
        };
        if let Some(em_curso) = em_curso {
            em_curso.publicar(resultado.clone());
        }
    }

    /// `Mutex` tolerante a envenenamento: pânico de outra thread não derruba o cache.
    fn trava(&self) -> MutexGuard<'_, Interno> {
        self.interno
            .lock()
            .unwrap_or_else(|envenenado| envenenado.into_inner())
    }
}

impl std::fmt::Debug for CacheRevisoes {
    fn fmt(&self, saida: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let metricas = self.metricas();
        saida
            .debug_struct("CacheRevisoes")
            .field("teto_bytes", &self.teto_bytes)
            .field("entradas", &metricas.entradas)
            .field("bytes", &metricas.bytes)
            .field("acertos", &metricas.acertos)
            .field("erros", &metricas.erros)
            .field("expulsoes", &metricas.expulsoes)
            .finish()
    }
}

#[cfg(test)]
mod testes {
    use super::{ChaveRevisao, sha256_da_fonte};

    #[test]
    fn o_hash_da_fonte_e_sha256_em_hexadecimal_minusculo() {
        // Vetor conhecido: sha256 de "abc".
        assert_eq!(
            sha256_da_fonte("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_chave_nova_carrega_o_hash_do_texto() {
        let chave = ChaveRevisao::nova("agendar_visita", 3, "abc");
        assert_eq!(chave.fluxo_id(), "agendar_visita");
        assert_eq!(chave.numero(), 3);
        assert_eq!(chave.hash(), sha256_da_fonte("abc"));
        assert_ne!(chave, ChaveRevisao::nova("agendar_visita", 3, "abd"));
    }

    #[test]
    fn a_chave_com_hash_usa_o_valor_gravado_na_publicacao() {
        let chave = ChaveRevisao::com_hash("triagem", 1, "abc123");
        assert_eq!(chave.hash(), "abc123");
    }
}
