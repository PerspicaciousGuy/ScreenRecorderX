#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=1.26.11
stage="$PWD/dist/ffmpeg-windows/third-party/gstreamer"
vendor="$PWD/.local/vendor"
mkdir -p "$stage" "$vendor"
archive="$stage/cerbero-$version.tar.gz"
curl --fail --location --retry 3 "https://codeload.github.com/GStreamer/cerbero/tar.gz/refs/tags/$version" -o "$archive"
printf '%s  %s\n' c385254bd01a975a48087f28a10e530ae8444ca4351b07831cda60ca8f9c858c "$archive" | sha256sum --check --status
tar -xzf "$archive" -C "$vendor"
cp -R "$vendor/cerbero-$version/data/licenses" "$stage/"
for component in gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad; do
    recipe="$vendor/cerbero-$version/recipes/$component-1.0.recipe"
    checksum=$(sed -n "s/.*tarball_checksum = '\([^']*\)'.*/\1/p" "$recipe" | head -n1)
    test "${#checksum}" = 64
    source="$stage/$component-$version.tar.xz"
    curl --fail --location --retry 3 "https://gstreamer.freedesktop.org/src/$component/$component-$version.tar.xz" -o "$source"
    printf '%s  %s\n' "$checksum" "$source" | sha256sum --check --status
done
