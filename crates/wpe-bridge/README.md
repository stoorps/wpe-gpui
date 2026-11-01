# wpe-bridge

Experimental UI-thread-local Rust/C WPEPlatform adapter used by `gpui-wpe`.
Ordinary applications should use `gpui-wpe`'s managed entity and lifecycle rather
than calling this low-level bridge directly.

The bridge links dynamically to WPE WebKit/WPEPlatform 2.54+ and native
EGL/GLES/Wayland/xkbcommon/DRM libraries. It implements the custom WPE platform,
frame import/presentation, owned readiness snapshots and callbacks. All WPE,
GLib, EGL and view operations belong to the owning UI thread. Unsafe constructors
require borrowed native handles to remain live for their documented lifetime;
workers must not dispatch the native main context.

Restricted provided-resource views attach an engine-owned WebKit content filter
before loading application HTML. Its constant HTTP(S) block rule covers resource
requests and speculative transport hints; the immutable default CSP provides
additional restrictions. Disabling the public `LinkPreconnect` feature is an
additional defense. WPE 2.54's deprecated DNS-prefetch setting is a no-op and is
not used as a security boundary. WebKit's
[preconnect](https://github.com/WebKit/WebKit/blob/wpewebkit-2.54.2/Source/WebCore/loader/LinkLoader.cpp#L290)
and [DNS-prefetch](https://github.com/WebKit/WebKit/blob/wpewebkit-2.54.2/Source/WebCore/loader/FrameLoader.cpp#L4617)
paths both consult content filters before opening their transport.

Filter compilation is asynchronous and shared across restricted views. Only the
latest queued document is retained for loading, `loading` remains true while it
waits, and no WebKit frame is submitted before policy readiness. Compilation
failure or the five-second deadline fails closed with a typed load failure;
restricted loading stays unavailable for that engine session. The private
temporary store contains constant policy bytes, never document content. Closing
a waiting view removes its callback eligibility; shutdown cancels compilation
and gives cleanup up to 100 ms. A slower compiler callback retains its own
retired job until completion, without retaining or accessing closed views.

The engine/presenter ownership and orderly shutdown rules are part of the safety
contract. Check the containing project's source, tests and declared capabilities
before embedding directly. This is an internal integration boundary, not a stable
cross-platform raw-native-handle API.

Project code is Apache-2.0; see the packaged `LICENSE-APACHE` and the
[repository](https://github.com/stoorps/wpe-gpui-fast)'s `NOTICE`/`THIRD_PARTY.md`.
WPE/WebKit and native dependencies retain their mixed component licenses,
including applicable LGPL terms. Packaging this bridge does not replace those
license, notice or source-material requirements for native binary distributions.

The optional `native-failure-injection` feature compiles trusted diagnostic
constructor failpoints. It is disabled by default; production builds ignore
`GPUI_WPE_FAIL_STAGE` and `GPUI_WPE_FAIL_SKIP`. Use the repository's
`scripts/test-native-failures.py` on an isolated Wayland display for the durable
partial-initialization checks.
The same opt-in feature enables the constant-filter compiler fault used by
`scripts/test-native-policy.py`; ordinary builds ignore
`GPUI_WPE_POLICY_FAULT`. The policy suite checks pending close, replacement,
async failure, rejected follow-up loads, and temporary-store cleanup.
