//! Unit tests over a mocked HTTP layer (wiremock). Each test mounts strict
//! matchers (method + path + body + headers); a mismatch yields a 404 the SDK
//! surfaces as an error, failing the `unwrap`. So an `Ok` result is itself proof
//! the request was shaped correctly.

use millionsend::*;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok_json(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
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
            "scheduled_at": null, "message_id": "m1", "last_event": "delivered"
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
    let cancelled = ms.emails.cancel("e1").await.unwrap();
    assert_eq!(cancelled.id, "e1");
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

// ---- audiences -----------------------------------------------------------

#[tokio::test]
async fn audiences_cover_create_get_list_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audiences"))
        .and(body_json(json!({ "name": "Users" })))
        .respond_with(ok_json(
            json!({ "object": "audience", "id": "a1", "name": "Users" }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/audiences/a1"))
        .respond_with(ok_json(json!({
            "object": "audience", "id": "a1", "name": "Users",
            "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/audiences"))
        .and(query_param("limit", "10"))
        .respond_with(ok_json(
            json!({ "object": "list", "data": [], "has_more": false }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/audiences/a1"))
        .respond_with(ok_json(
            json!({ "object": "audience", "id": "a1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    assert_eq!(ms.audiences.create("Users").await.unwrap().id, "a1");
    assert_eq!(ms.audiences.get("a1").await.unwrap().name, "Users");
    let options = ListOptions {
        limit: Some(10),
        ..Default::default()
    };
    assert!(!ms.audiences.list(Some(&options)).await.unwrap().has_more);
    assert!(ms.audiences.delete("a1").await.unwrap().deleted);
}

// ---- contacts ------------------------------------------------------------

#[tokio::test]
async fn contacts_create_scoped_and_top_level() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audiences/a1/contacts"))
        .and(body_json(
            json!({ "email": "c@x.dev", "first_name": "Ada" }),
        ))
        .respond_with(ok_json(json!({ "object": "contact", "id": "c1" })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/contacts"))
        .and(body_json(json!({ "email": "c@x.dev" })))
        .respond_with(ok_json(json!({ "object": "contact", "id": "c2" })))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let scoped = CreateContactOptions {
        audience_id: Some("a1".into()),
        email: "c@x.dev".into(),
        first_name: Some("Ada".into()),
        ..Default::default()
    };
    assert_eq!(ms.contacts.create(&scoped).await.unwrap().id, "c1");
    let top = CreateContactOptions::new("c@x.dev");
    assert_eq!(ms.contacts.create(&top).await.unwrap().id, "c2");
}

#[tokio::test]
async fn contacts_address_by_id_email_and_scoped_id() {
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
    Mock::given(method("GET"))
        .and(path("/audiences/a1/contacts/c1"))
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
    ms.contacts
        .get(ContactAddress::id("c1").in_audience("a1"))
        .await
        .unwrap();
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
async fn contacts_delete_and_list_scoped() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/contacts/c(%40|@)x\.dev$"))
        .respond_with(ok_json(json!({
            "object": "contact", "contact": "c1", "deleted": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/audiences/a1/contacts"))
        .and(query_param("after", "cur"))
        .respond_with(ok_json(
            json!({ "object": "list", "data": [], "has_more": false }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    assert!(
        ms.contacts
            .delete(ContactAddress::email("c@x.dev"))
            .await
            .unwrap()
            .deleted
    );
    let options = ListOptions {
        after: Some("cur".into()),
        ..Default::default()
    };
    ms.contacts.list(Some("a1"), Some(&options)).await.unwrap();
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
            "audience_id": "a1", "from": "a@x.dev", "subject": "News", "html": "<p>hi</p>"
        })))
        .respond_with(ok_json(json!({ "id": "b1" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/broadcasts/b1"))
        .respond_with(ok_json(json!({
            "object": "broadcast", "id": "b1", "name": null, "audience_id": "a1",
            "segment_id": null, "status": "draft", "created_at": "2026-01-01T00:00:00Z",
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
        audience_id: Some("a1".into()),
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
async fn segments_cover_create_get_list_update_delete_on_segments2() {
    let server = MockServer::start().await;
    let filter = json!({
        "match": "all",
        "conditions": [{ "field": "email", "op": "is_set" }]
    });
    Mock::given(method("POST"))
        .and(path("/segments2"))
        .and(body_json(
            json!({ "name": "Active", "audience_id": "a1", "filter": filter }),
        ))
        .respond_with(ok_json(json!({
            "object": "segment", "id": "s1", "name": "Active", "audience_id": "a1",
            "filter": filter, "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/segments2/s1"))
        .respond_with(ok_json(json!({
            "object": "segment", "id": "s1", "name": "Active", "audience_id": "a1",
            "filter": filter, "created_at": "2026-01-01T00:00:00Z", "contact_count": 42
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/segments2"))
        .and(query_param("before", "cur"))
        .respond_with(ok_json(
            json!({ "object": "list", "data": [], "has_more": false }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/segments2/s1"))
        .and(body_json(json!({ "name": "Renamed" })))
        .respond_with(ok_json(json!({
            "object": "segment", "id": "s1", "name": "Renamed", "audience_id": "a1",
            "filter": filter, "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/segments2/s1"))
        .respond_with(ok_json(
            json!({ "object": "segment", "id": "s1", "deleted": true }),
        ))
        .mount(&server)
        .await;

    let ms = MillionSend::with_base_url("ms_test", server.uri());
    let create = CreateSegmentOptions {
        name: "Active".into(),
        audience_id: "a1".into(),
        filter: SegmentFilter {
            match_: SegmentMatch::All,
            conditions: vec![SegmentCondition {
                field: "email".into(),
                op: "is_set".into(),
                value: None,
            }],
        },
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
