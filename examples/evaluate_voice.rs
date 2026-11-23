use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use intellicord::detection::{Intelligence, ObservationSource};
use intellicord::ingestion::{Producer, SensorBus, SensorMessage, SensorPayload, VoiceFrame};
use intellicord::{Channel, Command, History, User};
use serde::{Deserialize, Serialize};

const WALL_ORIGIN_MS: u64 = 1_700_000_000_000;
const TRACES: &str = include_str!("../tests/fixtures/voice-patterns.json");
const LABELS: &str = include_str!("../tests/fixtures/voice-evaluation.json");

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Rule {
    JoinBurst,
    RapidReentry,
}

const RULES: [Rule; 2] = [Rule::JoinBurst, Rule::RapidReentry];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Trace {
    name: String,
    expected_rules: Vec<Rule>,
    expected_transitions: usize,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    elapsed_ms: u64,
    ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    schema_version: u16,
    dataset_version: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    trace: String,
    rationale: String,
    positive_rules: Vec<Rule>,
    onset_ms: Option<u64>,
    expected_detected_rules: Vec<Rule>,
    #[serde(default)]
    timing_offsets_ms: Vec<i64>,
    #[serde(default)]
    capture_gap_steps: Vec<usize>,
}

#[derive(Serialize)]
struct Detection {
    rule: Rule,
    detector_version: String,
    first_detection_elapsed_ms: u64,
    receipt_latency_ms: Option<u64>,
    incident_count: usize,
}

#[derive(Serialize)]
struct CaseResult {
    name: String,
    trace: String,
    rationale: String,
    positive_rules: Vec<Rule>,
    expected_detected_rules: Vec<Rule>,
    matches_expected_output: bool,
    onset_ms: Option<u64>,
    timing_offsets_ms: Vec<i64>,
    capture_gap_steps: Vec<usize>,
    duration_ms: u64,
    gap_count: u64,
    known_missing_transitions: u64,
    detections: Vec<Detection>,
}

#[derive(Serialize)]
struct RuleResult {
    #[serde(flatten)]
    counts: Counts,
    precision: Option<f64>,
    recall: Option<f64>,
}

#[derive(Serialize)]
struct Report {
    schema_version: u16,
    dataset_version: String,
    evaluation_unit: &'static str,
    limitations: Vec<&'static str>,
    rules: BTreeMap<Rule, RuleResult>,
    cases: Vec<CaseResult>,
}

#[derive(Debug, Default, Serialize, PartialEq)]
struct Counts {
    true_positive: u64,
    false_positive: u64,
    true_negative: u64,
    false_negative: u64,
}

impl Counts {
    fn record(&mut self, positive: bool, detected: bool) {
        match (positive, detected) {
            (true, true) => self.true_positive += 1,
            (false, true) => self.false_positive += 1,
            (false, false) => self.true_negative += 1,
            (true, false) => self.false_negative += 1,
        }
    }

    fn precision(&self) -> Option<f64> {
        ratio(self.true_positive, self.true_positive + self.false_positive)
    }

