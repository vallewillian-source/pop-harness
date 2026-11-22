//! Usuários virtuais (fantasmas) e nome de exibição — MX-10.
//!
//! O módulo fala na sala em nome de um usuário virtual por participante: um por cliente, um por
//! agente e, na fase 2, um por aparelho da instância do WhatsApp ("Humano no Aparelho"). Este
//! arquivo cuida de três coisas:
//!
//! 1. **Montar o id** de cada fantasma, com `localpart` válida para o Matrix. O id é
//!    determinístico: a mesma conversa sempre cai no mesmo fantasma, mesmo depois de reiniciar.
//! 2. **Registrar e nomear** o fantasma de forma idempotente: registrar duas vezes não é erro, e o
//!    nome de exibição só é reescrito quando mudou.
//! 3. **Lembrar** o que já está pronto, num cache em memória com teto (senão ele cresceria com o
//!    número de conversas e viraria vazamento de memória).
//!
//! ## Ids
//!
//! | Participante | Id | Exemplo |
//! |---|---|---|
//! | Cliente | `@<prefixo>_wa_<telefone normalizado>:<servidor>` | `@ph_wa_5511999999999:matrix-dev.local` |
//! | Agente | `@<prefixo>_ag_<id do agente>` | `@ph_ag_01912345-…:matrix-dev.local` |
//! | Aparelho (fase 2) | `@<prefixo>_ap_<instância da Evolution>` | `@ph_ap_inst_1:matrix-dev.local` |
//!
//! A `localpart` do Matrix só aceita `a-z`, `0-9`, `.`, `_` e `-` (e o Synapse recusa o que não casa
//! com o namespace `@<prefixo>_.*` do registro do Application Service — comprovado na MX-02).
//!
//! ## Normalização
//!
//! - **Telefone:** só a formatação padrão (`+`, `-`, espaço, `(`, `)`, `.`) é descartada; o
//!   resultado são os dígitos. Isso é de propósito: `+55 (11) 99999-9999` e `5511999999999` são o
//!   **mesmo cliente** e precisam cair no mesmo fantasma — senão a mesma pessoa apareceria com dois
//!   nomes na sala. Telefones diferentes (um dígito de diferença) nunca colidem, porque o corpo do
//!   id é o próprio número.
//! - **Qualquer outro identificador** (id de agente, instância, telefone com letra ou símbolo
//!   inesperado): cada caractere inseguro sai e o resultado leva um sufixo com o resumo FNV-1a do
//!   texto original. Assim dois identificadores distintos que "limparam" para o mesmo texto continuam
//!   distintos (`Fulano de Tal` ≠ `fulano-de-tal`), e nada fica com caractere inválido.
//! - **Tamanho:** o corpo do id é cortado em [`LIMITE_CORPO_LOCALPART`] caracteres, sempre com o
//!   resumo do original quando houve corte — id longo não colide com outro id longo do mesmo prefixo.
//!
//! ## Nome de exibição
//!
//! Cliente vira `Nome (WhatsApp)`; agente vira o nome do agente. Nome vazio tem padrão. O perfil só é
//! reescrito quando o nome desejado difere do que já foi aplicado, e o que foi aplicado fica no
//! cache: a mesma conversa não gera requisição de perfil de novo.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;

use poph_types::MatrixConfig;

use crate::cliente::ClienteMatrix;
use crate::erro::ErroChamada;

