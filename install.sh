#!/bin/sh
# Install the latest glyphwave release as ~/.local/bin/glyphwave. No root.
#
#   curl -fsSL https://github.com/LuigiKraken/glyphwave/releases/latest/download/install.sh | sh
#
# GLYPHWAVE_BASE points it at another release, e.g.
# https://github.com/LuigiKraken/glyphwave/releases/download/v0.1.0
set -eu

base=${GLYPHWAVE_BASE:-https://github.com/LuigiKraken/glyphwave/releases/latest/download}
case $(uname -m) in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *)
        echo "glyphwave: no prebuilt binary for $(uname -m); build it with cargo instead" >&2
        exit 1
        ;;
esac
name=glyphwave-$arch-linux
dir=$HOME/.local/bin

mkdir -p "$dir"
tmp=$(mktemp "$dir/.glyphwave.XXXXXX")
trap 'rm -f "$tmp"' EXIT
curl -fsSL "$base/$name" -o "$tmp"
want=$(curl -fsSL "$base/$name.sha256" | cut -d' ' -f1)
if [ "$(sha256sum "$tmp" | cut -d' ' -f1)" != "$want" ]; then
    echo "glyphwave: checksum mismatch, not installed" >&2
    exit 1
fi
chmod 755 "$tmp"
mv "$tmp" "$dir/glyphwave"
echo "glyphwave: installed $dir/glyphwave"

case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "glyphwave: $dir isn't on your PATH; add it, or run $dir/glyphwave" ;;
esac
if ! command -v parec >/dev/null && ! command -v pw-record >/dev/null; then
    echo "glyphwave: for music visuals install parec (pulseaudio-utils) or pw-record (pipewire-utils)"
fi
