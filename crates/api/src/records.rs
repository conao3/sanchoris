use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use aws_sdk_s3::presigning::PresigningConfig;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use chrono::Utc;
use sanchoris_core::aws::S3BlobStore;
use sanchoris_core::local::DirBlobStore;
use sanchoris_core::ports::BlobStore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::config::{ConfigError, RecordsConfig};
use crate::signing::HmacKey;

const URL_TTL_SECONDS: i64 = 900;
pub const MAX_RECORD_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum RecordError {
    #[error("invalid record key: {0}")]
    InvalidKey(String),
    #[error("record storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresignedPut {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[async_trait]
pub trait RecordStore: Send + Sync {
    async fn presign_put(
        &self,
        tenant_id: Uuid,
        key: &str,
        content_length: i64,
        sha256: &str,
    ) -> Result<PresignedPut, RecordError>;
    async fn presign_get(&self, tenant_id: Uuid, key: &str) -> Result<String, RecordError>;
    async fn put(&self, tenant_id: Uuid, key: &str, body: Vec<u8>) -> Result<(), RecordError>;
}

pub fn audit_export_key(tenant_id: Uuid, file_name: &str) -> Result<String, RecordError> {
    if !is_segment(file_name) {
        return Err(RecordError::InvalidKey(format!("file name {file_name:?}")));
    }
    Ok(format!("t/{tenant_id}/audit/{file_name}"))
}

fn is_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn record_key(
    tenant_id: Uuid,
    run_id: Uuid,
    session_id: &str,
    turn: i64,
) -> Result<String, RecordError> {
    if !is_segment(session_id) {
        return Err(RecordError::InvalidKey(format!(
            "session id {session_id:?}"
        )));
    }
    Ok(format!(
        "t/{tenant_id}/runs/{run_id}/{session_id}/turn-{turn}.jsonl"
    ))
}

fn tenant_prefix(tenant_id: Uuid) -> String {
    format!("t/{tenant_id}/")
}

fn check_key(tenant_id: Uuid, key: &str) -> Result<(), RecordError> {
    if !key.starts_with(&tenant_prefix(tenant_id)) || !key.split('/').all(is_segment) {
        return Err(RecordError::InvalidKey(key.to_string()));
    }
    Ok(())
}

pub struct S3RecordStore {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl S3RecordStore {
    pub fn new(config: &aws_config::SdkConfig, bucket: &str) -> Self {
        Self {
            client: aws_sdk_s3::Client::new(config),
            bucket: bucket.to_string(),
        }
    }

    fn presigning() -> Result<PresigningConfig, RecordError> {
        PresigningConfig::expires_in(Duration::from_secs(URL_TTL_SECONDS as u64))
            .map_err(|error| RecordError::Storage(error.to_string()))
    }
}

#[async_trait]
impl RecordStore for S3RecordStore {
    async fn presign_put(
        &self,
        tenant_id: Uuid,
        key: &str,
        content_length: i64,
        sha256: &str,
    ) -> Result<PresignedPut, RecordError> {
        check_key(tenant_id, key)?;
        let digest = hex::decode(sha256)
            .map_err(|_| RecordError::InvalidKey(format!("sha256 {sha256:?}")))?;
        let request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_length(content_length)
            .checksum_sha256(base64::engine::general_purpose::STANDARD.encode(digest))
            .presigned(Self::presigning()?)
            .await
            .map_err(|error| RecordError::Storage(error.to_string()))?;
        Ok(PresignedPut {
            url: request.uri().to_string(),
            headers: request
                .headers()
                .filter(|(name, _)| !name.eq_ignore_ascii_case("content-length"))
                .map(|(name, value)| (name.to_ascii_lowercase(), value.to_string()))
                .collect(),
        })
    }

    async fn presign_get(&self, tenant_id: Uuid, key: &str) -> Result<String, RecordError> {
        check_key(tenant_id, key)?;
        let request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(Self::presigning()?)
            .await
            .map_err(|error| RecordError::Storage(error.to_string()))?;
        Ok(request.uri().to_string())
    }

    async fn put(&self, tenant_id: Uuid, key: &str, body: Vec<u8>) -> Result<(), RecordError> {
        check_key(tenant_id, key)?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body.into())
            .send()
            .await
            .map_err(|error| RecordError::Storage(error.to_string()))?;
        Ok(())
    }
}

pub fn s3_records(
    config: &RecordsConfig,
    sdk: &aws_config::SdkConfig,
) -> Result<Arc<dyn RecordStore>, ConfigError> {
    match config {
        RecordsConfig::S3 { bucket } => Ok(Arc::new(S3RecordStore::new(sdk, bucket))),
        RecordsConfig::Local { .. } => Err(ConfigError::Invalid {
            name: "SANCHORIS_RECORDS_STORE",
            message: "Lambda functions store records in S3; set it to s3".into(),
        }),
    }
}

pub fn record_blobs(config: &RecordsConfig, sdk: &aws_config::SdkConfig) -> Arc<dyn BlobStore> {
    match config {
        RecordsConfig::S3 { bucket } => Arc::new(S3BlobStore::new(
            aws_sdk_s3::Client::new(sdk),
            bucket.clone(),
        )),
        RecordsConfig::Local { dir, .. } => Arc::new(DirBlobStore::new(dir.clone())),
    }
}

pub struct LocalRecordStore {
    dir: PathBuf,
    key: HmacKey,
    public_url: String,
}

fn put_message(key: &str, expires: i64, length: i64, sha256: &str) -> String {
    format!("PUT\n{key}\n{expires}\n{length}\n{sha256}")
}

fn get_message(key: &str, expires: i64) -> String {
    format!("GET\n{key}\n{expires}")
}

impl LocalRecordStore {
    pub fn new(dir: &Path, key: HmacKey, public_url: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            key,
            public_url: public_url.trim_end_matches('/').to_string(),
        }
    }

    fn path(&self, key: &str) -> Result<PathBuf, RecordError> {
        if !key.split('/').all(is_segment) {
            return Err(RecordError::InvalidKey(key.to_string()));
        }
        Ok(self.dir.join(key))
    }

    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/{*key}", get(get_record).put(put_record))
            .layer(DefaultBodyLimit::max(MAX_RECORD_BYTES as usize))
            .with_state(self)
    }
}

