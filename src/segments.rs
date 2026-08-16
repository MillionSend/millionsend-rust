use std::sync::Arc;

use crate::error::Result;
use crate::http::Config;
use crate::types::{
    list_query, CreateSegmentOptions, DeleteSegmentResponse, List, ListOptions, Segment,
    UpdateSegmentOptions,
};

/// Dynamic segments — a saved filter over an audience's contacts (MillionSend
/// extension, no Resend equivalent; served at `/segments2`). `get` returns a
/// live `contact_count`.
#[derive(Clone)]
pub struct Segments(pub(crate) Arc<Config>);

impl Segments {
    /// `POST /segments2`
    pub async fn create(&self, segment: &CreateSegmentOptions) -> Result<Segment> {
        self.0.post(&["segments2"], segment, None).await
    }

    /// `GET /segments2/:id` — includes `contact_count`.
    pub async fn get(&self, id: &str) -> Result<Segment> {
        self.0.get(&["segments2", id], &[]).await
    }

    /// `GET /segments2`
    pub async fn list(&self, options: Option<&ListOptions>) -> Result<List<Segment>> {
        self.0.get(&["segments2"], &list_query(options)).await
    }

    /// `PATCH /segments2/:id`
    pub async fn update(&self, id: &str, changes: &UpdateSegmentOptions) -> Result<Segment> {
        self.0.patch(&["segments2", id], changes).await
    }

    /// `DELETE /segments2/:id`
    pub async fn delete(&self, id: &str) -> Result<DeleteSegmentResponse> {
        self.0.delete(&["segments2", id]).await
    }
}
