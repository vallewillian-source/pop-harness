use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;

use poph_types::MatrixConfig;

use crate::client::MatrixClient;
use crate::error::CallError;

pub const MARK_CLIENT: &str = "wa";
pub const MARK_AGENT: &str = "ag";
pub const MARK_DEVICE: &str = "ap";
pub const SUFFIX_BOT: &str = "as_bot";
const GHOST_MARKS: [&str; 3] = [MARK_CLIENT, MARK_AGENT, MARK_DEVICE];
pub const CLIENT_NAME_SUFFIX: &str = " (WhatsApp)";
pub const DEVICE_NAME: &str = "Humano no Aparelho";
pub const DEFAULT_CLIENT_NAME: &str = "Cliente";
pub const DEFAULT_CACHE_CAP: usize = 1024;
pub const LOCALPART_BODY_LIMIT: usize = 48;

const ERRCODE_ALREADY_EXISTS: &str = "M_USER_IN_USE";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GhostsError {
    #[error("identificador de {target} inválido: {reason}")]
    Identifier {
        target: &'static str,
        reason: &'static str,
    },
    #[error("configuração do módulo Matrix inválida para montar usuários virtuais: {reason}")]
    Configuration { reason: &'static str },
    #[error(
        "o Synapse devolveu um id de usuário diferente do esperado — confira `matrix.server_name`"
    )]
    DivergentUser,
    #[error(transparent)]
    Call(#[from] CallError),
}

#[derive(Clone, PartialEq, Eq)]
pub struct Ghost {
    localpart: String,
    user_id: String,
}

impl Ghost {
    pub fn localpart(&self) -> &str {
        &self.localpart
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }
}

impl fmt::Display for Ghost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_id)
    }
}

impl fmt::Debug for Ghost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Fantasma").field(&self.user_id).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostIds {
    prefix: String,
    server_name: String,
}

impl GhostIds {
    pub fn new(
        prefix: impl Into<String>,
        server_name: impl Into<String>,
    ) -> Result<Self, GhostsError> {
        let prefix = prefix.into();
        if prefix.is_empty() || prefix.chars().count() > 12 {
            return Err(GhostsError::Configuration {
                reason: "matrix.prefixo deve ter de 1 a 12 caracteres",
            });
        }
        if !prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return Err(GhostsError::Configuration {
                reason: "matrix.prefixo só aceita a-z e 0-9",
            });
        }
        let server_name = server_name.into();
        let server_name = server_name.trim();
        if server_name.is_empty() {
            return Err(GhostsError::Configuration {
                reason: "matrix.server_name está vazio",
            });
        }
        Ok(Self {
            prefix,
            server_name: server_name.to_owned(),
        })
    }

    pub fn of_config(config: &MatrixConfig) -> Result<Self, GhostsError> {
        Self::new(config.prefix.clone(), config.server_name.clone())
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn client(&self, identifier: &str) -> Result<Ghost, GhostsError> {
        let body = client_body(identifier)?;
        Ok(self.build(MARK_CLIENT, &body))
    }

    pub fn agent(&self, id_agent: &str) -> Result<Ghost, GhostsError> {
        let body = opaque_body(id_agent).ok_or(GhostsError::Identifier {
            target: "agente",
            reason: "vazio",
        })?;
        Ok(self.build(MARK_AGENT, &body))
    }

    pub fn device(&self, instance: &str) -> Result<Ghost, GhostsError> {
        let body = opaque_body(instance).ok_or(GhostsError::Identifier {
            target: "aparelho",
            reason: "vazio",
        })?;
        Ok(self.build(MARK_DEVICE, &body))
    }

    pub fn bot(&self) -> Ghost {
        let localpart = format!("{}_{SUFFIX_BOT}", self.prefix);
        let user_id = format!("@{localpart}:{}", self.server_name);
        Ghost { localpart, user_id }
    }

    pub fn eh_bot(&self, user_id: &str) -> bool {
        match partir_id(user_id) {
            Some((localpart, server)) => {
                server.eq_ignore_ascii_case(&self.server_name)
                    && localpart == format!("{}_{SUFFIX_BOT}", self.prefix)
            }
            None => false,
        }
    }

    pub fn is_ghost(&self, user_id: &str) -> bool {
        let Some((localpart, server)) = partir_id(user_id) else {
            return false;
        };
        if !server.eq_ignore_ascii_case(&self.server_name) {
            return false;
        }
        let Some(rest) = localpart.strip_prefix(&format!("{}_", self.prefix)) else {
            return false;
        };
        GHOST_MARKS.iter().any(|mark| {
            rest.strip_prefix(mark)
                .and_then(|after| after.strip_prefix('_'))
                .is_some_and(|body| !body.is_empty())
        })
    }

    fn build(&self, mark: &str, body: &str) -> Ghost {
        let localpart = format!("{}_{mark}_{body}", self.prefix);
        let user_id = format!("@{localpart}:{}", self.server_name);
        Ghost { localpart, user_id }
    }
}

