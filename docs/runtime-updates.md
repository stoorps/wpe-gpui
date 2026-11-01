# Runtime updates and packaging

Native WebKit updates are part of consuming a webview, especially for untrusted
HTML. Cargo dependency updates alone do not update WebKit. This project's
single-maintainer contract is automated discovery and reviewable patch proposals,
without automatic merging or a guaranteed human triage/deployment deadline.

## Choose and identify the runtime

System-runtime applications receive WPE through their distribution's package
channel and must install its security updates. The repository's optional Arch
development runtime is controlled separately by
[`native-runtime.json`](../native-runtime.json). A present `.deps/` runtime takes
precedence over system libraries; updating the system does not update that copy.

Use `gpui_wpe::runtime_info()` to report the loaded WebKit version/library path,
and record installation provenance. The tracked pin feeds the actual local
installer, rather than serving as decorative version metadata. Its archive SHA256
and independent package signature are both verified. Close running instances before
replacing a local runtime; staged installation avoids obsolete package files.

The private development overlay is not a complete production distribution. A
consuming app must choose supported system packages, a managed runtime, or an app
package, and verify helper paths, sandbox, graphics devices, fonts, install/update/
uninstall and failure behavior on clean systems.

## Update proposals

| Automation | Scope |
| --- | --- |
| [Dependabot](../.github/dependabot.yml) | Weekly Cargo and GitHub Actions proposals; native WebKit is outside its Cargo graph. |
| [Native runtime workflow](../.github/workflows/native-runtime-update.yml) | Daily/manual official Arch package check; updates the installer pin and proposes a patch PR when a newer package is available. |

The native checker does not install/execute candidate packages or merge its PR.
It follows Arch availability, not every upstream advisory, distro backport or CVE.
A numeric version alone cannot establish vulnerability status. Review engine/distro
advisories and compatibility; users remain responsible for deployment.

Repository maintainers must enable Actions-created PRs and appropriate workflow
permissions. Default-branch scheduling and `GITHUB_TOKEN` restrictions affect when
jobs run and whether a bot PR triggers further CI. The authoritative setup details
are in [the automation README](../.github/README.md). Configuration in the tree is
not evidence that a hosted job or patch PR has actually run.

## Review an update

```sh
python3 scripts/update-native-runtime.py --check
python3 scripts/update-native-runtime.py --print-install-info
```

`--check` reads availability without changing the pin. The ordinary command without
those flags proposes a metadata update. Review the package source, version, archive
and checksum before installation. Install via `just setup-local` on a supported Arch
machine and confirm the actual loaded engine.

Run `just check`, `just smoke-local`, debug/release interop, native policy/failure/
window checks, and relevant resize/runtime measurements. Test representative live
content separately; the remote smoke must complete loading and exit successfully.
Keep restrictions, TLS validation and the sandbox enabled. Tool tests with mocked
transport/signatures test verification paths, not native compatibility or package
authenticity in a live environment.

Do not automatically roll back to a known-vulnerable engine. If an app cannot supply
a safe compatible engine, disable affected HTML rendering and use host-rendered
plain text. A fallback page rendered in the same vulnerable engine is insufficient.

## Distribution and licenses

Project code is Apache-2.0; retain [LICENSE-APACHE](../LICENSE-APACHE),
[NOTICE](../NOTICE), upstream source headers, and [THIRD_PARTY.md](../THIRD_PARTY.md).
The wrapper's permissive license does not relicense WPE or other dependencies.
WebKit has mixed component licenses including LGPL terms; the email consumer also
uses MPL-2.0 `cssparser`.

```sh
just licenses
cargo package -p gpui-wpe --list --allow-dirty --offline
cargo package -p wpe-bridge --list --allow-dirty --offline
```

The inventory records the locked target/feature graph and collects discovered
license/notice texts and hashes under `target/`. Review missing materials. Add the
native/runtime dependencies and all files actually shipped, along with applicable
notices and corresponding source/relinking materials. A package file listing is
not a publication or verification of a complete binary distribution. The Cargo
inventory is neither a complete native BOM nor a legal-compliance determination.
