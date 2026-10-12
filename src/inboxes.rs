use std::sync::Arc;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    list_query, CreateInboxDraftOptions, CreateInboxLabelOptions, CreateInboxOptions, Deleted,
    ForwardInThreadOptions, GetInboxThreadEmailOptions, Idempotent, Inbox, InboxDraft,
    InboxDraftSent, InboxLabel, InboxSafety, InboxSafetyUpdate, InboxThread, InboxThreadEmail,
    InboxThreadListItem, LatestCodeOptions, List, ListInboxThreadEmailsOptions,
    ListInboxThreadsOptions, ListInboxesOptions, ListOptions, QuarantineRelease, QuarantinedEmail,
    ReleaseQuarantinedOptions, ReplyInThreadOptions, SearchInboxThreadsOptions, SendOutcome,
    UpdateInboxDraftOptions, UpdateInboxLabelOptions, UpdateInboxOptions, UpdateInboxThreadOptions,
    VerificationCode,
};

/// Agent and human inboxes: Resend's Inboxes beta plus MillionSend's safety
/// rules, codes and quarantine.
#[derive(Clone)]
pub struct Inboxes(pub(crate) Arc<Config>);

impl Inboxes {
    /// `POST /inboxes` — `&CreateInboxOptions::default()` gets a random
    /// address on the shared inbox domain.
    pub async fn create(&self, inbox: &CreateInboxOptions) -> Result<Inbox> {
        self.0.post(&["inboxes"], inbox).await
    }

    /// `GET /inboxes` — newest first.
    pub async fn list(&self, options: Option<&ListInboxesOptions>) -> Result<List<Inbox>> {
        let query = options.map(|o| o.to_query()).unwrap_or_default();
        self.0.get(&["inboxes"], &query).await
    }

    /// `GET /inboxes/:id_or_email`
    pub async fn get(&self, id_or_email: &str) -> Result<Inbox> {
        self.0.get(&["inboxes", id_or_email], &[]).await
    }

    /// `PATCH /inboxes/:id` — name, display name, expiry.
    pub async fn update(&self, id: &str, changes: &UpdateInboxOptions) -> Result<Inbox> {
        self.0.patch(&["inboxes", id], changes).await
    }

    /// `DELETE /inboxes/:id` — its threads, drafts and labels go with it, and
    /// its inbox-scoped keys are revoked.
    pub async fn delete(&self, id: &str) -> Result<Deleted> {
        self.0.delete(&["inboxes", id]).await
    }

    /// Who may write in and what an agent may send.
    pub fn safety(&self) -> InboxSafetyRules {
        InboxSafetyRules(self.0.clone())
    }

    /// Verification codes and sign-in links found in received mail.
    pub fn codes(&self) -> InboxCodes {
        InboxCodes(self.0.clone())
    }

    /// Mail the inbox's rules held back.
    pub fn quarantine(&self) -> InboxQuarantine {
        InboxQuarantine(self.0.clone())
    }

    pub fn labels(&self) -> InboxLabels {
        InboxLabels(self.0.clone())
    }

    pub fn threads(&self) -> InboxThreads {
        InboxThreads(self.0.clone())
    }

    pub fn drafts(&self) -> InboxDrafts {
        InboxDrafts(self.0.clone())
    }
}

/// `/inboxes/:id/safety`
#[derive(Clone)]
pub struct InboxSafetyRules(pub(crate) Arc<Config>);

impl InboxSafetyRules {
    /// `GET /inboxes/:id/safety`
    pub async fn get(&self, inbox_id: &str) -> Result<InboxSafety> {
        self.0.get(&["inboxes", inbox_id, "safety"], &[]).await
    }

    /// `PATCH /inboxes/:id/safety` — returns every rule. Inbox-scoped keys get 403.
    pub async fn update(&self, inbox_id: &str, changes: &InboxSafetyUpdate) -> Result<InboxSafety> {
        self.0
            .patch(&["inboxes", inbox_id, "safety"], changes)
            .await
    }
}

/// `/inboxes/:id/codes`
#[derive(Clone)]
pub struct InboxCodes(pub(crate) Arc<Config>);

impl InboxCodes {
    /// `GET /inboxes/:id/codes/latest` — the newest code or sign-in link;
    /// always 200, `status` says whether one was found. With `wait`, a
    /// long-poll; the request timeout grows by the wait.
    pub async fn latest(
        &self,
        inbox_id: &str,
        options: Option<&LatestCodeOptions>,
    ) -> Result<VerificationCode> {
        let query = options.map(|o| o.to_query()).unwrap_or_default();
        let wait = options.and_then(|o| o.wait);
        self.0
            .get_wait(&["inboxes", inbox_id, "codes", "latest"], &query, wait)
            .await
    }
}

/// `/inboxes/:id/quarantine`: held mail is kept and listed, but agents see it
/// only once a person releases it.
#[derive(Clone)]
pub struct InboxQuarantine(pub(crate) Arc<Config>);

