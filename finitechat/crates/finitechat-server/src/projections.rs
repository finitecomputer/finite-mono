//! Room membership projection and delivery member-id helpers.

use std::collections::{BTreeMap, BTreeSet};

use finitechat_delivery::HttpSequence;
use finitechat_proto::{
    DeviceMembership, DeviceRef, MAX_ACCOUNT_DEVICES_PER_ROOM, MembershipAddV1, MembershipDeltaV1,
    MembershipInterval, RoomProtocol, RoomStatus, WelcomeRecord, delivery_member_id_for_device,
};
use finitechat_transport::transport::TransportMessage;
use finitechat_transport::{GroupId, MemberId};
use serde::{Deserialize, Serialize};

use crate::ServerHttpError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HttpRoomMembershipProjection {
    pub(crate) room_id: String,
    pub(crate) mls_group_id: String,
    pub(crate) current_epoch: u64,
    pub(crate) last_seq: HttpSequence,
    pub(crate) status: RoomStatus,
    #[serde(default = "default_membership_complete")]
    pub(crate) membership_complete: bool,
    /// Accounts allowed to change membership for other accounts (ADR 0003 §2
    /// as amended by ADR 0004 §4). Creator-initialized at typed bootstrap.
    #[serde(default)]
    pub(crate) admins: BTreeSet<String>,
    /// Accounts that left (ADR 0003 §3) and still await the MLS removal
    /// commit. The server already filters their delivery; this marker lets
    /// member workers discover the pending cryptographic cleanup.
    #[serde(default)]
    pub(crate) departed: BTreeSet<String>,
    /// Per-room protocol slots (ADR 0003 §1).
    #[serde(default)]
    protocol: RoomProtocol,
    #[serde(default)]
    pub(crate) membership: BTreeMap<String, DeviceMembership>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObservedRoomHead {
    pub(crate) current_epoch: u64,
    pub(crate) last_seq: HttpSequence,
    pub(crate) raw_commit_without_projection: bool,
}

impl HttpRoomMembershipProjection {
    pub(crate) fn device_for_member_id(&self, member_id: &MemberId) -> Option<&DeviceRef> {
        self.membership
            .values()
            .map(|membership| &membership.device)
            .find(|device| {
                member_id_for_device(device)
                    .ok()
                    .is_some_and(|candidate| candidate == *member_id)
            })
    }

    pub(crate) fn tracks_device(&self, device: &DeviceRef) -> bool {
        self.membership.contains_key(&DeviceMembership::key(device))
    }

    pub(crate) fn device_active_at_head(&self, device: &DeviceRef) -> bool {
        self.membership
            .get(&DeviceMembership::key(device))
            .map(|membership| {
                membership.intervals.iter().any(|interval| {
                    interval.active
                        && interval.start_seq <= self.last_seq
                        && interval.end_seq.is_none()
                })
            })
            .unwrap_or(false)
    }

    pub(crate) fn device_was_member_for_seq(&self, device: &DeviceRef, seq: HttpSequence) -> bool {
        self.membership
            .get(&DeviceMembership::key(device))
            .map(|membership| {
                membership.intervals.iter().any(|interval| {
                    interval.start_seq <= seq && interval.end_seq.is_none_or(|end| seq <= end)
                })
            })
            .unwrap_or(false)
    }

    pub(crate) fn current_or_pending_device_count_for_account(&self, account_id: &str) -> usize {
        self.membership
            .values()
            .filter(|membership| membership.device.account_id == account_id)
            .filter(|membership| {
                membership
                    .intervals
                    .iter()
                    .any(|interval| interval.end_seq.is_none())
            })
            .count()
    }

    fn device_current_or_pending_at_head(&self, device: &DeviceRef) -> bool {
        self.membership
            .get(&DeviceMembership::key(device))
            .map(|membership| {
                membership
                    .intervals
                    .iter()
                    .any(|interval| interval.end_seq.is_none())
            })
            .unwrap_or(false)
    }

    pub(crate) fn activate_interval(
        &mut self,
        device: &DeviceRef,
        start_seq: HttpSequence,
    ) -> bool {
        let Some(membership) = self.membership.get_mut(&DeviceMembership::key(device)) else {
            return false;
        };
        let Some(interval) = membership
            .intervals
            .iter_mut()
            .find(|interval| interval.start_seq == start_seq && !interval.active)
        else {
            return false;
        };
        interval.active = true;
        true
    }
}

pub(crate) fn validate_membership_adds_for_projection(
    projection: &HttpRoomMembershipProjection,
    adds: &[MembershipAddV1],
) -> Result<(), ServerHttpError> {
    let mut added_devices_by_account = BTreeMap::<String, usize>::new();
    for add in adds {
        let current_devices =
            projection.current_or_pending_device_count_for_account(&add.device.account_id);
        let added_devices = added_devices_by_account
            .entry(add.device.account_id.clone())
            .or_insert(0);
        *added_devices += 1;
        let proposed = current_devices + *added_devices;
        if proposed > MAX_ACCOUNT_DEVICES_PER_ROOM as usize {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: format!(
                    "room.devices_per_account has {proposed} items, max {MAX_ACCOUNT_DEVICES_PER_ROOM}"
                ),
            });
        }
        if projection.device_current_or_pending_at_head(&add.device) {
            return Err(ServerHttpError::InvalidCommitRequest {
                reason: format!(
                    "device {:?} is already current or pending in room",
                    add.device
                ),
            });
        }
    }
    Ok(())
}

