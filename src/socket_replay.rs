use std::{
    os::{
        linux::net::SocketAddrExt,
        unix::net::{SocketAddr, UnixListener as StdListener, UnixStream as StdStream},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use intellicord::observe::{
    progress::{socket_owner, supervisor::health_channel},
    stream::Source,
    supervisor::Outcome,
};
use tokio::net::{UnixListener, UnixStream};

const BINDING: &str = "INTELLICORD_GENERATED_SOCKET_SESSION";
const PREFIX: &str = "intellicord-generated-progress-";

fn address(session: &str) -> Result<SocketAddr> {
    ensure!(
        session.len() == 32
            && session
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "Invalid generated socket replay binding"
    );
    Ok(SocketAddr::from_abstract_name(format!(
        "{PREFIX}{session}"
    ))?)
}

pub async fn supervise() -> Result<()> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).context("Generated socket replay entropy unavailable")?;
    let session: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let listener = StdListener::bind_addr(&address(&session)?)?;
    listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(listener)?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .arg("socket-progress-replay-child")
        .env_clear()
        .env(BINDING, &session);
    let (sender, mut receiver) = health_channel();
    let retained = receiver.clone();
    let owner = socket_owner::supervise(
        command,
        listener,
        &session,
        Source::GeneratedReplay,
        socket_owner::Limits {
            total: Duration::from_secs(6),
            idle: Duration::from_secs(3),
            accept: Duration::from_secs(2),
        },
        async {
            tokio::select! {
                _ = terminate.recv() => {},
                _ = interrupt.recv() => {},
            }
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
    let health_revoked = retained.latest(Duration::MAX).is_none();
    println!(
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "source": "generated_replay",
            "sensor_running": false,
            "load_test_performed": false,
            "transport": "kernel_verified_unix_socket",
            "session_binding": "parent_selected",
            "observed_health": observed_health,
            "generated_only": generated_only,
            "health_revoked": health_revoked,
            "run": run,
        })
    );
    ensure!(
        run.outcome == Outcome::Completed && observed_health && generated_only && health_revoked,
        "Generated socket progress supervision did not pass"
    );
    Ok(())
}

pub async fn child() -> Result<()> {
    let session = std::env::var(BINDING).context("Generated socket replay binding required")?;
    let stream = StdStream::connect_addr(&address(&session)?)?;
    stream.set_nonblocking(true)?;
    let mut stream = UnixStream::from_std(stream)?;
    super::write_periodic_progress(&mut stream, &session).await?;
    tokio::io::AsyncWriteExt::shutdown(&mut stream).await?;
    Ok(())
}
