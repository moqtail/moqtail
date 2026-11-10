// Copyright 2026 The MOQtail Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Remembers which Objects a track has already ingested, so the same Object served
//! by several publishers is forwarded once.
//!
//! # Shape of the problem
//!
//! Every Object on every track goes through here, so the structure has to satisfy
//! three things at once:
//!
//! - **No shared lock.** A single lock over all groups would put every Object of
//!   every publisher behind one mutex. Each group owns its own lock instead, and
//!   nothing above it is locked at all.
//! - **Bounded memory.** Retention costs `groups x objects-per-group`. The caller
//!   bounds the first factor (`dedup_retained_groups`); this module bounds the
//!   second, so a publisher cannot decide how much memory the relay spends by
//!   choosing a huge group.
//! - **Cheap for ordinary tracks.** A group of a few dozen Objects should cost tens
//!   of bytes and no allocation.
//!
//! # How
//!
//! A fixed ring of group slots, indexed `group % slots`, each behind its own lock.
//! Group ids advance, so that index keeps the newest N groups, which is the same
//! retention the caller asks for.
//!
//! That holds exactly while group ids advance one at a time. Groups whose ids differ
//! by a multiple of the ring length share a slot and evict each other while other
//! slots stand empty, so a publisher numbering its groups in jumps gets less
//! retention than it asked for -- `N / gcd(N, stride)` groups for a constant stride,
//! and roughly two thirds of N for ids that look random. The cost is a narrower
//! window in which duplicates are caught, never a dropped Object.
//!
//! Inside a slot, object ids are bits rather than keys: they are dense small
//! integers, the worst case for a hash set and the best case for a bitmap. The
//! bitmap starts inline ([`INLINE_BITS`] ids, no allocation), and a group that
//! outgrows it switches once to a heap bitmap capped at [`WINDOW_BITS`] ids. Past
//! that cap the bitmap stops growing and recycles its oldest bits for the newest
//! ids -- the same "keep the newest, forget the oldest" rule the group ring applies,
//! one level down.
//!
//! # What it can get wrong
//!
//! Both bounds fail in the same direction: forgetting. A duplicate whose original
//! has been forgotten is reported as new and gets forwarded. The reverse -- a new
//! Object mistaken for a duplicate and dropped -- cannot happen, because a bit is
//! only ever consulted while its slot still describes that exact group and that
//! exact id. Losing a duplicate costs a redundant Object downstream; losing an
//! Object would cost data, so the asymmetry is deliberate.
//!
//! Specifically, an Object is reported as new when:
//!
//! - its group has fallen out of the ring (more than `retained_groups` newer groups
//!   have arrived), or a newer group has taken over its slot;
//! - its id is more than [`WINDOW_BITS`] behind the highest id seen in its group.
//!
//! Each needs a publisher to be a long way behind the others serving the same
//! track. Within those bounds detection is exact, including for Objects arriving out
//! of order -- which they do, since a group's subgroups arrive on separate streams.

use moqtail::model::common::location::Location;
use std::fmt;
use std::sync::Mutex;

/// Ids covered by a group's inline bitmap, counted from object id 0. 256 bits is 32
/// bytes held directly in the slot, which covers a normal group whole, with no
/// allocation. A group numbering its Objects from something higher simply starts in
/// the heap window instead.
const INLINE_BITS: u64 = 256;
const INLINE_WORDS: usize = (INLINE_BITS / 64) as usize;

/// Ids covered once a group has outgrown its inline bitmap, and the cap on what one
/// group can cost: 4096 bits is 512 bytes on the heap. A group larger than this is
/// still tracked, but only its newest 4096 ids are remembered at any moment.
///
/// It bounds the reorder distance the module can see across publishers: two
/// publishers of the same track are caught as long as they are less than this many
/// Objects apart.
const WINDOW_BITS: u64 = 4096;
const WINDOW_WORDS: usize = (WINDOW_BITS / 64) as usize;

