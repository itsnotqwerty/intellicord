use std::{process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, process::Command};

use super::{ObjectCheck, supervisor::Outcome};

const REPLY_LIMIT: u64 = 2048;

#[derive(Serialize)]
pub struct Preflight {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub outcome: Outcome,
    pub helper_started: bool,
    pub helper_reaped: bool,
    pub object: Option<ObjectCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    schema_version: u16,
    inspector_available: bool,
    status: String,
    load_test_performed: bool,
    sensor_running: bool,
    #[serde(default)]
    load_probe_available: Option<bool>,
    #[serde(default)]
    code: Option<super::CodeInventory>,
}

fn decode(bytes: &[u8]) -> Option<ObjectCheck> {
    let reply: Reply = serde_json::from_slice(bytes).ok()?;
    if reply.schema_version != 1 || reply.sensor_running || reply.load_test_performed {
        return None;
    }
    if reply
        .load_probe_available
        .is_some_and(|available| available != reply.inspector_available)
    {
        return None;
    }
    let status = match (reply.inspector_available, reply.status.as_str()) {
        (true, "not_configured") => "not_configured",
        (true, "layout_matched") => "layout_matched",
        (true, "rejected") => "rejected",
        (true, "unreadable") => "unreadable",
        (false, "feature_disabled") => "feature_disabled",
        (false, "unsupported") => "unsupported",
        _ => return None,
    };
    if reply
        .code
        .is_some_and(|code| status != "layout_matched" || !code.valid())
    {
        return None;
    }
    let mut check = ObjectCheck::new(reply.inspector_available, status);
    check.load_probe_available = reply.load_probe_available;
    check.code = reply.code;
    Some(check)
}

pub async fn check() -> Preflight {
    let mut report = Preflight {
        schema_version: 1,
        source: "offline_object_preflight",
        sensor_running: false,
        load_test_performed: false,
        outcome: Outcome::SpawnFailed,
        helper_started: false,
        helper_reaped: false,
        object: None,
    };
    let Ok(executable) = std::env::current_exe() else {
        return report;
    };
    let mut command = Command::new(executable);
    command.arg("object-preflight");
    inspect(command, &mut report).await;
    report
}

