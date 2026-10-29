use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use reqwest::Method;
use sanchoris_core::db::{self, DbError, Store};
use sanchoris_core::items::Tenant;
use sanchoris_core::keys;
use sanchoris_core::model::AuditOrigin;
use sanchoris_core::tenants::{
    NewTenant, PENDING_CHECKOUT_MINUTES, PendingCheckout, PendingSession, TenantError,
    create_tenant, discard_pending_tenant, drop_pending_checkout, is_payment_pending,
    pending_checkout,
};
use sanchoris_graphql::{CheckoutPort, PlanChange, PortError, TenantCheckout};
use serde_json::Value;
use uuid::Uuid;

use crate::config::ApiConfig;

const SETTLED_STATUSES: [&str; 2] = ["active", "trialing"];
const ENDED_STATUSES: [&str; 2] = ["canceled", "incomplete_expired"];

pub struct StripeCheckout {
    http: reqwest::Client,
    config: Arc<ApiConfig>,
    store: Store,
}

struct PaidPlan {
    id: String,
    price: Option<String>,
    paid: bool,
}

struct Session {
    id: String,
    url: String,
}

struct Subscription {
    id: String,
    item_id: String,
    status: String,
}

enum Payer<'a> {
    Email(&'a str),
    Customer(&'a str),
    Unknown,
}

fn db_error(context: &str) -> impl Fn(DbError) -> PortError + '_ {
    move |error| PortError::Failed(format!("{context}: {error}"))
}

fn field<'a>(value: &'a Value, name: &str, what: &str) -> Result<&'a str, PortError> {
    value[name]
        .as_str()
        .ok_or_else(|| PortError::Failed(format!("the Stripe {what} has no {name}")))
}

impl StripeCheckout {
    pub fn new(http: reqwest::Client, config: Arc<ApiConfig>, store: Store) -> Self {
        Self {
            http,
            config,
            store,
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        form: &[(String, String)],
    ) -> Result<Value, PortError> {
        let url = format!("{}{path}", self.config.stripe.api_url);
        let request = self
            .http
            .request(method.clone(), &url)
            .bearer_auth(&self.config.stripe.secret_key);
        let request = if method == Method::GET {
            request.query(form)
        } else {
            request.form(form)
        };
        let response = request
            .send()
            .await
            .map_err(|error| PortError::Failed(format!("Stripe {path}: {error}")))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|error| PortError::Failed(format!("Stripe {path}: {error}")))?;
        if !status.is_success() {
            return Err(PortError::Failed(format!(
                "Stripe {path} returned {status}: {}",
                body.pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("no error message")
            )));
        }
        Ok(body)
    }

    async fn plan(&self, plan_id: &str) -> Result<PaidPlan, PortError> {
        let plan = db::plan(&self.store, plan_id)
            .await
            .map_err(|error| match error {
                DbError::Missing(_) => {
                    PortError::Rejected(format!("plan {plan_id} does not exist"))
                }
                other => db_error("read the plan")(other),
            })?;
        Ok(PaidPlan {
            id: plan.id,
            price: plan.stripe_price_id,
            paid: plan.price_cents > 0,
        })
    }

    fn price(plan: &PaidPlan) -> Result<&str, PortError> {
        plan.price
            .as_deref()
            .ok_or_else(|| PortError::Rejected(format!("plan {} has no Stripe price", plan.id)))
    }

