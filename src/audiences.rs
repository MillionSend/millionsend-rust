use std::sync::Arc;

use serde_json::json;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    list_query, Audience, AudienceListItem, DeleteAudienceResponse, List, ListOptions,
};

/// Named contact lists. Resend-compatible — a migrating app's `audiences.*`
/// calls map straight over. (MillionSend's dynamic-filter `segments` are a
/// separate, richer resource.)
#[derive(Clone)]
pub struct Audiences(pub(crate) Arc<Config>);

impl Audiences {
    /// `POST /audiences`
    pub async fn create(&self, name: impl Into<String>) -> Result<Audience> {
        let name: String = name.into();
        self.0
            .post(&["audiences"], &json!({ "name": name }), None)
            .await
    }

    /// `GET /audiences/:id`
    pub async fn get(&self, id: &str) -> Result<Audience> {
        self.0.get(&["audiences", id], &[]).await
    }

    /// `GET /audiences`
    pub async fn list(&self, options: Option<&ListOptions>) -> Result<List<AudienceListItem>> {
        self.0.get(&["audiences"], &list_query(options)).await
    }

    /// `DELETE /audiences/:id`
    pub async fn delete(&self, id: &str) -> Result<DeleteAudienceResponse> {
        self.0.delete(&["audiences", id]).await
    }
}
