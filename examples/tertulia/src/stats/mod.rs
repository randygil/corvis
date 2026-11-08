use std::time::Duration;

use corvis::prelude::*;
use serde::Serialize;

use crate::chat::ChatGateway;
use crate::rooms::RoomsService;

/// Live numbers, as JSON and as a Server-Sent Events stream (one-way, plain HTTP: a good
/// fit for dashboards that only listen).
#[controller("stats")]
pub struct StatsController {
    chat: Arc<ChatGateway>,
    rooms: Arc<RoomsService>,
    config: Arc<ConfigService>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    connections: usize,
    online_users: Vec<String>,
    messages: i64,
    rooms: Vec<RoomStats>,
}

#[derive(Debug, Serialize)]
struct RoomStats {
    name: String,
    members: usize,
}

async fn snapshot(chat: &ChatGateway, rooms: &RoomsService) -> Snapshot {
    let list = rooms.list().await.unwrap_or_default();
    Snapshot {
        connections: chat.connections(),
        online_users: chat.online_users(),
        messages: rooms.message_count().await.unwrap_or(0),
        rooms: list.into_iter().map(|r| RoomStats { members: chat.members(&r.name).len(), name: r.name }).collect(),
    }
}

#[routes]
impl StatsController {
    #[get]
    async fn get(&self) -> Snapshot {
        snapshot(&self.chat, &self.rooms).await
    }

    /// `GET /api/stats/stream`: a `stats` event right away, then every `STATS_INTERVAL_MS`.
    #[sse("stream")]
    fn stream(&self) -> Sse {
        let every = Duration::from_millis(self.config.get_or("STATS_INTERVAL_MS", 2000u64));
        let state = (self.chat.clone(), self.rooms.clone(), 0u64);
        let events = corvis::futures::stream::unfold(state, move |(chat, rooms, n)| async move {
            if n > 0 {
                corvis::tokio::time::sleep(every).await;
            }
            let data = snapshot(&chat, &rooms).await;
            Some((MessageEvent::new(json!(data)).event("stats").id(n.to_string()), (chat, rooms, n + 1)))
        });
        Sse::new(events)
    }
}