/// Marca do fantasma do cliente (WhatsApp) na `localpart`.
pub const MARCA_CLIENTE: &str = "wa";
/// Marca do fantasma do agente na `localpart`.
pub const MARCA_AGENTE: &str = "ag";
/// Marca do fantasma do aparelho (fase 2, "Humano no Aparelho") na `localpart`.
pub const MARCA_APARELHO: &str = "ap";
/// `localpart` do bot do Application Service (`sender_localpart` do registro).
///
/// O registro do AS é gerado com `sender_localpart: <prefixo>_as_bot` (PRD §6.2 e o modelo em
/// `tests/matrix/registro-teste.yaml.template`). O bot não é um fantasma de participante: é o
/// remetente com que o próprio Application Service escreve, e mensagem dele nunca é resposta de
/// atendente (MX-17).
pub const SUFIXO_BOT: &str = "as_bot";
/// Marcas que identificam um fantasma desta instância na `localpart`.
const MARCAS_FANTASMA: [&str; 3] = [MARCA_CLIENTE, MARCA_AGENTE, MARCA_APARELHO];
/// Sufixo do nome de exibição do cliente.
pub const SUFIXO_NOME_CLIENTE: &str = " (WhatsApp)";
/// Nome de exibição do cliente quando a conversa não trouxe nome.
pub const NOME_CLIENTE_PADRAO: &str = "Cliente";
/// Teto padrão do cache de fantasmas prontos.
///
/// O teto é parâmetro de [`UsuariosVirtuais::novo`]; este valor é o que a ligação do módulo usa
/// quando não houver outro. Ele existe para o cache não crescer com o número de conversas.
pub const TETO_PADRAO_CACHE: usize = 1024;
/// Maior corpo de `localpart` montado por este módulo, em caracteres.
///
/// O limite real do Synapse é bem maior (255 no id inteiro); este teto é folgado de propósito e
/// serve só para o id não crescer com identificador absurdo.
pub const LIMITE_CORPO_LOCALPART: usize = 48;

/// `errcode` que o Synapse devolve ao registrar um usuário que já existe (comprovado na MX-02).
const ERRCODE_JA_EXISTE: &str = "M_USER_IN_USE";

/// Falha ao montar o id ou ao deixar o fantasma pronto.
///
/// Nenhuma variante carrega o telefone, o id de origem ou o token: só o papel do identificador e um
/// motivo curto.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErroFantasmas {
    /// O identificador não serve para montar um id (vazio, por exemplo).
    #[error("identificador de {alvo} inválido: {motivo}")]
    Identificador {
        /// Papel do identificador (`cliente`, `agente`, `aparelho`).
        alvo: &'static str,
        /// Motivo curto, sem repetir o valor.
        motivo: &'static str,
    },
    /// A seção `[matrix]` trouxe prefixo ou `server_name` que não servem para montar id.
    #[error("configuração do módulo Matrix inválida para montar usuários virtuais: {motivo}")]
    Configuracao {
        /// O que não serve (nome da chave, nunca o valor).
        motivo: &'static str,
    },
    /// O Synapse registrou (ou já tinha) um usuário com id diferente do calculado.
    #[error(
        "o Synapse devolveu um id de usuário diferente do esperado — confira `matrix.server_name`"
    )]
    UsuarioDivergente,
    /// Falha ao falar com o Synapse (registro, perfil ou nome).
    #[error(transparent)]
    Chamada(#[from] ErroChamada),
}

/// Um usuário virtual: a `localpart`, o id completo e o servidor de onde ele veio.
#[derive(Clone, PartialEq, Eq)]
pub struct Fantasma {
    localpart: String,
    user_id: String,
}

impl Fantasma {
    /// `localpart` sem `@` e sem `:servidor` — o que o `register` recebe.
    pub fn localpart(&self) -> &str {
        &self.localpart
    }

    /// Id completo (`@localpart:servidor`), o que vai em `?user_id=` e no perfil.
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
}

impl fmt::Display for Fantasma {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_id)
    }
}

impl fmt::Debug for Fantasma {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A `localpart` repete o id; no `Debug` fica só o id, que é o que aparece em log de sala.
        f.debug_tuple("Fantasma").field(&self.user_id).finish()
    }
}

/// Monta os ids dos fantasmas desta instância.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdsFantasmas {
    prefixo: String,
    server_name: String,
}

