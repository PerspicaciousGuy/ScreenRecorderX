use anyhow::{Context, Result};
use ashpd::desktop::{
    PersistMode, Session,
    screencast::{CursorMode, Screencast, SourceType},
};
use std::os::fd::{AsRawFd, OwnedFd};

/// Both the D-Bus session and the restricted PipeWire FD must outlive capture.
pub struct PortalCapture {
    pub fd: OwnedFd,
    pub node: u32,
    pub size: (u32, u32),
    _session: Session<'static, Screencast<'static>>,
}

impl PortalCapture {
    pub async fn close(&self) {
        let _ = self._session.close().await;
    }
    pub async fn select(cursor: bool, monitor_only: bool) -> Result<Self> {
        let portal = Screencast::new().await.context("Screen-sharing portal unavailable. Install xdg-desktop-portal and your desktop's portal backend")?;
        let session = portal.create_session().await?;
        let types = if monitor_only {
            SourceType::Monitor.into()
        } else {
            SourceType::Monitor | SourceType::Window
        };
        portal
            .select_sources(
                &session,
                if cursor {
                    CursorMode::Embedded
                } else {
                    CursorMode::Hidden
                },
                types,
                false,
                None,
                PersistMode::DoNot,
            )
            .await?
            .response()?;
        let response = portal
            .start(&session, None)
            .await?
            .response()
            .context("Screen selection cancelled or denied")?;
        let stream = response
            .streams()
            .first()
            .context("Portal returned no capture stream")?;
        // Geometry is optional and may be logical pixels. It only seeds the
        // snapshot negotiation; PreparedCapture measures physical stream caps.
        let size = stream
            .size()
            .filter(|(w, h)| *w >= 64 && *h >= 64)
            .unwrap_or((1920, 1080));
        let fd = portal.open_pipe_wire_remote(&session).await?;
        Ok(Self {
            fd,
            node: stream.pipe_wire_node_id(),
            size: (size.0 as u32, size.1 as u32),
            _session: session,
        })
    }

    pub fn source(&self) -> Result<gstreamer::Element> {
        use gstreamer::prelude::*;
        let source = super::element("pipewiresrc", "capture")?;
        source.set_property("fd", self.fd.as_raw_fd());
        source.set_property("path", self.node.to_string());
        source.set_property("do-timestamp", true);
        Ok(source)
    }
}
