//! Opt-in end-to-end smoke test against a real MillionSend instance. Runs only
//! when `MILLIONSEND_API_KEY` is set (and `MILLIONSEND_BASE_URL` if not the
//! localhost default); otherwise it returns early. Exercises the audience +
//! contact lifecycle, which needs no verified sender domain.
//!
//! ```sh
//! MILLIONSEND_API_KEY=ms_... MILLIONSEND_BASE_URL=http://localhost:3001 \
//!   cargo test --test e2e -- --nocapture
//! ```

use millionsend::{CreateContactOptions, MillionSend, UpdateContactOptions};

#[tokio::test]
async fn audience_and_contact_lifecycle() {
    if std::env::var("MILLIONSEND_API_KEY").is_err() {
        eprintln!("skipping e2e: MILLIONSEND_API_KEY not set");
        return;
    }
    let ms = MillionSend::from_env().expect("MILLIONSEND_API_KEY");

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let audience = ms
        .audiences
        .create(format!("sdk-e2e-{stamp}"))
        .await
        .expect("create audience");
    let audience_id = audience.id;

    let email = format!("sdk-e2e-{stamp}@example.com");
    ms.contacts
        .create(&CreateContactOptions {
            audience_id: Some(audience_id.clone()),
            email: email.clone(),
            first_name: Some("Ada".into()),
            ..Default::default()
        })
        .await
        .expect("create contact");

    let address =
        |email: &str| millionsend::ContactAddress::email(email).in_audience(audience_id.clone());

    let fetched = ms.contacts.get(address(&email)).await.expect("get contact");
    assert_eq!(fetched.email, email);
    assert_eq!(fetched.first_name.as_deref(), Some("Ada"));

    ms.contacts
        .update(
            address(&email),
            &UpdateContactOptions {
                unsubscribed: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("update contact");

    let removed = ms
        .contacts
        .delete(address(&email))
        .await
        .expect("delete contact");
    assert!(removed.deleted);

    // A missing contact surfaces as a typed error, not a panic.
    let missing = ms
        .contacts
        .get(millionsend::ContactAddress::email(
            "does-not-exist@example.com",
        ))
        .await;
    assert!(missing.is_err());

    ms.audiences
        .delete(&audience_id)
        .await
        .expect("delete audience");
}