    async fn checkout_session(
        &self,
        tenant_id: Uuid,
        plan: &PaidPlan,
        payer: Payer<'_>,
        paths: (&str, &str),
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<Session, PortError> {
        let public = &self.config.public_url;
        let (success_path, cancel_path) = paths;
        let mut form = vec![
            ("mode".to_string(), "subscription".to_string()),
            ("client_reference_id".into(), tenant_id.to_string()),
            (
                "line_items[0][price]".into(),
                Self::price(plan)?.to_string(),
            ),
            ("line_items[0][quantity]".into(), "1".into()),
            ("metadata[plan_id]".into(), plan.id.clone()),
            ("metadata[tenant_id]".into(), tenant_id.to_string()),
            (
                "subscription_data[metadata][tenant_id]".into(),
                tenant_id.to_string(),
            ),
            ("success_url".into(), format!("{public}{success_path}")),
            ("cancel_url".into(), format!("{public}{cancel_path}")),
        ];
        if let Some(expires_at) = expires_at {
            form.push(("expires_at".into(), expires_at.timestamp().to_string()));
        }
        match payer {
            Payer::Email(email) => form.push(("customer_email".into(), email.to_string())),
            Payer::Customer(customer) => form.push(("customer".into(), customer.to_string())),
            Payer::Unknown => {}
        }
        let session = self
            .send(Method::POST, "/v1/checkout/sessions", &form)
            .await?;
        Ok(Session {
            id: field(&session, "id", "checkout session")?.to_string(),
            url: field(&session, "url", "checkout session")?.to_string(),
        })
    }

    async fn close_session(&self, session_id: &str) -> Result<(), PortError> {
        let path = format!("/v1/checkout/sessions/{session_id}");
        let session = self.send(Method::GET, &path, &[]).await?;
        match field(&session, "status", "checkout session")? {
            "open" => {
                self.send(Method::POST, &format!("{path}/expire"), &[])
                    .await?;
                Ok(())
            }
            "expired" => Ok(()),
            _ => Err(PortError::Rejected(
                "the previous checkout was completed; wait until its tenant is active".into(),
            )),
        }
    }

    async fn clear_pending(&self, pending: &PendingCheckout) -> Result<(), PortError> {
        let tenant = self
            .store
            .get::<Tenant>(&keys::tenant_key(pending.tenant_id))
            .await
            .map_err(db_error("read the pending tenant"))?;
        match tenant {
            Some(tenant) if is_payment_pending(&tenant) => {
                if tenant.stripe_customer_id.is_some() {
                    return Err(PortError::Rejected(format!(
                        "tenant {} is waiting for its payment",
                        tenant.name
                    )));
                }
                self.close_session(&pending.session_id).await?;
                discard_pending_tenant(
                    &self.store,
                    tenant.id,
                    "a new checkout replaced it",
                    Utc::now(),
                )
                .await
                .map_err(db_error("discard the pending tenant"))?;
                Ok(())
            }
            _ => drop_pending_checkout(&self.store, pending)
                .await
                .map_err(db_error("drop the pending checkout")),
        }
    }

    async fn subscription(&self, customer: &str) -> Result<Option<Subscription>, PortError> {
        let list = self
            .send(
                Method::GET,
                "/v1/subscriptions",
                &[
                    ("customer".into(), customer.to_string()),
                    ("status".into(), "all".into()),
                ],
            )
            .await?;
        let data = list["data"]
            .as_array()
            .ok_or_else(|| PortError::Failed("the Stripe subscription list has no data".into()))?;
        let live = data
            .iter()
            .filter(|subscription| {
                subscription["status"]
                    .as_str()
                    .is_none_or(|status| !ENDED_STATUSES.contains(&status))
            })
            .collect::<Vec<_>>();
        let [subscription] = live.as_slice() else {
            return match live.len() {
                0 => Ok(None),
                n => Err(PortError::Failed(format!(
                    "Stripe customer {customer} has {n} live subscriptions"
                ))),
            };
        };
        let id = field(subscription, "id", "subscription")?;
        let items = subscription
            .pointer("/items/data")
            .and_then(Value::as_array)
            .ok_or_else(|| PortError::Failed(format!("Stripe subscription {id} has no items")))?;
        let [item] = items.as_slice() else {
            return Err(PortError::Failed(format!(
                "Stripe subscription {id} must have exactly one item"
            )));
        };
        Ok(Some(Subscription {
            id: id.to_string(),
            item_id: field(item, "id", "subscription item")?.to_string(),
            status: field(subscription, "status", "subscription")?.to_string(),
        }))
    }

    async fn tenant(&self, tenant_id: Uuid) -> Result<Tenant, PortError> {
        self.store
            .get::<Tenant>(&keys::tenant_key(tenant_id))
            .await
            .map_err(db_error("read the tenant"))?
            .ok_or_else(|| PortError::Failed(format!("tenant {tenant_id} does not exist")))
    }

    async fn portal(&self, customer: String) -> Result<String, PortError> {
        let session = self
            .send(
                Method::POST,
                "/v1/billing_portal/sessions",
                &[
                    ("customer".into(), customer),
                    (
                        "configuration".into(),
                        self.config.stripe.portal_configuration.clone(),
                    ),
                    (
                        "return_url".into(),
                        format!("{}/settings/billing", self.config.public_url),
                    ),
                ],
            )
            .await?;
        Ok(field(&session, "url", "portal session")?.to_string())
    }
}

#[async_trait]
impl CheckoutPort for StripeCheckout {
    async fn tenant_checkout(&self, checkout: &TenantCheckout) -> Result<String, PortError> {
        let plan = self.plan(&checkout.plan_id).await?;
        if !plan.paid {
            return Err(PortError::Rejected(format!("plan {} is free", plan.id)));
        }
        if let Some(pending) = pending_checkout(&self.store, checkout.user_id)
            .await
            .map_err(db_error("read the pending checkout"))?
        {
            self.clear_pending(&pending).await?;
        }
        let tenant_id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + Duration::minutes(PENDING_CHECKOUT_MINUTES);
        let session = self
            .checkout_session(
                tenant_id,
                &plan,
                Payer::Email(&checkout.email),
                ("/?checkout=success", "/tenants/new?checkout=canceled"),
                Some(expires_at),
            )
            .await?;
        let created = create_tenant(
            &self.store,
            &NewTenant {
                id: tenant_id,
                name: &checkout.tenant_name,
                plan_id: &plan.id,
                user_id: checkout.user_id,
                checkout: Some(PendingSession {
                    session_id: &session.id,
                    expires_at,
                }),
                origin: AuditOrigin::Web,
                source_ip: None,
                now,
            },
        )
        .await;
        match created {
            Ok(_) => Ok(session.url),
            Err(error) => {
                self.close_session(&session.id).await?;
                Err(match error {
                    TenantError::Db(error) => db_error("create the tenant")(error),
                    other => PortError::Rejected(other.to_string()),
                })
            }
        }
    }

