# millionsend

Official Rust SDK for [MillionSend](https://github.com/MillionSend) — a
self-hostable, Resend-compatible email API on AWS SES.

The HTTP API is wire-compatible with Resend, and this crate mirrors the shape of
[`resend-rs`](https://crates.io/crates/resend-rs), so migrating is mostly a
find-and-replace: swap the crate and the client type (and, if you self-host,
point the base URL at your instance).

Async (`tokio` + `reqwest`). Every fallible call returns `Result<T, Error>`.

## Install

```toml
[dependencies]
millionsend = "0.9"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Quickstart

```rust
use millionsend::{MillionSend, SendEmailOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ms = MillionSend::new("ms_123"); // Cloud; self-hosted: with_base_url(key, origin)

    let sent = ms
        .emails
        .send(&SendEmailOptions {
            from: "Acme <onboarding@acme.dev>".into(),
            to: "success@simulator.amazonses.com".into(),
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
many (`vec!["a@b.dev", "c@d.dev"].into()`).

## Configuration

```rust
use millionsend::MillionSend;

// Key only: MillionSend Cloud (https://api.millionsend.com), unless
// MILLIONSEND_BASE_URL is set.
let ms = MillionSend::new("ms_123");

// Self-hosted: explicit base URL (wins over the environment).
let ms = MillionSend::with_base_url("ms_123", "https://mail.acme.dev");
assert_eq!(ms.base_url(), "https://mail.acme.dev");

// Both from the environment (MILLIONSEND_API_KEY + optional MILLIONSEND_BASE_URL).
let ms = MillionSend::from_env()?;

// Bring your own reqwest client (proxies, TLS, timeouts). The default has a
// 30s request timeout and a 10s connect timeout; a long-poll (`wait`) gets
// 30s plus its wait.
let ms = MillionSend::new("ms_123").with_client(reqwest::Client::new());

// Accept a non-loopback http:// base URL (refused by default).
let ms = MillionSend::with_base_url("ms_123", "http://10.0.0.5:3001").allow_insecure_http();
```

With no base URL the client talks to MillionSend Cloud, so the key alone is
enough; a self-hosted instance sets its origin with `with_base_url` or
`MILLIONSEND_BASE_URL`. Every request carries
`Authorization: Bearer <api_key>` and a `millionsend-rust/<version>` User-Agent.
Plain `http://` is only accepted for loopback hosts (`localhost`, `127.0.0.1`, `::1`);
any other `http://` URL makes every call return `Error::Api` named `insecure_base_url`,
since the API key is sent as a bearer header. Call `allow_insecure_http()` to talk to a
non-TLS instance elsewhere (e.g. inside a private network).

## Error handling

Fallible calls return `Result<T, millionsend::Error>`:

- `Error::Api(ApiError { status_code, name, message, retry_after })` — a
  non-2xx response. `name` is a stable snake_case code you can match on
  (`validation_error`, `not_found`, `forbidden`, `restricted_api_key`,
  `rate_limit_exceeded`, `sending_paused`, `all_recipients_suppressed`, and for
  receiving and inboxes `inbox_send_policy`, `inbox_send_cap_reached`,
  `approval_required`, `email_quarantined`, `email_blocked`, `too_many_waits`,
  `name_review_unavailable`, …). `retry_after` is the response's `retry-after`
  header in seconds, `None` when it sent none.
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

An unknown route answers 404 `not_found` (`application_error` from servers
before v0.13.0). Match on `name` and `status_code`; messages may change.

### Retries

The SDK never retries on its own. Worth retrying after `err.retry_after()`
seconds: 503 `name_review_unavailable`, 429 `too_many_waits`, and 429
`rate_limit_exceeded` when `retry_after` is 60 or less (the per-minute limits).
Not worth retrying in the same process: 429 `inbox_send_cap_reached` and the
daily name-review budget (429 `rate_limit_exceeded` with a `retry_after` that
runs to the next UTC midnight), and any 403, 404, 409 or 422.

Retry a send with the same idempotency key: a batch the API already accepted
gets its stored response back. A held draft (`SendOutcome::Draft`) is not a
failure; never send it again. `inboxes.create` takes no idempotency key: after
a 429 or 503 nothing was created, but after a timeout a retry can create a
second inbox. With a chosen `username`, a 409 on the retry usually means the
first attempt succeeded: read it with `inboxes.get(address)`.

## Pagination

`list` calls take `limit`, `after` and `before` (keyset cursors) and return
`List { object, data, has_more }`. Lists run newest first (MillionSend v0.13.0
and later): `after` takes the last id of a page and pages toward older items,
`before` takes the first id and pages toward newer ones, and `has_more` refers
to that direction. Threads sort by last activity, drafts by last update, a
thread's emails run oldest first, and labels and topics are not paginated.

```rust
use millionsend::ListOptions;

// Incremental sync: from the newest id you stored, page toward newer items.
let mut before = newest_stored_id;
loop {
    let page = ms.emails.list(Some(&ListOptions {
        before: Some(before.clone()),
        limit: Some(100),
        ..Default::default()
    })).await?;
    save(&page.data);                                  // newest first within the page
    match page.data.first() {
        Some(first) if page.has_more => before = first.id.clone(),
        _ => break,
    }
}
```

A cursor whose item was deleted answers 422 `validation_error` (404
`not_found` on the inbox lists): start again without a cursor and stop at the
ids you already have.

## Resources

### Emails

```rust
use millionsend::{Attachment, IdempotentTrait, SendEmailOptions, Tag, UpdateEmailOptions};

let email = SendEmailOptions {
    from: "Acme <onboarding@acme.dev>".into(),
    to: vec!["ada@acme.dev", "bob@acme.dev"].into(),
    subject: "Invoice".into(),
    html: Some("<p>Attached.</p>".into()),
    reply_to: Some("billing@acme.dev".into()),
    tags: Some(vec![Tag { name: "kind".into(), value: "invoice".into() }]),
    topic_id: Some(topic.id.clone()),                      // skips opted-out recipients
    attachments: Some(vec![Attachment {
        filename: "invoice.pdf".into(),
        content: Some(base64_pdf),                          // base64
        content_type: Some("application/pdf".into()),
        ..Default::default()
    }]),
    headers: Some([("X-Entity-Ref-ID".to_string(), "42".to_string())].into()),
    ..Default::default()
};

ms.emails.send(&email).await?;                                    // POST /emails
ms.emails.send(email.with_idempotency_key("inv-42")).await?;      // + Idempotency-Key (Resend shape)
ms.emails.send_with_idempotency_key(&email, "inv-42").await?;     // same, explicit
ms.emails.get(&id).await?;                                        // GET /emails/:id
ms.emails.list(None).await?;                                      // GET /emails
ms.emails.update(&id, &UpdateEmailOptions {                       // PATCH /emails/:id
    scheduled_at: "2026-09-01T09:00:00Z".into(),
}).await?;
ms.emails.get_insights(&id).await?;                               // GET /emails/:id/insights
ms.emails.cancel(&id).await?;                                     // POST /emails/:id/cancel
ms.emails.delete(&id).await?;                                     // DELETE /emails/:id
```

Every field is put on the wire, including `template`, which the API currently
rejects with a 422 (send `html`/`text` instead). `send` and `batch.send` answer
422 `all_recipients_suppressed` when every `to` recipient is on the suppression
list or opted out of the send's `topic_id`. `get` includes a nullable
best-practice `score` (0–10); `get_insights` returns the full per-check report
behind it (404 `not_found` until insights exist).

`subject` takes up to 998 characters. A `to`, `cc` or `bcc` address on a domain
that can never receive mail (`example.com`, `.test`, `.invalid`, `.localhost`,
a TLD that doesn't exist, …) answers 422 `validation_error`; to test sending
without emailing anyone, use `success@simulator.amazonses.com`. An inbox's
address on the shared domain can be the `from` too, under the rules in
[Sending from the shared domain](#sending-from-the-shared-domain).

#### Batch

```rust
use millionsend::{BatchValidation, IdempotentTrait};

let emails = vec![email_a, email_b];                       // 1–100
ms.batch.send(&emails).await?;                             // POST /emails/batch
ms.batch.send(emails.with_idempotency_key("batch-1")).await?;
ms.batch.send_with_idempotency_key(&emails, "batch-1").await?;

// x-batch-validation: permissive — invalid items land in `errors` instead of
// failing the whole call (strict is the server default).
let res = ms.batch.send_with_batch_validation(&emails, BatchValidation::Permissive).await?;
for err in &res.errors {
    eprintln!("email #{} rejected: {}", err.index, err.message);
}
```

Batch items never send from the shared domain: a strict batch fails with 403
`inbox_send_policy` (the message starts with `emails.<index>: `), a permissive
one lists the item in `errors`.

### Receiving

Mail your domains receive: Resend's `receiving` plus MillionSend's long-poll,
server-side reply and forward, and erase. Inbox-scoped keys never see mail
held for screening or in quarantine.

```rust
use millionsend::{
    ForwardReceivedEmailOptions, GetReceivedEmailOptions, IdempotentTrait,
    ListReceivedEmailsOptions, ReplyReceivedEmailOptions, SendOutcome,
};

let page = ms.receiving.list(None).await?;                         // GET /emails/receiving
let email = ms.receiving.get(&id, None).await?;                    // GET /emails/receiving/:id
let email = ms.receiving.get(&id, Some(&GetReceivedEmailOptions {
    html_format: Some("cid".into()),                               // default data_uri
})).await?;
// email.extracted_text: the visible text, hidden text and invisible characters removed
// email.screening: Option<Screening { verdict, sender_trust, injection_score, … }>
// email.extracted: Option<ReceivedEmailExtracted { kinds, code, link, … }> on verification mail
ms.receiving.attachments().list(&id, None).await?;                 // each with a signed download_url (1 hour)
ms.receiving.attachments().get(&id, &attachment_id).await?;
ms.receiving.delete(&id).await?;                                   // erase now (full-access keys)

// Long-poll: wait up to 50 s for the next match
let page = ms.receiving.list(Some(&ListReceivedEmailsOptions {
    inbox_id: Some(inbox_id.clone()),
    from: Some("github.com".into()),                               // an address, or a domain and its subdomains
    wait: Some(30),
    ..Default::default()
})).await?;
// page.data: what arrived; page.held: Vec<HeldEmail { id, inbox_id, reason, quarantine_reason, created_at }>,
// matches held for screening or quarantine

// Reply and forward from the server, threaded
let reply = ReplyReceivedEmailOptions { text: Some("Thanks!".into()), ..Default::default() };
match ms.receiving.reply(&id, reply.with_idempotency_key("reply-1")).await? {
    SendOutcome::Sent(sent) => println!("sent {}", sent.id),
    SendOutcome::Draft(draft) => println!("held for approval: {:?}", draft.approval_reasons),
}
ms.receiving.forward(&id, &ForwardReceivedEmailOptions {
    to: "ops@acme.dev".into(),
    mode: Some("attachment".into()),                               // the original as an .eml
    ..Default::default()
}).await?;
```

`ListReceivedEmailsOptions` also takes `before`/`after`, `since`, `to`,
`subject_contains`, `unread` (with `inbox_id` on a team key; an inbox-scoped
key reads its own inbox), and `include_quarantined` or `quarantined` (held mail
too, or held mail only; full-access keys). An `inbox_id` that doesn't exist
answers 404 `not_found` at once, even with `wait`. Replies and forwards return
a `SendOutcome`: `Sent` once the email went out, or `Draft` when an inbox's
rules hold it for a person to approve (HTTP 202).

### Inboxes (beta)

Addresses whose mail lands in threads, for an agent (`kind: "agent"`, the
default: screening, code extraction and send rules on) or for people
(`kind: "human"`). Mirrors Resend's Inboxes beta; `safety()`, `codes()` and
`quarantine()` are MillionSend extensions.

```rust
use millionsend::{
    CreateInboxDraftOptions, CreateInboxLabelOptions, CreateInboxOptions, ForwardInThreadOptions,
    IdempotentTrait, InboxSafetyUpdate, LatestCodeOptions, ListInboxThreadsOptions,
    ReleaseQuarantinedOptions, ReplyInThreadOptions, SearchInboxThreadsOptions,
    UpdateInboxDraftOptions, UpdateInboxOptions, UpdateInboxThreadOptions,
};

let inbox = ms.inboxes.create(&CreateInboxOptions {
    name: Some("Signup bot".into()),                               // no email_address: an address on the shared domain
    username: Some("signup-bot".into()),                           // signup-bot@<team>.<shared domain>; omit it for a random one
    expires_in: Some("24h".into()),                                // "1h" to "365d"; omitted, it never expires
    ..Default::default()
}).await?;
ms.inboxes.create(&CreateInboxOptions {
    domain_id: Some(domain.id.clone()),                            // a domains.list id: orders@ that domain
    username: Some("orders".into()),
    ..Default::default()
}).await?;
ms.inboxes.list(None).await?;
ms.inboxes.get("support@acme.dev").await?;                         // by id or address
ms.inboxes.update(&inbox.id, &UpdateInboxOptions {
    name: Some("Support".into()),
    expires_in: Some(None),                                        // null: never expires
    ..Default::default()
}).await?;
ms.inboxes.delete(&inbox.id).await?;                               // also revokes its inbox-scoped keys

// Safety rules: who may write in and what an agent may send
ms.inboxes.safety().get(&inbox.id).await?;
ms.inboxes.safety().update(&inbox.id, &InboxSafetyUpdate {
    accept_from: Some("allowlist".into()),
    allowlist: Some(vec!["*@stripe.com".into()]),
    send_cap: Some(None),                                          // null: no daily cap
    ..Default::default()
}).await?;

// Verification codes and sign-in links (never fetched); waits up to 50 s
let code = ms.inboxes.codes().latest(&inbox.id, Some(&LatestCodeOptions {
    from: Some("github.com".into()),
    wait: Some(30),
    ..Default::default()
})).await?;
// code.status: "found" | "none" | "ambiguous" | "timeout"; code.code or code.url
// The next sign-in: only a code received after the one already used
ms.inboxes.codes().latest(&inbox.id, Some(&LatestCodeOptions {
    since: code.received_at.clone(),
    wait: Some(30),
    ..Default::default()
})).await?;

// Quarantine: held mail is kept and listed; agents see it only once a person releases it
ms.inboxes.quarantine().list(&inbox.id, None).await?;
ms.inboxes.quarantine().release(&inbox.id, &email_id, Some(&ReleaseQuarantinedOptions {
    trust_sender: Some(true),                                      // also allowlists the sender
})).await?;

// Threads and their messages
ms.inboxes.threads().list(&inbox.id, Some(&ListInboxThreadsOptions {
    folders: Some(vec!["inbox".into(), "archive".into()]),         // sent comma-separated
    read: Some(false),
    include_quarantined: true,                                     // owner views: each row's `quarantined` count
    ..Default::default()
})).await?;
ms.inboxes.threads().search(&inbox.id, &SearchInboxThreadsOptions {
    query: Some("invoice".into()),
    from: Some(vec!["stripe.com".into()]),
    ..Default::default()
}).await?;
ms.inboxes.threads().get(&inbox.id, &thread_id).await?;
ms.inboxes.threads().update(&inbox.id, &thread_id, &UpdateInboxThreadOptions {
    folder: Some("archive".into()),
    ..Default::default()
}).await?;
ms.inboxes.threads().delete(&inbox.id, &thread_id).await?;         // to trash, purged after 30 days
let emails = ms.inboxes.threads().emails();
emails.list(&inbox.id, &thread_id, None).await?;                   // oldest first
emails.get(&inbox.id, &thread_id, &email_id, None).await?;         // both take include_quarantined (owner views)
let reply = ReplyInThreadOptions { text: Some("On it.".into()), ..Default::default() };
emails.reply(&inbox.id, &thread_id, &email_id, reply.with_idempotency_key("k1")).await?;
emails.forward(&inbox.id, &thread_id, &email_id, &ForwardInThreadOptions {
    to: "ops@acme.dev".into(),
    ..Default::default()
}).await?;

// Labels
let label = ms.inboxes.labels().create(&inbox.id, &CreateInboxLabelOptions {
    name: "Billing".into(),
    color: Some("#FFAA00".into()),
}).await?;
ms.inboxes.labels().list(&inbox.id).await?;                        // also update and delete

// Drafts
let draft = ms.inboxes.drafts().create(&inbox.id, &CreateInboxDraftOptions {
    to: Some("ada@x.dev".into()),
    subject: Some("Hello".into()),
    text: Some("Hi Ada".into()),
    ..Default::default()
}).await?;
ms.inboxes.drafts().update(&inbox.id, &draft.id, &UpdateInboxDraftOptions {
    subject: Some(None),                                           // null clears
    ..Default::default()
}).await?;
ms.inboxes.drafts().send(&inbox.id, &draft.id, Some("draft-1")).await?; // Idempotency-Key
ms.inboxes.drafts().reject(&inbox.id, &draft.id).await?;           // approvers only
```

Thread replies, forwards and `drafts().send` return a `SendOutcome` like
`receiving.reply`. An agent's reply to a Reply-To at another organization is
held that way too, with `reply_to_redirect` among its `approval_reasons`. An
inbox-scoped key or an MCP token cannot send a draft that waits for approval
(`approval_required`); a person in the dashboard or a full-access team key can.
Resend's hosted agent settings (`/inboxes/:id/agent`) are not exposed:
MillionSend runs no agent.

An inbox never expires unless `expires_in` says so, and its `name` takes up to
320 characters. `domain_id` takes a `domains.list` row instead of
`email_address`: the `shared: true` row works like leaving both out, and one of
your own domains needs a `username`.

On the shared domain, an inbox unused for 90 days (no mail in or out, no API
call on it) is deleted, which fires `inbox.expired`, then `inbox.deleted`. A
chosen `username` or `from_name` there is reviewed first: reserved names
(`postmaster`, `admin`, `support`, `noreply`, …), a `fwd-` prefix and
look-alikes of brands answer 422, a username in use 409, and 429 means today's
review budget is spent. 503 `name_review_unavailable` means try again after
`retry_after` seconds: the review is down, or the team's new subdomain still
waits for its own, and then every shared or forwarding create gets it. Safety
rules differ there too: `send_policy: "allowlist"` answers 422, and so does
`"open"` unless the team's `sending_mode` is `open`; `owner_addresses` don't
make senders trusted; `forward_to` takes team members only.

#### Sending from the shared domain

What the team's shared addresses may send is the `sending_mode` of the
`shared: true` row of `domains.list`: `open` starts conversations, `reply_only`
only answers inside a thread to its own correspondents, and `receive_only`
sends nothing. The same answers come back from `emails.send`, replies,
forwards and drafts:

- 403 `inbox_send_policy`: the mode doesn't allow this send (or the inbox's
  name looks like a brand, which only replies even under `open`);
- 429 `inbox_send_cap_reached`: the team's daily cap for the shared domain,
  with a `retry_after` that runs to the next UTC midnight;
- 422 `validation_error`: more than 10 distinct recipients across `to`, `cc`
  and `bcc`.

`emails.send` takes a shared address in `open` mode only: the inbox's exact
`email_address` (no +tag), a display name left out or equal to its
`from_name`, and no `reply_to`, `attachments`, `headers` or `scheduled_at`. A
key restricted to a domain gets 403 `restricted_api_key`. Batch items and
broadcasts never send from the shared domain. Match on `name` and
`status_code`; the messages may change.

### Contacts

Contacts are team-global — one record per email address (case-insensitive);
creating a duplicate is a 409 `validation_error`.

```rust
use millionsend::{
    ContactAddress, ContactInclude, ContactTopicUpdate, CreateContactOptions,
    DeleteContactOptions, ListContactsOptions, SegmentRef, TopicSubscription,
    UpdateContactOptions,
};

ms.contacts.create(&CreateContactOptions {
    email: "ada@acme.dev".into(),
    first_name: Some("Ada".into()),
    properties: Some([("plan".to_string(), "pro".into())].into()),
    segments: Some(vec![SegmentRef { id: segment.id.clone() }]),
    topics: Some(vec![ContactTopicUpdate {
        id: topic.id.clone(),
        subscription: TopicSubscription::OptIn,
    }]),
    ..Default::default()
}).await?;

// Address by id (a bare &str) or email; email wins if both are set.
ms.contacts.get("contact-id").await?;
ms.contacts.get(ContactAddress::email("ada@acme.dev")).await?;

// null clears a field, omitted leaves it unchanged.
ms.contacts.update("contact-id", &UpdateContactOptions {
    first_name: Some(None),        // clear
    unsubscribed: Some(true),      // set
    ..Default::default()
}).await?;

ms.contacts.delete(ContactAddress::email("ada@acme.dev"), None).await?; // the contact's emails stay in the send log
ms.contacts.delete("contact-id", Some(&DeleteContactOptions { erase: true })).await?; // ?erase=true — also scrubs the address from email history, event payloads and API logs (GDPR/LGPD)
ms.contacts.list(Some(&ListContactsOptions { limit: Some(20), ..Default::default() })).await?;
// Bulk read (MillionSend extension): attach properties and topic subscriptions to every item,
// so an audience reads in one request per 100 contacts instead of one per contact.
ms.contacts.list(Some(&ListContactsOptions {
    limit: Some(100),
    include: Some(vec![ContactInclude::Properties, ContactInclude::Topics]), // ?include=properties,topics
    ..Default::default()
})).await?;
```

`Contact.properties` values arrive as typed wrappers on the wire
(`{ "type": "string", "value": "pro" }` / `{ "type": "number", "value": 3 }`).
With `include`, each `ContactListItem` carries the same `properties` map and
the same `topics` rows as `contacts.get` / `contacts.topics.list`; without it
both are `None`. `segments.list_contacts` takes the same options.

#### Batch create, get and remove (MillionSend extensions)

```rust
use millionsend::{BatchContactsOptions, BatchRemoveContactsOptions, BatchValidation, DeleteContactOptions, OnConflict};

let res = ms.contacts.create_batch(&contacts, Some(&BatchContactsOptions {
    on_conflict: Some(OnConflict::Upsert),                 // error (default) | skip | upsert
    batch_validation: Some(BatchValidation::Permissive),
})).await?;                                                // POST /contacts/batch?on_conflict=upsert
println!("{} created, {} failed", res.counts.created, res.counts.failed);
for err in &res.errors { eprintln!("contacts.{}: {}", err.index, err.message); }
```

Up to 1000 contacts per call; each `data` entry carries the request `index`,
the contact `id` and a `status` (`created` | `updated` | `skipped`). Each
`errors` entry (`BatchContactError`) also carries the `status_code` and `name`
strict mode would answer for the item.

```rust
ms.contacts.batch_remove(&BatchRemoveContactsOptions::Ids(vec![id]), None).await?;              // POST /contacts/batch/remove
ms.contacts.batch_remove(&BatchRemoveContactsOptions::Emails(vec!["a@acme.dev".into()]), None).await?;
ms.contacts.batch_remove(&BatchRemoveContactsOptions::Ids(vec![id]), Some(&DeleteContactOptions { erase: true })).await?; // { "ids": [...], "erase": true }
```

Exactly one of ids or emails, up to 1000; `data` lists only the contacts
actually deleted (`{ object, contact, deleted }`), unknown ones are skipped.
Either way the contacts' emails stay in the send log; `erase: true` also scrubs
each address from email history, event payloads and API logs (a GDPR/LGPD
erasure). Needs MillionSend v0.6.54.

```rust
use millionsend::{BatchGetContactsOptions, ContactAddress, ContactInclude};

let res = ms.contacts.batch_get(
    &["contact-id".into(), ContactAddress::email("ada@acme.dev")],
    Some(&BatchGetContactsOptions { include: Some(vec![ContactInclude::Topics]) }),
).await?;                                                  // POST /contacts/batch/get
res.data;    // the contacts found, in request order, each with `topics` attached
res.missing; // [{ index, id?, email? }] — request entries that matched nobody
```

Up to 1000 addresses per call, ids and emails mixed (emails match
case-insensitively), and one request against the rate limit; unknown entries
land in `missing` instead of failing the call.

#### Topic subscriptions, segment membership and the preference page

```rust
use millionsend::{ContactTopicUpdate, TopicSubscription};

ms.contacts.topics.list("contact-id").await?;                   // GET   /contacts/:id/topics
ms.contacts.topics.update("contact-id", &[ContactTopicUpdate {
    id: "topic-id".into(),
    subscription: TopicSubscription::OptOut,
}]).await?;                                                     // PATCH /contacts/:id/topics

ms.contacts.segments.add("contact-id", &segment.id).await?;     // POST   /contacts/:id/segments/:segmentId
ms.contacts.segments.remove("contact-id", &segment.id).await?;  // DELETE /contacts/:id/segments/:segmentId

let link = ms.contacts.preferences_link(ContactAddress::email("ada@acme.dev")).await?;
println!("{}", link.url);                                       // POST /contacts/:idOrEmail/preferences-link
```

`topics.list` returns every topic of the team (one page) with the contact's
effective `subscription` — the explicit choice, else the topic default —
`explicit: false` when it is the default, and the topic's `visibility` (the
hosted preference page lists public topics only).

`preferences_link` (MillionSend extension) mints the contact's hosted
preference page — the page their emails' unsubscribe links open. The URL never
expires and lets its holder change that contact's preferences, so show it only
to the contact. 422 when the instance cannot build hosted links.

#### Contact properties

Typed definitions for the keys of `contact.properties`.

```rust
use millionsend::{ContactPropertyType, CreateContactPropertyOptions, UpdateContactPropertyOptions};
use serde_json::{json, Value};

let plan = ms.contacts.properties.create(&CreateContactPropertyOptions {
    key: "plan".into(),
    r#type: ContactPropertyType::String,
    fallback_value: Some(json!("free")),
}).await?;                                                 // POST /contact-properties
ms.contacts.properties.get(&plan.id).await?;
ms.contacts.properties.list(None).await?;
ms.contacts.properties.update(&plan.id, &UpdateContactPropertyOptions {
    fallback_value: Some(Value::Null),                     // null clears the fallback
}).await?;
ms.contacts.properties.delete(&plan.id).await?;
```

### Topics

```rust
use millionsend::{CreateTopicOptions, TopicSubscription, TopicVisibility, UpdateTopicOptions};

let mut topic = CreateTopicOptions::new("Product updates", TopicSubscription::OptIn);
topic.visibility = Some(TopicVisibility::Public);
ms.topics.create(&topic).await?;
ms.topics.get(&id).await?;
ms.topics.list().await?;
ms.topics.update(&id, &UpdateTopicOptions { name: Some("News".into()), ..Default::default() }).await?;
ms.topics.delete(&id).await?;
```

### Broadcasts

Target a segment (`segment_id`) and/or a topic's subscribers (`topic_id`);
with neither set, the broadcast goes to every contact.

```rust
use millionsend::{CreateBroadcastOptions, UpdateBroadcastOptions};

let broadcast = ms.broadcasts.create(&CreateBroadcastOptions {
    name: Some("Launch".into()),
    segment_id: Some(segment.id.clone()),
    from: "Acme <news@acme.dev>".into(),
    subject: "Launch".into(),
    html: Some("<p>Hi {{{FIRST_NAME|there}}}</p>".into()),
    preview_text: Some("It's here".into()),
    topic_id: Some(topic.id.clone()),
    send: Some(true),                                      // send now instead of saving a draft
    scheduled_at: Some("in 1 hour".into()),                // with send: true — schedule instead
    ..Default::default()
}).await?;

ms.broadcasts.list(None).await?;
ms.broadcasts.get(&broadcast.id).await?;
ms.broadcasts.update(&broadcast.id, &UpdateBroadcastOptions {
    subject: Some("Launch 🚀".into()),
    clear_topic_id: true,                                  // sends "topic_id": null (detach)
    ..Default::default()
}).await?;                                                 // draft only
ms.broadcasts.send(&broadcast.id, Some("2026-09-01T09:00:00Z")).await?;  // None = send now
ms.broadcasts.cancel(&broadcast.id).await?;                // scheduled only
ms.broadcasts.delete(&broadcast.id).await?;                // draft only
```

`subject` takes up to 998 characters. A broadcast never sends from the shared
domain: `create` with `send: Some(true)` and `send` answer 403
`inbox_send_policy`.

### Segments (MillionSend extension)

A segment is either a saved filter over the team's contacts or, with no
filter, a manual list fed by `contacts.segments.add`. `Segment.filter` is
`None` for manual segments.

```rust
use millionsend::{
    CreateSegmentOptions, SegmentCondition, SegmentFilter, SegmentMatch, UpdateSegmentOptions,
};

let segment = ms.segments.create(&CreateSegmentOptions {
    name: "Pro plan".into(),
    filter: Some(SegmentFilter {
        match_: SegmentMatch::All,
        conditions: vec![SegmentCondition {
            field: "property:plan".into(),
            op: "equals".into(),
            value: Some("pro".into()),
        }],
    }),
}).await?;
let vips = ms.segments.create(&CreateSegmentOptions {
    name: "VIPs".into(),
    filter: None,                            // manual segment: members come from contacts.segments.add
}).await?;

ms.segments.get(&id).await?;                 // includes a live contact_count
ms.segments.list(None).await?;
ms.segments.list_contacts(&id, None).await?; // GET /segments/:id/contacts — takes ListContactsOptions (include works here too)
ms.segments.update(&id, &UpdateSegmentOptions {
    filter: Some(None),                      // null drops the filter, keeping the members added by hand
    ..Default::default()
}).await?;
ms.segments.delete(&id).await?;
```

### Suppressions

Addresses the team never sends to. Entries are addressable by id or email.

```rust
use millionsend::{
    AddSuppressionOptions, BatchAddSuppressionOptions, BatchRemoveSuppressionOptions,
    ListSuppressionsOptions, SuppressionOrigin,
};

ms.suppressions.add(&AddSuppressionOptions::new("bounced@example.com")).await?;   // POST /suppressions
ms.suppressions.get("bounced@example.com").await?;                                // GET /suppressions/:idOrEmail
ms.suppressions.list(Some(&ListSuppressionsOptions {
    origin: Some(SuppressionOrigin::Complaint),                                    // bounce | complaint | manual | unsubscribe
    ..Default::default()
})).await?;
ms.suppressions.remove("bounced@example.com").await?;                             // DELETE /suppressions/:idOrEmail

ms.suppressions.batch_add(&BatchAddSuppressionOptions {
    emails: vec!["a@example.com".into(), "b@example.com".into()],                 // up to 1000
    origin: Some(SuppressionOrigin::Manual),
}).await?;
ms.suppressions.batch_remove(&BatchRemoveSuppressionOptions::Emails(vec!["a@example.com".into()])).await?;
ms.suppressions.batch_remove(&BatchRemoveSuppressionOptions::Ids(vec![id])).await?;
```

### Domains

```rust
use millionsend::{CreateDomainOptions, UpdateDomainOptions};

let domain = ms.domains.create(&CreateDomainOptions {
    name: "acme.dev".into(),
    open_tracking: Some(true),
    click_tracking: Some(true),
    tracking_subdomain: Some("links".into()),              // links.acme.dev
    ..Default::default()                                   // region/custom_return_path: deployment defaults
}).await?;
for record in &domain.records {
    println!("{} {} {}", record.r#type, record.name, record.value);
}

ms.domains.list(None).await?;                              // items carry no records
ms.domains.get(&domain.id).await?;
ms.domains.verify(&domain.id).await?;                      // POST /domains/:id/verify
ms.domains.update(&domain.id, &UpdateDomainOptions {
    click_tracking: Some(false),
    tracking_subdomain: Some(None),                        // null clears the branded host
    ..Default::default()
}).await?;
ms.domains.update(&domain.id, &UpdateDomainOptions {
    receiving_catch_all: Some(true),                       // keep mail to addresses without an inbox (paid plans on Cloud)
    ..Default::default()
}).await?;
ms.domains.delete(&domain.id).await?;
```

On an instance with a shared inbox domain, the team's subdomain of it is the
last row of `domains.list` (`shared: true`; MillionSend v0.14.0 and later). It
is verified, has no DNS records, and its id is the `domain_id` of the team's
shared inboxes. Its `sending_mode` (`open`, `reply_only` or `receive_only`)
says what those inboxes may send, and `sending_mode_reason` (`plan` or
`instance`, `None` under `open`) why it is held below `open`. It is read-only:
`update`, `verify` and `delete` answer 403 `forbidden`, and an API key can't be
restricted to it. It comes after every own domain whatever its `created_at`,
alone on the next page when they exactly fill one. A team whose new subdomain
still waits for its name review has no such row yet. Older servers send no
`shared`: it reads as `false`.

It sends only from its inboxes' own addresses, so leave it out when picking a
domain to send from:

```rust
use millionsend::ListOptions;

let domains = ms.domains.list(Some(&ListOptions { limit: Some(100), ..Default::default() })).await?;
let sender = domains.data.iter().find(|d| d.status == "verified" && !d.shared);
```

### Webhooks

```rust
use millionsend::{CreateWebhookOptions, RotateWebhookOptions, UpdateWebhookOptions, WebhookStatus};

let hook = ms.webhooks.create(&CreateWebhookOptions {
    endpoint: "https://acme.dev/hooks/millionsend".into(),
    events: vec!["email.delivered".into(), "email.bounced".into(), "deliverability.paused".into()],
    signing_secret: None,                                  // minted; or pass your own whsec_…
    inbox_ids: None,                                       // or Some(ids): inbox events of these inboxes only
}).await?;
println!("verify payloads with {}", hook.signing_secret);

ms.webhooks.list(None).await?;
ms.webhooks.get(&hook.id).await?;                          // includes signing_secret, previous_secret_expires_at
ms.webhooks.update(&hook.id, &UpdateWebhookOptions {
    status: Some(WebhookStatus::Disabled),
    inbox_ids: Some(None),                                 // null: events of every inbox
    ..Default::default()
}).await?;

let rotated = ms.webhooks.rotate(&hook.id, None).await?;   // POST /webhooks/:id/rotate with {}
println!("switch to {} before {:?}", rotated.signing_secret, rotated.previous_secret_expires_at);
ms.webhooks.rotate(&hook.id, Some(&RotateWebhookOptions {
    signing_secret: Some("whsec_…".into()),                // bring your own
    overlap_hours: Some(0),                                // 0–72, default 24; 0 drops the old secret at once
})).await?;
ms.webhooks.delete(&hook.id).await?;
```

`rotate` (MillionSend extension) mints or takes a new signing secret; until
`previous_secret_expires_at` every delivery carries both signatures, so the
receiver can switch at any point in the window. Subscribable event names
include `email.*`, `deliverability.*`, `contact.created`/`updated`/`deleted`/
`unsubscribed`/`resubscribed`/`topic_opt_in`/`topic_opt_out`,
`suppression.added`/`removed`, `email.received`, and the opt-in inbox events
`inbox.created`, `inbox.updated`, `inbox.deleted`, `inbox.expired`,
`inbox.thread.created`, `inbox.thread.folder.updated`,
`inbox.thread.labels.updated`, `inbox.email.received`, `inbox.email.sent`,
`inbox.email.quarantined`, `inbox.draft.created`, `inbox.draft.updated`,
`inbox.draft.sent` and `inbox.draft.deleted`, which reach an endpoint only when
its `events` name them. Inbox events from sends made in the dashboard carry
`data.source: "dashboard"`.

### API keys

```rust
use millionsend::{ApiKeyPermission, CreateApiKeyOptions};

let key = ms.api_keys.create(&CreateApiKeyOptions {
    name: "ci".into(),
    permission: Some(ApiKeyPermission::SendingAccess),     // default full_access
    domain_id: Some(domain.id.clone()),                    // restrict sending to one domain
    inbox_id: None,                                        // or a full_access key for one inbox only
}).await?;
println!("{}", key.token);                                 // shown once
ms.api_keys.list(None).await?;
ms.api_keys.delete(&key.id).await?;
```

`domain_id` takes one of your own domains: the `shared: true` row of
`domains.list` answers 422 `validation_error`.

### Templates

Addressable by id or alias.

```rust
use millionsend::{CreateTemplateOptions, UpdateTemplateOptions};

let mut welcome = CreateTemplateOptions::new("Welcome", "<p>Hi {{{FIRST_NAME|there}}}</p>");
welcome.subject = Some("Welcome!".into());
welcome.alias = Some("welcome".into());
let created = ms.templates.create(&welcome).await?;

ms.templates.get("welcome").await?;
ms.templates.list(None).await?;
ms.templates.update("welcome", &UpdateTemplateOptions {
    subject: Some(Some("Hello".into())),                   // set
    alias: Some(None),                                     // null clears
    ..Default::default()
}).await?;
ms.templates.publish(&created.id).await?;                  // no-op kept for Resend compatibility
ms.templates.duplicate(&created.id).await?;
ms.templates.delete(&created.id).await?;
```

`from`, `reply_to` and `variables` are passed through; the API currently
answers 422 when they are set.

### Deliverability (MillionSend extension)

Account-level score over the trailing 30 days; scores are `None` until there is
enough data.

```rust
let report = ms.deliverability.get().await?;   // GET /deliverability
if let Some(score) = report.score {
    println!("{score} ({})", report.band.as_deref().unwrap_or("-"));
}
```

### Usage (MillionSend extension)

```rust
let usage = ms.usage.get().await?;             // GET /usage
println!("{} sent today, resets {}", usage.today.emails_sent, usage.today.resets_at);
if let Some(cap) = usage.limits.emails_per_day {
    println!("plan {:?} caps at {cap}/day", usage.plan);
}
```

## Server versions

Older servers leave newer response fields out (`Domain.shared` then reads as
`false`) and ignore request fields they don't know. Each feature needs at
least:

| Feature | MillionSend |
|---|---|
| `receiving` | v0.8.0 |
| `inboxes` | v0.10.0 |
| 422 for recipients that can never receive mail; a batch `Idempotency-Key` replayed before validation | v0.11.0 |
| `username`; inboxes that never expire by default; `emails.send` from a shared inbox; `name_review_unavailable`; `status_code`/`name` on contacts batch errors | v0.12.0 |
| Newest-first lists; JSON 404 for unknown routes; `since` on `codes().latest`; `quarantined` on thread rows; 404 at once for an unknown `inbox_id`; `unread` with an inbox-scoped key | v0.13.0 |
| The shared row of `domains.list` (`shared`, `sending_mode`, `sending_mode_reason`); `domain_id` on `inboxes.create`; `subject` up to 998 characters | v0.14.0 |

## Migrating from Resend

```diff
- use resend_rs::{Resend, types::CreateEmailBaseOptions};
- let resend = Resend::new("re_123");
+ use millionsend::{MillionSend, SendEmailOptions};
+ let ms = MillionSend::new("ms_123"); // self-hosted: with_base_url("ms_123", "https://mail.acme.dev")
```

Resource and method names match: `emails`, `batch`, `receiving`, `contacts`,
`topics`, `broadcasts`, `segments`, `suppressions`, `domains`, `webhooks`,
`api_keys`, `templates`. Notes:

- **Contacts nest** their sub-resources — `contacts.topics.list`/`update`,
  `contacts.segments.add`/`remove`, `contacts.properties.*` — where `resend-rs`
  keeps them flat (`get_contact_topics`, `update_contact_topics`,
  `add_contact_segment`, `create_property`, …).

- **No audiences.** Contacts are team-global; the API's `/audiences/*` routes
  are a compatibility shim and are not part of this SDK. Use `segments` (saved
  filters or manual lists) to target a subset, or a broadcast with no
  `segment_id`/`topic_id` to reach everyone.
- **Received mail** attachments hang off `receiving.attachments()` where
  `resend-rs` has `get_attachment`/`list_attachments`; `receiving.forward` is a
  server-side forward taking `ForwardReceivedEmailOptions`.
- **MillionSend extensions** with no Resend counterpart: `segments`,
  `contacts.create_batch`, `contacts.batch_get`, `contacts.batch_remove`,
  `contacts.preferences_link`,
  `webhooks.rotate`, `deliverability`, `usage`, `emails.get_insights`, the
  receiving long-poll, `receiving.reply`/`delete`, `inboxes.safety()`,
  `codes()` and `quarantine()`, the `username` and `domain_id` of
  `inboxes.create`, and `Error::retry_after`.
- **Templates** are always published; `publish` is a no-op kept for
  compatibility, and `from`/`reply_to`/`variables` are rejected with 422.
- **Nullable clears**: `Option<Option<T>>` fields (`Some(None)`) send JSON
  `null`; `UpdateBroadcastOptions::clear_topic_id` does the same for
  `topic_id`.

## License

MIT
