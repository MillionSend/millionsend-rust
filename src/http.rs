use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::{Client, RequestBuilder};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Write as _;

use crate::error::{ApiError, Error, Result};

const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The set JS `encodeURIComponent` leaves untouched: encode everything but the
/// unreserved marks. Keeps path segments (emails contain `@`) wire-identical to
/// the other MillionSend SDKs.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// Shared transport: holds auth + base URL and turns typed calls into requests.
/// Cloneable and cheap to share — every service holds an `Arc<Config>`.
#[derive(Clone)]
pub(crate) struct Config {
    api_key: String,
    base_url: String,
    user_agent: String,
    client: Client,
}

impl Config {
    pub(crate) fn new(api_key: String, base_url: String) -> Self {
        Config {
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            user_agent: format!("millionsend-rust/{SDK_VERSION}"),
            client: Client::new(),
        }
    }

    fn url(&self, segments: &[&str]) -> String {
        let mut url = self.base_url.clone();
        for &segment in segments {
            url.push('/');
            let _ = write!(url, "{}", utf8_percent_encode(segment, COMPONENT));
        }
        url
    }

    fn url_with_query(&self, segments: &[&str], query: &[(&'static str, String)]) -> String {
        let mut url = self.url(segments);
        for (i, (key, value)) in query.iter().enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            let _ = write!(url, "{}={}", key, utf8_percent_encode(value, COMPONENT));
        }
        url
    }

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&'static str, String)],
    ) -> Result<T> {
        let url = self.url_with_query(segments, query);
        self.run(self.client.get(url)).await
    }

    pub(crate) async fn post<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &B,
        idempotency_key: Option<&str>,
    ) -> Result<T> {
        let mut req = self.client.post(self.url(segments)).json(body);
        // Idempotency is POST-only on the wire; only emails.send/batch.send pass it.
        if let Some(key) = idempotency_key {
            req = req.header("Idempotency-Key", key);
        }
        self.run(req).await
    }

    pub(crate) async fn post_empty<T: DeserializeOwned>(&self, segments: &[&str]) -> Result<T> {
        self.run(self.client.post(self.url(segments))).await
    }

    pub(crate) async fn patch<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &B,
    ) -> Result<T> {
        self.run(self.client.patch(self.url(segments)).json(body))
            .await
    }

    pub(crate) async fn delete<T: DeserializeOwned>(&self, segments: &[&str]) -> Result<T> {
        self.run(self.client.delete(self.url(segments))).await
    }

    async fn run<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let response = req
            .bearer_auth(&self.api_key)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, self.user_agent.as_str())
            .send()
            .await
            .map_err(Error::Http)?;

        let status = response.status();
        let body = response.bytes().await.map_err(Error::Http)?;

        if status.is_success() {
            serde_json::from_slice(&body).map_err(Error::Parse)
        } else {
            Err(Error::Api(ApiError::parse(status.as_u16(), &body)))
        }
    }
}