    async fn change_plan(&self, tenant_id: Uuid, plan_id: &str) -> Result<PlanChange, PortError> {
        let plan = self.plan(plan_id).await?;
        let tenant = self.tenant(tenant_id).await?;
        if is_payment_pending(&tenant) {
            if plan.paid {
                return Err(PortError::Rejected(
                    "the tenant is waiting for its checkout; finish it or choose the free plan"
                        .into(),
                ));
            }
            if let Some(pending) = pending_checkout(&self.store, tenant.created_by_user_id)
                .await
                .map_err(db_error("read the pending checkout"))?
                .filter(|pending| pending.tenant_id == tenant_id)
            {
                self.close_session(&pending.session_id).await?;
            }
        }
        let subscription = match &tenant.stripe_customer_id {
            Some(customer) => self.subscription(customer).await?,
            None => None,
        };
        match (plan.paid, subscription) {
            (false, Some(subscription)) => {
                self.send(
                    Method::DELETE,
                    &format!("/v1/subscriptions/{}", subscription.id),
                    &[],
                )
                .await?;
                Ok(PlanChange::Applied)
            }
            (false, None) => Ok(PlanChange::Applied),
            (true, Some(subscription))
                if SETTLED_STATUSES.contains(&subscription.status.as_str()) =>
            {
                self.send(
                    Method::POST,
                    &format!("/v1/subscriptions/{}", subscription.id),
                    &[
                        ("items[0][id]".into(), subscription.item_id),
                        ("items[0][price]".into(), Self::price(&plan)?.to_string()),
                        ("proration_behavior".into(), "create_prorations".into()),
                        ("metadata[plan_id]".into(), plan.id.clone()),
                    ],
                )
                .await?;
                Ok(PlanChange::Applied)
            }
            (true, Some(_)) => {
                let customer = tenant
                    .stripe_customer_id
                    .clone()
                    .ok_or_else(|| PortError::Failed("the subscription has no customer".into()))?;
                Ok(PlanChange::Portal {
                    url: self.portal(customer).await?,
                })
            }
            (true, None) => {
                let payer = match &tenant.stripe_customer_id {
                    Some(customer) => Payer::Customer(customer),
                    None => Payer::Unknown,
                };
                let session = self
                    .checkout_session(
                        tenant_id,
                        &plan,
                        payer,
                        (
                            "/settings/billing?checkout=success",
                            "/settings/billing?checkout=canceled",
                        ),
                        None,
                    )
                    .await?;
                Ok(PlanChange::Checkout { url: session.url })
            }
        }
    }

    async fn billing_portal(&self, tenant_id: Uuid) -> Result<String, PortError> {
        let customer = self
            .tenant(tenant_id)
            .await?
            .stripe_customer_id
            .ok_or_else(|| {
                PortError::Rejected(
                    "the tenant has no Stripe customer; choose a paid plan first".into(),
                )
            })?;
        self.portal(customer).await
    }

    async fn end_billing(&self, tenant_id: Uuid) -> Result<(), PortError> {
        let tenant = self.tenant(tenant_id).await?;
        if let Some(pending) = pending_checkout(&self.store, tenant.created_by_user_id)
            .await
            .map_err(db_error("read the pending checkout"))?
            .filter(|pending| pending.tenant_id == tenant_id)
        {
            self.close_session(&pending.session_id).await?;
        }
        let Some(customer) = &tenant.stripe_customer_id else {
            return Ok(());
        };
        if let Some(subscription) = self.subscription(customer).await? {
            self.send(
                Method::DELETE,
                &format!("/v1/subscriptions/{}", subscription.id),
                &[],
            )
            .await?;
        }
        Ok(())
    }
}