/// Object ids a track has already ingested, one lock per group.
pub(crate) struct SeenObjects {
  /// Ring of group slots, indexed `group % slots.len()`. Empty when detection is off.
  slots: Box<[Mutex<GroupSlot>]>,
}

impl SeenObjects {
  /// `retained_groups` is how many groups stay remembered; 0 turns detection off,
  /// and then nothing is allocated and no lock is ever taken.
  pub(crate) fn new(retained_groups: usize) -> Self {
    let slots = (0..retained_groups)
      .map(|_| Mutex::new(GroupSlot { state: None }))
      .collect();
    Self { slots }
  }

  /// Records `location` and reports whether it had already been seen.
  ///
  /// Recording is not a separate call on purpose: the test and the record are one
  /// operation under one lock, so two publishers delivering the same Object at the
  /// same moment cannot both be told it is new.
  pub(crate) fn is_duplicate(&self, location: &Location) -> bool {
    // Detection off: not even the index arithmetic is worth doing.
    if self.slots.is_empty() {
      return false;
    }

    let index = (location.group % self.slots.len() as u64) as usize;
    // A poisoned lock means some caller panicked mid-update. These bits are a hint,
    // never a correctness invariant, so recovering the slot beats spreading the panic
    // to every later Object on the track.
    let mut slot = self.slots[index]
      .lock()
      .unwrap_or_else(|poisoned| poisoned.into_inner());

    match &mut slot.state {
      // The slot already describes this group: the bitmap has the answer.
      Some(state) if state.group == location.group => state.bits.test_and_set(location.object),
      // A newer group claims the slot, evicting whatever it held. That eviction is
      // the retention bound: the group leaving is `retained_groups` behind this one.
      Some(state) if location.group > state.group => {
        *state = GroupState::new(location.group, location.object);
        false
      }
      // An older group hashing onto a live slot is already outside retention. Report
      // it as new and leave the newer group's bits intact.
      Some(_) => false,
      // First use of this slot.
      None => {
        slot.state = Some(GroupState::new(location.group, location.object));
        false
      }
    }
  }

  /// How many groups are remembered at once. 0 means detection is off.
  #[cfg(test)]
  pub(crate) fn retained_groups(&self) -> usize {
    self.slots.len()
  }
}

impl fmt::Debug for SeenObjects {
  /// Deliberately opaque: the contents are megabits of bookkeeping and dumping them
  /// would swamp any log line carrying a Track.
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("SeenObjects")
      .field("retained_groups", &self.slots.len())
      .finish()
  }
}

/// One ring position. Empty until a group first lands on it.
struct GroupSlot {
  state: Option<GroupState>,
}

/// The group a slot currently describes, and which of its Objects have been seen.
struct GroupState {
  group: u64,
  bits: GroupBits,
}

impl GroupState {
  /// Starts a group off with its first Object already recorded.
  fn new(group: u64, first_object: u64) -> Self {
    let mut bits = if first_object < INLINE_BITS {
      GroupBits::Inline {
        words: [0u64; INLINE_WORDS],
      }
    } else {
      // Numbered past the inline range from the very first Object, so there is
      // nothing to be gained by starting inline only to promote immediately.
      GroupBits::Window {
        high: first_object,
        words: Box::new([0u64; WINDOW_WORDS]),
      }
    };
    bits.test_and_set(first_object);
    Self { group, bits }
  }
}

/// Which Objects of one group have been seen, as bits.
///
/// A group begins `Inline` and switches to `Window` at most once, when an id lands
/// beyond the inline range. There is no path back: a group that grew stays grown
/// until its slot is reused.
enum GroupBits {
  /// Ids `0 ..= INLINE_BITS - 1`, one bit each, stored in the slot itself.
  Inline { words: [u64; INLINE_WORDS] },
  /// The newest [`WINDOW_BITS`] ids, `high + 1 - WINDOW_BITS ..= high`, as a circular
  /// bitmap indexed `id % WINDOW_BITS`. Advancing `high` evicts the oldest ids.
  Window {
    high: u64,
    words: Box<[u64; WINDOW_WORDS]>,
  },
}

