use anyhow::Result;
use intellicord::observe::{ProcessHealth, generated_replay};
use serde::Serialize;

#[derive(Serialize)]
struct Report {
    schema_version: u16,
    source: &'static str,
    stop_reason: &'static str,
    sensor_running: bool,
    health: ProcessHealth,
}

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let json = arguments
        .first()
        .is_some_and(|argument| argument == "capture")
        && arguments
            .last()
            .is_some_and(|argument| argument == "--status-json");
    if let Err(error) = run(arguments).await {
        if !json {
            eprintln!("{error:#}");
        }
        std::process::exit(1);
    }
}

async fn run(arguments: Vec<String>) -> Result<()> {
    if arguments.len() == 3
        && arguments[0] == "evaluate-progress-reports"
        && arguments[1] == "--seconds"
    {
        use std::io::Read;
        let seconds = arguments[2].parse::<u64>()?;
        let mut input = Vec::new();
        std::io::stdin()
            .take(64 * 1024 + 1)
            .read_to_end(&mut input)?;
        let evaluation = intellicord::observe::progress::evaluation::evaluate(&input, seconds)?;
        println!("{}", serde_json::to_string(&evaluation)?);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments
        .first()
        .is_some_and(|argument| argument == "supervise-socket-progress-capture")
    {
        use intellicord::observe::progress::capture::Capture;
        use std::path::Path;
        anyhow::ensure!(
            arguments.len() == 8
                && arguments[1] == "--object"
                && !arguments[2].is_empty()
                && arguments[3] == "--pid"
                && arguments[5] == "--seconds"
                && arguments[7] == "--acknowledge-supervised-socket-progress",
            "Usage: intellicord-observe supervise-socket-progress-capture --object PATH --pid PID --seconds 1..60 --acknowledge-supervised-socket-progress"
        );
        let process_id: u32 = arguments[4].parse()?;
        let seconds: u64 = arguments[6].parse()?;
        anyhow::ensure!(
            (1..=i32::MAX as u32).contains(&process_id) && (1..=60).contains(&seconds),
            "invalid target or duration"
        );
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let (capture, _receiver) = Capture::prepare(
            &std::env::current_exe()?,
            &std::env::temp_dir(),
            Path::new(&arguments[2]),
            process_id,
            seconds,
        )?;
        let report = capture
            .run(async {
                tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
            })
            .await;
        println!("{}", serde_json::to_string(&report)?);
        anyhow::ensure!(
            report.endpoint_removed,
            "private progress endpoint cleanup failed"
        );
        anyhow::ensure!(
            report.completed(),
            "supervised socket progress capture did not complete"
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments
        .first()
        .is_some_and(|argument| argument == "supervise-progress-capture")
    {
        use intellicord::observe::{
            progress::supervisor::supervise, stream::Source, supervisor::Outcome,
        };
        use std::time::Duration;
        anyhow::ensure!(
            arguments.len() == 8
                && arguments[1] == "--object"
                && !arguments[2].is_empty()
                && arguments[3] == "--pid"
                && arguments[5] == "--seconds"
                && arguments[7] == "--acknowledge-supervised-progress",
            "Usage: intellicord-observe supervise-progress-capture --object PATH --pid PID --seconds 1..60 --acknowledge-supervised-progress"
        );
        let process_id: u32 = arguments[4].parse()?;
        let seconds: u64 = arguments[6].parse()?;
        anyhow::ensure!(
            (1..=i32::MAX as u32).contains(&process_id) && (1..=60).contains(&seconds),
            "invalid target or duration"
        );
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let session = select_progress_session()?;
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command.args([
            "capture-progress",
            "--object",
            &arguments[2],
            "--pid",
            &arguments[4],
            "--seconds",
            &arguments[6],
            "--session",
            &session,
            "--acknowledge-process-progress",
        ]);
        let run = supervise(
            command,
            &session,
            Source::KernelTracepoint,
            Duration::from_secs(seconds + 3),
            Duration::from_secs(3),
            async {
                tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
            },
        )
        .await;
        println!(
            "{}",
            serde_json::json!({"schema_version":1,
                "source":"supervised_kernel_progress", "session_binding":"parent_selected",
                "run":run})
        );
        anyhow::ensure!(
            run.outcome == Outcome::Completed,
            "supervised progress capture did not complete"
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments
        .first()
        .is_some_and(|argument| argument == "supervise-capture")
    {
        use intellicord::observe::{
            stream::Source,
            supervisor::{Outcome, supervise},
        };
        use std::time::Duration;
        anyhow::ensure!(
            arguments.len() == 8
                && arguments[1] == "--object"
                && !arguments[2].is_empty()
                && arguments[3] == "--pid"
                && arguments[5] == "--seconds"
                && arguments[7] == "--acknowledge-supervised-observation",
            "Usage: intellicord-observe supervise-capture --object PATH --pid PID --seconds 1..60 --acknowledge-supervised-observation"
        );
        let process_id: u32 = arguments[4].parse()?;
        let seconds: u64 = arguments[6].parse()?;
        anyhow::ensure!(
            (1..=i32::MAX as u32).contains(&process_id) && (1..=60).contains(&seconds),
            "invalid target or duration"
        );
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command.args([
            "capture",
            "--object",
            &arguments[2],
            "--pid",
            &arguments[4],
            "--seconds",
            &arguments[6],
            "--acknowledge-process-observation",
            "--status-json",
        ]);
        let run = supervise(
            command,
            Source::KernelTracepoint,
            Duration::from_secs(seconds + 3),
            async {
                tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
            },
        )
        .await;
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::health::CaptureReport::new(run))?
        );
        anyhow::ensure!(
            run.outcome == Outcome::Completed,
            "supervised observation did not complete"
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments
        .first()
        .is_some_and(|argument| argument == "delivery-child" || argument == "delivery-child-exit")
    {
        anyhow::ensure!(
            arguments.len() == 1,
            "Usage: internal delivery child takes no arguments"
        );
        if arguments[0] == "delivery-child" {
            intellicord::observe::delivery::live::child_helper()?;
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments
        .first()
        .is_some_and(|argument| argument == "verify-delivery")
    {
        anyhow::ensure!(
            arguments.len() == 4
                && arguments[1] == "--object"
                && !arguments[2].is_empty()
                && arguments[3] == "--acknowledge-bpf-delivery",
            "Usage: intellicord-observe verify-delivery --object PATH --acknowledge-bpf-delivery; observes only a controlled child exec and then cleans up"
        );
        let report =
            intellicord::observe::delivery::live::verify_path(std::path::Path::new(&arguments[2]))
                .await;
        println!("{}", serde_json::to_string(&report)?);
        anyhow::ensure!(
            report.outcome == intellicord::observe::delivery::live::Outcome::Verified,
            "controlled delivery verification did not succeed"
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments
        .first()
        .is_some_and(|argument| argument == "verify-attach")
    {
        anyhow::ensure!(
            arguments.len() == 4
                && arguments[1] == "--object"
                && !arguments[2].is_empty()
                && arguments[3] == "--acknowledge-bpf-attach",
            "Usage: intellicord-observe verify-attach --object PATH --acknowledge-bpf-attach; briefly attaches the reviewed program with target disabled, then detaches"
        );
        let report =
            intellicord::observe::attachment::verify_path(std::path::Path::new(&arguments[2]));
        println!("{}", serde_json::to_string(&report)?);
        anyhow::ensure!(
            report.outcome == intellicord::observe::attachment::Outcome::Verified,
            "attach-only verification did not succeed"
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments
        .first()
        .is_some_and(|argument| argument == "verify-load")
    {
        anyhow::ensure!(
            arguments.len() == 4
                && arguments[1] == "--object"
                && !arguments[2].is_empty()
                && arguments[3] == "--acknowledge-bpf-load",
            "Usage: intellicord-observe verify-load --object PATH --acknowledge-bpf-load; creates temporary maps and loads the reviewed program without attaching"
        );
        let report =
            intellicord::observe::verification::verify_path(std::path::Path::new(&arguments[2]));
        println!("{}", serde_json::to_string(&report)?);
        anyhow::ensure!(
            report.outcome == intellicord::observe::verification::Outcome::Verified,
            "load-only verification did not succeed"
        );
        return Ok(());
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "inspect-events")
    {
        use anyhow::ensure;
        use intellicord::observe::events::EventStream;
        use std::io::Read;
        ensure!(
            arguments.len() == 13
                && arguments[1] == "--pid"
                && arguments[3] == "--source-anchor"
                && arguments[5] == "--before"
                && arguments[7] == "--after"
                && arguments[9] == "--receipt"
                && arguments[11] == "--exit",
            "Usage: intellicord-observe inspect-events --pid PID --source-anchor NS --before NS --after NS --receipt NS --exit success|failure"
        );
        let process_id: u32 = arguments[2].parse()?;
        let mut stream = EventStream::new(
            process_id,
            arguments[4].parse()?,
            arguments[6].parse()?,
            arguments[8].parse()?,
        )?;
        let receipt_ns: u64 = arguments[10].parse()?;
        let success = match arguments[12].as_str() {
            "success" => true,
            "failure" => false,
            _ => anyhow::bail!("unsupported process exit classification"),
        };
        let mut input = std::io::stdin().lock();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            stream.push(&buffer[..count], receipt_ns)?;
        }
        let report = stream.finish(success)?;
        println!(
            "{}",
            serde_json::json!({"schema_version":1,"source":"event_stream_validation",
            "sensor_running":false,"load_test_performed":false,"report":report})
        );
        return Ok(());
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "inspect-status")
    {
        use anyhow::ensure;
        use intellicord::observe::stream::{Source, StatusStream};
        use std::io::Read;
        ensure!(
            arguments.len() == 5 && arguments[1] == "--source" && arguments[3] == "--exit",
            "Usage: intellicord-observe inspect-status --source generated_replay|kernel_tracepoint --exit success|failure"
        );
        let source = match arguments[2].as_str() {
            "generated_replay" => Source::GeneratedReplay,
            "kernel_tracepoint" => Source::KernelTracepoint,
            _ => anyhow::bail!("unsupported status source"),
        };
        let success = match arguments[4].as_str() {
            "success" => true,
            "failure" => false,
            _ => anyhow::bail!("unsupported process exit classification"),
        };
        let mut stream = StatusStream::new(source);
        let mut input = std::io::stdin().lock();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            stream.push(&buffer[..count])?;
        }
        let report = stream.finish(success)?;
        println!(
            "{}",
            serde_json::json!({"schema_version":1,"source":"status_stream_validation",
            "sensor_running":false,"load_test_performed":false,"report":report})
        );
        return Ok(());
    }
    if arguments == ["replay-lifecycle"] {
        for record in intellicord::observe::lifecycle::generated_lifecycle() {
            println!("{}", serde_json::to_string(&record)?);
        }
        return Ok(());
    }
    if arguments == ["replay-negotiated"] {
        return intellicord::observe::collection::write_negotiated_generated().await;
    }
    if arguments == ["probe-target"] {
        println!(
            "{}",
            serde_json::to_string(&intellicord::observe::target_check())?
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments.len() == 3 && arguments[0] == "inspect" && arguments[1] == "--object" {
        use intellicord::observe::object::{inspect_object, read_object};
        let bytes = read_object(std::path::Path::new(&arguments[2]))?;
        let layout = inspect_object(&bytes)?;
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "schema_version":1,"source":"object_metadata","layout":layout
            }))?
        );
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if arguments.first().is_some_and(|argument| {
        matches!(
            argument.as_str(),
            "capture-progress" | "capture-socket-progress"
        )
    }) {
        return capture_progress(&arguments).await;
    }
    let report = if arguments == ["replay"] {
        Report {
            schema_version: 1,
            source: "generated_replay",
            stop_reason: "completed",
            sensor_running: false,
            health: generated_replay(),
        }
    } else {
        live(&arguments).await?
    };
    let event_output = arguments.iter().any(|argument| argument == "--events");
    if !event_output {
        println!("{}", serde_json::to_string(&report)?);
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn live(_arguments: &[String]) -> Result<Report> {
    anyhow::bail!("Live observation is supported only on Linux; use replay for generated input")
}

#[cfg(target_os = "linux")]
type ProgressOutput = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;

#[cfg(target_os = "linux")]
enum CaptureStatus {
    Legacy(intellicord::observe::status::StatusReporter),
    Progress(Box<intellicord::observe::progress::reporter::Reporter<ProgressOutput>>),
}

#[cfg(target_os = "linux")]
impl CaptureStatus {
    fn is_json(&self) -> bool {
        match self {
            Self::Legacy(status) => status.is_json(),
            Self::Progress(_) => true,
        }
    }

    async fn emit(
        &mut self,
        phase: intellicord::observe::lifecycle::Phase,
        reason: Option<intellicord::observe::lifecycle::Reason>,
        health: Option<ProcessHealth>,
    ) -> Result<()> {
        match self {
            Self::Legacy(status) => status.emit(phase, reason, health),
            Self::Progress(status) => {
                use anyhow::Context;
                status
                    .emit(phase, reason, health)
                    .await
                    .context(intellicord::observe::lifecycle::Reason::DeliveryFailed)
            }
        }
    }

    async fn periodic_health(&mut self, health: ProcessHealth) -> Result<()> {
        if let Self::Progress(status) = self {
            use anyhow::Context;
            status
                .periodic_health(health)
                .await
                .context(intellicord::observe::lifecycle::Reason::DeliveryFailed)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn select_progress_session() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("could not select process progress session: {error}"))?;
    Ok(format!("{:032x}", u128::from_be_bytes(bytes)))
}

#[cfg(all(test, target_os = "linux"))]
mod progress_session_tests {
    #[test]
    fn parent_sessions_are_fresh_valid_bindings_not_the_fixture() {
        use intellicord::observe::{progress::ProgressStream, stream::Source};
        let first = super::select_progress_session().unwrap();
        let second = super::select_progress_session().unwrap();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
        assert_ne!(first, "00000000000000000000000000000001");
        assert!(ProgressStream::new(&first, Source::KernelTracepoint).is_ok());
        assert!(ProgressStream::new(&second, Source::KernelTracepoint).is_ok());
    }
}

#[cfg(target_os = "linux")]
async fn connect_progress_parent(path: &str, parent_pid: u32) -> Result<tokio::net::UnixStream> {
    use anyhow::ensure;
    use std::{
        os::unix::fs::{FileTypeExt, MetadataExt},
        path::Path,
        time::Duration,
    };
    ensure!(
        (1..=i32::MAX as u32).contains(&parent_pid)
            && unsafe { libc::getppid() } == parent_pid as i32,
        "socket progress requires the actual parent PID"
    );
    let path = Path::new(path);
    ensure!(
        path.is_absolute(),
        "socket progress requires an absolute endpoint path"
    );
    let directory = std::fs::symlink_metadata(
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("socket directory required"))?,
    )?;
    let uid = unsafe { libc::geteuid() };
    ensure!(
        directory.is_dir() && directory.uid() == uid && directory.mode() & 0o777 == 0o700,
        "socket progress requires a same-user private 0700 directory"
    );
    let endpoint = std::fs::symlink_metadata(path)?;
    ensure!(
        endpoint.file_type().is_socket() && endpoint.uid() == uid,
        "socket progress requires a same-user socket endpoint"
    );
    let stream = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::UnixStream::connect(path),
    )
    .await??;
    let peer = stream.peer_cred()?;
    ensure!(
        peer.uid() == uid
            && peer.pid() == Some(parent_pid as i32)
            && unsafe { libc::getppid() } == parent_pid as i32,
        "socket progress server credentials did not match the parent"
    );
    Ok(stream)
}

#[cfg(target_os = "linux")]
async fn capture_progress(arguments: &[String]) -> Result<()> {
    use anyhow::ensure;
    use intellicord::observe::{
        lifecycle::{Phase, Reason},
        progress::reporter::Reporter,
        stream::Source,
    };
    let socket = arguments[0] == "capture-socket-progress";
    let shape = if socket {
        arguments.len() == 14
            && arguments[9] == "--socket"
            && !arguments[10].is_empty()
            && arguments[11] == "--parent-pid"
            && arguments[13] == "--acknowledge-socket-progress"
    } else {
        arguments.len() == 10 && arguments[9] == "--acknowledge-process-progress"
    };
    ensure!(
        shape
            && arguments[1] == "--object"
            && !arguments[2].is_empty()
            && arguments[3] == "--pid"
            && arguments[5] == "--seconds"
            && arguments[7] == "--session",
        "Usage: intellicord-observe capture-progress --object PATH --pid PID --seconds 1..60 --session HEX32 --acknowledge-process-progress; or capture-socket-progress --object PATH --pid PID --seconds 1..60 --session HEX32 --socket PATH --parent-pid PID --acknowledge-socket-progress"
    );
    let process_id: u32 = arguments[4].parse()?;
    let seconds: u64 = arguments[6].parse()?;
    ensure!(
        (1..=i32::MAX as u32).contains(&process_id) && (1..=60).contains(&seconds),
        "invalid target or duration"
    );
    intellicord::observe::progress::ProgressStream::new(&arguments[8], Source::KernelTracepoint)?;
    let output: ProgressOutput = if socket {
        Box::new(connect_progress_parent(&arguments[10], arguments[12].parse()?).await?)
    } else {
        Box::new(tokio::io::stdout())
    };
    let reporter = Reporter::new(&arguments[8], Source::KernelTracepoint, output)?;
    let mut status = CaptureStatus::Progress(Box::new(reporter));
    status.emit(Phase::Validating, None, None).await?;
    let result = live_session(
        process_id,
        std::time::Duration::from_secs(seconds),
        &arguments[2],
        false,
        false,
        &mut status,
    )
    .await;
    if let Err(error) = &result {
        let _ = status
            .emit(Phase::Faulted, Some(Reason::from_error(error)), None)
            .await;
    }
    if let CaptureStatus::Progress(reporter) = status {
        let finished = reporter.finish(result.is_ok());
        result?;
        finished?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn live(arguments: &[String]) -> Result<Report> {
    use anyhow::{Context, ensure};
    use intellicord::observe::{
        lifecycle::{Phase, Reason},
        status::StatusReporter,
    };
    let json = arguments
        .last()
        .is_some_and(|argument| argument == "--status-json");
    let arguments = if json {
        &arguments[..arguments.len() - 1]
    } else {
        arguments
    };
    let mut status = CaptureStatus::Legacy(StatusReporter::new(json));
    status.emit(Phase::Validating, None, None).await?;
    let events = arguments
        .last()
        .is_some_and(|argument| argument == "--events");
    let arguments = if events {
        &arguments[..arguments.len() - 1]
    } else {
        arguments
    };
    let handshake = arguments
        .last()
        .is_some_and(|argument| argument == "--clock-handshake");
    let arguments = if handshake {
        &arguments[..arguments.len() - 1]
    } else {
        arguments
    };
    let request = (|| -> Result<(u32, std::time::Duration)> {
        ensure!(!events || json, "--events requires trailing --status-json");
        ensure!(
            !handshake || (events && json),
            "--clock-handshake requires --events --status-json"
        );
        ensure!(
            arguments.len() == 8
                && arguments[0] == "capture"
                && arguments[1] == "--object"
                && arguments[3] == "--pid"
                && arguments[5]
                    == if handshake {
                        "--milliseconds"
                    } else {
                        "--seconds"
                    }
                && arguments[7] == "--acknowledge-process-observation",
            "Usage: intellicord-observe capture --object PATH --pid PID --seconds 1..60 --acknowledge-process-observation [--events] [--status-json]; negotiated export requires --milliseconds 1..500 and trailing --clock-handshake --events --status-json"
        );
        let process_id: u32 = arguments[4].parse().context("invalid process ID")?;
        ensure!(
            (1..=i32::MAX as u32).contains(&process_id),
            "process ID out of range"
        );
        let duration: u64 = arguments[6].parse().context("invalid duration")?;
        let duration = if handshake {
            ensure!(
                (1..=500).contains(&duration),
                "negotiated duration must be 1..500 milliseconds"
            );
            std::time::Duration::from_millis(duration)
        } else {
            ensure!(
                (1..=60).contains(&duration),
                "duration must be 1..60 seconds"
            );
            std::time::Duration::from_secs(duration)
        };
        Ok((process_id, duration))
    })();
    let (process_id, duration) = match request {
        Ok(request) => request,
        Err(error) => {
            let _ = status
                .emit(Phase::Faulted, Some(Reason::InvalidRequest), None)
                .await;
            return Err(error);
        }
    };
    let result = live_session(
        process_id,
        duration,
        &arguments[2],
        events,
        handshake,
        &mut status,
    )
    .await;
    if let Err(error) = &result {
        let _ = status
            .emit(Phase::Faulted, Some(Reason::from_error(error)), None)
            .await;
    }
    result
}

#[cfg(target_os = "linux")]
enum EventReader {
    Health(intellicord::observe::ProcessMonitor),
    Export(intellicord::observe::export::EventExporter),
}

#[cfg(target_os = "linux")]
impl EventReader {
    fn new(process_id: u32, events: bool) -> Result<Self> {
        if events {
            Ok(Self::Export(
                intellicord::observe::export::EventExporter::new(process_id, std::io::stdout())?,
            ))
        } else {
            Ok(Self::Health(
                intellicord::observe::ProcessMonitor::new(process_id)
                    .ok_or_else(|| anyhow::anyhow!("invalid process ID"))?,
            ))
        }
    }

    fn accept(&mut self, bytes: &[u8]) -> Result<()> {
        use intellicord::observe::{export::ExportError, lifecycle::Reason};
        match self {
            Self::Health(monitor) => anyhow::ensure!(monitor.accept(bytes), Reason::InputRejected),
            Self::Export(exporter) => exporter.emit(bytes).map_err(|error| {
                anyhow::anyhow!(match error {
                    ExportError::Record | ExportError::Limit => Reason::InputRejected,
                    ExportError::Output | ExportError::Rejected => Reason::DeliveryFailed,
                })
            })?,
        }
        Ok(())
    }

    fn health(&self) -> ProcessHealth {
        match self {
            Self::Health(monitor) => monitor.health(),
            Self::Export(exporter) => exporter.health(),
        }
    }

    fn update_ring_dropped(&mut self, count: u64) {
        match self {
            Self::Health(monitor) => monitor.update_ring_dropped(count),
            Self::Export(exporter) => exporter.update_ring_dropped(count),
        }
    }
}

#[cfg(target_os = "linux")]
async fn live_session(
    process_id: u32,
    duration: std::time::Duration,
    object_path: &str,
    events: bool,
    handshake: bool,
    status: &mut CaptureStatus,
) -> Result<Report> {
    use anyhow::{Context, ensure};
    use aya::{
        Ebpf,
        maps::{Array, PerCpuArray, RingBuf},
        programs::TracePoint,
    };
    use futures_util::FutureExt;
    use intellicord::observe::{
        lifecycle::{Phase, Reason},
        object::{inspect_object, read_object},
        target::TargetGuard,
    };
    use std::time::Duration;

    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context(Reason::SignalSetupFailed)?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context(Reason::SignalSetupFailed)?;
    let bytes = read_object(std::path::Path::new(object_path)).context(Reason::InvalidObject)?;
    inspect_object(&bytes).context(Reason::InvalidObject)?;
    let target_guard = TargetGuard::open(process_id).context(Reason::TargetUnavailable)?;
    ensure!(
        target_guard.is_alive().context(Reason::TargetUnavailable)?,
        Reason::TargetUnavailable
    );
    let mut monitor = EventReader::new(process_id, events).context(Reason::DeliveryFailed)?;
    status.emit(Phase::Loading, None, None).await?;
    let mut bpf = Ebpf::load(&bytes).context(Reason::LoadFailed)?;
    let mut target = Array::<_, u32>::try_from(bpf.take_map("TARGET").context("missing TARGET")?)?;
    target.set(0, process_id, 0)?;
    let mut ring = RingBuf::try_from(bpf.take_map("EVENTS").context("missing EVENTS")?)?;
    let dropped =
        PerCpuArray::<_, u64>::try_from(bpf.take_map("DROPPED").context("missing DROPPED")?)?;
    let program: &mut TracePoint = bpf
        .program_mut("process_exec")
        .context("missing process_exec")?
        .try_into()?;
    program.load().context(Reason::LoadFailed)?;
    if handshake {
        use intellicord::observe::handshake::{ResponseError, reply_bounded};
        reply_bounded(async {
            tokio::select! {
                biased;
                _ = interrupt.recv() => {},
                _ = terminate.recv() => {},
            }
        })
        .await
        .map_err(|error| {
            anyhow::anyhow!(match error {
                ResponseError::Cancelled => Reason::Cancelled,
                ResponseError::Spawn => Reason::DeliveryFailed,
                ResponseError::Deadline | ResponseError::Rejected => Reason::InputRejected,
            })
        })?;
    }
    if !status.is_json() {
        eprintln!(
            "PROCESS OBSERVATION REQUESTED; pidfd lifetime guard with numeric PID filter; no participant attribution"
        );
    }
    ensure!(
        interrupt.recv().now_or_never().is_none() && terminate.recv().now_or_never().is_none(),
        Reason::Cancelled
    );
    ensure!(
        target_guard.is_alive().context(Reason::TargetUnavailable)?,
        Reason::TargetUnavailable
    );
    status.emit(Phase::Attaching, None, None).await?;
    let link = program
        .attach("sched", "sched_process_exec")
        .context(Reason::AttachFailed)?;
    status
        .emit(Phase::Observing, None, Some(monitor.health()))
        .await?;
    let deadline = tokio::time::sleep(duration);
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    let stop_reason = loop {
        tokio::select! {
            biased;
            _ = interrupt.recv() => break "interrupt",
            _ = terminate.recv() => break "terminate",
            _ = &mut deadline => break "duration_limit",
            _ = tick.tick() => {
                if !target_guard.is_alive().context(Reason::TargetUnavailable)? { break "target_exited"; }
                for _ in 0..64 {
                    let Some(event) = ring.next() else { break; };
                    monitor.accept(&event)?;
                }
                monitor.update_ring_dropped(intellicord_observe_common::ring_drop_total(
                    dropped.get(&0, 0).context(Reason::ReadFailed)?.iter().copied(),
                ));
                status.periodic_health(monitor.health()).await?;
            }
        }
    };
    let stopping_status = status
        .emit(Phase::Stopping, None, Some(monitor.health()))
        .await;
    program.detach(link).context(Reason::DetachFailed)?;
    stopping_status?;
    let mut stop_reason = if stop_reason == "target_exited"
        || !target_guard.is_alive().context(Reason::TargetUnavailable)?
    {
        "target_exited"
    } else {
        stop_reason
    };
    if stop_reason != "target_exited" {
        for _ in 0..intellicord_observe_common::RING_BYTES as usize
            / intellicord_observe_common::EVENT_BYTES
        {
            if !target_guard.is_alive().context(Reason::TargetUnavailable)? {
                stop_reason = "target_exited";
                break;
            }
            let Some(event) = ring.next() else {
                break;
            };
            monitor.accept(&event)?;
        }
    }
    monitor.update_ring_dropped(intellicord_observe_common::ring_drop_total(
        dropped
            .get(&0, 0)
            .context(Reason::ReadFailed)?
            .iter()
            .copied(),
    ));
    status
        .emit(
            Phase::Stopped,
            Some(Reason::from_stop(stop_reason)),
            Some(monitor.health()),
        )
        .await?;
    if !status.is_json() {
        eprintln!("PROCESS OBSERVATION STOPPED");
    }
    Ok(Report {
        schema_version: 1,
        source: "kernel_tracepoint",
        stop_reason,
        sensor_running: false,
        health: monitor.health(),
    })
}
