# Rendering email content

`crates/email-renderer` is application-layer code. It does not depend on GPUI,
WPE, or the core webview crate and does not change the core's defaults. Its caller
supplies decoded HTML/plain text and a validated Content-ID/attachment map from
the application's MIME parser.

Use the [restricted construction example](security.md#restricted-supplied-resource-views)
for the view, then prepare documents on an application worker. An integration
helper can preserve the core-generated identity throughout preparation:

```rust
use std::collections::BTreeMap;
use email_renderer::{InlineResource, Limits, PreparedEmail, prepare_email};
use gpui_wpe::Document;

fn prepare_message(
    html: &str,
    text: &str,
    inline: &BTreeMap<String, InlineResource>,
) -> Result<(Document, PreparedEmail), Box<dyn std::error::Error>> {
    let document = Document::html("");
    let prepared = prepare_email(document.id().get(), html, text, inline, &Limits::default())?;
    let mut document = document.with_html(prepared.html.clone());
    for (key, image) in &prepared.resources {
        document = document.with_resource(key, image.bytes.clone(), &image.mime)?;
    }
    Ok((document, prepared))
}
```

Send the completed `Document` to the UI thread and load it into the already
restricted view. Retain `PreparedEmail` for remote-image consent and host fallback.
When applying worker results, also check the application's current message generation.

## Default preparation limits

| Input/work | Default |
| --- | --- |
| HTML | 4 MiB, 50,000 nodes, depth 128 (configurable hard maximum 256). |
| Images | 64 images; 8 MiB each, 32 MiB encoded/normalized resources in total. |
| Decode | 8 million pixels/image, 64 MiB decoder allocation limit, 128 MiB aggregate decoded footprint accounting for at least RGBA8. |
| Fetch | Three redirects, 15-second request timeout, three-second DNS timeout. |

Align these with the core's own [resource limits](security.md#bounds-and-stale-work).
Compressed file size alone does not bound decoded image memory. The limits bound
specific paths, not every parser/engine allocation or all CPU time.

## Integration contract

1. Allocate the core `DocumentId` before preparing the content, and pass its
   nonzero `u64` value to `prepare_email(id, html, plain_text, inline, &limits)`.
   Allocate a **fresh ID** whenever switching or rebuilding a message; never reuse
   another message's ID. Consent matches that identity, not sender HTML or mailbox IDs.
2. Use a dedicated view with JavaScript disabled and the core's provided-resource
   policy enabled **before** loading. The app's sanitized HTML also carries a
   restrictive CSP, but that does not replace the engine-level resource policy.
3. Copy `PreparedEmail.html` and its resource map into the document with that same
   ID. Resource URLs are `gpui-resource://{id}/{opaque-key}`. Relative and arbitrary
   application file resources are not exposed by this consumer.
4. Handle links as intentions. `allowed_link()` admits anchors and HTTP(S)/mailto;
   it never launches another application. The host owns destination display,
   confirmation, same-document scrolling and compose/open actions.
5. Keep `fallback_text` for a host-owned GPUI text element if HTML rendering is
   unavailable, oversized, or must be disabled for a vulnerable runtime. The HTML
   fallback is useful only when it is safe to keep using the engine.

The crate preserves tables, basic email layout attributes, Unicode content, and a
bounded allowlist of tokenized inline CSS declarations. It parses markup with
the maintained Kuchikiki fork and sanitizes the result with Ammonia. CSS values are parsed with cssparser;
escaped `url()`, active expressions, imports, variables and resource-loading
properties are rejected rather than removed with regular expressions. Duplicate
anchor IDs are removed after the first occurrence. Parsed DOM depth defaults to
128 and can only be configured from 1 through 256, before recursive serialization.
HTML templates are rejected by an html5ever tokenizer before DOM construction:
their separate document fragments bypass ordinary DOM depth traversal. This
conservative preflight can also reject template-like markup inside a script that
would otherwise be stripped. Byte/node quotas bound parser input; parsing and
image normalization belong on a worker, since hostile malformed HTML can still
consume substantial CPU within these limits.

## Inline and remote images

The demo's offline email fixture includes an inline image, a public HTTPS W3C
logo, and a deliberately denied localhost tracking image. Selecting “Load remote
images for this message” requires internet access to load the logo; the localhost
image stays blocked after consent. A successful run reports one image loaded and
one denied or failed.

Construct `InlineResource::new(bytes, mime, &limits)` for allowed CID raster
images. PNG/JPEG/GIF/WebP input is decoded within pixel/allocation limits and
normalized to a **static PNG**. This deliberately removes animation and metadata.
SVG is not accepted. Publicly constructed resource fields are revalidated before
use, normalized PNG output writes are bounded, and aggregate resource quotas are
checked. Canonical Content-ID collisions are rejected. Image orientation is applied before metadata is discarded. MIME extraction,
arbitrary attachments and attachment save/open UX belong to the application.

Remote `<img>` URLs are replaced with document-scoped resource placeholders.
Only image elements surviving the sanitizer enter the approval worklist; removed
SVG/foreign/active subtrees cannot cause hidden approval fetches. There is no
WebKit remote request before consent. An app may expose a per-message
“load remote images” action:

- Create `RemoteConsent::for_document(id)` in response to that action. Keep a
  clone and call `revoke()` when the message changes, closes, or consent is reset.
- Run `prepared.approve_remote_images(&consent, &RemoteFetcher::new(limits))` on
  an application worker thread. Never call the blocking fetcher from GPUI's UI
  thread. Recheck the document ID/generation before applying its result.
- Install the approved resource bytes and reload/replace that same message's
  document to retry placeholders that may already have failed. A failed fetch
  stays a placeholder; it does not restore the remote URL to WebKit.
- For an application image proxy or cache, use `approve_remote_images_with()`.
  The supplied fetch function owns its network policy; returned images are still
  decoded/revalidated against this document's limits.

The built-in fetcher uses HTTPS on port 443, validates DNS results against a
conservative public-address policy, pins those addresses into the HTTP connection,
and checks every redirect again. It uses no environment proxy, URL credentials,
cookies or referrer and keeps TLS verification enabled. Encoded HTTP responses are
rejected. Redirect count, encoded/decoded image size and request deadlines are
bounded. Error text does not embed the resource URL or credentials.

DNS resolution uses one bounded worker queue. A system resolver call cannot be
forcibly interrupted; the caller has a deadline and abandoned results are dropped.
A stuck system resolver therefore makes later DNS work fail/timeout rather than
creating unbounded resolver threads. Consent is checked before requests/redirects,
during body reads and before resource installation. A blocked connect/read can
only finish at its configured network timeout, so revocation is not an assertion
that an in-flight TCP operation is instantly terminated. The host must discard
results for obsolete message generations.

## Evidence and limits

The crate's offline tests cover active markup/event/CSP replacement attempts,
CSS URL/import filtering, CID scoping/traversal, malformed and oversized images,
image MIME mismatch, data-image normalization, remote placeholders, approval,
revocation during a fetch, partial fetch failure, resource quotas, Unicode/plain
text fallback and pre-network local-address/credential/scheme rejection. The
original `fixtures/email-security.html` corpus adds malformed SVG/MathML, encoded
active links, escaped CSS URLs and attempts to replace the CSP. Tests also cover
12,000-level DOM/template attacks, canonical CID collisions, normalized-output
write limits, decoder allocation limits, stale consent/reprepare behavior and
unusual local IPv4 spellings after URL canonicalization.

These tests do **not** constitute a security review of the application, an email
fidelity corpus, or proof of production readiness. The live HTTPS path and real
image-proxy deployment need their own network/TLS/redirect integration coverage.
The app's engine-level zero-network behavior needs the core native request-recorder
suite; this crate's string/DOM tests alone do not prove WebKit made no requests.

Stylesheets/media queries and remote fonts are currently removed. `srcset`, CSS
background images, SVG, active embeds and animation are unsupported; alt text and
host placeholders are the fallback. Basic safe inline layout is preserved rather
than promising pixel-perfect fidelity for arbitrary newsletters. The initial
scope is a bounded reading pane; auto-height conversation layout, composition,
full accessibility and rich clipboard are separate capabilities.

The demo's View menu offers Light/Dark **reading surface** settings. These apply
application-owned default background/foreground CSS and matching initial clear
color to a newly prepared document and newly restricted view. Explicit sender
colors and images remain intact; there is no inversion or process-wide
`prefers-color-scheme` override. Rebuilding allocates a fresh document ID, revokes
old remote consent and ignores stale worker completions. WPE's global display
appearance does not provide a safe per-view media-query override on this target.

## Explicit live consent example

The optional worker-side example exercises preparation, an explicit per-document
consent token, TLS/public-address policy, bounded download/decode and normalized
resource installation. It keeps downloaded content in memory:

```sh
cargo run -p email-renderer --example consented_image -- \
  https://fullsphere.co.uk/misc/free-template/images/logo-white-background.jpg
```

The [2026-10-31 checkpoint](history/2026-10-31-validation.md) recorded a
64,843-byte static PNG from this image. This
is one successful live HTTPS path, not general DNS/redirect/TLS coverage. Unit
checks remain offline. JPEG/PNG/WebP orientation is applied before metadata is
dropped; a JPEG Exif-rotation regression checks normalized dimensions.

## Related guides

Use the [embedding guide](embedding.md) for view construction and lifecycle,
[security guide](security.md) for core enforcement and failure behavior, and
[support matrix](support.md) for capabilities that remain unavailable.
