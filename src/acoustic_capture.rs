use std::{future::Future, process::Stdio, time::Duration};

use crate::acoustic::{
    BufferedAcousticSession, Format, OutputHealth, PcmEncoding, ProducerFailure, SessionPermit,
    SessionStatus,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    time::{Instant, timeout},
};

#[path = "acoustic_capture/status.rs"]
mod status;
use crate::detection::AcousticOwnerReason;
use status::{CapturePhase, CaptureReason, StatusReporter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureOutcome {
    pub completed: bool,
    pub cleanup_confirmed: bool,
    pub reason: Option<AcousticOwnerReason>,
}

pub fn capture_outcome(result: &Result<()>) -> CaptureOutcome {
    let reason = result
        .as_ref()
        .err()
        .map(|error| match CaptureReason::from_error(error) {
            CaptureReason::DiscoveryFailed => AcousticOwnerReason::DiscoveryFailed,
            CaptureReason::SignalSetupFailed => AcousticOwnerReason::SignalSetupFailed,
            CaptureReason::SourceRejected => AcousticOwnerReason::SourceRejected,
            CaptureReason::RecorderStartFailed => AcousticOwnerReason::RecorderStartFailed,
            CaptureReason::InputStalled => AcousticOwnerReason::InputStalled,
            CaptureReason::StreamEnded => AcousticOwnerReason::StreamEnded,
            CaptureReason::StreamReadFailed => AcousticOwnerReason::StreamReadFailed,
            CaptureReason::ProcessingFailed => AcousticOwnerReason::ProcessingFailed,
            CaptureReason::DeliveryFailed => AcousticOwnerReason::DeliveryFailed,
            CaptureReason::RecorderExited => AcousticOwnerReason::RecorderExited,
            CaptureReason::CleanupFailed => AcousticOwnerReason::CleanupFailed,
            CaptureReason::DurationLimit
            | CaptureReason::Interrupt
            | CaptureReason::Terminate
            | CaptureReason::OwnerCancelled
            | CaptureReason::InternalFailure => AcousticOwnerReason::InternalFailure,
        });
    CaptureOutcome {
        completed: result.is_ok(),
        cleanup_confirmed: result.as_ref().err().is_none_or(|error| {
            error.downcast_ref::<CaptureReason>() != Some(&CaptureReason::CleanupFailed)
        }),
        reason,
    }
}

const FRAME_BYTES: usize = 960 * 2 * 4;
const METADATA_LIMIT: u64 = 4 * 1024 * 1024;
const HELP: &str = "Opt-in PipeWire mixed-output analysis (Linux)\n\
Usage:\n  intellicord-acoustic sources\n  \
intellicord-acoustic capture --sink-serial SERIAL --acknowledge-output-capture --seconds 1..60 [--status-json]\n\
The selected output sink can include ALL applications, not just Discord.\n\
No microphone/default-source selection, raw-audio files, attribution or automatic action.\n\
Foreground only. Ctrl+C or SIGTERM stops capture; every run requires explicit consent.";

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    Sources,
    Capture {
        serial: u64,
        seconds: u64,
        status_json: bool,
    },
}

