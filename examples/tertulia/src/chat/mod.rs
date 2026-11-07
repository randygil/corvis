mod chat_gateway;
mod dto;
mod room_member_guard;
mod ws_logging_interceptor;

use corvis::prelude::*;

pub use chat_gateway::{CLOSE_UNAUTHORIZED, ChatGateway};
pub use dto::{RoomDto, SendMessageDto, TypingDto, room_name};
pub use room_member_guard::RoomMemberGuard;
pub use ws_logging_interceptor::WsLoggingInterceptor;

use crate::firehose::FirehoseService;
use crate::rooms::{RoomsController, RoomsService};
use crate::stats::StatsController;

#[module(
    controllers = [RoomsController, StatsController],
    providers = [ChatGateway, RoomsService, FirehoseService],
    exports = [ChatGateway, RoomsService],
)]
pub struct ChatModule;
