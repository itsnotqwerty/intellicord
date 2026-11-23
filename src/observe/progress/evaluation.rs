use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use super::Summary;
use crate::observe::{
    lifecycle::{Phase, Reason},
    stream::Source,
    supervisor::Outcome,
};

const INPUT_LIMIT: usize = 64 * 1024;
const LINE_LIMIT: usize = 4096;
const RUN_LIMIT: usize = 60;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRun {
    schema_version: u16,
    outcome: Outcome,
    collector_started: bool,
    collector_reaped: bool,
    deadline: Option<serde_json::Value>,
    summary: Option<Summary>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputReport {
    schema_version: u16,
    source: String,
    transport: String,
    session_binding: String,
    endpoint_removed: bool,
    run: InputRun,
}

#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct Totals {
    accepted: u64,
    rejected_record: u64,
    rejected_clock: u64,
    ring_dropped: u64,
}

#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct Evaluation {
    schema_version: u16,
    source: &'static str,
    runs: usize,
    requested_seconds: u64,
    minimum_health_samples_per_run: u64,
    health_samples: u64,
    totals: Totals,
    completed: bool,
    cleanup_confirmed: bool,
    attachment_confirmed: bool,
    detachment_confirmed: bool,
    record_conservation: bool,
}

pub fn evaluate(input: &[u8], seconds: u64) -> anyhow::Result<Evaluation> {
    anyhow::ensure!((1..=60).contains(&seconds), "invalid evaluation duration");
    anyhow::ensure!(
        !input.is_empty() && input.len() <= INPUT_LIMIT,
        "invalid evaluation input size"
    );
    let minimum_samples = seconds.saturating_sub(1);
    let mut runs = 0_usize;
    let mut health_samples = 0_u64;
    let mut totals = Totals {
        accepted: 0,
        rejected_record: 0,
        rejected_clock: 0,
        ring_dropped: 0,
    };
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        anyhow::ensure!(
            line.len() <= LINE_LIMIT && runs < RUN_LIMIT,
            "evaluation input limit"
        );
        let report: InputReport = serde_json::from_slice(line)?;
        let summary = report
            .run
            .summary
            .as_ref()
            .context("missing progress summary")?;
        let terminal = summary
            .terminal_health
            .context("missing terminal process health")?;
        anyhow::ensure!(
            report.schema_version == 1
                && report.source == "supervised_kernel_socket_progress"
                && report.transport == "kernel_verified_unix_socket"
                && report.session_binding == "parent_selected"
                && report.endpoint_removed
                && report.run.schema_version == 1
                && report.run.outcome == Outcome::Completed
                && report.run.collector_started
                && report.run.collector_reaped
                && report.run.deadline.is_none()
                && summary.schema_version == 1
                && summary.records == summary.lifecycle.records + summary.health_samples
                && summary.health_samples >= minimum_samples
                && summary.lifecycle.schema_version == 1
                && summary.lifecycle.source == Source::KernelTracepoint
                && summary.lifecycle.records == 6
                && summary.lifecycle.terminal_phase == Phase::Stopped
                && summary.lifecycle.reason == Reason::DurationLimit
                && summary.lifecycle.attachment_confirmed
                && summary.lifecycle.detachment_confirmed
                && terminal.schema_version == 1,
            "incomplete or inconsistent progress report"
        );
        for count in [
            terminal.accepted,
            terminal.rejected_record,
            terminal.rejected_clock,
            terminal.ring_dropped,
        ] {
            anyhow::ensure!(count <= MAX_SAFE_INTEGER, "unsafe process counter");
        }
        health_samples = health_samples
            .checked_add(summary.health_samples)
            .context("health sample total overflow")?;
        totals.accepted = totals
            .accepted
            .checked_add(terminal.accepted)
            .context("accepted total overflow")?;
        totals.rejected_record = totals
            .rejected_record
            .checked_add(terminal.rejected_record)
            .context("record rejection total overflow")?;
        totals.rejected_clock = totals
            .rejected_clock
            .checked_add(terminal.rejected_clock)
            .context("clock rejection total overflow")?;
        totals.ring_dropped = totals
            .ring_dropped
            .checked_add(terminal.ring_dropped)
            .context("ring drop total overflow")?;
        runs += 1;
    }
    anyhow::ensure!(runs > 0, "no progress reports");
    Ok(Evaluation {
        schema_version: 1,
        source: "offline_progress_report_evaluation",
        runs,
        requested_seconds: seconds,
        minimum_health_samples_per_run: minimum_samples,
        health_samples,
        totals,
        completed: true,
        cleanup_confirmed: true,
        attachment_confirmed: true,
        detachment_confirmed: true,
        record_conservation: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(samples: u64, ring_dropped: u64) -> String {
        serde_json::json!({
            "schema_version":1,
            "source":"supervised_kernel_socket_progress",
            "transport":"kernel_verified_unix_socket",
            "session_binding":"parent_selected",
            "endpoint_removed":true,
            "run":{"schema_version":1,"outcome":"completed","collector_started":true,
                "collector_reaped":true,"deadline":null,"summary":{"schema_version":1,
                "records":6+samples,"health_samples":samples,
                "lifecycle":{"schema_version":1,"source":"kernel_tracepoint","records":6,
                    "terminal_phase":"stopped","reason":"duration_limit",
                    "attachment_confirmed":true,"detachment_confirmed":true},
                "terminal_health":{"schema_version":1,"accepted":3,"rejected_record":0,
                    "rejected_clock":0,"ring_dropped":ring_dropped}}}
        })
        .to_string()
    }

    #[test]
    fn aggregates_completed_runs_and_rejects_missing_cadence_or_cleanup() {
        let input = format!("{}\n{}\n", report(4, 0), report(5, 2));
        let evaluation = evaluate(input.as_bytes(), 5).unwrap();
        assert_eq!(evaluation.runs, 2);
        assert_eq!(evaluation.health_samples, 9);
        assert_eq!(evaluation.totals.ring_dropped, 2);
        for mutation in [
            report(3, 0),
            report(4, 0).replace("\"endpoint_removed\":true", "\"endpoint_removed\":false"),
            report(4, 0).replace(
                "\"terminal_health\":{",
                "\"unknown\":0,\"terminal_health\":{",
            ),
        ] {
            assert!(evaluate(format!("{mutation}\n").as_bytes(), 5).is_err());
        }
    }
}
