use std::sync::Arc;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    list_query, Contact, ContactAddress, ContactId, ContactListItem, ContactTopicUpdate,
    CreateContactOptions, DeleteContactResponse, List, ListOptions, UpdateContactOptions,
    UpdateContactTopicsResponse,
};

/// Contacts — team-global, addressable by id or email (email wins). Mirrors
/// Resend's `contacts` resource, plus a nested `topics`.
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

    /// `POST /contacts` — 409 `validation_error` on a duplicate email
    /// (case-insensitive per team).
    pub async fn create(&self, contact: &CreateContactOptions) -> Result<ContactId> {
        self.config.post(&["contacts"], contact, None).await
    }

    /// `GET /contacts/:idOrEmail`
    pub async fn get(&self, address: impl Into<ContactAddress>) -> Result<Contact> {
        let address = address.into();
        self.config.get(&["contacts", address.key()], &[]).await
    }

    /// `PATCH /contacts/:idOrEmail` — `null` clears a field, omitted leaves it.
    pub async fn update(
        &self,
        address: impl Into<ContactAddress>,
        changes: &UpdateContactOptions,
    ) -> Result<ContactId> {
        let address = address.into();
        self.config
            .patch(&["contacts", address.key()], changes)
            .await
    }

    /// `DELETE /contacts/:idOrEmail`
    pub async fn delete(
        &self,
        address: impl Into<ContactAddress>,
    ) -> Result<DeleteContactResponse> {
        let address = address.into();
        self.config.delete(&["contacts", address.key()]).await
    }

    /// `GET /contacts`
    pub async fn list(&self, options: Option<&ListOptions>) -> Result<List<ContactListItem>> {
        self.config.get(&["contacts"], &list_query(options)).await
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
