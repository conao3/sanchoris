use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::DateTime;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use tokio::sync::RwLock;

use super::{AuthError, Identity, IdentityVerifier};
use crate::config::CognitoConfig;

#[derive(serde::Deserialize)]
struct CognitoClaims {
    sub: String,
    email: Option<String>,
    #[serde(default, deserialize_with = "verified")]
    email_verified: bool,
    #[serde(rename = "cognito:username")]
    username: Option<String>,
    auth_time: Option<i64>,
    token_use: Option<String>,
    aud: Option<String>,
    client_id: Option<String>,
}

const JWKS_REFETCH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Jwks {
    set: Option<JwkSet>,
    fetched_at: Option<Instant>,
}

pub struct CognitoVerifier {
    config: CognitoConfig,
    region: String,
    jwks_url: String,
    http: reqwest::Client,
    jwks: RwLock<Jwks>,
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
        let jwks_url = format!("{}/.well-known/jwks.json", issuer(&region, &config));
        Ok(Self {
            config,
            region,
            jwks_url,
            http,
            jwks: RwLock::new(Jwks::default()),
        })
    }

    fn issuer(&self) -> String {
        issuer(&self.region, &self.config)
    }

    async fn key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let Some(set) = self.jwks.read().await.set.as_ref()
            && set.find(kid).is_some()
        {
            return decoding_key(set, kid);
        }
        let mut jwks = self.jwks.write().await;
        if let Some(set) = jwks.set.as_ref()
            && set.find(kid).is_some()
        {
            return decoding_key(set, kid);
        }
        if jwks
            .fetched_at
            .is_some_and(|at| at.elapsed() < JWKS_REFETCH_INTERVAL)
        {
            return Err(AuthError::Unauthorized);
        }
        jwks.fetched_at = Some(Instant::now());
        let url = &self.jwks_url;
        let set = self
            .http
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| AuthError::Internal(format!("fetch {url}: {error}")))?
            .json::<JwkSet>()
            .await
            .map_err(|error| AuthError::Internal(format!("parse {url}: {error}")))?;
        let key = decoding_key(&set, kid);
        jwks.set = Some(set);
        key
    }
}

fn issuer(region: &str, config: &CognitoConfig) -> String {
    format!(
        "https://cognito-idp.{region}.amazonaws.com/{}",
        config.user_pool_id
    )
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
            claims.email.as_deref(),
            claims.username.as_deref(),
            &claims.sub,
        );
        let auth_time = claims
            .auth_time
            .map(|seconds| DateTime::from_timestamp(seconds, 0).ok_or(AuthError::Unauthorized))
            .transpose()?;
        Ok(Identity {
            subject: claims.sub,
            email: claims.email,
            email_verified: claims.email_verified,
            display_name,
            auth_time,
        })
    }
}

fn verified<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Flag {
        Bool(bool),
        Text(String),
    }
    Ok(
        match <Flag as serde::Deserialize>::deserialize(deserializer)? {
            Flag::Bool(value) => value,
            Flag::Text(value) => value == "true",
        },
    )
}

fn display_name(email: Option<&str>, username: Option<&str>, subject: &str) -> String {
    let candidate = email
        .and_then(|value| value.split('@').next().map(str::to_string))
        .or_else(|| username.map(str::to_string))
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::{
        AuthError, CognitoClaims, CognitoConfig, CognitoVerifier, IdentityVerifier,
        JWKS_REFETCH_INTERVAL, display_name, region_of,
    };
    use serde_json::json;

    #[tokio::test]
    async fn an_unknown_key_id_fetches_the_jwks_at_most_once_a_minute() {
        let fetches = Arc::new(AtomicUsize::new(0));
        let counter = fetches.clone();
        let router = axum::Router::new().route(
            "/jwks.json",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { axum::Json(json!({ "keys": [] })) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut verifier = CognitoVerifier::new(
            CognitoConfig {
                user_pool_id: "ap-northeast-1_Test".into(),
                client_id: "client".into(),
            },
            reqwest::Client::new(),
        )
        .unwrap();
        verifier.jwks_url = format!("http://{address}/jwks.json");
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = Some("unknown".into());
        let token = jsonwebtoken::encode(
            &header,
            &json!({ "sub": "s" }),
            &jsonwebtoken::EncodingKey::from_secret(b"x"),
        )
        .unwrap();
        for _ in 0..5 {
            assert!(matches!(
                verifier.verify(&token).await,
                Err(AuthError::Unauthorized)
            ));
        }
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        verifier.jwks.write().await.fetched_at =
            Some(Instant::now() - JWKS_REFETCH_INTERVAL - Duration::from_secs(1));
        assert!(matches!(
            verifier.verify(&token).await,
            Err(AuthError::Unauthorized)
        ));
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn email_verified_reads_a_boolean_or_the_string_true() {
        let verified = |claim: Option<serde_json::Value>| {
            let mut claims = json!({ "sub": "s", "email": "a@sancode.dev" });
            if let Some(claim) = claim {
                claims["email_verified"] = claim;
            }
            serde_json::from_value::<CognitoClaims>(claims)
                .unwrap()
                .email_verified
        };
        assert!(verified(Some(json!(true))));
        assert!(verified(Some(json!("true"))));
        assert!(!verified(Some(json!(false))));
        assert!(!verified(Some(json!("false"))));
        assert!(!verified(None));
    }

    #[test]
    fn auth_time_is_read_when_the_token_has_it() {
        let auth_time = |claims: serde_json::Value| {
            serde_json::from_value::<CognitoClaims>(claims)
                .unwrap()
                .auth_time
        };
        assert_eq!(
            auth_time(json!({ "sub": "s", "auth_time": 1_790_000_000 })),
            Some(1_790_000_000)
        );
        assert_eq!(auth_time(json!({ "sub": "s" })), None);
    }

    #[test]
    fn the_region_comes_from_the_pool_id() {
        assert_eq!(region_of("ap-northeast-1_AbC").unwrap(), "ap-northeast-1");
        assert!(region_of("nopool").is_err());
        assert!(region_of("_abc").is_err());
    }

    #[test]
    fn display_names_prefer_the_email_then_the_username() {
        assert_eq!(
            display_name(Some("Mika.Dev@example.com"), Some("Google_1234"), "s"),
            "mika-dev"
        );
        assert_eq!(display_name(None, Some("Google_1234"), "s"), "google-1234");
        assert_eq!(display_name(None, None, "abc-123"), "abc-123");
    }
}
