use std::sync::Arc;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    Deleted, ForwardReceivedEmailOptions, GetReceivedEmailOptions, Idempotent, List, ListOptions,
    ListReceivedEmailsOptions, ReceivedAttachment, ReceivedEmail, ReceivedEmailList,
    ReceivedEmailSent, ReplyReceivedEmailOptions, SendOutcome,
};

/// Received mail: Resend's `emails.receiving` surface plus MillionSend's
/// long-poll, server-side reply and forward, and erase. Inbox-scoped keys
/// never see mail held for screening or in quarantine.
#[derive(Clone)]
pub struct Receiving(pub(crate) Arc<Config>);

impl Receiving {
    /// `GET /emails/receiving` — newest first. With `wait`, a long-poll; the
    /// request timeout grows by the wait.
    pub async fn list(
        &self,
        options: Option<&ListReceivedEmailsOptions>,
    ) -> Result<ReceivedEmailList> {
        let query = options.map(|o| o.to_query()).unwrap_or_default();
        let wait = options.and_then(|o| o.wait);
        self.0
            .get_wait(&["emails", "receiving"], &query, wait)
            .await
    }

    /// `GET /emails/receiving/:id` — with its content, screening and, on
    /// verification mail, the extracted code or link.
    pub async fn get(
        &self,
        id: &str,
        options: Option<&GetReceivedEmailOptions>,
    ) -> Result<ReceivedEmail> {
        let query: Vec<_> = options
            .and_then(|o| o.html_format.clone())
            .map(|format| ("html_format", format))
            .into_iter()
            .collect();
        self.0.get(&["emails", "receiving", id], &query).await
    }

    /// `POST /emails/receiving/:id/reply` — pass `&reply`, or
    /// `reply.with_idempotency_key("k")`. [`SendOutcome::Draft`] when an
    /// inbox's rules hold the reply for a person to approve.
    pub async fn reply<'a>(
        &self,
        id: &str,
        reply: impl Into<Idempotent<&'a ReplyReceivedEmailOptions>>,
    ) -> Result<SendOutcome<ReceivedEmailSent>> {
        let reply = reply.into();
        self.0
            .post_outcome(
                &["emails", "receiving", id, "reply"],
                Some(reply.data),
                reply.idempotency_key.as_deref(),
            )
            .await
    }

    /// `POST /emails/receiving/:id/forward` — same options and outcome as
    /// [`reply`](Receiving::reply).
    pub async fn forward<'a>(
        &self,
        id: &str,
        forward: impl Into<Idempotent<&'a ForwardReceivedEmailOptions>>,
    ) -> Result<SendOutcome<ReceivedEmailSent>> {
        let forward = forward.into();
        self.0
            .post_outcome(
                &["emails", "receiving", id, "forward"],
                Some(forward.data),
                forward.idempotency_key.as_deref(),
            )
            .await
    }

    /// `DELETE /emails/receiving/:id` — erase now (full-access keys not scoped
    /// to an inbox).
    pub async fn delete(&self, id: &str) -> Result<Deleted> {
        self.0.delete(&["emails", "receiving", id]).await
    }

    /// Attachments of received emails, with signed download links.
    pub fn attachments(&self) -> ReceivingAttachments {
        ReceivingAttachments(self.0.clone())
    }
}

/// `/emails/receiving/:id/attachments`; each item carries a `download_url`
/// valid for an hour without an API key.
#[derive(Clone)]
pub struct ReceivingAttachments(pub(crate) Arc<Config>);

impl ReceivingAttachments {
    /// `GET /emails/receiving/:email_id/attachments`
    pub async fn list(
        &self,
        email_id: &str,
        options: Option<&ListOptions>,
    ) -> Result<List<ReceivedAttachment>> {
        let query = options.map(ListOptions::to_query).unwrap_or_default();
        self.0
            .get(&["emails", "receiving", email_id, "attachments"], &query)
            .await
    }

    /// `GET /emails/receiving/:email_id/attachments/:id`
    pub async fn get(&self, email_id: &str, id: &str) -> Result<ReceivedAttachment> {
        self.0
            .get(&["emails", "receiving", email_id, "attachments", id], &[])
            .await
    }
}