#[async_trait]
impl RecordStore for LocalRecordStore {
    async fn presign_put(
        &self,
        tenant_id: Uuid,
        key: &str,
        content_length: i64,
        sha256: &str,
    ) -> Result<PresignedPut, RecordError> {
        check_key(tenant_id, key)?;
        let expires = Utc::now().timestamp() + URL_TTL_SECONDS;
        let signature = self
            .key
            .hex(put_message(key, expires, content_length, sha256).as_bytes());
        Ok(PresignedPut {
            url: format!(
                "{}/records/{key}?expires={expires}&length={content_length}&sha256={sha256}&signature={signature}",
                self.public_url
            ),
            headers: BTreeMap::from([(
                "content-type".to_string(),
                "application/octet-stream".to_string(),
            )]),
        })
    }

    async fn presign_get(&self, tenant_id: Uuid, key: &str) -> Result<String, RecordError> {
        check_key(tenant_id, key)?;
        let expires = Utc::now().timestamp() + URL_TTL_SECONDS;
        let signature = self.key.hex(get_message(key, expires).as_bytes());
        Ok(format!(
            "{}/records/{key}?expires={expires}&signature={signature}",
            self.public_url
        ))
    }

    async fn put(&self, tenant_id: Uuid, key: &str, body: Vec<u8>) -> Result<(), RecordError> {
        check_key(tenant_id, key)?;
        let path = self.path(key)?;
        let parent = path.parent().expect("record paths have a parent");
        let partial = path.with_extension("partial");
        let written = async {
            tokio::fs::create_dir_all(parent).await?;
            tokio::fs::write(&partial, &body).await?;
            tokio::fs::rename(&partial, &path).await
        }
        .await;
        written.map_err(|error| RecordError::Storage(format!("write {key}: {error}")))
    }
}

