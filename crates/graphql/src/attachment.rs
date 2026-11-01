use async_graphql::{Context, ID, Object, Result};
use sanchoris_core::items::ChatAttachment as ChatAttachmentItem;
use sanchoris_core::keys;
use uuid::Uuid;

use crate::context::state;
use crate::error::{bad_input, port};

pub(crate) const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
pub(crate) const MAX_ATTACHMENT_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_ATTACHMENTS: usize = 10;
const MAX_NAME_CHARS: usize = 100;

/// A file name that is one safe path segment: other characters become `_`.
pub(crate) fn attachment_name(name: &str) -> Result<String> {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(MAX_NAME_CHARS)
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return Err(bad_input("the attachment has no file name"));
    }
    Ok(cleaned)
}

pub(crate) struct ChatAttachment {
    pub tenant_id: Uuid,
    pub item: ChatAttachmentItem,
}

#[Object]
impl ChatAttachment {
    async fn id(&self) -> ID {
        ID(self.item.id.to_string())
    }

    async fn name(&self) -> &str {
        &self.item.name
    }

    async fn mime_type(&self) -> &str {
        &self.item.mime_type
    }

    async fn size_bytes(&self) -> i64 {
        self.item.size_bytes
    }

    /// A link to the file that works for 15 minutes.
    async fn url(&self, ctx: &Context<'_>) -> Result<String> {
        state(ctx)
            .deps
            .attachments
            .url(
                self.tenant_id,
                &keys::attachment_object(self.tenant_id, self.item.id, &self.item.name),
            )
            .await
            .map_err(port)
    }
}
