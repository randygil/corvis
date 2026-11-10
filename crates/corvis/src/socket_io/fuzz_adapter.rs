//! Property tests of the room maps of the in-memory adapter. `cargo test -p corvis --features socket-io --lib fuzz`
//! runs them (the name of the module selects them); `PROPTEST_CASES=20000` runs them harder.
//!
//! Two properties: the selection of a broadcast equals a naive model for random memberships and options, and after
//! any sequence of joins, leaves and removals the two maps mirror each other and hold no empty room and no entry of
//! a socket that was removed.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use proptest::prelude::*;

use super::adapter::{BroadcastOptions, Membership, Room};

const SOCKETS: usize = 8;
const ROOMS: usize = 6;

fn sid(n: usize) -> String {
    format!("socket-{n:013}")
}

fn room(n: usize) -> Room {
    // Rooms named like sockets exist too: every socket is in the room of its own id.
    if n < ROOMS { Arc::from(format!("room-{n}")) } else { Arc::from(sid(n - ROOMS)) }
}

#[derive(Debug, Clone)]
enum Op {
    Join(usize, Vec<usize>),
    Leave(usize, usize),
    Remove(usize),
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..SOCKETS, proptest::collection::vec(0..ROOMS + SOCKETS, 0..4)).prop_map(|(s, r)| Op::Join(s, r)),
        2 => (0..SOCKETS, 0..ROOMS + SOCKETS).prop_map(|(s, r)| Op::Leave(s, r)),
        1 => (0..SOCKETS).prop_map(Op::Remove),
    ]
}

fn naive(model: &[(usize, BTreeSet<usize>)], rooms: &BTreeSet<usize>, except: &BTreeSet<usize>) -> BTreeSet<String> {
    let excluded = |rooms_of: &BTreeSet<usize>| except.iter().any(|r| rooms_of.contains(r));
    model
        .iter()
        .filter(|(_, rooms_of)| !rooms_of.is_empty())
        .filter(|(_, rooms_of)| rooms.is_empty() || rooms.iter().any(|r| rooms_of.contains(r)))
        .filter(|(_, rooms_of)| !excluded(rooms_of))
        .map(|(s, _)| sid(*s))
        .collect()
}

proptest! {
    #[test]
    fn fuzz_selection_equals_the_naive_model(
        ops in proptest::collection::vec(arb_op(), 0..60),
        to in proptest::collection::btree_set(0..ROOMS + SOCKETS, 0..4),
        except in proptest::collection::btree_set(0..ROOMS + SOCKETS, 0..4),
    ) {
        let mut m = Membership::default();
        // The model: for each socket, the rooms it is in.
        let mut model: Vec<(usize, BTreeSet<usize>)> = (0..SOCKETS).map(|s| (s, BTreeSet::new())).collect();
        for op in ops {
            match op {
                Op::Join(s, rooms) => {
                    let list: Vec<Room> = rooms.iter().map(|r| room(*r)).collect();
                    m.add(&sid(s), &list, usize::MAX).unwrap();
                    model[s].1.extend(rooms);
                }
                Op::Leave(s, r) => {
                    m.remove(&sid(s), &room(r));
                    model[s].1.remove(&r);
                }
                Op::Remove(s) => {
                    m.remove_all(&sid(s));
                    model[s].1.clear();
                }
            }
        }
        let opts = BroadcastOptions::new().rooms(to.iter().map(|r| room(*r))).except(except.iter().map(|r| room(*r)));
        let selected = m.select(&opts);
        let unique: HashSet<&str> = selected.iter().map(|s| &**s).collect();
        prop_assert_eq!(unique.len(), selected.len(), "each socket once");
        let got: BTreeSet<String> = selected.iter().map(|s| s.to_string()).collect();
        prop_assert_eq!(got, naive(&model, &to, &except));
    }

    #[test]
    fn fuzz_the_two_maps_mirror_each_other_and_hold_nothing_empty(ops in proptest::collection::vec(arb_op(), 0..120)) {
        let mut m = Membership::default();
        let mut removed: HashSet<usize> = HashSet::new();
        for op in ops {
            match op {
                Op::Join(s, rooms) => {
                    let list: Vec<Room> = rooms.iter().map(|r| room(*r)).collect();
                    m.add(&sid(s), &list, usize::MAX).unwrap();
                    removed.remove(&s);
                }
                Op::Leave(s, r) => m.remove(&sid(s), &room(r)),
                Op::Remove(s) => {
                    m.remove_all(&sid(s));
                    removed.insert(s);
                }
            }
            prop_assert_eq!(m.check(), Ok(()));
        }
        for s in removed {
            prop_assert!(!m.sids.contains_key(sid(s).as_str()), "a removed socket leaves no entry");
        }
    }

    #[test]
    fn fuzz_the_room_cap_is_all_or_nothing(first in proptest::collection::vec(0..40usize, 0..10), second in proptest::collection::vec(0..40usize, 0..10), cap in 1usize..12) {
        let mut m = Membership::default();
        let a: Vec<Room> = first.iter().map(|r| room(*r)).collect();
        let b: Vec<Room> = second.iter().map(|r| room(*r)).collect();
        let _ = m.add("s", &a, cap);
        let before = m.sids.get("s").cloned().unwrap_or_default();
        match m.add("s", &b, cap) {
            Ok(()) => prop_assert!(m.sids.get("s").map_or(0, HashSet::len) <= cap),
            Err(_) => prop_assert_eq!(m.sids.get("s").cloned().unwrap_or_default(), before, "a refused join changes nothing"),
        }
        prop_assert_eq!(m.check(), Ok(()));
    }
}
