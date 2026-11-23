mod inject_options;
mod launcher;
mod preferences;
#[cfg(all(target_os = "linux", feature = "observe"))]
mod socket_replay;

use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use intellicord::detection::{Intelligence, ObservationSource};
use intellicord::ingestion::{Producer, SensorBus, SensorMessage, SensorPayload, VoiceFrame};
use intellicord::platform::LinuxReadiness;
use intellicord::{Command, History};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

const SCRIPT: &str = include_str!("../dist/client.js");
const BINDING: &str = "__callmon_send";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    id: String,
    url: String,
    #[serde(rename = "type")]
    kind: String,
    web_socket_debugger_url: Option<String>,
}

struct Cdp {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    sequence: u64,
    pending: VecDeque<Value>,
}

impl Cdp {
    async fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        self.sequence += 1;
        self.socket
            .send(Message::Text(
                json!({
                    "id": self.sequence, "method": method, "params": params
                })
                .to_string()
                .into(),
            ))
            .await?;
        Ok(self.sequence)
    }

    async fn receive(&mut self) -> Result<Value> {
        loop {
            match self.socket.next().await.context("Discord disconnected")?? {
                Message::Text(text) => return Ok(serde_json::from_str(&text)?),
                Message::Close(_) => bail!("Discord closed the debugging connection"),
                _ => {}
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params).await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let value = self.receive().await?;
                if value["id"].as_u64() == Some(id) {
                    check_response(&value)?;
                    return Ok(value["result"].clone());
                }
                self.pending.push_back(value);
            }
        })
        .await
        .context("Discord did not answer within 10 seconds")?
    }

    async fn evaluate(&mut self, expression: &str) -> Result<Value> {
        self.request(
            "Runtime.evaluate",
            json!({
                "expression": expression, "returnByValue": true, "awaitPromise": true
            }),
        )
        .await
    }
}

fn check_response(value: &Value) -> Result<()> {
    if let Some(error) = value.get("error") {
        bail!("DevTools error: {error}");
    }
    if let Some(error) = value["result"].get("exceptionDetails") {
        bail!("Client script failed: {error}");
    }
    Ok(())
}

fn discord_page(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && matches!(
                url.host_str(),
                Some("discord.com" | "canary.discord.com" | "ptb.discord.com")
            )
            && (matches!(url.path(), "/app" | "/quest-home")
                || url.path().starts_with("/channels/"))
    })
}

