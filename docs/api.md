# API structure

This tree describes the current crate, rather than reserved names for future
features. Generate exact signatures with `cargo doc -p gpui-wpe --no-deps --open`.
Construction and ordinary embedding follow `gpui-webview` conventions; see
[compatibility](compatibility.md) for intentional differences.

```text
gpui_wpe
├── WebView                          Render + Focusable + WebViewEvent emitter
│   ├── build(builder, window, cx) -> Result<Self>
│   ├── load_url / try_load_url / load_html / load_document
│   ├── back / forward / reload / stop_loading
│   ├── state / capabilities / handle / close
│   ├── set_geometry / set_bounds / bounds / viewport_bounds
│   ├── set_visible / show / hide / visible / block_input
│   ├── set_zoom / zoom / set_theme
│   ├── copy / cut / paste / paste_text / select_all / undo / redo
│   ├── request_selected_text / selected_text / last_copied_text
│   ├── find / find_next / find_previous / clear_find / find_state
│   ├── hit_test / editing_state / provide_resource
│   └── diagnostic operations        see diagnostic boundary below
├── WebViewBuilder
│   ├── new / with_url / with_html / with_document
│   ├── with_settings / with_background_color / with_resource_policy
│   ├── with_navigation_handler
│   └── with_resource_provider       with_resource_handler is also available
├── WebViewHandle                    Clone; weak, UI-thread-local
│   └── load_url / set_zoom / close
├── WebViewState / WebViewCapabilities
├── WebViewEvent / SelectionState
├── Document / DocumentId / ResourceRequest / ResourceResponse / ResourceResponder
├── Error / Result<T>
├── settings::{WebViewSettings, JavaScript, Theme}
├── policy::{ResourcePolicy, NavigationRequest, NavigationDecision}
├── resource                         document/resource types and URI/key helpers
├── geometry::Rect                   logical GPUI Point<Pixels> + Size<Pixels>
├── find::{FindOptions, FindState}
├── Action / EditingState / HitTest / Status
├── LoadEvent / LoadPhase / WebViewFailure / FailureKind
└── runtime_info / live_metrics      engine provenance and bridge counters
```

`WebViewState` contains `url`, `title`, `loading`, `closed`, `can_go_back`,
`can_go_forward`, `document_id`, and `error`. Construction and request acceptance
are distinct from asynchronous load completion. A rendered frame does not imply
that the page finished loading.

`WebViewEvent` variants are `StateChanged`, `NavigationRequested`, `FindChanged`,
`Error`, `Closed`, `SelectionCopied`, `SelectionChanged`, `LoadChanged`,
`ContextMenuRequested`, and `Failure`. The entity also declares
`EventEmitter<DismissEvent>`, but currently does not emit it; use `Closed` for
native lifetime state.

## Defaults and errors

| Setting | Default / contract |
| --- | --- |
| `JavaScript` | `Enabled`; `Disabled` is an explicit construction choice. |
| `ResourcePolicy` | `Unrestricted`; `ProvidedOnly` requires disabled JavaScript. |
| `Theme` | `Auto`; independent `Light` / `Dark` are rejected as unsupported. |
| Background | Opaque white; finite RGBA channels from 0 to 1. |
| User zoom | Finite values from 0.5 through 3.0; invalid values return an error. |
| Document bytes | 4 MiB; configurable up to the native 8 MiB hard cap. |
| Resource bytes | 16 MiB each, 32 MiB aggregate; native hard caps are 32 / 64 MiB. |

Typed errors are `Closed`, `Cancelled`, `Unsupported`, `InvalidInput`,
`PolicyDenied`, and `Native`. `load_url`, `show`, and `hide` are convenience methods
that queue failures as events. Prefer `try_load_url` and fallible setters when the
caller needs immediate validation. Other asynchronous failures still arrive later.

## Selection and appearance

`request_selected_text()` requests an asynchronous `SelectionChanged` event without
writing the host clipboard. `selected_text()` is an alias, not a synchronous getter.
It requires an engine session which has never enabled page scripting and permits
only one outstanding query per engine session. A query timeout/cancellation may
invalidate the shared clipboard until restart. Ordinary Copy/Paste remain available
in mixed sessions until a protocol failure. Check `capabilities().selected_text`.

Native per-view `prefers-color-scheme` overrides are unavailable. `Theme::Auto`
follows WPE's shared settings; application-owned HTML/CSS can provide a reading
surface without changing another view's native media preference.

## Diagnostic boundary

`evaluate` / `take_evaluation`, `sample_pixel`, `metrics`, geometry timestamps, and
process-termination helpers support trusted diagnostics. Evaluation is rejected
when JavaScript is disabled. They are not a production IPC interface, isolated
script world, concurrent typed evaluation API, or page-exposed native bridge.
Do not grant untrusted page content access to them. Readback can perturb timing.

The entity also retains low-level compatibility/diagnostic helpers such as
`action`, `status`, and `take_new_uri`. Ordinary hosts should use the managed
entity and named operations. Native servicing and shutdown are library-owned;
there are no public core `pump` or `shutdown` functions for a host to call.

Session/profile management, IPC, initialization scripts, production script worlds,
advanced permission/download hooks, raw Wry handles, and accessibility bridge types
are not part of the current API. Their desired capabilities live in the
[roadmap](roadmap.md), without pre-allocating unsupported public type names.
