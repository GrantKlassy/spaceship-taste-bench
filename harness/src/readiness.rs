//! Controller-owned readiness results. Never accepts a user-written certificate.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warning,
    Blocked,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub id: String,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub ready: bool,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn push(&mut self, id: &str, status: Status, detail: impl Into<String>) {
        self.checks.push(Check {
            id: id.into(),
            status,
            detail: detail.into(),
        });
        self.ready = self.checks.iter().all(|c| c.status != Status::Blocked);
    }

    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        self.checks.iter().map(|check| {
            let label = match check.status {
                Status::Pass => "PASS",
                Status::Warning => "WARNING",
                Status::Blocked => "BLOCKED",
            };
            format!("{label} [{}]: {}", check.id, check.detail)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_evidence_and_no_blocked_checks() {
        let mut report = Report::default();
        assert!(!report.ready);
        report.push("host", Status::Pass, "supported");
        report.push(
            "update_lookup",
            Status::Warning,
            "optional lookup unavailable",
        );
        assert!(report.ready);
        report.push("isolation", Status::Blocked, "gateway reachable");
        report.push("images", Status::Pass, "present");
        assert!(!report.ready);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["ready"], false);
        assert_eq!(value["checks"][2]["status"], "blocked");
    }
}