    fn recall(&self) -> Option<f64> {
        ratio(self.true_positive, self.true_positive + self.false_negative)
    }
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn evaluate(traces_json: &str, labels_json: &str) -> Result<Report> {
    ensure!(
        traces_json.len() <= 1_048_576 && labels_json.len() <= 1_048_576,
        "Fixture input exceeds 1 MiB"
    );
    let traces: Vec<Trace> = serde_json::from_str(traces_json).context("Invalid voice traces")?;
    let suite: Suite = serde_json::from_str(labels_json).context("Invalid evaluation labels")?;
    ensure!(suite.schema_version == 1, "Unsupported evaluation schema");
    ensure!(
        !suite.dataset_version.is_empty() && suite.dataset_version.len() <= 128,
        "Invalid dataset version"
    );
    ensure!(
        !traces.is_empty()
            && traces.len() <= 64
            && !suite.cases.is_empty()
            && suite.cases.len() <= 128,
        "Invalid fixture collection size"
    );
    let mut trace_names = BTreeSet::new();
    for trace in &traces {
        ensure!(trace_names.insert(&trace.name), "Duplicate trace name");
        ensure!(
            !trace.steps.is_empty() && trace.steps.len() <= 512,
            "Invalid trace step count"
        );
        ensure!(trace.steps[0].elapsed_ms == 0, "Trace must start at zero");
        ensure!(
            trace.expected_transitions <= 10_000 && trace.expected_rules.len() <= 2,
            "Invalid trace expectations"
        );
    }
    let mut names = BTreeSet::new();
    let mut counts: BTreeMap<_, _> = RULES
        .into_iter()
        .map(|rule| (rule, Counts::default()))
        .collect();
    let mut cases = Vec::new();
    for case in suite.cases {
        ensure!(
            !case.name.is_empty() && case.name.len() <= 256 && names.insert(case.name.clone()),
            "Invalid or duplicate case name"
        );
        ensure!(
            !case.rationale.is_empty() && case.rationale.len() <= 4000,
            "Missing or oversized rationale"
        );
        let trace = traces
            .iter()
            .find(|trace| trace.name == case.trace)
            .context("Unknown trace reference")?;
        ensure!(
            case.timing_offsets_ms.is_empty() || case.timing_offsets_ms.len() == trace.steps.len(),
            "Timing offsets must match steps"
        );
        ensure!(
            case.capture_gap_steps
                .iter()
                .all(|index| *index < trace.steps.len()),
            "Invalid capture gap index"
        );
        for rules in [&case.positive_rules, &case.expected_detected_rules] {
            ensure!(
                rules.iter().collect::<BTreeSet<_>>().len() == rules.len(),
                "Duplicate rule label"
            );
        }
        ensure!(
            case.positive_rules.is_empty() == case.onset_ms.is_none(),
            "Positive labels require onset; negative labels must not have onset"
        );
        let bus = SensorBus::default();
        let publisher = bus.publisher();
        let mut history = History::default();
        let mut engine = Intelligence::new("offline-evaluation".into());
        let mut previous_ms = 0;
        for (index, step) in trace.steps.iter().enumerate() {
            let offset = case.timing_offsets_ms.get(index).copied().unwrap_or(0);
            let elapsed_ms = step
                .elapsed_ms
                .checked_add_signed(offset)
                .context("Invalid adjusted timestamp")?;
            ensure!(
                elapsed_ms <= 3_600_000
                    && elapsed_ms >= previous_ms
                    && (index != 0 || elapsed_ms == 0),
                "Nonmonotonic or oversized trace duration"
            );
            ensure!(
                step.ids.len() <= 100
                    && step.ids.iter().collect::<BTreeSet<_>>().len() == step.ids.len(),
                "Duplicate or excessive participants"
            );
            ensure!(
                step.ids.iter().all(|id| !id.is_empty()
                    && id.len() <= 20
                    && id.bytes().all(|byte| byte.is_ascii_digit())),
                "Invalid string participant ID"
            );
            let timestamp = WALL_ORIGIN_MS + elapsed_ms;
            history
                .apply(
                    Command::Snapshot {
                        channel: Some(Channel {
                            id: "100".into(),
                            name: "Synthetic evaluation".into(),
                            kind: None,
                        }),
                        users: step
                            .ids
                            .iter()
                            .map(|id| User {
                                id: id.clone(),
                                username: "synthetic".into(),
                                display_name: "Synthetic".into(),
                                avatar_url: None,
                            })
                            .collect(),
                    },
                    timestamp,
                )
                .map_err(anyhow::Error::msg)?;
            let source = if index == 0 {
                ObservationSource::InitialSnapshot
            } else {
                ObservationSource::Reconciliation
            };
            publisher
                .publish(SensorMessage {
                    schema_version: 1,
                    session_id: engine.session_id.clone(),
                    producer: Producer::Application,
                    producer_sequence: index as u64 + 1,
                    monotonic_ns: elapsed_ms * 1_000_000,
                    wall_time_unix_ms: timestamp,
                    payload: SensorPayload::Voice(VoiceFrame::from_history(
                        &history,
                        source,
                        case.capture_gap_steps.contains(&index),
                    )),
                })
                .map_err(|error| anyhow::anyhow!("Evaluation publication rejected: {error:?}"))?;
            ensure!(
                engine.drain(&bus) == 1 && engine.ingestion.accepted == index as u64 + 1,
                "Evaluation input rejected by fusion"
            );
            previous_ms = elapsed_ms;
        }
        ensure!(
            case.onset_ms.is_none_or(|onset| onset <= previous_ms),
            "Onset exceeds trace duration"
        );
        ensure!(
            engine.dropped_incidents == 0 && engine.dropped_observations == 0,
            "Evaluation exceeds evidence retention; metrics would be incomplete"
        );
        let mut detections = Vec::new();
        for rule in RULES {
            let rule_name = serde_json::to_value(rule)?;
            let incidents: Vec<_> = engine
                .incidents
                .iter()
                .filter(|incident| rule_name.as_str() == Some(incident.rule))
                .collect();
            let positive = case.positive_rules.contains(&rule);
            counts
                .get_mut(&rule)
                .unwrap()
                .record(positive, !incidents.is_empty());
            if let Some(first) = incidents.first() {
                let elapsed = first.wall_time_unix_ms - WALL_ORIGIN_MS;
                let latency = if positive {
                    Some(
                        elapsed
                            .checked_sub(case.onset_ms.unwrap())
                            .context("Detection precedes labeled onset")?,
                    )
                } else {
                    None
                };
                detections.push(Detection {
                    rule,
                    detector_version: first.detector_version.into(),
                    first_detection_elapsed_ms: elapsed,
                    receipt_latency_ms: latency,
                    incident_count: incidents.len(),
                });
            }
        }
        let detected: BTreeSet<_> = detections.iter().map(|entry| entry.rule).collect();
        let matches_expected_output =
            detected == case.expected_detected_rules.iter().copied().collect();
        cases.push(CaseResult {
            name: case.name,
            trace: case.trace,
            rationale: case.rationale,
            positive_rules: case.positive_rules,
            expected_detected_rules: case.expected_detected_rules,
            matches_expected_output,
            onset_ms: case.onset_ms,
            timing_offsets_ms: case.timing_offsets_ms,
            capture_gap_steps: case.capture_gap_steps,
            duration_ms: previous_ms,
            gap_count: engine.gap_count,
            known_missing_transitions: engine.missing_transitions,
            detections,
        });
    }
    let rules = counts
        .into_iter()
        .map(|(rule, counts)| {
            (
                rule,
                RuleResult {
                    precision: counts.precision(),
                    recall: counts.recall(),
                    counts,
                },
            )
        })
        .collect();
    Ok(Report {
        schema_version: 1,
        dataset_version: suite.dataset_version,
        evaluation_unit: "One binary decision per labeled synthetic case per rule; repeated findings do not add true positives",
        limitations: vec![
            "Hand-authored synthetic labels are not evidence of real user intent or measured field accuracy.",
            "Cases reuse traces and include deterministic perturbations; cases are not independent population samples.",
            "Receipt latency is measured from the fixture onset in backend receipt time, not true remote action time.",
            "Null precision/recall means no denominator. Benign synchronized arrivals and reconnects can produce false positives.",
            "No live Discord, audio, mixed-speaker attribution, learned baselines, alerts, or automatic moderation is evaluated.",
            "False alerts per hour and comparison to independent detection baselines require representative exposure data; neither is estimated here.",
        ],
        rules,
        cases,
    })
}

fn main() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "Usage: cargo run --example evaluate_voice (uses bundled synthetic fixtures only)"
    );
    let report = evaluate(TRACES, LABELS)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    ensure!(
        report.cases.iter().all(|case| case.matches_expected_output),
        "Detector output changed from fixture expectations; inspect the JSON report"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_keep_false_positives_and_misses_visible() {
        let mut counts = Counts::default();
        for pair in [
            (true, true),
            (true, true),
            (false, true),
            (false, false),
            (true, false),
        ] {
            counts.record(pair.0, pair.1);
        }
        assert_eq!(counts.true_positive, 2);
        assert_eq!(counts.false_positive, 1);
        assert_eq!(counts.true_negative, 1);
        assert_eq!(counts.false_negative, 1);
        assert_eq!(counts.precision(), Some(2.0 / 3.0));
        assert_eq!(counts.recall(), Some(2.0 / 3.0));
    }

    #[test]
    fn empty_denominators_are_unknown_not_perfect() {
        let counts = Counts::default();
        assert_eq!(counts.precision(), None);
        assert_eq!(counts.recall(), None);
        let mut counts = counts;
        counts.record(true, false);
        assert_eq!(counts.precision(), None);
        assert_eq!(counts.recall(), Some(0.0));
    }

    #[test]
    fn bundled_evaluation_records_known_false_positives_misses_and_receipt_latency() {
        let report = evaluate(TRACES, LABELS).unwrap();
        assert_eq!(report.cases.len(), 10);
        assert!(report.cases.iter().all(|case| case.matches_expected_output));
        let burst = &report.rules[&Rule::JoinBurst];
        assert_eq!(
            burst.counts,
            Counts {
                true_positive: 2,
                false_positive: 1,
                true_negative: 5,
                false_negative: 2
            }
        );
        assert_eq!(burst.precision, Some(2.0 / 3.0));
        assert_eq!(burst.recall, Some(0.5));
        let reentry = &report.rules[&Rule::RapidReentry];
        assert_eq!(
            reentry.counts,
            Counts {
                true_positive: 2,
                false_positive: 1,
                true_negative: 7,
                false_negative: 0
            }
        );
        assert_eq!(reentry.precision, Some(2.0 / 3.0));
        assert_eq!(reentry.recall, Some(1.0));
        let case = |name| report.cases.iter().find(|case| case.name == name).unwrap();
        assert_eq!(
            case("synthetic disruptive burst").detections[0].receipt_latency_ms,
            Some(4000)
        );
        assert_eq!(
            case("burst with deterministic receipt jitter").detections[0].receipt_latency_ms,
            Some(3875)
        );
        assert_eq!(
            case("reentry with deterministic receipt jitter").detections[0].receipt_latency_ms,
            Some(5125)
        );
        assert_eq!(
            case("benign scheduled group arrival").detections[0].receipt_latency_ms,
            None
        );
        for name in [
            "capture interruption suppresses burst inference",
            "delayed receipt suppresses burst inference",
        ] {
            let result = case(name);
            assert!(result.detections.is_empty());
            assert_eq!(result.gap_count, 1);
            assert_eq!(result.known_missing_transitions, 0);
        }
        let first = serde_json::to_string(&report).unwrap();
        assert_eq!(
            first,
            serde_json::to_string(&evaluate(TRACES, LABELS).unwrap()).unwrap()
        );
        assert!(
            !first.contains("9007199254740993"),
            "Aggregate report must not include participant identities"
        );
    }

    fn edited_labels(edit: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut labels: serde_json::Value = serde_json::from_str(LABELS).unwrap();
        edit(&mut labels);
        labels.to_string()
    }

    #[test]
    fn malformed_labels_fail_instead_of_silently_affecting_scores() {
        for labels in [
            edited_labels(|value| value["schema_version"] = 2.into()),
            edited_labels(|value| value["dataset_version"] = "".into()),
            edited_labels(|value| value["cases"] = serde_json::json!([])),
            edited_labels(|value| value["cases"][0]["trace"] = "missing trace".into()),
            edited_labels(|value| value["cases"][0]["unknown"] = true.into()),
            edited_labels(|value| {
                value["cases"][0]["positive_rules"] = serde_json::json!(["unknown_rule"])
            }),
            edited_labels(|value| value["cases"][0]["onset_ms"] = 0.into()),
            edited_labels(|value| value["cases"][2]["onset_ms"] = serde_json::Value::Null),
            edited_labels(|value| value["cases"][2]["onset_ms"] = 6000.into()),
            edited_labels(|value| value["cases"][2]["onset_ms"] = (-1).into()),
            edited_labels(|value| value["cases"][0]["timing_offsets_ms"] = serde_json::json!([0])),
            edited_labels(|value| {
                value["cases"][0]["timing_offsets_ms"] = serde_json::json!([1, 0])
            }),
            edited_labels(|value| {
                value["cases"][0]["timing_offsets_ms"] = serde_json::json!([0, -6000])
            }),
            edited_labels(|value| value["cases"][0]["capture_gap_steps"] = serde_json::json!([99])),
            edited_labels(|value| {
                value["cases"][2]["positive_rules"] =
                    serde_json::json!(["join_burst", "join_burst"])
            }),
            edited_labels(|value| value["cases"][1]["name"] = value["cases"][0]["name"].clone()),
        ] {
            assert!(
                evaluate(TRACES, &labels).is_err(),
                "Accepted malformed labels: {labels}"
            );
        }
    }

    #[test]
    fn invalid_trace_ids_time_and_retention_are_rejected() {
        let mutate = |edit: fn(&mut serde_json::Value)| {
            let mut traces: serde_json::Value = serde_json::from_str(TRACES).unwrap();
            edit(&mut traces);
            traces.to_string()
        };
        for traces in [
            mutate(|value| value[0]["steps"][0]["ids"] = serde_json::json!([9007199254740993_u64])),
            mutate(|value| value[0]["steps"][0]["ids"] = serde_json::json!(["bad"])),
            mutate(|value| value[0]["steps"][0]["ids"] = serde_json::json!(["1", "1"])),
            mutate(|value| value[0]["steps"][0]["elapsed_ms"] = 1.into()),
            mutate(|value| value[0]["steps"][1]["elapsed_ms"] = 3_600_001.into()),
            mutate(|value| value[0]["steps"] = serde_json::json!([])),
            mutate(|value| value[1]["name"] = value[0]["name"].clone()),
        ] {
            assert!(evaluate(&traces, LABELS).is_err());
        }
        let traces = mutate(|value| {
            value[0]["steps"] = (0..512).map(|index| serde_json::json!({"elapsed_ms":index,"ids":if index % 2 == 0 {vec!["1", "2"]} else {vec![]}})).collect();
        });
        assert!(
            evaluate(&traces, LABELS)
                .err()
                .unwrap()
                .to_string()
                .contains("retention")
        );
        assert!(evaluate(&" ".repeat(1_048_577), LABELS).is_err());
    }

    #[test]
    fn expected_output_is_not_used_as_ground_truth() {
        let labels = edited_labels(|value| {
            value["cases"][3]["expected_detected_rules"] = serde_json::json!([])
        });
        let report = evaluate(TRACES, &labels).unwrap();
        assert!(!report.cases[3].matches_expected_output);
        assert_eq!(report.rules[&Rule::JoinBurst].counts.false_positive, 1);
    }
}
