#!/usr/bin/env bash
# Source this file when using a locally extracted WPE package under .deps/.
WPE_DEMO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -d "$WPE_DEMO_ROOT/.deps/usr/lib/pkgconfig" ]]; then
    export PKG_CONFIG_PATH="$WPE_DEMO_ROOT/.deps/usr/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    export LD_LIBRARY_PATH="$WPE_DEMO_ROOT/.deps/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
