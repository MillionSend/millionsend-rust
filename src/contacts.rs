use std::sync::Arc;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    list_query, Contact, ContactAddress, ContactId, ContactListItem, ContactTopicUpdate,
    CreateContactOptions, DeleteContactResponse, List, ListOptions, UpdateContactOptions,
    UpdateContactTopicsResponse,
};

/// Contacts — addressable by id or email (email wins), scoped to an audience or
/// top-level. Mirrors Resend's `contacts` resource, plus a nested `topics`.
#[derive(Clone)]
pub struct Contacts {
    config: Arc<Config>,
    pub topics: ContactTopics,
}

impl Contacts {
    pub(crate) fn new(config: Arc<Config>) -> Self {
        Contacts {
            topics: ContactTopics(config.clone()),
            config,
        }
    }

    /// `POST /audiences/:audienceId/contacts` (or `POST /contacts` when
    /// `audience_id` is unset).
    pub async fn create(&self, contact: &CreateContactOptions) -> Result<ContactId> {
        let segments = match &contact.audience_id {
            Some(audience_id) => vec!["audiences", audience_id.as_str(), "contacts"],
            None => vec!["contacts"],
        };
        self.config.post(&segments, contact, None).await
    }

    /// `GET /contacts/:idOrEmail` (audience-scoped when the address carries one).
    pub async fn get(&self, address: impl Into<ContactAddress>) -> Result<Contact> {
        let address = address.into();
        self.config.get(&contact_segments(&address), &[]).await
    }

    /// `PATCH /contacts/:idOrEmail` — `null` clears a field, omitted leaves it.
    pub async fn update(
        &self,
        address: impl Into<ContactAddress>,
        changes: &UpdateContactOptions,
    ) -> Result<ContactId> {
        let address = address.into();
        self.config
            .patch(&contact_segments(&address), changes)
            .await
    }

    /// `DELETE /contacts/:idOrEmail`
    pub async fn delete(
        &self,
        address: impl Into<ContactAddress>,
    ) -> Result<DeleteContactResponse> {
        let address = address.into();
        self.config.delete(&contact_segments(&address)).await
    }

    /// `GET /audiences/:audienceId/contacts` (or `GET /contacts`).
    pub async fn list(
        &self,
        audience_id: Option<&str>,
        options: Option<&ListOptions>,
    ) -> Result<List<ContactListItem>> {
        let segments = match audience_id {
            Some(audience_id) => vec!["audiences", audience_id, "contacts"],
            None => vec!["contacts"],
        };
        self.config.get(&segments, &list_query(options)).await
    }
}

/// Per-contact topic subscriptions (opt a contact in/out of a topic).
#[derive(Clone)]
pub struct ContactTopics(pub(crate) Arc<Config>);

impl ContactTopics {
    /// `PATCH /contacts/:idOrEmail/topics` with a bare array of updates.
    pub async fn update(
        &self,
        address: impl Into<ContactAddress>,
        topics: &[ContactTopicUpdate],
    ) -> Result<UpdateContactTopicsResponse> {
        let address = address.into();
        self.0
            .patch(&["contacts", address.key(), "topics"], topics)
            .await
    }
}

fn contact_segments(address: &ContactAddress) -> Vec<&str> {
    match &address.audience_id {
        Some(audience_id) => vec!["audiences", audience_id.as_str(), "contacts", address.key()],
        None => vec!["contacts", address.key()],
    }
}
