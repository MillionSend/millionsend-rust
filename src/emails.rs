use std::sync::Arc;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    BatchResponse, CancelEmailResponse, CreateEmailResponse, Email, SendEmailOptions,
};

/// Transactional email. Mirrors Resend's `emails` resource.
#[derive(Clone)]
pub struct Emails(pub(crate) Arc<Config>);

impl Emails {
    /// `POST /emails`
    pub async fn send(&self, email: &SendEmailOptions) -> Result<CreateEmailResponse> {
        self.0.post(&["emails"], email, None).await
    }

    /// `POST /emails` with an `Idempotency-Key` (retries collapse to one send).
    pub async fn send_with_idempotency_key(
        &self,
        email: &SendEmailOptions,
        idempotency_key: &str,
    ) -> Result<CreateEmailResponse> {
        self.0.post(&["emails"], email, Some(idempotency_key)).await
    }

    /// `GET /emails/:id`
    pub async fn get(&self, id: &str) -> Result<Email> {
        self.0.get(&["emails", id], &[]).await
    }

    /// `POST /emails/:id/cancel` — only scheduled, unsent emails.
    pub async fn cancel(&self, id: &str) -> Result<CancelEmailResponse> {
        self.0.post_empty(&["emails", id, "cancel"]).await
    }
}

/// Batch send. Mirrors Resend's `batch` resource.
#[derive(Clone)]
pub struct Batch(pub(crate) Arc<Config>);

impl Batch {
    /// `POST /emails/batch` — 1–100 emails as a bare array.
    pub async fn send(&self, emails: &[SendEmailOptions]) -> Result<BatchResponse> {
        self.0.post(&["emails", "batch"], emails, None).await
    }

    /// `POST /emails/batch` with an `Idempotency-Key`.
    pub async fn send_with_idempotency_key(
        &self,
        emails: &[SendEmailOptions],
        idempotency_key: &str,
    ) -> Result<BatchResponse> {
        self.0
            .post(&["emails", "batch"], emails, Some(idempotency_key))
            .await
    }
}
