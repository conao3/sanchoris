use crate::auth::jwks;
use crate::env::{self, BackendEnv};
use jsonwebtoken::{Algorithm, Validation, decode, decode_header};

#[derive(Debug)]
pub enum AuthError {
    Unauthorized,
    Internal(String),
}

#[derive(serde::Deserialize)]
pub struct CognitoClaims {
    pub sub: String,
    pub email: Option<String>,
    #[serde(rename = "cognito:username")]
    pub username: Option<String>,
    pub token_use: Option<String>,
    pub aud: Option<String>,
    pub client_id: Option<String>,
}

pub struct AuthIdentity {
    pub sub: String,
    pub email: Option<String>,
    pub username: String,
}

pub async fn verify_token(token: &str, env: &BackendEnv) -> Result<AuthIdentity, AuthError> {
    let region = env::cognito_region(&env.cognito_user_pool_id);

    let header = decode_header(token).map_err(|_| AuthError::Unauthorized)?;
    let kid = header.kid.ok_or(AuthError::Unauthorized)?;
    let decoding_key = jwks::get_decoding_key(&kid, &region, &env.cognito_user_pool_id).await?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_aud = false;
    validation.set_issuer(&[format!(
        "https://cognito-idp.{}.amazonaws.com/{}",
        region, env.cognito_user_pool_id
    )]);

    let token_data = decode::<CognitoClaims>(token, &decoding_key, &validation)
        .map_err(|_| AuthError::Unauthorized)?;
    let claims = token_data.claims;

    match claims.token_use.as_deref() {
        Some("access") => {
            if claims.client_id.as_deref() != Some(env.cognito_client_id.as_str()) {
                return Err(AuthError::Unauthorized);
            }
        }
        Some("id") => {
            if claims.aud.as_deref() != Some(env.cognito_client_id.as_str()) {
                return Err(AuthError::Unauthorized);
            }
        }
        _ => return Err(AuthError::Unauthorized),
    }

    let username = username_from_identity(claims.username.as_deref(), claims.email.as_deref(), &claims.sub);

    Ok(AuthIdentity {
        sub: claims.sub,
        email: claims.email,
        username,
    })
}

fn username_from_identity(username: Option<&str>, email: Option<&str>, sub: &str) -> String {
    let candidate = username
        .map(str::to_string)
        .or_else(|| email.and_then(|value| value.split('@').next().map(str::to_string)))
        .unwrap_or_else(|| sub.to_string());
    let slug = slugify(&candidate);
    if slug.is_empty() {
        slugify(sub)
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
