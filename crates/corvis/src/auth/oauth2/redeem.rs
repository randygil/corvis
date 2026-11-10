//! Cost controls of the callback, local to one strategy instance: a `state` is presented to the token endpoint once,
//! and one client address cannot hold every slot of the in-flight cap.
//!
//! Neither is a security boundary across instances: a flow record kept in a cookie stays valid in the browser until it
//! expires, so another instance would accept the same `state` once more (the provider refuses a used code on its own).
//! What these do is keep a client that replays one flow, or many flows from one address, from spending the
//! application's quota at the provider and its in-flight slots.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest, Sha256};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
struct RedeemedInner {
    seen: HashSet<[u8; 32]>,
    /// Insertion order, which is also expiry order: every entry lives for the same time.
    order: VecDeque<([u8; 32], i64)>,
}

/// The `state` values that were presented to the token endpoint (or are being redeemed right now), kept for `keep`
/// seconds, at most `max` of them. Only states that verified against the flow record are added, so a callback with a
/// state that does not verify makes it grow by nothing.
pub(crate) struct Redeemed {
    inner: Mutex<RedeemedInner>,
    max: usize,
    keep: i64,
}

impl Redeemed {
    pub(crate) fn new(max: usize, keep_seconds: i64) -> Self {
        Redeemed { inner: Mutex::new(RedeemedInner::default()), max, keep: keep_seconds }
    }

    /// Records `state` as redeemed at `now` (seconds). `false` when it already was: the callback is a replay, or the
    /// same flow is being redeemed at this moment. O(1) amortized.
    pub(crate) fn claim(&self, state: &str, now: i64) -> bool {
        let key: [u8; 32] = Sha256::digest(state.as_bytes()).into();
        let mut inner = lock(&self.inner);
        while let Some(&(old, expires)) = inner.order.front() {
            if expires > now {
                break;
            }
            inner.order.pop_front();
            inner.seen.remove(&old);
        }
        if inner.seen.contains(&key) {
            return false;
        }
        while inner.order.len() >= self.max {
            match inner.order.pop_front() {
                Some((old, _)) => {
                    inner.seen.remove(&old);
                }
                None => break,
            }
        }
        inner.seen.insert(key);
        inner.order.push_back((key, now.saturating_add(self.keep)));
        true
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        lock(&self.inner).order.len()
    }
}

/// How many callbacks of one client address may wait on the provider at once.
pub(crate) struct ClientSlots {
    held: Mutex<HashMap<String, usize>>,
    per_client: usize,
}

impl ClientSlots {
    pub(crate) fn new(per_client: usize) -> Arc<Self> {
        Arc::new(ClientSlots { held: Mutex::new(HashMap::new()), per_client })
    }

    /// A slot for `key`, or `None` when that client already holds its share. The slot is returned when the guard drops;
    /// an address with no slot held has no entry, so the map is as large as the slots in use.
    pub(crate) fn acquire(self: &Arc<Self>, key: String) -> Option<ClientSlot> {
        let mut held = lock(&self.held);
        let count = held.entry(key.clone()).or_insert(0);
        if *count >= self.per_client {
            return None;
        }
        *count += 1;
        Some(ClientSlot { slots: self.clone(), key })
    }

    #[cfg(test)]
    pub(crate) fn addresses(&self) -> usize {
        lock(&self.held).len()
    }
}

pub(crate) struct ClientSlot {
    slots: Arc<ClientSlots>,
    key: String,
}

impl Drop for ClientSlot {
    fn drop(&mut self) {
        let mut held = lock(&self.slots.held);
        if let Some(count) = held.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                held.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_is_claimed_once_until_it_expires() {
        let r = Redeemed::new(100, 10);
        assert!(r.claim("a", 0));
        assert!(!r.claim("a", 0));
        assert!(!r.claim("a", 9));
        assert!(r.claim("b", 9));
        assert!(r.claim("a", 10), "an entry expires after the retention time");
        assert!(!r.claim("b", 18));
        assert!(r.claim("b", 19));
    }

    #[test]
    fn the_set_is_bounded_and_drops_the_oldest() {
        let r = Redeemed::new(3, 1000);
        for s in ["1", "2", "3", "4", "5"] {
            assert!(r.claim(s, 0));
            assert!(r.len() <= 3);
        }
        assert!(r.claim("1", 0), "the oldest entries were dropped");
        assert!(!r.claim("5", 0), "the newest entries stay");
    }

    #[test]
    fn a_client_holds_at_most_its_share_and_gives_it_back() {
        let slots = ClientSlots::new(2);
        let a = slots.acquire("1.1.1.1".into()).unwrap();
        let _b = slots.acquire("1.1.1.1".into()).unwrap();
        assert!(slots.acquire("1.1.1.1".into()).is_none());
        let _other = slots.acquire("2.2.2.2".into()).unwrap();
        drop(a);
        assert!(slots.acquire("1.1.1.1".into()).is_some());
        drop(_b);
        drop(_other);
        assert_eq!(slots.addresses(), 0, "released addresses leave no entry");
    }
}
