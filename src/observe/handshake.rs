use serde::{Deserialize, Serialize};

use super::events::{EventStream, StreamError};

pub const MESSAGE_LIMIT: usize = 256;
pub const MAX_BRACKET_NS: u64 = 25_000_000;
pub const RESPONSE_DEADLINE_MS: u64 = 2000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready<'buffer> {
    schema_version: u16,
    record_type: &'buffer str,
    clock_domain: &'buffer str,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request<'buffer> {
    schema_version: u16,
    record_type: &'buffer str,
    nonce: u64,
}

fn framed(bytes: &[u8]) -> bool {
    bytes.len() <= MESSAGE_LIMIT
        && bytes.last() == Some(&b'\n')
        && !bytes[..bytes.len().saturating_sub(1)].contains(&b'\n')
}

#[derive(Serialize)]
pub struct HandshakeRun {
    pub schema_version: u16,
    pub outcome: super::supervisor::Outcome,
    pub helper_started: bool,
    pub helper_reaped: bool,
    pub bracket_width_ns: Option<u64>,
}

pub async fn exchange(
    mut command: tokio::process::Command,
    origin: std::time::Instant,
    nonce: u64,
    cancel: impl std::future::Future<Output = ()>,
) -> (HandshakeRun, Option<ClockHandshake>) {
    use super::supervisor::Outcome;
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    use tokio::io::AsyncReadExt;
    let mut report = HandshakeRun {
        schema_version: 1,
        outcome: Outcome::InvalidStream,
        helper_started: false,
        helper_reaped: false,
        bracket_width_ns: None,
    };
    if nonce == 0 || Instant::now().checked_duration_since(origin).is_none() {
        return (report, None);
    }
    tokio::pin!(cancel);
    if futures_util::FutureExt::now_or_never(cancel.as_mut()).is_some() {
        report.outcome = Outcome::Cancelled;
        return (report, None);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        report.outcome = Outcome::SpawnFailed;
        return (report, None);
    };
    report.helper_started = true;
    let input = child.stdin.take().expect("piped clock request");
    let mut output = child.stdout.take().expect("piped clock reply");
    let work = async {
        let handshake = negotiate(input, &mut output, origin, nonce).await?;
        let mut tail = [0; 1];
        if output
            .read(&mut tail)
            .await
            .map_err(|_| Outcome::ReadFailed)?
            != 0
        {
            return Err(Outcome::InvalidStream);
        }
        let exit = child.wait().await.map_err(|_| Outcome::WaitFailed)?;
        report.helper_reaped = true;
        if !exit.success() {
            return Err(Outcome::CollectorFault);
        }
        if elapsed(origin).is_none_or(|now| {
            handshake
                .after_ns
                .checked_add(super::clock::HORIZON_NS)
                .is_none_or(|expiry| now > expiry)
        }) {
            return Err(Outcome::InvalidStream);
        }
        Ok(handshake)
    };
    let result = tokio::select! {
        biased;
        _ = &mut cancel => Err(Outcome::Cancelled),
        _ = tokio::time::sleep(Duration::from_secs(2)) => Err(Outcome::Deadline),
        result = work => result,
    };
    match result {
        Ok(handshake) => {
            report.outcome = Outcome::Completed;
            report.bracket_width_ns = Some(handshake.bracket_width_ns());
            (report, Some(handshake))
        }
        Err(outcome) => {
            report.outcome = outcome;
            if !report.helper_reaped {
                let _ = child.start_kill();
                match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                    Ok(Ok(_)) => report.helper_reaped = true,
                    _ => report.outcome = Outcome::CleanupFailed,
                }
            }
            (report, None)
        }
    }
}

