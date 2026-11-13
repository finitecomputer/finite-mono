//! How Core's errors reach a caller.

use super::*;

/// A start refused by FIN-463's hold is 423 with the words the old
/// dashboard shows, never the 409 of an overlap.
#[test]
fn a_held_start_is_locked_with_its_words() {
    let error = ApiError::from(CoreError::RuntimeMoving);
    assert_eq!(error.status, StatusCode::LOCKED);
    assert_eq!(error.message, "This agent is moving to finite.chat.");
    assert_eq!(error.correlation_id, None);
}
