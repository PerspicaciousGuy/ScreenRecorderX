#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
command -v linuxdeploy >/dev/null
command -v appimagetool >/dev/null
command -v linuxdeploy-plugin-gstreamer >/dev/null
test -x dist/ffmpeg/bin/ffmpeg || { echo 'Run scripts/build-ffmpeg.sh first' >&2; exit 1; }
test -d dist/third-party/gstreamer || { echo 'Stage audited GStreamer licenses and corresponding source/build materials in dist/third-party/gstreamer' >&2; exit 1; }
if dist/ffmpeg/bin/ffmpeg -buildconf 2>&1 | grep -Eq -- '--enable-(gpl|nonfree)'; then
    echo 'Refusing GPL/nonfree FFmpeg in this package profile' >&2; exit 1
fi
cargo build --release --locked
appdir="$PWD/dist/Clearcast.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/clearcast"
install -m755 dist/ffmpeg/bin/ffmpeg "$appdir/usr/bin/ffmpeg"
cp -R dist/ffmpeg/third-party "$appdir/usr/share/clearcast/"
cp -R dist/third-party/gstreamer "$appdir/usr/share/clearcast/third-party/"
cp LICENSE THIRD_PARTY.md "$appdir/usr/share/clearcast/"
linuxdeploy --appdir "$appdir" --executable target/release/clearcast \
    --desktop-file packaging/clearcast.desktop --icon-file packaging/clearcast.svg --plugin gstreamer
# The plugin discovers the required GStreamer runtime. Keep this wrapper's
# directory-relative executable lookup when adding FFmpeg alongside the app.
ARCH=x86_64 appimagetool "$appdir" "$PWD/dist/Clearcast-x86_64.AppImage"
