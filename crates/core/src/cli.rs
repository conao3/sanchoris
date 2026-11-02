use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::DbError;
use crate::item::{Entity, ItemReader, ItemWriter, Key};
use crate::keys;

pub const CLI_REQUEST: &str = "cli_request";
pub const DEVICE_CODE_SECONDS: i64 = 600;
pub const ACCESS_TOKEN_SECONDS: i64 = 600;
pub const POLL_INTERVAL_SECONDS: i64 = 5;
pub const SLOW_DOWN_SECONDS: i64 = 5;
pub const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";
pub const USER_CODE_LENGTH: usize = 8;

pub const PENDING: &str = "pending";
pub const APPROVED: &str = "approved";
pub const DENIED: &str = "denied";
pub const ISSUED: &str = "issued";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliPurpose {
    ClaudeCredential,
    CodexCredential,
}

impl CliPurpose {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "claude-credential" => Some(Self::ClaudeCredential),
            "codex-credential" => Some(Self::CodexCredential),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCredential => "claude-credential",
            Self::CodexCredential => "codex-credential",
        }
    }

    pub fn provider(self) -> &'static str {
        match self {
            Self::ClaudeCredential => "claude",
            Self::CodexCredential => "codex",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CliRequest {
    pub id: Uuid,
    pub user_code: String,
    pub client: String,
    pub purpose: String,
    pub requested_scope: String,
    pub device_code_hash: String,
    pub status: String,
    pub interval_seconds: i64,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub code_expires_at: DateTime<Utc>,
    pub user_id: Option<Uuid>,
    pub tenant_id: Option<Uuid>,
    pub member_id: Option<Uuid>,
    pub scope: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub token_hash: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
}

impl CliRequest {
    pub fn purpose(&self) -> Result<CliPurpose, DbError> {
        CliPurpose::parse(&self.purpose)
            .ok_or_else(|| DbError::Decode(format!("cli request purpose {}", self.purpose)))
    }

    pub fn is_expired(&self, at: DateTime<Utc>) -> bool {
        self.status != ISSUED && at >= self.code_expires_at
    }

    fn purge_at(&self) -> DateTime<Utc> {
        self.token_expires_at
            .map_or(self.code_expires_at, |at| at.max(self.code_expires_at))
    }
}

impl Entity for CliRequest {
    const KIND: &'static str = CLI_REQUEST;

    fn key(&self) -> Key {
        cli_request_key(&self.user_code)
    }

    fn write(&self, w: &mut ItemWriter) {
        w.uuid("id", self.id)
            .s("user_code", &self.user_code)
            .s("client", &self.client)
            .s("purpose", &self.purpose)
            .s("requested_scope", &self.requested_scope)
            .s("device_code_hash", &self.device_code_hash)
            .s("status", &self.status)
            .n("interval_seconds", self.interval_seconds)
            .opt_time("last_polled_at", self.last_polled_at)
            .time("created_at", self.created_at)
            .time("code_expires_at", self.code_expires_at)
            .opt_uuid("user_id", self.user_id)
            .opt_uuid("tenant_id", self.tenant_id)
            .opt_uuid("member_id", self.member_id)
            .opt_s("scope", self.scope.as_deref())
            .opt_time("decided_at", self.decided_at)
            .opt_s("token_hash", self.token_hash.as_deref())
            .opt_time("token_expires_at", self.token_expires_at)
            .ttl(self.purge_at());
    }

    fn read(r: &ItemReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: r.uuid("id")?,
            user_code: r.s("user_code")?,
            client: r.s("client")?,
            purpose: r.s("purpose")?,
            requested_scope: r.s("requested_scope")?,
            device_code_hash: r.s("device_code_hash")?,
            status: r.s("status")?,
            interval_seconds: r.n("interval_seconds")?,
            last_polled_at: r.opt_time("last_polled_at")?,
            created_at: r.time("created_at")?,
            code_expires_at: r.time("code_expires_at")?,
            user_id: r.opt_uuid("user_id")?,
            tenant_id: r.opt_uuid("tenant_id")?,
            member_id: r.opt_uuid("member_id")?,
            scope: r.opt_s("scope")?,
            decided_at: r.opt_time("decided_at")?,
            token_hash: r.opt_s("token_hash")?,
            token_expires_at: r.opt_time("token_expires_at")?,
        })
    }
}