impl GroupBits {
  /// Records `id` and reports whether its bit was already set.
  fn test_and_set(&mut self, id: u64) -> bool {
    match self {
      GroupBits::Inline { words } => {
        if id < INLINE_BITS {
          let seen = bit_is_set(words, id);
          set_bit(words, id);
          return seen;
        }
        // Outgrown the inline range. Switch to the capped window and answer from
        // there; the copy happens once per group, and recursion stops at one level
        // because `Window` never promotes.
        let words = *words;
        *self = Self::promote(words);
        self.test_and_set(id)
      }
      GroupBits::Window { high, words } => window_test_and_set(high, words.as_mut_slice(), id),
    }
  }

  /// Moves an inline bitmap into a window, preserving every id it had recorded.
  ///
  /// The window is wider than the inline range and both start at id 0, so every
  /// recorded id lands inside the window with no two colliding onto one circular
  /// position. `high` becomes the top of the inline range: nothing above it can have
  /// been recorded yet, since a higher id is exactly what triggered this.
  fn promote(words: [u64; INLINE_WORDS]) -> Self {
    let mut window = Box::new([0u64; WINDOW_WORDS]);
    for id in 0..INLINE_BITS {
      if bit_is_set(&words, id) {
        set_bit(window.as_mut_slice(), id % WINDOW_BITS);
      }
    }
    GroupBits::Window {
      high: INLINE_BITS - 1,
      words: window,
    }
  }
}

/// Records `id` in a circular window covering `high + 1 - WINDOW_BITS ..= high`, and
/// reports whether it was already there.
fn window_test_and_set(high: &mut u64, words: &mut [u64], id: u64) -> bool {
  if id > *high {
    // The window slides up. Ids `(high, id]` enter it, and the ids they displace sit
    // in exactly the same circular positions -- `x` and `x - WINDOW_BITS` share an
    // index -- so clearing those positions is the whole eviction step.
    if id - *high >= WINDOW_BITS {
      // The jump is wider than the window: nothing recorded survives it.
      words.fill(0);
    } else {
      for displaced in (*high + 1)..=id {
        clear_bit(words, displaced % WINDOW_BITS);
      }
    }
    set_bit(words, id % WINDOW_BITS);
    *high = id;
    return false;
  }

  // Older than the window still describes. Its bit has since been handed to a newer
  // id, so there is nothing trustworthy to read.
  if *high - id >= WINDOW_BITS {
    return false;
  }

  let index = id % WINDOW_BITS;
  let seen = bit_is_set(words, index);
  set_bit(words, index);
  seen
}

fn bit_is_set(words: &[u64], index: u64) -> bool {
  words[(index >> 6) as usize] & (1u64 << (index & 63)) != 0
}

fn set_bit(words: &mut [u64], index: u64) {
  words[(index >> 6) as usize] |= 1u64 << (index & 63);
}