fn parse(arguments: &[String]) -> Result<Request> {
    if arguments == ["--help"] || arguments == ["-h"] {
        return Ok(Request::Help);
    }
    if arguments == ["sources"] {
        return Ok(Request::Sources);
    }
    ensure!(
        arguments
            .first()
            .is_some_and(|argument| argument == "capture"),
        "an explicit subcommand is required; use --help"
    );
    let mut serial = None;
    let mut seconds = None;
    let mut consent = false;
    let mut status_json = false;
    let mut arguments = arguments[1..].iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--sink-serial" if serial.is_none() => {
                serial = Some(
                    arguments
                        .next()
                        .context("missing sink serial")?
                        .parse::<u64>()?,
                );
            }
            "--seconds" if seconds.is_none() => {
                seconds = Some(
                    arguments
                        .next()
                        .context("missing duration")?
                        .parse::<u64>()?,
                );
            }
            "--acknowledge-output-capture" if !consent => consent = true,
            "--status-json" if !status_json => status_json = true,
            _ => bail!("unknown or repeated capture option; use --help"),
        }
    }
    ensure!(consent, "explicit --acknowledge-output-capture is required");
    let serial = serial.context("explicit --sink-serial is required")?;
    ensure!(serial > 0, "automatic/unlinked targets are not allowed");
    let seconds = seconds.context("explicit --seconds is required")?;
    ensure!(
        (1..=60).contains(&seconds),
        "duration must be 1..60 seconds"
    );
    Ok(Request::Capture {
        serial,
        seconds,
        status_json,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Sink {
    serial: u64,
    name: String,
}

fn selected_sink(sinks: &[Sink], serial: u64) -> Result<&Sink> {
    sinks
        .iter()
        .find(|sink| sink.serial == serial)
        .context("selected serial is not a current output sink; refusing capture")
        .context(CaptureReason::SourceRejected)
}

fn sinks(bytes: &[u8]) -> Result<Vec<Sink>> {
    ensure!(
        bytes.len() <= METADATA_LIMIT as usize,
        "PipeWire metadata exceeds limit"
    );
    let nodes: Vec<Value> = serde_json::from_slice(bytes).context("invalid PipeWire metadata")?;
    let mut sinks = Vec::new();
    for node in nodes {
        if node["type"] != "PipeWire:Interface:Node" {
            continue;
        }
        let props = &node["info"]["props"];
        if props["media.class"] != "Audio/Sink" {
            continue;
        }
        let serial = props["object.serial"]
            .as_u64()
            .or_else(|| props["object.serial"].as_str()?.parse().ok())
            .context("sink has no valid serial")?;
        ensure!(
            serial > 0 && !sinks.iter().any(|sink: &Sink| sink.serial == serial),
            "invalid or ambiguous sink serial"
        );
        let name = props["node.name"].as_str().context("sink has no name")?;
        ensure!(name.len() <= 512, "sink name exceeds limit");
        sinks.push(Sink {
            serial,
            name: name.to_owned(),
        });
        ensure!(sinks.len() <= 256, "too many output sinks");
    }
    Ok(sinks)
}

async fn discover(stop: impl Future<Output = Result<&'static str>>) -> Result<Vec<Sink>> {
    let mut child = Command::new("pw-dump")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("cannot run pw-dump")?;
    let mut stdout = child
        .stdout
        .take()
        .context("missing metadata pipe")?
        .take(METADATA_LIMIT + 1);
    let request = timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await?;
        ensure!(
            bytes.len() <= METADATA_LIMIT as usize,
            "PipeWire metadata exceeds limit"
        );
        ensure!(child.wait().await?.success(), "pw-dump failed");
        sinks(&bytes)
    });
    let result = tokio::select! {
        biased;
        reason = stop => match reason {
            Ok(reason) => Err(anyhow::anyhow!("capture cancelled before recorder startup: {reason}").context(CaptureReason::from_stop(reason))),
            Err(error) => Err(error.context("capture startup cancellation failed")),
        },
        result = request => result.context("PipeWire metadata request timed out").and_then(|result| result).context(CaptureReason::DiscoveryFailed),
    };
    if child.id().is_some() {
        child
            .kill()
            .await
            .context("cannot terminate metadata tool")
            .context(CaptureReason::CleanupFailed)?;
        child
            .wait()
            .await
            .context("cannot reap metadata tool")
            .context(CaptureReason::CleanupFailed)?;
    }
    result
}

