#![forbid(unsafe_code)]

//! Display-only classification. These groups never grant release authority.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Success,
    Failure,
    Active,
    Other,
}

pub fn classify(status: &str) -> Group {
    match status {
        "success" | "succeeded" | "completed_success" => Group::Success,
        "failure" | "failed" | "timed_out" | "completed_failure" | "startup_failure" => Group::Failure,
        "queued" | "pending" | "waiting" | "requested" | "running" | "in_progress" => Group::Active,
        _ => Group::Other,
    }
}

impl Group {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Active => "active",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub total: usize,
    pub success: usize,
    pub failure: usize,
    pub active: usize,
    pub other: usize,
}

pub fn summarize<'a>(statuses: impl IntoIterator<Item = &'a str>) -> Counts {
    let mut counts = Counts::default();
    for status in statuses {
        counts.total += 1;
        match classify(status) {
            Group::Success => counts.success += 1,
            Group::Failure => counts.failure += 1,
            Group::Active => counts.active += 1,
            Group::Other => counts.other += 1,
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_success_is_success() {
        for value in ["success", "succeeded", "completed_success"] {
            assert_eq!(classify(value), Group::Success);
        }
    }

    #[test]
    fn known_failures_remain_visible() {
        for value in ["failure", "failed", "timed_out", "completed_failure", "startup_failure"] {
            assert_eq!(classify(value), Group::Failure);
        }
    }

    #[test]
    fn active_is_not_success() {
        for value in ["queued", "pending", "waiting", "requested", "running", "in_progress"] {
            assert_eq!(classify(value), Group::Active);
        }
    }

    #[test]
    fn completed_or_unknown_is_never_inferred_success() {
        for value in ["", "completed", "skipped", "cancelled", "canceled", "neutral", "SUCCESS", "new-state"] {
            assert_eq!(classify(value), Group::Other);
        }
    }

    #[test]
    fn empty_window_does_not_report_success() {
        assert_eq!(summarize([]), Counts::default());
    }

    #[test]
    fn every_observation_is_counted_once() {
        let counts = summarize(["success", "failure", "pending", "completed", "skipped"]);
        assert_eq!(counts.total, 5);
        assert_eq!(counts.success, 1);
        assert_eq!(counts.failure, 1);
        assert_eq!(counts.active, 1);
        assert_eq!(counts.other, 2);
    }

    #[test]
    fn grouping_is_lossless_for_every_supported_window_size() {
        for length in 0..=200 {
            let statuses: Vec<_> = ["success", "failed", "running", "unrecognized"]
                .into_iter()
                .cycle()
                .take(length)
                .collect();
            let counts = summarize(statuses);
            assert_eq!(counts.total, length);
            assert_eq!(counts.total, counts.success + counts.failure + counts.active + counts.other);
        }
    }

    #[test]
    fn filter_labels_are_stable() {
        assert_eq!(Group::Success.as_str(), "success");
        assert_eq!(Group::Failure.as_str(), "failure");
        assert_eq!(Group::Active.as_str(), "active");
        assert_eq!(Group::Other.as_str(), "other");
    }
}
