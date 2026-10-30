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
    /// The signed-in user, the tenants they belong to and the invitations addressed to them.
    async fn viewer(&self, ctx: &Context<'_>) -> Result<Viewer> {
        crate::context::auth(ctx)?;
        Ok(Viewer)
    }

    /// The plans a tenant can choose, ordered by price. The Admin plan is not among them.
    async fn plans(&self, ctx: &Context<'_>) -> Result<Vec<Plan>> {
        crate::context::auth(ctx)?;
        Plan::for_sale(ctx).await
    }

    /// A tenant the viewer is a member of. Null when it does not exist or the viewer is not a member.
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

    /// Operator console. Null unless the viewer is a sanchoris operator.
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

    /// Templates offered by New from template on Triggers and by the first-run setup.
    async fn trigger_templates(&self, ctx: &Context<'_>) -> Result<Vec<&'static TriggerTemplate>> {
        crate::context::auth(ctx)?;
        Ok(templates())
    }
}