pub(super) async fn negotiate(
    mut input: tokio::process::ChildStdin,
    output: &mut tokio::process::ChildStdout,
    origin: std::time::Instant,
    nonce: u64,
) -> Result<ClockHandshake, super::supervisor::Outcome> {
    use super::supervisor::Outcome;
    use tokio::io::AsyncWriteExt;
    let bytes = read_message(output).await?;
    let ready: Ready<'_> = serde_json::from_slice(&bytes).map_err(|_| Outcome::InvalidStream)?;
    if ready.schema_version != 1
        || ready.record_type != "clock_ready"
        || ready.clock_domain != intellicord_observe_common::CLOCK_DOMAIN
    {
        return Err(Outcome::InvalidStream);
    }
    let mut request = serde_json::to_vec(&Request {
        schema_version: 1,
        record_type: "clock_request",
        nonce,
    })
    .map_err(|_| Outcome::InvalidStream)?;
    request.push(b'\n');
    let before = elapsed(origin).ok_or(Outcome::InvalidStream)?;
    input
        .write_all(&request)
        .await
        .map_err(|_| Outcome::ReadFailed)?;
    input.shutdown().await.map_err(|_| Outcome::ReadFailed)?;
    drop(input);
    let reply = read_message(output).await?;
    let after = elapsed(origin).ok_or(Outcome::InvalidStream)?;
    ClockHandshake::accept(&reply, nonce, before, after).map_err(|_| Outcome::InvalidStream)
}

fn elapsed(origin: std::time::Instant) -> Option<u64> {
    u64::try_from(
        std::time::Instant::now()
            .checked_duration_since(origin)?
            .as_nanos(),
    )
    .ok()
}

async fn read_message(
    reader: &mut tokio::process::ChildStdout,
) -> Result<Vec<u8>, super::supervisor::Outcome> {
    use super::supervisor::Outcome;
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::with_capacity(MESSAGE_LIMIT);
    while bytes.len() < MESSAGE_LIMIT {
        let mut byte = [0];
        if reader
            .read(&mut byte)
            .await
            .map_err(|_| Outcome::ReadFailed)?
            == 0
        {
            return Err(Outcome::InvalidStream);
        }
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(bytes);
        }
    }
    Err(Outcome::InvalidStream)
}

pub fn reply_stdio() -> anyhow::Result<()> {
    #[cfg(all(feature = "observe", target_os = "linux"))]
    {
        respond_stdio(super::clock::read_monotonic_ns)
    }
    #[cfg(not(all(feature = "observe", target_os = "linux")))]
    anyhow::bail!("clock handshake requires Linux and the observe feature")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseError {
    Cancelled,
    Deadline,
    Spawn,
    Rejected,
}

impl std::fmt::Display for ResponseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "clock response {self:?}")
    }
}

impl std::error::Error for ResponseError {}

pub async fn reply_bounded(
    cancel: impl std::future::Future<Output = ()>,
) -> Result<(), ResponseError> {
    bounded_response(reply_stdio, cancel).await
}

pub(super) async fn generated_response() -> Result<(), ResponseError> {
    bounded_response(|| respond_stdio(|| Some(1000)), std::future::pending()).await
}

