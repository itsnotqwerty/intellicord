use std::{future::Future, time::Duration};

use tokio::net::{UnixListener, UnixStream};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpectedPeer {
    uid: u32,
    pid: u32,
}

impl ExpectedPeer {
    pub fn new(uid: u32, pid: u32) -> Result<Self, Error> {
        if !(1..=i32::MAX as u32).contains(&pid) {
            return Err(Error::InvalidPolicy);
        }
        Ok(Self { uid, pid })
    }
}

#[derive(Debug)]
pub enum Error {
    InvalidPolicy,
    Cancelled,
    Deadline,
    Accept(std::io::Error),
    Credentials(std::io::Error),
    PeerMismatch,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPolicy => formatter.write_str("invalid process peer acceptance policy"),
            Self::Cancelled => formatter.write_str("process peer acceptance cancelled"),
            Self::Deadline => formatter.write_str("process peer acceptance deadline exceeded"),
            Self::Accept(error) => write!(formatter, "process peer accept failed: {error}"),
            Self::Credentials(error) => {
                write!(formatter, "process peer credentials unavailable: {error}")
            }
            Self::PeerMismatch => formatter.write_str("process peer credentials did not match"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Accept(error) | Self::Credentials(error) => Some(error),
            _ => None,
        }
    }
}

pub struct VerifiedStream {
    stream: UnixStream,
    peer: ExpectedPeer,
}

impl VerifiedStream {
    pub fn peer(&self) -> ExpectedPeer {
        self.peer
    }

    pub fn into_stream(self) -> UnixStream {
        self.stream
    }
}

pub async fn accept(
    listener: &UnixListener,
    expected: ExpectedPeer,
    limit: Duration,
    cancel: impl Future<Output = ()>,
) -> Result<VerifiedStream, Error> {
    if limit.is_zero() || limit > Duration::from_secs(2) {
        return Err(Error::InvalidPolicy);
    }
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(cancel, deadline);
    tokio::select! {
        biased;
        _ = &mut cancel => Err(Error::Cancelled),
        _ = &mut deadline => Err(Error::Deadline),
        accepted = listener.accept() => {
            let (stream, _) = accepted.map_err(Error::Accept)?;
            let credentials = stream.peer_cred().map_err(Error::Credentials)?;
            if credentials.uid() != expected.uid || credentials.pid() != Some(expected.pid as i32) {
                return Err(Error::PeerMismatch);
            }
            Ok(VerifiedStream { stream, peer: expected })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn own_peer() -> ExpectedPeer {
        ExpectedPeer::new(
            std::fs::metadata("/proc/self").unwrap().uid(),
            std::process::id(),
        )
        .unwrap()
    }

    use std::os::unix::fs::MetadataExt;

    fn listener() -> UnixListener {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::{
            os::linux::net::SocketAddrExt,
            os::unix::net::{SocketAddr, UnixListener as StdListener},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "intellicord-peer-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let address = SocketAddr::from_abstract_name(name).unwrap();
        let listener = StdListener::bind_addr(&address).unwrap();
        listener.set_nonblocking(true).unwrap();
        UnixListener::from_std(listener).unwrap()
    }

    async fn connect(listener: &UnixListener) -> UnixStream {
        use std::os::unix::net::UnixStream as StdStream;
        let address: std::os::unix::net::SocketAddr = listener.local_addr().unwrap().into();
        let stream = StdStream::connect_addr(&address).unwrap();
        stream.set_nonblocking(true).unwrap();
        UnixStream::from_std(stream).unwrap()
    }

    #[tokio::test]
    async fn kernel_credentials_gate_stream_without_consuming_payload() {
        let listener = listener();
        let mut client = connect(&listener).await;
        client.write_all(b"health").await.unwrap();
        let expected = own_peer();
        let verified = accept(
            &listener,
            expected,
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await
        .unwrap();
        assert_eq!(verified.peer(), expected);
        let mut stream = verified.into_stream();
        let mut bytes = [0; 6];
        stream.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"health");
    }

    #[tokio::test]
    async fn wrong_uid_or_pid_rejects_and_closes_without_reading() {
        let own = own_peer();
        for expected in [
            ExpectedPeer {
                uid: own.uid.wrapping_add(1),
                ..own
            },
            ExpectedPeer {
                pid: if own.pid == 1 { 2 } else { 1 },
                ..own
            },
        ] {
            let listener = listener();
            let mut client = connect(&listener).await;
            assert!(matches!(
                accept(
                    &listener,
                    expected,
                    Duration::from_secs(1),
                    std::future::pending()
                )
                .await,
                Err(Error::PeerMismatch)
            ));
            let mut bytes = [0];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), client.read(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        }
    }

    #[tokio::test]
    async fn distinct_child_process_credentials_match_owned_child_pid() {
        use std::{os::linux::net::SocketAddrExt, process::Stdio};
        let listener = listener();
        let address: std::os::unix::net::SocketAddr = listener.local_addr().unwrap().into();
        let name = std::str::from_utf8(address.as_abstract_name().unwrap()).unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "observe::progress::peer::tests::connector_child"])
            .env("INTELLICORD_TEST_PEER_SOCKET", name)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let expected = ExpectedPeer::new(own_peer().uid, child.id().unwrap()).unwrap();
        let verified = accept(
            &listener,
            expected,
            Duration::from_secs(2),
            std::future::pending(),
        )
        .await
        .unwrap();
        assert_eq!(verified.peer(), expected);
        assert_ne!(expected.pid, own_peer().pid);
        let mut stream = verified.into_stream();
        let mut bytes = [0; 5];
        tokio::time::timeout(Duration::from_secs(1), stream.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes, b"child");
        assert!(
            tokio::time::timeout(Duration::from_secs(2), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn connector_child() {
        use std::{
            io::Write,
            os::{
                linux::net::SocketAddrExt,
                unix::net::{SocketAddr, UnixStream as StdStream},
            },
        };
        let Ok(name) = std::env::var("INTELLICORD_TEST_PEER_SOCKET") else {
            return;
        };
        let address = SocketAddr::from_abstract_name(name).unwrap();
        StdStream::connect_addr(&address)
            .unwrap()
            .write_all(b"child")
            .unwrap();
    }

    #[tokio::test]
    async fn policy_deadline_and_ready_cancellation_are_fail_closed() {
        assert!(ExpectedPeer::new(0, 0).is_err());
        assert!(ExpectedPeer::new(0, u32::MAX).is_err());
        let listener = listener();
        for limit in [Duration::ZERO, Duration::from_secs(3)] {
            assert!(matches!(
                accept(&listener, own_peer(), limit, std::future::pending()).await,
                Err(Error::InvalidPolicy)
            ));
        }
        assert!(matches!(
            accept(
                &listener,
                own_peer(),
                Duration::from_millis(10),
                std::future::pending()
            )
            .await,
            Err(Error::Deadline)
        ));
        let _client = connect(&listener).await;
        assert!(matches!(
            accept(
                &listener,
                own_peer(),
                Duration::from_secs(1),
                std::future::ready(())
            )
            .await,
            Err(Error::Cancelled)
        ));
        assert!(
            accept(
                &listener,
                own_peer(),
                Duration::from_secs(1),
                std::future::pending()
            )
            .await
            .is_ok()
        );
    }
}