fn websocket_url(url: &str, port: u16) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "ws"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            && url.port() == Some(port)
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn launch_options(arguments: &[String]) -> Result<(Option<&str>, inject_options::Options)> {
    let (url, options) = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .map_or((None, arguments), |url| {
            (Some(url.as_str()), &arguments[1..])
        });
    let options = inject_options::Options::parse(options)?;
    if options.port != 9222 || options.target.is_some() {
        bail!("Launch uses fixed port 9222 and automatic Discord target selection");
    }
    Ok((url, options))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SensorSessionStatus {
    SensorFree,
    Pending,
    Settled,
    Attention,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SensorCleanupStatus {
    NotApplicable,
    Pending,
    Confirmed,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct SensorSession {
    schema_version: u16,
    source: &'static str,
    status: SensorSessionStatus,
    configured: u8,
    pending: u8,
    finished: u8,
    cleanup: SensorCleanupStatus,
}

impl SensorSession {
    fn read(
        process: intellicord::observe::progress::client::OwnerSnapshot,
        acoustic: intellicord::detection::AcousticOwner,
    ) -> Self {
        use intellicord::{
            detection::{AcousticOwnerOutcome, AcousticOwnerStatus},
            observe::{progress::client::OwnerStatus, supervisor::Outcome},
        };

        let mut configured = 0;
        let mut pending = 0;
        let mut finished = 0;
        let mut attention = false;
        let mut cleanup_confirmed = true;

        match process.status {
            OwnerStatus::NotConfigured => {}
            OwnerStatus::AwaitingCompletion => {
                configured += 1;
                pending += 1;
            }
            OwnerStatus::Unavailable => {
                configured += 1;
                attention = true;
                cleanup_confirmed = false;
            }
            OwnerStatus::Finished => {
                configured += 1;
                finished += 1;
                if let Some(result) = process.result {
                    let clean = result.endpoint_removed
                        && (!result.collector_started || result.collector_reaped);
                    cleanup_confirmed &= clean;
                    attention |= !clean
                        || !matches!(result.outcome, Outcome::Completed | Outcome::Cancelled);
                } else {
                    attention = true;
                    cleanup_confirmed = false;
                }
            }
        }
        match acoustic.status {
            AcousticOwnerStatus::NotConfigured => {}
            AcousticOwnerStatus::Prepared | AcousticOwnerStatus::Running => {
                configured += 1;
                pending += 1;
            }
            AcousticOwnerStatus::Finished => {
                configured += 1;
                finished += 1;
                if let Some(result) = acoustic.result {
                    cleanup_confirmed &= result.cleanup_confirmed;
                    attention |= !result.cleanup_confirmed
                        || result.outcome == AcousticOwnerOutcome::Faulted;
                } else {
                    attention = true;
                    cleanup_confirmed = false;
                }
            }
        }

        let status = if attention {
            SensorSessionStatus::Attention
        } else if pending > 0 {
            SensorSessionStatus::Pending
        } else if configured > 0 {
            SensorSessionStatus::Settled
        } else {
            SensorSessionStatus::SensorFree
        };
        let cleanup = if configured == 0 {
            SensorCleanupStatus::NotApplicable
        } else if pending > 0 {
            SensorCleanupStatus::Pending
        } else if cleanup_confirmed {
            SensorCleanupStatus::Confirmed
        } else {
            SensorCleanupStatus::Unconfirmed
        };
        Self {
            schema_version: 1,
            source: "owner_lifecycle_aggregate",
            status,
            configured,
            pending,
            finished,
            cleanup,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReceiptClock {
    BackendSessionMonotonic,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AcousticFrameClock {
    BackendSessionInterval,
    CaptureRelative,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProcessTiming {
    ReceiptFreshnessOnly,
    Unavailable,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct SessionTiming {
    schema_version: u16,
    source: &'static str,
    voice_receipt_clock: ReceiptClock,
    acoustic_receipt_clock: ReceiptClock,
    acoustic_frame_clock: AcousticFrameClock,
    process_timing: ProcessTiming,
    cross_sensor_correlation: &'static str,
}

impl SessionTiming {
    fn read(
        process: intellicord::observe::progress::client::OwnerSnapshot,
        intelligence: &Intelligence,
    ) -> Self {
        use intellicord::observe::progress::client::OwnerStatus;

        let acoustic_receipt_clock = if intelligence.acoustic.is_some() {
            ReceiptClock::BackendSessionMonotonic
        } else {
            ReceiptClock::Unavailable
        };
        let acoustic_frame_clock = intelligence
            .acoustic
            .as_ref()
            .and_then(|summary| summary.latest.as_ref())
            .map_or(AcousticFrameClock::Unavailable, |frame| {
                if frame.session_start_interval_ns.is_some() {
                    AcousticFrameClock::BackendSessionInterval
                } else {
                    AcousticFrameClock::CaptureRelative
                }
            });
        Self {
            schema_version: 1,
            source: "backend_session_clock_inventory",
            voice_receipt_clock: ReceiptClock::BackendSessionMonotonic,
            acoustic_receipt_clock,
            acoustic_frame_clock,
            process_timing: if process.status == OwnerStatus::NotConfigured {
                ProcessTiming::Unavailable
            } else {
                ProcessTiming::ReceiptFreshnessOnly
            },
            cross_sensor_correlation: "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionEvidenceStatus {
    SensorFree,
    AwaitingEvidence,
    Partial,
    IndependentEvidence,
    Degraded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProcessEvidenceStatus {
    NotConfigured,
    AwaitingRecentSample,
    RecentKernelSample,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AcousticEvidenceStatus {
    NotConfigured,
    AwaitingFrame,
    FrameAvailable,
    ProcessingLoss,
    Faulted,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct SessionEvidence {
    schema_version: u16,
    source: &'static str,
    status: SessionEvidenceStatus,
    owner_integrity: SensorSessionStatus,
    process: ProcessEvidenceStatus,
    acoustic: AcousticEvidenceStatus,
    cross_sensor_correlation: &'static str,
}

impl SessionEvidence {
    fn read(
        owners: SensorSession,
        process: &intellicord::observe::progress::client::Snapshot,
        intelligence: &Intelligence,
    ) -> Self {
        use intellicord::{
            acoustic::SessionStatus, ingestion::AcousticState, observe::progress::client::Status,
        };

        let process = match process.status {
            Status::NotConnected => ProcessEvidenceStatus::NotConfigured,
            Status::NoRecentSample => ProcessEvidenceStatus::AwaitingRecentSample,
            Status::RecentKernelSample => ProcessEvidenceStatus::RecentKernelSample,
            Status::GeneratedRejected | Status::InvalidSample => ProcessEvidenceStatus::Rejected,
        };
        let acoustic = intelligence.acoustic.as_ref().map_or_else(
            || {
                if intelligence.acoustic_owner.status
                    == intellicord::detection::AcousticOwnerStatus::NotConfigured
                {
                    AcousticEvidenceStatus::NotConfigured
                } else {
                    AcousticEvidenceStatus::AwaitingFrame
                }
            },
            |summary| {
                if summary.status == AcousticState::Faulted
                    || summary.processing_health.is_some_and(|health| {
                        matches!(health.status, SessionStatus::Faulted { .. })
                    })
                {
                    AcousticEvidenceStatus::Faulted
                } else if summary
                    .processing_health
                    .is_some_and(|health| health.discarded.frames > 0 || health.overflow_events > 0)
                {
                    AcousticEvidenceStatus::ProcessingLoss
                } else if summary.latest.is_some() {
                    AcousticEvidenceStatus::FrameAvailable
                } else {
                    AcousticEvidenceStatus::AwaitingFrame
                }
            },
        );
        let degraded = owners.status == SensorSessionStatus::Attention
            || process == ProcessEvidenceStatus::Rejected
            || matches!(
                acoustic,
                AcousticEvidenceStatus::ProcessingLoss | AcousticEvidenceStatus::Faulted
            );
        let available = usize::from(process == ProcessEvidenceStatus::RecentKernelSample)
            + usize::from(acoustic == AcousticEvidenceStatus::FrameAvailable);
        let status = if degraded {
            SessionEvidenceStatus::Degraded
        } else if process == ProcessEvidenceStatus::NotConfigured
            && acoustic == AcousticEvidenceStatus::NotConfigured
        {
            SessionEvidenceStatus::SensorFree
        } else if available == 2 {
            SessionEvidenceStatus::IndependentEvidence
        } else if available == 1 {
            SessionEvidenceStatus::Partial
        } else {
            SessionEvidenceStatus::AwaitingEvidence
        };
        Self {
            schema_version: 1,
            source: "backend_session_evidence_aggregate",
            status,
            owner_integrity: owners.status,
            process,
            acoustic,
            cross_sensor_correlation: "unavailable",
        }
    }
}

async fn inject_configured(options: inject_options::Options) -> Result<()> {
    let port = options.port;
    let target_id = options.target.as_deref();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?;
    let targets: Vec<Target> = client.get(format!("http://127.0.0.1:{port}/json/list"))
        .send().await.context("Cannot reach Discord DevTools. Fully quit Discord, then launch it with --remote-debugging-port and a loopback address; see README.md")?
        .error_for_status()?.json().await?;
    let mut matches: Vec<_> = targets
        .into_iter()
        .filter(|target| {
            target.kind == "page"
                && discord_page(&target.url)
                && target_id.is_none_or(|id| target.id == id)
        })
        .collect();
    if matches.len() != 1 {
        for target in &matches {
            eprintln!("Target {}: {}", target.id, target.url);
        }
        bail!(
            "Expected one Discord page, found {}. Log in first, or specify --target ID.",
            matches.len()
        );
    }
    let target = matches.remove(0);
    let url = target
        .web_socket_debugger_url
        .context("Target has no debugging websocket")?;
    if !websocket_url(&url, port) {
        bail!("Refusing non-loopback or unexpected debugging websocket");
    }
    let (socket, _) = tokio_tungstenite::connect_async(url).await?;
    let mut cdp = Cdp {
        socket,
        sequence: 0,
        pending: VecDeque::new(),
    };
    cdp.request("Runtime.enable", json!({})).await?;
    let attached = cdp.evaluate("Boolean(globalThis.__callmon)").await?;
    if attached["result"]["value"] == true {
        bail!(
            "Intellicord is already attached. Stop the existing injector first; reload Discord if its backend exited unexpectedly."
        );
    }
    cdp.request("Runtime.addBinding", json!({ "name": BINDING }))
        .await?;
    if let Err(error) = cdp.evaluate(SCRIPT).await {
        let _ = cdp.evaluate("globalThis.__callmon?.dispose()").await;
        let _ = cdp
            .request("Runtime.removeBinding", json!({ "name": BINDING }))
            .await;
        return Err(error);
    }
    println!(
        "Intellicord attached. Open Callmon at the bottom-left of the call view. Ctrl+C detaches all features."
    );
    let result = track_configured(&mut cdp, options.observation, options.acoustic).await;
    let _ = cdp.evaluate("globalThis.__callmon?.dispose()").await;
    let _ = cdp
        .request("Runtime.removeBinding", json!({ "name": BINDING }))
        .await;
    result
}

async fn publish_view(
    cdp: &mut Cdp,
    history: &History,
    intelligence: &Intelligence,
    progress: Option<&intellicord::observe::progress::supervisor::HealthReceiver>,
    owner: Option<
        &tokio::sync::watch::Receiver<intellicord::observe::progress::client::OwnerSnapshot>,
    >,
) -> Result<()> {
    let mut state = serde_json::to_value(history)?;
    state["intelligence"] = serde_json::to_value(intelligence)?;
    let process_owner = intellicord::observe::progress::client::OwnerSnapshot::read(owner);
    state["intelligence"]["process_owner"] = serde_json::to_value(process_owner)?;
    let process_progress = intellicord::observe::progress::client::Snapshot::read(progress);
    state["intelligence"]["process_progress"] = serde_json::to_value(&process_progress)?;
    let session_sensors = SensorSession::read(process_owner, intelligence.acoustic_owner);
    state["intelligence"]["session_sensors"] = serde_json::to_value(session_sensors)?;
    state["intelligence"]["session_timing"] =
        serde_json::to_value(SessionTiming::read(process_owner, intelligence))?;
    state["intelligence"]["session_evidence"] = serde_json::to_value(SessionEvidence::read(
        session_sensors,
        &process_progress,
        intelligence,
    ))?;
    let expression = format!(
        "globalThis.__callmon?.update({})",
        serde_json::to_string(&state)?
    );
    cdp.send("Runtime.evaluate", json!({"expression": expression}))
        .await?;
    Ok(())
}

#[derive(Default)]
struct DiagnosticChecks {
    sequence: u64,
    last_check: Option<Instant>,
}

impl DiagnosticChecks {
    fn respond(&mut self, value: &Value, session: &str) -> Option<Value> {
        let sequence = value["request_id"].as_u64()?;
        if value["session_id"].as_str() != Some(session)
            || sequence <= self.sequence
            || sequence > 9_007_199_254_740_991
        {
            return None;
        }
        self.sequence = sequence;
        let rate_limited = self
            .last_check
            .is_some_and(|last| last.elapsed() < Duration::from_secs(2));
        let report = if rate_limited {
            None
        } else {
            self.last_check = Some(Instant::now());
            Some(LinuxReadiness::check())
        };
        let process_replay = (!rate_limited).then(|| {
            json!({
                "schema_version":1,"source":"generated_replay","sensor_running":false,
                "health":intellicord::observe::generated_replay(),
                "selection":intellicord::observe::generated_selection_checks(),
                "producer":intellicord::observe::producer::generated_replay(),
                "attachment":intellicord::observe::attachment::generated_replay(),
                "delivery":intellicord::observe::delivery::generated_replay(),
                "progress":intellicord::observe::progress::generated_replay(),
                "record_layout":intellicord::observe::record::generated_replay()
            })
        });
        Some(
            json!({"schema_version":1,"session_id":session,"request_id":sequence,"source":"host",
            "report":report,"process_replay":process_replay,
            "process_object":Value::Null,
            "process_target":(!rate_limited).then(intellicord::observe::target_check),
            "process_clock":(!rate_limited).then(intellicord::observe::clock::check),
            "process_timed":(!rate_limited).then(intellicord::observe::timed::generated_replay),
            "process_lifecycle":(!rate_limited).then(intellicord::observe::lifecycle::generated_lifecycle),
            "process_validation":(!rate_limited).then(intellicord::observe::stream::generated_check),
            "error":if rate_limited { Some("rate_limited") } else { None }}),
        )
    }
}

#[cfg(test)]
mod diagnostic_check_tests {
    use super::*;

    #[test]
    fn stale_replayed_and_malformed_requests_do_not_advance_the_gate() {
        let mut checks = DiagnosticChecks::default();
        for request in [
            json!({"session_id":"stale","request_id":1}),
            json!({"session_id":"current","request_id":0}),
            json!({"session_id":"current","request_id":-1}),
            json!({"session_id":"current","request_id":"1"}),
            json!({"session_id":"current","request_id":9_007_199_254_740_992_u64}),
        ] {
            assert!(checks.respond(&request, "current").is_none());
            assert_eq!(checks.sequence, 0);
            assert!(checks.last_check.is_none());
        }
        let request = json!({"session_id":"current","request_id":1});
        assert!(checks.respond(&request, "current").is_some());
        let checked_at = checks.last_check;
        assert!(checks.respond(&request, "current").is_none());
        assert_eq!(checks.last_check, checked_at);
    }

    #[test]
    fn rate_limit_acknowledges_without_rechecking_and_allows_later_retry() {
        let mut checks = DiagnosticChecks::default();
        let response = checks
            .respond(&json!({"session_id":"current","request_id":1}), "current")
            .unwrap();
        assert_eq!(response["source"], "host");
        assert!(response["error"].is_null());
        assert_eq!(response["report"]["sensor_running"], false);
        assert_eq!(response["report"]["load_test_performed"], false);
        assert_eq!(response["process_replay"]["source"], "generated_replay");
        assert_eq!(response["process_replay"]["sensor_running"], false);
        assert_eq!(response["process_replay"]["health"]["accepted"], 2);
        assert_eq!(
            response["process_replay"]["delivery"],
            serde_json::to_value(intellicord::observe::delivery::generated_replay()).unwrap()
        );
        assert_eq!(
            response["process_replay"]["attachment"],
            serde_json::to_value(intellicord::observe::attachment::generated_replay()).unwrap()
        );
        let selection = response["process_replay"]["selection"].as_array().unwrap();
        assert_eq!(selection.len(), 7);
        assert!(selection.iter().all(|case| case["passed"] == true));
        assert_eq!(
            response["process_replay"]["record_layout"],
            serde_json::to_value(intellicord::observe::record::generated_replay()).unwrap()
        );
        assert_eq!(
            response["process_replay"]["producer"],
            serde_json::to_value(intellicord::observe::producer::generated_replay()).unwrap()
        );
        assert!(response["process_object"].is_null());
        assert_eq!(response["process_target"]["source"], "self_pidfd_probe");
        assert_eq!(
            response["process_clock"]["source"],
            "local_kernel_clock_probe"
        );
        assert_eq!(response["process_clock"]["sensor_running"], false);
        assert_eq!(response["process_timed"]["source"], "generated_replay");
        assert_eq!(response["process_timed"]["sensor_running"], false);
        assert_eq!(response["process_timed"]["health"]["accepted"], 2);
        assert_eq!(
            response["process_timed"]["cases"].as_array().unwrap().len(),
            9
        );
        assert_eq!(response["process_clock"]["load_test_performed"], false);
        assert_eq!(response["process_target"]["target_selected"], false);
        assert_eq!(response["process_target"]["sensor_running"], false);
        assert_eq!(response["process_target"]["load_test_performed"], false);
        assert_eq!(response["process_lifecycle"][5]["phase"], "stopped");
        assert_eq!(response["process_validation"]["source"], "generated_replay");
        assert_eq!(response["process_validation"]["sensor_running"], false);
        assert_eq!(
            response["process_validation"]["cases"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
        assert!(
            response["process_validation"]["cases"]
                .as_array()
                .unwrap()
                .iter()
                .all(|case| case["passed"] == true)
        );
        assert_eq!(
            response["process_lifecycle"][5]["attachment_confirmed"],
            false
        );
        #[cfg(all(target_os = "linux", not(feature = "observe")))]
        assert_eq!(response["process_target"]["status"], "feature_disabled");
        let checked_at = checks.last_check;
        let limited = checks
            .respond(&json!({"session_id":"current","request_id":2}), "current")
            .unwrap();
        assert_eq!(limited["request_id"], 2);
        assert_eq!(limited["error"], "rate_limited");
        assert!(limited["report"].is_null());
        assert!(limited["process_replay"].is_null());
        assert!(limited["process_object"].is_null());
        assert!(limited["process_target"].is_null());
        assert!(limited["process_clock"].is_null());
        assert!(limited["process_timed"].is_null());
        assert!(limited["process_lifecycle"].is_null());
        assert!(limited["process_validation"].is_null());
        assert_eq!(checks.last_check, checked_at);
        checks.last_check = Some(Instant::now() - Duration::from_secs(3));
        let retry = checks
            .respond(&json!({"session_id":"current","request_id":3}), "current")
            .unwrap();
        assert!(retry["error"].is_null());
        assert!(retry["report"].is_object());
    }
}

async fn track_configured(
    cdp: &mut Cdp,
    observation: Option<inject_options::Observation>,
    acoustic: Option<inject_options::Acoustic>,
) -> Result<()> {
    if let Some(inject_options::Observation {
        object,
        process_id,
        seconds,
    }) = observation
    {
        #[cfg(all(target_os = "linux", feature = "observe"))]
        {
            let prepared = (|| {
                const SYSTEM_OBJECT: &str = "/usr/lib/intellicord/intellicord-process";
                if object == std::path::Path::new(SYSTEM_OBJECT) {
                    intellicord::observe::progress::capture::Capture::prepare_privileged(
                        std::path::Path::new("/usr/libexec/intellicord/intellicord-bpf-helper"),
                        &std::env::temp_dir(),
                        process_id,
                        seconds,
                    )
                } else {
                    let collector = std::env::current_exe()?.with_file_name("intellicord-observe");
                    intellicord::observe::progress::capture::Capture::prepare(
                        &collector,
                        &std::env::temp_dir(),
                        &object,
                        process_id,
                        seconds,
                    )
                }
            })();
            let (capture, receiver) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    eprintln!(
                        "Optional process observation setup failed; baseline tracking continues without a collector. No retry or elevation will be attempted."
                    );
                    let (sender, outcome) = tokio::sync::watch::channel(
                        intellicord::observe::progress::client::OwnerSnapshot::pending(),
                    );
                    drop(sender);
                    track(cdp, None, Some(&outcome), acoustic)
                        .await
                        .context("Optional process observation was unavailable during tracking")?;
                    return Err(error.context("Injected observation could not be prepared"));
                }
            };
            eprintln!(
                "Explicit bounded process observation requested; no automatic elevation or restart."
            );
            let outcome = capture.outcome();
            let (result, report) = capture
                .run_with(track(cdp, Some(&receiver), Some(&outcome), acoustic))
                .await;
            eprintln!(
                "Injected observation final report: {}",
                serde_json::to_string(&report)?
            );
            anyhow::ensure!(
                report.endpoint_removed
                    && (!report.run.collector_started || report.run.collector_reaped),
                "Injected observation cleanup was not confirmed"
            );
            anyhow::ensure!(
                matches!(
                    report.run.outcome,
                    intellicord::observe::supervisor::Outcome::Completed
                        | intellicord::observe::supervisor::Outcome::Cancelled
                ),
                "Injected observation did not complete: {:?}",
                report.run.outcome
            );
            return result;
        }
        #[cfg(not(all(target_os = "linux", feature = "observe")))]
        {
            let _ = (object, process_id, seconds);
            bail!("Injected observation requires Linux and the observe feature");
        }
    }
    track(cdp, None, None, acoustic).await
}

fn stop_requested() -> Result<impl std::future::Future<Output = Result<()>>> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        Ok(async move {
            tokio::select! {
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
            Ok(())
        })
    }
    #[cfg(not(unix))]
    {
        Ok(async { Ok(tokio::signal::ctrl_c().await?) })
    }
}

async fn track(
    cdp: &mut Cdp,
    progress: Option<&intellicord::observe::progress::supervisor::HealthReceiver>,
    owner: Option<
        &tokio::sync::watch::Receiver<intellicord::observe::progress::client::OwnerSnapshot>,
    >,
    acoustic: Option<inject_options::Acoustic>,
) -> Result<()> {
    let mut history = History::default();
    let started = Instant::now();
    let session_start = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let mut intelligence = Intelligence::new(format!("{}-{session_start}", std::process::id()));
    let bus = SensorBus::default();
    let publisher = bus.publisher();
    let acoustic_enabled = acoustic.is_some();
    let acoustic_deferred = acoustic.as_ref().is_some_and(|policy| policy.defer_start);
    if acoustic_enabled {
        intelligence.configure_acoustic(acoustic_deferred);
    }
    intelligence.acoustic_start_available = false;
    intelligence.acoustic_stop_available = acoustic_enabled && !acoustic_deferred;
    let feed = intellicord::acoustic::AcousticFeed::new(
        publisher.clone(),
        intelligence.session_id.clone(),
        started,
        intellicord::ingestion::AcousticSource::MixedOutput,
    );
    let (cancel_capture, capture_cancelled) = tokio::sync::oneshot::channel();
    let mut cancel_capture = Some(cancel_capture);
    let (start_capture_sender, start_capture) = tokio::sync::oneshot::channel();
    let mut start_capture_sender = Some(start_capture_sender);
    if acoustic_enabled && !acoustic_deferred {
        let _ = start_capture_sender
            .take()
            .expect("capture start sender")
            .send(());
    }
    let (capture_finished, mut capture_completion) = tokio::sync::oneshot::channel();
    let capture = async move {
        let mut capture_cancelled = capture_cancelled;
        let started = if acoustic.is_some() {
            tokio::select! {
                biased;
                _ = &mut capture_cancelled => false,
                result = start_capture => result.is_ok(),
            }
        } else {
            false
        };
        let result = if let Some(policy) = acoustic.filter(|_| started) {
            eprintln!(
                "Explicit output capture requested: ALL applications on the selected sink may be analyzed; no participant attribution or automatic action."
            );
            #[cfg(unix)]
            {
                let result = intellicord::acoustic_capture::capture_into(
                    policy.serial,
                    policy.seconds,
                    feed,
                    capture_cancelled,
                )
                .await;
                if result.is_err() {
                    eprintln!(
                        "Optional acoustic capture failed; voice tracking continues without retry."
                    );
                }
                result
            }
            #[cfg(not(unix))]
            {
                let _ = (policy, feed, capture_cancelled);
                bail!("Acoustic capture requires Linux");
            }
        } else {
            Ok(())
        };
        let outcome = intellicord::acoustic_capture::capture_outcome(&result);
        let _ = capture_finished.send((started, outcome));
        result
    };
    let mut application_sequence = 0_u64;
    let mut diagnostic_checks = DiagnosticChecks::default();
    let mut diagnostic_jobs: tokio::task::JoinSet<Value> = tokio::task::JoinSet::new();
    let mut preferences =
        preferences::PreferenceStore::new(launcher::state_directory()?.join("preferences.json"));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let stop = stop_requested()?;
    tokio::pin!(stop);
    let mut progress_changes = progress.cloned();
    let mut owner_changes = owner.cloned();
    publish_view(cdp, &history, &intelligence, progress, owner).await?;
    let mut acoustic_tick = tokio::time::interval(Duration::from_millis(200));
    let mut acoustic_pending = acoustic_enabled;
    let mut acoustic_call_id = None;
    acoustic_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let tracking = async {
        let result: Result<()> = async {
        loop {
            let value = if let Some(value) = cdp.pending.pop_front() {
                value
            } else {
                tokio::select! {
                    result = &mut capture_completion, if acoustic_pending => {
                        acoustic_pending = false;
                        intelligence.acoustic_start_available = false;
                        intelligence.acoustic_stop_available = false;
                        intelligence.drain(&bus);
                        let (capture_started, outcome) = result.unwrap_or((false,
                            intellicord::acoustic_capture::CaptureOutcome {
                                completed: false,
                                cleanup_confirmed: false,
                                reason: Some(intellicord::detection::AcousticOwnerReason::InternalFailure),
                            }));
                        intelligence.finish_acoustic_owner(
                            outcome.completed,
                            outcome.cleanup_confirmed,
                            outcome.reason,
                        );
                        if capture_started {
                            intelligence.finish_acoustic(intellicord::ingestion::AcousticSource::MixedOutput,
                                outcome.completed, u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
                        }
                        publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        continue;
                    }
                    _ = acoustic_tick.tick(), if acoustic_pending => {
                        let previous = intelligence.ingestion.clone();
                        let consumed = intelligence.drain(&bus);
                        if consumed > 0 || previous != intelligence.ingestion {
                            publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        }
                        continue;
                    }
                    result = async { progress_changes.as_mut().expect("enabled progress subscription").changed().await }, if progress_changes.is_some() => {
                        if result.is_err() {
                            progress_changes = None;
                        }
                        publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        continue;
                    }
                    result = async { owner_changes.as_mut().expect("enabled owner subscription").changed().await }, if owner_changes.is_some() => {
                        if result.is_err() {
                            owner_changes = None;
                        }
                        publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        continue;
                    }
                    result = diagnostic_jobs.join_next(), if !diagnostic_jobs.is_empty() => {
                        if let Some(Ok(mut response)) = result {
                            response["process_progress"] = serde_json::to_value(
                                intellicord::observe::progress::client::Snapshot::read(progress)
                            ).expect("bounded owner progress handoff");
                            response["process_owner"] = serde_json::to_value(
                                intellicord::observe::progress::client::OwnerSnapshot::read(owner)
                            ).expect("bounded capture owner outcome");
                            let expression = format!("globalThis.__callmon?.diagnostics?.update({response})");
                            cdp.send("Runtime.evaluate", json!({"expression":expression})).await?;
                        }
                        continue;
                    }
                    result = cdp.receive() => result?,
                    _ = heartbeat.tick() => {
                        cdp.send("Runtime.evaluate", json!({"expression": "globalThis.__callmon?.heartbeat()"})).await?;
                        let previous = intelligence.ingestion.clone();
                        let consumed = intelligence.drain(&bus);
                        if consumed > 0 || previous != intelligence.ingestion {
                            publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        }
                        continue;
                    }
                    result = &mut stop => { result?; return Ok(()); }
                }
            };
            check_response(&value)?;
            match value["method"].as_str() {
                Some("Runtime.bindingCalled") if value["params"]["name"] == BINDING => {
                    let payload = value["params"]["payload"]
                        .as_str()
                        .context("Missing binding payload")?;
                    if payload.len() > 2_000_000 {
                        bail!("Client snapshot exceeds size limit");
                    }
                    let value: Value =
                        serde_json::from_str(payload).context("Invalid client command")?;
                    if value["kind"] == "acoustic_start" {
                        let active_call_id = intelligence.active_call_id();
                        if value.as_object().is_some_and(|command| command.len() == 2)
                            && value["session_id"].as_str() == Some(intelligence.session_id.as_str())
                            && acoustic_pending && intelligence.acoustic_start_available
                            && active_call_id.is_some()
                        {
                            intelligence.acoustic_start_available = false;
                            intelligence.acoustic_stop_available = true;
                            intelligence.start_acoustic();
                            acoustic_call_id = active_call_id;
                            if let Some(start) = start_capture_sender.take() {
                                let _ = start.send(());
                            }
                            publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        }
                        continue;
                    }
                    if value["kind"] == "acoustic_stop" {
                        if value.as_object().is_some_and(|command| command.len() == 2)
                            && value["session_id"].as_str() == Some(intelligence.session_id.as_str())
                            && acoustic_pending && intelligence.acoustic_stop_available
                        {
                            intelligence.acoustic_stop_available = false;
                            if let Some(cancel) = cancel_capture.take() {
                                let _ = cancel.send(());
                            }
                            publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        }
                        continue;
                    }
                    if value["kind"] == "diagnostics_check" {
                        if let Some(mut response) =
                            diagnostic_checks.respond(&value, &intelligence.session_id)
                        {
                            if response["error"].is_null() && diagnostic_jobs.is_empty() {
                                diagnostic_jobs.spawn(async move {
                                    let (replay, preflight, handshake, health) = tokio::join!(
                                        intellicord::observe::supervisor::generated_replay(),
                                        intellicord::observe::preflight::check(),
                                        intellicord::observe::handshake::check(),
                                        intellicord::observe::health::check(),
                                    );
                                    response["process_health_snapshot"] =
                                        serde_json::to_value(health)
                                            .expect("bounded supplied health snapshot");
                                    response["process_clock_handshake"] =
                                        serde_json::to_value(handshake)
                                            .expect("bounded clock handshake report");
                                    response["process_object"] =
                                        serde_json::to_value(preflight.object)
                                            .expect("bounded object report");
                                    response["process_preflight"] = serde_json::to_value(preflight)
                                        .expect("bounded preflight report");
                                    response["process_supervision"] = serde_json::to_value(replay)
                                        .expect("bounded supervision report");
                                    response
                                });
                                continue;
                            }
                            if response["error"].is_null() {
                                response["error"] = json!("rate_limited");
                                response["report"] = Value::Null;
                            }
                            let expression =
                                format!("globalThis.__callmon?.diagnostics?.update({response})");
                            cdp.send("Runtime.evaluate", json!({"expression":expression}))
                                .await?;
                        }
                        continue;
                    }
                    if matches!(
                        value["kind"].as_str(),
                        Some("preferences_get" | "preferences_set")
                    ) {
                        if value["kind"] == "preferences_set" {
                            let disabled = value["disabled"]
                                .as_bool()
                                .context("Content preference must be boolean")?;
                            let key = value["key"].as_str().unwrap_or("sponsored");
                            if key == "sponsored" {
                                preferences.set(disabled);
                            } else {
                                let _ = preferences.set_surface(key, disabled);
                            }
                        }
                        let expression = format!(
                            "globalThis.__intellicord?.preferences.update({})",
                            preferences.response()
                        );
                        cdp.send("Runtime.evaluate", json!({"expression":expression}))
                            .await?;
                        continue;
                    }
                    if value["kind"] == "reset_intelligence" {
                        if value["session_id"].as_str() == Some(intelligence.session_id.as_str())
                            && value["reset_count"].as_u64() == Some(intelligence.reset_count)
                        {
                            intelligence.drain(&bus);
                            intelligence.reset_review();
                        }
                        publish_view(cdp, &history, &intelligence, progress, owner).await?;
                        continue;
                    }
                    let source = serde_json::from_value::<ObservationSource>(
                        value["observation_source"].clone(),
                    )
                    .unwrap_or_default();
                    let capture_gap = value["capture_gap"].as_bool().unwrap_or(false);
                    let is_snapshot = value["kind"] == "snapshot";
                    let command: Command =
                        serde_json::from_value(value).context("Invalid client command")?;
                    let timestamp =
                        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
                    history
                        .apply(command, timestamp)
                        .map_err(anyhow::Error::msg)?;
                    if is_snapshot {
                        let monotonic_ns =
                            u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        application_sequence = application_sequence
                            .checked_add(1)
                            .context("Application sequence exhausted")?;
                        let _ = publisher.publish(SensorMessage {
                            schema_version: 1,
                            session_id: intelligence.session_id.clone(),
                            producer: Producer::Application,
                            producer_sequence: application_sequence,
                            monotonic_ns,
                            wall_time_unix_ms: timestamp,
                            payload: SensorPayload::Voice(VoiceFrame::from_history(
                                &history,
                                source,
                                capture_gap,
                            )),
                        });
                        intelligence.drain(&bus);
                        if acoustic_deferred && acoustic_pending {
                            let active_call_id = intelligence.active_call_id();
                            if intelligence.acoustic_owner.status
                                == intellicord::detection::AcousticOwnerStatus::Prepared
                            {
                                intelligence.acoustic_start_available = active_call_id.is_some();
                            } else if acoustic_call_id.is_some()
                                && acoustic_call_id != active_call_id
                            {
                                intelligence.acoustic_stop_available = false;
                                if let Some(cancel) = cancel_capture.take() {
                                    let _ = cancel.send(());
                                }
                            }
                        }
                    }
                    publish_view(cdp, &history, &intelligence, progress, owner).await?;
                }
                Some("Runtime.executionContextsCleared") => {
                    bail!("Discord reloaded. Run the injector again to attach to the new page.")
                }
                _ => {}
            }
        }
        }.await;
        if let Some(cancel) = cancel_capture.take() {
            let _ = cancel.send(());
        }
        result
    };
    let (result, capture_result) = tokio::join!(tracking, capture);
    result?;
    capture_result.context("Injected acoustic capture failed")
}

async fn write_periodic_progress(
    output: impl tokio::io::AsyncWrite + Unpin,
    session: &str,
) -> Result<()> {
    use intellicord::observe::{
        ProcessHealth,
        lifecycle::{Phase, generated_lifecycle},
        progress::reporter::Reporter,
        stream::Source,
    };
    let mut reporter = Reporter::new(session, Source::GeneratedReplay, output)?;
    for status in generated_lifecycle().into_iter().take(4) {
        reporter
            .emit(status.phase, status.reason, status.health)
            .await?;
    }
    let mut accepted = 1;
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while accepted <= 2 {
        tick.tick().await;
        if reporter
            .periodic_health(ProcessHealth {
                schema_version: 1,
                accepted,
                ..ProcessHealth::default()
            })
            .await?
        {
            accepted += 1;
        }
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    for status in generated_lifecycle().into_iter().skip(4) {
        reporter
            .emit(status.phase, status.reason, status.health)
            .await?;
    }
    let summary = reporter.finish(true)?;
    anyhow::ensure!(
        summary.lifecycle.terminal_phase == Phase::Stopped,
        "generated replay did not stop"
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if matches!(
        args.first().map(String::as_str),
        Some("supervise-socket-progress-replay" | "socket-progress-replay-child")
    ) {
        anyhow::ensure!(args.len() == 1, "Socket progress replay accepts no options");
        #[cfg(all(target_os = "linux", feature = "observe"))]
        {
            return if args[0] == "supervise-socket-progress-replay" {
                socket_replay::supervise().await
            } else {
                socket_replay::child().await
            };
        }
        #[cfg(not(all(target_os = "linux", feature = "observe")))]
        bail!("Socket progress replay requires Linux and the observe feature");
    }
    if args.first().map(String::as_str) == Some("supervise-periodic-progress-replay") {
        if args.len() != 1 {
            bail!("Usage: intellicord supervise-periodic-progress-replay");
        }
        use intellicord::observe::{
            progress::supervisor::{health_channel, supervise_with_health},
            stream::Source,
            supervisor::Outcome,
        };
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command.arg("periodic-progress-replay-child");
        let (sender, mut receiver) = health_channel();
        let retained = receiver.clone();
        let owner = supervise_with_health(
            command,
            "00000000000000000000000000000005",
            Source::GeneratedReplay,
            Duration::from_secs(6),
            Duration::from_secs(3),
            async {
                let _ = tokio::signal::ctrl_c().await;
            },
            sender,
        );
        let consumer = async move {
            let mut observed = false;
            let mut generated_only = true;
            while receiver.changed().await.is_ok() {
                if let Some(sample) = receiver.latest(Duration::from_secs(2)) {
                    observed = true;
                    generated_only &= sample.source == Source::GeneratedReplay;
                }
            }
            (observed, generated_only)
        };
        let (run, (observed_health, generated_only)) = tokio::join!(owner, consumer);
        let revoked = retained.latest(Duration::MAX).is_none();
        println!(
            "{}",
            json!({"schema_version":1, "source":"generated_replay",
            "sensor_running":false, "load_test_performed":false,
            "observed_health":observed_health, "generated_only":generated_only,
            "health_revoked":revoked, "run":run})
        );
        if run.outcome != Outcome::Completed || !observed_health || !generated_only || !revoked {
            bail!("Generated periodic progress supervision did not pass");
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("periodic-progress-replay-child") {
        if args.len() != 1 {
            bail!("Usage: intellicord periodic-progress-replay-child");
        }
        return write_periodic_progress(tokio::io::stdout(), "00000000000000000000000000000005")
            .await;
    }
    if args.first().map(String::as_str) == Some("supervise-progress-replay") {
        if args.len() != 1 {
            bail!("Usage: intellicord supervise-progress-replay");
        }
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command.arg("progress-replay-child");
        let run = intellicord::observe::progress::supervisor::supervise(
            command,
            "00000000000000000000000000000001",
            intellicord::observe::stream::Source::GeneratedReplay,
            Duration::from_secs(3),
            Duration::from_secs(2),
            async {
                let _ = tokio::signal::ctrl_c().await;
            },
        )
        .await;
        println!(
            "{}",
            serde_json::to_string(&json!({"schema_version":1,
            "source":"generated_replay", "sensor_running":false, "load_test_performed":false,
            "run":run}))?
        );
        if run.outcome != intellicord::observe::supervisor::Outcome::Completed {
            bail!("Generated progress supervision did not complete");
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("replay-process-progress") {
        if args.len() != 1 {
            bail!("Usage: intellicord replay-process-progress");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::progress::generated_replay())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("progress-replay-child") {
        if args.len() != 1 {
            bail!("Usage: intellicord progress-replay-child");
        }
        use intellicord::observe::{
            ProcessHealth,
            lifecycle::{Phase, generated_lifecycle},
            progress::writer::Writer,
            stream::Source,
        };
        let mut writer = Writer::new(
            "00000000000000000000000000000001",
            Source::GeneratedReplay,
            tokio::io::stdout(),
        )?;
        for status in generated_lifecycle() {
            writer.lifecycle(status).await?;
            if status.phase == Phase::Observing {
                for accepted in [1, 2] {
                    writer
                        .health(ProcessHealth {
                            schema_version: 1,
                            accepted,
                            ..ProcessHealth::default()
                        })
                        .await?;
                }
            }
        }
        writer.finish(true)?;
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("health-snapshot-inspect") {
        if args.len() != 1 {
            bail!("Usage: intellicord health-snapshot-inspect");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::health::read_configured_report())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("check-health-snapshot") {
        if args.len() != 1 {
            bail!("Usage: intellicord check-health-snapshot");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::health::check().await)?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("replay-process-delivery") {
        if args.len() != 1 {
            bail!("Usage: intellicord replay-process-delivery");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::delivery::generated_replay())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("replay-process-attachment") {
        if args.len() != 1 {
            bail!("Usage: intellicord replay-process-attachment");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::attachment::generated_replay())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("replay-process-record") {
        if args.len() != 1 {
            bail!("Usage: intellicord replay-process-record");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::record::generated_replay())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("replay-process-producer") {
        if args.len() != 1 {
            bail!("Usage: intellicord replay-process-producer");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::producer::generated_replay())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("replay-process-selection") {
        if args.len() != 1 {
            bail!("Usage: intellicord replay-process-selection");
        }
        println!(
            "{}",
            json!({
                "schema_version":1,"source":"generated_replay",
                "sensor_running":false,"load_test_performed":false,
                "selection":intellicord::observe::generated_selection_checks()
            })
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("probe-clock-handshake") {
        if args.len() != 1 {
            bail!("Usage: intellicord probe-clock-handshake");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::handshake::check().await)?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("clock-handshake-reply") {
        if args.len() != 1 {
            bail!("Usage: intellicord clock-handshake-reply");
        }
        intellicord::observe::handshake::reply_bounded(std::future::pending()).await?;
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("replay-timed-process") {
        if args.len() != 1 {
            bail!("Usage: intellicord replay-timed-process");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::timed::generated_replay())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("probe-clock") {
        if args.len() != 1 {
            bail!("Usage: intellicord probe-clock");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::clock::check())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("object-preflight") {
        if args.len() != 1 {
            bail!("Usage: intellicord object-preflight");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::object_check())?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("check-object") {
        if args.len() != 1 {
            bail!("Usage: intellicord check-object");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::preflight::check().await)?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("process-replay") {
        if args != ["process-replay", "--status-json"] {
            bail!("Usage: intellicord process-replay --status-json");
        }
        use std::io::Write;
        let mut output = std::io::stderr().lock();
        for record in intellicord::observe::lifecycle::generated_lifecycle() {
            writeln!(output, "{}", serde_json::to_string(&record)?)?;
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("process-collection-replay") {
        if args != ["process-collection-replay"] {
            bail!("Usage: intellicord process-collection-replay");
        }
        intellicord::observe::collection::write_generated()?;
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("process-negotiated-replay") {
        if args.len() != 1 {
            bail!("Usage: intellicord process-negotiated-replay");
        }
        return intellicord::observe::collection::write_negotiated_generated().await;
    }
    if args.first().map(String::as_str) == Some("supervise-replay") {
        if args.len() != 1 {
            bail!("Usage: intellicord supervise-replay");
        }
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::supervisor::generated_replay().await)?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("diagnostics") {
        if args.len() != 1 {
            bail!("Usage: intellicord diagnostics");
        }
        println!("{}", serde_json::to_string(&LinuxReadiness::check())?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("launch") {
        let (url, options) = launch_options(&args[1..])?;
        return launcher::launch(url, options).await;
    }
    if args.first().map(String::as_str) != Some("inject") {
        println!(
            "Intellicord 0.1.0\n\nUsage: intellicord launch [discord://URL] [explicit sensor options]\n       intellicord inject [--port 9222] [--target ID] [explicit sensor options]\n       intellicord diagnostics\n\nOptional Linux/observe injection requires all of:\n  --observe-object PATH --observe-pid PID --observe-seconds 1..60\n  --acknowledge-injected-observation\nSee docs/process-observation.md before deliberate use.\n\nLaunch starts Discord with loopback-only DevTools and attaches automatically. Without explicit sensor options it includes only Callmon voice-call history.\nDiagnostics checks Linux prerequisites without attachment or starting sensors.\nUI preview: open preview.html. See README.md for compatibility and privacy notes."
        );
        println!(
            "\nOptional Linux output analysis requires all of:\n  --acoustic-sink-serial SERIAL --acoustic-seconds 1..60\n  --acknowledge-injected-output-capture\nAdd --defer-acoustic-start for active-call confirmation and stop-on-call-end.\nALL applications on that output may be analyzed. No participant attribution.\nSee docs/acoustic-capture.md before deliberate use."
        );
        return Ok(());
    }
    let options = inject_options::Options::parse(&args[1..])?;
    inject_configured(options).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process_owner(
        status: intellicord::observe::progress::client::OwnerStatus,
        result: Option<intellicord::observe::progress::client::OwnerResult>,
    ) -> intellicord::observe::progress::client::OwnerSnapshot {
        intellicord::observe::progress::client::OwnerSnapshot {
            schema_version: 1,
            source: "capture_owner_outcome",
            status,
            result,
        }
    }

    #[test]
    fn sensor_session_aggregates_owner_lifecycle_without_claiming_liveness() {
        use intellicord::{
            detection::{
                AcousticOwner, AcousticOwnerOutcome, AcousticOwnerResult, AcousticOwnerStatus,
            },
            observe::{
                progress::client::{OwnerResult, OwnerStatus},
                supervisor::Outcome,
            },
        };

        let acoustic = AcousticOwner {
            schema_version: 1,
            status: AcousticOwnerStatus::NotConfigured,
            call_bound: false,
            result: None,
        };
        let sensor_free =
            SensorSession::read(process_owner(OwnerStatus::NotConfigured, None), acoustic);
        assert_eq!(sensor_free.status, SensorSessionStatus::SensorFree);
        assert_eq!(sensor_free.cleanup, SensorCleanupStatus::NotApplicable);
        assert_eq!(
            (
                sensor_free.configured,
                sensor_free.pending,
                sensor_free.finished
            ),
            (0, 0, 0)
        );

        let pending = SensorSession::read(
            process_owner(OwnerStatus::AwaitingCompletion, None),
            AcousticOwner {
                status: AcousticOwnerStatus::Prepared,
                call_bound: true,
                ..acoustic
            },
        );
        assert_eq!(pending.status, SensorSessionStatus::Pending);
        assert_eq!(pending.cleanup, SensorCleanupStatus::Pending);
        assert_eq!(
            (pending.configured, pending.pending, pending.finished),
            (2, 2, 0)
        );

        let process_result = OwnerResult {
            outcome: Outcome::Completed,
            collector_started: true,
            collector_reaped: true,
            endpoint_removed: true,
        };
        let settled = SensorSession::read(
            process_owner(OwnerStatus::Finished, Some(process_result)),
            AcousticOwner {
                status: AcousticOwnerStatus::Finished,
                result: Some(AcousticOwnerResult {
                    outcome: AcousticOwnerOutcome::Completed,
                    cleanup_confirmed: true,
                    reason: None,
                }),
                ..acoustic
            },
        );
        assert_eq!(settled.status, SensorSessionStatus::Settled);
        assert_eq!(settled.cleanup, SensorCleanupStatus::Confirmed);
        assert_eq!(
            (settled.configured, settled.pending, settled.finished),
            (2, 0, 2)
        );

        for attention in [
            SensorSession::read(process_owner(OwnerStatus::Unavailable, None), acoustic),
            SensorSession::read(
                process_owner(
                    OwnerStatus::Finished,
                    Some(OwnerResult {
                        endpoint_removed: false,
                        ..process_result
                    }),
                ),
                acoustic,
            ),
            SensorSession::read(
                process_owner(OwnerStatus::NotConfigured, None),
                AcousticOwner {
                    status: AcousticOwnerStatus::Finished,
                    result: Some(AcousticOwnerResult {
                        outcome: AcousticOwnerOutcome::Faulted,
                        cleanup_confirmed: true,
                        reason: Some(intellicord::detection::AcousticOwnerReason::RecorderExited),
                    }),
                    ..acoustic
                },
            ),
        ] {
            assert_eq!(attention.status, SensorSessionStatus::Attention);
        }
    }

    #[test]
    fn session_timing_classifies_existing_clock_domains_without_correlation_claims() {
        use intellicord::{
            detection::AcousticSummary,
            ingestion::{AcousticFrame, AcousticSessionInterval, AcousticSource, AcousticState},
            observe::progress::client::OwnerStatus,
        };

        let process = process_owner(OwnerStatus::NotConfigured, None);
        let mut intelligence = Intelligence::new("session".into());
        let empty = SessionTiming::read(process, &intelligence);
        assert_eq!(
            empty.voice_receipt_clock,
            ReceiptClock::BackendSessionMonotonic
        );
        assert_eq!(empty.acoustic_receipt_clock, ReceiptClock::Unavailable);
        assert_eq!(empty.acoustic_frame_clock, AcousticFrameClock::Unavailable);
        assert_eq!(empty.process_timing, ProcessTiming::Unavailable);
        assert_eq!(empty.cross_sensor_correlation, "unavailable");

        intelligence.acoustic = Some(AcousticSummary {
            schema_version: 3,
            source: AcousticSource::MixedOutput,
            status: AcousticState::Receiving,
            frames: 1,
            discontinuities: 0,
            missing_messages: 0,
            last_receipt_ns: 30,
            latest: Some(AcousticFrame {
                source: AcousticSource::MixedOutput,
                frame_start_ns: 0,
                session_start_interval_ns: Some(AcousticSessionInterval {
                    earliest_ns: 10,
                    latest_ns: 20,
                }),
                rms: 0.1,
                peak: 0.2,
                clipped_sample_ratio: 0.0,
                discontinuity: false,
                analysis: None,
            }),
            processing_health: None,
        });
        let mapped = SessionTiming::read(
            process_owner(OwnerStatus::AwaitingCompletion, None),
            &intelligence,
        );
        assert_eq!(
            mapped.acoustic_receipt_clock,
            ReceiptClock::BackendSessionMonotonic
        );
        assert_eq!(
            mapped.acoustic_frame_clock,
            AcousticFrameClock::BackendSessionInterval
        );
        assert_eq!(mapped.process_timing, ProcessTiming::ReceiptFreshnessOnly);
        assert_eq!(
            serde_json::to_value(mapped)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            7
        );

        intelligence
            .acoustic
            .as_mut()
            .unwrap()
            .latest
            .as_mut()
            .unwrap()
            .session_start_interval_ns = None;
        assert_eq!(
            SessionTiming::read(process, &intelligence).acoustic_frame_clock,
            AcousticFrameClock::CaptureRelative
        );
    }

    #[test]
    fn session_evidence_combines_independent_streams_and_surfaces_processing_loss() {
        use intellicord::{
            acoustic::{OutputCounts, OutputHealth, SessionStatus},
            detection::{AcousticOwner, AcousticOwnerStatus, AcousticSummary},
            ingestion::{AcousticFrame, AcousticSource, AcousticState},
            observe::progress::client::{Snapshot, Status},
        };

        let process = Snapshot {
            schema_version: 1,
            source: "owner_progress_subscription",
            status: Status::RecentKernelSample,
            sensor_running: false,
            valid_for_ms: 1_000,
            sample: None,
        };
        let mut intelligence = Intelligence::new("session".into());
        intelligence.acoustic_owner = AcousticOwner {
            schema_version: 1,
            status: AcousticOwnerStatus::Running,
            call_bound: true,
            result: None,
        };
        intelligence.acoustic = Some(AcousticSummary {
            schema_version: 4,
            source: AcousticSource::MixedOutput,
            status: AcousticState::Receiving,
            frames: 1,
            discontinuities: 0,
            missing_messages: 0,
            last_receipt_ns: 30,
            latest: Some(AcousticFrame {
                source: AcousticSource::MixedOutput,
                frame_start_ns: 0,
                session_start_interval_ns: None,
                rms: 0.1,
                peak: 0.2,
                clipped_sample_ratio: 0.0,
                discontinuity: false,
                analysis: None,
            }),
            processing_health: None,
        });
        let owners = SensorSession::read(
            process_owner(
                intellicord::observe::progress::client::OwnerStatus::AwaitingCompletion,
                None,
            ),
            intelligence.acoustic_owner,
        );
        let available = SessionEvidence::read(owners, &process, &intelligence);
        assert_eq!(available.status, SessionEvidenceStatus::IndependentEvidence);
        assert_eq!(available.process, ProcessEvidenceStatus::RecentKernelSample);
        assert_eq!(available.acoustic, AcousticEvidenceStatus::FrameAvailable);
        assert_eq!(available.cross_sensor_correlation, "unavailable");

        intelligence.acoustic.as_mut().unwrap().processing_health = Some(OutputHealth {
            schema_version: 1,
            status: SessionStatus::Active,
            capacity_batches: 8,
            pending: OutputCounts::default(),
            processed: OutputCounts {
                batches: 2,
                frames: 2,
            },
            delivered: OutputCounts {
                batches: 1,
                frames: 1,
            },
            discarded: OutputCounts {
                batches: 1,
                frames: 1,
            },
            overflow_events: 1,
            high_water_batches: 8,
        });
        let degraded = SessionEvidence::read(owners, &process, &intelligence);
        assert_eq!(degraded.status, SessionEvidenceStatus::Degraded);
        assert_eq!(degraded.acoustic, AcousticEvidenceStatus::ProcessingLoss);
        assert_eq!(
            serde_json::to_value(degraded)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            7
        );
    }

    fn launch_arguments(arguments: &[&str]) -> Result<(Option<String>, inject_options::Options)> {
        let arguments = arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect::<Vec<_>>();
        launch_options(&arguments).map(|(url, options)| (url.map(str::to_owned), options))
    }

    #[test]
    fn launch_reuses_explicit_sensor_policy_without_changing_desktop_defaults() {
        let (url, ordinary) = launch_arguments(&[]).unwrap();
        assert!(url.is_none());
        assert!(ordinary.observation.is_none());
        assert!(ordinary.acoustic.is_none());

        let (url, configured) = launch_arguments(&[
            "discord://-/channels/@me",
            "--acoustic-sink-serial",
            "42",
            "--acoustic-seconds",
            "1",
            "--acknowledge-injected-output-capture",
            "--defer-acoustic-start",
        ])
        .unwrap();
        assert_eq!(url.as_deref(), Some("discord://-/channels/@me"));
        let acoustic = configured.acoustic.unwrap();
        assert_eq!(acoustic.serial, 42);
        assert_eq!(acoustic.seconds, 1);
        assert!(acoustic.defer_start);

        for invalid in [
            vec!["--port", "9223"],
            vec!["--target", "page"],
            vec!["--acoustic-sink-serial", "42"],
            vec!["discord://-/channels/@me", "extra"],
        ] {
            assert!(launch_arguments(&invalid).is_err());
        }
    }

    #[test]
    fn only_explicit_discord_and_loopback_targets_are_allowed() {
        assert!(discord_page("https://discord.com/channels/@me"));
        assert!(discord_page("https://canary.discord.com/app"));
        assert!(discord_page("https://discord.com/quest-home"));
        assert!(discord_page("https://ptb.discord.com/quest-home"));
        assert!(!discord_page("http://discord.com/quest-home"));
        assert!(!discord_page(
            "https://discord.com.attacker.test/quest-home"
        ));
        assert!(!discord_page("https://discord.com/quest-home/unknown"));
        assert!(!discord_page(
            "https://discord.com.attacker.test/channels/@me"
        ));
        assert!(!discord_page("https://discord.com/login"));
        assert!(websocket_url("ws://127.0.0.1:9222/devtools/page/1", 9222));
        assert!(!websocket_url(
            "ws://192.168.1.2:9222/devtools/page/1",
            9222
        ));
        assert!(!websocket_url("ws://127.0.0.1:9223/devtools/page/1", 9222));
    }

    #[test]
    fn script_exceptions_are_not_treated_as_success() {
        assert!(
            check_response(&json!({"result": {"exceptionDetails": {"text": "Error"}}})).is_err()
        );
        assert!(check_response(&json!({"error": {"message": "Missing method"}})).is_err());
        assert!(check_response(&json!({"result": {"result": {"value": true}}})).is_ok());
    }

    #[tokio::test]
    async fn transport_preserves_early_snapshots_and_emits_history() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            let Some(Ok(Message::Text(request))) = socket.next().await else {
                bail!("missing bootstrap");
            };
            let request: Value = serde_json::from_str(&request)?;
            let snapshot = |channel_id: &str, ids: &[&str]| {
                json!({
                    "method": "Runtime.bindingCalled", "params": { "name": BINDING,
                        "payload": json!({"kind":"snapshot", "observation_source":"store_notification", "capture_gap":ids.is_empty(), "channel":{"id":channel_id, "name":"General"},
                            "users": ids.iter().map(|id| json!({"id":id,"username":"morgan","display_name":"Morgan"})).collect::<Vec<_>>()
                        }).to_string()
                    }
                })
            };
            socket
                .send(Message::Text(
                    snapshot("100", &["9007199254740993"]).to_string().into(),
                ))
                .await?;
            socket
                .send(Message::Text(
                    json!({"id":request["id"], "result": {}}).to_string().into(),
                ))
                .await?;
            let mut views = Vec::new();
            let mut initial_seen = false;
            while views.len() < 7 {
                let Some(Ok(Message::Text(text))) = socket.next().await else {
                    bail!("missing view");
                };
                let request: Value = serde_json::from_str(&text)?;
                let expression = request["params"]["expression"].as_str().unwrap_or_default();
                if let Some(view) = expression
                    .strip_prefix("globalThis.__callmon?.update(")
                    .and_then(|value| value.strip_suffix(')'))
                {
                    let view = serde_json::from_str::<Value>(view)?;
                    if !initial_seen {
                        assert!(view["channel"].is_null());
                        assert!(view["events"].as_array().unwrap().is_empty());
                        assert_eq!(
                            view["intelligence"]["process_owner"]["status"],
                            "not_configured"
                        );
                        initial_seen = true;
                        socket
                            .send(Message::Text(
                                json!({"id":request["id"], "result":{}}).to_string().into(),
                            ))
                            .await?;
                        continue;
                    }
                    views.push(view);
                    if views.len() == 1 {
                        assert_eq!(views[0]["intelligence"]["acoustic_stop_available"], false);
                        socket.send(Message::Text(json!({"method":"Runtime.bindingCalled","params":{
                            "name":BINDING,"payload":json!({"kind":"acoustic_stop","session_id":views[0]["intelligence"]["session_id"]}).to_string()
                        }}).to_string().into())).await?;
                        socket
                            .send(Message::Text(snapshot("100", &[]).to_string().into()))
                            .await?;
                    } else if views.len() == 2 {
                        socket
                            .send(Message::Text(snapshot("200", &["2"]).to_string().into()))
                            .await?;
                    } else if views.len() == 3 {
                        socket
                            .send(Message::Text(
                                json!({"method":"Runtime.bindingCalled", "params":{
                                    "name":BINDING,"payload":"{\"kind\":\"clear\"}"
                                }})
                                .to_string()
                                .into(),
                            ))
                            .await?;
                    } else if (4..=6).contains(&views.len()) {
                        let view = views.last().unwrap();
                        let session = if views.len() == 4 {
                            "stale-session"
                        } else {
                            view["intelligence"]["session_id"].as_str().unwrap()
                        };
                        socket.send(Message::Text(json!({"method":"Runtime.bindingCalled", "params":{
                            "name":BINDING,"payload":json!({"kind":"reset_intelligence", "session_id":session,"reset_count":0}).to_string()
                        }}).to_string().into())).await?;
                    }
                }
                socket
                    .send(Message::Text(
                        json!({"id":request["id"], "result":{}}).to_string().into(),
                    ))
                    .await?;
            }
            socket
                .send(Message::Text(
                    json!({"method":"Runtime.executionContextsCleared"})
                        .to_string()
                        .into(),
                ))
                .await?;
            Ok::<_, anyhow::Error>(views)
        });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}")).await?;
        let mut cdp = Cdp {
            socket,
            sequence: 0,
            pending: VecDeque::new(),
        };
        cdp.request("Runtime.evaluate", json!({"expression":"bootstrap"}))
            .await?;
        assert_eq!(cdp.pending.len(), 1);
        let result =
            tokio::time::timeout(Duration::from_secs(5), track(&mut cdp, None, None, None)).await?;
        assert!(result.unwrap_err().to_string().contains("reloaded"));
        let views = server.await??;
        assert!(views[0]["events"].as_array().unwrap().is_empty());
        assert!(
            views
                .iter()
                .all(|view| view["intelligence"]["acoustic"].is_null()
                    && view["intelligence"]["acoustic_stop_available"] == false)
        );
        assert_eq!(views[1]["events"][0]["action"], "left");
        assert_eq!(views[1]["events"][0]["user"]["id"], "9007199254740993");
        assert!(views[2]["events"].as_array().unwrap().is_empty());
        assert_eq!(views[2]["current_call"]["channel"]["id"], "200");
        assert_eq!(views[2]["participants"]["2"]["id"], "2");
        assert_eq!(views[2]["archives"].as_array().unwrap().len(), 1);
        assert_eq!(views[2]["archives"][0]["channel"]["id"], "100");
        assert!(views[2]["archives"][0]["ended_at"].is_u64());
        assert_eq!(views[2]["archives"][0]["events"], views[1]["events"]);
        assert!(views[3]["events"].as_array().unwrap().is_empty());
        assert_eq!(views[3]["archives"], views[2]["archives"]);
        assert_eq!(views[0]["intelligence"]["schema_version"], 1);
        assert_eq!(views[0]["intelligence"]["ingestion"]["queue_capacity"], 64);
        assert_eq!(views[0]["intelligence"]["ingestion"]["accepted"], 1);
        assert_eq!(views[2]["intelligence"]["ingestion"]["accepted"], 3);
        assert_eq!(views[2]["intelligence"]["ingestion"]["dropped_full"], 0);
        assert!(views[2]["intelligence"]["ingestion"]["diagnostics"].is_null());
        assert_eq!(
            views[0]["intelligence"]["observations"][0]["kind"]["type"],
            "baseline"
        );
        assert_eq!(views[1]["intelligence"]["gap_count"], 1);
        let observations = views[1]["intelligence"]["observations"].as_array().unwrap();
        let departure = observations.last().unwrap();
        assert_eq!(departure["kind"]["type"], "left");
        assert_eq!(departure["source"], "store_notification");
        assert_eq!(departure["subject"]["user_id"], "9007199254740993");
        assert_eq!(views[3]["intelligence"], views[2]["intelligence"]);
        assert_eq!(views[4], views[3], "stale sessions must not reset review");
        assert_eq!(views[5]["intelligence"]["reset_count"], 1);
        assert_eq!(views[5]["intelligence"]["observations"], json!([]));
        assert_eq!(views[5]["intelligence"]["incidents"], json!([]));
        assert_eq!(views[5]["intelligence"]["gap_count"], 0);
        assert_eq!(
            views[5]["intelligence"]["ingestion"],
            views[4]["intelligence"]["ingestion"]
        );
        let mut reset_view = views[5].clone();
        reset_view["intelligence"] = views[4]["intelligence"].clone();
        assert_eq!(
            reset_view, views[4],
            "reset must not change Callmon history"
        );
        assert_eq!(views[6], views[5], "replayed reset must be inert");
        Ok(())
    }
}