async fn bounded_response(
    respond: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    cancel: impl std::future::Future<Output = ()>,
) -> Result<(), ResponseError> {
    tokio::pin!(cancel);
    if futures_util::FutureExt::now_or_never(cancel.as_mut()).is_some() {
        return Err(ResponseError::Cancelled);
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("process-clock".into())
        .spawn(move || {
            let _ = sender.send(respond().map_err(|_| ResponseError::Rejected));
        })
        .map_err(|_| ResponseError::Spawn)?;
    tokio::select! {
        biased;
        _ = &mut cancel => Err(ResponseError::Cancelled),
        _ = tokio::time::sleep(std::time::Duration::from_millis(RESPONSE_DEADLINE_MS)) => Err(ResponseError::Deadline),
        result = receiver => result.unwrap_or(Err(ResponseError::Rejected)),
    }
}

pub(super) fn respond_stdio(source: impl FnOnce() -> Option<u64>) -> anyhow::Result<()> {
    use std::io::{BufRead, Read, Write};
    let mut output = std::io::stdout().lock();
    writeln!(
        output,
        "{}",
        serde_json::json!({"schema_version":1,"record_type":"clock_ready","clock_domain":intellicord_observe_common::CLOCK_DOMAIN})
    )?;
    output.flush()?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(MESSAGE_LIMIT as u64 + 1)
        .read_until(b'\n', &mut bytes)?;
    anyhow::ensure!(framed(&bytes), "invalid clock request framing");
    let request: Request<'_> = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        request.schema_version == 1 && request.record_type == "clock_request" && request.nonce > 0,
        "invalid clock request"
    );
    let source = source().ok_or_else(|| anyhow::anyhow!("monotonic clock unavailable"))?;
    writeln!(
        output,
        "{}",
        serde_json::json!({"schema_version":1,"record_type":"clock_reply",
            "clock_domain":intellicord_observe_common::CLOCK_DOMAIN,"nonce":request.nonce,"source_ns":source})
    )?;
    output.flush()?;
    Ok(())
}

#[derive(Serialize)]
pub struct HandshakeCheck {
    pub schema_version: u16,
    pub source: &'static str,
    pub status: &'static str,
    pub clock_domain: &'static str,
    pub max_bracket_ns: u64,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub result: Option<HandshakeRun>,
}

