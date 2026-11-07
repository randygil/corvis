use std::time::Duration;

use corvis::prelude::*;

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

async fn snapshot(chat: &ChatGateway, rooms: &RoomsService) -> Value {
    let list = rooms.list().await.unwrap_or_default();
    json!({
        "connections": chat.connections(),
        "onlineUsers": chat.online_users(),
        "messages": rooms.message_count().await.unwrap_or(0),
        "rooms": list.iter().map(|r| json!({ "name": r.name, "members": chat.members(&r.name).len() })).collect::<Vec<_>>(),
    })
}

#[routes]
impl StatsController {
    #[get]
    async fn get(&self) -> Value {
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
            Some((MessageEvent::new(data).event("stats").id(n.to_string()), (chat, rooms, n + 1)))
        });
        Sse::new(events)
    }
}
