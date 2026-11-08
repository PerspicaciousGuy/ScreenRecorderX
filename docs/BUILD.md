# Building and packaging

## Linux

Ubuntu 24.04 LTS or newer, x64:

```sh
sudo apt-get update
sudo apt-get install build-essential pkg-config libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev libxkbcommon-dev libwayland-dev \
  libegl1-mesa-dev libgl1-mesa-dev libx11-dev libxi-dev libxrandr-dev \
  libxcursor-dev libxinerama-dev libx11-xcb-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-libav gstreamer1.0-pipewire \
  pipewire pipewire-pulse xdg-desktop-portal ffmpeg
cargo build --release --locked
./target/release/clearcast doctor
```

Install your GNOME/KDE portal backend and the hardware driver for your GPU.
Do not run Clearcast as root. Intel/AMD require working VAAPI H.264 encode;
NVIDIA requires the proprietary driver and GStreamer's NVENC plugin. Some
distributions package hardware codecs separately. Check `gst-inspect-1.0
vah264enc`, `gst-inspect-1.0 vaapih264enc`, or `gst-inspect-1.0 nvh264enc`.

Fedora build packages: `gcc gcc-c++ pkgconf-pkg-config gstreamer1-devel
gstreamer1-plugins-base-devel libxkbcommon-devel wayland-devel mesa-libEGL-devel
mesa-libGL-devel libX11-devel libXi-devel libXrandr-devel libXcursor-devel
libXinerama-devel`. Runtime package names and H.264 availability depend on enabled
Fedora codec repositories. Install PipeWire, the desktop portal backend, GStreamer
base/good/bad plugins, hardware encode plugins and an AAC encoder. `clearcast
doctor` reports individual plugin availability.

The cloud workspace can source `scripts/dev-env.sh` to use user-local headers.
That script is only a development convenience; packaged apps never use it.

## Windows

Use Windows 10 22H2/11 x64, Rust's **MSVC** toolchain and Visual Studio C++ Build
Tools. Install the matching GStreamer 1.26.11 MSVC x64 runtime and development
MSIs with the complete feature set from:

https://gstreamer.freedesktop.org/data/pkg/windows/1.26.11/msvc/

Set `GSTREAMER_1_0_ROOT_MSVC_X86_64` to the installation root (usually
`C:\gstreamer\1.0\msvc_x86_64`). Add its `bin` directory to PATH, install
pkg-config if needed, and set `PKG_CONFIG_PATH` to its `lib\pkgconfig` directory.

```powershell
$env:Path = "$env:GSTREAMER_1_0_ROOT_MSVC_X86_64\bin;$env:Path"
$env:PKG_CONFIG_PATH = "$env:GSTREAMER_1_0_ROOT_MSVC_X86_64\lib\pkgconfig"
cargo build --release --locked
.\target\release\clearcast.exe doctor
```

Required elements: `d3d11screencapturesrc`, `d3d11convert`, `d3d11download`,
`mfh264enc` (or another listed hardware encoder), `wasapi2src`, `mfaacenc` or
another AAC encoder, `h264parse`, `aacparse`, `matroskamux`, and common
audio/video conversion elements. Windows hardware testing is still required.

Put FFmpeg beside `clearcast.exe`, in its `bin` subdirectory, or on PATH. Use
an LGPL build for redistributable packages. System FFmpeg installations used
for development may have different licensing.

## Packaging

Packaging is separate from compiling: do not distribute a random system FFmpeg
or GStreamer tree without its dependency licenses and corresponding sources.

`scripts/build-ffmpeg.sh` builds a pinned, small LGPL FFmpeg for **remuxing only**
and stages its source and license materials. It does not build software video
encoders. Run it before the platform packaging script, or supply an equivalently
audited FFmpeg distribution.

For Linux, use `scripts/package-linux.sh` on Ubuntu 24.04 with `linuxdeploy`,
its GStreamer plugin, and `appimagetool` installed. It produces an AppDir and
AppImage, includes the application and minimal FFmpeg, and delegates GStreamer
dependency collection to linuxdeploy. Review collected plugins and license/source
materials before release. System portal services, GPU drivers and glibc remain
host dependencies.

### Windows setup (bundled dependencies)

End users install one setup; GStreamer and FFmpeg do not need separate installs
or PATH changes. A portable ZIP contains the same application payload.

CI builds a minimal LGPL Windows FFmpeg from pinned source on Linux using MinGW,
then packages it alongside the Windows application and selected GStreamer plugins.
The `clearcast-windows-x64-setup` artifact contains both downloads. The installer
works offline and installs per-user without administrator privileges. GPU drivers
and Windows graphics/audio components remain OS dependencies.

To build it yourself, prepare the Windows SDK described above and install NSIS.
On Linux, build the Windows remuxer:

```sh
sudo apt-get install gcc-mingw-w64-x86-64 make
CLEARCAST_FFMPEG_TARGET=windows scripts/build-ffmpeg.sh
```

Copy the complete `dist/ffmpeg-windows` directory to the Windows build machine,
then run:

```powershell
./scripts/package-windows.ps1
# Optional: -GStreamerRoot C:/gstreamer/1.0/msvc_x86_64
# Optional: -FFmpegRoot D:/build/ffmpeg-windows
```

The script builds Rust, stages selected runtime plugins and their recursively
resolved DLL dependencies, includes GStreamer's license materials plus source/build
references, and includes FFmpeg's source archive, configuration and license.
It refuses an existing stage directory to prevent obsolete files leaking into
new packages; use a new `-Stage` path for another build. `-SkipBuild` reuses an
existing release binary; `-SkipInstaller` produces and tests only the payload.

The private runtime preserves GStreamer's relocatable `bin`, `lib` and `libexec`
layout. The Start menu shortcut launches `bin/clearcast.exe`. The payload is
tested with a fresh plugin cache and SDK/plugin paths removed from the environment:
an explicit synthetic recording must produce an H.264/AAC MP4 using bundled FFmpeg.
This does not demonstrate hardware capture performance or a clean-machine install.

CI also uploads unbundled development binaries. Public releases still require
review of third-party source/license materials for the actual collected DLLs,
Windows signing if desired, and the physical hardware validation matrix.