impl IdsFantasmas {
    /// Monta com o prefixo e o domínio do Matrix, validando o que quebraria o id.
    ///
    /// O prefixo segue a regra da seção `[matrix]`: `[a-z0-9]{1,12}`.
    pub fn novo(
        prefixo: impl Into<String>,
        server_name: impl Into<String>,
    ) -> Result<Self, ErroFantasmas> {
        let prefixo = prefixo.into();
        if prefixo.is_empty() || prefixo.chars().count() > 12 {
            return Err(ErroFantasmas::Configuracao {
                motivo: "matrix.prefixo deve ter de 1 a 12 caracteres",
            });
        }
        if !prefixo
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return Err(ErroFantasmas::Configuracao {
                motivo: "matrix.prefixo só aceita a-z e 0-9",
            });
        }
        let server_name = server_name.into();
        let server_name = server_name.trim();
        if server_name.is_empty() {
            return Err(ErroFantasmas::Configuracao {
                motivo: "matrix.server_name está vazio",
            });
        }
        Ok(Self {
            prefixo,
            server_name: server_name.to_owned(),
        })
    }

    /// Monta a partir da seção `[matrix]` já validada.
    pub fn da_config(config: &MatrixConfig) -> Result<Self, ErroFantasmas> {
        Self::novo(config.prefixo.clone(), config.server_name.clone())
    }

    /// Prefixo em uso.
    pub fn prefixo(&self) -> &str {
        &self.prefixo
    }

    /// Domínio dos ids Matrix em uso.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Fantasma do cliente: `@<prefixo>_wa_<telefone normalizado>:<servidor>`.
    ///
    /// O identificador é o `peer` da conversa (número com ou sem formatação; um jid de grupo também
    /// vira id válido, pelo caminho opaco).
    pub fn cliente(&self, identificador: &str) -> Result<Fantasma, ErroFantasmas> {
        let corpo = corpo_de_cliente(identificador)?;
        Ok(self.montar(MARCA_CLIENTE, &corpo))
    }

    /// Fantasma do agente: `@<prefixo>_ag_<id do agente>:<servidor>`.
    pub fn agente(&self, id_agente: &str) -> Result<Fantasma, ErroFantasmas> {
        let corpo = corpo_opaco(id_agente).ok_or(ErroFantasmas::Identificador {
            alvo: "agente",
            motivo: "vazio",
        })?;
        Ok(self.montar(MARCA_AGENTE, &corpo))
    }

    /// Fantasma do aparelho (fase 2): `@<prefixo>_ap_<instância da Evolution>:<servidor>`.
    ///
    /// Só monta o id; registro, nome e uso na sala são da MX-42.
    pub fn aparelho(&self, instancia: &str) -> Result<Fantasma, ErroFantasmas> {
        let corpo = corpo_opaco(instancia).ok_or(ErroFantasmas::Identificador {
            alvo: "aparelho",
            motivo: "vazio",
        })?;
        Ok(self.montar(MARCA_APARELHO, &corpo))
    }

    /// Bot do Application Service: `@<prefixo>_as_bot:<servidor>`.
    ///
    /// É o `sender_localpart` do registro do AS — não é fantasma de participante, é o remetente com
    /// que o próprio Application Service escreve.
    pub fn bot(&self) -> Fantasma {
        let localpart = format!("{}_{SUFIXO_BOT}", self.prefixo);
        let user_id = format!("@{localpart}:{}", self.server_name);
        Fantasma { localpart, user_id }
    }

    /// O id é o bot desta instância?
    pub fn eh_bot(&self, user_id: &str) -> bool {
        match partir_id(user_id) {
            Some((localpart, servidor)) => {
                servidor.eq_ignore_ascii_case(&self.server_name)
                    && localpart == format!("{}_{SUFIXO_BOT}", self.prefixo)
            }
            None => false,
        }
    }

    /// O id é um fantasma desta instância (cliente, agente ou aparelho)?
    ///
    /// A conferência é exata: o domínio precisa ser o desta instância e a `localpart` precisa ser
    /// `<prefixo>_<marca>_<corpo>`, com a marca entre as de [`MARCAS_FANTASMA`] e corpo não vazio.
    /// Prefixo parecido **não** basta — `@ph_willian:…` é um atendente humano, e tratá-lo como
    /// fantasma faria a resposta dele ser jogada fora em silêncio (MX-17).
    pub fn eh_fantasma(&self, user_id: &str) -> bool {
        let Some((localpart, servidor)) = partir_id(user_id) else {
            return false;
        };
        if !servidor.eq_ignore_ascii_case(&self.server_name) {
            return false;
        }
        let Some(resto) = localpart.strip_prefix(&format!("{}_", self.prefixo)) else {
            return false;
        };
        MARCAS_FANTASMA.iter().any(|marca| {
            resto
                .strip_prefix(marca)
                .and_then(|depois| depois.strip_prefix('_'))
                .is_some_and(|corpo| !corpo.is_empty())
        })
    }

    fn montar(&self, marca: &str, corpo: &str) -> Fantasma {
        let localpart = format!("{}_{marca}_{corpo}", self.prefixo);
        let user_id = format!("@{localpart}:{}", self.server_name);
        Fantasma { localpart, user_id }
    }
}

