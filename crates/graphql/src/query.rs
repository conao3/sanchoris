use async_graphql::{Context, Object, Result};
use sanchoris_core::items::{Guard, Tenant as TenantItem, uniq};
use sanchoris_core::keys;
use sanchoris_core::store::{Index, SortKey};

use crate::account::{Plan, Viewer};
use crate::context::{is_operator, membership_in, store};
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
        crate::context::auth(ctx)?;
        let Some(guard) = store(ctx)
            .get::<Guard>(&keys::uniq_key(uniq::SLUG, &slug))
            .await?
        else {
            return Ok(None);
        };
        let Some(membership) = membership_in(ctx, guard.target).await? else {
            return Ok(None);
        };
        Ok(Some(Tenant::load(ctx, membership.scope()).await?))
    }

    async fn operator(&self, ctx: &Context<'_>) -> Result<Option<Operator>> {
        if !is_operator(ctx).await? {
            return Ok(None);
        }
        let mut tenants = store(ctx)
            .query_index::<TenantItem>(Index::Gsi2, keys::TENANTS_INDEX, SortKey::All)
            .await?;
        tenants.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
        Ok(Some(Operator {
            tenant_ids: tenants.into_iter().map(|tenant| tenant.id).collect(),
        }))
    }

    async fn trigger_templates(&self, ctx: &Context<'_>) -> Result<Vec<&'static TriggerTemplate>> {
        crate::context::auth(ctx)?;
        Ok(templates())
    }
}
