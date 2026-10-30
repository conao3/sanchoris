use std::borrow::Cow;
use std::path::PathBuf;

use axum::extract::State;
use axum::http::{Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../apps/frontend/dist"]
#[allow_missing = true]
struct Dist;

pub const INDEX: &str = "index.html";
pub const SERVER_PREFIXES: [&str; 6] = [
    "/api",
    "/webhooks",
    "/auth/github",
    "/health",
    "/records",
    "/runs",
];
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

#[derive(Debug, Clone)]
pub enum Frontend {
    Dir(PathBuf),
    Embedded,
}

impl Frontend {
    pub fn dir(path: PathBuf) -> Result<Self, String> {
        if !path.join(INDEX).is_file() {
            return Err(format!(
                "SANCHORIS_FRONTEND_DIR: {} has no {INDEX} (run vp run build-frontend)",
                path.display()
            ));
        }
        Ok(Self::Dir(path))
    }

    pub fn embedded() -> Result<Self, String> {
        if Dist::get(INDEX).is_none() {
            return Err(format!(
                "the api binary embeds no apps/frontend/dist/{INDEX} (run vp run build-frontend before building it)"
            ));
        }
        Ok(Self::Embedded)
    }

    async fn read(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>, std::io::Error> {
        match self {
            Self::Embedded => Ok(Dist::get(path).map(|file| file.data)),
            Self::Dir(dir) => match tokio::fs::read(dir.join(path)).await {
                Ok(bytes) => Ok(Some(Cow::Owned(bytes))),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory
                    ) =>
                {
                    Ok(None)
                }
                Err(error) => Err(error),
            },
        }
    }
}

fn is_server_path(path: &str) -> bool {
    SERVER_PREFIXES
        .iter()
        .any(|prefix| match path.strip_prefix(prefix) {
            Some(rest) => rest.is_empty() || rest.starts_with('/'),
            None => false,
        })
}

fn asset_path(path: &str) -> Option<&str> {
    let relative = path.trim_start_matches('/');
    if relative.is_empty() {
        return Some(INDEX);
    }
    relative
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        .then_some(relative)
}

fn file(path: &str, bytes: Cow<'static, [u8]>, method: &Method) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("assets/") {
        IMMUTABLE
    } else {
        REVALIDATE
    };
    let headers = [
        (header::CONTENT_TYPE, mime.essence_str().to_string()),
        (header::CACHE_CONTROL, cache.to_string()),
    ];
    if method == Method::HEAD {
        return (headers, ()).into_response();
    }
    (headers, bytes.into_owned()).into_response()
}

pub async fn serve(State(frontend): State<Frontend>, method: Method, uri: Uri) -> Response {
    if is_server_path(uri.path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let Some(path) = asset_path(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let found = match frontend.read(path).await {
        Ok(found) => found,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    if let Some(bytes) = found {
        return file(path, bytes, &method);
    }
    if path
        .rsplit('/')
        .next()
        .is_some_and(|name| name.contains('.'))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match frontend.read(INDEX).await {
        Ok(Some(bytes)) => file(INDEX, bytes, &method),
        Ok(None) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{INDEX} is missing"),
        )
            .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_paths_and_traversal_are_not_spa_routes() {
        assert!(is_server_path("/api/graphql"));
        assert!(is_server_path("/auth/github/callback"));
        assert!(is_server_path("/health"));
        assert!(!is_server_path("/healthy"));
        assert!(!is_server_path("/auth/callback"));
        assert!(!is_server_path("/tasks/acme"));
        assert_eq!(asset_path("/"), Some(INDEX));
        assert_eq!(asset_path("/assets/app.js"), Some("assets/app.js"));
        assert_eq!(asset_path("/assets/../secret"), None);
        assert_eq!(asset_path("/a//b"), None);
    }
}
