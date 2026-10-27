use async_trait::async_trait;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use tokio::sync::RwLock;

use super::{AuthError, Identity, IdentityVerifier};
use crate::config::CognitoConfig;

#[derive(serde::Deserialize)]
struct CognitoClaims {
    sub: String,
    email: Option<String>,
    #[serde(rename = "cognito:username")]
    username: Option<String>,
    token_use: Option<String>,
    aud: Option<String>,
    client_id: Option<String>,
}

pub struct CognitoVerifier {
    config: CognitoConfig,
    region: String,
    http: reqwest::Client,
    jwks: RwLock<Option<JwkSet>>,
}

pub fn region_of(user_pool_id: &str) -> Result<String, String> {
    user_pool_id
        .split_once('_')
        .map(|(region, _)| region)
        .filter(|region| !region.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{user_pool_id:?} is not <region>_<id>"))
}

impl CognitoVerifier {
    pub fn new(config: CognitoConfig, http: reqwest::Client) -> Result<Self, String> {
        let region = region_of(&config.user_pool_id)?;
        Ok(Self {
            config,
            region,
            http,
            jwks: RwLock::new(None),
        })
    }

    fn issuer(&self) -> String {
        format!(
            "https://cognito-idp.{}.amazonaws.com/{}",
            self.region, self.config.user_pool_id
        )
    }

    async fn key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let Some(set) = self.jwks.read().await.as_ref()
            && set.find(kid).is_some()
        {
            return decoding_key(set, kid);
        }
        let url = format!("{}/.well-known/jwks.json", self.issuer());
        let set = self
            .http
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| AuthError::Internal(format!("fetch {url}: {error}")))?
            .json::<JwkSet>()
            .await
            .map_err(|error| AuthError::Internal(format!("parse {url}: {error}")))?;
        let key = decoding_key(&set, kid)?;
        *self.jwks.write().await = Some(set);
        Ok(key)
    }
}

fn decoding_key(set: &JwkSet, kid: &str) -> Result<DecodingKey, AuthError> {
    let jwk = set.find(kid).ok_or(AuthError::Unauthorized)?;
    match &jwk.algorithm {
        AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
            .map_err(|error| AuthError::Internal(format!("invalid RSA JWK: {error}"))),
        _ => Err(AuthError::Unauthorized),
    }
}

#[async_trait]
impl IdentityVerifier for CognitoVerifier {
    async fn verify(&self, token: &str) -> Result<Identity, AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::Unauthorized)?;
        let kid = header.kid.ok_or(AuthError::Unauthorized)?;
        let key = self.key(&kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        validation.set_issuer(&[self.issuer()]);
        let claims = decode::<CognitoClaims>(token, &key, &validation)
            .map_err(|_| AuthError::Unauthorized)?
            .claims;
        let audience = match claims.token_use.as_deref() {
            Some("access") => claims.client_id.as_deref(),
            Some("id") => claims.aud.as_deref(),
            _ => None,
        };
        if audience != Some(self.config.client_id.as_str()) {
            return Err(AuthError::Unauthorized);
        }
        let display_name = display_name(
            claims.username.as_deref(),
            claims.email.as_deref(),
            &claims.sub,
        );
        Ok(Identity {
            subject: claims.sub,
            email: claims.email,
            display_name,
        })
    }
}

fn display_name(username: Option<&str>, email: Option<&str>, subject: &str) -> String {
    let candidate = username
        .map(str::to_string)
        .or_else(|| email.and_then(|value| value.split('@').next().map(str::to_string)))
        .unwrap_or_else(|| subject.to_string());
    let slug = slugify(&candidate);
    if slug.is_empty() {
        slugify(subject)
    } else {
        slug
    }
}

fn slugify(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            result.push('-');
            last_dash = true;
        }
    }
    result.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::{display_name, region_of};

    #[test]
    fn the_region_comes_from_the_pool_id() {
        assert_eq!(region_of("ap-northeast-1_AbC").unwrap(), "ap-northeast-1");
        assert!(region_of("nopool").is_err());
        assert!(region_of("_abc").is_err());
    }

    #[test]
    fn display_names_prefer_the_username_then_the_email() {
        assert_eq!(display_name(Some("Google_1234"), None, "s"), "google-1234");
        assert_eq!(
            display_name(None, Some("Mika.Dev@example.com"), "s"),
            "mika-dev"
        );
        assert_eq!(display_name(None, None, "abc-123"), "abc-123");
    }
}
