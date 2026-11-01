#!/usr/bin/env bash
# Arch-only fallback. Close browser-demo before replacing its native runtime.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
mkdir -p .deps/downloads
runtime_info="$(python3 scripts/update-native-runtime.py --print-install-info)"
mapfile -t runtime_fields <<< "$runtime_info"
url="${runtime_fields[0]}"
checksum="${runtime_fields[1]}"
package=".deps/downloads/${url##*/}"
curl --fail --location --proto '=https' --proto-redir '=https' "$url" --output "$package"
curl --fail --location --proto '=https' --proto-redir '=https' "$url.sig" --output "$package.sig"
printf '%s  %s\n' "$checksum" "$package" | sha256sum --check -
pacman-key --verify "$package.sig" "$package"
# Extract into a fresh directory: an update must not retain obsolete native files.
stage="$(mktemp -d .deps/.wpe-extract.XXXXXXXX)"
trap 'python3 -c '\''import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)'\'' "$stage"' EXIT
tar -xf "$package" -C "$stage"
python3 - "$stage" <<'PY'
from pathlib import Path
import shutil
import sys
stage = Path(sys.argv[1])
prefix = Path('.deps/usr').resolve()
for pc in (stage / 'usr/lib/pkgconfig').glob('*.pc'):
    pc.write_text(pc.read_text().replace('prefix=/usr', f'prefix={prefix}').replace('libdir=/usr/lib', f'libdir={prefix}/lib'))
if not (stage / 'usr/lib/wpe-webkit-2.0/WPEWebProcess').is_file():
    raise SystemExit('Verified package has no expected WPE runtime; leaving installed files unchanged')
old = stage / 'previous-usr'
if prefix.exists():
    prefix.rename(old)
try:
    (stage / 'usr').resolve().rename(prefix)
except BaseException:
    if old.exists():
        old.rename(prefix)
    raise
shutil.copyfile('native-runtime.json', '.deps/native-runtime-installed.json')
PY
printf '%s\n' 'Next: cargo run -p browser-demo'
