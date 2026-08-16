# millionsend

Official Rust SDK for [MillionSend](https://github.com/MillionSend) — a
self-hostable, Resend-compatible email API on AWS SES.

The HTTP API is wire-compatible with Resend, and this crate mirrors the shape of
[`resend-rs`](https://crates.io/crates/resend-rs), so migrating is mostly a
find-and-replace: swap the crate, the client type, and point the base URL at
your instance.

Async (`tokio` + `reqwest`). Every fallible call returns `Result<T, Error>`.

## Install

```toml
[dependencies]
millionsend = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Quickstart

```rust
use millionsend::{MillionSend, SendEmailOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ms = MillionSend::with_base_url("ms_123", "https://mail.acme.dev");

    let sent = ms
        .emails
        .send(&SendEmailOptions {
            from: "Acme <onboarding@acme.dev>".into(),
            to: "delivered@resend.dev".into(),
            subject: "Hello from MillionSend".into(),
            html: Some("<strong>It works!</strong>".into()),
            ..Default::default()
        })
        .await?;

    println!("sent {}", sent.id);
    Ok(())
}
```

`to`, `cc`, `bcc`, and `reply_to` accept a single address (`"a@b.dev".into()`) or
many (`vec!["a@b.dev".to_string(), "c@d.dev".to_string()].into()`).

## Configuration

```rust
use millionsend::MillionSend;

// Explicit base URL.
let ms = MillionSend::with_base_url("ms_123", "https://mail.acme.dev");

// Key only; base URL falls back to MILLIONSEND_BASE_URL, then http://localhost:3001.
let ms = MillionSend::new("ms_123");

// Both from the environment (MILLIONSEND_API_KEY + optional MILLIONSEND_BASE_URL).
let ms = MillionSend::from_env()?;
```

MillionSend is self-hosted, so there is no cloud default — **set the base URL to
your deployment in production.** Every request carries
`Authorization: Bearer <api_key>` and a `millionsend-rust/<version>` User-Agent.

## Error handling

Fallible calls return `Result<T, millionsend::Error>`:

- `Error::Api(ApiError { status_code, name, message })` — a non-2xx response.
  `name` is a stable snake_case code you can match on (`validation_error`,
  `not_found`, `restricted_api_key`, `sending_paused`, …).
- `Error::Http(_)` — a transport failure that never reached the API;
  `err.status_code()` is `None`.
- `Error::Parse(_)` — a 2xx body that failed to deserialize.

```rust
match ms.emails.get(&id).await {
    Ok(email) => println!("{}", email.last_event),
    Err(err) if err.name() == Some("not_found") => { /* … */ }
    Err(err) => eprintln!("{err}"),
}
```

## Resources

### Emails

```rust
use millionsend::SendEmailOptions;

ms.emails.send(&email).await?;                                    // POST /emails
ms.emails.send_with_idempotency_key(&email, "key-123").await?;   // + Idempotency-Key
ms.emails.get(&id).await?;                                        // GET /emails/:id
ms.emails.cancel(&id).await?;                                     // POST /emails/:id/cancel

// Batch: 1–100 in one call.
ms.batch.send(&[email_a, email_b]).await?;                        // POST /emails/batch
ms.batch.send_with_idempotency_key(&emails, "batch-1").await?;
```

### Audiences & contacts

```rust
use millionsend::{ContactAddress, CreateContactOptions, ListOptions, UpdateContactOptions};

let audience = ms.audiences.create("Registered users").await?;
ms.audiences.list(Some(&ListOptions { limit: Some(20), ..Default::default() })).await?;
ms.audiences.get(&audience.id).await?;
ms.audiences.delete(&audience.id).await?;

ms.contacts.create(&CreateContactOptions {
    audience_id: Some(audience.id.clone()),
    email: "ada@acme.dev".into(),
    first_name: Some("Ada".into()),
    ..Default::default()
}).await?;

// Address by id (a bare &str) or email; email wins if both are set.
ms.contacts.get("contact-id").await?;
ms.contacts.get(ContactAddress::email("ada@acme.dev").in_audience(&audience.id)).await?;

// null clears a field, omitted leaves it unchanged.
ms.contacts.update("contact-id", &UpdateContactOptions {
    first_name: Some(None),        // clear
    unsubscribed: Some(true),      // set
    ..Default::default()
}).await?;

ms.contacts.delete(ContactAddress::email("ada@acme.dev")).await?;
ms.contacts.list(Some(&audience.id), None).await?;   // or list(None, None) for top-level
```

Topic subscriptions (granular unsubscribe):

```rust
use millionsend::{ContactTopicUpdate, TopicSubscription};

ms.contacts.topics.update("contact-id", &[ContactTopicUpdate {
    id: "topic-id".into(),
    subscription: TopicSubscription::OptOut,
}]).await?;
```

### Topics

```rust
use millionsend::{CreateTopicOptions, TopicSubscription};

ms.topics.create(&CreateTopicOptions::new("Product updates", TopicSubscription::OptIn)).await?;
ms.topics.get(&id).await?;
ms.topics.list().await?;    // bare { data } — topics are unpaginated
ms.topics.delete(&id).await?;
```

### Broadcasts

```rust
use millionsend::{CreateBroadcastOptions, UpdateBroadcastOptions};

let broadcast = ms.broadcasts.create(&CreateBroadcastOptions {
    audience_id: Some(audience.id.clone()),
    from: "Acme <news@acme.dev>".into(),
    subject: "Launch".into(),
    html: Some("<p>Hi {{{FIRST_NAME|there}}}</p>".into()),
    ..Default::default()
}).await?;

ms.broadcasts.list(None).await?;
ms.broadcasts.get(&broadcast.id).await?;
ms.broadcasts.update(&broadcast.id, &UpdateBroadcastOptions {
    subject: Some("Launch 🚀".into()),
    ..Default::default()
}).await?;                                                 // draft only
ms.broadcasts.send(&broadcast.id, Some("2026-09-01T09:00:00Z")).await?;  // None = send now
ms.broadcasts.cancel(&broadcast.id).await?;                // scheduled only
ms.broadcasts.delete(&broadcast.id).await?;                // draft only
```

### Segments (MillionSend extension)

Dynamic segments are a saved filter over an audience's contacts — a MillionSend
superset with no Resend equivalent (served at `/segments2`).

```rust
use millionsend::{CreateSegmentOptions, SegmentCondition, SegmentFilter, SegmentMatch};

ms.segments.create(&CreateSegmentOptions {
    name: "Pro plan".into(),
    audience_id: audience.id.clone(),
    filter: SegmentFilter {
        match_: SegmentMatch::All,
        conditions: vec![SegmentCondition {
            field: "property:plan".into(),
            op: "equals".into(),
            value: Some("pro".into()),
        }],
    },
}).await?;

ms.segments.get(&id).await?;   // includes a live contact_count
ms.segments.list(None).await?;
ms.segments.update(&id, &Default::default()).await?;
ms.segments.delete(&id).await?;
```

## Migrating from Resend

```diff
- use resend_rs::{Resend, types::CreateEmailBaseOptions};
- let resend = Resend::new("re_123");
+ use millionsend::{MillionSend, SendEmailOptions};
+ let ms = MillionSend::with_base_url("ms_123", "https://mail.acme.dev");
```

Method names and nesting match. Notes:

- **Domains and API keys** are managed in the MillionSend dashboard, not via the
  API — there are no `domains`/`api_keys` resources here.
- Resend's `segments` is an alias of audiences; MillionSend's `segments` is the
  distinct dynamic-filter feature. Use `audiences` for a straight port.

## License

MIT
