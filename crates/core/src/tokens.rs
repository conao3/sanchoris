use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const TASK_TOKEN_SIGNING_KEY_ENV: &str = "TASK_TOKEN_SIGNING_KEY";
pub const RUN_TOKEN_SIGNING_KEY_ENV: &str = "RUN_TOKEN_SIGNING_KEY";
pub const RELAY_TOKEN_SIGNING_KEY_ENV: &str = "RELAY_TOKEN_SIGNING_KEY";
pub const RUN_TOKEN_HOURS: i64 = 12;
pub const BOOTSTRAP_TOKEN_MINUTES: i64 = 15;
const RUN_TOKEN_KIND: &str = "run";
const BOOTSTRAP_TOKEN_KIND: &str = "bootstrap";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("the signing key must be at least 32 bytes")]
    ShortKey,
    #[error("the token is malformed")]
    Malformed,
    #[error("the token signature does not match")]
    BadSignature,
    #[error("the token expired at {0}")]
    Expired(DateTime<Utc>),
}

#[derive(Clone)]
pub struct Signer {
    key: Vec<u8>,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Signer(..)")
    }
}

impl Signer {
    pub fn new(key: &[u8]) -> Result<Self, TokenError> {
        if key.len() < 32 {
            return Err(TokenError::ShortKey);
        }
        Ok(Self { key: key.to_vec() })
    }

    pub fn from_env(name: &str) -> Self {
        let value = std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set"));
        Self::new(value.as_bytes()).unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    fn mac(&self, message: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(message.as_bytes());
        hex(&mac.finalize().into_bytes())
    }

    fn verify(&self, message: &str, signature: &str) -> Result<(), TokenError> {
        let bytes = unhex(signature).ok_or(TokenError::Malformed)?;
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(message.as_bytes());
        mac.verify_slice(&bytes)
            .map_err(|_| TokenError::BadSignature)
    }
}

#[derive(Debug, Clone)]
pub struct TaskTokenSigner(Signer);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedTaskToken {
    pub skill_run_id: Uuid,
    pub task_token: String,
}

impl TaskTokenSigner {
    pub fn new(signer: Signer) -> Self {
        Self(signer)
    }

    pub fn from_env() -> Self {
        Self(Signer::from_env(TASK_TOKEN_SIGNING_KEY_ENV))
    }

    pub fn sign(&self, skill_run_id: Uuid, task_token: &str) -> String {
        let message = format!("{skill_run_id}.{task_token}");
        let signature = self.0.mac(&message);
        format!("{message}.{signature}")
    }

