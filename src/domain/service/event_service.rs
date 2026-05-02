use crate::domain::model::chat_session_event::ChatSessionEvent;
use tokio::sync::broadcast;

pub struct EventService {
    tx: broadcast::Sender<ChatSessionEvent>,
}

impl Default for EventService {
    fn default() -> Self {
        Self::new()
    }
}

impl EventService {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }

    pub fn publish(&self, event: ChatSessionEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ChatSessionEvent> {
        self.tx.subscribe()
    }
}
