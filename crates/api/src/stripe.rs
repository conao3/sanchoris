use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use reqwest::Method;
use sanchoris_core::model::AuditOrigin;
use sanchoris_core::tenants::{NewTenant, create_tenant};
use sanchoris_graphql::{CheckoutPort, PlanChange, PortError, TenantCheckout};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::ServeConfig;

pub struct StripeCheckout {
    http: reqwest::Client,
    config: Arc<ServeConfig>,
    pool: PgPool,
}

struct PaidPlan {
    id: String,
    price: Option<String>,
    paid: bool,
}

struct Subscription {
    id: String,
    item_id: String,
}

enum Payer<'a> {
    Email(&'a str),
    Customer(&'a str),
    Unknown,
}

fn port_error(context: &str) -> impl Fn(sqlx::Error) -> PortError + '_ {
    move |error| PortError::Failed(format!("{context}: {error}"))
}

impl StripeCheckout {
    pub fn new(http: reqwest::Client, config: Arc<ServeConfig>, pool: PgPool) -> Self {
        Self { http, config, pool }
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
        let (id, price_cents, price) = sqlx::query_as::<_, (String, i32, Option<String>)>(
            "SELECT id, price_cents, stripe_price_id FROM sanchoris.plans WHERE id = $1",
        )
        .bind(plan_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error("read the plan"))?
        .ok_or_else(|| PortError::Rejected(format!("plan {plan_id} does not exist")))?;
        Ok(PaidPlan {
            id,
            price,
            paid: price_cents > 0,
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
        success_path: &str,
        cancel_path: &str,
    ) -> Result<String, PortError> {
        let public = &self.config.public_url;
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
        match payer {
            Payer::Email(email) => form.push(("customer_email".into(), email.to_string())),
            Payer::Customer(customer) => form.push(("customer".into(), customer.to_string())),
            Payer::Unknown => {}
        }
        let session = self
            .send(Method::POST, "/v1/checkout/sessions", &form)
            .await?;
        session["url"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| PortError::Failed("the Stripe checkout session has no url".into()))
    }

    async fn active_subscription(&self, customer: &str) -> Result<Option<Subscription>, PortError> {
        let list = self
            .send(
                Method::GET,
                "/v1/subscriptions",
                &[
                    ("customer".into(), customer.to_string()),
                    ("status".into(), "active".into()),
                ],
            )
            .await?;
        let data = list["data"]
            .as_array()
            .ok_or_else(|| PortError::Failed("the Stripe subscription list has no data".into()))?;
        let [subscription] = data.as_slice() else {
            return match data.len() {
                0 => Ok(None),
                n => Err(PortError::Failed(format!(
                    "Stripe customer {customer} has {n} active subscriptions"
                ))),
            };
        };
        let id = subscription["id"]
            .as_str()
            .ok_or_else(|| PortError::Failed("the Stripe subscription has no id".into()))?;
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
            item_id: item["id"]
                .as_str()
                .ok_or_else(|| PortError::Failed(format!("an item of {id} has no id")))?
                .to_string(),
        }))
    }

    async fn customer(&self, tenant_id: Uuid) -> Result<Option<String>, PortError> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT stripe_customer_id FROM sanchoris.tenants WHERE id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error("read the tenant"))?
        .ok_or_else(|| PortError::Failed(format!("tenant {tenant_id} does not exist")))
    }
}

#[async_trait]
impl CheckoutPort for StripeCheckout {
    async fn tenant_checkout(&self, checkout: &TenantCheckout) -> Result<String, PortError> {
        let plan = self.plan(&checkout.plan_id).await?;
        if !plan.paid {
            return Err(PortError::Rejected(format!("plan {} is free", plan.id)));
        }
        let tenant_id = Uuid::new_v4();
        let url = self
            .checkout_session(
                tenant_id,
                &plan,
                Payer::Email(&checkout.email),
                "/?checkout=success",
                "/tenants/new?checkout=canceled",
            )
            .await?;
        create_tenant(
            &self.pool,
            &NewTenant {
                id: tenant_id,
                name: &checkout.tenant_name,
                plan_id: &plan.id,
                user_id: checkout.user_id,
                stripe_customer_id: None,
                payment_pending: true,
                origin: AuditOrigin::Web,
                source_ip: None,
                now: Utc::now(),
            },
        )
        .await
        .map_err(|error| PortError::Failed(format!("create the tenant: {error}")))?;
        Ok(url)
    }

    async fn change_plan(&self, tenant_id: Uuid, plan_id: &str) -> Result<PlanChange, PortError> {
        let plan = self.plan(plan_id).await?;
        let customer = self.customer(tenant_id).await?;
        let subscription = match &customer {
            Some(customer) => self.active_subscription(customer).await?,
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
            (true, Some(subscription)) => {
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
            (true, None) => {
                let payer = match &customer {
                    Some(customer) => Payer::Customer(customer),
                    None => Payer::Unknown,
                };
                let url = self
                    .checkout_session(
                        tenant_id,
                        &plan,
                        payer,
                        "/settings/billing?checkout=success",
                        "/settings/billing?checkout=canceled",
                    )
                    .await?;
                Ok(PlanChange::Checkout { url })
            }
        }
    }

    async fn billing_portal(&self, tenant_id: Uuid) -> Result<String, PortError> {
        let customer = self.customer(tenant_id).await?.ok_or_else(|| {
            PortError::Rejected(
                "the tenant has no Stripe customer; choose a paid plan first".into(),
            )
        })?;
        let session = self
            .send(
                Method::POST,
                "/v1/billing_portal/sessions",
                &[
                    ("customer".into(), customer),
                    (
                        "return_url".into(),
                        format!("{}/settings/billing", self.config.public_url),
                    ),
                ],
            )
            .await?;
        session["url"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| PortError::Failed("the Stripe portal session has no url".into()))
    }
}