pub fn cli_request_key(user_code: &str) -> Key {
    keys::uniq_key(CLI_REQUEST, user_code)
}

pub fn new_request(
    client: &str,
    purpose: CliPurpose,
    requested_scope: &str,
    at: DateTime<Utc>,
) -> (CliRequest, String) {
    let user_code = new_user_code();
    let secret = new_secret();
    let request = CliRequest {
        id: Uuid::new_v4(),
        user_code: user_code.clone(),
        client: client.to_string(),
        purpose: purpose.as_str().to_string(),
        requested_scope: requested_scope.to_string(),
        device_code_hash: hash_secret(&secret),
        status: PENDING.into(),
        interval_seconds: POLL_INTERVAL_SECONDS,
        last_polled_at: None,
        created_at: at,
        code_expires_at: at + Duration::seconds(DEVICE_CODE_SECONDS),
        user_id: None,
        tenant_id: None,
        member_id: None,
        scope: None,
        decided_at: None,
        token_hash: None,
        token_expires_at: None,
    };
    (request, format!("{user_code}.{secret}"))
}

pub fn issue_token(request: &mut CliRequest, at: DateTime<Utc>) -> String {
    let secret = new_secret();
    request.status = ISSUED.into();
    request.token_hash = Some(hash_secret(&secret));
    request.token_expires_at = Some(at + Duration::seconds(ACCESS_TOKEN_SECONDS));
    format!("{}.{secret}", request.user_code)
}

pub fn split_code(code: &str) -> Option<(String, &str)> {
    let (user_code, secret) = code.trim().split_once('.')?;
    let user_code = normalize_user_code(user_code)?;
    (secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some((user_code, secret))
}

pub fn secret_matches(secret: &str, hash: &str) -> bool {
    let actual = hash_secret(secret);
    actual.len() == hash.len()
        && actual
            .bytes()
            .zip(hash.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

pub fn normalize_user_code(text: &str) -> Option<String> {
    let code = text
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect::<String>();
    (code.len() == USER_CODE_LENGTH && code.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)))
        .then_some(code)
}

pub fn display_user_code(code: &str) -> String {
    let (head, tail) = code.split_at(code.len() / 2);
    format!("{head}-{tail}")
}

fn new_user_code() -> String {
    let mut code = String::with_capacity(USER_CODE_LENGTH);
    while code.len() < USER_CODE_LENGTH {
        for byte in Uuid::new_v4().into_bytes() {
            let limit = 256 - 256 % USER_CODE_ALPHABET.len();
            if (byte as usize) < limit && code.len() < USER_CODE_LENGTH {
                code.push(USER_CODE_ALPHABET[byte as usize % USER_CODE_ALPHABET.len()] as char);
            }
        }
    }
    code
}

fn new_secret() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    let digest = Sha256::digest(&bytes);
    hex(&digest)
}

fn hash_secret(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_use_the_unambiguous_consonants() {
        for _ in 0..200 {
            let code = new_user_code();
            assert_eq!(normalize_user_code(&code), Some(code.clone()));
        }
        assert_eq!(
            normalize_user_code(" wdjb-mjht "),
            Some("WDJBMJHT".to_string())
        );
        assert_eq!(normalize_user_code("WDJB-MJH"), None);
        assert_eq!(normalize_user_code("WDJB-MJHA"), None);
        assert_eq!(display_user_code("WDJBMJHT"), "WDJB-MJHT");
    }

    #[test]
    fn codes_carry_the_user_code_and_a_secret_that_only_its_hash_matches() {
        let at = Utc::now();
        let (mut request, device_code) =
            new_request("sancli", CliPurpose::ClaudeCredential, "personal", at);
        let (user_code, secret) = split_code(&device_code).unwrap();
        assert_eq!(user_code, request.user_code);
        assert!(secret_matches(secret, &request.device_code_hash));
        assert!(!secret_matches(&"0".repeat(64), &request.device_code_hash));
        let token = issue_token(&mut request, at);
        let (_, secret) = split_code(&token).unwrap();
        assert!(secret_matches(
            secret,
            request.token_hash.as_deref().unwrap()
        ));
        assert!(split_code("WDJBMJHT.short").is_none());
        assert_eq!(
            request.purge_at(),
            at + Duration::seconds(ACCESS_TOKEN_SECONDS)
        );
    }
}