fn partir_id(user_id: &str) -> Option<(&str, &str)> {
    let rest = user_id.strip_prefix('@')?;
    let (localpart, server) = rest.split_once(':')?;
    if localpart.is_empty() || server.is_empty() {
        return None;
    }
    Some((localpart, server))
}

pub fn client_display_name(name: &str) -> String {
    let clean = colapsar(name);
    if clean.is_empty() {
        return format!("{DEFAULT_CLIENT_NAME}{CLIENT_NAME_SUFFIX}");
    }
    if clean.ends_with(CLIENT_NAME_SUFFIX) {
        return clean;
    }
    format!("{clean}{CLIENT_NAME_SUFFIX}")
}

pub fn agent_display_name(name: &str, default: &str) -> String {
    let clean = colapsar(name);
    if clean.is_empty() {
        default.to_owned()
    } else {
        clean
    }
}

pub struct VirtualUsers {
    ids: GhostIds,
    client: MatrixClient,
    readies: Mutex<ReadyCache>,
}

impl VirtualUsers {
    pub fn new(client: MatrixClient, ids: GhostIds, cache_cap: usize) -> Self {
        Self {
            ids,
            client,
            readies: Mutex::new(ReadyCache::new(cache_cap.max(1))),
        }
    }

    pub fn of_config(client: MatrixClient, config: &MatrixConfig) -> Result<Self, GhostsError> {
        let ids = GhostIds::of_config(config)?;
        Ok(Self::new(client, ids, DEFAULT_CACHE_CAP))
    }

    pub fn ids(&self) -> &GhostIds {
        &self.ids
    }

    pub fn cache(&self) -> (usize, usize) {
        let cache = self.lock();
        (cache.map.len(), cache.cap)
    }

    pub async fn ensure_client(&self, identifier: &str, name: &str) -> Result<Ghost, GhostsError> {
        let ghost = self.ids.client(identifier)?;
        self.ensure(ghost, client_display_name(name)).await
    }

    pub async fn ensure_agent(&self, id_agent: &str, name: &str) -> Result<Ghost, GhostsError> {
        let ghost = self.ids.agent(id_agent)?;
        let display = agent_display_name(name, id_agent);
        self.ensure(ghost, display).await
    }

    pub async fn ensure_device(&self, ghost: Ghost) -> Result<Ghost, GhostsError> {
        let mark_device = format!("{}_{MARK_DEVICE}_", self.ids.prefix());
        if !ghost.localpart().starts_with(&mark_device) {
            return Err(GhostsError::Identifier {
                target: "aparelho",
                reason: "o id não é o do aparelho desta instância",
            });
        }
        self.ensure(ghost, DEVICE_NAME.to_owned()).await
    }

    async fn ensure(&self, ghost: Ghost, name: String) -> Result<Ghost, GhostsError> {
        if let Some(apply) = self.applied_name(ghost.user_id()) {
            if apply == name {
                return Ok(ghost);
            }
            self.apply_name(&ghost, &name).await?;
            return Ok(ghost);
        }

        let freshly_registered = match self.client.register_user(ghost.localpart()).await {
            Ok(user_id) => {
                if user_id != ghost.user_id() {
                    return Err(GhostsError::DivergentUser);
                }
                true
            }
            Err(CallError::Response {
                errcode: Some(errcode),
                ..
            }) if errcode == ERRCODE_ALREADY_EXISTS => false,
            Err(error) => return Err(error.into()),
        };

        if !freshly_registered {
            let profile = self.client.profile(ghost.user_id()).await?;
            if profile.displayname.as_deref() == Some(name.as_str()) {
                self.memorizar(ghost.user_id(), &name);
                return Ok(ghost);
            }
        }
        self.client
            .set_display_name(ghost.user_id(), ghost.user_id(), &name)
            .await?;
        self.memorizar(ghost.user_id(), &name);
        Ok(ghost)
    }

    async fn apply_name(&self, ghost: &Ghost, name: &str) -> Result<(), GhostsError> {
        self.client
            .set_display_name(ghost.user_id(), ghost.user_id(), name)
            .await?;
        self.memorizar(ghost.user_id(), name);
        Ok(())
    }

    fn applied_name(&self, user_id: &str) -> Option<String> {
        self.lock().map.get(user_id).cloned()
    }

