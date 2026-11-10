//! Who may hold how much of the server-wide unread budget.
//!
//! Everything a connection keeps that came from the client (unread messages, the packets being reassembled, the
//! decoded packets their handlers still hold) is charged to its Engine.IO session's gauge, which is chained to the
//! server-wide one. Two rules decide whether a decoded packet may take its room, checked before a single value is
//! built:
//!
//! - **The guaranteed share.** A session may always hold up to the fair share of the server-wide budget
//!   (`max(16 KiB, max_total_unread_bytes / max_sessions)`, the number the Engine.IO layer already uses to decide who
//!   keeps reading), whatever others hold: an honest session with small packets is never starved.
//! - **The server-wide budget.** Anything beyond the share needs room in the server-wide budget (an atomic check, so
//!   concurrent connections cannot overshoot together).
//!
//! Both are also limited per client address, counting the handlers still running for sessions it already closed: an
//! address may hold a quarter of the budget (never less than one packet) plus the guaranteed shares of as many
//! sessions as it may have open. The count belongs to the address, not to the session, so reconnecting does not
//! reset it, and other addresses keep their shares and the rest of the budget.
//!
//! A packet that finds no room waits, exactly like an unread backlog: its connection takes no further message from
//! the engine and keeps serving its heartbeat.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::engine::{EngineSocket, Held};

/// The part of the server-wide budget one address may hold beyond its sessions' guaranteed shares: a quarter.
const ADDRESS_FRACTION: usize = 4;

/// What every address currently holds, keyed by the address key the Engine.IO layer counts sessions under.
pub(crate) struct AddressBudgets {
    /// What an address may hold beyond the guaranteed shares (the server-wide rule).
    quota: usize,
    /// What the guaranteed shares of the sessions an address may have open add to it.
    guarantee: usize,
    slots: Mutex<HashMap<String, Weak<AddressSlot>>>,
}

impl AddressBudgets {
    /// `total` is `max_total_unread_bytes`; `packet` is `max_decoded_bytes`, the largest reservation: an address
    /// can always hold at least one packet. `guarantee` is the fair share times `max_sessions_per_ip`.
    pub(crate) fn new(total: usize, packet: usize, guarantee: usize) -> Arc<AddressBudgets> {
        let quota = (total / ADDRESS_FRACTION).max(packet).min(total);
        Arc::new(AddressBudgets { quota, guarantee, slots: Mutex::new(HashMap::new()) })
    }

    /// The most one address may hold beyond the guaranteed shares.
    #[cfg(test)]
    pub(crate) fn quota(&self) -> usize {
        self.quota
    }

    /// The most one address may hold in all.
    #[cfg(test)]
    pub(crate) fn address_cap(&self) -> usize {
        self.quota + self.guarantee
    }

    /// The slot of an address, shared by every connection and reservation of it.
    pub(crate) fn slot(self: &Arc<Self>, key: &str) -> Arc<AddressSlot> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = slots.get(key).and_then(Weak::upgrade) {
            return slot;
        }
        let slot = Arc::new(AddressSlot {
            key: key.to_string(),
            held: AtomicUsize::new(0),
            quota: self.quota,
            cap: self.quota + self.guarantee,
            owner: Arc::downgrade(self),
        });
        slots.insert(key.to_string(), Arc::downgrade(&slot));
        slot
    }

    /// Addresses with a slot alive: connections open or reservations held.
    #[cfg(test)]
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// What one address holds, guaranteed shares included.
pub(crate) struct AddressSlot {
    key: String,
    held: AtomicUsize,
    quota: usize,
    cap: usize,
    owner: Weak<AddressBudgets>,
}

