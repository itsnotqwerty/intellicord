use std::{path::Path, process::Stdio, time::Duration};

use aya::{
    Ebpf,
    maps::{Array, MapData, PerCpuArray, RingBuf},
    programs::{TracePoint, trace_point::TracePointLinkId},
};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    time::timeout,
};

use crate::observe::{
    object::{inspect_object, read_object},
    target::TargetGuard,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    ObjectUnreadable,
    ObjectRejected,
    MapSetupFailed,
    ProgramLoadFailed,
    ChildFailed,
    TargetSetupFailed,
    AttachFailed,
    TriggerFailed,
    DeliveryTimeout,
    EventRejected,
    ReadFailed,
    RingDrops,
    CleanupFailed,
    Verified,
}

#[derive(Serialize)]
pub struct Report {
    pub schema_version: u16,
    pub source: &'static str,
    pub outcome: Outcome,
    pub program_loaded: bool,
    pub child_started: bool,
    pub child_reaped: bool,
    pub attachment_confirmed: bool,
    pub detachment_confirmed: bool,
    pub event_verified: bool,
    pub target_cleared: bool,
    pub owned_handles_dropped: bool,
    pub sensor_running: bool,
}

impl Report {
    fn new() -> Self {
        Self {
            schema_version: 1,
            source: "explicit_kernel_delivery_probe",
            outcome: Outcome::ObjectUnreadable,
            program_loaded: false,
            child_started: false,
            child_reaped: false,
            attachment_confirmed: false,
            detachment_confirmed: false,
            event_verified: false,
            target_cleared: false,
            owned_handles_dropped: true,
            sensor_running: false,
        }
    }
}

#[derive(Default)]
struct Resources {
    bpf: Option<Ebpf>,
    target: Option<Array<MapData, u32>>,
    ring: Option<RingBuf<MapData>>,
    dropped: Option<PerCpuArray<MapData, u64>>,
    child: Option<Child>,
    link: Option<TracePointLinkId>,
}

impl Resources {
    fn program(&mut self) -> Result<&mut TracePoint, Outcome> {
        self.bpf
            .as_mut()
            .and_then(|bpf| bpf.program_mut("process_exec"))
            .and_then(|program| program.try_into().ok())
            .ok_or(Outcome::ProgramLoadFailed)
    }