/// Separa `@localpart:servidor` em `(localpart, servidor)`. Qualquer outro formato não é id Matrix.
fn partir_id(user_id: &str) -> Option<(&str, &str)> {
    let resto = user_id.strip_prefix('@')?;
    let (localpart, servidor) = resto.split_once(':')?;
    if localpart.is_empty() || servidor.is_empty() {
        return None;
    }
    Some((localpart, servidor))
}

/// Nome de exibição do cliente: `Nome (WhatsApp)`.
///
/// Espaços em excesso somem; nome vazio vira [`NOME_CLIENTE_PADRAO`]. Se o nome já vier com o
/// sufixo (por exemplo, lido de volta do perfil), ele não é aplicado duas vezes.
pub fn nome_exibicao_cliente(nome: &str) -> String {
    let limpo = colapsar(nome);
    if limpo.is_empty() {
        return format!("{NOME_CLIENTE_PADRAO}{SUFIXO_NOME_CLIENTE}");
    }
    if limpo.ends_with(SUFIXO_NOME_CLIENTE) {
        return limpo;
    }
    format!("{limpo}{SUFIXO_NOME_CLIENTE}")
}

/// Nome de exibição do agente: o nome do agente; vazio cai no `padrao` informado (o id do agente).
pub fn nome_exibicao_agente(nome: &str, padrao: &str) -> String {
    let limpo = colapsar(nome);
    if limpo.is_empty() {
        padrao.to_owned()
    } else {
        limpo
    }
}

/// Deixa o fantasma registrado no Synapse e com o nome de exibição desejado.
///
/// O cache guarda o que já está pronto (`user_id` → nome aplicado), com teto: o fantasma que sai do
/// cache só custa uma verificação de perfil a mais, nunca um erro.
pub struct UsuariosVirtuais {
    ids: IdsFantasmas,
    cliente: ClienteMatrix,
    prontos: Mutex<CacheProntos>,
}

impl UsuariosVirtuais {
    /// Monta o serviço com o cliente da MX-09 e o teto do cache.
    ///
    /// Teto `0` é tratado como `1`: um cache que não guarda nada só multiplicaria requisições.
    pub fn novo(cliente: ClienteMatrix, ids: IdsFantasmas, teto_cache: usize) -> Self {
        Self {
            ids,
            cliente,
            prontos: Mutex::new(CacheProntos::novo(teto_cache.max(1))),
        }
    }

    /// Monta a partir da seção `[matrix]`, com o teto padrão ([`TETO_PADRAO_CACHE`]).
    pub fn da_config(cliente: ClienteMatrix, config: &MatrixConfig) -> Result<Self, ErroFantasmas> {
        let ids = IdsFantasmas::da_config(config)?;
        Ok(Self::novo(cliente, ids, TETO_PADRAO_CACHE))
    }

    /// Os ids em uso (quem precisa montar um id sem falar com o Synapse usa isto).
    pub fn ids(&self) -> &IdsFantasmas {
        &self.ids
    }

    /// `(ocupado, teto)` do cache de fantasmas prontos, para métrica e teste.
    pub fn cache(&self) -> (usize, usize) {
        let cache = self.travar();
        (cache.mapa.len(), cache.teto)
    }

    /// Garante o fantasma do cliente, com o nome `Nome (WhatsApp)`.
    pub async fn garantir_cliente(
        &self,
        identificador: &str,
        nome: &str,
    ) -> Result<Fantasma, ErroFantasmas> {
        let fantasma = self.ids.cliente(identificador)?;
        self.garantir(fantasma, nome_exibicao_cliente(nome)).await
    }