impl AddressSlot {
    /// Takes `size` if what the address holds then stays within `limit`.
    fn try_take(&self, size: usize, limit: usize) -> bool {
        let mut current = self.held.load(Ordering::SeqCst);
        loop {
            let Some(next) = current.checked_add(size).filter(|n| *n <= limit) else { return false };
            match self.held.compare_exchange_weak(current, next, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    fn give_back(&self, size: usize) {
        self.held.fetch_sub(size, Ordering::SeqCst);
    }

    /// What the address holds now.
    #[cfg(test)]
    pub(crate) fn held(&self) -> usize {
        self.held.load(Ordering::SeqCst)
    }
}

impl Drop for AddressSlot {
    fn drop(&mut self) {
        // Forget the address once nothing refers to it. A newer slot of the same address may have replaced this one
        // already: only a dead entry is removed.
        if let Some(owner) = self.owner.upgrade() {
            let mut slots = owner.slots.lock().unwrap_or_else(|e| e.into_inner());
            if slots.get(&self.key).is_some_and(|w| w.strong_count() == 0) {
                slots.remove(&self.key);
            }
        }
    }
}

/// Room taken in the budget: released, in the address quota first and then in the session and server gauges (which
/// wakes the connections that wait), when this is dropped.
pub(crate) struct Reservation {
    held: Held,
    /// What counts against the client address: all of `held`, except for the wire messages of a packet being
    /// reassembled, which only the session's own gauge counts.
    quota: Option<(Arc<AddressSlot>, usize)>,
}

impl Reservation {
    /// Wraps bytes that are charged elsewhere than the decoded-packet rules: a received message kept while its
    /// packet is reassembled.
    pub(crate) fn plain(held: Held) -> Reservation {
        Reservation { held, quota: None }
    }

    /// Tries to reserve `bytes` for a connection of `engine` at `slot`; `None` when there is no room now.
    pub(crate) fn try_new(engine: &EngineSocket, slot: &Arc<AddressSlot>, bytes: usize) -> Option<Reservation> {
        // The guaranteed share: the session stays within its fair share, the address within its cap.
        if slot.try_take(bytes, slot.cap) {
            if let Some(held) = engine.hold_within_share(bytes) {
                return Some(Reservation { held, quota: Some((slot.clone(), bytes)) });
            }
            slot.give_back(bytes);
        }
        // Beyond the share: room in the server-wide budget and in the address's quota.
        if !slot.try_take(bytes, slot.quota) {
            return None;
        }
        match engine.hold_within_total(bytes) {
            Some(held) => Some(Reservation { held, quota: Some((slot.clone(), bytes)) }),
            None => {
                slot.give_back(bytes);
                None
            }
        }
    }

    /// Lowers the reservation to `bytes` (never raises it), giving the rest back to the budget.
    pub(crate) fn shrink_to(&mut self, bytes: usize) {
        if let Some((slot, taken)) = self.quota.as_mut()
            && bytes < *taken
        {
            slot.give_back(*taken - bytes);
            *taken = bytes;
        }
        self.held.shrink_to(bytes);
    }

    /// The bytes reserved.
    pub(crate) fn bytes(&self) -> usize {
        self.held.bytes()
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        // The quota before the gauges: the gauge release wakes the waiters, which then find the quota free.
        if let Some((slot, taken)) = self.quota.take() {
            slot.give_back(taken);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quota_is_a_quarter_of_the_budget_but_at_least_one_packet() {
        assert_eq!(AddressBudgets::new(256 << 20, 32 << 20, 0).quota(), 64 << 20);
        assert_eq!(AddressBudgets::new(64 << 20, 32 << 20, 0).quota(), 32 << 20, "never less than one packet");
        assert_eq!(AddressBudgets::new(16 << 20, 32 << 20, 0).quota(), 16 << 20, "never more than the budget");
        assert_eq!(AddressBudgets::new(256 << 20, 32 << 20, 1 << 20).address_cap(), (64 << 20) + (1 << 20));
    }

    #[test]
    fn a_slot_is_shared_per_address_and_forgotten_when_unused() {
        let budgets = AddressBudgets::new(1 << 20, 1 << 10, 0);
        let a = budgets.slot("203.0.113.1");
        let again = budgets.slot("203.0.113.1");
        assert!(Arc::ptr_eq(&a, &again));
        let other = budgets.slot("203.0.113.2");
        assert!(!Arc::ptr_eq(&a, &other));
        assert_eq!(budgets.slot_count(), 2);
        drop((a, again));
        assert_eq!(budgets.slot_count(), 1);
        drop(other);
        assert_eq!(budgets.slot_count(), 0);
        // A new slot for an address whose old one is being dropped is not removed by it.
        let first = budgets.slot("203.0.113.3");
        drop(first);
        let second = budgets.slot("203.0.113.3");
        assert_eq!(budgets.slot_count(), 1);
        drop(second);
    }

    #[test]
    fn the_quota_is_taken_atomically_and_given_back() {
        let budgets = AddressBudgets::new(400, 100, 0);
        let slot = budgets.slot("203.0.113.1");
        assert!(slot.try_take(60, 100));
        assert!(!slot.try_take(41, 100));
        assert!(slot.try_take(40, 100));
        assert_eq!(slot.held(), 100);
        slot.give_back(100);
        assert_eq!(slot.held(), 0);
    }
}
