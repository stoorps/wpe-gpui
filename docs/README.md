# gpui-wpe documentation

`gpui-wpe` embeds WPE WebKit in GPUI Fast applications on native Linux Wayland.
The public crate supplies embedding and policy mechanisms. Applications own
browser controls, content preparation, consent, and workload-specific hardening.

The current tree is an experimental 0.1.0 candidate. These guides describe the
implemented interfaces; the [roadmap](roadmap.md) identifies unfinished work.

## Start here

| Task | Guide |
| --- | --- |
| Install dependencies and run the examples | [Getting started](getting-started.md) |
| Create, render, control, and close a view | [Embedding](embedding.md) |
| Browse the public type tree and operation contracts | [API structure](api.md) |
| Share application code with upstream gpui-webview | [GPUI compatibility](compatibility.md) |
| Configure navigation, resource, and scripting boundaries | [Security mechanisms](security.md) |
| Build an application-level email reader | [Email rendering](email-rendering.md) |
| Understand platform capabilities and exclusions | [Support and limitations](support.md) |
| Understand native ownership, rendering, and event servicing | [Architecture](architecture.md) |
| Run checks and collect meaningful evidence | [Testing](testing.md) |
| Measure resize, runtime, idle, and lifecycle behavior | [Performance](performance.md) |
| Run native diagnostics on a private compositor | [Isolated Wayland](isolated-wayland.md) |
| Update WebKit and prepare distribution materials | [Runtime updates and packaging](runtime-updates.md) |

## Status and evidence

The [feature matrix and roadmap](roadmap.md) distinguish the 0.1.0 contract from
later capabilities. The [evidence index](evidence/README.md) links dated local
measurements and verification summaries. A historical pass establishes its
recorded scope and source checkpoint; it does not certify a later tree or every
hardware configuration.

The [2026-10-31 validation record](history/2026-10-31-validation.md) preserves
implementation history, including failed diagnostics and the interrupted soak.

For exact Rust signatures, generate API documentation from the checkout:

```sh
cargo doc -p gpui-wpe --no-deps --open
```
