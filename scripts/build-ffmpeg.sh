#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=7.1.1
sha256=733984395e0dbbe5c046abda2dc49a5544e7e0e1e2366bba849222ae9e3a03b1
vendor="$PWD/.local/vendor"
stage="$PWD/dist/ffmpeg"
mkdir -p "$vendor" "$stage/bin" "$stage/third-party/ffmpeg"
archive="$vendor/ffmpeg-$version.tar.xz"
if [ ! -f "$archive" ]; then
    curl --fail --location --retry 3 "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz" -o "$archive"
fi
printf '%s  %s\n' "$sha256" "$archive" | sha256sum --check --status
if [ ! -d "$vendor/ffmpeg-$version" ]; then tar -xJf "$archive" -C "$vendor"; fi
cd "$vendor/ffmpeg-$version"
./configure --prefix="$stage" --disable-autodetect --disable-everything \
    --disable-network --disable-doc --disable-debug --disable-x86asm \
    --enable-small --enable-ffmpeg --enable-protocol=file \
    --enable-demuxer=matroska --enable-muxer=mp4 \
    --enable-parser=h264,aac,hevc --enable-decoder=h264,aac \
    --enable-bsf=aac_adtstoasc,extract_extradata \
    --enable-filter=anull,null > "$vendor/ffmpeg-configure.log"
make -j"${CLEARCAST_BUILD_JOBS:-2}" ffmpeg > "$vendor/ffmpeg-make.log" 2>&1
install -m755 ffmpeg "$stage/bin/ffmpeg"
cp "$archive" COPYING.LGPLv2.1 LICENSE.md "$stage/third-party/ffmpeg/"
cp ffbuild/config.mak "$stage/third-party/ffmpeg/"
"$stage/bin/ffmpeg" -buildconf > "$stage/third-party/ffmpeg/buildconf.txt" 2>&1
printf 'FFmpeg %s staged at %s\n' "$version" "$stage"
