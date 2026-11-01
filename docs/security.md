# Security mechanisms and application responsibilities

`gpui-wpe` is a generic embedding crate. Its defaults use normal engine networking
and enabled JavaScript. Loading application HTML does not sanitize it or select an
email/browser profile. Choose an enforcement contract explicitly before creating
a view, and keep workload-specific preparation and trust decisions in the app.

## Restricted supplied-resource views

Construct a restricted view with disabled JavaScript and `ProvidedOnly`:

```rust
use gpui_wpe::{WebViewBuilder, policy::ResourcePolicy};
use gpui_wpe::settings::{JavaScript, WebViewSettings};

fn restricted_builder() -> WebViewBuilder {
    WebViewBuilder::new()
        .with_settings(WebViewSettings {
            javascript: JavaScript::Disabled,
            ..Default::default()
        })
        .with_resource_policy(ResourcePolicy::ProvidedOnly)
}
```

`ProvidedOnly` with enabled JavaScript fails construction. Policy/settings are fixed
per view. Application-supplied documents and their explicitly provided resources
use generated document-scoped schemes. Diagnostic JavaScript evaluation is denied
in a scripting-disabled view. The native restricted document CSP adds restrictions
to resource types, frames, forms, and active content; CSP alone is not the network
boundary.

Before loading a restricted document, the native layer attaches an engine-owned
WebKit content filter with a constant HTTP(S) block rule. This covers resource
requests and speculative transport hints on the reference WPE implementation.
The public `LinkPreconnect` feature is also disabled and checked. The deprecated
DNS-prefetch setting in WPE 2.54 is a no-op and is not relied on for enforcement.

Filter compilation is asynchronous and shared. While waiting, `loading` stays true,
only the newest queued document is retained, and no WebKit frame is submitted.
Compilation failure or the five-second deadline leaves restricted content blank
and emits a typed load failure. Restricted loading then remains unavailable for
that engine session; there is no unrestricted fallback or automatic unsafe retry.

The temporary policy store is private and contains only constant policy bytes,
never document content. Closing a waiting view removes its callback eligibility.
Shutdown cancels the job and permits bounded cleanup; a late compiler callback owns
its retired state without accessing a closed view.

The [native policy and recorder checks](testing.md) verify pending close/replacement,
compiler failure, and zero HTTP requests/TCP accepts for their fixtures. DNS packets
were not independently captured in the historical run; the audited native hint
path consults content filters. These bounded checks are not a complete hostile-
content audit or proof of isolation against every engine bug.

## Navigation and privileged interactions

`with_navigation_handler()` receives `NavigationRequest` and returns `Allow`,
`Cancel`, or `Delegate`. Delegation emits `NavigationRequested` for host UX. Treat
destination and available gesture metadata as decision inputs; do not infer a
trusted user gesture where it is unavailable. A request for a new window does not
automatically create a GPUI view or launch another application.

Restricted views refuse general network navigation. Handle external HTTP(S) links
and mailto in application UI, with application-owned launch/compose policy. Local
anchors are a reader concern; test them with the chosen sanitization policy. Do
not enable unrestricted WebKit networking to implement a “load images” button.

Unhandled native JS dialogs, file choosers, authentication, permissions, and downloads
are explicitly cancelled/denied. Fullscreen is disabled. Rich host hooks for these
operations are not implemented; there is no implicit authorization from embedding
the view. Do not bypass TLS verification or disable the WebKit sandbox to load a page.

## Bounds and stale work

| Boundary | Current bound |
| --- | --- |
| Core default document | 4 MiB; native hard cap 8 MiB. |
| Core default resource | 16 MiB each; native hard cap 32 MiB. |
| Core default resource aggregate | 32 MiB; native hard cap 64 MiB. |
| Native document resource entries | 1,024. |
| Pending async responses | 128 library slots; native pending waits are also bounded. |
| Queued/delivering async payloads | 64 MiB shared library budget. |
| Async resource deadline | Five seconds. |
| Physical viewport | At most 16,384 pixels per dimension and 16,777,216 total pixels. |
| Native frame/cache ownership | At most four pending frames and eight imports per live view. |

The host may lower byte limits. Invalid MIME values, resource keys, geometry, and
over-budget loads return errors. Resource responders are exactly-once capabilities;
drop, expiry, document replacement, and close revoke obsolete work. Worker results
must still be checked against the app's current message/generation before installation.
These quotas do not bound every WebKit/driver internal allocation or guarantee
availability against all malformed content.

Sessions are ephemeral per view; persistent profile and cookie-management APIs are
absent. Replacing a document revokes its pending work, but is not a promise of a new
network session or a new operating-system process. Choose separate views when an
application needs independent session lifetime, and review the shared constraints
in [architecture](architecture.md).

## Application hardening

An email app owns MIME parsing, HTML/CSS sanitization, CID/attachment validation,
remote-image consent and fetching, external-link UX, caches, retention, and a safe
host-rendered text fallback. The [email guide](email-rendering.md) demonstrates
these layers without changing core defaults. Work on hostile parsing and image
normalization belongs on bounded application workers, not the UI thread.

Keep message bodies, credentials, resource URLs, and script contents out of default
diagnostics. Native tracing is opt-in and still includes coordinates and pointer
identities. WebKit must receive timely updates through the application's chosen
runtime channel; see [runtime updates](runtime-updates.md). An up-to-date Cargo
lockfile does not update the native engine.