    fn memorizar(&self, user_id: &str, name: &str) {
        self.lock().register(user_id.to_owned(), name.to_owned());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ReadyCache> {
        self.readies
            .lock()
            .unwrap_or_else(|envenenado| envenenado.into_inner())
    }
}

impl fmt::Debug for VirtualUsers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (ocupado, cap) = self.cache();
        f.debug_struct("UsuariosVirtuais")
            .field("ids", &self.ids)
            .field("cache", &format_args!("{ocupado}/{cap}"))
            .finish()
    }
}

struct ReadyCache {
    cap: usize,
    map: HashMap<String, String>,
    order: VecDeque<String>,
}

impl ReadyCache {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn register(&mut self, user_id: String, name: String) {
        if let Some(current) = self.map.get_mut(&user_id) {
            *current = name;
            return;
        }
        while self.map.len() >= self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            } else {
                break;
            }
        }
        self.order.push_back(user_id.clone());
        self.map.insert(user_id, name);
    }
}

fn client_body(identifier: &str) -> Result<String, GhostsError> {
    let raw = identifier.trim();
    if raw.is_empty() {
        return Err(GhostsError::Identifier {
            target: "cliente",
            reason: "vazio",
        });
    }
    let phone_formatting_only = raw
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | ' ' | '(' | ')' | '.' | '\u{a0}'));
    if phone_formatting_only {
        let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return Err(GhostsError::Identifier {
                target: "cliente",
                reason: "telefone sem dígito",
            });
        }
        if digits.chars().count() <= LOCALPART_BODY_LIMIT {
            return Ok(digits);
        }
        return opaque_body(&digits).ok_or(GhostsError::Identifier {
            target: "cliente",
            reason: "telefone sem dígito",
        });
    }
    opaque_body(raw).ok_or(GhostsError::Identifier {
        target: "cliente",
        reason: "sem caractere utilizável",
    })
}

fn opaque_body(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let base: String = raw
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '_' | '-' => Some(c),
            'A'..='Z' => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let summary = format!("{:016x}", fnv1a_digest(raw));
    if base.is_empty() {
        return Some(format!("h_{summary}"));
    }
    let short: String = base.chars().take(LOCALPART_BODY_LIMIT).collect();
    if short == raw {
        Some(short)
    } else {
        Some(format!("{short}_{summary}"))
    }
}

