use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const TASK_TOKEN_SIGNING_KEY_ENV: &str = "TASK_TOKEN_SIGNING_KEY";
pub const RUN_TOKEN_SIGNING_KEY_ENV: &str = "RUN_TOKEN_SIGNING_KEY";

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

    pub fn sign(&self, run_id: Uuid, expires_at: DateTime<Utc>) -> String {
        let message = format!("{run_id}.{}", expires_at.timestamp());
        let signature = self.0.mac(&message);
        format!("{message}.{signature}")
    }

    pub fn verify(&self, token: &str, now: DateTime<Utc>) -> Result<Uuid, TokenError> {
        let (message, signature) = token.rsplit_once('.').ok_or(TokenError::Malformed)?;
        self.0.verify(message, signature)?;
        let (run_id, expires) = message.split_once('.').ok_or(TokenError::Malformed)?;
        let expires_at = expires
            .parse::<i64>()
            .ok()
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
            .ok_or(TokenError::Malformed)?;
        if now >= expires_at {
            return Err(TokenError::Expired(expires_at));
        }
        run_id.parse().map_err(|_| TokenError::Malformed)
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
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap();
        let token = signer.sign(id, now + Duration::hours(1));
        assert_eq!(signer.verify(&token, now).unwrap(), id);
        assert!(matches!(
            signer.verify(&token, now + Duration::hours(1)),
            Err(TokenError::Expired(_))
        ));
    }

    #[test]
    fn short_keys_are_rejected() {
        assert_eq!(Signer::new(b"short").unwrap_err(), TokenError::ShortKey);
    }
}
