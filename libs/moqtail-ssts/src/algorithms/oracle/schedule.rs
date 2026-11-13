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

//! The capacity schedule the host hands the oracle: the applied `tc` steps of
//! one bandwidth trace, as `cap_kbps:duration_ms` pairs cumulative from the
//! trace's start.
//!
//! The schedule is the oracle's whole clairvoyance, and it arrives as a tuning
//! value — the host reads the profile and formats the steps, so the crate
//! never touches the filesystem and a test can hand it any trace it wants.

/// The parsed `oracle.steps` tuning value.
#[derive(Debug, Clone, Default)]
pub(crate) struct Schedule {
  /// `(capacity kbps, start ms)`, ascending by start; the first starts at 0.
  steps: Vec<(f64, u64)>,
  /// Total ms the schedule covers.
  total_ms: u64,
}

impl Schedule {
  /// From the tuning value's text: `cap_kbps:duration_ms` pairs joined by `;`.
  ///
  /// An entry that does not parse, or that covers no time, is dropped with a
  /// warning — which shortens the schedule rather than shifting it. The driver
  /// formats the value from the profile it already validated, so this guards a
  /// hand-typed flag only; a deployment typo still says so in the log.
  pub(crate) fn from_steps_value(value: &str) -> Self {
    let mut steps = Vec::new();
    let mut total_ms = 0u64;
    for pair in value.split(';') {
      let pair = pair.trim();
      if pair.is_empty() {
        continue;
      }
      let dropped = format!("SSTS oracle: dropping step '{pair}'");
      let Some((cap, dur)) = pair.split_once(':') else {
        tracing::warn!("{dropped}: no ':' in it");
        continue;
      };
      let (cap, dur) = match (cap.trim().parse::<f64>(), dur.trim().parse::<u64>()) {
        (Ok(cap), Ok(dur)) => (cap.max(0.0), dur),
        _ => {
          tracing::warn!("{dropped}: its rate or its duration does not parse");
          continue;
        }
      };
      if dur == 0 {
        tracing::warn!("{dropped}: it covers no time");
        continue;
      }
      steps.push((cap, total_ms));
      total_ms += dur;
    }
    Self { steps, total_ms }
  }

  /// The schedule re-based at `origin_ms`: the lead-in before it is dropped —
  /// the run's bring-up consumes it in every run and no sender could have used
  /// it — and the step it straddles starts there, so the windows measured from
  /// the origin are the run's own delivery windows.
  pub(crate) fn rebased_at(self, origin_ms: u64) -> Self {
    // The last step that starts at or before the origin carries it; everything
    // after it is untouched. An origin before the schedule's start re-bases at
    // its first step.
    let index = self
      .steps
      .iter()
      .rposition(|&(_, start)| start <= origin_ms)
      .unwrap_or(0);
    let steps = self.steps[index..]
      .iter()
      .map(|&(cap, start)| (cap, start.saturating_sub(origin_ms)))
      .collect();
    Self {
      steps,
      total_ms: self.total_ms.saturating_sub(origin_ms),
    }
  }

  /// Mean capacity in kbps over `[from_ms, to_ms)` of the schedule's own time,
  /// holding the last step's capacity past its end (the run is over; the path
  /// is not).
  pub(crate) fn mean_kbps(&self, from_ms: u64, to_ms: u64) -> f64 {
    if to_ms <= from_ms {
      return self.capacity_at(from_ms);
    }
    // The step `from_ms` lands in, then every step that overlaps the window.
    let mut index = self
      .steps
      .iter()
      .rposition(|&(_, start)| start <= from_ms)
      .unwrap_or(0);
    let mut accumulated = 0.0;
    let mut at = from_ms;
    while at < to_ms {
      let Some(&(cap, _)) = self.steps.get(index) else {
        // Past the schedule's end: the last capacity holds to infinity.
        accumulated += self.last_capacity() * (to_ms - at) as f64;
        break;
      };
      let end = self
        .steps
        .get(index + 1)
        .map_or(self.total_ms, |&(_, start)| start);
      let stop = end.min(to_ms).max(at);
      accumulated += cap * (stop - at) as f64;
      at = stop;
      index += 1;
    }
    accumulated / (to_ms - from_ms) as f64
  }

  /// The capacity in kbps at `at_ms`.
  fn capacity_at(&self, at_ms: u64) -> f64 {
    self
      .steps
      .iter()
      .rev()
      .find(|&&(_, start)| start <= at_ms)
      .map_or(self.last_capacity(), |&(cap, _)| cap)
  }

  /// The last step's capacity, or zero on a schedule with no steps.
  fn last_capacity(&self) -> f64 {
    self.steps.last().map_or(0.0, |&(cap, _)| cap)
  }
}