fn fnv1a_digest(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn colapsar(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> GhostIds {
        GhostIds::new("ph", "matrix-dev.local").expect("prefixo e servidor válidos")
    }

    fn valid_localpart(localpart: &str) -> bool {
        !localpart.is_empty()
            && localpart.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
            })
    }

    #[test]
    fn digits_only_client_gets_no_digest_suffix() {
        let ghost = ids().client("5511999999999").expect("telefone válido");
        assert_eq!(ghost.localpart(), "ph_wa_5511999999999");
        assert_eq!(ghost.user_id(), "@ph_wa_5511999999999:matrix-dev.local");
    }

    #[test]
    fn formatting_of_same_phone_lands_on_same_ghost() {
        let expected = ids()
            .client("5511999999999")
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
            let ghost = ids().client(variacao).expect("variação válida");
            assert_eq!(
                ghost.user_id(),
                expected,
                "formatação {variacao} criou outro fantasma"
            );
        }
    }

    #[test]
    fn different_phones_never_collide() {
        let base = ids().client("5511999999999").expect("telefone");
        for other in [
            "5511999999998",
            "5511989999999",
            "5521999999999",
            "511999999999",
            "55119999999990",
        ] {
            let ghost = ids().client(other).expect("telefone");
            assert_ne!(
                ghost.user_id(),
                base.user_id(),
                "telefone {other} colidiu com outro cliente"
            );
        }
    }

    #[test]
    fn identifier_with_invalid_character_yields_valid_unique_localpart() {
        let cases = [
            "120363012345678901@g.us",
            "5511ABC999",
            "55 11 99999-9999 ramal 2",
            "!!!",
            "cliente/sem/numero",
            "cliente:sem:numero",
        ];
        let mut seen = std::collections::BTreeSet::new();
        for case in cases {
            let ghost = ids()
                .client(case)
                .unwrap_or_else(|error| panic!("{case} deveria gerar id: {error}"));
            assert!(
                valid_localpart(ghost.localpart()),
                "{case} gerou localpart inválida: {}",
                ghost.localpart()
            );
            assert!(
                ghost.user_id().starts_with("@ph_wa_"),
                "{case} saiu do namespace: {}",
                ghost.user_id()
            );
            assert!(seen.insert(ghost.user_id().to_owned()), "{case} colidiu");
        }
    }

    #[test]
    fn agent_ids_with_invalid_character_do_not_collide() {
        let cases = [
            "Agente Um",
            "agente um",
            "AGENTE-UM",
            "agente_um",
            "agente.um",
        ];
        let mut seen = std::collections::BTreeSet::new();
        for case in cases {
            let ghost = ids().agent(case).expect("id de agente");
            assert!(valid_localpart(ghost.localpart()), "{case}");
            assert!(seen.insert(ghost.user_id().to_owned()), "{case} colidiu");
        }
    }

    #[test]
    fn agent_uuid_id_stays_readable() {
        let ghost = ids()
            .agent("01912345-6789-7abc-8def-0123456789ab")
            .expect("uuid");
        assert_eq!(
            ghost.user_id(),
            "@ph_ag_01912345-6789-7abc-8def-0123456789ab:matrix-dev.local"
        );
    }

    #[test]
    fn long_id_cut_at_cap_without_colliding_with_other_long_id() {
        let a = "z".repeat(200);
        let mut b = "z".repeat(200);
        b.push('1');
        let fa = ids().client(&a).expect("id longo");
        let fb = ids().client(&b).expect("id longo");
        let body = fa.localpart().trim_start_matches("ph_wa_");
        assert_eq!(body.chars().count(), LOCALPART_BODY_LIMIT + 1 + 16);
        assert!(valid_localpart(fa.localpart()));
        assert_ne!(fa.user_id(), fb.user_id(), "corte colidiu");
    }

    #[test]
    fn empty_identifier_is_refused_without_repeating_value() {
        for empty in ["", "   "] {
            let error = ids().client(empty).expect_err("vazio deve ser recusado");
            assert_eq!(
                error,
                GhostsError::Identifier {
                    target: "cliente",
                    reason: "vazio"
                }
            );
            assert!(!error.to_string().contains("   "));
        }
        assert!(ids().agent("").is_err());
        assert!(ids().device("").is_err());
    }

    #[test]
    fn phone_with_only_formatting_is_refused() {
        let error = ids().client("+() -.").expect_err("sem dígito");
        assert_eq!(
            error,
            GhostsError::Identifier {
                target: "cliente",
                reason: "telefone sem dígito"
            }
        );
    }

    #[test]
    fn device_ghost_builds_phase_2_id() {
        let ghost = ids().device("inst_1").expect("instância");
        assert_eq!(ghost.localpart(), "ph_ap_inst_1");
        assert_eq!(ghost.user_id(), "@ph_ap_inst_1:matrix-dev.local");
    }

    #[test]
    fn invalid_prefix_refused_naming_key_without_value() {
        for prefix in ["", "PH", "ph!", "abcdefghijklm"] {
            let error = GhostIds::new(prefix, "matrix-dev.local").expect_err(prefix);
            match error {
                GhostsError::Configuration { reason } => {
                    assert!(reason.contains("matrix.prefixo"), "{reason}");
                }
                other => panic!("esperado erro de configuração: {other:?}"),
            }
        }
        assert!(GhostIds::new("ph", "  ").is_err());
    }

    #[test]
    fn client_name_carries_suffix_and_does_not_repeat() {
        assert_eq!(client_display_name("Maria"), "Maria (WhatsApp)");
        assert_eq!(
            client_display_name("  Maria \n da  Silva "),
            "Maria da Silva (WhatsApp)"
        );
        assert_eq!(client_display_name(""), "Cliente (WhatsApp)");
        assert_eq!(client_display_name("   "), "Cliente (WhatsApp)");
        assert_eq!(client_display_name("Maria (WhatsApp)"), "Maria (WhatsApp)");
    }

    #[test]
    fn agent_name_gets_no_suffix_and_falls_back_to_id_when_empty() {
        let id = "01912345-6789-7abc-8def-0123456789ab";
        assert_eq!(agent_display_name("  Suporte  ", id), "Suporte");
        assert_eq!(agent_display_name("", id), id);
        assert_eq!(agent_display_name("   ", id), id);
    }

    #[test]
    fn cache_respects_cap_and_forgets_oldest() {
        let mut cache = ReadyCache::new(3);
        for index in 0..10 {
            cache.register(format!("@u{index}"), format!("nome {index}"));
            assert!(cache.map.len() <= 3, "passou do teto na inserção {index}");
        }
        assert_eq!(cache.map.len(), 3);
        assert_eq!(cache.map.get("@u9").map(String::as_str), Some("nome 9"));
        assert_eq!(cache.map.get("@u8").map(String::as_str), Some("nome 8"));
        assert_eq!(cache.map.get("@u7").map(String::as_str), Some("nome 7"));
        assert!(
            !cache.map.contains_key("@u0"),
            "o mais antigo deveria ter saído"
        );

        cache.register("@u9".to_owned(), "nome novo".to_owned());
        assert_eq!(cache.map.len(), 3);
        assert_eq!(cache.map.get("@u9").map(String::as_str), Some("nome novo"));
    }
}
