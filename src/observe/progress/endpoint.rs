use std::{
    fs, io,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt},
        net::UnixListener as StdListener,
    },
    path::{Path, PathBuf},
};

use tokio::net::UnixListener;

pub struct Endpoint {
    directory: PathBuf,
    directory_identity: (u64, u64),
    socket_identity: Option<(u64, u64)>,
    closed: bool,
}

fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

impl Endpoint {
    pub fn bind(root: &Path, session: &str) -> io::Result<(Self, UnixListener)> {
        if session.len() != 32 || !session.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid endpoint session",
            ));
        }
        let directory = fs::canonicalize(root)?.join(format!("ic-progress-{session}"));
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let metadata = fs::symlink_metadata(&directory)?;
        let mut endpoint = Self {
            directory,
            directory_identity: identity(&metadata),
            socket_identity: None,
            closed: false,
        };
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "endpoint directory is not private",
            ));
        }
        let listener = StdListener::bind(endpoint.path())?;
        endpoint.socket_identity = Some(identity(&fs::symlink_metadata(endpoint.path())?));
        listener.set_nonblocking(true)?;
        Ok((endpoint, UnixListener::from_std(listener)?))
    }

    pub fn path(&self) -> PathBuf {
        self.directory.join("progress.sock")
    }

    pub fn close(mut self) -> io::Result<()> {
        self.closed = true;
        self.remove_owned()
    }

    fn remove_owned(&self) -> io::Result<()> {
        let directory = fs::symlink_metadata(&self.directory)?;
        if !directory.is_dir() || identity(&directory) != self.directory_identity {
            return Err(io::Error::other("endpoint directory identity changed"));
        }
        if let Some(expected) = self.socket_identity {
            match fs::symlink_metadata(self.path()) {
                Ok(metadata)
                    if metadata.file_type().is_socket() && identity(&metadata) == expected =>
                {
                    fs::remove_file(self.path())?;
                }
                Ok(_) => return Err(io::Error::other("endpoint socket identity changed")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        fs::remove_dir(&self.directory)
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.remove_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> String {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).unwrap();
        format!("{:032x}", u128::from_be_bytes(random))
    }

    #[tokio::test]
    async fn private_endpoint_is_removed_on_close_and_drop() {
        for explicit in [false, true] {
            let (endpoint, listener) = Endpoint::bind(&std::env::temp_dir(), &session()).unwrap();
            let path = endpoint.path();
            assert!(path.is_absolute());
            let directory = fs::symlink_metadata(path.parent().unwrap()).unwrap();
            assert_eq!(directory.mode() & 0o777, 0o700);
            assert_eq!(directory.uid(), unsafe { libc::geteuid() });
            assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
            drop(listener);
            if explicit {
                endpoint.close().unwrap();
            } else {
                drop(endpoint);
            }
            assert!(!path.parent().unwrap().exists());
        }
    }

    #[tokio::test]
    async fn collision_and_invalid_binding_never_remove_existing_contents() {
        let session = session();
        let (endpoint, listener) = Endpoint::bind(&std::env::temp_dir(), &session).unwrap();
        assert!(Endpoint::bind(&std::env::temp_dir(), &session).is_err());
        assert!(endpoint.path().exists());
        for invalid in ["", "../escape", "fffffffffffffffffffffffffffffff/"] {
            assert!(Endpoint::bind(&std::env::temp_dir(), invalid).is_err());
        }
        drop(listener);
        endpoint.close().unwrap();
    }

    #[tokio::test]
    async fn bind_failure_removes_only_the_new_directory() {
        let (endpoint, listener) = Endpoint::bind(&std::env::temp_dir(), &session()).unwrap();
        let root = endpoint.path().parent().unwrap().to_path_buf();
        assert!(Endpoint::bind(&root, &session()).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert!(endpoint.path().exists());
        drop(listener);
        endpoint.close().unwrap();
    }

    #[tokio::test]
    async fn cleanup_refuses_replaced_socket_and_nonempty_directory() {
        for replace in [false, true] {
            let (endpoint, listener) = Endpoint::bind(&std::env::temp_dir(), &session()).unwrap();
            let path = endpoint.path();
            drop(listener);
            let foreign = if replace {
                fs::remove_file(&path).unwrap();
                path.clone()
            } else {
                path.parent().unwrap().join("foreign")
            };
            fs::write(&foreign, b"preserve").unwrap();
            assert!(endpoint.close().is_err());
            assert_eq!(fs::read(&foreign).unwrap(), b"preserve");
            fs::remove_file(foreign).unwrap();
            fs::remove_dir(path.parent().unwrap()).unwrap();
        }
    }

    #[tokio::test]
    async fn cleanup_refuses_a_replacement_directory() {
        let (endpoint, listener) = Endpoint::bind(&std::env::temp_dir(), &session()).unwrap();
        let original = endpoint.path().parent().unwrap().to_path_buf();
        let moved = original.with_extension("held");
        fs::rename(&original, &moved).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&original).unwrap();
        let foreign = original.join("preserve");
        fs::write(&foreign, b"foreign").unwrap();
        drop(listener);
        assert!(endpoint.close().is_err());
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert!(moved.join("progress.sock").exists());
        fs::remove_file(foreign).unwrap();
        fs::remove_dir(original).unwrap();
        fs::remove_file(moved.join("progress.sock")).unwrap();
        fs::remove_dir(moved).unwrap();
    }
}
