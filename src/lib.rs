//! Official Rust SDK for [MillionSend](https://github.com/MillionSend) — a
//! self-hostable, Resend-compatible email API.
//!
//! Construct a [`MillionSend`] once and reuse it (it is cheap to [`Clone`]). Each
//! resource hangs off a public field: `emails`, `batch`, `contacts`, `topics`,
//! `broadcasts`, `segments`.
//!
//! ```no_run
//! use millionsend::{MillionSend, SendEmailOptions};
//!
//! # async fn run() -> millionsend::Result<()> {
//! let ms = MillionSend::with_base_url("ms_123", "https://mail.acme.dev");
//! let sent = ms
//!     .emails
//!     .send(&SendEmailOptions {
//!         from: "Acme <onboarding@acme.dev>".into(),
//!         to: "delivered@resend.dev".into(),
//!         subject: "Hello".into(),
//!         html: Some("<strong>it works</strong>".into()),
//!         ..Default::default()
//!     })
//!     .await?;
//! println!("sent {}", sent.id);
//! # Ok(())
//! # }
//! ```
//!
//! Fallible calls return [`Result<T, Error>`](Error); [`Error::Api`] carries the
//! API's `{ statusCode, name, message }`, [`Error::Http`] a transport failure
//! (its [`status_code`](Error::status_code) is `None`).

mod broadcasts;
mod contacts;
mod emails;
mod error;
mod http;
mod segments;
mod topics;
mod types;

use std::sync::Arc;

use http::Config;

pub use broadcasts::Broadcasts;
pub use contacts::{ContactTopics, Contacts};
pub use emails::{Batch, Emails};
pub use error::{ApiError, Error, Result};
pub use segments::Segments;
pub use topics::Topics;
pub use types::*;

const DEFAULT_BASE_URL: &str = "http://localhost:3001";

/// The MillionSend client. Construct once and reuse.
#[derive(Clone)]
pub struct MillionSend {
    pub emails: Emails,
    pub batch: Batch,
    pub contacts: Contacts,
    pub topics: Topics,
    pub broadcasts: Broadcasts,
    pub segments: Segments,
}

impl MillionSend {
    /// Client for the given API key. The base URL falls back to
    /// `MILLIONSEND_BASE_URL`, then `http://localhost:3001`.
    pub fn new(api_key: impl Into<String>) -> Self {
        let base_url =
            std::env::var("MILLIONSEND_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self::build(api_key.into(), base_url)
    }

    /// Client for the given API key pointed at an explicit base URL.
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self::build(api_key.into(), base_url.into())
    }

    /// Client built from `MILLIONSEND_API_KEY` (and optional
    /// `MILLIONSEND_BASE_URL`). Errors if the key is unset.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("MILLIONSEND_API_KEY").map_err(|_| {
            Error::Api(ApiError {
                status_code: None,
                name: "missing_api_key".to_string(),
                message: "Set MILLIONSEND_API_KEY or use MillionSend::new(api_key).".to_string(),
            })
        })?;
        Ok(Self::new(api_key))
    }

    fn build(api_key: String, base_url: String) -> Self {
        let config = Arc::new(Config::new(api_key, base_url));
        MillionSend {
            emails: Emails(config.clone()),
            batch: Batch(config.clone()),
            contacts: Contacts::new(config.clone()),
            topics: Topics(config.clone()),
            broadcasts: Broadcasts(config.clone()),
            segments: Segments(config),
        }
    }
}
