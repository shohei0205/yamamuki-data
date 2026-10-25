use std::time::{Duration, Instant};

// 最新の視点を一つだけ保留し、連続操作で更新期限を先送りしない。
pub struct Scheduler {
    pub desired: u64,
    epoch: u64,
    due: Option<Instant>,
    flight: Option<(u64, u64)>,
}
impl Scheduler {
    pub fn new(now: Instant) -> Self {
        Self {
            desired: 0,
            epoch: 0,
            due: Some(now),
            flight: None,
        }
    }
    pub fn invalidate(&mut self, now: Instant) {
        self.desired += 1;
        self.epoch += 1;
        self.due = Some(now);
    }
    pub fn moved(&mut self, now: Instant) {
        self.desired += 1;
        self.due.get_or_insert(now + Duration::from_millis(100));
    }
    pub fn take(&mut self, now: Instant) -> Option<(u64, u64)> {
        if self.flight.is_some() || !self.due.is_some_and(|due| due <= now) {
            return None;
        }
        let id = (self.desired, self.epoch);
        self.flight = Some(id);
        self.due = None;
        Some(id)
    }
    pub fn complete(&mut self, id: u64, epoch: u64) -> bool {
        if self.flight != Some((id, epoch)) {
            return false;
        }
        self.flight = None;
        epoch == self.epoch
    }
    pub fn pending(&self) -> bool {
        self.flight.is_some() || self.due.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_does_not_postpone_deadline() {
        let start = Instant::now();
        let mut s = Scheduler::new(start);
        let (id, epoch) = s.take(start).unwrap();
        assert!(s.complete(id, epoch));
        for ms in [0, 16, 32, 48, 64, 80, 96] {
            s.moved(start + Duration::from_millis(ms));
        }
        assert!(s.take(start + Duration::from_millis(101)).is_some());
    }
    #[test]
    fn movement_accepts_intermediate_result_and_keeps_latest_pending() {
        let start = Instant::now();
        let mut s = Scheduler::new(start);
        let (id, epoch) = s.take(start).unwrap();
        for ms in [16, 32, 48, 64, 80, 96, 112, 128] {
            s.moved(start + Duration::from_millis(ms));
        }
        assert!(s.take(start + Duration::from_millis(200)).is_none());
        assert!(s.complete(id, epoch));
        assert!(s.pending());
        let (next, _) = s.take(start + Duration::from_millis(201)).unwrap();
        assert_eq!(next, s.desired);
    }
    #[test]
    fn changed_settings_reject_previous_generation() {
        let start = Instant::now();
        let mut s = Scheduler::new(start);
        let (id, epoch) = s.take(start).unwrap();
        s.invalidate(start);
        assert!(!s.complete(id, epoch));
        let (next, new_epoch) = s.take(start).unwrap();
        assert_ne!(new_epoch, epoch);
        assert!(s.complete(next, new_epoch));
        assert!(!s.pending());
    }
}
