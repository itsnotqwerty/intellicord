use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

pub struct TargetGuard {
    descriptor: OwnedFd,
}

impl TargetGuard {
    pub fn open(process_id: u32) -> io::Result<Self> {
        if !(1..=i32::MAX as u32).contains(&process_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "process ID out of range",
            ));
        }
        let descriptor =
            unsafe { libc::syscall(libc::SYS_pidfd_open, process_id as libc::pid_t, 0) };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            descriptor: unsafe { OwnedFd::from_raw_fd(descriptor as i32) },
        })
    }

    pub fn is_alive(&self) -> io::Result<bool> {
        let mut descriptor = libc::pollfd {
            fd: self.descriptor.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if descriptor.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(io::Error::other("process lifetime handle failed"));
        }
        Ok(descriptor.revents & (libc::POLLIN | libc::POLLHUP) == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_alive_and_invalid_ids_are_rejected() {
        let guard = TargetGuard::open(std::process::id()).unwrap();
        assert!(guard.is_alive().unwrap());
        let flags = unsafe { libc::fcntl(guard.descriptor.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        for process_id in [0, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(
                TargetGuard::open(process_id).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