fn clear_bit(words: &mut [u64], index: u64) {
  words[(index >> 6) as usize] &= !(1u64 << (index & 63));
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_repeated_location_is_a_duplicate() {
    let seen = SeenObjects::new(4);
    let loc = Location::new(4, 2);

    assert!(!seen.is_duplicate(&loc), "first sighting is new");
    assert!(seen.is_duplicate(&loc), "second is a duplicate");

    // A different object in the same group, and the same object id in another group,
    // are both distinct locations.
    assert!(!seen.is_duplicate(&Location::new(4, 3)));
    assert!(!seen.is_duplicate(&Location::new(5, 2)));
  }

  /// Retaining nothing keeps nothing, so every Object looks new. That is duplicate
  /// detection turned off, not unbounded retention.
  #[test]
  fn zero_retention_disables_detection() {
    let seen = SeenObjects::new(0);
    let loc = Location::new(1, 1);

    assert!(!seen.is_duplicate(&loc));
    assert!(!seen.is_duplicate(&loc));
    assert_eq!(seen.retained_groups(), 0);
  }

  #[test]
  fn groups_beyond_retention_are_forgotten() {
    let seen = SeenObjects::new(2);
    let early = Location::new(0, 0);
    assert!(!seen.is_duplicate(&early));

    // Two further groups push the first out of the ring.
    seen.is_duplicate(&Location::new(1, 0));
    seen.is_duplicate(&Location::new(2, 0));

    // The evicted group is no longer recognised. This is the documented limit of the
    // bound, not a defect: a publisher that far behind gets its duplicate through.
    assert!(!seen.is_duplicate(&early));
  }

  /// A group arriving after it has already been evicted must not throw out the newer
  /// group occupying its slot.
  #[test]
  fn a_stale_group_does_not_evict_a_live_one() {
    let seen = SeenObjects::new(2);
    seen.is_duplicate(&Location::new(2, 7));

    // Group 0 shares a slot with group 2 and is older, so it is reported as new and
    // changes nothing.
    assert!(!seen.is_duplicate(&Location::new(0, 7)));
    assert!(
      seen.is_duplicate(&Location::new(2, 7)),
      "the live group's bits survived"
    );
  }

  /// Objects are deduplicated whatever order they arrive in, as long as they stay
  /// inside the window.
  #[test]
  fn out_of_order_objects_are_deduplicated() {
    let seen = SeenObjects::new(4);
    assert!(!seen.is_duplicate(&Location::new(1, 9)));
    assert!(!seen.is_duplicate(&Location::new(1, 3)));
    assert!(seen.is_duplicate(&Location::new(1, 9)));
    assert!(seen.is_duplicate(&Location::new(1, 3)));
  }

  /// A group larger than the inline bitmap keeps working: the promotion to the heap
  /// window preserves what was already recorded.
  #[test]
  fn a_group_outgrowing_the_inline_bitmap_keeps_its_history() {
    let seen = SeenObjects::new(4);

    // Fill the inline range, then step past it to force promotion.
    for object in 0..INLINE_BITS {
      assert!(!seen.is_duplicate(&Location::new(1, object)));
    }
    assert!(!seen.is_duplicate(&Location::new(1, INLINE_BITS)));

    // Ids from before the promotion are still known, as is the one that caused it.
    assert!(seen.is_duplicate(&Location::new(1, 0)));
    assert!(seen.is_duplicate(&Location::new(1, INLINE_BITS - 1)));
    assert!(seen.is_duplicate(&Location::new(1, INLINE_BITS)));
  }

  /// The window is the cap on what one group costs: ids far enough behind the newest
  /// are forgotten, and a group of any size still occupies one bounded slot.
  #[test]
  fn ids_older_than_the_window_are_forgotten() {
    let seen = SeenObjects::new(4);
    assert!(!seen.is_duplicate(&Location::new(1, 0)));

    // Push the window well past the first id.
    assert!(!seen.is_duplicate(&Location::new(1, WINDOW_BITS * 2)));

    assert!(!seen.is_duplicate(&Location::new(1, 0)), "forgotten");
    assert!(
      seen.is_duplicate(&Location::new(1, WINDOW_BITS * 2)),
      "the newest id is still known"
    );
  }

  /// Sliding the window by one evicts exactly one id, not the whole bitmap.
  #[test]
  fn sliding_the_window_evicts_only_what_it_passes() {
    let seen = SeenObjects::new(4);
    let group = 1;

    // Record 0 and 1, then advance the window by exactly one id past 0.
    seen.is_duplicate(&Location::new(group, 0));
    seen.is_duplicate(&Location::new(group, 1));
    seen.is_duplicate(&Location::new(group, WINDOW_BITS));

    // 0 has dropped out; 1 is still inside the window.
    assert!(!seen.is_duplicate(&Location::new(group, 0)));
    assert!(seen.is_duplicate(&Location::new(group, 1)));
  }
}
