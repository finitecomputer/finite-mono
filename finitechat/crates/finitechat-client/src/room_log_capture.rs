//! Operator capture of one room's server-side log into the
//! [`CapturedRoomLogFile`] format consumed by
//! `finitechat diagnose rejected-entry`.
//!
//! Unlike the `rejected_entry_diagnostic` module (local copies only, never a
//! server), this module DOES talk to a running finitechat server: it pages
//! `/sync/group` through the production [`RuntimeDelivery`] client path,
//! exactly as a device sync would, and records the wire-format
//! [`RoomLogEntry`] values untouched. The capture is read-only: it never
//! submits commits, never appends events, and never advances any cursor.

use std::fmt;

use finitechat_proto::{DeviceRef, RoomLogEntry};

use crate::RuntimeDelivery;
use crate::rejected_entry_diagnostic::{CapturedRoomLog, CapturedRoomLogFile};

/// Default upper bound on sync pages pulled for one capture. Matches the
/// bounded replay limit of the rejected-entry diagnostic
/// (`REPLAY_MAX_SYNC_PAGES_PER_ROOM`), so a default capture always fits the
/// diagnostic that consumes it.
pub const DEFAULT_MAX_CAPTURE_PAGES: u32 = 64;

/// Everything one capture run needs. The `requester` device identity is sent
/// to the server exactly like a device's sync request sends it; the server
/// scopes the returned entries to that device's room membership.
#[derive(Debug)]
pub struct RoomLogCaptureRequest {
    pub room_id: String,
    pub requester: DeviceRef,
    /// First cursor to pull from; `0` captures the whole visible log.
    pub after_seq: u64,
    pub max_pages: u32,
}

#[derive(Debug)]
pub enum RoomLogCaptureError<E> {
    Delivery(E),
    /// A page carried an entry for another room, or sequence numbers that
    /// were not strictly increasing past the requested cursor.
    InvalidPage,
    /// The server kept reporting `has_more` without advancing the cursor.
    PaginationStalled {
        after_seq: u64,
    },
    /// The log did not fit the page bound; rerun with `--after-seq` set to
    /// the last captured sequence (or a higher `--max-pages`).
    PageBoundExceeded {
        max_pages: u32,
    },
}

impl<E: fmt::Display> fmt::Display for RoomLogCaptureError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Delivery(error) => write!(formatter, "room log sync failed: {error}"),
            Self::InvalidPage => write!(
                formatter,
                "server returned an out-of-order or cross-room sync page"
            ),
            Self::PaginationStalled { after_seq } => write!(
                formatter,
                "server pagination stalled at after_seq {after_seq}"
            ),
            Self::PageBoundExceeded { max_pages } => write!(
                formatter,
                "room log exceeds the capture bound of {max_pages} pages"
            ),
        }
    }
}

impl<E: fmt::Display + fmt::Debug> std::error::Error for RoomLogCaptureError<E> {}

/// Page the room log from `request.after_seq` until the server reports the
/// log is complete, and return it as a [`CapturedRoomLogFile`] with the room
/// as both the target and the only captured room.
pub fn capture_room_log<D: RuntimeDelivery>(
    delivery: &mut D,
    request: &RoomLogCaptureRequest,
) -> Result<CapturedRoomLogFile, RoomLogCaptureError<D::Error>>
where
    D::Error: fmt::Display,
{
    let mut entries: Vec<RoomLogEntry> = Vec::new();
    let mut after_seq = request.after_seq;
    let mut pages = 0u32;
    loop {
        let page = delivery
            .sync_events(&request.room_id, &request.requester, after_seq)
            .map_err(RoomLogCaptureError::Delivery)?;
        pages = pages.saturating_add(1);
        if pages > request.max_pages {
            return Err(RoomLogCaptureError::PageBoundExceeded {
                max_pages: request.max_pages,
            });
        }
        let mut previous = after_seq;
        for entry in &page.entries {
            if entry.room_id != request.room_id || entry.seq <= previous {
                return Err(RoomLogCaptureError::InvalidPage);
            }
            previous = entry.seq;
        }
        entries.extend(page.entries);
        if !page.has_more {
            break;
        }
        if page.next_after_seq <= after_seq {
            return Err(RoomLogCaptureError::PaginationStalled { after_seq });
        }
        after_seq = page.next_after_seq;
    }
    Ok(CapturedRoomLogFile {
        target_room_id: request.room_id.clone(),
        rooms: vec![CapturedRoomLog {
            room_id: request.room_id.clone(),
            entries,
        }],
    })
}
