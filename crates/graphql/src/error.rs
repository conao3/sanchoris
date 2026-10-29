use std::sync::Arc;

use async_graphql::extensions::{Extension, ExtensionContext, ExtensionFactory, NextExecute};
use async_graphql::{Error, ErrorExtensions, ID, Response};
use sanchoris_core::receive::ReceiveLimited;
use uuid::Uuid;

use crate::ports::PortError;

const INTERNAL_MESSAGE: &str = "internal error";

pub(crate) struct HideInternalErrors;

impl ExtensionFactory for HideInternalErrors {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(HideInternalErrors)
    }
}

#[async_trait::async_trait]
impl Extension for HideInternalErrors {
    async fn execute(
        &self,
        ctx: &ExtensionContext<'_>,
        operation_name: Option<&str>,
        next: NextExecute<'_>,
    ) -> Response {
        let mut response = next.run(ctx, operation_name).await;
        for error in &mut response.errors {
            let extensions = error.extensions.get_or_insert_with(Default::default);
            if extensions.get("code").is_none() {
                eprintln!(
                    "sanchoris graphql: {} at {:?} in {}: {}",
                    operation_name.unwrap_or("anonymous operation"),
                    error.path,
                    error
                        .locations
                        .first()
                        .map_or_else(String::new, |at| format!("{}:{}", at.line, at.column)),
                    error.message
                );
                error.message = INTERNAL_MESSAGE.into();
                extensions.set("code", "INTERNAL");
            }
        }
        response
    }
}

fn coded(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(message.into()).extend_with(|_, e| e.set("code", code))
}

pub fn forbidden(message: impl Into<String>) -> Error {
    coded("FORBIDDEN", message)
}

pub fn not_found(what: impl Into<String>) -> Error {
    coded("NOT_FOUND", format!("{} does not exist", what.into()))
}

pub fn bad_input(message: impl Into<String>) -> Error {
    coded("BAD_USER_INPUT", message)
}

pub fn conflict(message: impl Into<String>) -> Error {
    coded("CONFLICT", message)
}

pub fn rate_limited(limited: &ReceiveLimited) -> Error {
    let limit = limited.limit;
    let retry_after = limited.retry_after_seconds;
    Error::new(limited.reason()).extend_with(|_, e| {
        e.set("code", "RATE_LIMITED");
        e.set("limit", limit);
        e.set("retryAfterSeconds", retry_after);
    })
}

pub fn port(error: PortError) -> Error {
    match error {
        PortError::Rejected(message) => bad_input(message),
        PortError::Failed(message) => Error::new(message),
    }
}

pub fn unauthenticated() -> Error {
    coded("UNAUTHENTICATED", "the request has no authenticated user")
}

pub fn parse_id(id: &ID) -> Result<Uuid, Error> {
    id.parse::<Uuid>()
        .map_err(|_| bad_input(format!("{} is not a valid id", id.as_str())))
}
