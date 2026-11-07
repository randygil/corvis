//! Additional official-like modules (events, scheduling, cache, static files, throttling, health).

pub mod cache;
pub mod event_emitter;
pub mod health;
pub mod schedule;
pub mod serve_static;
pub mod throttler;

use std::sync::Arc;

use crate::di::Token;
use crate::di::container::Container;
use crate::di::provider::downcast;
use crate::discovery::{Discovered, HandlerKind};
use crate::logger::Logger;

/// Wires discovered handlers into the event emitter and the scheduler.
pub(crate) fn register_handlers(container: &Arc<Container>, handlers: &[Discovered]) -> crate::Result<()> {
    let emitter = container
        .find_instance(&Token::of::<event_emitter::EventEmitter>())
        .and_then(|i| downcast::<event_emitter::EventEmitter>(&i));
    let scheduler = scheduler(container);
    let schedule_options = container
        .find_instance(&Token::of::<schedule::ScheduleModuleOptions>())
        .and_then(|i| downcast::<schedule::ScheduleModuleOptions>(&i))
        .map(|o| *o)
        .unwrap_or_default();
    for d in handlers {
        let h = &d.handler;
        let default_name = format!("{}.{}", h.class_name, h.method_name);
        match &h.kind {
            HandlerKind::Event { events, call } => match &emitter {
                Some(e) => {
                    for ev in events {
                        e.add_listener(ev, call.clone());
                    }
                }
                None => Logger::new("EventEmitter").warn(format!(
                    "{default_name} uses #[on_event] but EventEmitterModule::for_root() is not imported"
                )),
            },
            HandlerKind::Cron { cron_time, options, call } => match &scheduler {
                Some(s) if schedule_options.cron_jobs => s.add_cron(cron_time.clone(), options.clone(), call.clone())?,
                Some(_) => {}
                None => Logger::new("Scheduler").warn(format!("{default_name} uses #[cron] but ScheduleModule::for_root() is not imported")),
            },
            HandlerKind::Interval { millis, name, call } => match &scheduler {
                Some(s) if schedule_options.intervals => s.add_interval_handler(name.clone(), *millis, call.clone())?,
                Some(_) => {}
                None => Logger::new("Scheduler").warn(format!("{default_name} uses #[interval] but ScheduleModule::for_root() is not imported")),
            },
            HandlerKind::Timeout { millis, name, call } => match &scheduler {
                Some(s) if schedule_options.timeouts => s.add_timeout_handler(name.clone(), *millis, call.clone())?,
                Some(_) => {}
                None => Logger::new("Scheduler").warn(format!("{default_name} uses #[timeout] but ScheduleModule::for_root() is not imported")),
            },
            HandlerKind::WsMessage { .. } => {}
        }
    }
    Ok(())
}

fn scheduler(container: &Arc<Container>) -> Option<Arc<schedule::SchedulerRegistry>> {
    container
        .find_instance(&Token::of::<schedule::SchedulerRegistry>())
        .and_then(|i| downcast::<schedule::SchedulerRegistry>(&i))
}

/// Mounts the collected `#[timeout]`, `#[interval]` and `#[cron]` handlers (before `onApplicationBootstrap`).
pub(crate) fn mount_scheduler(container: &Arc<Container>) -> crate::Result<()> {
    match scheduler(container) {
        Some(s) => s.mount(),
        None => Ok(()),
    }
}

/// Clears every timer and cron job (before `beforeApplicationShutdown`).
pub(crate) fn stop_scheduler(container: &Arc<Container>) {
    if let Some(s) = scheduler(container) {
        s.shutdown();
    }
}