impl InboxQuarantine {
    /// `GET /inboxes/:id/quarantine` — newest first.
    pub async fn list(
        &self,
        inbox_id: &str,
        options: Option<&ListOptions>,
    ) -> Result<List<QuarantinedEmail>> {
        self.0
            .get(&["inboxes", inbox_id, "quarantine"], &list_query(options))
            .await
    }

    /// `POST /inboxes/:id/quarantine/:email_id/release` — `None` sends `{}`.
    pub async fn release(
        &self,
        inbox_id: &str,
        email_id: &str,
        options: Option<&ReleaseQuarantinedOptions>,
    ) -> Result<QuarantineRelease> {
        let options = options.cloned().unwrap_or_default();
        self.0
            .post(
                &["inboxes", inbox_id, "quarantine", email_id, "release"],
                &options,
            )
            .await
    }
}

/// `/inboxes/:id/labels`, applied to threads one at a time.
#[derive(Clone)]
pub struct InboxLabels(pub(crate) Arc<Config>);

impl InboxLabels {
    /// `GET /inboxes/:id/labels` — every label, oldest first (unpaginated).
    pub async fn list(&self, inbox_id: &str) -> Result<List<InboxLabel>> {
        self.0.get(&["inboxes", inbox_id, "labels"], &[]).await
    }

    /// `POST /inboxes/:id/labels`
    pub async fn create(
        &self,
        inbox_id: &str,
        label: &CreateInboxLabelOptions,
    ) -> Result<InboxLabel> {
        self.0.post(&["inboxes", inbox_id, "labels"], label).await
    }

    /// `PATCH /inboxes/:id/labels/:label_id`
    pub async fn update(
        &self,
        inbox_id: &str,
        label_id: &str,
        changes: &UpdateInboxLabelOptions,
    ) -> Result<InboxLabel> {
        self.0
            .patch(&["inboxes", inbox_id, "labels", label_id], changes)
            .await
    }

    /// `DELETE /inboxes/:id/labels/:label_id` — also takes it off every thread.
    pub async fn delete(&self, inbox_id: &str, label_id: &str) -> Result<Deleted> {
        self.0
            .delete(&["inboxes", inbox_id, "labels", label_id])
            .await
    }
}

/// `/inboxes/:id/threads`: conversations in both directions.
#[derive(Clone)]
pub struct InboxThreads(pub(crate) Arc<Config>);

impl InboxThreads {
    /// `GET /inboxes/:id/threads` — most recently active first.
    pub async fn list(
        &self,
        inbox_id: &str,
        options: Option<&ListInboxThreadsOptions>,
    ) -> Result<List<InboxThreadListItem>> {
        let query = options.map(|o| o.to_query()).unwrap_or_default();
        self.0.get(&["inboxes", inbox_id, "threads"], &query).await
    }

    /// `GET /inboxes/:id/threads/search` — subjects, senders and recipients.
    pub async fn search(
        &self,
        inbox_id: &str,
        options: &SearchInboxThreadsOptions,
    ) -> Result<List<InboxThreadListItem>> {
        self.0
            .get(
                &["inboxes", inbox_id, "threads", "search"],
                &options.to_query(),
            )
            .await
    }

    /// `GET /inboxes/:id/threads/:thread_id` — without its messages; see
    /// [`emails`](InboxThreads::emails).
    pub async fn get(&self, inbox_id: &str, thread_id: &str) -> Result<InboxThread> {
        self.0
            .get(&["inboxes", inbox_id, "threads", thread_id], &[])
            .await
    }

    /// `PATCH /inboxes/:id/threads/:thread_id` — read state, folder, label.
    pub async fn update(
        &self,
        inbox_id: &str,
        thread_id: &str,
        changes: &UpdateInboxThreadOptions,
    ) -> Result<InboxThread> {
        self.0
            .patch(&["inboxes", inbox_id, "threads", thread_id], changes)
            .await
    }

    /// `DELETE /inboxes/:id/threads/:thread_id` — moves it to trash, purged
    /// after 30 days; `deleted` is false when it was there already.
    pub async fn delete(&self, inbox_id: &str, thread_id: &str) -> Result<Deleted> {
        self.0
            .delete(&["inboxes", inbox_id, "threads", thread_id])
            .await
    }

    /// The messages of a thread.
    pub fn emails(&self) -> InboxThreadEmails {
        InboxThreadEmails(self.0.clone())
    }
}

/// `/inboxes/:id/threads/:thread_id/emails`
#[derive(Clone)]
pub struct InboxThreadEmails(pub(crate) Arc<Config>);

impl InboxThreadEmails {
    /// `GET .../threads/:thread_id/emails` — oldest first; `after` pages
    /// toward newer.
    pub async fn list(
        &self,
        inbox_id: &str,
        thread_id: &str,
        options: Option<&ListInboxThreadEmailsOptions>,
    ) -> Result<List<InboxThreadEmail>> {
        let query = options.map(|o| o.to_query()).unwrap_or_default();
        self.0
            .get(
                &["inboxes", inbox_id, "threads", thread_id, "emails"],
                &query,
            )
            .await
    }

