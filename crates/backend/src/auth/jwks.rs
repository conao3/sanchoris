use crate::auth::verify::AuthError;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use std::sync::OnceLock;
use tokio::sync::RwLock;

static JWKS_CACHE: OnceLock<RwLock<Option<JwkSet>>> = OnceLock::new();

fn cache() -> &'static RwLock<Option<JwkSet>> {
    JWKS_CACHE.get_or_init(|| RwLock::new(None))
}

pub async fn get_decoding_key(
    kid: &str,
    region: &str,
    user_pool_id: &str,
) -> Result<DecodingKey, AuthError> {
    if let Some(key) = lookup_cached(kid).await? {
        return Ok(key);
    }

    let fetched = fetch_jwks(region, user_pool_id).await?;
    let key = decoding_key_for(&fetched, kid)?;
    {
        let mut guard = cache().write().await;
        *guard = Some(fetched);
    }
    Ok(key)
}

async fn lookup_cached(kid: &str) -> Result<Option<DecodingKey>, AuthError> {
    let guard = cache().read().await;
    match guard.as_ref() {
        Some(set) => match set.find(kid) {
            Some(_) => decoding_key_for(set, kid).map(Some),
            None => Ok(None),
        },
        None => Ok(None),
    }
}

async fn fetch_jwks(region: &str, user_pool_id: &str) -> Result<JwkSet, AuthError> {
    let url = format!(
        "https://cognito-idp.{region}.amazonaws.com/{user_pool_id}/.well-known/jwks.json"
    );
    let response = reqwest::get(&url)
        .await
        .map_err(|error| AuthError::Internal(format!("failed to fetch JWKS: {error}")))?;
    response
        .json::<JwkSet>()
        .await
        .map_err(|error| AuthError::Internal(format!("failed to parse JWKS: {error}")))
}

fn decoding_key_for(set: &JwkSet, kid: &str) -> Result<DecodingKey, AuthError> {
    let jwk = set.find(kid).ok_or(AuthError::Unauthorized)?;
    match &jwk.algorithm {
        AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
            .map_err(|error| AuthError::Internal(format!("invalid RSA JWK: {error}"))),
        _ => Err(AuthError::Unauthorized),
    }
}
