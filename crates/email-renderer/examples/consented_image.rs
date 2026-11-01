//! Explicit app consent flow, optionally exercised against a public HTTPS image.
//! Downloads stay in memory; no content/assets are written or redistributed.
use email_renderer::{Limits, RemoteConsent, RemoteFetcher, prepare_email};
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args()
        .nth(1)
        .ok_or("pass a public HTTPS image URL")?;
    let url = url::Url::parse(&input)?;
    let attribute = url.as_str().replace('&', "&amp;").replace('"', "&quot;");
    let limits = Limits::default();
    let mut email = prepare_email(
        1,
        &format!("<p>Consented image example</p><img src=\"{attribute}\" alt=\"Remote image\">"),
        "Consented image example",
        &BTreeMap::new(),
        &limits,
    )?;
    assert!(
        email.resources.is_empty(),
        "remote bytes must be absent before consent"
    );
    if email.remote_images.len() != 1 {
        return Err("image URL was not accepted by preparation".into());
    }
    // Running this explicit example grants this one document's app-owned consent.
    let consent = RemoteConsent::for_document(email.document_id);
    let report = email.approve_remote_images(&consent, &RemoteFetcher::new(limits))?;
    if report.loaded != 1 {
        return Err(format!("image not loaded: {:?}", report.failed).into());
    }
    let image = email
        .resources
        .values()
        .next()
        .ok_or("missing approved resource")?;
    println!(
        "consent flow passed: {} bytes, {}",
        image.bytes.len(),
        image.mime
    );
    Ok(())
}
