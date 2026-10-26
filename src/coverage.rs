//! Same unit as upstream's title: native dispatches versus interpreted steps.
//! This is not the fraction of translated ROM bytes or executed instructions.
#[derive(Clone, Copy, Default)]
pub struct Coverage {
    pub native_dispatches: u64,
    pub interpreted_steps: u64,
}

impl Coverage {
    pub fn title(self, interpreter: bool, paused: bool, notice: &str) -> String {
        let status = if interpreter {
            format!(
                "Native dispatch: 0.00% | interpreted: {}",
                self.interpreted_steps
            )
        } else {
            let total = self.native_dispatches as f64 + self.interpreted_steps as f64;
            let percent = if total == 0.0 {
                "--".to_owned()
            } else if self.interpreted_steps == 0 {
                "100.00%".to_owned()
            } else {
                // Never round away an actual fallback to a misleading 100%.
                format!(
                    "{:.2}%",
                    (100.0 * self.native_dispatches as f64 / total).min(99.99)
                )
            };
            format!(
                "Native dispatch: {percent} | interpreted: {}",
                self.interpreted_steps
            )
        };
        format!(
            "AdvanceLiberty: Enhanced | {status}{}{}",
            if paused { " | paused" } else { "" },
            if notice.is_empty() {
                String::new()
            } else {
                format!(" | {notice}")
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_fallback_is_visible_even_when_percentage_would_round_to_100() {
        let title = Coverage {
            native_dispatches: 100_000_000,
            interpreted_steps: 1,
        }
        .title(false, false, "");
        assert!(title.contains("99.99%"));
        assert!(title.contains("interpreted: 1"));
    }

    #[test]
    fn distinguishes_unmeasured_native_and_explicit_interpreter_sessions() {
        assert!(Coverage::default()
            .title(false, false, "")
            .contains("Native dispatch: --"));
        let native = Coverage {
            native_dispatches: 50,
            interpreted_steps: 0,
        };
        assert!(native
            .title(false, true, "")
            .contains("100.00% | interpreted: 0 | paused"));
        assert!(native
            .title(true, false, "")
            .contains("0.00% | interpreted: 0"));
    }
}