    /// Garante o fantasma do agente, com o nome do agente (ou o id, se o nome vier vazio).
    pub async fn garantir_agente(
        &self,
        id_agente: &str,
        nome: &str,
    ) -> Result<Fantasma, ErroFantasmas> {
        let fantasma = self.ids.agente(id_agente)?;
        let exibicao = nome_exibicao_agente(nome, id_agente);
        self.garantir(fantasma, exibicao).await
    }

    /// Registra (idempotente) e aplica o nome de exibição só quando ele mudou.
    async fn garantir(&self, fantasma: Fantasma, nome: String) -> Result<Fantasma, ErroFantasmas> {
        // 1. Já foi feito nesta vida do processo? Mesmo nome: não fala com o Synapse. Nome novo:
        //    o registro já está feito, então só o perfil muda (sem nova leitura de perfil).
        if let Some(aplicado) = self.nome_aplicado(fantasma.user_id()) {
            if aplicado == nome {
                return Ok(fantasma);
            }
            self.aplicar_nome(&fantasma, &nome).await?;
            return Ok(fantasma);
        }

        // 2. Registro idempotente. `M_USER_IN_USE` é o caso normal de um fantasma que já existe.
        let recem_registrado = match self.cliente.registrar_usuario(fantasma.localpart()).await {
            Ok(usuario_id) => {
                if usuario_id != fantasma.user_id() {
                    return Err(ErroFantasmas::UsuarioDivergente);
                }
                true
            }
            Err(ErroChamada::Resposta {
                errcode: Some(errcode),
                ..
            }) if errcode == ERRCODE_JA_EXISTE => false,
            Err(erro) => return Err(erro.into()),
        };

        // 3. Nome de exibição: usuário novo nasce sem nome; usuário existente é conferido antes de
        //    reescrever.
        if !recem_registrado {
            let perfil = self.cliente.perfil(fantasma.user_id()).await?;
            if perfil.displayname.as_deref() == Some(nome.as_str()) {
                self.memorizar(fantasma.user_id(), &nome);
                return Ok(fantasma);
            }
        }
        self.cliente
            .definir_nome_exibicao(fantasma.user_id(), fantasma.user_id(), &nome)
            .await?;
        self.memorizar(fantasma.user_id(), &nome);
        Ok(fantasma)
    }

    /// Reescreve o nome de exibição e guarda no cache (operação idempotente no Synapse).
    async fn aplicar_nome(&self, fantasma: &Fantasma, nome: &str) -> Result<(), ErroFantasmas> {
        self.cliente
            .definir_nome_exibicao(fantasma.user_id(), fantasma.user_id(), nome)
            .await?;
        self.memorizar(fantasma.user_id(), nome);
        Ok(())
    }

    fn nome_aplicado(&self, user_id: &str) -> Option<String> {
        self.travar().mapa.get(user_id).cloned()
    }

    fn memorizar(&self, user_id: &str, nome: &str) {
        self.travar().registrar(user_id.to_owned(), nome.to_owned());
    }

    fn travar(&self) -> std::sync::MutexGuard<'_, CacheProntos> {
        // A trava nunca é mantida através de um `await`: as seções críticas são só leitura e escrita
        // de mapa. Um pânico de outra tarefa envenenaria o cache, e perder o cache não é corrupção:
        // o pior caso é refazer registro/perfil.
        self.prontos
            .lock()
            .unwrap_or_else(|envenenado| envenenado.into_inner())
    }
}

impl fmt::Debug for UsuariosVirtuais {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (ocupado, teto) = self.cache();
        f.debug_struct("UsuariosVirtuais")
            .field("ids", &self.ids)
            .field("cache", &format_args!("{ocupado}/{teto}"))
            .finish()
    }
}

/// Cache com teto do que já está pronto: `user_id` → nome de exibição aplicado.
///
/// A política é FIFO simples (o mais antigo sai primeiro). Não precisa ser LRU: o custo de errar é
/// uma leitura de perfil a mais, e FIFO cabe em poucas linhas.
struct CacheProntos {
    teto: usize,
    mapa: HashMap<String, String>,
    ordem: VecDeque<String>,
}

