use std::sync::Arc;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    list_query, CreateWebhookOptions, CreateWebhookResponse, DeleteWebhookResponse, List,
    ListOptions, UpdateWebhookOptions, Webhook, WebhookId,
};

/// Webhook endpoints. Mirrors Resend's `webhooks` resource.
#[derive(Clone)]
pub struct Webhooks(pub(crate) Arc<Config>);

impl Webhooks {
    /// `POST /webhooks` — returns the `signing_secret` to verify payloads with.
    pub async fn create(&self, webhook: &CreateWebhookOptions) -> Result<CreateWebhookResponse> {
        self.0.post(&["webhooks"], webhook).await
    }

    /// `GET /webhooks/:id` — includes `signing_secret`.
    pub async fn get(&self, id: &str) -> Result<Webhook> {
        self.0.get(&["webhooks", id], &[]).await
    }

    /// `GET /webhooks` — items carry no `signing_secret`.
    pub async fn list(&self, options: Option<&ListOptions>) -> Result<List<Webhook>> {
        self.0.get(&["webhooks"], &list_query(options)).await
    }

    /// `PATCH /webhooks/:id`
    pub async fn update(&self, id: &str, changes: &UpdateWebhookOptions) -> Result<WebhookId> {
        self.0.patch(&["webhooks", id], changes).await
    }

    /// `DELETE /webhooks/:id`
    pub async fn delete(&self, id: &str) -> Result<DeleteWebhookResponse> {
        self.0.delete(&["webhooks", id]).await
    }
}
