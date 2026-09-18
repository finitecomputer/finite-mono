use super::*;

/// A caller's transcript window. Reading it never changes the Device's saved
/// navigation or publishes an AppUpdate to another caller.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct AppView {
    pub room_id: Option<String>,
    pub topic_id: Option<String>,
    pub chat_id: Option<String>,
    pub limit: Option<u32>,
}

impl FiniteChatRuntime {
    pub fn state_for_view(&self, view: AppView) -> Result<AppState, FiniteChatCoreError> {
        let (response, receiver) = mpsc::sync_channel(1);
        self.command_tx
            .send(AppRuntimeCommand::ReadView { view, response })
            .map_err(|_| client_error("runtime actor is stopped"))?;
        receiver
            .recv()
            .map_err(|_| client_error("runtime actor stopped before reading view"))?
    }
}

impl AppRuntimeState {
    pub(super) fn state_for_view(&self, view: AppView) -> Result<AppState, FiniteChatCoreError> {
        let Some(room_id) = view.room_id else {
            if view.topic_id.is_some() || view.chat_id.is_some() || view.limit.is_some() {
                return Err(client_error("a transcript view requires room_id"));
            }
            return Ok(self.app.clone());
        };
        if self.room(&room_id).is_none() {
            return Err(client_error(
                "transcript room is not available on this Device",
            ));
        }
        if view.chat_id.is_some() && view.topic_id.is_none() {
            return Err(client_error("a chat view requires topic_id"));
        }
        let topic_id = view.topic_id.or_else(|| {
            self.topic_exists(&room_id, HOME_TOPIC_ID)
                .then(|| HOME_TOPIC_ID.to_owned())
        });
        if let Some(topic_id) = &topic_id
            && !self.topic_exists(&room_id, topic_id)
        {
            return Err(client_error(
                "transcript topic is not available in this room",
            ));
        }
        let chat_id = view.chat_id.or_else(|| {
            topic_id
                .as_deref()
                .and_then(|topic| self.default_chat_id_for_topic(&room_id, topic))
        });
        if let (Some(topic_id), Some(chat_id)) = (&topic_id, &chat_id)
            && !self.chat_exists(&room_id, topic_id, chat_id)
        {
            return Err(client_error(
                "transcript chat is not available in this topic",
            ));
        }
        let count = view.limit.map_or(DEFAULT_TRANSCRIPT_WINDOW, |limit| {
            (limit as usize).clamp(DEFAULT_TRANSCRIPT_WINDOW, MAX_APP_MESSAGES)
        });
        let mut state = self.app.clone();
        state.messages =
            self.transcript_messages(&room_id, topic_id.as_deref(), chat_id.as_deref(), count);
        let total = match (topic_id.as_deref(), chat_id.as_deref()) {
            (Some(topic), Some(chat)) => self
                .chat_projection
                .chat_message_count(&room_id, topic, chat),
            (Some(topic), None) => self.chat_projection.topic_message_count(&room_id, topic),
            _ => self.chat_projection.room_message_count(&room_id),
        };
        for room in &mut state.rooms {
            room.can_load_older = room.room_id == room_id && total > count;
        }
        state.selected_room_id = Some(room_id);
        state.selected_topic_id = topic_id;
        state.selected_chat_id = chat_id;
        // These optional native inspector surfaces describe the saved Device
        // selection, not this read-only transcript view.
        state.media_gallery = None;
        state.room_details = None;
        Ok(state)
    }

    pub(super) fn transcript_messages(
        &self,
        room_id: &str,
        topic_id: Option<&str>,
        chat_id: Option<&str>,
        count: usize,
    ) -> Vec<ChatMessage> {
        let mut messages = match (topic_id, chat_id) {
            (Some(topic), Some(chat)) => self
                .chat_projection
                .messages_for_chat_window(room_id, topic, chat, count),
            (Some(topic), None) => self
                .chat_projection
                .messages_for_topic_window(room_id, topic, count),
            _ => self
                .chat_projection
                .messages_for_room_window(room_id, count),
        };
        self.core.apply_attachment_cache_paths(
            self.chat_projection.attachment_blob_map(),
            &mut messages,
        );
        self.apply_attachment_download_progress(&mut messages);
        messages
    }
}
