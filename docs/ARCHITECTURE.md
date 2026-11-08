# Architecture

The UI owns no media pipeline. A dedicated engine thread accepts bounded
commands and emits bounded state/statistics events. Recording state prevents
configuration changes while capture or finalization is active. Media work,
portal dialogs and remuxing run off the UI thread.

`config` defines sources, region validation, even output dimensions and settings.
`platform` enumerates sources and devices and adapts native capture/audio.
`preview` prepares a capture source, owns portal session lifetime and produces
one downscaled snapshot. The selected source can be reused for recording.
`media` constructs the GStreamer pipeline. `controller` manages its lifecycle.
`storage` manages journals, single-instance locking and no-clobber publication.

Video: native source → three-frame leaky queue → crop if needed → frame pacing
→ GPU conversion/scaling where supported → H.264 hardware encoder → parser
→ bounded non-leaky encoded queue → Matroska muxer → unbuffered file sink.
Dropping raw frames is safe; dropping encoded packets is not. The default
Windows Media Foundation path negotiates NV12 D3D11 memory. Linux VA capture
without cropping uses `vapostproc`/VA memory when available. Other paths use
bounded system-memory conversion and upload. Counters report captured/encoded
frames and queue-overrun notifications, not a promised exact dropped-frame count.

Audio: WASAPI or Pulse-compatible device source → conversion/resampling → gain
→ bounded queue → shared mixer → 48 kHz stereo → AAC → parser → muxer.
GStreamer uses one pipeline clock and audio clock slaving for synchronization.
Pausing adjusts pipeline base time across streams; it does not create a gap in
the output timeline. Hardware encoders may retain a small driver-owned buffer.

Each recording has a UUID journal written before capture. MKV clusters are
limited to approximately one second. After EOS/timeout the pipeline is closed;
FFmpeg performs a streaming remux into a temporary MP4. Successful output is
synced and published without overwriting an existing destination. Only then are
the source MKV and journal deleted. Failures preserve recovery inputs. Filesystem
and power-loss behavior can still affect the unwritten tail; there is no promise
that every crash is recoverable. Force-kill recovery is integration-tested.

Application settings use atomic temporary-file replacement. Journals live in
the platform's per-user application-data directory, allowing recordings in
custom output folders to be found after restart. FFmpeg stderr is written to a
temporary file and only its final 16 KiB is read into memory.
