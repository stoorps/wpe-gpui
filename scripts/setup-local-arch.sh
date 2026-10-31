#!/usr/bin/env bash
# Arch-only fallback when WPE cannot be installed system-wide. No sudo required.
set -euo pipefail
cd -- "$(dirname -- "$0")/.."
mkdir -p .deps/downloads
url="$(pacman -Sp --print-format '%l' extra/wpewebkit | head -n 1)"
[[ "$url" == https://* ]] || { echo "Expected an HTTPS Arch package URL" >&2; exit 1; }
package=".deps/downloads/${url##*/}"
curl --fail --location "$url" --output "$package"
curl --fail --location "$url.sig" --output "$package.sig"
pacman-key --verify "$package.sig" "$package"
tar -xf "$package" -C .deps
python3 - <<'PY'
from pathlib import Path
prefix = Path('.deps/usr').resolve()
for pc in (prefix / 'lib/pkgconfig').glob('*.pc'):
    pc.write_text(pc.read_text().replace('prefix=/usr', f'prefix={prefix}').replace('libdir=/usr/lib', f'libdir={prefix}/lib'))
PY
printf '%s\n' 'Next: cargo run -p browser-demo'
