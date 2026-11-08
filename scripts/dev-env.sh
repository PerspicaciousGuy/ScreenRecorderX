#!/usr/bin/env sh
# Optional local development dependencies; ordinary system installs need none.
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
clearcast_dev_root="${CLEARCAST_DEV_ROOT:-$PWD}"
if [ -d "$clearcast_dev_root/.local/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig" ]; then
    export PKG_CONFIG_PATH="$clearcast_dev_root/.local/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    export LIBRARY_PATH="$clearcast_dev_root/.local/sysroot/usr/lib/x86_64-linux-gnu${LIBRARY_PATH:+:$LIBRARY_PATH}"
    export LD_LIBRARY_PATH="$clearcast_dev_root/.local/sysroot/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export GST_PLUGIN_PATH="$clearcast_dev_root/.local/sysroot/usr/lib/x86_64-linux-gnu/gstreamer-1.0${GST_PLUGIN_PATH:+:$GST_PLUGIN_PATH}"
    export PATH="$clearcast_dev_root/.local/sysroot/usr/bin:$PATH"
fi