impl CacheProntos {
    fn novo(teto: usize) -> Self {
        Self {
            teto,
            mapa: HashMap::new(),
            ordem: VecDeque::new(),
        }
    }

    /// Guarda ou atualiza o nome aplicado, respeitando o teto.
    fn registrar(&mut self, user_id: String, nome: String) {
        if let Some(atual) = self.mapa.get_mut(&user_id) {
            *atual = nome;
            return;
        }
        while self.mapa.len() >= self.teto {
            if let Some(antigo) = self.ordem.pop_front() {
                self.mapa.remove(&antigo);
            } else {
                break;
            }
        }
        self.ordem.push_back(user_id.clone());
        self.mapa.insert(user_id, nome);
    }
}

// ───────────────────────────── normalização ─────────────────────────────

/// Corpo do id do cliente: dígitos para telefone com formatação normal; opaco para o resto.
fn corpo_de_cliente(identificador: &str) -> Result<String, ErroFantasmas> {
    let bruto = identificador.trim();
    if bruto.is_empty() {
        return Err(ErroFantasmas::Identificador {
            alvo: "cliente",
            motivo: "vazio",
        });
    }
    let so_formatacao_de_telefone = bruto
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | ' ' | '(' | ')' | '.' | '\u{a0}'));
    if so_formatacao_de_telefone {
        let digitos: String = bruto.chars().filter(char::is_ascii_digit).collect();
        if digitos.is_empty() {
            return Err(ErroFantasmas::Identificador {
                alvo: "cliente",
                motivo: "telefone sem dígito",
            });
        }
        if digitos.chars().count() <= LIMITE_CORPO_LOCALPART {
            return Ok(digitos);
        }
        // Telefone absurdamente longo: cai no caminho opaco para caber no teto sem colidir.
        return corpo_opaco(&digitos).ok_or(ErroFantasmas::Identificador {
            alvo: "cliente",
            motivo: "telefone sem dígito",
        });
    }
    corpo_opaco(bruto).ok_or(ErroFantasmas::Identificador {
        alvo: "cliente",
        motivo: "sem caractere utilizável",
    })
}

/// Corpo de id a partir de texto livre: mantém `a-z`, `0-9`, `.`, `_` e `-`; só `A-Z` vira
/// minúscula; o resto sai.
///
/// Quando qualquer coisa mudou (corte, descarte ou caixa), o resultado leva `_<resumo FNV-1a>`, para
/// dois textos distintos nunca caírem no mesmo id. `None` só quando o texto é vazio.
fn corpo_opaco(bruto: &str) -> Option<String> {
    if bruto.is_empty() {
        return None;
    }
    let base: String = bruto
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '_' | '-' => Some(c),
            'A'..='Z' => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let resumo = format!("{:016x}", resumo_fnv1a(bruto));
    if base.is_empty() {
        return Some(format!("h_{resumo}"));
    }
    let curto: String = base.chars().take(LIMITE_CORPO_LOCALPART).collect();
    if curto == bruto {
        Some(curto)
    } else {
        Some(format!("{curto}_{resumo}"))
    }
}

