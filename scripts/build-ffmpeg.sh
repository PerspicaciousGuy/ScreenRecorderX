#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=7.1.1
sha256=733984395e0dbbe5c046abda2dc49a5544e7e0e1e2366bba849222ae9e3a03b1
vendor="$PWD/.local/vendor"
target="${CLEARCAST_FFMPEG_TARGET:-native}"
configure_target=()
executable=ffmpeg
stage="$PWD/dist/ffmpeg"
if [ "$target" = windows ]; then
    stage="$PWD/dist/ffmpeg-windows"
    executable=ffmpeg.exe
    configure_target=(--enable-cross-compile --target-os=mingw32 --arch=x86_64
        --cross-prefix=x86_64-w64-mingw32- --extra-ldflags=-static)
elif [ "$target" != native ]; then
    echo "Unsupported FFmpeg target: $target" >&2; exit 1
fi
mkdir -p "$vendor" "$stage/bin" "$stage/third-party/ffmpeg"
archive="$vendor/ffmpeg-$version.tar.xz"
if [ ! -f "$archive" ]; then
    curl --fail --location --retry 3 "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz" -o "$archive"
fi
printf '%s  %s\n' "$sha256" "$archive" | sha256sum --check --status
if [ ! -d "$vendor/ffmpeg-$version" ]; then tar -xJf "$archive" -C "$vendor"; fi
build="$vendor/ffmpeg-build-$target"
mkdir -p "$build"
cd "$build"
"$vendor/ffmpeg-$version/configure" "${configure_target[@]}" --prefix="$stage" --disable-autodetect --disable-everything \
    --disable-network --disable-doc --disable-debug --disable-x86asm \
    --enable-small --enable-ffmpeg --enable-protocol=file \
    --enable-demuxer=matroska,mov --enable-muxer=mp4 \
    --enable-parser=h264,aac,hevc --enable-decoder=h264,aac \
    --enable-bsf=aac_adtstoasc,extract_extradata \
    --enable-filter=anull,null > "$vendor/ffmpeg-configure.log"
make -j"${CLEARCAST_BUILD_JOBS:-2}" "$executable" > "$vendor/ffmpeg-make.log" 2>&1
install -m755 "$executable" "$stage/bin/$executable"
cp "$archive" "$vendor/ffmpeg-$version/COPYING.LGPLv2.1" "$vendor/ffmpeg-$version/LICENSE.md" "$stage/third-party/ffmpeg/"
cp ffbuild/config.mak "$stage/third-party/ffmpeg/"
if [ "$target" = native ]; then
    "$stage/bin/ffmpeg" -buildconf > "$stage/third-party/ffmpeg/buildconf.txt" 2>&1
fi
printf 'FFmpeg %s staged at %s\n' "$version" "$stage"
