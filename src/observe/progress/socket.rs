use std::{future::Future, time::Duration};

use tokio::{io::AsyncReadExt, time::Instant};

use super::{
    ProgressStream, Source, StreamError, Summary, peer::VerifiedStream, supervisor::HealthSender,
};

#[derive(Debug)]
pub enum Error {
    InvalidDuration,
    Cancelled,
    TotalDeadline,
    IdleDeadline,
    Stream(StreamError),
    Read(std::io::Error),
    Exit(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDuration => formatter.write_str("invalid progress socket duration"),
            Self::Cancelled => formatter.write_str("progress socket reception cancelled"),
            Self::TotalDeadline => formatter.write_str("progress socket total deadline exceeded"),
            Self::IdleDeadline => formatter.write_str("progress socket idle deadline exceeded"),
            Self::Stream(error) => write!(formatter, "progress socket stream rejected: {error}"),
            Self::Read(error) => write!(formatter, "progress socket read failed: {error}"),
            Self::Exit(error) => write!(
                formatter,
                "progress socket exit observation failed: {error}"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Stream(error) => Some(error),
            Self::Read(error) | Self::Exit(error) => Some(error),
            _ => None,
        }
    }
}

pub struct Limits {
    pub total: Duration,
    pub idle: Duration,
}

pub async fn receive(
    verified: VerifiedStream,
    session: &str,
    source: Source,
    limits: Limits,
    exit: impl Future<Output = std::io::Result<bool>>,
    cancel: impl Future<Output = ()>,
    health: HealthSender,
) -> Result<Summary, Error> {
    if limits.total.is_zero()
        || limits.total > Duration::from_secs(65)
        || limits.idle.is_zero()
        || limits.idle > limits.total
    {
        return Err(Error::InvalidDuration);
    }
    let mut stream = ProgressStream::new(session, source).map_err(Error::Stream)?;
    let mut socket = verified.into_stream();
    let origin = Instant::now();
    let total = tokio::time::sleep_until(origin + limits.total);
    let idle = tokio::time::sleep_until(origin + limits.idle);
    tokio::pin!(exit, cancel, total, idle);
    let mut eof = false;
    let mut observed_exit = None;
    let mut buffer = [0_u8; 1024];
    loop {
        if eof && let Some(success) = observed_exit {
            return stream.finish(success).map_err(Error::Stream);
        }
        tokio::select! {
            biased;
            _ = &mut cancel => return Err(Error::Cancelled),
            _ = &mut total => return Err(Error::TotalDeadline),
            _ = &mut idle => return Err(Error::IdleDeadline),
            read = socket.read(&mut buffer), if !eof => {
                let count = read.map_err(Error::Read)?;
                if count == 0 {
                    eof = true;
                } else {
                    let previous = stream.records();
                    let now = Instant::now();
                    let receipt = u64::try_from(now.duration_since(origin).as_nanos())
                        .map_err(|_| Error::Stream(StreamError::InvalidRecord))?;
                    stream.push(&buffer[..count], receipt).map_err(Error::Stream)?;
                    health.publish(&stream, source, origin);
                    if stream.records() > previous {
                        idle.as_mut().reset(now + limits.idle);
                    }
                }
            }
            result = &mut exit, if observed_exit.is_none() => {
                observed_exit = Some(result.map_err(Error::Exit)?);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::progress::{
        generated_stream,
        peer::{ExpectedPeer, accept},
        supervisor::health_channel,
    };
    use std::os::unix::fs::MetadataExt;
    use tokio::{
        io::AsyncWriteExt,
        net::{UnixListener, UnixStream},
    };

    const SESSION: &str = "00000000000000000000000000000001";

    async fn connected() -> (VerifiedStream, UnixStream) {
        use std::{
            os::{
                linux::net::SocketAddrExt,
                unix::net::{SocketAddr, UnixListener as StdListener, UnixStream as StdStream},
            },
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let address = SocketAddr::from_abstract_name(format!(
            "intellicord-progress-socket-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
        .unwrap();
        let listener = StdListener::bind_addr(&address).unwrap();
        listener.set_nonblocking(true).unwrap();
        let listener = UnixListener::from_std(listener).unwrap();
        let client = StdStream::connect_addr(&address).unwrap();
        client.set_nonblocking(true).unwrap();
        let client = UnixStream::from_std(client).unwrap();
        let expected = ExpectedPeer::new(
            std::fs::metadata("/proc/self").unwrap().uid(),
            std::process::id(),
        )
        .unwrap();
        let verified = accept(
            &listener,
            expected,
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await
        .unwrap();
        (verified, client)
    }

    fn limits() -> Limits {
        Limits {
            total: Duration::from_secs(2),
            idle: Duration::from_millis(200),
        }
    }

    #[tokio::test]
    async fn verified_socket_publishes_interim_health_and_revokes_on_completion() {
        let (verified, mut client) = connected().await;
        let bytes = generated_stream();
        let records: Vec<_> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
        let prefix = records[..6].concat();
        let suffix = records[6..].concat();
        let (sender, mut receiver) = health_channel();
        let retained = receiver.clone();
        let producer = async move {
            for fragment in prefix.chunks(7) {
                client.write_all(fragment).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            client.write_all(&suffix).await.unwrap();
            client.shutdown().await.unwrap();
        };
        let consumer = async move {
            let mut observed = false;
            while receiver.changed().await.is_ok() {
                if let Some(sample) = receiver.latest(Duration::from_secs(1)) {
                    assert_eq!(sample.source, Source::GeneratedReplay);
                    assert!((1..=2).contains(&sample.health.accepted));
                    observed = true;
                }
            }
            observed
        };
        let (summary, _, observed) = tokio::join!(
            receive(
                verified,
                SESSION,
                Source::GeneratedReplay,
                limits(),
                std::future::ready(Ok(true)),
                std::future::pending(),
                sender
            ),
            producer,
            consumer
        );
        let summary = summary.unwrap();
        assert_eq!(summary.records, 8);
        assert_eq!(summary.health_samples, 2);
        assert!(observed);
        assert!(retained.latest(Duration::MAX).is_none());
    }

    #[tokio::test]
    async fn published_health_is_revoked_when_cancelled_or_aborted() {
        for abort in [false, true] {
            let (verified, mut client) = connected().await;
            let bytes = generated_stream();
            let prefix: Vec<u8> = bytes
                .split_inclusive(|byte| *byte == b'\n')
                .take(6)
                .flatten()
                .copied()
                .collect();
            client.write_all(&prefix).await.unwrap();
            let (sender, mut receiver) = health_channel();
            let (cancel, cancelled) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(receive(
                verified,
                SESSION,
                Source::GeneratedReplay,
                Limits {
                    total: Duration::from_secs(3),
                    idle: Duration::from_secs(2),
                },
                std::future::pending(),
                async {
                    let _ = cancelled.await;
                },
                sender,
            ));
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    receiver.changed().await.unwrap();
                    if receiver.latest(Duration::from_secs(1)).is_some() {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            if abort {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                cancel.send(()).unwrap();
                assert!(matches!(task.await.unwrap(), Err(Error::Cancelled)));
            }
            assert!(receiver.latest(Duration::MAX).is_none());
        }
    }

    #[tokio::test]
    async fn terminal_status_revokes_health_before_socket_eof() {
        let (verified, mut client) = connected().await;
        let bytes = generated_stream();
        let records: Vec<_> = bytes.split_inclusive(|byte| *byte == b'\n').collect();
        client.write_all(&records[..6].concat()).await.unwrap();
        let (sender, mut receiver) = health_channel();
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(receive(
            verified,
            SESSION,
            Source::GeneratedReplay,
            Limits {
                total: Duration::from_secs(3),
                idle: Duration::from_secs(2),
            },
            std::future::ready(Ok(true)),
            async {
                let _ = cancelled.await;
            },
            sender,
        ));
        tokio::time::timeout(Duration::from_secs(1), async {
            while receiver.latest(Duration::from_secs(1)).is_none() {
                receiver.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        client.write_all(&records[6..].concat()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while receiver.latest(Duration::MAX).is_some() {
                receiver.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished());
        cancel.send(()).unwrap();
        assert!(matches!(task.await.unwrap(), Err(Error::Cancelled)));
        assert!(receiver.latest(Duration::MAX).is_none());
    }

    #[tokio::test]
    async fn socket_terminal_eof_and_exit_must_agree() {
        for mode in [
            "foreign",
            "malformed",
            "truncated",
            "exit_mismatch",
            "eof_no_exit",
            "terminal_no_eof",
        ] {
            let (verified, mut client) = connected().await;
            let mut bytes = generated_stream();
            if mode == "malformed" {
                bytes = b"{}\n".to_vec();
            }
            if mode == "truncated" {
                bytes.pop();
            }
            client.write_all(&bytes).await.unwrap();
            if mode != "terminal_no_eof" {
                client.shutdown().await.unwrap();
            }
            let (sender, receiver) = health_channel();
            let exit = async {
                if mode == "eof_no_exit" {
                    std::future::pending::<()>().await;
                }
                Ok(mode != "exit_mismatch")
            };
            let result = receive(
                verified,
                if mode == "foreign" {
                    "00000000000000000000000000000002"
                } else {
                    SESSION
                },
                Source::GeneratedReplay,
                limits(),
                exit,
                std::future::pending(),
                sender,
            )
            .await;
            if matches!(mode, "eof_no_exit" | "terminal_no_eof") {
                assert!(matches!(result, Err(Error::IdleDeadline)), "{mode}");
            } else {
                assert!(matches!(result, Err(Error::Stream(_))), "{mode}");
            }
            assert!(receiver.latest(Duration::MAX).is_none());
        }
    }

    #[tokio::test]
    async fn dribbles_deadlines_cancellation_exit_failure_and_policy_are_bounded() {
        let (verified, mut client) = connected().await;
        let (sender, receiver) = health_channel();
        let dribbles = async move {
            loop {
                if client.write_all(b" ").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let (result, _) = tokio::join!(
            receive(
                verified,
                SESSION,
                Source::GeneratedReplay,
                limits(),
                std::future::pending(),
                std::future::pending(),
                sender
            ),
            dribbles
        );
        assert!(matches!(result, Err(Error::IdleDeadline)));
        assert!(receiver.latest(Duration::MAX).is_none());
        for mode in [
            "total",
            "cancel",
            "exit_failure",
            "invalid_policy",
            "bad_session",
        ] {
            let (verified, _client) = connected().await;
            let (sender, receiver) = health_channel();
            let selected = match mode {
                "invalid_policy" => Limits {
                    total: Duration::ZERO,
                    idle: Duration::ZERO,
                },
                "total" => Limits {
                    total: Duration::from_millis(10),
                    idle: Duration::from_millis(10),
                },
                _ => limits(),
            };
            let exit = async {
                if mode != "exit_failure" {
                    std::future::pending::<()>().await;
                }
                Err(std::io::ErrorKind::Other.into())
            };
            let cancel = async {
                if mode != "cancel" {
                    std::future::pending::<()>().await;
                }
            };
            let result = receive(
                verified,
                if mode == "bad_session" {
                    "bad"
                } else {
                    SESSION
                },
                Source::GeneratedReplay,
                selected,
                exit,
                cancel,
                sender,
            )
            .await;
            assert!(
                match mode {
                    "total" => matches!(result, Err(Error::TotalDeadline)),
                    "cancel" => matches!(result, Err(Error::Cancelled)),
                    "exit_failure" => matches!(result, Err(Error::Exit(_))),
                    "invalid_policy" => matches!(result, Err(Error::InvalidDuration)),
                    _ => matches!(result, Err(Error::Stream(_))),
                },
                "{mode}"
            );
            assert!(receiver.latest(Duration::MAX).is_none());
        }
    }
}
