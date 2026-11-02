use async_graphql::{Context, Enum, ID, InputObject, Object, Result};
use sanchoris_core::cli::{
    APPROVED, CliPurpose, CliRequest as CliRequestItem, DENIED, PENDING, cli_request_key,
    display_user_code, normalize_user_code,
};

use crate::context::{
    AuditTarget, Role, audit_member, auth, member_scope, now, tx, user_tx, write,
};
use crate::enums::CredentialScope;
use crate::error::{bad_input, conflict, not_found, parse_id};
use crate::scalars::{Timestamp, ts};

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRequestPurpose {
    ClaudeCredential,
    CodexCredential,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRequestState {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(InputObject)]
pub(crate) struct ApproveCliRequestInput {
    #[graphql(name = "userCode")]
    user_code: String,
    #[graphql(name = "tenantId")]
    tenant_id: ID,
    scope: CredentialScope,
}

/// A `sancli` sign-in request that waits for a member to approve it in the browser.
pub(crate) struct CliRequest(CliRequestItem);

#[Object]
impl CliRequest {
    /// The code the CLI shows, such as WDJB-MJHT.
    async fn user_code(&self) -> String {
        display_user_code(&self.0.user_code)
    }

    /// The program that asked, as it named itself.
    async fn client(&self) -> &str {
        &self.0.client
    }

    async fn purpose(&self) -> Result<CliRequestPurpose> {
        Ok(match self.0.purpose()? {
            CliPurpose::ClaudeCredential => CliRequestPurpose::ClaudeCredential,
            CliPurpose::CodexCredential => CliRequestPurpose::CodexCredential,
        })
    }

    /// The scope the CLI asked for with or without --org; the approver chooses the scope.
    async fn requested_scope(&self) -> Result<CredentialScope> {
        CredentialScope::from_db(&self.0.requested_scope)
    }

    async fn state(&self, ctx: &Context<'_>) -> CliRequestState {
        if self.0.is_expired(now(ctx)) {
            return CliRequestState::Expired;
        }
        match self.0.status.as_str() {
            PENDING => CliRequestState::Pending,
            DENIED => CliRequestState::Denied,
            _ => CliRequestState::Approved,
        }
    }

    async fn expires_at(&self) -> Timestamp {
        ts(self.0.code_expires_at)
    }
}

fn user_code(text: &str) -> Result<String> {
    normalize_user_code(text).ok_or_else(|| bad_input(format!("{text:?} is not a sancli code")))
}

fn purpose_text(request: &CliRequestItem) -> &'static str {
    match request.purpose() {
        Ok(CliPurpose::CodexCredential) => "a Codex subscription",
        _ => "a Claude subscription",
    }
}

pub(crate) async fn cli_request(ctx: &Context<'_>, code: &str) -> Result<Option<CliRequest>> {
    auth(ctx)?;
    let Some(code) = normalize_user_code(code) else {
        return Ok(None);
    };
    Ok(user_tx(ctx)
        .get::<CliRequestItem>(&cli_request_key(&code))
        .await?
        .map(CliRequest))
}

fn pending(request: Option<CliRequestItem>, ctx: &Context<'_>) -> Result<CliRequestItem> {
    let request = request.ok_or_else(|| not_found("the sancli request"))?;
    if request.is_expired(now(ctx)) {
        return Err(conflict(
            "the sancli request expired; run the command again",
        ));
    }
    if request.status != PENDING {
        return Err(conflict("the sancli request was already decided"));
    }
    Ok(request)
}

pub(crate) async fn approve(
    ctx: &Context<'_>,
    input: ApproveCliRequestInput,
) -> Result<CliRequest> {
    let user_id = auth(ctx)?.user_id;
    let code = user_code(&input.user_code)?;
    let tenant_id = parse_id(&input.tenant_id)?;
    let scope = member_scope(ctx, tenant_id).await?;
    let (member_id, _) = scope.member()?;
    if input.scope == CredentialScope::Organization {
        scope.require(Role::Admin, "adding organization credentials")?;
    }
    let at = now(ctx);
    let code = &code;
    let approved = write(|| async {
        let mut tx = tx(ctx, tenant_id);
        let mut request = pending(tx.get::<CliRequestItem>(&cli_request_key(code)).await?, ctx)?;
        request.status = APPROVED.into();
        request.user_id = Some(user_id);
        request.tenant_id = Some(tenant_id);
        request.member_id = Some(member_id);
        request.scope = Some(input.scope.as_db().to_string());
        request.decided_at = Some(at);
        tx.put(&request)?;
        let shown = display_user_code(code);
        let shared = match input.scope {
            CredentialScope::Organization => "the organization",
            CredentialScope::Personal => "personal use",
        };
        audit_member(
            ctx,
            &mut tx,
            member_id,
            "cli.approve",
            AuditTarget {
                kind: "cli_request",
                id: request.id.to_string(),
                name: shown.clone(),
            },
            &format!(
                "Approved the {} request {shown} to add {} for {shared}",
                request.client,
                purpose_text(&request)
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(request)
    })
    .await?;
    Ok(CliRequest(approved))
}

pub(crate) async fn deny(ctx: &Context<'_>, code: &str) -> Result<CliRequest> {
    let user_id = auth(ctx)?.user_id;
    let code = user_code(code)?;
    let at = now(ctx);
    let code = &code;
    let denied = write(|| async {
        let mut tx = user_tx(ctx);
        let mut request = pending(tx.get::<CliRequestItem>(&cli_request_key(code)).await?, ctx)?;
        request.status = DENIED.into();
        request.user_id = Some(user_id);
        request.decided_at = Some(at);
        tx.put(&request)?;
        tx.commit().await?;
        Ok(request)
    })
    .await?;
    Ok(CliRequest(denied))
}
