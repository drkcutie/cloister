use std::time::{Duration, Instant};

/// The two-second hold-to-commit, for a pointer or a held key. The UI calls [`Hold::poll`]
/// on every frame while [`Hold::is_holding`] and commits when it returns `Some`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hold {
    since: Option<Instant>,
}

impl Hold {
    /// How long to hold. The fill sinks linearly over this time.
    pub const DURATION: Duration = Duration::from_secs(2);

    /// Start holding. Ignored while already holding, so key repeat doesn't restart it.
    pub fn press(&mut self, now: Instant) {
        self.since.get_or_insert(now);
    }

    /// Let go. Before the two seconds are up nothing is committed.
    pub fn release(&mut self) {
        self.since = None;
    }

    pub fn is_holding(&self) -> bool {
        self.since.is_some()
    }

    /// From 0 to 1.
    pub fn progress(&self, now: Instant) -> f32 {
        self.since.map_or(0.0, |since| {
            (now.saturating_duration_since(since).as_secs_f32() / Self::DURATION.as_secs_f32())
                .min(1.0)
        })
    }

    /// `Some(())` exactly once, when the hold reaches two seconds; the hold then ends.
    pub fn poll(&mut self, now: Instant) -> Option<()> {
        if self.since.is_some() && self.progress(now) >= 1.0 {
            self.since = None;
            Some(())
        } else {
            None
        }
    }
}
