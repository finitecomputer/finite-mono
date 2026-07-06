//! Runs the shared Finite Chat delivery conformance suite against
//! finitechat's durable SQLite-backed HTTP server state.
//!
//! The delivery crate ships the `HttpDelivery` contract, an in-memory
//! reference implementation, and an executable conformance module. This test
//! adapts `HttpServerState` to that contract and proves it preserves every
//! checked invariant, including the restart-survival checks the in-memory
//! reference skips.

use std::path::PathBuf;

use finitechat_delivery::conformance::{self, HttpDeliveryHarness};
use finitechat_delivery::{
    HttpClaimedKeyPackage, HttpDelivery, HttpKeyPackagePublication, HttpPublishReceipt,
    HttpPublishTarget, HttpSequence, HttpServerError, HttpSyncPage,
};
use finitechat_http::{ClaimKeyPackageRequest, GroupSyncRequest, PublishMessageRequest};
use finitechat_server::{HttpServerState, ServerHttpError};
use finitechat_transport::transport::TransportMessage;
use finitechat_transport::{GroupId, MemberId};

/// The durable server state viewed through the shared delivery contract.
struct DurableDelivery {
    state: HttpServerState,
}

fn delivery_error<T>(result: Result<T, ServerHttpError>) -> Result<T, HttpServerError> {
    result.map_err(|error| match error {
        ServerHttpError::Delivery(inner) => inner,
        other => panic!(
            "durable server returned a non-delivery error for a raw contract operation: {other:?}"
        ),
    })
}

impl HttpDelivery for DurableDelivery {
    fn publish(
        &mut self,
        target: HttpPublishTarget,
        message: TransportMessage,
    ) -> Result<HttpPublishReceipt, HttpServerError> {
        delivery_error(self.state.publish_message(PublishMessageRequest {
            target,
            message,
            idempotency_key: None,
        }))
    }

    fn sync_group(
        &self,
        group_id: &GroupId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        delivery_error(self.state.sync_group(GroupSyncRequest {
            group_id: group_id.clone(),
            after_seq,
            limit,
            requester: None,
        }))
    }

    fn sync_inbox(
        &self,
        recipient: &MemberId,
        after_seq: HttpSequence,
        limit: usize,
    ) -> Result<HttpSyncPage, HttpServerError> {
        delivery_error(self.state.sync_inbox(recipient, after_seq, limit))
    }

    fn publish_key_package(
        &mut self,
        publication: HttpKeyPackagePublication,
    ) -> Result<(), HttpServerError> {
        delivery_error(self.state.publish_key_package(publication)).map(|_| ())
    }

    fn claim_key_package(
        &mut self,
        owner: &MemberId,
    ) -> Result<Option<HttpClaimedKeyPackage>, HttpServerError> {
        delivery_error(self.state.claim_key_package(ClaimKeyPackageRequest {
            owner: owner.clone(),
        }))
    }
}

struct DurableHarness {
    _dir: tempfile::TempDir,
    db_path: PathBuf,
    delivery: DurableDelivery,
}

impl DurableHarness {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir for conformance SQLite file");
        let db_path = dir.path().join("http-conformance.sqlite3");
        let delivery = DurableDelivery {
            state: HttpServerState::from_sqlite_path(&db_path)
                .expect("open SQLite-backed HTTP server state"),
        };
        Self {
            _dir: dir,
            db_path,
            delivery,
        }
    }
}

impl HttpDeliveryHarness for DurableHarness {
    type Delivery = DurableDelivery;

    fn delivery(&mut self) -> &mut DurableDelivery {
        &mut self.delivery
    }

    fn restart(&mut self) -> bool {
        self.delivery = DurableDelivery {
            state: HttpServerState::from_sqlite_path(&self.db_path)
                .expect("reopen SQLite-backed HTTP server state after restart"),
        };
        true
    }
}

#[test]
fn sqlite_backed_server_passes_upstream_delivery_conformance_suite() {
    conformance::check_all(&mut DurableHarness::new());
}

#[test]
fn sqlite_backed_server_passes_upstream_restart_conformance_alone() {
    conformance::check_state_survives_restart(&mut DurableHarness::new());
}