pub(crate) fn apply_room_membership_delta(
    rooms: &mut BTreeMap<String, HttpRoomMembershipProjection>,
    room_id: &str,
    mls_group_id: &str,
    sender: &DeviceRef,
    expected_epoch: u64,
    membership_delta: &MembershipDeltaV1,
    accepted_seq: HttpSequence,
) -> Result<HttpRoomMembershipProjection, ServerHttpError> {
    let projection = rooms.entry(room_id.to_owned()).or_insert_with(|| {
        initial_room_membership_projection(
            room_id,
            mls_group_id,
            sender,
            expected_epoch,
            0,
            expected_epoch == 0,
            RoomProtocol::default(),
        )
    });
    if projection.room_id != room_id || projection.mls_group_id != mls_group_id {
        return Err(ServerHttpError::RoomMembershipConflict {
            room_id: room_id.to_owned(),
            reason: "membership delta targets a different room or MLS group".to_owned(),
        });
    }
    if projection.current_epoch != expected_epoch {
        return Err(ServerHttpError::RoomMembershipConflict {
            room_id: room_id.to_owned(),
            reason: format!(
                "membership delta expected epoch {expected_epoch}, projection is at {}",
                projection.current_epoch
            ),
        });
    }

    validate_membership_adds_for_projection(projection, &membership_delta.adds)?;

    for remove in &membership_delta.removes {
        if let Some(membership) = projection
            .membership
            .get_mut(&DeviceMembership::key(&remove.device))
            && let Some(interval) = membership
                .intervals
                .iter_mut()
                .rev()
                .find(|interval| interval.active && interval.end_seq.is_none())
        {
            interval.end_seq = Some(accepted_seq);
        }
        // The MLS removal commit for a departed account completes the leave.
        projection.departed.remove(&remove.device.account_id);
    }
    for add in &membership_delta.adds {
        projection
            .membership
            .entry(DeviceMembership::key(&add.device))
            .or_insert_with(|| DeviceMembership {
                device: add.device.clone(),
                intervals: Vec::new(),
            })
            .intervals
            .push(MembershipInterval {
                start_seq: accepted_seq,
                end_seq: None,
                active: false,
            });
    }
    projection.current_epoch = membership_delta.post_commit_epoch;
    projection.last_seq = accepted_seq;
    Ok(projection.clone())
}

pub(crate) fn member_id_for_device(device: &DeviceRef) -> Result<MemberId, ServerHttpError> {
    Ok(MemberId::new(delivery_member_id_for_device(device)))
}

pub(crate) fn ensure_device_not_revoked_in(
    revoked_devices: &BTreeSet<String>,
    device: &DeviceRef,
) -> Result<(), ServerHttpError> {
    if revoked_devices.contains(&DeviceMembership::key(device)) {
        Err(ServerHttpError::DeviceRevoked {
            device: device.clone(),
        })
    } else {
        Ok(())
    }
}

pub(crate) fn ensure_welcome_message_recipient_not_revoked(
    revoked_devices: &BTreeSet<String>,
    message: &TransportMessage,
) -> Result<(), ServerHttpError> {
    let Ok(welcome) = serde_json::from_slice::<WelcomeRecord>(&message.payload) else {
        return Ok(());
    };
    ensure_device_not_revoked_in(revoked_devices, &welcome.recipient)
}

pub(crate) fn group_id_for_room(room_id: &str) -> GroupId {
    GroupId::new(room_id.as_bytes().to_vec())
}

pub(crate) fn room_id_for_group_id(group_id: &GroupId) -> Result<String, ServerHttpError> {
    String::from_utf8(group_id.as_slice().to_vec()).map_err(|error| {
        ServerHttpError::InvalidGroupSyncRequest {
            reason: format!("group_id must be a UTF-8 Finite room_id: {error}"),
        }
    })
}

pub(crate) fn transport_group_id_for_room(room_id: &str) -> Vec<u8> {
    room_id.as_bytes().to_vec()
}

pub(crate) fn initial_room_membership_projection(
    room_id: &str,
    mls_group_id: &str,
    creator: &DeviceRef,
    current_epoch: u64,
    last_seq: HttpSequence,
    membership_complete: bool,
    protocol: RoomProtocol,
) -> HttpRoomMembershipProjection {
    let mut membership = BTreeMap::new();
    membership.insert(
        DeviceMembership::key(creator),
        DeviceMembership {
            device: creator.clone(),
            intervals: vec![MembershipInterval {
                start_seq: 0,
                end_seq: None,
                active: true,
            }],
        },
    );
    HttpRoomMembershipProjection {
        room_id: room_id.to_owned(),
        mls_group_id: mls_group_id.to_owned(),
        current_epoch,
        last_seq,
        status: RoomStatus::Open,
        membership_complete,
        admins: BTreeSet::from([creator.account_id.clone()]),
        departed: BTreeSet::new(),
        protocol,
        membership,
    }
}

fn default_membership_complete() -> bool {
    true
}