    /// `GET .../threads/:thread_id/emails/:email_id`
    pub async fn get(
        &self,
        inbox_id: &str,
        thread_id: &str,
        email_id: &str,
        options: Option<&GetInboxThreadEmailOptions>,
    ) -> Result<InboxThreadEmail> {
        let query = options.map(|o| o.to_query()).unwrap_or_default();
        self.0
            .get(
                &[
                    "inboxes", inbox_id, "threads", thread_id, "emails", email_id,
                ],
                &query,
            )
            .await
    }

    /// `POST .../emails/:email_id/reply` — pass `&reply`, or
    /// `reply.with_idempotency_key("k")`. [`SendOutcome::Draft`] when the
    /// inbox's rules hold the reply for approval.
    pub async fn reply<'a>(
        &self,
        inbox_id: &str,
        thread_id: &str,
        email_id: &str,
        reply: impl Into<Idempotent<&'a ReplyInThreadOptions>>,
    ) -> Result<SendOutcome<InboxThreadEmail>> {
        let reply = reply.into();
        self.0
            .post_outcome(
                &[
                    "inboxes", inbox_id, "threads", thread_id, "emails", email_id, "reply",
                ],
                Some(reply.data),
                reply.idempotency_key.as_deref(),
            )
            .await
    }

    /// `POST .../emails/:email_id/forward` — same options and outcome as
    /// [`reply`](InboxThreadEmails::reply).
    pub async fn forward<'a>(
        &self,
        inbox_id: &str,
        thread_id: &str,
        email_id: &str,
        forward: impl Into<Idempotent<&'a ForwardInThreadOptions>>,
    ) -> Result<SendOutcome<InboxThreadEmail>> {
        let forward = forward.into();
        self.0
            .post_outcome(
                &[
                    "inboxes", inbox_id, "threads", thread_id, "emails", email_id, "forward",
                ],
                Some(forward.data),
                forward.idempotency_key.as_deref(),
            )
            .await
    }
}

/// `/inboxes/:id/drafts`: standalone or reply drafts, sent on demand or held
/// for approval.
#[derive(Clone)]
pub struct InboxDrafts(pub(crate) Arc<Config>);

impl InboxDrafts {
    /// `GET /inboxes/:id/drafts` — unsent drafts, most recently updated first.
    pub async fn list(
        &self,
        inbox_id: &str,
        options: Option<&ListOptions>,
    ) -> Result<List<InboxDraft>> {
        self.0
            .get(&["inboxes", inbox_id, "drafts"], &list_query(options))
            .await
    }

    /// `POST /inboxes/:id/drafts` — nothing is sent until
    /// [`send`](InboxDrafts::send).
    pub async fn create(
        &self,
        inbox_id: &str,
        draft: &CreateInboxDraftOptions,
    ) -> Result<InboxDraft> {
        self.0.post(&["inboxes", inbox_id, "drafts"], draft).await
    }

    /// `GET /inboxes/:id/drafts/:draft_id` — also once sent.
    pub async fn get(&self, inbox_id: &str, draft_id: &str) -> Result<InboxDraft> {
        self.0
            .get(&["inboxes", inbox_id, "drafts", draft_id], &[])
            .await
    }

    /// `PATCH /inboxes/:id/drafts/:draft_id`
    pub async fn update(
        &self,
        inbox_id: &str,
        draft_id: &str,
        changes: &UpdateInboxDraftOptions,
    ) -> Result<InboxDraft> {
        self.0
            .patch(&["inboxes", inbox_id, "drafts", draft_id], changes)
            .await
    }

    /// `DELETE /inboxes/:id/drafts/:draft_id`
    pub async fn delete(&self, inbox_id: &str, draft_id: &str) -> Result<Deleted> {
        self.0
            .delete(&["inboxes", inbox_id, "drafts", draft_id])
            .await
    }

    /// `POST /inboxes/:id/drafts/:draft_id/send` — no body.
    /// [`SendOutcome::Draft`] when the inbox's rules hold it for approval; an
    /// inbox-scoped key gets 403 `approval_required` for a draft that waits
    /// for a person.
    pub async fn send(
        &self,
        inbox_id: &str,
        draft_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<SendOutcome<InboxDraftSent>> {
        self.0
            .post_outcome(
                &["inboxes", inbox_id, "drafts", draft_id, "send"],
                None::<&()>,
                idempotency_key,
            )
            .await
    }

    /// `POST /inboxes/:id/drafts/:draft_id/reject` — approvers only.
    pub async fn reject(&self, inbox_id: &str, draft_id: &str) -> Result<InboxDraft> {
        self.0
            .post_empty(&["inboxes", inbox_id, "drafts", draft_id, "reject"])
            .await
    }
}
