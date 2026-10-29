use async_graphql::{Error, ErrorExtensions, ID};
use sanchoris_core::receive::ReceiveLimited;
use uuid::Uuid;

use crate::ports::PortError;

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