async fn inspect(mut command: Command, report: &mut Preflight) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        return;
    };
    report.helper_started = true;
    let mut output = child
        .stdout
        .take()
        .expect("piped object helper stdout")
        .take(REPLY_LIMIT + 1);
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        output
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Outcome::ReadFailed)?;
        if bytes.len() as u64 > REPLY_LIMIT {
            return Err(Outcome::InvalidStream);
        }
        let object = decode(&bytes).ok_or(Outcome::InvalidStream)?;
        let exit = child.wait().await.map_err(|_| Outcome::WaitFailed)?;
        report.helper_reaped = true;
        if !exit.success() {
            return Err(Outcome::CollectorFault);
        }
        Ok(object)
    })
    .await;
    match result {
        Ok(Ok(object)) => {
            report.outcome = Outcome::Completed;
            report.object = Some(object);
        }
        Ok(Err(outcome)) => report.outcome = outcome,
        Err(_) => report.outcome = Outcome::Deadline,
    }
    if !report.helper_reaped {
        let _ = child.start_kill();
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(_)) => report.helper_reaped = true,
            _ => report.outcome = Outcome::CleanupFailed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subprocess_output_and_exit_are_bounded_and_fail_closed() {
        for (script, expected) in [
            (
                "process.stdout.write(' '.repeat(4096)); setInterval(() => {}, 1000)",
                Outcome::InvalidStream,
            ),
            ("process.stdout.write('{}');", Outcome::InvalidStream),
            ("setInterval(() => {}, 1000)", Outcome::Deadline),
            (
                "require('node:fs').writeSync(1, JSON.stringify({schema_version:1,inspector_available:true,status:'layout_matched',sensor_running:false,load_test_performed:false})); require('node:fs').closeSync(1); setInterval(() => {}, 1000)",
                Outcome::Deadline,
            ),
            (
                "process.stdout.write(JSON.stringify({schema_version:1,inspector_available:true,status:'layout_matched',sensor_running:false,load_test_performed:false})); process.exitCode = 1",
                Outcome::CollectorFault,
            ),
        ] {
            let mut command = Command::new("node");
            command.args(["-e", script]);
            let mut report = Preflight {
                schema_version: 1,
                source: "offline_object_preflight",
                sensor_running: false,
                load_test_performed: false,
                outcome: Outcome::SpawnFailed,
                helper_started: false,
                helper_reaped: false,
                object: None,
            };
            tokio::time::timeout(Duration::from_secs(6), inspect(command, &mut report))
                .await
                .unwrap();
            assert_eq!(report.outcome, expected);
            assert!(report.helper_started);
            assert!(report.helper_reaped);
            assert!(report.object.is_none());
            assert!(serde_json::to_vec(&report).unwrap().len() < 2048);
        }
    }

    #[test]
    fn code_inventory_requires_matched_layout_and_bounded_consistent_counts() {
        let code = super::super::CodeInventory {
            schema_version: 1,
            functions: 1,
            instruction_slots: 10,
            clock_calls: 1,
            ..super::super::CodeInventory::default()
        };
        let base = serde_json::json!({"schema_version":1,"inspector_available":true,
            "status":"layout_matched","sensor_running":false,"load_test_performed":false,"code":code});
        assert_eq!(
            decode(&serde_json::to_vec(&base).unwrap()).unwrap().code,
            Some(code)
        );
        for (key, value) in [
            ("schema_version", 2),
            ("functions", 0),
            ("functions", 4097),
            ("instruction_slots", 0),
            ("instruction_slots", 524289),
            ("clock_calls", 11),
            ("clock_calls", -1),
            ("unknown", 0),
        ] {
            let mut invalid = base.clone();
            invalid["code"][key] = value.into();
            assert!(
                decode(&serde_json::to_vec(&invalid).unwrap()).is_none(),
                "{key}={value}"
            );
        }
        for status in ["not_configured", "rejected", "unreadable"] {
            let mut invalid = base.clone();
            invalid["status"] = status.into();
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        for key in serde_json::to_value(code)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
        {
            let mut invalid = base.clone();
            invalid["code"].as_object_mut().unwrap().remove(key);
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        let mut invalid = base;
        invalid["code"]["clock_calls"] = u32::MAX.into();
        invalid["code"]["local_calls"] = u32::MAX.into();
        assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
    }

    #[test]
    fn probe_availability_is_build_metadata_not_a_load_result() {
        let base = serde_json::json!({"schema_version":1,"inspector_available":true,
            "status":"not_configured","sensor_running":false,"load_test_performed":false});
        assert_eq!(
            decode(&serde_json::to_vec(&base).unwrap())
                .unwrap()
                .load_probe_available,
            None
        );
        let mut available = base.clone();
        available["load_probe_available"] = true.into();
        assert_eq!(
            decode(&serde_json::to_vec(&available).unwrap())
                .unwrap()
                .load_probe_available,
            Some(true)
        );
        for value in [
            serde_json::json!(false),
            serde_json::json!(1),
            serde_json::json!("true"),
            serde_json::json!({}),
        ] {
            let mut invalid = base.clone();
            invalid["load_probe_available"] = value;
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        available["inspector_available"] = false.into();
        available["status"] = "feature_disabled".into();
        assert!(decode(&serde_json::to_vec(&available).unwrap()).is_none());
        available["load_probe_available"] = false.into();
        assert_eq!(
            decode(&serde_json::to_vec(&available).unwrap())
                .unwrap()
                .load_probe_available,
            Some(false)
        );
    }

    #[test]
    fn reply_validation_rejects_live_claims_unknown_fields_and_contradictions() {
        let base = serde_json::json!({"schema_version":1,"inspector_available":true,
            "status":"layout_matched","sensor_running":false,"load_test_performed":false});
        assert_eq!(
            decode(&serde_json::to_vec(&base).unwrap()).unwrap().status,
            "layout_matched"
        );
        for (key, value) in [
            ("schema_version", serde_json::json!(2)),
            ("sensor_running", serde_json::json!(true)),
            ("load_test_performed", serde_json::json!(true)),
            ("inspector_available", serde_json::json!(false)),
            ("status", serde_json::json!("attached")),
            ("path", serde_json::json!("private")),
        ] {
            let mut invalid = base.clone();
            invalid[key] = value;
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        for key in [
            "schema_version",
            "inspector_available",
            "status",
            "sensor_running",
            "load_test_performed",
        ] {
            let mut invalid = base.clone();
            invalid.as_object_mut().unwrap().remove(key);
            assert!(decode(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
        assert!(decode(b"{}\n{}").is_none());
    }
}
