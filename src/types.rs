//! Request and response types. Rust's idiomatic snake_case is already the wire
//! casing, so request structs `#[derive(Serialize)]` straight onto the wire
//! (`Option::None` fields are omitted); responses `#[derive(Deserialize)]` the
//! wire shape verbatim, so `object`/`created_at`/`first_name` read as returned.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A recipient field that accepts a single address or a list — serializes as a
/// bare string or a JSON array to match the wire's `string | string[]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Recipients {
    One(String),
    Many(Vec<String>),
}

impl Default for Recipients {
    fn default() -> Self {
        Recipients::Many(Vec::new())
    }
}

impl From<&str> for Recipients {
    fn from(value: &str) -> Self {
        Recipients::One(value.to_string())
    }
}

impl From<String> for Recipients {
    fn from(value: String) -> Self {
        Recipients::One(value)
    }
}

impl From<Vec<String>> for Recipients {
    fn from(value: Vec<String>) -> Self {
        Recipients::Many(value)
    }
}

impl From<Vec<&str>> for Recipients {
    fn from(value: Vec<&str>) -> Self {
        Recipients::Many(value.into_iter().map(String::from).collect())
    }
}

impl<const N: usize> From<[&str; N]> for Recipients {
    fn from(value: [&str; N]) -> Self {
        Recipients::Many(value.iter().map(|s| s.to_string()).collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub value: String,
}

// ---- shared list envelope ------------------------------------------------

/// Keyset pagination for `list` calls. `after`/`before` are mutually exclusive
/// UUID cursors; `limit` is 1–100 (server default 20).
#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    pub limit: Option<u32>,
    pub after: Option<String>,
    pub before: Option<String>,
}

impl ListOptions {
    pub(crate) fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        if let Some(limit) = self.limit {
            query.push(("limit", limit.to_string()));
        }
        if let Some(after) = &self.after {
            query.push(("after", after.clone()));
        }
        if let Some(before) = &self.before {
            query.push(("before", before.clone()));
        }
        query
    }
}

pub(crate) fn list_query(options: Option<&ListOptions>) -> Vec<(&'static str, String)> {
    options.map(ListOptions::to_query).unwrap_or_default()
}

/// The `{ object: "list", data, has_more }` envelope every paginated list returns.
#[derive(Debug, Clone, Deserialize)]
pub struct List<T> {
    pub object: String,
    pub data: Vec<T>,
    pub has_more: bool,
}

// ---- emails --------------------------------------------------------------

/// Build with `SendEmailOptions::new(from, to, subject)` then set the rest, or a
/// struct literal with `..Default::default()`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SendEmailOptions {
    pub from: String,
    pub to: Recipients,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<Recipients>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<Recipients>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Recipients>,
    /// ISO 8601 with offset; up to 30 days ahead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<Tag>>,
}

