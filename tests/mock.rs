//! Unit tests over a mocked HTTP layer (wiremock). Each test mounts strict
//! matchers (method + path + body + headers); a mismatch yields a 404 the SDK
//! surfaces as an error, failing the `unwrap`. So an `Ok` result is itself proof
//! the request was shaped correctly.

use millionsend::*;
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{body_json, body_string, header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok_json(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

// ---- transport -----------------------------------------------------------

#[tokio::test]
async fn refuses_non_loopback_http_unless_allowed() {
    let ms = MillionSend::with_base_url("ms_test", "http://mail.invalid");
    let err = ms.emails.get("e1").await.unwrap_err();
    assert_eq!(err.name(), Some("insecure_base_url"));
    assert_eq!(err.status_code(), None);

    // Opted in: the request leaves the SDK (and fails at the transport, not the guard).
    let err = ms.allow_insecure_http().emails.get("e1").await.unwrap_err();
    assert!(matches!(err, Error::Http(_)), "got {err}");
}

#[tokio::test]
async fn with_client_applies_custom_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/e1"))
        .respond_with(ok_json(json!({ "id": "e1" })).set_delay(Duration::from_millis(500)))
        .mount(&server)
        .await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let ms = MillionSend::with_base_url("ms_test", server.uri()).with_client(client);
    let err = ms.emails.get("e1").await.unwrap_err();
    match err {
        Error::Http(e) => assert!(e.is_timeout(), "{e}"),
        other => panic!("expected a timeout, got {other}"),
    }
}

// ---- emails --------------------------------------------------------------

#[tokio::test]
async fn emails_send_maps_body_headers_and_idempotency() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(header("authorization", "Bearer ms_test"))
        .and(header("accept", "application/json"))
        .and(header("content-type", "application/json"))
        .and(header("idempotency-key", "key-123"))
        .and(body_json(json!({
            "from": "a@x.dev",
            "to": ["b@x.dev"],
            "subject": "s",
            "html": "<p>h</p>",
            "reply_to": "r@x.dev",
            "scheduled_at": "2999-01-01T00:00:00Z"
        })))
        .respond_with(ok_json(json!({ "id": "abc" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let email = SendEmailOptions {
        from: "a@x.dev".into(),
        to: vec!["b@x.dev".to_string()].into(),
        subject: "s".into(),
        html: Some("<p>h</p>".into()),
        reply_to: Some("r@x.dev".into()),
        scheduled_at: Some("2999-01-01T00:00:00Z".into()),
        ..Default::default()
    };
    let res = ms
        .emails
        .send_with_idempotency_key(&email, "key-123")
        .await
        .unwrap();
    assert_eq!(res.id, "abc");
}

#[tokio::test]
async fn emails_send_omits_none_and_sends_no_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        // Single recipient serializes as a bare string; cc/bcc/html/tags omitted.
        .and(body_json(json!({
            "from": "a@x.dev",
            "to": "b@x.dev",
            "subject": "s",
            "text": "t"
        })))
        .respond_with(ok_json(json!({ "id": "abc" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let mut email = SendEmailOptions::new("a@x.dev", "b@x.dev", "s");
    email.text = Some("t".into());
    ms.emails.send(&email).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.get("idempotency-key").is_none());
    let ua = requests[0]
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ua.starts_with("millionsend-rust/"), "user agent: {ua}");
}

#[tokio::test]
async fn emails_get_and_cancel_hit_the_right_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/e1"))
        .respond_with(ok_json(json!({
            "object": "email", "id": "e1", "from": "a@x.dev", "to": ["b@x.dev"],
            "cc": null, "bcc": null, "reply_to": null, "subject": "s",
            "html": null, "text": "t", "created_at": "2026-01-01T00:00:00Z",
            "scheduled_at": null, "message_id": "m1", "last_event": "delivered",
            "score": 8.5
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/emails/e1/cancel"))
        .respond_with(ok_json(json!({ "object": "email", "id": "e1" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let email = ms.emails.get("e1").await.unwrap();
    assert_eq!(email.message_id, "m1");
    assert_eq!(email.score, Some(8.5));
    let cancelled = ms.emails.cancel("e1").await.unwrap();
    assert_eq!(cancelled.id, "e1");
}

#[tokio::test]
async fn emails_get_score_null_maps_to_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/e2"))
        .respond_with(ok_json(json!({
            "object": "email", "id": "e2", "from": "a@x.dev", "to": ["b@x.dev"],
            "cc": null, "bcc": null, "reply_to": null, "subject": "s",
            "html": null, "text": "t", "created_at": "2026-01-01T00:00:00Z",
            "scheduled_at": null, "message_id": "m2", "last_event": "sent",
            "score": null
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    assert_eq!(ms.emails.get("e2").await.unwrap().score, None);
}

#[tokio::test]
async fn emails_get_insights_maps_full_report() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/e1/insights"))
        .respond_with(ok_json(json!({
            "object": "email_insights",
            "email_id": "e1",
            "score": 8.5,
            "score_version": 1,
            "band": "excellent",
            "marketing": true,
            "html_size_bytes": 12345,
            "computed_at": "2026-01-01T00:00:00Z",
            "checks": [
                { "id": "has_unsubscribe", "severity": "critical", "status": "fail",
                  "penalty": 1.25, "detail": { "reason": "no List-Unsubscribe", "count": 2 } },
                { "id": "plain_text_part", "severity": "minor", "status": "pass", "penalty": 0 },
                // The check catalog and enums grow across score versions; unknown
                // future values must deserialize, not throw.
                { "id": "some_future_check", "severity": "cosmic", "status": "soft_fail",
                  "penalty": 0.5 }
            ]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let insights = ms.emails.get_insights("e1").await.unwrap();
    assert_eq!(insights.object, "email_insights");
    assert_eq!(insights.email_id, "e1");
    assert_eq!(insights.score, 8.5);
    assert_eq!(insights.score_version, 1);
    assert_eq!(insights.band, "excellent");
    assert!(insights.marketing);
    assert_eq!(insights.html_size_bytes, Some(12345));
    assert_eq!(insights.computed_at, "2026-01-01T00:00:00Z");
    assert_eq!(insights.checks.len(), 3);

    let failed = &insights.checks[0];
    assert_eq!(failed.id, "has_unsubscribe");
    assert_eq!(failed.severity, "critical");
    assert_eq!(failed.status, "fail");
    assert_eq!(failed.penalty, 1.25);
    let detail = failed.detail.as_ref().unwrap();
    assert_eq!(detail["reason"], json!("no List-Unsubscribe"));
    assert_eq!(detail["count"], json!(2));

    assert!(insights.checks[1].detail.is_none());
    assert_eq!(insights.checks[2].status, "soft_fail");
}

#[tokio::test]
async fn emails_get_insights_404_surfaces_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/nope/insights"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "statusCode": 404, "name": "not_found", "message": "Email not found"
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let err = ms.emails.get_insights("nope").await.unwrap_err();
    assert_eq!(err.status_code(), Some(404));
    assert_eq!(err.name(), Some("not_found"));
}

// ---- deliverability ------------------------------------------------------

#[tokio::test]
async fn deliverability_get_maps_full_report() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/deliverability"))
        .respond_with(ok_json(json!({
            "object": "deliverability",
            "score": 8.7, "band": "good",
            "content_score": 8.2, "outcome_score": 9.1,
            "complaint_rate": 0.0002, "hard_bounce_rate": 0.001,
            "emails_sent": 12345, "scored_recipients": 23456,
            "window_days": 30, "insufficient_outcome_data": false,
            "guardrail_status": "ok",
            "score_version": 1
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let report = ms.deliverability.get().await.unwrap();
    assert_eq!(report.object, "deliverability");
    assert_eq!(report.score, Some(8.7));
    assert_eq!(report.band.as_deref(), Some("good"));
    assert_eq!(report.content_score, Some(8.2));
    assert_eq!(report.outcome_score, Some(9.1));
    assert_eq!(report.complaint_rate, 0.0002);
    assert_eq!(report.hard_bounce_rate, 0.001);
    assert_eq!(report.emails_sent, 12345);
    assert_eq!(report.scored_recipients, 23456);
    assert_eq!(report.window_days, 30);
    assert!(!report.insufficient_outcome_data);
    assert_eq!(report.guardrail_status, "ok");
    assert_eq!(report.score_version, 1);
}

#[tokio::test]
async fn deliverability_get_maps_null_scores() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/deliverability"))
        .respond_with(ok_json(json!({
            "object": "deliverability",
            "score": null, "band": null,
            "content_score": null, "outcome_score": null,
            "complaint_rate": 0.0, "hard_bounce_rate": 0.0,
            "emails_sent": 0, "scored_recipients": 0,
            "window_days": 30, "insufficient_outcome_data": true,
            "guardrail_status": "ok",
            "score_version": 1
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let report = ms.deliverability.get().await.unwrap();
    assert_eq!(report.score, None);
    assert_eq!(report.band, None);
    assert_eq!(report.content_score, None);
    assert_eq!(report.outcome_score, None);
    assert!(report.insufficient_outcome_data);
}

#[tokio::test]
async fn batch_send_posts_bare_array_with_idempotency() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails/batch"))
        .and(header("idempotency-key", "batch-1"))
        .and(body_json(json!([
            { "from": "a@x.dev", "to": "b@x.dev", "subject": "1", "text": "one" },
            { "from": "a@x.dev", "to": "c@x.dev", "subject": "2", "text": "two" }
        ])))
        .respond_with(ok_json(json!({ "data": [{ "id": "1" }, { "id": "2" }] })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let mut one = SendEmailOptions::new("a@x.dev", "b@x.dev", "1");
    one.text = Some("one".into());
    let mut two = SendEmailOptions::new("a@x.dev", "c@x.dev", "2");
    two.text = Some("two".into());
    let res = ms
        .batch
        .send_with_idempotency_key(&[one, two], "batch-1")
        .await
        .unwrap();
    assert_eq!(res.data.len(), 2);
}

// ---- contacts ------------------------------------------------------------

#[tokio::test]
async fn contacts_create_posts_top_level() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contacts"))
        .and(body_json(
            json!({ "email": "c@x.dev", "first_name": "Ada" }),
        ))
        .respond_with(ok_json(json!({ "object": "contact", "id": "c1" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let contact = CreateContactOptions {
        email: "c@x.dev".into(),
        first_name: Some("Ada".into()),
        ..Default::default()
    };
    assert_eq!(ms.contacts.create(&contact).await.unwrap().id, "c1");
}

#[tokio::test]
async fn contacts_address_by_id_and_email() {
    let server = MockServer::start().await;
    let contact = |id: &str| {
        json!({
            "object": "contact", "id": id, "email": "c@x.dev",
            "first_name": null, "last_name": null,
            "created_at": "2026-01-01T00:00:00Z", "unsubscribed": false,
            "properties": {}
        })
    };
    Mock::given(method("GET"))
        .and(path("/contacts/c1"))
        .respond_with(ok_json(contact("c1")))
        .mount(&server)
        .await;
    // Email is percent-encoded like encodeURIComponent (`@` -> `%40`); allow
    // either form so the assertion is robust to the mock's path decoding.
    Mock::given(method("GET"))
        .and(path_regex(r"^/contacts/c(%40|@)x\.dev$"))
        .respond_with(ok_json(contact("c1")))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    assert_eq!(ms.contacts.get("c1").await.unwrap().id, "c1");
    assert_eq!(
        ms.contacts
            .get(ContactAddress::email("c@x.dev"))
            .await
            .unwrap()
            .email,
        "c@x.dev"
    );
}

#[tokio::test]
async fn contacts_update_sends_only_provided_keys_null_clears() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/contacts/c1"))
        .and(body_json(
            json!({ "first_name": null, "unsubscribed": true }),
        ))
        .respond_with(ok_json(json!({ "object": "contact", "id": "c1" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let changes = UpdateContactOptions {
        first_name: Some(None),
        unsubscribed: Some(true),
        ..Default::default()
    };
    assert_eq!(ms.contacts.update("c1", &changes).await.unwrap().id, "c1");
}

#[tokio::test]
async fn contacts_delete_and_list() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/contacts/c2"))
        .and(query_param("erase", "true"))
        .respond_with(ok_json(json!({
            "object": "contact", "contact": "c2", "deleted": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/contacts/c(%40|@)x\.dev$"))
        .respond_with(ok_json(json!({
            "object": "contact", "contact": "c1", "deleted": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/contacts"))
        .and(query_param("after", "cur"))
        .respond_with(ok_json(
            json!({ "object": "list", "data": [], "has_more": false }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    assert!(
        ms.contacts
            .delete(ContactAddress::email("c@x.dev"), None)
            .await
            .unwrap()
            .deleted
    );
    let erased = ms
        .contacts
        .delete("c2", Some(&DeleteContactOptions { erase: true }))
        .await
        .unwrap();
    assert_eq!(erased.contact, "c2");
    let options = ListContactsOptions {
        after: Some("cur".into()),
        ..Default::default()
    };
    ms.contacts.list(Some(&options)).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.query(), None);
    assert_eq!(requests[1].url.query(), Some("erase=true"));
    assert_eq!(requests[2].url.query(), Some("after=cur"));
}

#[tokio::test]
async fn contacts_list_passes_include_and_parses_properties_and_topics() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/contacts"))
        .and(query_param("limit", "2"))
        .and(query_param("include", "properties,topics"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{
                "id": "c1", "email": "c@x.dev", "first_name": null, "last_name": null,
                "created_at": "2026-01-01T00:00:00Z", "unsubscribed": false,
                "properties": { "plan": { "type": "string", "value": "pro" } },
                "topics": [{ "id": "t1", "name": "Insights", "description": null,
                             "subscription": "opt_in", "explicit": false, "visibility": "public" }]
            }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let options = ListContactsOptions {
        limit: Some(2),
        include: Some(vec![ContactInclude::Properties, ContactInclude::Topics]),
        ..Default::default()
    };
    let list = ms.contacts.list(Some(&options)).await.unwrap();
    let item = &list.data[0];
    assert_eq!(
        item.properties.as_ref().unwrap()["plan"],
        json!({ "type": "string", "value": "pro" })
    );
    let topics = item.topics.as_ref().unwrap();
    assert_eq!(topics[0].id, "t1");
    assert_eq!(topics[0].subscription, TopicSubscription::OptIn);
    assert!(!topics[0].explicit);
    assert_eq!(topics[0].visibility, Some(TopicVisibility::Public));
}

#[tokio::test]
async fn contact_topics_update_patches_bare_array() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/contacts/c1/topics"))
        .and(body_json(
            json!([{ "id": "t1", "subscription": "opt_out" }]),
        ))
        .respond_with(ok_json(json!({ "id": "c1" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let updates = vec![ContactTopicUpdate {
        id: "t1".into(),
        subscription: TopicSubscription::OptOut,
    }];
    assert_eq!(
        ms.contacts.topics.update("c1", &updates).await.unwrap().id,
        "c1"
    );
}

// ---- topics --------------------------------------------------------------

#[tokio::test]
async fn topics_cover_create_get_list_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/topics"))
        .and(body_json(
            json!({ "name": "Product", "default_subscription": "opt_in" }),
        ))
        .respond_with(ok_json(json!({ "id": "t1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/topics/t1"))
        .respond_with(ok_json(json!({
            "id": "t1", "name": "Product", "default_subscription": "opt_in",
            "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    // Bare { data } — no object/has_more.
    Mock::given(method("GET"))
        .and(path("/topics"))
        .respond_with(ok_json(json!({ "data": [] })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/topics/t1"))
        .respond_with(ok_json(
            json!({ "id": "t1", "object": "topic", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let topic = CreateTopicOptions::new("Product", TopicSubscription::OptIn);
    assert_eq!(ms.topics.create(&topic).await.unwrap().id, "t1");
    assert_eq!(ms.topics.get("t1").await.unwrap().name, "Product");
    assert_eq!(ms.topics.list().await.unwrap().data.len(), 0);
    assert!(ms.topics.delete("t1").await.unwrap().deleted);
}

// ---- broadcasts ----------------------------------------------------------

#[tokio::test]
async fn broadcasts_cover_the_full_lifecycle() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/broadcasts"))
        .and(body_json(json!({
            "segment_id": "s1", "from": "a@x.dev", "subject": "News", "html": "<p>hi</p>"
        })))
        .respond_with(ok_json(json!({ "id": "b1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/broadcasts/b1"))
        .respond_with(ok_json(json!({
            "object": "broadcast", "id": "b1", "name": null,
            "segment_id": "s1", "status": "draft", "created_at": "2026-01-01T00:00:00Z",
            "scheduled_at": null, "sent_at": null, "from": "a@x.dev", "subject": "News",
            "reply_to": null, "preview_text": null, "topic_id": null,
            "html": "<p>hi</p>", "text": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/broadcasts"))
        .respond_with(ok_json(
            json!({ "object": "list", "data": [], "has_more": false }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/broadcasts/b1"))
        .and(body_json(json!({ "subject": "New" })))
        .respond_with(ok_json(json!({ "id": "b1" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/broadcasts/b1/send"))
        .and(body_json(json!({ "scheduled_at": "2999-01-01T00:00:00Z" })))
        .respond_with(ok_json(json!({ "id": "b1" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/broadcasts/b1/cancel"))
        .respond_with(ok_json(json!({ "object": "broadcast", "id": "b1" })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/broadcasts/b1"))
        .respond_with(ok_json(
            json!({ "object": "broadcast", "id": "b1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let create = CreateBroadcastOptions {
        segment_id: Some("s1".into()),
        from: "a@x.dev".into(),
        subject: "News".into(),
        html: Some("<p>hi</p>".into()),
        ..Default::default()
    };
    assert_eq!(ms.broadcasts.create(&create).await.unwrap().id, "b1");
    assert_eq!(ms.broadcasts.get("b1").await.unwrap().subject, "News");
    ms.broadcasts.list(None).await.unwrap();
    let update = UpdateBroadcastOptions {
        subject: Some("New".into()),
        ..Default::default()
    };
    ms.broadcasts.update("b1", &update).await.unwrap();
    ms.broadcasts
        .send("b1", Some("2999-01-01T00:00:00Z"))
        .await
        .unwrap();
    ms.broadcasts.cancel("b1").await.unwrap();
    assert!(ms.broadcasts.delete("b1").await.unwrap().deleted);
}

#[tokio::test]
async fn broadcasts_send_now_posts_empty_object() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/broadcasts/b1/send"))
        .and(body_json(json!({})))
        .respond_with(ok_json(json!({ "id": "b1" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    assert_eq!(ms.broadcasts.send("b1", None).await.unwrap().id, "b1");
}

// ---- segments ------------------------------------------------------------

#[tokio::test]
async fn segments_cover_create_get_list_update_delete() {
    let server = MockServer::start().await;
    let filter = json!({
        "match": "all",
        "conditions": [{ "field": "email", "op": "is_set" }]
    });
    Mock::given(method("POST"))
        .and(path("/segments"))
        .and(body_json(json!({ "name": "Active", "filter": filter })))
        .respond_with(ok_json(json!({
            "object": "segment", "id": "s1", "name": "Active",
            "filter": filter, "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/segments/s1"))
        .respond_with(ok_json(json!({
            "object": "segment", "id": "s1", "name": "Active",
            "filter": filter, "created_at": "2026-01-01T00:00:00Z", "contact_count": 42
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/segments"))
        .and(query_param("before", "cur"))
        .respond_with(ok_json(
            json!({ "object": "list", "data": [], "has_more": false }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/segments/s1"))
        .and(body_json(json!({ "name": "Renamed" })))
        .respond_with(ok_json(json!({
            "object": "segment", "id": "s1", "name": "Renamed",
            "filter": filter, "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/segments/s1"))
        .respond_with(ok_json(
            json!({ "object": "segment", "id": "s1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let create = CreateSegmentOptions {
        name: "Active".into(),
        filter: Some(SegmentFilter {
            match_: SegmentMatch::All,
            conditions: vec![SegmentCondition {
                field: "email".into(),
                op: "is_set".into(),
                value: None,
            }],
        }),
    };
    assert_eq!(ms.segments.create(&create).await.unwrap().id, "s1");
    assert_eq!(ms.segments.get("s1").await.unwrap().contact_count, Some(42));
    let options = ListOptions {
        before: Some("cur".into()),
        ..Default::default()
    };
    ms.segments.list(Some(&options)).await.unwrap();
    let update = UpdateSegmentOptions {
        name: Some("Renamed".into()),
        ..Default::default()
    };
    assert_eq!(
        ms.segments.update("s1", &update).await.unwrap().name,
        "Renamed"
    );
    assert!(ms.segments.delete("s1").await.unwrap().deleted);
}

// ---- error handling ------------------------------------------------------

#[tokio::test]
async fn non_2xx_parses_into_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "statusCode": 422, "name": "validation_error", "message": "bad"
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let err = ms
        .emails
        .send(&SendEmailOptions::new("a@x.dev", "b@x.dev", "s"))
        .await
        .unwrap_err();
    match err {
        Error::Api(ref api) => {
            assert_eq!(api.status_code, Some(422));
            assert_eq!(api.name, "validation_error");
            assert_eq!(api.message, "bad");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
    assert_eq!(err.status_code(), Some(422));
    assert_eq!(err.name(), Some("validation_error"));
}

#[tokio::test]
async fn non_2xx_non_canonical_body_falls_back_to_generic() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/e1"))
        .respond_with(ResponseTemplate::new(500).set_body_string("gateway boom"))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let err = ms.emails.get("e1").await.unwrap_err();
    match err {
        Error::Api(api) => {
            assert_eq!(api.status_code, Some(500));
            assert_eq!(api.name, "application_error");
            assert_eq!(api.message, "Request failed with status 500");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn transport_failure_surfaces_as_http_with_null_status() {
    // Nothing listens on port 1 -> connection refused before reaching any API.
    let ms = MillionSend::with_base_url("ms_test", "http://127.0.0.1:1");
    let err = ms.emails.get("e1").await.unwrap_err();
    assert!(matches!(err, Error::Http(_)), "got {err:?}");
    assert_eq!(err.status_code(), None);
}

// ---- body completeness ---------------------------------------------------

#[tokio::test]
async fn emails_send_puts_every_field_on_the_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(body_json(json!({
            "from": "Acme <a@x.dev>",
            "to": ["b@x.dev", "c@x.dev"],
            "subject": "s",
            "html": "<p>h</p>",
            "text": "t",
            "cc": "cc@x.dev",
            "bcc": ["bcc@x.dev"],
            "reply_to": "r@x.dev",
            "scheduled_at": "2999-01-01T00:00:00Z",
            "tags": [{ "name": "k", "value": "v" }],
            "topic_id": "t1",
            "attachments": [{
                "filename": "a.txt", "content": "aGk=", "content_type": "text/plain",
                "content_id": "cid"
            }, {
                "filename": "b.pdf", "path": "https://x.dev/b.pdf"
            }],
            "headers": { "X-Entity-Ref-ID": "123" },
            "template": { "id": "tpl_1", "variables": { "name": "Ada" } }
        })))
        .respond_with(ok_json(json!({ "id": "abc" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let email = SendEmailOptions {
        from: "Acme <a@x.dev>".into(),
        to: vec!["b@x.dev", "c@x.dev"].into(),
        subject: "s".into(),
        html: Some("<p>h</p>".into()),
        text: Some("t".into()),
        cc: Some("cc@x.dev".into()),
        bcc: Some(vec!["bcc@x.dev"].into()),
        reply_to: Some("r@x.dev".into()),
        scheduled_at: Some("2999-01-01T00:00:00Z".into()),
        tags: Some(vec![Tag {
            name: "k".into(),
            value: "v".into(),
        }]),
        topic_id: Some("t1".into()),
        attachments: Some(vec![
            Attachment {
                filename: "a.txt".into(),
                content: Some("aGk=".into()),
                content_type: Some("text/plain".into()),
                content_id: Some("cid".into()),
                path: None,
            },
            Attachment {
                filename: "b.pdf".into(),
                path: Some("https://x.dev/b.pdf".into()),
                ..Default::default()
            },
        ]),
        headers: Some(std::collections::HashMap::from([(
            "X-Entity-Ref-ID".to_string(),
            "123".to_string(),
        )])),
        template: Some(json!({ "id": "tpl_1", "variables": { "name": "Ada" } })),
    };
    assert_eq!(ms.emails.send(&email).await.unwrap().id, "abc");
}

#[tokio::test]
async fn emails_send_accepts_resend_shaped_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .and(header("idempotency-key", "key-1"))
        .respond_with(ok_json(json!({ "id": "abc" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let email = SendEmailOptions::new("a@x.dev", "b@x.dev", "s");
    let res = ms
        .emails
        .send(email.with_idempotency_key("key-1"))
        .await
        .unwrap();
    assert_eq!(res.id, "abc");
}

#[tokio::test]
async fn batch_send_with_validation_sends_header_and_types_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails/batch"))
        .and(header("x-batch-validation", "permissive"))
        .and(header("idempotency-key", "batch-2"))
        .and(body_json(json!([
            { "from": "a@x.dev", "to": "b@x.dev", "subject": "1", "text": "one" },
            { "from": "a@x.dev", "to": "not-an-email", "subject": "2", "text": "two" }
        ])))
        .respond_with(ok_json(json!({
            "data": [{ "id": "1" }],
            "errors": [{ "index": 1, "message": "to: invalid email" }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let mut one = SendEmailOptions::new("a@x.dev", "b@x.dev", "1");
    one.text = Some("one".into());
    let mut two = SendEmailOptions::new("a@x.dev", "not-an-email", "2");
    two.text = Some("two".into());
    let emails = [one, two];
    let res = ms
        .batch
        .send_with_batch_validation(
            emails.with_idempotency_key("batch-2"),
            BatchValidation::Permissive,
        )
        .await
        .unwrap();
    assert_eq!(res.data.len(), 1);
    assert_eq!(
        res.errors,
        vec![BatchError {
            index: 1,
            message: "to: invalid email".into()
        }]
    );
}

#[tokio::test]
async fn batch_send_accepts_arrays_and_vecs_without_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails/batch"))
        .respond_with(ok_json(json!({ "data": [{ "id": "1" }] })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let one = SendEmailOptions::new("a@x.dev", "b@x.dev", "1");
    let pair = [one.clone(), one.clone()];
    let res = ms.batch.send(&pair).await.unwrap();
    assert!(res.errors.is_empty());
    ms.batch.send(&pair.to_vec()).await.unwrap();
    ms.batch
        .send_with_batch_validation(&[one], BatchValidation::Strict)
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].headers.get("idempotency-key").is_none());
    assert!(requests[0].headers.get("x-batch-validation").is_none());
    assert_eq!(
        requests[2].headers.get("x-batch-validation").unwrap(),
        "strict"
    );
}

#[tokio::test]
async fn emails_update_list_and_delete_hit_the_right_paths() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/emails/e1"))
        .and(body_json(json!({ "scheduled_at": "2999-01-02T00:00:00Z" })))
        .respond_with(ok_json(json!({ "object": "email", "id": "e1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/emails"))
        .and(query_param("limit", "5"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{
                "id": "e1", "from": "a@x.dev", "to": ["b@x.dev"], "cc": null, "bcc": null,
                "reply_to": null, "subject": "s", "created_at": "2026-01-01T00:00:00Z",
                "scheduled_at": null, "last_event": "delivered"
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/emails/e1"))
        .respond_with(ok_json(
            json!({ "object": "email", "id": "e1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let changes = UpdateEmailOptions {
        scheduled_at: "2999-01-02T00:00:00Z".into(),
    };
    assert_eq!(ms.emails.update("e1", &changes).await.unwrap().id, "e1");
    let options = ListOptions {
        limit: Some(5),
        ..Default::default()
    };
    let list = ms.emails.list(Some(&options)).await.unwrap();
    assert_eq!(list.data[0].last_event, "delivered");
    assert!(ms.emails.delete("e1").await.unwrap().deleted);
}

#[tokio::test]
async fn contacts_create_puts_every_field_on_the_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contacts"))
        .and(body_json(json!({
            "email": "c@x.dev",
            "first_name": "Ada",
            "last_name": "Lovelace",
            "unsubscribed": false,
            "properties": { "plan": "pro", "seats": 3 },
            "segments": [{ "id": "s1" }],
            "topics": [{ "id": "t1", "subscription": "opt_in" }]
        })))
        .respond_with(ok_json(json!({ "object": "contact", "id": "c1" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let contact = CreateContactOptions {
        email: "c@x.dev".into(),
        first_name: Some("Ada".into()),
        last_name: Some("Lovelace".into()),
        unsubscribed: Some(false),
        properties: Some(std::collections::HashMap::from([
            ("plan".to_string(), json!("pro")),
            ("seats".to_string(), json!(3)),
        ])),
        segments: Some(vec![SegmentRef { id: "s1".into() }]),
        topics: Some(vec![ContactTopicUpdate {
            id: "t1".into(),
            subscription: TopicSubscription::OptIn,
        }]),
    };
    assert_eq!(ms.contacts.create(&contact).await.unwrap().id, "c1");
}

#[tokio::test]
async fn contacts_create_batch_sends_query_header_and_typed_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contacts/batch"))
        .and(query_param("on_conflict", "upsert"))
        .and(header("x-batch-validation", "permissive"))
        .and(body_json(json!([
            { "email": "a@x.dev", "first_name": "Ada" },
            { "email": "bad" }
        ])))
        .respond_with(ok_json(json!({
            "data": [{ "object": "contact", "index": 0, "id": "c1", "status": "updated" }],
            "counts": { "created": 0, "updated": 1, "skipped": 0, "failed": 1 },
            "errors": [{ "index": 1, "message": "email: invalid" }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let mut ada = CreateContactOptions::new("a@x.dev");
    ada.first_name = Some("Ada".into());
    let options = BatchContactsOptions {
        on_conflict: Some(OnConflict::Upsert),
        batch_validation: Some(BatchValidation::Permissive),
    };
    let res = ms
        .contacts
        .create_batch(&[ada, CreateContactOptions::new("bad")], Some(&options))
        .await
        .unwrap();
    assert_eq!(res.data[0].status, BatchContactStatus::Updated);
    assert_eq!(res.data[0].index, 0);
    assert_eq!(res.counts.failed, 1);
    assert_eq!(res.errors[0].index, 1);
}

#[tokio::test]
async fn contacts_create_batch_defaults_omit_query_and_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contacts/batch"))
        .respond_with(ok_json(json!({
            "data": [{ "object": "contact", "index": 0, "id": "c1", "status": "created" }],
            "counts": { "created": 1, "updated": 0, "skipped": 0, "failed": 0 }
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let res = ms
        .contacts
        .create_batch(&[CreateContactOptions::new("a@x.dev")], None)
        .await
        .unwrap();
    assert!(res.errors.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[0].url.query(), None);
    assert!(requests[0].headers.get("x-batch-validation").is_none());
}

#[tokio::test]
async fn contact_segments_add_and_remove() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contacts/c1/segments/s1"))
        .respond_with(ok_json(json!({ "id": "c1" })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/contacts/c(%40|@)x\.dev/segments/s1$"))
        .respond_with(ok_json(
            json!({ "id": "c1", "audienceId": "s1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    assert_eq!(ms.contacts.segments.add("c1", "s1").await.unwrap().id, "c1");
    let removed = ms
        .contacts
        .segments
        .remove(ContactAddress::email("c@x.dev"), "s1")
        .await
        .unwrap();
    assert_eq!(removed.audience_id, "s1");
    assert!(removed.deleted);
}

#[tokio::test]
async fn contact_properties_cover_create_get_list_update_delete() {
    let server = MockServer::start().await;
    let property = json!({
        "object": "contact_property", "id": "p1", "key": "plan", "type": "string",
        "fallback_value": "free", "created_at": "2026-01-01T00:00:00Z"
    });
    Mock::given(method("POST"))
        .and(path("/contact-properties"))
        .and(body_json(
            json!({ "key": "plan", "type": "string", "fallback_value": "free" }),
        ))
        .respond_with(ok_json(property.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/contact-properties/p1"))
        .respond_with(ok_json(property.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/contact-properties"))
        .and(query_param("limit", "10"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "p2", "key": "seats", "type": "number", "fallback_value": null,
                       "created_at": "2026-01-01T00:00:00Z" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/contact-properties/p1"))
        .and(body_json(json!({ "fallback_value": null })))
        .respond_with(ok_json(json!({ "object": "contact_property", "id": "p1" })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/contact-properties/p1"))
        .respond_with(ok_json(
            json!({ "object": "contact_property", "id": "p1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let created = ms
        .contacts
        .properties
        .create(&CreateContactPropertyOptions {
            key: "plan".into(),
            r#type: ContactPropertyType::String,
            fallback_value: Some(json!("free")),
        })
        .await
        .unwrap();
    assert_eq!(created.id, "p1");
    assert_eq!(created.fallback_value, Some(json!("free")));
    assert_eq!(
        ms.contacts.properties.get("p1").await.unwrap().r#type,
        ContactPropertyType::String
    );
    let options = ListOptions {
        limit: Some(10),
        ..Default::default()
    };
    let list = ms.contacts.properties.list(Some(&options)).await.unwrap();
    assert_eq!(list.data[0].r#type, ContactPropertyType::Number);
    assert_eq!(list.data[0].fallback_value, None);
    let cleared = UpdateContactPropertyOptions {
        fallback_value: Some(serde_json::Value::Null),
    };
    assert_eq!(
        ms.contacts
            .properties
            .update("p1", &cleared)
            .await
            .unwrap()
            .id,
        "p1"
    );
    assert!(ms.contacts.properties.delete("p1").await.unwrap().deleted);
}

#[tokio::test]
async fn broadcasts_create_puts_every_field_on_the_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/broadcasts"))
        .and(body_json(json!({
            "name": "Launch",
            "segment_id": "s1",
            "from": "Acme <news@x.dev>",
            "subject": "News",
            "html": "<p>hi</p>",
            "text": "hi",
            "reply_to": ["r@x.dev"],
            "preview_text": "Preheader",
            "topic_id": "t1",
            "send": true,
            "scheduled_at": "in 1 hour"
        })))
        .respond_with(ok_json(json!({ "id": "b1" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let create = CreateBroadcastOptions {
        name: Some("Launch".into()),
        segment_id: Some("s1".into()),
        from: "Acme <news@x.dev>".into(),
        subject: "News".into(),
        html: Some("<p>hi</p>".into()),
        text: Some("hi".into()),
        reply_to: Some(vec!["r@x.dev"].into()),
        preview_text: Some("Preheader".into()),
        topic_id: Some("t1".into()),
        send: Some(true),
        scheduled_at: Some("in 1 hour".into()),
    };
    assert_eq!(ms.broadcasts.create(&create).await.unwrap().id, "b1");
}

#[tokio::test]
async fn broadcasts_update_sets_or_clears_topic_id() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/broadcasts/b1"))
        .and(body_json(json!({ "preview_text": "p", "topic_id": "t2" })))
        .respond_with(ok_json(json!({ "id": "b1" })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/broadcasts/b2"))
        .and(body_json(json!({ "subject": "x", "topic_id": null })))
        .respond_with(ok_json(json!({ "id": "b2" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let set = UpdateBroadcastOptions {
        preview_text: Some("p".into()),
        topic_id: Some("t2".into()),
        ..Default::default()
    };
    assert_eq!(ms.broadcasts.update("b1", &set).await.unwrap().id, "b1");
    let clear = UpdateBroadcastOptions {
        subject: Some("x".into()),
        topic_id: Some("ignored".into()),
        clear_topic_id: true,
        ..Default::default()
    };
    assert_eq!(ms.broadcasts.update("b2", &clear).await.unwrap().id, "b2");
}

#[tokio::test]
async fn topics_update_and_visibility() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/topics"))
        .and(body_json(json!({
            "name": "Product", "description": "d", "default_subscription": "opt_out",
            "visibility": "public"
        })))
        .respond_with(ok_json(json!({ "id": "t1" })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/topics/t1"))
        .and(body_json(
            json!({ "name": "Renamed", "visibility": "private" }),
        ))
        .respond_with(ok_json(json!({ "id": "t1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/topics/t1"))
        .respond_with(ok_json(json!({
            "id": "t1", "name": "Renamed", "default_subscription": "opt_out",
            "visibility": "private", "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let mut topic = CreateTopicOptions::new("Product", TopicSubscription::OptOut);
    topic.description = Some("d".into());
    topic.visibility = Some(TopicVisibility::Public);
    assert_eq!(ms.topics.create(&topic).await.unwrap().id, "t1");
    let changes = UpdateTopicOptions {
        name: Some("Renamed".into()),
        visibility: Some(TopicVisibility::Private),
        ..Default::default()
    };
    assert_eq!(ms.topics.update("t1", &changes).await.unwrap().id, "t1");
    assert_eq!(
        ms.topics.get("t1").await.unwrap().visibility,
        Some(TopicVisibility::Private)
    );
}

#[tokio::test]
async fn segments_manual_create_omits_filter_and_update_null_clears_it() {
    let server = MockServer::start().await;
    let manual = json!({
        "object": "segment", "id": "s1", "name": "VIPs", "filter": null,
        "created_at": "2026-01-01T00:00:00Z"
    });
    Mock::given(method("POST"))
        .and(path("/segments"))
        .and(body_json(json!({ "name": "VIPs" })))
        .respond_with(ok_json(manual.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/segments/s1"))
        .and(body_json(json!({ "filter": null })))
        .respond_with(ok_json(manual))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let create = CreateSegmentOptions {
        name: "VIPs".into(),
        filter: None,
    };
    assert!(ms.segments.create(&create).await.unwrap().filter.is_none());
    let clear = UpdateSegmentOptions {
        filter: Some(None),
        ..Default::default()
    };
    assert!(ms
        .segments
        .update("s1", &clear)
        .await
        .unwrap()
        .filter
        .is_none());
}

#[tokio::test]
async fn segments_manual_segment_parses_null_filter_and_lists_contacts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/segments/s1"))
        .respond_with(ok_json(json!({
            "object": "segment", "id": "s1", "name": "VIPs", "filter": null,
            "created_at": "2026-01-01T00:00:00Z", "contact_count": 2
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/segments/s1/contacts"))
        .and(query_param("after", "cur"))
        .and(query_param("include", "topics"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "c1", "email": "c@x.dev", "first_name": null, "last_name": null,
                       "created_at": "2026-01-01T00:00:00Z", "unsubscribed": false,
                       "topics": [] }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let segment = ms.segments.get("s1").await.unwrap();
    assert!(segment.filter.is_none());
    let options = ListContactsOptions {
        after: Some("cur".into()),
        include: Some(vec![ContactInclude::Topics]),
        ..Default::default()
    };
    let members = ms
        .segments
        .list_contacts("s1", Some(&options))
        .await
        .unwrap();
    assert_eq!(members.data[0].email, "c@x.dev");
    assert!(members.data[0].properties.is_none());
    assert!(members.data[0].topics.as_ref().unwrap().is_empty());
}

// ---- suppressions --------------------------------------------------------

#[tokio::test]
async fn suppressions_cover_add_get_list_remove_and_batches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/suppressions"))
        .and(body_json(
            json!({ "email": "x@x.dev", "origin": "unsubscribe" }),
        ))
        .respond_with(ok_json(json!({ "object": "suppression", "id": "sp1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/suppressions/x(%40|@)x\.dev$"))
        .respond_with(ok_json(json!({
            "object": "suppression", "id": "sp1", "email": "x@x.dev", "origin": "bounce",
            "source_id": "e1", "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/suppressions"))
        .and(query_param("limit", "2"))
        .and(query_param("origin", "complaint"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "sp2", "email": "y@x.dev", "origin": "complaint", "source_id": null,
                       "created_at": "2026-01-01T00:00:00Z" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/suppressions/sp1"))
        .respond_with(ok_json(
            json!({ "object": "suppression", "id": "sp1", "deleted": true }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/suppressions/batch/add"))
        .and(body_json(
            json!({ "emails": ["a@x.dev", "b@x.dev"], "origin": "manual" }),
        ))
        .respond_with(ok_json(json!({
            "data": [{ "object": "suppression", "id": "sp3" }, { "object": "suppression", "id": "sp4" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/suppressions/batch/remove"))
        .and(body_json(json!({ "emails": ["a@x.dev"] })))
        .respond_with(ok_json(json!({
            "data": [{ "object": "suppression", "id": "sp3", "deleted": true }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/suppressions/batch/remove"))
        .and(body_json(json!({ "ids": ["sp4"] })))
        .respond_with(ok_json(json!({
            "data": [{ "object": "suppression", "id": "sp4", "deleted": true }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let mut add = AddSuppressionOptions::new("x@x.dev");
    add.origin = Some(SuppressionOrigin::Unsubscribe);
    assert_eq!(ms.suppressions.add(&add).await.unwrap().id, "sp1");
    let got = ms.suppressions.get("x@x.dev").await.unwrap();
    assert_eq!(got.origin, SuppressionOrigin::Bounce);
    assert_eq!(got.source_id.as_deref(), Some("e1"));
    let options = ListSuppressionsOptions {
        limit: Some(2),
        origin: Some(SuppressionOrigin::Complaint),
        ..Default::default()
    };
    let list = ms.suppressions.list(Some(&options)).await.unwrap();
    assert_eq!(list.data[0].origin, SuppressionOrigin::Complaint);
    assert!(ms.suppressions.remove("sp1").await.unwrap().deleted);
    let added = ms
        .suppressions
        .batch_add(&BatchAddSuppressionOptions {
            emails: vec!["a@x.dev".into(), "b@x.dev".into()],
            origin: Some(SuppressionOrigin::Manual),
        })
        .await
        .unwrap();
    assert_eq!(added.data.len(), 2);
    let by_email = ms
        .suppressions
        .batch_remove(&BatchRemoveSuppressionOptions::Emails(vec![
            "a@x.dev".into()
        ]))
        .await
        .unwrap();
    assert_eq!(by_email.data[0].id, "sp3");
    let by_id = ms
        .suppressions
        .batch_remove(&BatchRemoveSuppressionOptions::Ids(vec!["sp4".into()]))
        .await
        .unwrap();
    assert!(by_id.data[0].deleted);
}

// ---- domains -------------------------------------------------------------

#[tokio::test]
async fn domains_cover_create_get_list_verify_update_delete() {
    let server = MockServer::start().await;
    let domain = json!({
        "object": "domain", "id": "d1", "name": "x.dev", "status": "pending",
        "created_at": "2026-01-01T00:00:00Z", "region": "us-east-1",
        "open_tracking": true, "click_tracking": false, "tracking_subdomain": "links",
        "capabilities": { "sending": "enabled", "receiving": "disabled" },
        "records": [
            { "record": "DKIM", "name": "k._domainkey", "type": "TXT", "ttl": "Auto",
              "status": "pending", "value": "p=abc" },
            { "record": "SPF", "name": "send", "type": "MX", "ttl": "Auto",
              "status": "pending", "value": "feedback-smtp.amazonses.com", "priority": 10 }
        ]
    });
    Mock::given(method("POST"))
        .and(path("/domains"))
        .and(body_json(json!({
            "name": "x.dev", "region": "us-east-1", "custom_return_path": "send",
            "open_tracking": true, "click_tracking": false, "tracking_subdomain": "links"
        })))
        .respond_with(ok_json(domain.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/domains/d1"))
        .respond_with(ok_json(domain.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/domains"))
        .and(query_param("limit", "3"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{
                "id": "d1", "name": "x.dev", "status": "verified",
                "created_at": "2026-01-01T00:00:00Z", "region": "us-east-1",
                "open_tracking": false, "click_tracking": false, "tracking_subdomain": null,
                "capabilities": { "sending": "enabled", "receiving": "disabled" }
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/domains/d1/verify"))
        .respond_with(ok_json(domain.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/domains/d1"))
        .and(body_json(
            json!({ "click_tracking": true, "tracking_subdomain": null }),
        ))
        .respond_with(ok_json(domain.clone()))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/domains/d1"))
        .respond_with(ok_json(
            json!({ "object": "domain", "id": "d1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let create = CreateDomainOptions {
        name: "x.dev".into(),
        region: Some("us-east-1".into()),
        custom_return_path: Some("send".into()),
        open_tracking: Some(true),
        click_tracking: Some(false),
        tracking_subdomain: Some("links".into()),
    };
    let created = ms.domains.create(&create).await.unwrap();
    assert_eq!(created.records.len(), 2);
    assert_eq!(created.records[0].priority, None);
    assert_eq!(created.records[1].priority, Some(10));
    assert_eq!(created.capabilities.sending, "enabled");
    assert_eq!(
        ms.domains
            .get("d1")
            .await
            .unwrap()
            .tracking_subdomain
            .as_deref(),
        Some("links")
    );
    let options = ListOptions {
        limit: Some(3),
        ..Default::default()
    };
    let list = ms.domains.list(Some(&options)).await.unwrap();
    assert!(list.data[0].records.is_empty());
    assert_eq!(ms.domains.verify("d1").await.unwrap().status, "pending");
    let changes = UpdateDomainOptions {
        click_tracking: Some(true),
        tracking_subdomain: Some(None),
        ..Default::default()
    };
    assert_eq!(ms.domains.update("d1", &changes).await.unwrap().id, "d1");
    assert!(ms.domains.delete("d1").await.unwrap().deleted);
}

// ---- webhooks ------------------------------------------------------------

#[tokio::test]
async fn webhooks_cover_create_get_list_update_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhooks"))
        .and(body_json(json!({
            "endpoint": "https://x.dev/hook",
            "events": ["email.delivered", "email.bounced"],
            "signing_secret": "whsec_abc"
        })))
        .respond_with(ok_json(
            json!({ "object": "webhook", "id": "w1", "signing_secret": "whsec_abc" }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/webhooks/w1"))
        .respond_with(ok_json(json!({
            "object": "webhook", "id": "w1", "endpoint": "https://x.dev/hook",
            "created_at": "2026-01-01T00:00:00Z", "status": "enabled",
            "events": ["email.delivered"], "signing_secret": "whsec_abc",
            "previous_secret_expires_at": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/webhooks"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "w1", "endpoint": "https://x.dev/hook",
                       "created_at": "2026-01-01T00:00:00Z", "status": "disabled", "events": null }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/webhooks/w1"))
        .and(body_json(json!({
            "endpoint": "https://x.dev/hook2", "events": ["email.opened"], "status": "disabled"
        })))
        .respond_with(ok_json(json!({ "object": "webhook", "id": "w1" })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/webhooks/w1"))
        .respond_with(ok_json(
            json!({ "object": "webhook", "id": "w1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let created = ms
        .webhooks
        .create(&CreateWebhookOptions {
            endpoint: "https://x.dev/hook".into(),
            events: vec!["email.delivered".into(), "email.bounced".into()],
            signing_secret: Some("whsec_abc".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(created.signing_secret, "whsec_abc");
    let got = ms.webhooks.get("w1").await.unwrap();
    assert_eq!(got.status, WebhookStatus::Enabled);
    assert_eq!(got.signing_secret.as_deref(), Some("whsec_abc"));
    assert!(got.previous_secret_expires_at.is_none());
    let list = ms.webhooks.list(None).await.unwrap();
    assert_eq!(list.data[0].status, WebhookStatus::Disabled);
    assert!(list.data[0].events.is_none());
    assert!(list.data[0].signing_secret.is_none());
    let changes = UpdateWebhookOptions {
        endpoint: Some("https://x.dev/hook2".into()),
        events: Some(vec!["email.opened".into()]),
        status: Some(WebhookStatus::Disabled),
        ..Default::default()
    };
    assert_eq!(ms.webhooks.update("w1", &changes).await.unwrap().id, "w1");
    assert!(ms.webhooks.delete("w1").await.unwrap().deleted);
}

// ---- api keys ------------------------------------------------------------

#[tokio::test]
async fn api_keys_cover_create_list_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api-keys"))
        .and(body_json(
            json!({ "name": "ci", "permission": "sending_access", "domain_id": "d1" }),
        ))
        .respond_with(ok_json(json!({ "id": "k1", "token": "ms_secret" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api-keys"))
        .and(query_param("before", "cur"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "k1", "name": "ci", "created_at": "2026-01-01T00:00:00Z",
                       "last_used_at": null }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api-keys/k1"))
        .respond_with(ok_json(
            json!({ "object": "api_key", "id": "k1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let created = ms
        .api_keys
        .create(&CreateApiKeyOptions {
            name: "ci".into(),
            permission: Some(ApiKeyPermission::SendingAccess),
            domain_id: Some("d1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(created.token, "ms_secret");
    let options = ListOptions {
        before: Some("cur".into()),
        ..Default::default()
    };
    let list = ms.api_keys.list(Some(&options)).await.unwrap();
    assert_eq!(list.data[0].name, "ci");
    assert!(list.data[0].last_used_at.is_none());
    assert!(ms.api_keys.delete("k1").await.unwrap().deleted);
}

// ---- templates -----------------------------------------------------------

#[tokio::test]
async fn templates_cover_the_full_lifecycle() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/templates"))
        .and(body_json(json!({
            "name": "Welcome", "html": "<p>Hi {{{FIRST_NAME|there}}}</p>",
            "subject": "Welcome!", "text": "Hi", "alias": "welcome"
        })))
        .respond_with(ok_json(json!({ "object": "template", "id": "tp1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/templates/welcome"))
        .respond_with(ok_json(json!({
            "object": "template", "id": "tp1", "name": "Welcome", "alias": "welcome",
            "status": "published", "published_at": "2026-01-01T00:00:00Z",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "current_version_id": "v1", "from": null, "subject": "Welcome!", "reply_to": null,
            "html": "<p>Hi</p>", "text": "Hi", "variables": [], "has_unpublished_versions": false
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/templates"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "tp1", "name": "Welcome", "alias": null, "status": "published",
                       "published_at": "2026-01-01T00:00:00Z", "created_at": "2026-01-01T00:00:00Z",
                       "updated_at": "2026-01-01T00:00:00Z" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/templates/tp1"))
        .and(body_json(json!({
            "name": "Welcome v2", "subject": "Hello", "text": null, "alias": null
        })))
        .respond_with(ok_json(json!({ "object": "template", "id": "tp1" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/templates/tp1/publish"))
        .respond_with(ok_json(json!({ "object": "template", "id": "tp1" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/templates/tp1/duplicate"))
        .respond_with(ok_json(json!({ "object": "template", "id": "tp2" })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/templates/tp1"))
        .respond_with(ok_json(
            json!({ "object": "template", "id": "tp1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let mut create = CreateTemplateOptions::new("Welcome", "<p>Hi {{{FIRST_NAME|there}}}</p>");
    create.subject = Some("Welcome!".into());
    create.text = Some("Hi".into());
    create.alias = Some("welcome".into());
    assert_eq!(ms.templates.create(&create).await.unwrap().id, "tp1");
    let got = ms.templates.get("welcome").await.unwrap();
    assert_eq!(got.current_version_id, "v1");
    assert_eq!(got.subject.as_deref(), Some("Welcome!"));
    assert!(got.reply_to.is_none());
    assert!(got.variables.is_empty());
    assert!(ms.templates.list(None).await.unwrap().data[0]
        .alias
        .is_none());
    let changes = UpdateTemplateOptions {
        name: Some("Welcome v2".into()),
        subject: Some(Some("Hello".into())),
        text: Some(None),
        alias: Some(None),
        ..Default::default()
    };
    assert_eq!(
        ms.templates.update("tp1", &changes).await.unwrap().id,
        "tp1"
    );
    assert_eq!(ms.templates.publish("tp1").await.unwrap().id, "tp1");
    assert_eq!(ms.templates.duplicate("tp1").await.unwrap().id, "tp2");
    assert!(ms.templates.delete("tp1").await.unwrap().deleted);
}

// ---- usage ---------------------------------------------------------------

#[tokio::test]
async fn usage_get_maps_report() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/usage"))
        .respond_with(ok_json(json!({
            "object": "usage", "cloud": true, "plan": "pro",
            "limits": { "emails_per_day": 50000, "domains": null },
            "today": { "emails_sent": 120, "resets_at": "2026-01-02T00:00:00Z" },
            "team": { "id": "team1", "name": "Acme" },
            "app_url": "https://app.x.dev"
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let usage = ms.usage.get().await.unwrap();
    assert!(usage.cloud);
    assert_eq!(usage.plan.as_deref(), Some("pro"));
    assert_eq!(usage.limits.emails_per_day, Some(50000));
    assert_eq!(usage.limits.domains, None);
    assert_eq!(usage.today.emails_sent, 120);
    assert_eq!(usage.team.name, "Acme");
    assert_eq!(usage.app_url.as_deref(), Some("https://app.x.dev"));
}

// ---- 0.5: cloud default, contact topics list, suppressed recipients --------

/// One test, sequential: `MILLIONSEND_BASE_URL` is process-global and this is
/// the only test that reads or writes it.
#[tokio::test]
async fn base_url_defaults_to_cloud_env_and_explicit_win() {
    std::env::remove_var("MILLIONSEND_BASE_URL");
    assert_eq!(
        MillionSend::new("ms_test").base_url(),
        "https://api.millionsend.com"
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/topics"))
        .respond_with(ok_json(json!({ "data": [] })))
        .mount(&server)
        .await;
    std::env::set_var("MILLIONSEND_BASE_URL", server.uri());
    let ms = MillionSend::new("ms_test");
    assert_eq!(ms.base_url(), server.uri());
    assert!(ms.topics.list().await.unwrap().data.is_empty());

    assert_eq!(
        MillionSend::with_base_url("ms_test", "https://mail.acme.dev/").base_url(),
        "https://mail.acme.dev"
    );
    std::env::remove_var("MILLIONSEND_BASE_URL");
}

#[tokio::test]
async fn contact_topics_list_gets_encoded_email_and_decodes_shape() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/contacts/ada%40acme.dev/topics"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [
                { "id": "t1", "name": "Insights", "description": "Weekly",
                  "subscription": "opt_out", "explicit": true, "visibility": "public" },
                { "id": "t2", "name": "Product", "description": null,
                  "subscription": "opt_in", "explicit": false, "visibility": "private" }
            ]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let topics = ms
        .contacts
        .topics
        .list(ContactAddress::email("ada@acme.dev"))
        .await
        .unwrap();
    assert_eq!(topics.object, "list");
    assert!(!topics.has_more);
    assert_eq!(topics.data.len(), 2);
    assert_eq!(topics.data[0].id, "t1");
    assert_eq!(topics.data[0].name, "Insights");
    assert_eq!(topics.data[0].description.as_deref(), Some("Weekly"));
    assert_eq!(topics.data[0].subscription, TopicSubscription::OptOut);
    assert!(topics.data[0].explicit);
    assert_eq!(topics.data[0].visibility, Some(TopicVisibility::Public));
    assert_eq!(topics.data[1].description, None);
    assert_eq!(topics.data[1].visibility, Some(TopicVisibility::Private));
    assert_eq!(topics.data[1].subscription, TopicSubscription::OptIn);
    assert!(!topics.data[1].explicit);
}

#[tokio::test]
async fn emails_send_surfaces_all_recipients_suppressed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "statusCode": 422, "name": "all_recipients_suppressed",
            "message": "All recipients are suppressed"
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let err = ms
        .emails
        .send(&SendEmailOptions::new("a@x.dev", "b@x.dev", "s"))
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), Some(422));
    assert_eq!(err.name(), Some("all_recipients_suppressed"));
}

#[tokio::test]
async fn contacts_batch_remove_by_ids_and_emails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contacts/batch/remove"))
        .and(body_json(json!({ "ids": ["c1", "c2"] })))
        .respond_with(ok_json(json!({
            "data": [{ "object": "contact", "contact": "c1", "deleted": true }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/contacts/batch/remove"))
        .and(body_json(json!({ "emails": ["a@x.dev"] })))
        .respond_with(ok_json(json!({
            "data": [{ "object": "contact", "contact": "c3", "deleted": true }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/contacts/batch/remove"))
        .and(body_json(json!({ "emails": ["b@x.dev"], "erase": true })))
        .respond_with(ok_json(json!({
            "data": [{ "object": "contact", "contact": "c4", "deleted": true }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let by_id = ms
        .contacts
        .batch_remove(
            &BatchRemoveContactsOptions::Ids(vec!["c1".into(), "c2".into()]),
            None,
        )
        .await
        .unwrap();
    assert_eq!(by_id.data.len(), 1);
    assert_eq!(by_id.data[0].object, "contact");
    assert_eq!(by_id.data[0].contact, "c1");
    assert!(by_id.data[0].deleted);
    // `erase: false` is omitted from the body, like `None` (the matcher is exact).
    let by_email = ms
        .contacts
        .batch_remove(
            &BatchRemoveContactsOptions::Emails(vec!["a@x.dev".into()]),
            Some(&DeleteContactOptions { erase: false }),
        )
        .await
        .unwrap();
    assert_eq!(by_email.data[0].contact, "c3");
    let erased = ms
        .contacts
        .batch_remove(
            &BatchRemoveContactsOptions::Emails(vec!["b@x.dev".into()]),
            Some(&DeleteContactOptions { erase: true }),
        )
        .await
        .unwrap();
    assert_eq!(erased.data[0].contact, "c4");
}

#[tokio::test]
async fn contacts_batch_get_posts_ids_and_emails_and_parses_missing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contacts/batch/get"))
        .and(body_json(json!({
            "contacts": [{ "id": "c1" }, { "email": "a@x.dev" }, { "id": "c9" }],
            "include": ["topics"]
        })))
        .respond_with(ok_json(json!({
            "object": "list",
            "data": [
                { "object": "contact", "id": "c1", "email": "c@x.dev", "first_name": "Ada",
                  "last_name": null, "created_at": "2026-01-01T00:00:00Z", "unsubscribed": false,
                  "topics": [{ "id": "t1", "name": "Insights", "description": null,
                               "subscription": "opt_out", "explicit": true, "visibility": "private" }] },
                { "object": "contact", "id": "c2", "email": "a@x.dev", "first_name": null,
                  "last_name": null, "created_at": "2026-01-01T00:00:00Z", "unsubscribed": true,
                  "topics": [] }
            ],
            "missing": [{ "index": 2, "id": "c9" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/contacts/batch/get"))
        .and(body_json(json!({ "contacts": [{ "email": "b@x.dev" }] })))
        .respond_with(ok_json(json!({
            "object": "list", "data": [], "missing": [{ "index": 0, "email": "b@x.dev" }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    // An address with both keys goes out as its email, like the path key.
    let both = ContactAddress {
        id: Some("c2".into()),
        email: Some("a@x.dev".into()),
    };
    let res = ms
        .contacts
        .batch_get(
            &["c1".into(), both, ContactAddress::id("c9")],
            Some(&BatchGetContactsOptions {
                include: Some(vec![ContactInclude::Topics]),
            }),
        )
        .await
        .unwrap();
    assert_eq!(res.object, "list");
    assert_eq!(res.data.len(), 2);
    assert_eq!(res.data[0].object, "contact");
    assert_eq!(res.data[0].email, "c@x.dev");
    assert_eq!(res.data[0].first_name.as_deref(), Some("Ada"));
    assert!(res.data[0].properties.is_none());
    let topics = res.data[0].topics.as_ref().unwrap();
    assert_eq!(topics[0].subscription, TopicSubscription::OptOut);
    assert!(res.data[1].unsubscribed);
    assert_eq!(
        res.missing,
        vec![MissingContact {
            index: 2,
            id: Some("c9".into()),
            email: None
        }]
    );

    let none = ms
        .contacts
        .batch_get(&[ContactAddress::email("b@x.dev")], None)
        .await
        .unwrap();
    assert!(none.data.is_empty());
    assert_eq!(none.missing[0].index, 0);
    assert_eq!(none.missing[0].email.as_deref(), Some("b@x.dev"));
    assert_eq!(none.missing[0].id, None);
}

#[tokio::test]
async fn contacts_preferences_link_posts_no_body_by_id_and_email() {
    let server = MockServer::start().await;
    let link = json!({
        "object": "preferences_link", "contact": "c1",
        "url": "https://app.x.dev/unsubscribe/tok"
    });
    Mock::given(method("POST"))
        .and(path("/contacts/c1/preferences-link"))
        .and(body_string(""))
        .respond_with(ok_json(link.clone()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/contacts/c(%40|@)x\.dev/preferences-link$"))
        .and(body_string(""))
        .respond_with(ok_json(link))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let by_id = ms.contacts.preferences_link("c1").await.unwrap();
    assert_eq!(by_id.object, "preferences_link");
    assert_eq!(by_id.contact, "c1");
    assert_eq!(by_id.url, "https://app.x.dev/unsubscribe/tok");
    let by_email = ms
        .contacts
        .preferences_link(ContactAddress::email("c@x.dev"))
        .await
        .unwrap();
    assert_eq!(by_email.contact, "c1");
}

#[tokio::test]
async fn webhooks_rotate_sends_empty_object_or_options() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/webhooks/w1/rotate"))
        .and(body_json(json!({})))
        .respond_with(ok_json(json!({
            "object": "webhook", "id": "w1", "signing_secret": "whsec_new",
            "previous_secret_expires_at": "2026-01-02T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webhooks/w1/rotate"))
        .and(body_json(
            json!({ "signing_secret": "whsec_mine", "overlap_hours": 0 }),
        ))
        .respond_with(ok_json(json!({
            "object": "webhook", "id": "w1", "signing_secret": "whsec_mine",
            "previous_secret_expires_at": null
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let minted = ms.webhooks.rotate("w1", None).await.unwrap();
    assert_eq!(minted.object, "webhook");
    assert_eq!(minted.id, "w1");
    assert_eq!(minted.signing_secret, "whsec_new");
    assert_eq!(
        minted.previous_secret_expires_at.as_deref(),
        Some("2026-01-02T00:00:00Z")
    );
    let own = ms
        .webhooks
        .rotate(
            "w1",
            Some(&RotateWebhookOptions {
                signing_secret: Some("whsec_mine".into()),
                overlap_hours: Some(0),
            }),
        )
        .await
        .unwrap();
    assert_eq!(own.signing_secret, "whsec_mine");
    assert!(own.previous_secret_expires_at.is_none());
}

// ---- receiving and inboxes -----------------------------------------------

fn draft_json(status: &str) -> serde_json::Value {
    json!({
        "object": "inbox_draft", "id": "d1", "type": "reply", "to": ["ada@x.dev"], "cc": [], "bcc": [],
        "subject": "Re: Hi", "html": null, "text": "Thanks", "thread_id": "t1", "reply_to_email_id": "m1",
        "email_id": null, "created_at": "2026-10-12T10:00:00.000Z", "updated_at": "2026-10-12T10:00:00.000Z",
        "status": status, "approval_reasons": ["send_policy", "ramp"], "forward": false,
        "decided_by": null, "decided_at": null, "expires_at": "2026-10-19T10:00:00.000Z"
    })
}

fn inbox_json() -> serde_json::Value {
    json!({
        "object": "inbox", "id": "i1", "name": "Signup bot",
        "email_address": "signup-bot-7f3k@acme.agents.example", "domain_id": "sd1",
        "receiving_address": null, "from_name": null, "unread": 2, "drafts": 1,
        "last_received": "2026-10-12T10:00:00.000Z", "created_at": "2026-10-12T09:00:00.000Z",
        "kind": "agent", "shared": true, "expires_at": "2026-10-13T09:00:00.000Z",
        "quarantined": 1, "pending_approval": 1
    })
}

fn draft_of<T: std::fmt::Debug>(outcome: SendOutcome<T>) -> InboxDraft {
    match outcome {
        SendOutcome::Draft(draft) => *draft,
        SendOutcome::Sent(sent) => panic!("expected a draft, got {sent:?}"),
    }
}

fn sent_of<T>(outcome: SendOutcome<T>) -> T {
    match outcome {
        SendOutcome::Sent(sent) => sent,
        SendOutcome::Draft(draft) => panic!("expected a sent email, got draft {}", draft.id),
    }
}

#[tokio::test]
async fn receiving_list_sends_every_filter_and_parses_held() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/receiving"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "r1", "from": "a@x.dev", "to": [], "cc": [], "bcc": [], "reply_to": [],
                       "received_for": ["bot@acme.dev"], "subject": "Code", "message_id": "<m@x.dev>",
                       "created_at": "2026-10-12T10:00:00.000Z", "attachments": [],
                       "screening": { "verdict": "clean", "sender_trust": "external" },
                       "extracted": { "kinds": ["code"] }, "read": false, "inbox_id": "i1",
                       "thread_id": "t1", "spam_verdict": "pass", "virus_verdict": null }],
            "held": [{ "id": "r2", "inbox_id": "i1", "reason": "not_allowlisted",
                       "created_at": "2026-10-12T10:00:00.000Z" },
                     { "id": "r3", "inbox_id": "i1", "reason": null,
                       "created_at": "2026-10-12T10:00:01.000Z" }]
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let options = ListReceivedEmailsOptions {
        limit: Some(10),
        before: Some("r0".into()),
        wait: Some(30),
        since: Some("2026-10-12T00:00:00Z".into()),
        inbox_id: Some("i1".into()),
        to: Some("bot@x.dev".into()),
        from: Some("stripe.com".into()),
        subject_contains: Some("code".into()),
        unread: Some(false),
        include_quarantined: true,
        ..Default::default()
    };
    let page = ms.receiving.list(Some(&options)).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].url.query(),
        Some("limit=10&before=r0&wait=30&since=2026-10-12T00%3A00%3A00Z&inbox_id=i1&to=bot%40x.dev&from=stripe.com&subject_contains=code&unread=false&include_quarantined=true")
    );
    let email = &page.data[0];
    assert_eq!(email.screening.as_ref().unwrap().sender_trust, "external");
    assert_eq!(email.extracted.as_ref().unwrap().kinds, vec!["code"]);
    assert_eq!(email.read, Some(false));
    assert!(email.virus_verdict.is_none());
    assert_eq!(
        page.held,
        vec![
            HeldEmail {
                id: "r2".into(),
                inbox_id: Some("i1".into()),
                reason: Some("not_allowlisted".into()),
                created_at: "2026-10-12T10:00:00.000Z".into(),
            },
            HeldEmail {
                id: "r3".into(),
                inbox_id: Some("i1".into()),
                reason: None,
                created_at: "2026-10-12T10:00:01.000Z".into(),
            },
        ]
    );

    ms.receiving.list(None).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests[1].url.query(), None);
}

/// A long-poll outlives the client's own timeout by the time it may wait.
#[tokio::test]
async fn long_polls_stretch_the_request_timeout() {
    let server = MockServer::start().await;
    let slow = |body: serde_json::Value| ok_json(body).set_delay(Duration::from_millis(200));
    Mock::given(method("GET"))
        .and(path("/emails/receiving"))
        .respond_with(slow(
            json!({ "object": "list", "has_more": false, "data": [], "held": [] }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/codes/latest"))
        .respond_with(slow(
            json!({ "object": "verification_code", "status": "timeout" }),
        ))
        .mount(&server)
        .await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let ms = MillionSend::with_base_url("ms_test", server.uri()).with_client(client);
    match ms.receiving.list(None).await.unwrap_err() {
        Error::Http(e) => assert!(e.is_timeout(), "{e}"),
        other => panic!("expected a timeout, got {other}"),
    }
    let waiting = ListReceivedEmailsOptions {
        wait: Some(1),
        ..Default::default()
    };
    assert!(ms
        .receiving
        .list(Some(&waiting))
        .await
        .unwrap()
        .data
        .is_empty());
    let code = ms
        .inboxes
        .codes()
        .latest(
            "i1",
            Some(&LatestCodeOptions {
                wait: Some(1),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(code.status, "timeout");
}

#[tokio::test]
async fn receiving_get_parses_extensions_and_tolerates_unknown_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/emails/receiving/r1"))
        .and(query_param("html_format", "cid"))
        .respond_with(ok_json(json!({
            "object": "email", "id": "r1", "from": "noreply@github.com", "to": ["bot@acme.dev"],
            "cc": [], "bcc": [], "reply_to": [], "received_for": ["bot@acme.dev"], "subject": "Your code",
            "message_id": "<m@github.com>", "created_at": "2026-10-12T10:00:00.000Z",
            "html": "<p>482913</p>", "html_format": "cid", "text": "Your code is 482913",
            "headers": { "from": "GitHub <noreply@github.com>" },
            "authentication": { "spf": "pass", "dkim": "pass", "dmarc": "gray" }, "raw": null,
            "attachments": [{ "id": "a1", "filename": null, "content_type": "image/png", "content_id": "logo",
                              "content_disposition": "inline", "size": 120 }],
            "inbox_id": "i1", "thread_id": "t1",
            "inboxes": [{ "inbox_id": "i1", "thread_id": "t1", "verdict": "cosmic" }],
            "screening": { "verdict": "clean", "sender_trust": "external", "quarantine_reason": null,
                           "reasons": ["targets_agent"], "injection_score": 3, "phishing_score": null,
                           "hidden_text_removed": true },
            "spam_verdict": "gray", "spam_score": 31, "virus_verdict": "processing_failed",
            "virus_signature": null, "extracted_text": "Your code is 482913",
            "extracted": { "kinds": ["code"], "code": "482913", "link": null,
                           "link_domain_matches_sender": null, "ambiguous": false },
            "brand_new": { "verdict": "cosmic" }
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let options = GetReceivedEmailOptions {
        html_format: Some("cid".into()),
    };
    let email = ms.receiving.get("r1", Some(&options)).await.unwrap();
    assert!(email.raw.is_none());
    assert_eq!(email.authentication.unwrap().dmarc, "gray");
    assert_eq!(
        email.headers.unwrap()["from"],
        "GitHub <noreply@github.com>"
    );
    assert_eq!(email.attachments[0].filename, None);
    assert_eq!(email.attachments[0].size, Some(120));
    assert_eq!(email.inboxes.unwrap()[0].verdict, "cosmic");
    let screening = email.screening.unwrap();
    assert_eq!(screening.injection_score, Some(3));
    assert_eq!(screening.phishing_score, None);
    assert!(screening.hidden_text_removed);
    assert_eq!(email.spam_score, Some(31));
    assert_eq!(email.virus_verdict.as_deref(), Some("processing_failed"));
    let extracted = email.extracted.unwrap();
    assert_eq!(extracted.code.as_deref(), Some("482913"));
    assert!(extracted.link_domain_matches_sender.is_none());
}

#[tokio::test]
async fn receiving_reply_tells_sent_from_drafted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails/receiving/r1/reply"))
        .and(header("idempotency-key", "rk-1"))
        .and(body_json(json!({
            "text": "Thanks", "html": "<p>Thanks</p>", "subject": "Re: Hi", "cc": ["c@x.dev"],
            "bcc": "b@x.dev", "reply_all": true, "requires_approval": true, "from": "bot@acme.dev",
            "inbox_id": "i1"
        })))
        .respond_with(ok_json(
            json!({ "object": "email", "id": "e9", "thread_id": "t1", "inbox_id": "i1" }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/emails/receiving/r2/reply"))
        .and(body_json(json!({ "text": "Thanks" })))
        .respond_with(ResponseTemplate::new(202).set_body_json(draft_json("pending_approval")))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let reply = ReplyReceivedEmailOptions {
        text: Some("Thanks".into()),
        html: Some("<p>Thanks</p>".into()),
        subject: Some("Re: Hi".into()),
        cc: Some(vec!["c@x.dev"].into()),
        bcc: Some("b@x.dev".into()),
        reply_all: Some(true),
        requires_approval: Some(true),
        from: Some("bot@acme.dev".into()),
        inbox_id: Some("i1".into()),
    };
    let sent = sent_of(
        ms.receiving
            .reply("r1", reply.with_idempotency_key("rk-1"))
            .await
            .unwrap(),
    );
    assert_eq!(
        sent,
        ReceivedEmailSent {
            object: "email".into(),
            id: "e9".into(),
            thread_id: Some("t1".into()),
            inbox_id: Some("i1".into()),
        }
    );

    let plain = ReplyReceivedEmailOptions {
        text: Some("Thanks".into()),
        ..Default::default()
    };
    let draft = draft_of(ms.receiving.reply("r2", &plain).await.unwrap());
    assert_eq!(draft.status, "pending_approval");
    assert_eq!(draft.approval_reasons, vec!["send_policy", "ramp"]);
    let requests = server.received_requests().await.unwrap();
    assert!(requests[1].headers.get("idempotency-key").is_none());
}

#[tokio::test]
async fn receiving_forward_delete_and_attachments() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/emails/receiving/r1/forward"))
        .and(header("idempotency-key", "fk-1"))
        .and(body_json(json!({
            "to": ["ops@acme.dev"], "text": "FYI", "from": "me@acme.dev", "mode": "attachment"
        })))
        .respond_with(ok_json(
            json!({ "object": "email", "id": "e9", "thread_id": null, "inbox_id": null }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/emails/receiving/r1"))
        .respond_with(ok_json(
            json!({ "object": "email", "id": "r1", "deleted": true }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/emails/receiving/r1/attachments"))
        .and(query_param("limit", "5"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "a1", "filename": "invoice.pdf", "size": 48213,
                       "content_type": "application/pdf", "content_disposition": "attachment",
                       "download_url": "https://api.test/d/a1", "expires_at": "2026-10-12T11:00:00.000Z" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/emails/receiving/r1/attachments/a1"))
        .respond_with(ok_json(json!({
            "object": "attachment", "id": "a1", "size": 48213, "content_type": "application/pdf",
            "content_disposition": "attachment", "download_url": "https://api.test/d/a1",
            "expires_at": "2026-10-12T11:00:00.000Z"
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let forward = ForwardReceivedEmailOptions {
        to: vec!["ops@acme.dev"].into(),
        text: Some("FYI".into()),
        from: Some("me@acme.dev".into()),
        mode: Some("attachment".into()),
        ..Default::default()
    };
    let sent = sent_of(
        ms.receiving
            .forward("r1", forward.with_idempotency_key("fk-1"))
            .await
            .unwrap(),
    );
    assert!(sent.thread_id.is_none());
    assert!(ms.receiving.delete("r1").await.unwrap().deleted);
    let options = ListOptions {
        limit: Some(5),
        ..Default::default()
    };
    let list = ms
        .receiving
        .attachments()
        .list("r1", Some(&options))
        .await
        .unwrap();
    assert_eq!(
        list.data[0].download_url.as_deref(),
        Some("https://api.test/d/a1")
    );
    let one = ms.receiving.attachments().get("r1", "a1").await.unwrap();
    assert_eq!(one.object.as_deref(), Some("attachment"));
    assert_eq!(one.filename, None);
}

#[tokio::test]
async fn inboxes_cover_create_list_get_update_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/inboxes"))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(201).set_body_json(inbox_json()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes"))
        .and(body_json(json!({
            "email_address": "bot@acme.dev", "name": "Bot", "from_name": "Acme Bot", "forwarding": true,
            "kind": "human", "expires_in": "7d",
            "safety": { "accept_from": "allowlist", "allowlist": ["*@stripe.com"], "send_cap": null }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(inbox_json()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes"))
        .and(query_param("kind", "agent"))
        .and(query_param("limit", "2"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "i1", "name": null, "email_address": "bot@acme.dev", "from_name": null,
                       "unread": 0, "last_received": null, "kind": "agent", "shared": false,
                       "expires_at": null }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes/bot%2Bsales%40acme.dev"))
        .respond_with(ok_json(inbox_json()))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/inboxes/i1"))
        .and(body_json(
            json!({ "name": "Renamed", "from_name": "", "expires_in": null }),
        ))
        .respond_with(ok_json(inbox_json()))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/inboxes/i1"))
        .respond_with(ok_json(
            json!({ "object": "inbox", "id": "i1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let inbox = ms
        .inboxes
        .create(&CreateInboxOptions::default())
        .await
        .unwrap();
    assert_eq!(inbox.email_address, "signup-bot-7f3k@acme.agents.example");
    assert!(inbox.shared);
    assert_eq!(inbox.quarantined, 1);
    assert!(inbox.receiving_address.is_none());
    let create = CreateInboxOptions {
        email_address: Some("bot@acme.dev".into()),
        name: Some("Bot".into()),
        from_name: Some("Acme Bot".into()),
        forwarding: Some(true),
        kind: Some("human".into()),
        expires_in: Some("7d".into()),
        safety: Some(InboxSafetyUpdate {
            accept_from: Some("allowlist".into()),
            allowlist: Some(vec!["*@stripe.com".into()]),
            send_cap: Some(None),
            ..Default::default()
        }),
    };
    ms.inboxes.create(&create).await.unwrap();
    let options = ListInboxesOptions {
        kind: Some("agent".into()),
        limit: Some(2),
        ..Default::default()
    };
    let list = ms.inboxes.list(Some(&options)).await.unwrap();
    assert!(list.data[0].name.is_none());
    assert_eq!(
        ms.inboxes
            .get("bot+sales@acme.dev")
            .await
            .unwrap()
            .pending_approval,
        1
    );
    let changes = UpdateInboxOptions {
        name: Some("Renamed".into()),
        from_name: Some(String::new()),
        expires_in: Some(None),
    };
    ms.inboxes.update("i1", &changes).await.unwrap();
    assert!(ms.inboxes.delete("i1").await.unwrap().deleted);
}

#[tokio::test]
async fn inbox_safety_codes_and_quarantine() {
    let server = MockServer::start().await;
    let safety = json!({
        "object": "inbox_safety", "inbox_id": "i1", "accept_from": "verified", "allowlist": [],
        "owner_addresses": ["ops@acme.dev"], "screening": true, "extract_codes": true,
        "send_policy": "reply_only", "require_approval": false, "private_codes": true,
        "send_cap": null, "forward_to": []
    });
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/safety"))
        .respond_with(ok_json(safety.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/inboxes/i1/safety"))
        .and(body_json(json!({
            "allowlist": [], "send_policy": "allowlist", "require_approval": true, "send_cap": 25
        })))
        .respond_with(ok_json(safety))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/codes/latest"))
        .and(query_param("from", "github.com"))
        .and(query_param("wait", "20"))
        .and(query_param("max_age", "600"))
        .respond_with(ok_json(json!({
            "object": "verification_code", "status": "found", "kind": "otp", "code": "482913",
            "url": null, "link_domain_matches_sender": null, "email_id": "r1",
            "from": "noreply@github.com", "received_at": "2026-10-12T10:00:00.000Z",
            "sender_trust": "external"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/quarantine"))
        .and(query_param("limit", "5"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "r1", "thread_id": "t1", "from": "x@y.dev", "subject": "Hi",
                       "received_at": "2026-10-12T10:00:00.000Z", "quarantine_reason": "injection",
                       "sender_trust": "external", "injection_score": 97, "phishing_score": null,
                       "virus_signature": null }]
        })))
        .mount(&server)
        .await;
    let release = json!({
        "object": "inbox_release", "inbox_id": "i1", "email_id": "r1",
        "released_at": "2026-10-12T10:05:00.000Z", "trusted_sender": "x@y.dev"
    });
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/quarantine/r1/release"))
        .and(body_json(json!({})))
        .respond_with(ok_json(release.clone()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/quarantine/r2/release"))
        .and(body_json(json!({ "trust_sender": true })))
        .respond_with(ok_json(release))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let rules = ms.inboxes.safety().get("i1").await.unwrap();
    assert_eq!(rules.send_cap, None);
    assert_eq!(rules.owner_addresses, vec!["ops@acme.dev"]);
    let changes = InboxSafetyUpdate {
        allowlist: Some(vec![]),
        send_policy: Some("allowlist".into()),
        require_approval: Some(true),
        send_cap: Some(Some(25)),
        ..Default::default()
    };
    ms.inboxes.safety().update("i1", &changes).await.unwrap();

    let options = LatestCodeOptions {
        from: Some("github.com".into()),
        wait: Some(20),
        max_age: Some(600),
    };
    let code = ms
        .inboxes
        .codes()
        .latest("i1", Some(&options))
        .await
        .unwrap();
    assert_eq!(code.code.as_deref(), Some("482913"));
    assert_eq!(code.sender_trust.as_deref(), Some("external"));

    let list_options = ListOptions {
        limit: Some(5),
        ..Default::default()
    };
    let held = ms
        .inboxes
        .quarantine()
        .list("i1", Some(&list_options))
        .await
        .unwrap();
    assert_eq!(held.data[0].injection_score, Some(97));
    let released = ms
        .inboxes
        .quarantine()
        .release("i1", "r1", None)
        .await
        .unwrap();
    assert_eq!(released.trusted_sender.as_deref(), Some("x@y.dev"));
    let trust = ReleaseQuarantinedOptions {
        trust_sender: Some(true),
    };
    ms.inboxes
        .quarantine()
        .release("i1", "r2", Some(&trust))
        .await
        .unwrap();
}

#[tokio::test]
async fn inbox_labels_cover_list_create_update_delete() {
    let server = MockServer::start().await;
    let label = json!({ "object": "inbox_label", "id": "l1", "name": "Billing", "color": "#FFAA00",
                        "created_at": "2026-10-12T10:00:00.000Z" });
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/labels"))
        .respond_with(ok_json(json!({
            "object": "list",
            "data": [{ "id": "l1", "name": "Billing", "color": "#FFAA00",
                       "created_at": "2026-10-12T10:00:00.000Z" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/labels"))
        .and(body_json(json!({ "name": "Billing" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(label.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/inboxes/i1/labels/l1"))
        .and(body_json(json!({ "color": "#00AAFF" })))
        .respond_with(ok_json(label))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/inboxes/i1/labels/l1"))
        .respond_with(ok_json(
            json!({ "object": "inbox_label", "id": "l1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let labels = ms.inboxes.labels().list("i1").await.unwrap();
    assert!(!labels.has_more);
    assert_eq!(labels.data[0].color, "#FFAA00");
    let create = CreateInboxLabelOptions {
        name: "Billing".into(),
        color: None,
    };
    let created = ms.inboxes.labels().create("i1", &create).await.unwrap();
    assert_eq!(created.object.as_deref(), Some("inbox_label"));
    let changes = UpdateInboxLabelOptions {
        color: Some("#00AAFF".into()),
        ..Default::default()
    };
    ms.inboxes
        .labels()
        .update("i1", "l1", &changes)
        .await
        .unwrap();
    assert!(
        ms.inboxes
            .labels()
            .delete("i1", "l1")
            .await
            .unwrap()
            .deleted
    );
}

#[tokio::test]
async fn inbox_threads_send_lists_as_csv() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/threads"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": true,
            "data": [{ "id": "t1", "subject": "Invoice 42", "from": "Stripe <billing@stripe.com>",
                       "to": ["bot@acme.dev"], "cc": [], "bcc": [],
                       "labels": [{ "id": "l1", "name": "Billing", "color": "#FFAA00" }],
                       "message_count": 2, "has_attachment": true, "has_draft": false, "read": true,
                       "received_at": "2026-10-12T10:00:00.000Z", "folder": "quarantine" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/threads/search"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "t1", "subject": null, "from": null, "to": [], "cc": [], "bcc": [],
                       "labels": [], "message_count": 1, "has_attachment": false, "has_draft": false,
                       "read": false, "received_at": "2026-10-12T10:00:00.000Z", "folder": "sent",
                       "matched_email_id": "m2", "highlights": { "subject": ["**Invoice** 42"] } }]
        })))
        .mount(&server)
        .await;
    let thread = json!({ "object": "inbox_thread", "id": "t1", "subject": null, "folder": "archive",
                         "labels": [], "read": true, "quarantined": 0 });
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/threads/t1"))
        .respond_with(ok_json(thread.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/inboxes/i1/threads/t1"))
        .and(body_json(
            json!({ "read": false, "folder": "archive", "label_id": "l1" }),
        ))
        .respond_with(ok_json(thread))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/inboxes/i1/threads/t1"))
        .respond_with(ok_json(
            json!({ "object": "inbox_thread", "id": "t1", "deleted": false }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let options = ListInboxThreadsOptions {
        folders: Some(vec!["inbox".into(), "archive".into()]),
        labels: Some(vec!["l1".into(), "l2".into()]),
        read: Some(false),
        include_quarantined: true,
        limit: Some(20),
        after: Some("t0".into()),
        ..Default::default()
    };
    let list = ms
        .inboxes
        .threads()
        .list("i1", Some(&options))
        .await
        .unwrap();
    assert_eq!(list.data[0].folder, "quarantine"); // an enum value this SDK never saw still parses
    assert_eq!(list.data[0].labels[0].name, "Billing");

    let search = SearchInboxThreadsOptions {
        query: Some("\"invoice 42\" -spam".into()),
        from: Some(vec!["stripe.com".into(), "ada".into()]),
        to: Some(vec!["bot@acme.dev".into()]),
        has_attachment: Some(true),
        start_date: Some("2026-10-01".into()),
        end_date: Some("2026-10-12".into()),
        folders: Some(vec!["sent".into()]),
        ..Default::default()
    };
    let found = ms.inboxes.threads().search("i1", &search).await.unwrap();
    assert_eq!(found.data[0].matched_email_id.as_deref(), Some("m2"));
    assert_eq!(found.data[0].highlights["subject"], vec!["**Invoice** 42"]);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0].url.query(),
        Some("limit=20&after=t0&folders=inbox%2Carchive&labels=l1%2Cl2&read=false&include_quarantined=true")
    );
    assert_eq!(
        requests[1].url.query(),
        Some("folders=sent&query=%22invoice%2042%22%20-spam&from=stripe.com%2Cada&to=bot%40acme.dev&has_attachment=true&start_date=2026-10-01&end_date=2026-10-12")
    );

    assert_eq!(
        ms.inboxes.threads().get("i1", "t1").await.unwrap().folder,
        "archive"
    );
    let changes = UpdateInboxThreadOptions {
        read: Some(false),
        folder: Some("archive".into()),
        label_id: Some("l1".into()),
    };
    ms.inboxes
        .threads()
        .update("i1", "t1", &changes)
        .await
        .unwrap();
    assert!(
        !ms.inboxes
            .threads()
            .delete("i1", "t1")
            .await
            .unwrap()
            .deleted
    );
}

#[tokio::test]
async fn inbox_thread_emails_list_get_reply_forward() {
    let server = MockServer::start().await;
    let inbound = json!({
        "id": "m1", "direction": "inbound", "from": "Ada <ada@x.dev>", "to": ["bot@acme.dev"],
        "cc": [], "bcc": [], "reply_to": [], "subject": "Hi", "message_id": "<a@x.dev>",
        "html": null, "text": "Hello", "attachments": [{ "id": "a1", "filename": null, "size": null }],
        "read": false, "received_at": "2026-10-12T10:00:00.000Z", "extracted_text": "Hello",
        "spam_verdict": "pass", "virus_verdict": "pass",
        "screening": { "verdict": "suspicious", "sender_trust": "unauthenticated",
                       "quarantine_reason": null, "reasons": [], "injection_score": null,
                       "phishing_score": null, "hidden_text_removed": false }
    });
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/threads/t1/emails"))
        .and(query_param("after", "m0"))
        .respond_with(ok_json(
            json!({ "object": "list", "has_more": false, "data": [inbound.clone()] }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/threads/t1/emails/m1"))
        .respond_with(ok_json(inbound))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/threads/t1/emails/m1/reply"))
        .and(header("idempotency-key", "tr-1"))
        .and(body_json(json!({ "text": "Thanks", "reply_all": true })))
        .respond_with(ok_json(json!({
            "id": "m2", "direction": "outbound", "from": "Signup bot <bot@acme.dev>",
            "to": ["ada@x.dev"], "cc": [], "bcc": [], "reply_to": [], "subject": "Re: Hi",
            "message_id": null, "html": null, "text": "Thanks", "attachments": [], "read": true,
            "received_at": "2026-10-12T10:01:00.000Z", "email_id": "m2", "thread_id": "t1",
            "inbox_id": "i1", "status": "queued"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/threads/t1/emails/m1/forward"))
        .and(body_json(json!({
            "to": "ops@acme.dev", "subject": "Fwd: Hi", "requires_approval": true
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(draft_json("pending_approval")))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let emails = ms.inboxes.threads().emails();
    let options = ListOptions {
        after: Some("m0".into()),
        ..Default::default()
    };
    let list = emails.list("i1", "t1", Some(&options)).await.unwrap();
    assert_eq!(
        list.data[0].screening.as_ref().unwrap().verdict,
        "suspicious"
    );
    assert_eq!(list.data[0].attachments[0].size, None);
    assert_eq!(
        emails
            .get("i1", "t1", "m1")
            .await
            .unwrap()
            .extracted_text
            .as_deref(),
        Some("Hello")
    );

    let reply = ReplyInThreadOptions {
        text: Some("Thanks".into()),
        reply_all: Some(true),
        ..Default::default()
    };
    let sent = sent_of(
        emails
            .reply("i1", "t1", "m1", reply.with_idempotency_key("tr-1"))
            .await
            .unwrap(),
    );
    assert_eq!(sent.email_id.as_deref(), Some("m2"));
    assert_eq!(sent.status.as_deref(), Some("queued"));

    let forward = ForwardInThreadOptions {
        to: "ops@acme.dev".into(),
        subject: Some("Fwd: Hi".into()),
        requires_approval: Some(true),
        ..Default::default()
    };
    let draft = draft_of(emails.forward("i1", "t1", "m1", &forward).await.unwrap());
    assert_eq!(draft.id, "d1");
}

#[tokio::test]
async fn inbox_drafts_cover_the_lifecycle() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/drafts"))
        .and(query_param("limit", "3"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "d1", "type": "standalone", "to": null, "cc": [], "bcc": [],
                       "subject": null, "snippet": "Hi Ada", "thread_id": null,
                       "reply_to_email_id": null, "updated_at": "2026-10-12T10:00:00.000Z",
                       "status": "draft", "approval_reasons": [] }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/drafts"))
        .and(body_json(json!({
            "to": ["ada@x.dev"], "subject": "Re: Hi", "text": "Thanks", "thread_id": "t1",
            "reply_to_email_id": "m1"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(draft_json("draft")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/inboxes/i1/drafts/d1"))
        .respond_with(ok_json(draft_json("pending_approval")))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/inboxes/i1/drafts/d1"))
        .and(body_json(
            json!({ "cc": null, "subject": null, "html": "<p>Thanks</p>" }),
        ))
        .respond_with(ok_json(draft_json("draft")))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/inboxes/i1/drafts/d1"))
        .respond_with(ok_json(
            json!({ "object": "inbox_draft", "id": "d1", "deleted": true }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/drafts/d1/reject"))
        .respond_with(ok_json(draft_json("rejected")))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let drafts = ms.inboxes.drafts();
    let options = ListOptions {
        limit: Some(3),
        ..Default::default()
    };
    let list = drafts.list("i1", Some(&options)).await.unwrap();
    assert!(list.data[0].to.is_none());
    assert_eq!(list.data[0].snippet.as_deref(), Some("Hi Ada"));
    let create = CreateInboxDraftOptions {
        to: Some(vec!["ada@x.dev"].into()),
        subject: Some("Re: Hi".into()),
        text: Some("Thanks".into()),
        thread_id: Some("t1".into()),
        reply_to_email_id: Some("m1".into()),
        ..Default::default()
    };
    assert_eq!(drafts.create("i1", &create).await.unwrap().r#type, "reply");
    let got = drafts.get("i1", "d1").await.unwrap();
    assert_eq!(got.expires_at.as_deref(), Some("2026-10-19T10:00:00.000Z"));
    let changes = UpdateInboxDraftOptions {
        cc: Some(None),
        subject: Some(None),
        html: Some(Some("<p>Thanks</p>".into())),
        ..Default::default()
    };
    drafts.update("i1", "d1", &changes).await.unwrap();
    assert!(drafts.delete("i1", "d1").await.unwrap().deleted);
    assert_eq!(drafts.reject("i1", "d1").await.unwrap().status, "rejected");
}

#[tokio::test]
async fn inbox_draft_send_posts_no_body_and_reports_a_held_draft() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/drafts/d1/send"))
        .and(header("idempotency-key", "ds-1"))
        .respond_with(ok_json(json!({
            "object": "inbox_draft", "id": "d1", "thread_id": "t1", "email_id": "e9"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/drafts/d2/send"))
        .respond_with(ResponseTemplate::new(202).set_body_json(draft_json("pending_approval")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/inboxes/i1/drafts/d3/send"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "statusCode": 403, "name": "approval_required", "message": "A person must approve this draft"
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let drafts = ms.inboxes.drafts();
    let sent = sent_of(drafts.send("i1", "d1", Some("ds-1")).await.unwrap());
    assert_eq!(
        sent,
        InboxDraftSent {
            object: "inbox_draft".into(),
            id: "d1".into(),
            thread_id: Some("t1".into()),
            email_id: Some("e9".into()),
        }
    );
    let requests = server.received_requests().await.unwrap();
    assert!(requests[0].body.is_empty());
    assert!(requests[0].headers.get("content-type").is_none());

    let draft = draft_of(drafts.send("i1", "d2", None).await.unwrap());
    assert_eq!(draft.status, "pending_approval");
    let requests = server.received_requests().await.unwrap();
    assert!(requests[1].headers.get("idempotency-key").is_none());

    let err = drafts.send("i1", "d3", None).await.unwrap_err();
    assert_eq!(err.name(), Some("approval_required"));
    assert_eq!(err.status_code(), Some(403));
}

#[tokio::test]
async fn inbox_fields_on_existing_resources() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/domains/d1"))
        .and(body_json(json!({ "receiving_catch_all": true })))
        .respond_with(ok_json(json!({
            "object": "domain", "id": "d1", "name": "x.dev", "status": "verified",
            "created_at": "2026-01-01T00:00:00Z", "region": "us-east-1",
            "open_tracking": false, "click_tracking": false, "tracking_subdomain": null,
            "capabilities": { "sending": "enabled", "receiving": "enabled" },
            "receiving_catch_all": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api-keys"))
        .and(body_json(
            json!({ "name": "agent", "permission": "full_access", "inbox_id": "i1" }),
        ))
        .respond_with(ok_json(json!({ "id": "k1", "token": "ms_x" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api-keys"))
        .respond_with(ok_json(json!({
            "object": "list", "has_more": false,
            "data": [{ "id": "k1", "name": "agent", "created_at": "2026-10-12T10:00:00.000Z",
                       "last_used_at": null, "inbox_id": "i1" }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/webhooks"))
        .and(body_json(json!({
            "endpoint": "https://x.dev/h", "events": ["email.received", "inbox.email.received"],
            "inbox_ids": ["i1"]
        })))
        .respond_with(ok_json(
            json!({ "object": "webhook", "id": "w1", "signing_secret": "whsec_x" }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/webhooks/w1"))
        .and(body_json(json!({ "inbox_ids": null })))
        .respond_with(ok_json(json!({ "object": "webhook", "id": "w1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/webhooks/w1"))
        .respond_with(ok_json(json!({
            "object": "webhook", "id": "w1", "endpoint": "https://x.dev/h",
            "created_at": "2026-10-12T10:00:00.000Z", "status": "enabled",
            "events": ["inbox.email.received"], "inbox_ids": ["i1"]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/usage"))
        .respond_with(ok_json(json!({
            "object": "usage", "cloud": true, "plan": "starter",
            "limits": { "emails_per_day": null, "domains": null, "inboxes": 25 },
            "today": { "emails_sent": 0, "resets_at": "2026-10-13T00:00:00Z" },
            "team": { "id": "tm1", "name": "Acme" }, "app_url": null
        })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let changes = UpdateDomainOptions {
        receiving_catch_all: Some(true),
        ..Default::default()
    };
    assert!(
        ms.domains
            .update("d1", &changes)
            .await
            .unwrap()
            .receiving_catch_all
    );
    ms.api_keys
        .create(&CreateApiKeyOptions {
            name: "agent".into(),
            permission: Some(ApiKeyPermission::FullAccess),
            inbox_id: Some("i1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let keys = ms.api_keys.list(None).await.unwrap();
    assert_eq!(keys.data[0].inbox_id.as_deref(), Some("i1"));
    ms.webhooks
        .create(&CreateWebhookOptions {
            endpoint: "https://x.dev/h".into(),
            events: vec!["email.received".into(), "inbox.email.received".into()],
            inbox_ids: Some(vec!["i1".into()]),
            ..Default::default()
        })
        .await
        .unwrap();
    let every_inbox = UpdateWebhookOptions {
        inbox_ids: Some(None),
        ..Default::default()
    };
    ms.webhooks.update("w1", &every_inbox).await.unwrap();
    assert_eq!(
        ms.webhooks.get("w1").await.unwrap().inbox_ids,
        Some(vec!["i1".to_string()])
    );
    assert_eq!(ms.usage.get().await.unwrap().limits.inboxes, Some(25));
}
