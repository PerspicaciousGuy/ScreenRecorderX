# Validation and release gates

## Verified in this workspace

On Debian 13 x64 with GStreamer 1.26.2 and an Xvfb desktop:

- Seven configuration/recovery tests and four opt-in media integration tests passed.
- The media tests passed with the bundled minimal LGPL FFmpeg 7.1.1, including
  forced-process recovery, pause/resume and real X11 snapshot capture/reuse.
- A release-build recording was probed as 1920 × 1080 H.264 at 60/1 fps with AAC
  audio. Its paused interval was removed from the output timeline.
- Optimized Linux build, rustfmt and clippy passed. Windows GNU target compilation
  and clippy passed. Windows MSVC CI is configured but has not run here.
- The desktop interface was opened and its snapshot/crop interaction exercised.
  `docs/screenshot.png` is an actual application screenshot.

No GPU device is exposed in this workspace. Software encoding was explicitly
enabled only for synthetic diagnostics. Hardware throughput, Windows capture,
GNOME/KDE Wayland behavior, clean-machine installers and the matrix below remain
unverified. The development archive requires an installed GStreamer runtime;
it is not a self-contained AppImage.

## Automated checks

- Linux and Windows compilation, rustfmt, clippy and ordinary tests.
- Output scaling, odd dimensions, invalid region arithmetic and invalid gain.
- Settings replacement, instance locking, restart discovery and malformed journals.
- Recovery refuses existing destination files and retains nonempty sources.
- Opt-in media tests: actual H.264/AAC MP4 output, pause timeline, A/V duration,
  abrupt-process recovery and an actual X11 source snapshot.

The synthetic diagnostic explicitly permits software encoding so integration
tests can run on GPU-less CI. It does not demonstrate hardware capture speed.

## Physical hardware matrix (required before release claims)

Test Windows 10/11 and Ubuntu/Fedora on GNOME/KDE Wayland and X11. Include Intel
UHD 620-class, AMD VAAPI and NVIDIA NVENC devices. Check both hardware encode
support and actual GPU memory negotiation; installed factories alone are not
proof of a functional encoder.

Targets on an otherwise lightly loaded i5-8250U/8 GB machine:

- Thirty-minute high-motion 1080p60 capture, <1% missed frame deadlines.
- <50 ms A/V drift, including after pause/resume and with both audio inputs.
- ≤150 MiB steady-state process-private memory; measure GPU/shared allocations
  separately. RSS includes shared libraries and is not equivalent to private bytes.
- <10% average total CPU usage. Record game FPS with/without recording separately.

Measure the native desktop app and all child processes, not just the Rust engine.
Do not infer these targets from synthetic encoding or a virtual display.

Exercise monitor disconnection, display resizing, mixed DPI, multi-GPU laptops,
exclusive fullscreen, portal denial/cancellation, unsupported cursor modes,
missing shortcuts portal, microphone removal, source termination, GPU reset,
low disk, failed MP4 conversion, existing output files, app termination and a
controlled power interruption. Check error messages and retained recovery inputs.

AppImage and Windows installer testing must use clean machines without a
development toolchain. Confirm bundled runtime discovery and licensing/source
materials, and verify no system media-library paths leak into the package.
