use async_trait::async_trait;
use serde_json::Value;

use crate::ports::GitHubError;

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, GitHubError>;
}

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("build the HTTP client"),
        }
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, GitHubError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| GitHubError::Transport(error.to_string()))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| GitHubError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| GitHubError::Transport(error.to_string()))?;
        Ok(HttpResponse { status, body })
    }
}