pub async fn check() -> HandshakeCheck {
    #[cfg(all(target_os = "linux", not(feature = "observe")))]
    let (status, result) = ("feature_disabled", None);
    #[cfg(not(target_os = "linux"))]
    let (status, result) = ("unsupported", None);
    #[cfg(all(target_os = "linux", feature = "observe"))]
    let (status, result) = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let run = match (
            std::env::current_exe(),
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            }),
        ) {
            (Ok(executable), Ok(nonce)) => {
                let mut command = tokio::process::Command::new(executable);
                command.arg("clock-handshake-reply");
                exchange(
                    command,
                    std::time::Instant::now(),
                    nonce,
                    std::future::pending(),
                )
                .await
                .0
            }
            _ => HandshakeRun {
                schema_version: 1,
                outcome: super::supervisor::Outcome::SpawnFailed,
                helper_started: false,
                helper_reaped: false,
                bracket_width_ns: None,
            },
        };
        let status = if run.outcome == super::supervisor::Outcome::Completed {
            "available"
        } else {
            "unavailable"
        };
        (status, Some(run))
    };
    HandshakeCheck {
        schema_version: 1,
        source: "local_clock_handshake",
        status,
        clock_domain: intellicord_observe_common::CLOCK_DOMAIN,
        max_bracket_ns: MAX_BRACKET_NS,
        sensor_running: false,
        load_test_performed: false,
        result,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandshakeError {
    Reply,
    Bracket,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply<'buffer> {
    schema_version: u16,
    record_type: &'buffer str,
    clock_domain: &'buffer str,
    nonce: u64,
    source_ns: u64,
}

pub struct ClockHandshake {
    source_ns: u64,
    before_ns: u64,
    after_ns: u64,
}

impl ClockHandshake {
    pub fn accept(
        bytes: &[u8],
        nonce: u64,
        before_ns: u64,
        after_ns: u64,
    ) -> Result<Self, HandshakeError> {
        if !framed(bytes) {
            return Err(HandshakeError::Reply);
        }
        let reply: Reply<'_> = serde_json::from_slice(bytes).map_err(|_| HandshakeError::Reply)?;
        if nonce == 0
            || reply.schema_version != 1
            || reply.record_type != "clock_reply"
            || reply.clock_domain != intellicord_observe_common::CLOCK_DOMAIN
            || reply.nonce != nonce
        {
            return Err(HandshakeError::Reply);
        }
        if after_ns
            .checked_sub(before_ns)
            .is_none_or(|width| width > MAX_BRACKET_NS)
            || EventStream::new(1, reply.source_ns, before_ns, after_ns).is_err()
        {
            return Err(HandshakeError::Bracket);
        }
        Ok(Self {
            source_ns: reply.source_ns,
            before_ns,
            after_ns,
        })
    }

    pub fn bracket_width_ns(&self) -> u64 {
        self.after_ns - self.before_ns
    }

    pub fn event_stream(&self, process_id: u32) -> Result<EventStream, StreamError> {
        EventStream::new(process_id, self.source_ns, self.before_ns, self.after_ns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_response_requires_completion_and_supports_cancellation_and_deadline() {
        assert_eq!(
            bounded_response(|| Ok(()), std::future::pending()).await,
            Ok(())
        );
        assert_eq!(
            bounded_response(|| anyhow::bail!("invalid request"), std::future::pending()).await,
            Err(ResponseError::Rejected)
        );
        assert_eq!(
            bounded_response(|| panic!("must not spawn"), std::future::ready(())).await,
            Err(ResponseError::Cancelled)
        );
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let result = bounded_response(
            move || {
                blocked.recv()?;
                Ok(())
            },
            std::future::pending(),
        )
        .await;
        release.send(()).unwrap();
        assert_eq!(result, Err(ResponseError::Deadline));
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let (started, ready) = tokio::sync::oneshot::channel();
        let result = bounded_response(
            move || {
                let _ = started.send(());
                blocked.recv()?;
                Ok(())
            },
            async {
                let _ = ready.await;
            },
        )
        .await;
        release.send(()).unwrap();
        assert_eq!(result, Err(ResponseError::Cancelled));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn subprocess_exchange_rejects_bad_frames_and_requires_exit_and_reaping() {
        use super::super::supervisor::Outcome;
        let code = r#"
            const fs = require('node:fs');
            const mode = process.argv[1];
            const hold = () => setInterval(() => {}, 1000);
            if (mode === 'bad_ready') { process.stdout.write('{}\n'); hold(); }
            else {
                process.stdout.write(JSON.stringify({schema_version:1,record_type:'clock_ready',clock_domain:'linux_monotonic'})+'\n');
                let input = '';
                process.stdin.on('data', chunk => {
                    input += chunk;
                    if (!input.endsWith('\n')) return;
                    const request = JSON.parse(input);
                    if (mode === 'stall') return hold();
                    const send = () => {
                        if (mode === 'oversize') { process.stdout.write('x'.repeat(257)); return hold(); }
                        process.stdout.write(JSON.stringify({schema_version:1,record_type:'clock_reply',clock_domain:'linux_monotonic',
                            nonce:request.nonce+(mode==='nonce'?1:0),source_ns:1000})+'\n');
                        if (mode === 'trailing') process.stdout.write('x');
                        if (mode === 'eof_stall') { fs.closeSync(1); hold(); }
                        if (mode === 'expired') { fs.closeSync(1); setTimeout(() => {}, 1100); }
                        if (mode === 'failure') process.exitCode = 1;
                    };
                    if (mode === 'slow') setTimeout(send, 50); else send();
                });
            }
        "#;
        for (mode, expected) in [
            ("complete", Outcome::Completed),
            ("nonce", Outcome::InvalidStream),
            ("bad_ready", Outcome::InvalidStream),
            ("oversize", Outcome::InvalidStream),
            ("trailing", Outcome::InvalidStream),
            ("slow", Outcome::InvalidStream),
            ("failure", Outcome::CollectorFault),
            ("expired", Outcome::InvalidStream),
            ("stall", Outcome::Deadline),
            ("eof_stall", Outcome::Deadline),
        ] {
            let mut command = tokio::process::Command::new("node");
            command.args(["--max-old-space-size=64", "-e", code, mode]);
            let (run, clock) = exchange(
                command,
                std::time::Instant::now(),
                42,
                std::future::pending(),
            )
            .await;
            assert_eq!(run.outcome, expected, "{mode}");
            assert!(run.helper_started && run.helper_reaped, "{mode}");
            assert_eq!(clock.is_some(), expected == Outcome::Completed);
            assert_eq!(
                run.bracket_width_ns.is_some(),
                expected == Outcome::Completed
            );
        }
        let (cancelled, _) = exchange(
            tokio::process::Command::new("node"),
            std::time::Instant::now(),
            42,
            std::future::ready(()),
        )
        .await;
        assert_eq!(cancelled.outcome, Outcome::Cancelled);
        assert!(!cancelled.helper_started && !cancelled.helper_reaped);
        let mut command = tokio::process::Command::new("node");
        command.args(["--max-old-space-size=64", "-e", code, "stall"]);
        let (cancelled, _) = exchange(command, std::time::Instant::now(), 42, async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        })
        .await;
        assert_eq!(cancelled.outcome, Outcome::Cancelled);
        assert!(cancelled.helper_started && cancelled.helper_reaped);
    }

    #[tokio::test]
    async fn invalid_requests_and_spawn_failures_never_claim_a_helper_or_clock() {
        use super::super::supervisor::Outcome;
        let now = std::time::Instant::now();
        for (nonce, origin, expected) in [
            (0, now, Outcome::InvalidStream),
            (
                42,
                now + std::time::Duration::from_secs(60),
                Outcome::InvalidStream,
            ),
            (42, now, Outcome::SpawnFailed),
        ] {
            let (run, clock) = exchange(
                tokio::process::Command::new("/no-such-intellicord-clock-helper"),
                origin,
                nonce,
                std::future::pending(),
            )
            .await;
            assert_eq!(run.outcome, expected);
            assert!(!run.helper_started && !run.helper_reaped);
            assert!(run.bracket_width_ns.is_none() && clock.is_none());
        }
    }

    fn reply(nonce: u64) -> Vec<u8> {
        format!("{{\"schema_version\":1,\"record_type\":\"clock_reply\",\"clock_domain\":\"linux_monotonic\",\"nonce\":{nonce},\"source_ns\":1000}}\n").into_bytes()
    }

    #[test]
    fn reply_binds_nonce_domain_and_bounded_bracket_before_creating_a_reader() {
        let bytes = reply(42);
        let handshake = ClockHandshake::accept(&bytes, 42, 100, 120).unwrap();
        assert_eq!(handshake.bracket_width_ns(), 20);
        let mut reader = handshake.event_stream(42).unwrap();
        reader
            .push(&super::super::timed::generated_record(42, 2000), 1200)
            .unwrap();
        assert_eq!(reader.finish(true).unwrap().records, 1);
        for (nonce, before, after) in [
            (0, 100, 120),
            (43, 100, 120),
            (42, 120, 100),
            (42, 0, MAX_BRACKET_NS + 1),
            (42, u64::MAX, u64::MAX),
        ] {
            assert!(ClockHandshake::accept(&bytes, nonce, before, after).is_err());
        }
        assert!(ClockHandshake::accept(&bytes, 42, 0, MAX_BRACKET_NS).is_ok());
        for bytes in [
            bytes[..bytes.len() - 1].to_vec(),
            [bytes.clone(), bytes.clone()].concat(),
            vec![b' '; MESSAGE_LIMIT + 1],
            String::from_utf8(bytes.clone())
                .unwrap()
                .replace("linux_monotonic", "boottime")
                .into_bytes(),
            String::from_utf8(bytes.clone())
                .unwrap()
                .replace("\"nonce\":42", "\"nonce\":42,\"extra\":true")
                .into_bytes(),
            String::from_utf8(bytes)
                .unwrap()
                .replace("\"nonce\":42", "\"nonce\":42,\"nonce\":42")
                .into_bytes(),
        ] {
            assert!(ClockHandshake::accept(&bytes, 42, 100, 120).is_err());
        }
    }
}
