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

//! GREASE reservations. Values matching `0x7f * N + 0x9D` are reserved across
//! several registries (Setup Options, Properties, error codes) so peers can emit
//! them to exercise unknown-value handling. Receivers treat them like any other
//! unknown value: ignore, never fatal.

/// Step between successive GREASE values.
pub const GREASE_STEP: u64 = 0x7f;
/// The smallest GREASE value (N = 0).
pub const GREASE_BASE: u64 = 0x9d;

/// Returns true if `value` is a reserved GREASE value, i.e. `0x7f * N + 0x9D`
/// for some non-negative `N`.
pub fn is_grease(value: u64) -> bool {
  value >= GREASE_BASE && (value - GREASE_BASE).is_multiple_of(GREASE_STEP)
}

/// The `N`-th GREASE value, `0x7f * N + 0x9D`. Returns `None` if it would
/// overflow `u64` (the pattern is capped at `0x3fffffffffffffde`).
pub fn grease_value(n: u64) -> Option<u64> {
  GREASE_STEP.checked_mul(n)?.checked_add(GREASE_BASE)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn recognizes_the_documented_grease_values() {
    // 0x9D, 0x11C, ..., 0x3fffffffffffffde.
    assert!(is_grease(0x9d));
    assert!(is_grease(0x11c));
    assert!(is_grease(0x3fffffffffffffde));
    for n in 0..1000 {
      assert!(is_grease(grease_value(n).unwrap()), "N={n}");
    }
  }

  #[test]
  fn rejects_non_grease_values() {
    for v in [0u64, 1, 0x02, 0x9c, 0x9e, 0x11b, 0x11d, 0x4000] {
      assert!(!is_grease(v), "{v:#x} must not be grease");
    }
  }

  #[test]
  fn grease_value_matches_the_formula() {
    assert_eq!(grease_value(0), Some(0x9d));
    assert_eq!(grease_value(1), Some(0x11c));
    // The largest grease value the spec lists.
    assert_eq!(grease_value(0x8102040810203f), Some(0x3fffffffffffffde));
    assert_eq!(grease_value(u64::MAX), None);
  }
}