#[derive(Deserialize)]
struct PutQuery {
    expires: i64,
    length: i64,
    sha256: String,
    signature: String,
}

#[derive(Deserialize)]
struct GetQuery {
    expires: i64,
    signature: String,
}

fn denied(message: &str) -> Response {
    (StatusCode::FORBIDDEN, message.to_string()).into_response()
}

async fn put_record(
    State(store): State<Arc<LocalRecordStore>>,
    UrlPath(key): UrlPath<String>,
    Query(query): Query<PutQuery>,
    body: Bytes,
) -> Response {
    let message = put_message(&key, query.expires, query.length, &query.sha256);
    if !store.key.verify_hex(message.as_bytes(), &query.signature) {
        return denied("invalid signature");
    }
    if Utc::now().timestamp() >= query.expires {
        return denied("expired");
    }
    if i64::try_from(body.len()).ok() != Some(query.length)
        || hex::encode(Sha256::digest(&body)) != query.sha256
    {
        return (
            StatusCode::BAD_REQUEST,
            "the body does not match the signed length and sha256",
        )
            .into_response();
    }
    let path = match store.path(&key) {
        Ok(path) => path,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let written = async {
        let parent = path.parent().expect("record paths have a parent");
        tokio::fs::create_dir_all(parent).await?;
        let partial = path.with_extension("partial");
        tokio::fs::write(&partial, &body).await?;
        tokio::fs::rename(&partial, &path).await
    }
    .await;
    match written {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn get_record(
    State(store): State<Arc<LocalRecordStore>>,
    UrlPath(key): UrlPath<String>,
    Query(query): Query<GetQuery>,
) -> Response {
    if !store.key.verify_hex(
        get_message(&key, query.expires).as_bytes(),
        &query.signature,
    ) {
        return denied("invalid signature");
    }
    if Utc::now().timestamp() >= query.expires {
        return denied("expired");
    }
    let path = match store.path(&key) {
        Ok(path) => path,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_stay_inside_the_tenant_prefix() {
        let tenant = Uuid::new_v4();
        let run = Uuid::new_v4();
        let key = record_key(tenant, run, "5f0c6a8e-2d55", 3).unwrap();
        assert_eq!(
            key,
            format!("t/{tenant}/runs/{run}/5f0c6a8e-2d55/turn-3.jsonl")
        );
        assert!(check_key(tenant, &key).is_ok());
        assert!(check_key(Uuid::new_v4(), &key).is_err());
        assert!(record_key(tenant, run, "../escape", 1).is_err());
        assert!(check_key(tenant, &format!("t/{tenant}/../x")).is_err());
    }

    #[tokio::test]
    async fn presigned_puts_sign_the_content_length() {
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new("ap-northeast-1"))
            .credentials_provider(aws_sdk_s3::config::Credentials::new(
                "test-id",
                "test-secret",
                None,
                None,
                "test",
            ))
            .build();
        let store = S3RecordStore {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket: "records".into(),
        };
        let tenant = Uuid::new_v4();
        let key = record_key(tenant, Uuid::new_v4(), "s-1", 1).unwrap();
        let presigned = store
            .presign_put(tenant, &key, 1234, &"0".repeat(64))
            .await
            .unwrap();
        let url = reqwest::Url::parse(&presigned.url).unwrap();
        let signed = url
            .query_pairs()
            .find(|(name, _)| name == "X-Amz-SignedHeaders")
            .unwrap()
            .1
            .into_owned();
        assert!(signed.split(';').any(|name| name == "content-length"));
        assert!(!presigned.headers.contains_key("content-length"));
    }
}
