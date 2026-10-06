//! Markdown export of a scenario report (German, for tickets and the shared drive).

use std::fmt::Write;

use crate::model::{CheckOutcome, ReportOutcome, ScenarioReport, TargetKind};
use crate::scenarios;

fn outcome_text(outcome: ReportOutcome) -> &'static str {
    match outcome {
        ReportOutcome::Passed => "bestanden",
        ReportOutcome::Failed => "fehlgeschlagen",
        ReportOutcome::Aborted => "abgebrochen",
    }
}

fn check_text(outcome: CheckOutcome) -> &'static str {
    match outcome {
        CheckOutcome::Passed => "bestanden",
        CheckOutcome::Failed => "fehlgeschlagen",
        CheckOutcome::Skipped => "übersprungen",
    }
}

fn target_text(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Local => "lokal",
        TargetKind::Live => "Produktivserver",
    }
}

/// Keeps a table cell on one line and stops text from breaking the table.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// "Label (identity)", or the id for reports written before label and identity were recorded.
fn wallbox_text(report: &ScenarioReport) -> String {
    match (&report.charge_point_label, &report.charge_point_identity) {
        (Some(label), Some(identity)) => format!("{label} ({identity})"),
        (Some(label), None) => label.clone(),
        (None, Some(identity)) => identity.clone(),
        (None, None) => report.charge_point_id.clone(),
    }
}

/// Renders the report as Markdown.
pub fn report_to_markdown(report: &ScenarioReport) -> String {
    let info = scenarios::info(report.scenario_id);
    let seconds = (report.finished_at - report.started_at).num_seconds();
    let format = "%d.%m.%Y %H:%M:%S UTC";
    let mut text = String::new();
    // Writing to a String cannot fail.
    let _ = writeln!(
        text,
        "# Szenario {:?}: {}\n",
        report.scenario_id, info.title
    );
    let _ = writeln!(text, "- Ergebnis: **{}**", outcome_text(report.outcome));
    // The internal UUID means nothing to a reader of the report; label and identity do.
    let _ = writeln!(text, "- Wallbox: {}", wallbox_text(report));
    let _ = writeln!(text, "- Ziel: {}", target_text(report.target_kind));
    let _ = writeln!(text, "- Start: {}", report.started_at.format(format));
    let _ = writeln!(
        text,
        "- Ende: {} ({seconds} s)\n",
        report.finished_at.format(format)
    );
    if report.checks.is_empty() {
        let _ = writeln!(text, "Es wurden keine Prüfungen aufgezeichnet.");
        return text;
    }
    let _ = writeln!(text, "| Prüfung | Ergebnis | Details |");
    let _ = writeln!(text, "|---|---|---|");
    for check in &report.checks {
        let _ = writeln!(
            text,
            "| {} | {} | {} |",
            cell(&check.name),
            check_text(check.outcome),
            cell(&check.detail)
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CheckResult, ScenarioId};
    use chrono::{Duration, TimeZone, Utc};

    fn report(checks: Vec<CheckResult>, outcome: ReportOutcome) -> ScenarioReport {
        let start = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
        ScenarioReport {
            scenario_id: ScenarioId::S5,
            charge_point_id: "box-1".into(),
            charge_point_label: Some("Wallbox 1".into()),
            charge_point_identity: Some("AP-TEST-1".into()),
            target_kind: TargetKind::Local,
            started_at: start,
            finished_at: start + Duration::seconds(42),
            outcome,
            checks,
        }
    }

    #[test]
    fn markdown_has_title_outcome_and_table() {
        let md = report_to_markdown(&report(
            vec![
                CheckResult {
                    name: "A".into(),
                    outcome: CheckOutcome::Passed,
                    detail: "ok".into(),
                },
                CheckResult {
                    name: "B|C".into(),
                    outcome: CheckOutcome::Skipped,
                    detail: "zwei\nZeilen".into(),
                },
            ],
            ReportOutcome::Passed,
        ));
        assert!(md.starts_with("# Szenario S5: StopTransaction mit transactionId 0"));
        assert!(md.contains("Ergebnis: **bestanden**"));
        assert!(md.contains("(42 s)"));
        assert!(md.contains("| A | bestanden | ok |"));
        assert!(
            md.contains("| B\\|C | übersprungen | zwei Zeilen |"),
            "table cells are escaped"
        );
    }

    #[test]
    fn header_names_the_wallbox_by_label_and_identity_not_by_internal_id() {
        let md = report_to_markdown(&report(vec![], ReportOutcome::Passed));
        assert!(md.contains("- Wallbox: Wallbox 1 (AP-TEST-1)"), "{md}");
        assert!(
            !md.contains("box-1"),
            "the internal id stays out of the report"
        );
        let mut old = report(vec![], ReportOutcome::Passed);
        old.charge_point_label = None;
        old.charge_point_identity = None;
        assert!(
            report_to_markdown(&old).contains("- Wallbox: box-1"),
            "reports without label fall back to the id"
        );
    }

    #[test]
    fn failed_and_aborted_reports_are_labelled_and_empty_ones_say_so() {
        let failed = report_to_markdown(&report(
            vec![CheckResult {
                name: "A".into(),
                outcome: CheckOutcome::Failed,
                detail: "x".into(),
            }],
            ReportOutcome::Failed,
        ));
        assert!(failed.contains("**fehlgeschlagen**"));
        assert!(failed.contains("| A | fehlgeschlagen | x |"));
        let aborted = report_to_markdown(&report(vec![], ReportOutcome::Aborted));
        assert!(aborted.contains("**abgebrochen**"));
        assert!(aborted.contains("keine Prüfungen"));
    }
}