/// FNV-1a de 64 bits: estável entre versões e máquinas (o `DefaultHasher` do Rust não promete isso).
fn resumo_fnv1a(texto: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in texto.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Espaços em excesso e quebras viram um espaço; pontas somem.
fn colapsar(texto: &str) -> String {
    texto.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod testes {
    use super::*;

    fn ids() -> IdsFantasmas {
        IdsFantasmas::novo("ph", "matrix-dev.local").expect("prefixo e servidor válidos")
    }

    /// Só `a-z`, `0-9`, `.`, `_` e `-`: o que o Synapse aceita na `localpart` (MX-02).
    fn localpart_valida(localpart: &str) -> bool {
        !localpart.is_empty()
            && localpart.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
            })
    }

    #[test]
    fn cliente_so_digitos_nao_ganha_sufixo_de_resumo() {
        let fantasma = ids().cliente("5511999999999").expect("telefone válido");
        assert_eq!(fantasma.localpart(), "ph_wa_5511999999999");
        assert_eq!(fantasma.user_id(), "@ph_wa_5511999999999:matrix-dev.local");
    }

    #[test]
    fn formatacao_do_mesmo_telefone_cai_no_mesmo_fantasma() {
        // Decisão: o mesmo número é o mesmo cliente. Formatação diferente não pode criar dois
        // fantasmas para a mesma pessoa.
        let esperado = ids()
            .cliente("5511999999999")
            .expect("telefone")
            .user_id()
            .to_owned();
        for variacao in [
            "+55 (11) 99999-9999",
            "+5511999999999",
            "55 11 99999 9999",
            "5511-99999-9999",
            " 5511999999999 ",
            "55.11.99999.9999",
        ] {
            let fantasma = ids().cliente(variacao).expect("variação válida");
            assert_eq!(
                fantasma.user_id(),
                esperado,
                "formatação {variacao} criou outro fantasma"
            );
        }
    }

    #[test]
    fn telefones_diferentes_nunca_colidem() {
        let base = ids().cliente("5511999999999").expect("telefone");
        for outro in [
            "5511999999998",
            "5511989999999",
            "5521999999999",
            "511999999999",
            "55119999999990",
        ] {
            let fantasma = ids().cliente(outro).expect("telefone");
            assert_ne!(
                fantasma.user_id(),
                base.user_id(),
                "telefone {outro} colidiu com outro cliente"
            );
        }
    }

    #[test]
    fn identificador_com_caractere_invalido_gera_localpart_valida_e_unica() {
        // Jid de grupo, telefone com letra e símbolo: cada um vira id válido e distinto.
        let casos = [
            "120363012345678901@g.us",
            "5511ABC999",
            "55 11 99999-9999 ramal 2",
            "!!!",
            "cliente/sem/numero",
            "cliente:sem:numero",
        ];
        let mut vistos = std::collections::BTreeSet::new();
        for caso in casos {
            let fantasma = ids()
                .cliente(caso)
                .unwrap_or_else(|erro| panic!("{caso} deveria gerar id: {erro}"));
            assert!(
                localpart_valida(fantasma.localpart()),
                "{caso} gerou localpart inválida: {}",
                fantasma.localpart()
            );
            assert!(
                fantasma.user_id().starts_with("@ph_wa_"),
                "{caso} saiu do namespace: {}",
                fantasma.user_id()
            );
            assert!(
                vistos.insert(fantasma.user_id().to_owned()),
                "{caso} colidiu"
            );
        }
    }

    #[test]
    fn ids_de_agente_com_caractere_invalido_nao_colidem_entre_si() {
        // Três formas que "limpam" para o mesmo texto precisam continuar três fantasmas.
        let casos = [
            "Agente Um",
            "agente um",
            "AGENTE-UM",
            "agente_um",
            "agente.um",
        ];
        let mut vistos = std::collections::BTreeSet::new();
        for caso in casos {
            let fantasma = ids().agente(caso).expect("id de agente");
            assert!(localpart_valida(fantasma.localpart()), "{caso}");
            assert!(
                vistos.insert(fantasma.user_id().to_owned()),
                "{caso} colidiu"
            );
        }
    }

    #[test]
    fn id_de_agente_uuid_fica_legivel() {
        let fantasma = ids()
            .agente("01912345-6789-7abc-8def-0123456789ab")
            .expect("uuid");
        assert_eq!(
            fantasma.user_id(),
            "@ph_ag_01912345-6789-7abc-8def-0123456789ab:matrix-dev.local"
        );
    }

    #[test]
    fn id_longo_e_cortado_no_teto_sem_colidir_com_outro_longo() {
        let a = "z".repeat(200);
        let mut b = "z".repeat(200);
        b.push('1');
        let fa = ids().cliente(&a).expect("id longo");
        let fb = ids().cliente(&b).expect("id longo");
        let corpo = fa.localpart().trim_start_matches("ph_wa_");
        assert_eq!(corpo.chars().count(), LIMITE_CORPO_LOCALPART + 1 + 16);
        assert!(localpart_valida(fa.localpart()));
        assert_ne!(fa.user_id(), fb.user_id(), "corte colidiu");
    }

    #[test]
    fn identificador_vazio_e_recusado_sem_repetir_o_valor() {
        for vazio in ["", "   "] {
            let erro = ids().cliente(vazio).expect_err("vazio deve ser recusado");
            assert_eq!(
                erro,
                ErroFantasmas::Identificador {
                    alvo: "cliente",
                    motivo: "vazio"
                }
            );
            assert!(!erro.to_string().contains("   "));
        }
        assert!(ids().agente("").is_err());
        assert!(ids().aparelho("").is_err());
    }

    #[test]
    fn telefone_que_e_so_formatacao_e_recusado() {
        let erro = ids().cliente("+() -.").expect_err("sem dígito");
        assert_eq!(
            erro,
            ErroFantasmas::Identificador {
                alvo: "cliente",
                motivo: "telefone sem dígito"
            }
        );
    }

    #[test]
    fn fantasma_do_aparelho_monta_id_da_fase_2() {
        let fantasma = ids().aparelho("inst_1").expect("instância");
        assert_eq!(fantasma.localpart(), "ph_ap_inst_1");
        assert_eq!(fantasma.user_id(), "@ph_ap_inst_1:matrix-dev.local");
    }

    #[test]
    fn prefixo_invalido_e_recusado_citando_a_chave_sem_o_valor() {
        for prefixo in ["", "PH", "ph!", "abcdefghijklm"] {
            let erro = IdsFantasmas::novo(prefixo, "matrix-dev.local").expect_err(prefixo);
            match erro {
                ErroFantasmas::Configuracao { motivo } => {
                    assert!(motivo.contains("matrix.prefixo"), "{motivo}");
                }
                outro => panic!("esperado erro de configuração: {outro:?}"),
            }
        }
        assert!(IdsFantasmas::novo("ph", "  ").is_err());
    }

    #[test]
    fn nome_do_cliente_leva_o_sufixo_e_nao_repete() {
        assert_eq!(nome_exibicao_cliente("Maria"), "Maria (WhatsApp)");
        assert_eq!(
            nome_exibicao_cliente("  Maria \n da  Silva "),
            "Maria da Silva (WhatsApp)"
        );
        assert_eq!(nome_exibicao_cliente(""), "Cliente (WhatsApp)");
        assert_eq!(nome_exibicao_cliente("   "), "Cliente (WhatsApp)");
        // Nome lido de volta do perfil não ganha o sufixo duas vezes.
        assert_eq!(
            nome_exibicao_cliente("Maria (WhatsApp)"),
            "Maria (WhatsApp)"
        );
    }

    #[test]
    fn nome_do_agente_nao_leva_sufixo_e_cai_no_id_quando_vazio() {
        let id = "01912345-6789-7abc-8def-0123456789ab";
        assert_eq!(nome_exibicao_agente("  Suporte  ", id), "Suporte");
        assert_eq!(nome_exibicao_agente("", id), id);
        assert_eq!(nome_exibicao_agente("   ", id), id);
    }

    #[test]
    fn cache_respeita_o_teto_e_esquece_o_mais_antigo() {
        let mut cache = CacheProntos::novo(3);
        for indice in 0..10 {
            cache.registrar(format!("@u{indice}"), format!("nome {indice}"));
            assert!(cache.mapa.len() <= 3, "passou do teto na inserção {indice}");
        }
        assert_eq!(cache.mapa.len(), 3);
        assert_eq!(cache.mapa.get("@u9").map(String::as_str), Some("nome 9"));
        assert_eq!(cache.mapa.get("@u8").map(String::as_str), Some("nome 8"));
        assert_eq!(cache.mapa.get("@u7").map(String::as_str), Some("nome 7"));
        assert!(
            !cache.mapa.contains_key("@u0"),
            "o mais antigo deveria ter saído"
        );

        // Atualizar um id que já está no cache não evicta ninguém.
        cache.registrar("@u9".to_owned(), "nome novo".to_owned());
        assert_eq!(cache.mapa.len(), 3);
        assert_eq!(cache.mapa.get("@u9").map(String::as_str), Some("nome novo"));
    }
}
