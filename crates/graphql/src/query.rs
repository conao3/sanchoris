use async_graphql::{Context, Object, Result};
use uuid::Uuid;

use crate::account::{Plan, Viewer};
use crate::context::{is_operator, memberships, user_tx};
use crate::tenant::{Operator, Tenant};
use crate::trigger::{TriggerTemplate, templates};

pub struct QueryRoot;

#[Object(name = "Query")]
impl QueryRoot {
    async fn viewer(&self, ctx: &Context<'_>) -> Result<Viewer> {
        crate::context::auth(ctx)?;
        Ok(Viewer)
    }

    async fn plans(&self, ctx: &Context<'_>) -> Result<Vec<Plan>> {
        crate::context::auth(ctx)?;
        Plan::all(ctx).await
    }

    async fn tenant(&self, ctx: &Context<'_>, slug: String) -> Result<Option<Tenant>> {
        let mut tx = user_tx(ctx).await?;
        let tenant_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT t.id FROM sanchoris.tenants t JOIN sanchoris.members m ON m.tenant_id = t.id \
             WHERE t.slug = $1 AND m.user_id = $2",
        )
        .bind(&slug)
        .bind(tx.user_id())
        .fetch_optional(tx.conn())
        .await?;
        drop(tx);
        let Some(tenant_id) = tenant_id else {
            return Ok(None);
        };
        let membership = memberships(ctx)
            .await?
            .into_iter()
            .find(|m| m.tenant_id == tenant_id)
            .expect("the tenant was found through the membership");
        Ok(Some(Tenant::load(ctx, membership.scope()).await?))
    }

    async fn operator(&self, ctx: &Context<'_>) -> Result<Option<Operator>> {
        if !is_operator(ctx).await? {
            return Ok(None);
        }
        let mut tx = user_tx(ctx).await?;
        let tenant_ids =
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM sanchoris.tenants ORDER BY name, id")
                .fetch_all(tx.conn())
                .await?;
        Ok(Some(Operator { tenant_ids }))
    }

    async fn trigger_templates(&self, ctx: &Context<'_>) -> Result<Vec<&'static TriggerTemplate>> {
        crate::context::auth(ctx)?;
        Ok(templates())
    }
}
