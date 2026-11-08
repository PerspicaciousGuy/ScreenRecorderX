# Third-party distribution

Clearcast's application source is MIT licensed. Cargo.lock pins Rust dependencies;
their copyright and license terms remain in force. Generate a complete dependency
notice set for each release (for example with cargo-about) and retain it with the
installer/AppImage.

GStreamer and the selected core/native media plugins are generally LGPL licensed.
Optional plugins can depend on differently licensed libraries: do not assume an
entire installed plugin directory is suitable for the intended distribution.
Include license texts, copyright notices, corresponding source/build information,
and required relinking/replacement support for the actual bundled binaries.

The FFmpeg build script disables autodetection, networking, external encoders and
GPL/nonfree features. It stages the exact source archive, configuration and
LGPL license alongside its remux-only executable. Keep those materials with
distribution. Third-party binaries provided to packaging scripts require the
same review. Development builds may use a system GPL-enabled FFmpeg; those
system binaries are not automatically included in release packages.

GStreamer runtime source and license materials supplied by the release builder
belong under `third-party/gstreamer` in each staging directory. The packaging
scripts refuse a missing third-party directory rather than declaring an
unaudited runtime license-compliant.
