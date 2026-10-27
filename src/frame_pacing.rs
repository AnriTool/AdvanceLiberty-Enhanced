//! Pace frame submission, rather than the start of the next simulation step.
use std::time::{Duration, Instant};
pub fn next_deadline(previous: Instant, now: Instant, period: Duration) -> Instant {
    let next = previous + period;
    if now.saturating_duration_since(next) >= period {
        now
    } else {
        next
    }
}
pub fn wait_until(deadline: Instant) {
    let margin = Duration::from_millis(1);
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if remaining > margin {
            std::thread::sleep(remaining - margin);
        }
    }
    while Instant::now() < deadline {
        std::hint::spin_loop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variable_render_cost_keeps_submission_on_the_frame_grid() {
        let base = Instant::now();
        let period = Duration::from_nanos(16_742_706);
        let mut last = base;
        for (i, cost) in [3, 12, 5, 14, 2, 11].into_iter().enumerate() {
            let ready = last + Duration::from_millis(cost);
            let target = next_deadline(last, ready, period);
            assert_eq!(target, base + period * (i as u32 + 1));
            assert!(target >= ready);
            last = target;
        }
    }
    #[test]
    fn long_stall_rebases_instead_of_submitting_a_burst_of_catchup_frames() {
        let base = Instant::now();
        let period = Duration::from_nanos(16_742_706);
        let ready = base + Duration::from_millis(120);
        let first = next_deadline(base, ready, period);
        assert_eq!(first, ready);
        assert_eq!(
            next_deadline(first, first + Duration::from_millis(4), period),
            first + period
        );
    }
}
