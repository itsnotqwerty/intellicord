#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs,
        os::unix::{fs::MetadataExt, process::CommandExt},
        path::{Path, PathBuf},
        process::Command,
    };

    use anyhow::{Context, Result, ensure};
    use caps::{CapSet, Capability};

    const COLLECTOR: &str = "/usr/libexec/intellicord/intellicord-observe";
    const OBJECT: &str = "/usr/lib/intellicord/intellicord-process";

    struct Request {
        process_id: u32,
        seconds: u64,
        session: String,
        socket: PathBuf,
        parent_id: u32,
    }

    fn parse(arguments: &[String]) -> Result<Request> {
        ensure!(
            arguments.len() == 12
                && arguments[0] == "capture-socket-progress"
                && arguments[1] == "--pid"
                && arguments[3] == "--seconds"
                && arguments[5] == "--session"
                && arguments[7] == "--socket"
                && arguments[9] == "--parent-pid"
                && arguments[11] == "--acknowledge-privileged-observation",
            "Usage: intellicord-bpf-helper capture-socket-progress --pid PID --seconds 1..60 --session HEX32 --socket PATH --parent-pid PID --acknowledge-privileged-observation"
        );
        let process_id = arguments[2].parse::<u32>()?;
        let seconds = arguments[4].parse::<u64>()?;
        let session = arguments[6].clone();
        let socket = PathBuf::from(&arguments[8]);
        let parent_id = arguments[10].parse::<u32>()?;
        ensure!(
            (1..=i32::MAX as u32).contains(&process_id)
                && (1..=60).contains(&seconds)
                && session.len() == 32
                && session.bytes().all(|byte| byte.is_ascii_hexdigit())
                && socket.is_absolute()
                && (1..=i32::MAX as u32).contains(&parent_id),
            "invalid privileged observation request"
        );
        Ok(Request {
            process_id,
            seconds,
            session,
            socket,
            parent_id,
        })
    }

    fn validate_owned_file(path: &Path, executable: bool) -> Result<()> {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("required system artifact unavailable: {}", path.display()))?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == 0
                && metadata.mode() & 0o022 == 0
                && (!executable || metadata.mode() & 0o111 != 0),
            "system artifact is not a root-owned protected regular file: {}",
            path.display()
        );
        Ok(())
    }

    fn run(arguments: &[String]) -> Result<()> {
        let request = parse(arguments)?;
        ensure!(
            request.parent_id == unsafe { libc::getppid() } as u32,
            "privileged observation parent mismatch"
        );
        let target = fs::metadata(format!("/proc/{}", request.process_id))?;
        ensure!(
            target.uid() == unsafe { libc::geteuid() },
            "observation target belongs to another user"
        );
        validate_owned_file(Path::new(COLLECTOR), true)?;
        validate_owned_file(Path::new(OBJECT), false)?;

        for capability in [
            Capability::CAP_BPF,
            Capability::CAP_PERFMON,
            Capability::CAP_DAC_READ_SEARCH,
        ] {
            caps::raise(None, CapSet::Inheritable, capability)?;
            caps::raise(None, CapSet::Ambient, capability)?;
        }

        let error = Command::new(COLLECTOR)
            .env_clear()
            .arg("capture-socket-progress")
            .arg("--object")
            .arg(OBJECT)
            .args(["--pid", &request.process_id.to_string()])
            .args(["--seconds", &request.seconds.to_string()])
            .args(["--session", &request.session])
            .arg("--socket")
            .arg(&request.socket)
            .args(["--parent-pid", &request.parent_id.to_string()])
            .arg("--acknowledge-socket-progress")
            .exec();
        Err(error).context("unable to execute constrained process collector")
    }

    pub fn main() -> Result<()> {
        run(&std::env::args().skip(1).collect::<Vec<_>>())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn valid() -> Vec<String> {
            [
                "capture-socket-progress",
                "--pid",
                "123",
                "--seconds",
                "5",
                "--session",
                "0123456789abcdef0123456789abcdef",
                "--socket",
                "/tmp/intellicord/progress.sock",
                "--parent-pid",
                &std::process::id().to_string(),
                "--acknowledge-privileged-observation",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        }

        #[test]
        fn accepts_only_the_fixed_collection_contract() {
            let request = parse(&valid()).unwrap();
            assert_eq!(request.process_id, 123);
            assert_eq!(request.seconds, 5);
            assert_eq!(request.parent_id, std::process::id());
            for (index, value) in [
                (0, "verify-load"),
                (1, "--object"),
                (3, "--duration"),
                (11, "--acknowledge-socket-progress"),
            ] {
                let mut invalid = valid();
                invalid[index] = value.to_owned();
                assert!(parse(&invalid).is_err());
            }
            for (index, value) in [
                (2, "0"),
                (4, "61"),
                (6, "fixture"),
                (8, "relative.sock"),
                (10, "0"),
            ] {
                let mut invalid = valid();
                invalid[index] = value.to_owned();
                assert!(parse(&invalid).is_err());
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("intellicord-bpf-helper requires Linux")
}
