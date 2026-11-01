# Update automation

These files take effect after they are committed and pushed to GitHub's default
branch. They have not been run on GitHub merely by adding them locally.

- `dependabot.yml` checks Cargo and GitHub Actions dependencies weekly and groups
  routine Rust patch updates. Enable Dependabot alerts/security updates in the
  repository settings where supported. Dependabot does not update the native
  WPE WebKit package and its alerts are not a native-WebKit advisory feed.
- `workflows/native-runtime-update.yml` checks the official Arch `extra/x86_64`
  WPE package daily and can be started manually with **Run workflow**. A newer
  package updates `native-runtime.json` and opens/refreshes one patch PR. The
  checker never installs or executes the package and does not merge the PR.

The native manifest is consumed by `scripts/setup-local-arch.sh`. It pins an
archive URL and SHA256; installation also requires the package's independent
`pacman-key` signature verification. Candidate checksums are computed from the
official archive download, not treated as an alternative signature authority.
Close running demo instances before replacing a local runtime. The installer
uses fresh staged contents so updates do not retain obsolete native files.

## Repository prerequisites

Allow GitHub Actions to create pull requests in **Settings → Actions → General**.
The native-update job requests `contents: write` and `pull-requests: write`; no
personal access token is required. If organization policy blocks those rights,
the workflow will fail rather than silently claim that a PR was created. Retain
notifications for failed scheduled jobs and use workflow_dispatch when needed.
Schedules depend on GitHub's default-branch workflow/scheduling rules and are not
a guaranteed security-response service.

PRs created with `GITHUB_TOKEN` generally do not trigger additional PR/push
workflows. Reviewers must run the prescribed validation explicitly, or configure
an appropriately scoped GitHub App token and its workflow policy if automatic PR
validation is desired. Do not auto-merge a native runtime update based solely on
metadata tests or package version numbers.

The daily checker follows **Arch package availability**. It cannot establish CVE
applicability, distro backports, upstream-to-distro lag, or compatibility on other
operating systems. Review upstream WPE/WebKit and distro advisories when assessing
a patch. Existing system-package consumers still obtain updates from their system
package channel; this manifest controls the optional local Arch runtime only.

## Local commands

```sh
python3 scripts/update-native-runtime.py --check  # Availability; no file writes
python3 scripts/update-native-runtime.py          # Propose metadata update
python3 scripts/update-native-runtime.py --print-install-info  # No network
python3 -m unittest discover -s tests -p 'test_native_runtime_update.py'
```

Before merging/installing a candidate, review the changed native version and
advisories, verify/install with `just setup-local` on a supported Arch machine,
then run `just check`, `just test-interop` and `just smoke`. Confirm the loaded
runtime and exercise resizing/input/content isolation. Offline updater tests mock
transport and signatures to test required verification and replacement behavior;
they do not claim cryptographic authenticity or native rendering compatibility.
