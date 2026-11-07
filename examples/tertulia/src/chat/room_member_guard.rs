use corvis::prelude::*;
use corvis::{WsClient, WsException};

/// Only members of `data.room` may talk, type or clear it. Guards run before pipes, so the
/// guard reads the raw payload through `switch_to_ws().get_data()`.
#[injectable]
pub struct RoomMemberGuard;

#[async_trait]
impl CanActivate for RoomMemberGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let ws = ctx.switch_to_ws();
        let Some(client) = ws.get_client::<WsClient>() else { return Ok(false) };
        // Without a room the payload is invalid: the validation pipe reports it.
        let Some(room) = ws.get_data().and_then(|d| d.get("room")).and_then(Value::as_str) else { return Ok(true) };
        if client.in_room(room) {
            return Ok(true);
        }
        Err(WsException::object(json!({
            "status": "error",
            "code": "NOT_IN_ROOM",
            "message": format!("Join #{room} first"),
            "room": room,
        })))
    }
}