fn recorder(serial: u64) -> Command {
    let mut command = Command::new("pw-record");
    command
        .args([
            "--target",
            &serial.to_string(),
            "--raw",
            "--format",
            "f32",
            "--rate",
            "48000",
            "--channels",
            "2",
            "--channel-map",
            "FL,FR",
            "--properties",
            "{ stream.capture.sink=true node.dont-reconnect=true node.dont-fallback=true }",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn stop_recorder(child: &mut Child) -> Result<()> {
    if child.id().is_some() {
        child
            .kill()
            .await
            .context("cannot terminate recorder")
            .context(CaptureReason::CleanupFailed)?;
    }
    child
        .wait()
        .await
        .context("cannot reap recorder")
        .context(CaptureReason::CleanupFailed)?;
    Ok(())
}

async fn consume(
    reader: &mut (impl AsyncRead + Unpin),
    session: &mut BufferedAcousticSession,
    permit: &SessionPermit,
    stop: impl Future<Output = Result<&'static str>>,
    stall: Duration,
    reporter: &mut StatusReporter,
) -> Result<&'static str> {
    let mut buffer = [0_u8; FRAME_BYTES];
    let mut filled = 0;
    let mut position = 0_u64;
    let mut last_frame = Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    tokio::pin!(stop);
    reporter.emit(CapturePhase::AwaitingFrames, Some(session.health()), false)?;
    loop {
        tokio::select! {
            biased;
            reason = &mut stop => return reason,
            _ = tokio::time::sleep_until(last_frame + stall) => return Err(anyhow::anyhow!("no complete audio frame within capture timeout").context(CaptureReason::InputStalled)),
            _ = heartbeat.tick() => {
                if !reporter.is_json() {
                    eprintln!("CAPTURE REQUEST ACTIVE: selected mixed output; analyzed_frames={}; Ctrl+C stops", session.health().delivered.frames);
                }
                if position > 0 {
                    reporter.emit(CapturePhase::ReceivingFrames, Some(session.health()), false)?;
                }
            }
            count = reader.read(&mut buffer[filled..]) => {
                let count = count.context("cannot read capture stream").context(CaptureReason::StreamReadFailed)?;
                if count == 0 {
                    return Err(anyhow::anyhow!("capture stream ended unexpectedly").context(CaptureReason::StreamEnded));
                }
                filled += count;
                if filled == buffer.len() {
                    session.push_bytes(permit, position, PcmEncoding::Float32Le, &buffer)
                        .map_err(|error| anyhow::anyhow!("capture processing fault: {error:?}").context(CaptureReason::ProcessingFailed))?;
                    let batch = session.pop().map_err(|error| anyhow::anyhow!("capture delivery fault: {error:?}"))
                        .and_then(|batch| batch.context("missing capture batch")).context(CaptureReason::DeliveryFailed)?;
                    ensure!(batch.features.len() == 1, "unexpected capture batch shape");
                    reporter.frame(&batch.features[0])?;
                    position = position.checked_add(960).context("sample position overflow")?;
                    if position == 960 {
                        reporter.emit(CapturePhase::ReceivingFrames, Some(session.health()), false)?;
                    }
                    buffer.fill(0);
                    filled = 0;
                    last_frame = Instant::now();
                }
            }
        }
    }
}

#[derive(Serialize)]
struct CaptureReport {
    schema_version: u16,
    stop_reason: &'static str,
    health: OutputHealth,
}

async fn supervise(
    mut child: Child,
    stop: impl Future<Output = Result<&'static str>>,
    stall: Duration,
    reporter: &mut StatusReporter,
) -> Result<CaptureReport> {
    let mut session = BufferedAcousticSession::default();
    let permit = session
        .start_encoded(Format::new(48_000, 2).unwrap(), 0, PcmEncoding::Float32Le)
        .unwrap();
    let mut reader = child.stdout.take().context("missing capture stdout")?;
    let result = tokio::select! {
        result = consume(&mut reader, &mut session, &permit, stop, stall, reporter) => result,
        exit = child.wait() => Err(anyhow::anyhow!("capture process exited unexpectedly: {exit:?}").context(CaptureReason::RecorderExited)),
    };
    if result.is_err() && session.status() == SessionStatus::Active {
        let _ = session.report_failure(&permit, ProducerFailure::SourceLost);
    }
    let final_processing_health = session.health();
    session.stop();
    let stopping = reporter.emit(CapturePhase::Stopping, Some(session.health()), false);
    stop_recorder(&mut child).await?;
    if !reporter.is_json() {
        eprintln!("CAPTURE STOPPED: recorder terminated; no automatic restart");
    }
    reporter.emit_reason(
        if result.is_ok() && stopping.is_ok() {
            CapturePhase::Stopped
        } else {
            CapturePhase::Faulted
        },
        Some(if result.is_ok() {
            session.health()
        } else {
            final_processing_health
        }),
        true,
        Some(match &result {
            Ok(reason) => CaptureReason::from_stop(reason),
            Err(error) => CaptureReason::from_error(error),
        }),
    )?;
    stopping?;
    Ok(CaptureReport {
        schema_version: 1,
        stop_reason: result?,
        health: session.health(),
    })
}

async fn capture_selected(
    serial: u64,
    stop: impl Future<Output = Result<&'static str>>,
    reporter: &mut StatusReporter,
) -> Result<CaptureReport> {
    tokio::pin!(stop);
    let available = discover(&mut stop).await.map_err(|error| {
        if error.downcast_ref::<CaptureReason>().is_some() {
            error
        } else {
            error.context(CaptureReason::DiscoveryFailed)
        }
    })?;
    tokio::select! {
        biased;
        reason = &mut stop => {
            let reason = reason?;
            return Err(anyhow::anyhow!("capture cancelled before recorder startup").context(CaptureReason::from_stop(reason)));
        },
        _ = std::future::ready(()) => {}
    }
    let selected = selected_sink(&available, serial)?.clone();
    if !reporter.is_json() {
        eprintln!(
            "STARTING CAPTURE: sink serial {serial}; ALL audio on this output may be analyzed. No audio files or participant attribution."
        );
    }
    reporter.emit(CapturePhase::RecorderRequested, None, false)?;
    let frame_clock_before = reporter.frame_clock_now();
    let mut child = recorder(serial)
        .spawn()
        .context("cannot start pw-record")
        .context(CaptureReason::RecorderStartFailed)?;
    let continuity = match discover(&mut stop).await {
        Ok(available) => match selected_sink(&available, serial) {
            Ok(current) if current == &selected => Ok(()),
            Ok(_) => Err(
                anyhow::anyhow!("selected output sink changed during recorder startup")
                    .context(CaptureReason::SourceRejected),
            ),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    if let Err(error) = continuity {
        stop_recorder(&mut child).await?;
        let reason = CaptureReason::from_error(&error);
        if let Some(stop_reason) = match reason {
            CaptureReason::DurationLimit => Some("duration_limit"),
            CaptureReason::Interrupt => Some("interrupt"),
            CaptureReason::Terminate => Some("terminate"),
            CaptureReason::OwnerCancelled => Some("owner_cancelled"),
            _ => None,
        } {
            let health = BufferedAcousticSession::default().health();
            if !reporter.is_json() {
                eprintln!("CAPTURE STOPPED: recorder terminated; no automatic restart");
            }
            reporter.emit_reason(CapturePhase::Stopped, Some(health), true, Some(reason))?;
            return Ok(CaptureReport {
                schema_version: 1,
                stop_reason,
                health,
            });
        }
        reporter.emit_reason(CapturePhase::Faulted, None, true, Some(reason))?;
        return Err(error);
    }
    reporter.prepare_frame_clock(frame_clock_before)?;
    supervise(child, stop, Duration::from_secs(3), reporter).await
}

pub async fn capture_into(
    serial: u64,
    seconds: u64,
    feed: crate::acoustic::AcousticFeed,
    cancel: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux") && cfg!(target_endian = "little"),
        "capture requires little-endian Linux"
    );
    ensure!(
        serial > 0 && (1..=60).contains(&seconds),
        "invalid acoustic capture policy"
    );
    let mut reporter = StatusReporter::with_feed(feed);
    reporter.emit(CapturePhase::Discovering, None, false)?;
    let stop = async {
        tokio::select! {
            biased;
            _ = cancel => Ok("owner_cancelled"),
            _ = tokio::time::sleep(Duration::from_secs(seconds)) => Ok("duration_limit"),
        }
    };
    let result = capture_selected(serial, stop, &mut reporter).await;
    if result.as_ref().is_err_and(|error| {
        error.downcast_ref::<CaptureReason>() == Some(&CaptureReason::OwnerCancelled)
    }) && !reporter.is_terminal()
    {
        reporter.emit(CapturePhase::Stopped, None, false)?;
        return Ok(());
    }
    if result.is_err() && !reporter.is_terminal() {
        reporter.emit_reason(
            CapturePhase::Faulted,
            None,
            false,
            Some(CaptureReason::InternalFailure),
        )?;
    }
    result.map(|_| ())
}

async fn run() -> Result<()> {
    ensure!(
        cfg!(target_endian = "little"),
        "capture requires a little-endian host"
    );
    let request = parse(&std::env::args().skip(1).collect::<Vec<_>>())?;
    match request {
        Request::Help => println!("{HELP}"),
        Request::Sources => println!(
            "{}",
            serde_json::to_string_pretty(&discover(std::future::pending()).await?)?
        ),
        Request::Capture {
            serial,
            seconds,
            status_json,
        } => {
            let mut reporter = StatusReporter::new(status_json);
            reporter.emit(CapturePhase::Discovering, None, false)?;
            let result: Result<CaptureReport> = async {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .context(CaptureReason::SignalSetupFailed)?;
                let mut interrupt =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                        .context(CaptureReason::SignalSetupFailed)?;
                let deadline = Instant::now() + Duration::from_secs(seconds);
                let stop = async {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => Ok("duration_limit"),
                        _ = interrupt.recv() => Ok("interrupt"),
                        _ = terminate.recv() => Ok("terminate"),
                    }
                };
                tokio::pin!(stop);
                capture_selected(serial, stop, &mut reporter).await
            }
            .await;
            if let Err(error) = &result
                && !reporter.is_terminal()
            {
                reporter.emit_reason(
                    CapturePhase::Faulted,
                    None,
                    false,
                    Some(CaptureReason::from_error(error)),
                )?;
            }
            let report = result?;
            println!("{}", serde_json::to_string(&report)?);
        }
    }
    Ok(())
}

pub async fn cli() -> std::process::ExitCode {
    #[cfg(unix)]
    let result = run().await;
    #[cfg(not(unix))]
    let result: Result<()> = Err(anyhow::anyhow!(
        "PipeWire capture is supported only on Linux/Unix"
    ));
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if !std::env::args().any(|argument| argument == "--status-json") {
                eprintln!("Error: {error:#}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context as TaskContext, Poll},
    };
    use tokio::{io::ReadBuf, sync::oneshot};

    #[test]
    fn owner_outcome_separates_capture_failure_from_cleanup_failure() {
        assert_eq!(
            capture_outcome(&Ok(())),
            CaptureOutcome {
                completed: true,
                cleanup_confirmed: true,
                reason: None,
            }
        );
        assert_eq!(
            capture_outcome(&Err(
                anyhow::anyhow!("invalid PCM").context(CaptureReason::ProcessingFailed)
            )),
            CaptureOutcome {
                completed: false,
                cleanup_confirmed: true,
                reason: Some(AcousticOwnerReason::ProcessingFailed),
            }
        );
        assert_eq!(
            capture_outcome(&Err(
                anyhow::anyhow!("cannot reap recorder").context(CaptureReason::CleanupFailed)
            )),
            CaptureOutcome {
                completed: false,
                cleanup_confirmed: false,
                reason: Some(AcousticOwnerReason::CleanupFailed),
            }
        );
    }

    #[test]
    fn selected_sink_requires_matching_current_metadata() {
        let selected = Sink {
            serial: 42,
            name: "expected-output".into(),
        };
        assert_eq!(
            selected_sink(std::slice::from_ref(&selected), 42).unwrap(),
            &selected
        );
        let error = selected_sink(std::slice::from_ref(&selected), 7).unwrap_err();
        assert_eq!(
            error.downcast_ref::<CaptureReason>(),
            Some(&CaptureReason::SourceRejected)
        );
    }

    struct Fragmented {
        bytes: Vec<u8>,
        offset: usize,
        width: usize,
        finished: Option<oneshot::Sender<()>>,
    }

    impl AsyncRead for Fragmented {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _context: &mut TaskContext<'_>,
            output: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.offset == self.bytes.len() {
                if let Some(finished) = self.finished.take() {
                    let _ = finished.send(());
                }
                return Poll::Pending;
            }
            let count = self
                .width
                .min(output.remaining())
                .min(self.bytes.len() - self.offset);
            output.put_slice(&self.bytes[self.offset..self.offset + count]);
            self.offset += count;
            Poll::Ready(Ok(()))
        }
    }

    fn processor() -> (BufferedAcousticSession, SessionPermit) {
        let mut session = BufferedAcousticSession::default();
        let permit = session
            .start_encoded(Format::new(48_000, 2).unwrap(), 0, PcmEncoding::Float32Le)
            .unwrap();
        (session, permit)
    }

    fn arguments(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn capture_requires_explicit_target_consent_and_bounded_duration() {
        assert_eq!(parse(&arguments("--help")).unwrap(), Request::Help);
        assert_eq!(parse(&arguments("sources")).unwrap(), Request::Sources);
        assert_eq!(
            parse(&arguments(
                "capture --sink-serial 42 --acknowledge-output-capture --seconds 30"
            ))
            .unwrap(),
            Request::Capture {
                serial: 42,
                seconds: 30,
                status_json: false
            }
        );
        assert_eq!(
            parse(&arguments(
                "capture --status-json --sink-serial 42 --acknowledge-output-capture --seconds 1"
            ))
            .unwrap(),
            Request::Capture {
                serial: 42,
                seconds: 1,
                status_json: true
            }
        );
        for invalid in [
            "",
            "capture",
            "capture --sink-serial 42 --seconds 30",
            "capture --acknowledge-output-capture --seconds 30",
            "capture --sink-serial 0 --acknowledge-output-capture --seconds 30",
            "capture --sink-serial 42 --acknowledge-output-capture --seconds 61",
            "capture --sink-serial 42 --acknowledge-output-capture --seconds 0",
            "sources extra",
            "capture --sink-serial 42 --acknowledge-output-capture --seconds 1 --seconds 2",
            "capture --sink-serial 42 --acknowledge-output-capture --seconds 1 --status-json --status-json",
            "capture --sink-serial 42 --seconds 1 --status-json",
        ] {
            assert!(parse(&arguments(invalid)).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn metadata_whitelists_only_unambiguous_output_sink_nodes() {
        let nodes = serde_json::json!([
            {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Source","object.serial":10,"node.name":"microphone"}}},
            {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Stream/Output/Audio","object.serial":11,"node.name":"application"}}},
            {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Sink","object.serial":"42","node.name":"selected\nsink"}}}
        ]);
        let bytes = serde_json::to_vec(&nodes).unwrap();
        let found = sinks(&bytes).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].serial, 42);
        assert!(
            serde_json::to_string(&found)
                .unwrap()
                .contains("selected\\nsink")
        );
        let mut duplicate = nodes.clone();
        duplicate.as_array_mut().unwrap().push(nodes[2].clone());
        assert!(sinks(&serde_json::to_vec(&duplicate).unwrap()).is_err());
        let mut missing = nodes;
        missing[2]["info"]["props"]["object.serial"] = Value::Null;
        assert!(sinks(&serde_json::to_vec(&missing).unwrap()).is_err());
        assert!(sinks(b"not json").is_err());
        assert!(sinks(&vec![0; METADATA_LIMIT as usize + 1]).is_err());
    }

    #[test]
    fn recorder_is_raw_explicit_and_disables_default_fallback() {
        let command = recorder(42);
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "--target",
                "42",
                "--raw",
                "--format",
                "f32",
                "--rate",
                "48000",
                "--channels",
                "2",
                "--channel-map",
                "FL,FR",
                "--properties",
                "{ stream.capture.sink=true node.dont-reconnect=true node.dont-fallback=true }",
                "-"
            ]
        );
    }

    #[tokio::test]
    async fn fragmented_pcm_is_bounded_aligned_drained_and_summary_only() {
        for width in [1, 7, FRAME_BYTES, FRAME_BYTES * 4] {
            let (mut session, permit) = processor();
            let (finished, receiver) = oneshot::channel();
            let mut reader = Fragmented {
                bytes: vec![0; FRAME_BYTES * 3 + 5],
                offset: 0,
                width,
                finished: Some(finished),
            };
            let reason = consume(
                &mut reader,
                &mut session,
                &permit,
                async {
                    receiver.await?;
                    Ok("test_stop")
                },
                Duration::from_secs(3),
                &mut StatusReporter::default(),
            )
            .await
            .unwrap();
            assert_eq!(reason, "test_stop");
            let health = session.health();
            assert_eq!(health.delivered.frames, 3);
            assert_eq!(health.pending.batches, 0);
            assert_eq!(health.high_water_batches, 1);
            let json = serde_json::to_string(&health).unwrap();
            assert!(!json.contains("rms"));
            assert!(!json.contains("samples"));
        }
    }

    #[tokio::test]
    async fn eof_invalid_pcm_and_stalled_stream_fail_visibly() {
        let (mut session, permit) = processor();
        let mut reader = &[0_u8; 3][..];
        let error = consume(
            &mut reader,
            &mut session,
            &permit,
            std::future::pending(),
            Duration::from_secs(1),
            &mut StatusReporter::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            CaptureReason::from_error(&error),
            CaptureReason::StreamEnded
        );
        assert_eq!(session.health().processed.frames, 0);
        let mut bytes = vec![0; FRAME_BYTES];
        bytes[FRAME_BYTES - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
        let mut reader = bytes.as_slice();
        assert!(
            consume(
                &mut reader,
                &mut session,
                &permit,
                std::future::pending(),
                Duration::from_secs(1),
                &mut StatusReporter::default()
            )
            .await
            .is_err()
        );
        assert_eq!(session.health().processed.frames, 0);
        assert!(matches!(session.status(), SessionStatus::Faulted { .. }));
        let (mut session, permit) = processor();
        let mut reader = Fragmented {
            bytes: vec![],
            offset: 0,
            width: 1,
            finished: None,
        };
        let error = consume(
            &mut reader,
            &mut session,
            &permit,
            std::future::pending(),
            Duration::from_millis(5),
            &mut StatusReporter::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            CaptureReason::from_error(&error),
            CaptureReason::InputStalled
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stop_and_timeout_terminate_and_reap_the_recorder_child() {
        for fail in [false, true] {
            let mut child = Command::new("cat")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let pid = child.id().unwrap();
            let input = child.stdin.take().unwrap();
            let result = if fail {
                supervise(
                    child,
                    std::future::pending(),
                    Duration::from_millis(5),
                    &mut StatusReporter::default(),
                )
                .await
            } else {
                supervise(
                    child,
                    async { Ok("test_interrupt") },
                    Duration::from_secs(1),
                    &mut StatusReporter::default(),
                )
                .await
            };
            drop(input);
            assert_eq!(result.is_err(), fail);
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
            if let Ok(report) = result {
                assert_eq!(report.health.status, SessionStatus::Stopped);
                assert_eq!(report.stop_reason, "test_interrupt");
                assert_eq!(report.health.pending.batches, 0);
            }
        }
    }
}
