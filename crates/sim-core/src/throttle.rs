//! Rate limit for UI events: at most one emission per interval, with a trailing emission so the last
//! change is never lost.

use std::time::{Duration, Instant};

/// Decides when a pending change may be emitted.
#[derive(Debug, Clone)]
pub struct Throttle {
    min_interval: Duration,
    last_emit: Option<Instant>,
    pending: bool,
}

impl Throttle {
    /// A throttle that allows one emission per `min_interval`.
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_emit: None,
            pending: false,
        }
    }

    /// Records that something changed and has not been emitted yet.
    pub fn mark_changed(&mut self) {
        self.pending = true;
    }

    /// True when a change is pending and the interval since the last emission has passed.
    pub fn is_due(&self, now: Instant) -> bool {
        self.pending && self.due_at().is_none_or(|at| now >= at)
    }

    /// The moment the pending change may be emitted; `None` when nothing is pending.
    pub fn next_emission(&self) -> Option<Instant> {
        if self.pending {
            Some(self.due_at().unwrap_or_else(Instant::now))
        } else {
            None
        }
    }

    fn due_at(&self) -> Option<Instant> {
        self.last_emit.map(|last| last + self.min_interval)
    }

    /// Records an emission and clears the pending flag.
    pub fn mark_emitted(&mut self, now: Instant) {
        self.last_emit = Some(now);
        self.pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_pending_is_never_due() {
        let t = Throttle::new(Duration::from_millis(250));
        assert!(!t.is_due(Instant::now()));
        assert!(t.next_emission().is_none());
    }

    #[test]
    fn first_change_is_emitted_immediately() {
        let mut t = Throttle::new(Duration::from_millis(250));
        t.mark_changed();
        assert!(t.is_due(Instant::now()));
    }

    #[test]
    fn at_most_four_emissions_per_second() {
        let mut t = Throttle::new(Duration::from_millis(250));
        let start = Instant::now();
        let mut emitted = 0;
        for ms in (0..1000).step_by(10) {
            let now = start + Duration::from_millis(ms);
            t.mark_changed();
            if t.is_due(now) {
                t.mark_emitted(now);
                emitted += 1;
            }
        }
        assert_eq!(emitted, 4, "0, 250, 500 and 750 ms");
    }

    #[test]
    fn counter_check_change_inside_the_interval_waits_and_is_not_lost() {
        let mut t = Throttle::new(Duration::from_millis(250));
        let start = Instant::now();
        t.mark_changed();
        t.mark_emitted(start);
        t.mark_changed();
        assert!(!t.is_due(start + Duration::from_millis(100)));
        assert_eq!(t.next_emission(), Some(start + Duration::from_millis(250)));
        assert!(t.is_due(start + Duration::from_millis(250)));
    }
}
