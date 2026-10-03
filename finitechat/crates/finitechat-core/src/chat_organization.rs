use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppChatPlacement {
    pub topic_id: String,
    pub position: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatAddress {
    pub topic_id: String,
    pub chat_id: String,
}

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub(super) fn valid_position(position: &str) -> bool {
    !position.is_empty()
        && !position.ends_with('0')
        && position.len() <= 512
        && position.bytes().all(|byte| DIGITS.contains(&byte))
}

fn identity_suffix(topic: &str, chat: &str) -> String {
    format!(
        "{:x}V",
        Sha256::digest(format!("{}:{topic}{chat}", topic.len()))
    )
}

// Positions are dense lexicographic fractions, with a source-identity suffix
// so independent moves into the same gap have a stable, unique order.
fn position_between(lower: &str, upper: Option<&str>, suffix: &str) -> Option<String> {
    if upper.is_some_and(|upper| lower >= upper) {
        return None;
    }
    let mut prefix = String::new();
    let mut upper = upper.map(str::as_bytes);
    for index in 0..(512 - suffix.len()) {
        let left = lower.as_bytes().get(index).copied().unwrap_or(b'0');
        let right = upper
            .and_then(|upper| upper.get(index))
            .copied()
            .unwrap_or(b'z');
        let a = DIGITS.iter().position(|byte| *byte == left)?;
        let b = DIGITS.iter().position(|byte| *byte == right)?;
        if b > a + 1 {
            prefix.push(DIGITS[(a + b) / 2] as char);
            prefix.push_str(suffix);
            return Some(prefix);
        }
        prefix.push(left as char);
        if a < b {
            upper = None;
        }
    }
    None
}

impl ChatProjectionState {
    pub(super) fn apply_chat_placement(&mut self, room: &str, seq: u64, value: ChatPlacementV1) {
        let key = (room.to_owned(), value.topic_id, value.chat_id);
        if self
            .chat_placements
            .get(&key)
            .is_none_or(|(previous, _)| seq > *previous)
        {
            self.chat_placements.insert(
                key,
                (
                    seq,
                    AppChatPlacement {
                        topic_id: value.destination_topic_id,
                        position: value.position,
                    },
                ),
            );
        }
    }

    pub(super) fn attach_chat_placements(&self, topics: &mut [AppTopicSummary]) {
        let available = topics
            .iter()
            .filter(|topic| !topic.archived)
            .map(|topic| (topic.room_id.clone(), topic.topic_id.clone()))
            .collect::<BTreeSet<_>>();
        for topic in topics {
            for chat in &mut topic.chats {
                let key = (
                    topic.room_id.clone(),
                    topic.topic_id.clone(),
                    chat.chat_id.clone(),
                );
                let mut placement = self
                    .chat_placements
                    .get(&key)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_else(|| AppChatPlacement {
                        topic_id: topic.topic_id.clone(),
                        // Stable across incoming messages: manual order must not jump.
                        position: format!(
                            "{:016x}V{}",
                            u64::MAX - chat.started_seq,
                            identity_suffix(&topic.topic_id, &chat.chat_id)
                        ),
                    });
                if !available.contains(&(topic.room_id.clone(), placement.topic_id.clone())) {
                    placement.topic_id = topic.topic_id.clone();
                }
                chat.placement = Some(placement);
            }
        }
    }
}

impl AppRuntimeState {
    pub(super) fn move_chat(
        &mut self,
        room_id: String,
        topic_id: String,
        chat_id: String,
        destination_topic_id: String,
        before: Option<ChatAddress>,
    ) -> Result<(), FiniteChatCoreError> {
        self.validate_chat_route(&room_id, &topic_id, &chat_id)?;
        let invalid = || FiniteChatCoreError::Client {
            reason: "The chats changed. Refresh and try moving the chat again.".to_owned(),
        };
        if !self.topic_exists(&room_id, &destination_topic_id) {
            return Err(invalid());
        }
        let source = ChatAddress {
            topic_id: topic_id.clone(),
            chat_id: chat_id.clone(),
        };
        let mut destination = Vec::new();
        for topic in self
            .app
            .topics
            .iter()
            .filter(|topic| topic.room_id == room_id && !topic.archived)
        {
            for chat in &topic.chats {
                let address = ChatAddress {
                    topic_id: topic.topic_id.clone(),
                    chat_id: chat.chat_id.clone(),
                };
                if address == source {
                    if chat.archived {
                        return Err(invalid());
                    }
                    continue;
                }
                let placement = chat.placement.as_ref().ok_or_else(invalid)?;
                if !chat.archived && placement.topic_id == destination_topic_id {
                    destination.push((address, placement.position.as_str()));
                }
            }
        }
        destination.sort_by(|a, b| a.1.cmp(b.1));
        let index = match before {
            Some(before) => destination
                .iter()
                .position(|(address, _)| address == &before)
                .ok_or_else(invalid)?,
            None => destination.len(),
        };
        let lower = index
            .checked_sub(1)
            .map(|index| destination[index].1)
            .unwrap_or("");
        let upper = destination.get(index).map(|(_, position)| *position);
        let position = position_between(lower, upper, &identity_suffix(&topic_id, &chat_id))
            .ok_or_else(|| FiniteChatCoreError::Client {
                reason:
                    "This position is too crowded. Move the chat to a different position first."
                        .to_owned(),
            })?;
        let placement = ChatPlacementV1 {
            topic_id: topic_id.clone(),
            chat_id: chat_id.clone(),
            destination_topic_id,
            position,
        };
        placement.validate_limits().map_err(client_error)?;
        let event = self.core.send_application_event_with_segment(
            &room_id,
            DurableAppEventKind::Namespaced {
                name: FINITECHAT_CHAT_PLACEMENT_EVENT_V1.to_owned(),
                policy: ApplicationDeliveryPolicy::NON_NOTIFYING,
            },
            Some(topic_id),
            Some(chat_id),
            &serde_json::to_vec(&placement).map_err(client_error)?,
            "chat-placement",
        )?;
        // Publish only the accepted event. A refused save leaves the old order intact.
        self.apply_projection_events(vec![event])?;
        self.app.status = "chat moved".to_owned();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_stay_between_neighbors_and_concurrent_moves_are_distinct() {
        for (lower, upper) in [
            ("", None),
            ("", Some("000V")),
            ("AV", Some("AV0V")),
            ("zV", None),
            ("AV", Some("BV")),
        ] {
            let a = position_between(lower, upper, &identity_suffix("topic", "a")).unwrap();
            let b = position_between(lower, upper, &identity_suffix("topic", "b")).unwrap();
            assert_ne!(a, b);
            for result in [&a, &b] {
                assert!(valid_position(result));
                assert!(lower < result.as_str());
                assert!(upper.is_none_or(|upper| result.as_str() < upper));
            }
        }
        let mut upper = "V".to_owned();
        for _ in 0..1000 {
            let next = position_between("", Some(&upper), "V").unwrap();
            assert!(next < upper);
            upper = next;
        }
        assert!(position_between("BV", Some("AV"), "V").is_none());
    }
}
