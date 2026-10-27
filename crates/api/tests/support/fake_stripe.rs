use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

pub const SECRET_KEY: &str = "sk_test_http";

#[derive(Debug, Clone)]
pub struct StripeRequest {
    pub method: &'static str,
    pub path: String,
    pub form: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub struct StripeState {
    pub requests: Vec<StripeRequest>,
    pub subscriptions: HashMap<String, Vec<Value>>,
    next_session: u32,
}

#[derive(Default)]
pub struct FakeStripe {
    pub state: Mutex<StripeState>,
}

type Fake = State<Arc<FakeStripe>>;
type Fields = Form<BTreeMap<String, String>>;

fn authorized(headers: &HeaderMap) -> bool {
    let expected = format!("Bearer {SECRET_KEY}");
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(expected.as_str())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": { "message": "Invalid API Key provided" } })),
    )
        .into_response()
}

impl FakeStripe {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn requests(&self, path: &str) -> Vec<StripeRequest> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|request| request.path == path)
            .cloned()
            .collect()
    }

    fn record(&self, method: &'static str, path: String, form: BTreeMap<String, String>) {
        self.state
            .lock()
            .unwrap()
            .requests
            .push(StripeRequest { method, path, form });
    }

    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/v1/checkout/sessions", post(checkout_session))
            .route("/v1/billing_portal/sessions", post(portal_session))
            .route("/v1/subscriptions", axum::routing::get(list_subscriptions))
            .route(
                "/v1/subscriptions/{id}",
                post(update_subscription).delete(cancel_subscription),
            )
            .with_state(self)
    }
}

async fn checkout_session(State(fake): Fake, headers: HeaderMap, Form(form): Fields) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    fake.record("POST", "/v1/checkout/sessions".into(), form);
    let id = {
        let mut state = fake.state.lock().unwrap();
        state.next_session += 1;
        format!("cs_test_{}", state.next_session)
    };
    Json(json!({ "id": id, "url": format!("https://checkout.stripe.test/{id}") })).into_response()
}

async fn portal_session(State(fake): Fake, headers: HeaderMap, Form(form): Fields) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    let customer = form["customer"].clone();
    fake.record("POST", "/v1/billing_portal/sessions".into(), form);
    Json(json!({ "url": format!("https://billing.stripe.test/{customer}") })).into_response()
}

async fn list_subscriptions(
    State(fake): Fake,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    let data = fake
        .state
        .lock()
        .unwrap()
        .subscriptions
        .get(&query["customer"])
        .cloned()
        .unwrap_or_default();
    fake.record("GET", "/v1/subscriptions".into(), query);
    Json(json!({ "object": "list", "data": data })).into_response()
}

async fn update_subscription(
    State(fake): Fake,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    Form(form): Fields,
) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    fake.record("POST", format!("/v1/subscriptions/{id}"), form);
    Json(json!({ "id": id, "status": "active" })).into_response()
}

async fn cancel_subscription(
    State(fake): Fake,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> Response {
    if !authorized(&headers) {
        return unauthorized();
    }
    fake.record("DELETE", format!("/v1/subscriptions/{id}"), BTreeMap::new());
    Json(json!({ "id": id, "status": "canceled" })).into_response()
}