impl SendEmailOptions {
    pub fn new(
        from: impl Into<String>,
        to: impl Into<Recipients>,
        subject: impl Into<String>,
    ) -> Self {
        SendEmailOptions {
            from: from.into(),
            to: to.into(),
            subject: subject.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateEmailResponse {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Email {
    pub object: String,
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub reply_to: Option<Vec<String>>,
    pub subject: String,
    pub html: Option<String>,
    pub text: Option<String>,
    pub created_at: String,
    pub scheduled_at: Option<String>,
    pub message_id: String,
    pub last_event: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelEmailResponse {
    pub object: String,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchResponse {
    pub data: Vec<CreateEmailResponse>,
}

// ---- contacts ------------------------------------------------------------

/// Build with `CreateContactOptions::new(email)`. Contacts are team-global;
/// duplicates (case-insensitive email) are a 409 `validation_error`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateContactOptions {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsubscribed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, serde_json::Value>>,
}

impl CreateContactOptions {
    pub fn new(email: impl Into<String>) -> Self {
        CreateContactOptions {
            email: email.into(),
            ..Default::default()
        }
    }
}

/// Address a contact by id or email (email wins when both are set). A bare
/// `&str`/`String` is treated as an id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactAddress {
    pub id: Option<String>,
    pub email: Option<String>,
}

impl ContactAddress {
    pub fn id(id: impl Into<String>) -> Self {
        ContactAddress {
            id: Some(id.into()),
            ..Default::default()
        }
    }

    pub fn email(email: impl Into<String>) -> Self {
        ContactAddress {
            email: Some(email.into()),
            ..Default::default()
        }
    }

    /// The path key: email wins over id.
    pub(crate) fn key(&self) -> &str {
        self.email.as_deref().or(self.id.as_deref()).unwrap_or("")
    }
}

impl From<&str> for ContactAddress {
    fn from(value: &str) -> Self {
        ContactAddress::id(value)
    }
}

impl From<String> for ContactAddress {
    fn from(value: String) -> Self {
        ContactAddress::id(value)
    }
}

impl From<&String> for ContactAddress {
    fn from(value: &String) -> Self {
        ContactAddress::id(value.clone())
    }
}

/// Fields default to "leave unchanged". For `first_name`/`last_name`,
/// `Some(Some(v))` sets, `Some(None)` clears the field (sends `null`), and
/// `None` omits it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateContactOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsubscribed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContactId {
    pub object: String,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Contact {
    pub object: String,
    pub id: String,
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub created_at: String,
    pub unsubscribed: bool,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContactListItem {
    pub id: String,
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub created_at: String,
    pub unsubscribed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteContactResponse {
    pub object: String,
    pub contact: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicSubscription {
    OptIn,
    OptOut,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContactTopicUpdate {
    pub id: String,
    pub subscription: TopicSubscription,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateContactTopicsResponse {
    pub id: String,
}

// ---- topics --------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CreateTopicOptions {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub default_subscription: TopicSubscription,
}

impl CreateTopicOptions {
    pub fn new(name: impl Into<String>, default_subscription: TopicSubscription) -> Self {
        CreateTopicOptions {
            name: name.into(),
            description: None,
            default_subscription,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Topic {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub default_subscription: TopicSubscription,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopicId {
    pub id: String,
}

/// `GET /topics` is a bare `{ data }` — topics are unpaginated (no
/// `object`/`has_more`).
#[derive(Debug, Clone, Deserialize)]
pub struct TopicList {
    pub data: Vec<Topic>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteTopicResponse {
    pub id: String,
    pub object: String,
    pub deleted: bool,
}

// ---- broadcasts ----------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateBroadcastOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Neither `segment_id` nor `topic_id` set = send to all contacts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    pub from: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Recipients>,
    // ponytail: cannot send an explicit null to clear topic_id; add Option<Option<String>>
    // if a "detach topic" update is ever needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
}

impl CreateBroadcastOptions {
    pub fn new(from: impl Into<String>, subject: impl Into<String>) -> Self {
        CreateBroadcastOptions {
            from: from.into(),
            subject: subject.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateBroadcastOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Recipients>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BroadcastId {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BroadcastListItem {
    pub id: String,
    pub name: Option<String>,
    pub segment_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub scheduled_at: Option<String>,
    pub sent_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Broadcast {
    pub object: String,
    pub id: String,
    pub name: Option<String>,
    pub segment_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub scheduled_at: Option<String>,
    pub sent_at: Option<String>,
    pub from: String,
    pub subject: String,
    pub reply_to: Option<Vec<String>>,
    pub preview_text: Option<String>,
    pub topic_id: Option<String>,
    pub html: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelBroadcastResponse {
    pub object: String,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteBroadcastResponse {
    pub object: String,
    pub id: String,
    pub deleted: bool,
}

// ---- segments (MillionSend dynamic segments) -----------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentMatch {
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentCondition {
    pub field: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentFilter {
    #[serde(rename = "match")]
    pub match_: SegmentMatch,
    pub conditions: Vec<SegmentCondition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSegmentOptions {
    pub name: String,
    pub filter: SegmentFilter,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateSegmentOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<SegmentFilter>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Segment {
    pub object: String,
    pub id: String,
    pub name: String,
    pub filter: SegmentFilter,
    pub created_at: String,
    /// Present on `get` (a live count); absent on `create`/`list`/`update`.
    #[serde(default)]
    pub contact_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteSegmentResponse {
    pub object: String,
    pub id: String,
    pub deleted: bool,
}