    async fn run(&mut self, bytes: &[u8], report: &mut Report) -> Result<(), Outcome> {
        let mut bpf = Ebpf::load(bytes).map_err(|_| Outcome::MapSetupFailed)?;
        self.target = Some(
            Array::try_from(bpf.take_map("TARGET").ok_or(Outcome::MapSetupFailed)?)
                .map_err(|_| Outcome::MapSetupFailed)?,
        );
        self.ring = Some(
            RingBuf::try_from(bpf.take_map("EVENTS").ok_or(Outcome::MapSetupFailed)?)
                .map_err(|_| Outcome::MapSetupFailed)?,
        );
        self.dropped = Some(
            PerCpuArray::try_from(bpf.take_map("DROPPED").ok_or(Outcome::MapSetupFailed)?)
                .map_err(|_| Outcome::MapSetupFailed)?,
        );
        self.bpf = Some(bpf);
        self.program()?
            .load()
            .map_err(|_| Outcome::ProgramLoadFailed)?;
        report.program_loaded = true;

        self.child = Some(
            Command::new(std::env::current_exe().map_err(|_| Outcome::ChildFailed)?)
                .arg("delivery-child")
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| Outcome::ChildFailed)?,
        );
        report.child_started = true;
        let child = self.child.as_mut().ok_or(Outcome::ChildFailed)?;
        let process_id = child.id().ok_or(Outcome::ChildFailed)?;
        let guard = TargetGuard::open(process_id).map_err(|_| Outcome::ChildFailed)?;
        let mut output = child.stdout.take().ok_or(Outcome::ChildFailed)?;
        let mut ready = [0];
        timeout(Duration::from_secs(2), output.read_exact(&mut ready))
            .await
            .map_err(|_| Outcome::ChildFailed)?
            .map_err(|_| Outcome::ChildFailed)?;
        if ready != *b"R" {
            return Err(Outcome::ChildFailed);
        }
        let target = self.target.as_mut().ok_or(Outcome::TargetSetupFailed)?;
        target
            .set(0, process_id, 0)
            .map_err(|_| Outcome::TargetSetupFailed)?;
        if target.get(&0, 0).map_err(|_| Outcome::TargetSetupFailed)? != process_id
            || !guard.is_alive().map_err(|_| Outcome::ChildFailed)?
        {
            return Err(Outcome::TargetSetupFailed);
        }
        self.link = Some(
            self.program()?
                .attach("sched", "sched_process_exec")
                .map_err(|_| Outcome::AttachFailed)?,
        );
        report.attachment_confirmed = true;
        if !guard.is_alive().map_err(|_| Outcome::ChildFailed)? {
            return Err(Outcome::ChildFailed);
        }
        let before = monotonic_ns()?;
        let mut input = self
            .child
            .as_mut()
            .and_then(|child| child.stdin.take())
            .ok_or(Outcome::TriggerFailed)?;
        timeout(Duration::from_millis(100), input.write_all(b"X"))
            .await
            .map_err(|_| Outcome::TriggerFailed)?
            .map_err(|_| Outcome::TriggerFailed)?;
        drop(input);
        let result = timeout(Duration::from_secs(2), async {
            loop {
                for _ in 0..64 {
                    let Some(record) = self.ring.as_mut().ok_or(Outcome::ReadFailed)?.next() else {
                        break;
                    };
                    if report.event_verified
                        || !super::matches_exec(&record, process_id, before, monotonic_ns()?)
                    {
                        return Err(Outcome::EventRejected);
                    }
                    report.event_verified = true;
                }
                if !guard.is_alive().map_err(|_| Outcome::ChildFailed)? && report.event_verified {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        result.map_err(|_| Outcome::DeliveryTimeout)?
    }

    async fn finish(mut self, report: &mut Report) {
        let mut cleanup_ok = true;
        if let Some(target) = self.target.as_mut() {
            report.target_cleared =
                target.set(0, 0, 0).is_ok() && matches!(target.get(&0, 0), Ok(0));
            cleanup_ok &= report.target_cleared;
        }
        if let Some(link) = self.link.take() {
            report.detachment_confirmed = self
                .program()
                .is_ok_and(|program| program.detach(link).is_ok());
            cleanup_ok &= report.detachment_confirmed;
        }
        if report.outcome == Outcome::Verified {
            report.outcome = if self.ring.as_mut().is_some_and(|ring| ring.next().is_some()) {
                Outcome::EventRejected
            } else {
                match self
                    .dropped
                    .as_ref()
                    .and_then(|counter| counter.get(&0, 0).ok())
                {
                    Some(values)
                        if intellicord_observe_common::ring_drop_total(values.iter().copied())
                            == 0 =>
                    {
                        Outcome::Verified
                    }
                    Some(_) => Outcome::RingDrops,
                    None => Outcome::ReadFailed,
                }
            };
        }
        drop(self.bpf.take());
        drop(self.target.take());
        drop(self.ring.take());
        drop(self.dropped.take());
        report.owned_handles_dropped = true;
        if let Some(child) = self.child.as_mut()
            && !report.child_reaped
        {
            let _ = child.start_kill();
            if let Ok(Ok(status)) = timeout(Duration::from_secs(2), child.wait()).await {
                report.child_reaped = true;
                if report.outcome == Outcome::Verified && !status.success() {
                    report.outcome = Outcome::ChildFailed;
                }
            }
            cleanup_ok &= report.child_reaped;
        }
        if !cleanup_ok {
            report.outcome = Outcome::CleanupFailed;
        }
    }
}

fn monotonic_ns() -> Result<u64, Outcome> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err(Outcome::ReadFailed);
    }
    u64::try_from(time.tv_sec)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000_000))
        .and_then(|base| {
            u64::try_from(time.tv_nsec)
                .ok()
                .and_then(|part| base.checked_add(part))
        })
        .ok_or(Outcome::ReadFailed)
}

pub async fn verify_path(path: &Path) -> Report {
    let mut report = Report::new();
    let Ok(bytes) = read_object(path) else {
        return report;
    };
    if inspect_object(&bytes).is_err() {
        report.outcome = Outcome::ObjectRejected;
        return report;
    }
    let mut resources = Resources::default();
    report.owned_handles_dropped = false;
    report.outcome = match resources.run(&bytes, &mut report).await {
        Ok(()) => Outcome::Verified,
        Err(outcome) => outcome,
    };
    resources.finish(&mut report).await;
    report
}

pub fn child_helper() -> anyhow::Result<()> {
    use std::{
        io::{Read, Write},
        os::unix::process::CommandExt,
    };
    std::io::stdout().write_all(b"R")?;
    std::io::stdout().flush()?;
    let mut input = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    anyhow::ensure!(
        unsafe { libc::poll(&mut input, 1, 5000) } > 0,
        "child release deadline"
    );
    let mut token = [0];
    std::io::stdin().read_exact(&mut token)?;
    anyhow::ensure!(token == *b"X", "invalid child release");
    let error = std::process::Command::new(std::env::current_exe()?)
        .arg("delivery-child-exit")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .exec();
    Err(error.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_probe_cleanup_reaps_its_waiting_child_without_kernel_handles() {
        let child = Command::new("node")
            .args(["-e", "process.stdin.resume()"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let process_id = child.id().unwrap();
        let resources = Resources {
            child: Some(child),
            ..Resources::default()
        };
        let mut report = Report::new();
        report.outcome = Outcome::ChildFailed;
        report.child_started = true;
        report.owned_handles_dropped = false;
        resources.finish(&mut report).await;
        assert_eq!(report.outcome, Outcome::ChildFailed);
        assert!(report.child_reaped);
        assert!(report.owned_handles_dropped);
        assert!(!report.detachment_confirmed);
        assert_eq!(unsafe { libc::kill(process_id as i32, 0) }, -1);
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.len() < 1024);
        for forbidden in ["process_id", "path", "monotonic_ns", "verifier_log"] {
            assert!(!json.contains(forbidden));
        }
    }
}