    pub fn verify(&self, signed: &str) -> Result<VerifiedTaskToken, TokenError> {
        let (message, signature) = signed.rsplit_once('.').ok_or(TokenError::Malformed)?;
        self.0.verify(message, signature)?;
        let (skill_run_id, task_token) = message.split_once('.').ok_or(TokenError::Malformed)?;
        Ok(VerifiedTaskToken {
            skill_run_id: skill_run_id.parse().map_err(|_| TokenError::Malformed)?,
            task_token: task_token.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct RunTokenSigner(Signer);

impl RunTokenSigner {
    pub fn new(signer: Signer) -> Self {
        Self(signer)
    }

    pub fn from_env() -> Self {
        Self(Signer::from_env(RUN_TOKEN_SIGNING_KEY_ENV))
    }

    fn sign_parts(&self, kind: &str, ids: &[Uuid], expires_at: DateTime<Utc>) -> String {
        let mut message = kind.to_string();
        for id in ids {
            message.push_str(&format!(".{id}"));
        }
        message.push_str(&format!(".{}", expires_at.timestamp()));
        let signature = self.0.mac(&message);
        format!("{message}.{signature}")
    }

    fn verify_parts(
        &self,
        token: &str,
        kind: &str,
        ids: usize,
        now: DateTime<Utc>,
    ) -> Result<Vec<Uuid>, TokenError> {
        let (message, signature) = token.rsplit_once('.').ok_or(TokenError::Malformed)?;
        let parts = message.split('.').collect::<Vec<_>>();
        if parts.len() != ids + 2 || parts[0] != kind {
            return Err(TokenError::Malformed);
        }
        self.0.verify(message, signature)?;
        let expires_at = parts[ids + 1]
            .parse::<i64>()
            .ok()
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
            .ok_or(TokenError::Malformed)?;
        if now >= expires_at {
            return Err(TokenError::Expired(expires_at));
        }
        parts[1..=ids]
            .iter()
            .map(|id| id.parse().map_err(|_| TokenError::Malformed))
            .collect()
    }

    pub fn sign(&self, tenant_id: Uuid, run_id: Uuid, expires_at: DateTime<Utc>) -> String {
        self.sign_parts(RUN_TOKEN_KIND, &[tenant_id, run_id], expires_at)
    }

    pub fn verify(&self, token: &str, now: DateTime<Utc>) -> Result<RunClaims, TokenError> {
        let ids = self.verify_parts(token, RUN_TOKEN_KIND, 2, now)?;
        Ok(RunClaims {
            tenant_id: ids[0],
            run_id: ids[1],
        })
    }

    pub fn sign_bootstrap(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
        workspace_id: Uuid,
        expires_at: DateTime<Utc>,
    ) -> String {
        self.sign_parts(
            BOOTSTRAP_TOKEN_KIND,
            &[tenant_id, run_id, workspace_id],
            expires_at,
        )
    }

    pub fn verify_bootstrap(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<BootstrapClaims, TokenError> {
        let ids = self.verify_parts(token, BOOTSTRAP_TOKEN_KIND, 3, now)?;
        Ok(BootstrapClaims {
            tenant_id: ids[0],
            run_id: ids[1],
            workspace_id: ids[2],
        })
    }

    pub fn verify_channel(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<ChannelClaims, TokenError> {
        match token.split_once('.') {
            Some((BOOTSTRAP_TOKEN_KIND, _)) => self
                .verify_bootstrap(token, now)
                .map(ChannelClaims::Bootstrap),
            _ => self.verify(token, now).map(ChannelClaims::Run),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RelayTokenSigner(Signer);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RelayClaims {
    pub run: Uuid,
    pub exp: i64,
    pub allow: Vec<String>,
}

impl RelayTokenSigner {
    pub fn new(signer: Signer) -> Self {
        Self(signer)
    }

    pub fn sign(&self, run_id: Uuid, allow: &[String], expires_at: DateTime<Utc>) -> String {
        let claims = RelayClaims {
            run: run_id,
            exp: expires_at.timestamp(),
            allow: allow.to_vec(),
        };
        let json = serde_json::to_string(&claims).expect("relay claims serialize");
        let message = hex(json.as_bytes());
        let signature = self.0.mac(&message);
        format!("{message}.{signature}")
    }

    pub fn verify(&self, token: &str, now: DateTime<Utc>) -> Result<RelayClaims, TokenError> {
        let (message, signature) = token.split_once('.').ok_or(TokenError::Malformed)?;
        self.0.verify(message, signature)?;
        let claims: RelayClaims = unhex(message)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or(TokenError::Malformed)?;
        let expires_at = DateTime::from_timestamp(claims.exp, 0).ok_or(TokenError::Malformed)?;
        if now >= expires_at {
            return Err(TokenError::Expired(expires_at));
        }
        Ok(claims)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunClaims {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootstrapClaims {
    pub tenant_id: Uuid,
    pub run_id: Uuid,
    pub workspace_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelClaims {
    Run(RunClaims),
    Bootstrap(BootstrapClaims),
}

impl ChannelClaims {
    pub fn run(self) -> RunClaims {
        match self {
            Self::Run(claims) => claims,
            Self::Bootstrap(claims) => RunClaims {
                tenant_id: claims.tenant_id,
                run_id: claims.run_id,
            },
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn signer() -> Signer {
        Signer::new(&[7u8; 32]).unwrap()
    }

    #[test]
    fn task_tokens_round_trip_and_reject_tampering() {
        let signer = TaskTokenSigner::new(signer());
        let id = Uuid::new_v4();
        let signed = signer.sign(id, "sanchoris-task:1:4:1");
        assert_eq!(
            signer.verify(&signed).unwrap(),
            VerifiedTaskToken {
                skill_run_id: id,
                task_token: "sanchoris-task:1:4:1".into()
            }
        );
        let tampered = signed.replace("sanchoris-task:1:4:1", "sanchoris-task:1:5:1");
        assert_eq!(signer.verify(&tampered), Err(TokenError::BadSignature));
        let other = TaskTokenSigner::new(Signer::new(&[8u8; 32]).unwrap());
        assert_eq!(other.verify(&signed), Err(TokenError::BadSignature));
    }

    #[test]
    fn run_tokens_expire() {
        let signer = RunTokenSigner::new(signer());
        let id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let token = signer.sign(tenant_id, id, now + Duration::hours(1));
        assert_eq!(
            signer.verify(&token, now).unwrap(),
            RunClaims {
                tenant_id,
                run_id: id
            }
        );
        assert!(matches!(
            signer.verify(&token, now + Duration::hours(1)),
            Err(TokenError::Expired(_))
        ));
    }

    #[test]
    fn bootstrap_tokens_and_run_tokens_are_never_taken_for_each_other() {
        let signer = RunTokenSigner::new(signer());
        let (tenant_id, run_id, workspace_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let bootstrap =
            signer.sign_bootstrap(tenant_id, run_id, workspace_id, now + Duration::minutes(15));
        let run = signer.sign(tenant_id, run_id, now + Duration::hours(12));
        let claims = BootstrapClaims {
            tenant_id,
            run_id,
            workspace_id,
        };
        assert_eq!(signer.verify_bootstrap(&bootstrap, now).unwrap(), claims);
        assert_eq!(signer.verify(&bootstrap, now), Err(TokenError::Malformed));
        assert_eq!(
            signer.verify_bootstrap(&run, now),
            Err(TokenError::Malformed)
        );
        assert_eq!(
            signer.verify_channel(&bootstrap, now).unwrap(),
            ChannelClaims::Bootstrap(claims)
        );
        assert_eq!(
            signer.verify_channel(&run, now).unwrap(),
            ChannelClaims::Run(RunClaims { tenant_id, run_id })
        );
        assert!(matches!(
            signer.verify_bootstrap(&bootstrap, now + Duration::minutes(15)),
            Err(TokenError::Expired(_))
        ));
        let relabeled = bootstrap.replacen("bootstrap.", "run.", 1);
        assert!(signer.verify_channel(&relabeled, now).is_err());
        let other = RunTokenSigner::new(Signer::new(&[8u8; 32]).unwrap());
        assert_eq!(
            other.verify_bootstrap(&bootstrap, now),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn relay_tokens_carry_the_allow_list_in_the_format_the_relay_verifies() {
        let signer = RelayTokenSigner::new(signer());
        let run_id: Uuid = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let allow = vec!["github.com".to_string(), "registry.npmjs.org".to_string()];
        let expires_at = DateTime::from_timestamp(1_793_061_000, 0).unwrap();
        let token = signer.sign(run_id, &allow, expires_at);
        assert_eq!(
            token,
            "7b2272756e223a2230303030303030302d303030302d343030302d383030302d303030303030303030303031222c22657870223a313739333036313030302c22616c6c6f77223a5b226769746875622e636f6d222c2272656769737472792e6e706d6a732e6f7267225d7d.059da54d7959fbfa409a87da6f695f81b67dee7f2fdc33d80d0b7299b5b95200"
        );
        let now = expires_at - Duration::minutes(1);
        assert_eq!(
            signer.verify(&token, now).unwrap(),
            RelayClaims {
                run: run_id,
                exp: 1_793_061_000,
                allow
            }
        );
        assert!(matches!(
            signer.verify(&token, expires_at),
            Err(TokenError::Expired(_))
        ));
        let other = RelayTokenSigner::new(Signer::new(&[8u8; 32]).unwrap());
        assert_eq!(other.verify(&token, now), Err(TokenError::BadSignature));
    }

    #[test]
    fn short_keys_are_rejected() {
        assert_eq!(Signer::new(b"short").unwrap_err(), TokenError::ShortKey);
    }
}
