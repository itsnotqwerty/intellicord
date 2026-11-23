use std::process::Command;

#[tokio::test]
async fn injected_observation_uses_matched_diagnostics_and_awaited_detach_cleanup() {
    use futures_util::{SinkExt, StreamExt};
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
    };
    use tokio_tungstenite::tungstenite::Message;

    let mut modes: Vec<(bool, Option<i32>, u8)> = vec![(false, None, 0)];
    if cfg!(all(target_os = "linux", feature = "observe")) {
        modes.extend([(true, None, 0), (true, None, 1), (true, None, 2)]);
    }
    #[cfg(all(target_os = "linux", feature = "observe"))]
    modes.extend([
        (true, Some(libc::SIGINT), 0),
        (true, Some(libc::SIGTERM), 0),
        (true, Some(libc::SIGINT), 1),
        (true, Some(libc::SIGTERM), 2),
    ]);
    for (observe, signal, setup_failure) in modes {
        let directory = std::env::temp_dir().join(format!(
            "ic-inject-{}-{observe}-{}-{setup_failure}",
            std::process::id(),
            signal.unwrap_or(0)
        ));
        std::fs::create_dir(&directory).unwrap();
        let temporary_root = if setup_failure == 0 {
            directory.clone()
        } else {
            directory.join("invalid-root")
        };
        if setup_failure == 2 {
            std::fs::write(&temporary_root, b"preserve").unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let child_pid = std::sync::atomic::AtomicU32::new(0);
        let mut renderer = tokio::process::Command::new("node")
            .arg("--max-old-space-size=256")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/observe-renderer.mjs"
            ))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut renderer_input = renderer.stdin.take().unwrap();
        let mut renderer_output = BufReader::new(renderer.stdout.take().unwrap());
        let server = async {
            let (mut http, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                http.read_exact(&mut byte).await.unwrap();
                headers.push(byte[0]);
                assert!(headers.len() < 8192);
            }
            let targets = serde_json::json!([{"id":"fixture", "type":"page", "url":"https://discord.com/channels/@me",
                "webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/page/fixture")}]).to_string();
            http.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{targets}", targets.len()).as_bytes()).await.unwrap();
            drop(http);
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut diagnostic_seen = false;
            let mut diagnostic_requested = false;
            let mut automatic_owner_seen = false;
            let mut voice_after_diagnostics = false;
            loop {
                let Some(Ok(Message::Text(text))) = socket.next().await else {
                    panic!("missing mocked DevTools request")
                };
                let request: serde_json::Value = serde_json::from_str(&text).unwrap();
                let mut ready_to_stop = false;
                let expression = request["params"]["expression"].as_str().unwrap_or_default();
                let result = if expression == "Boolean(globalThis.__callmon)" {
                    serde_json::json!({"result":{"value":false}})
                } else {
                    serde_json::json!({})
                };
                socket
                    .send(Message::Text(
                        serde_json::json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                if request["method"] == "Runtime.addBinding" {
                    socket.send(Message::Text(serde_json::json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send",
                        "payload":serde_json::json!({"kind":"snapshot","observation_source":"store_notification","capture_gap":false,
                            "channel":{"id":"100","name":"Fixture"},"users":[{"id":"9007199254740993","username":"morgan","display_name":"Morgan"}]}).to_string()}}).to_string().into())).await.unwrap();
                }
                if let Some(view) = expression
                    .strip_prefix("globalThis.__callmon?.update(")
                    .and_then(|value| value.strip_suffix(')'))
                {
                    let view: serde_json::Value = serde_json::from_str(view).unwrap();
                    renderer_input
                        .write_all(format!("{view}\n").as_bytes())
                        .await
                        .unwrap();
                    let mut rendered = String::new();
                    assert!(renderer_output.read_line(&mut rendered).await.unwrap() > 0);
                    let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();
                    let owner = &view["intelligence"]["process_owner"];
                    let progress = &view["intelligence"]["process_progress"];
                    let evidence = &view["intelligence"]["session_evidence"];
                    let sensors = &view["intelligence"]["session_sensors"];
                    let timing = &view["intelligence"]["session_timing"];
                    assert_eq!(owner["source"], "capture_owner_outcome");
                    assert_eq!(progress["source"], "owner_progress_subscription");
                    assert_eq!(progress["sensor_running"], false);
                    assert_eq!(
                        progress["status"],
                        if observe && setup_failure == 0 {
                            "no_recent_sample"
                        } else {
                            "not_connected"
                        }
                    );
                    assert!(progress["sample"].is_null());
                    assert_eq!(evidence["source"], "backend_session_evidence_aggregate");
                    assert_eq!(evidence["acoustic"], "not_configured");
                    assert_eq!(evidence["cross_sensor_correlation"], "unavailable");
                    assert_eq!(timing["source"], "backend_session_clock_inventory");
                    assert_eq!(timing["voice_receipt_clock"], "backend_session_monotonic");
                    assert_eq!(timing["acoustic_receipt_clock"], "unavailable");
                    assert_eq!(timing["acoustic_frame_clock"], "unavailable");
                    assert_eq!(timing["cross_sensor_correlation"], "unavailable");
                    assert_eq!(
                        timing["process_timing"],
                        if owner["status"] == "not_configured" {
                            "unavailable"
                        } else {
                            "receipt_freshness_only"
                        }
                    );
                    assert_eq!(
                        rendered["process_telemetry"]["status"],
                        if observe && setup_failure == 0 {
                            "No recent sample"
                        } else {
                            "Not connected"
                        }
                    );
                    assert_eq!(rendered["process_telemetry"]["liveness"], "Not established");
                    assert_eq!(
                        rendered["process_telemetry"]["feature"]["state"],
                        if observe && setup_failure == 0 {
                            "waiting"
                        } else {
                            "ready"
                        }
                    );
                    assert_eq!(sensors["source"], "owner_lifecycle_aggregate");
                    assert_eq!(
                        sensors["configured"],
                        if owner["status"] == "not_configured" {
                            0
                        } else {
                            1
                        }
                    );
                    assert_eq!(
                        sensors["pending"],
                        if owner["status"] == "awaiting_completion" {
                            1
                        } else {
                            0
                        }
                    );
                    assert_eq!(
                        sensors["finished"],
                        if owner["status"] == "finished" { 1 } else { 0 }
                    );
                    assert_ne!(rendered["sensor_session"]["state"], "unsupported");
                    assert_ne!(
                        rendered["session_evidence"]["feature"]["state"],
                        "unsupported"
                    );
                    assert_eq!(
                        rendered["session_evidence"]["correlation"],
                        "Unavailable; streams remain independent"
                    );
                    assert_eq!(
                        rendered["session_evidence"]["attribution"],
                        "Not participant evidence"
                    );
                    assert_ne!(
                        rendered["session_timing"]["feature"]["state"],
                        "unsupported"
                    );
                    assert_eq!(
                        rendered["session_timing"]["voice"],
                        "Backend session monotonic"
                    );
                    assert_eq!(rendered["session_timing"]["acoustic"], "Unavailable");
                    assert_eq!(
                        rendered["session_timing"]["correlation"],
                        "Unavailable; no calibrated synchronization"
                    );
                    let label = match owner["status"].as_str().unwrap() {
                        "not_configured" => "Not configured",
                        "awaiting_completion" => "Awaiting completion; activity unverified",
                        "finished" => "Finished",
                        "unavailable" => "Outcome unavailable",
                        other => panic!("unexpected owner state {other}"),
                    };
                    assert_eq!(rendered["status"], label);
                    assert_eq!(rendered["liveness"], "Not established");
                    let expected = if setup_failure != 0 {
                        "unavailable"
                    } else if observe {
                        "finished"
                    } else {
                        "not_configured"
                    };
                    if owner["status"] == expected {
                        automatic_owner_seen = true;
                        if observe && setup_failure == 0 {
                            assert_eq!(sensors["status"], "attention");
                            assert_eq!(evidence["status"], "degraded");
                            assert_eq!(sensors["cleanup"], "confirmed");
                            assert_eq!(owner["result"]["outcome"], "collector_fault");
                            assert_eq!(owner["result"]["collector_reaped"], true);
                            assert_eq!(owner["result"]["endpoint_removed"], true);
                            assert_eq!(rendered["outcome"], "collector fault");
                            assert_eq!(rendered["reaped"], "Confirmed");
                            assert_eq!(rendered["removed"], "Confirmed");
                            assert_eq!(rendered["feature"]["state"], "error");
                        }
                        if !observe && setup_failure == 0 {
                            assert_eq!(sensors["status"], "sensor_free");
                            assert_eq!(evidence["status"], "sensor_free");
                            assert_eq!(sensors["cleanup"], "not_applicable");
                            assert_eq!(rendered["sensor_session"]["state"], "ready");
                        }
                        if setup_failure != 0 {
                            assert_eq!(sensors["status"], "attention");
                            assert_eq!(evidence["status"], "degraded");
                            assert_eq!(sensors["cleanup"], "unconfirmed");
                            assert_eq!(rendered["sensor_session"]["state"], "error");
                        }
                    } else {
                        assert_eq!(owner["status"], "awaiting_completion");
                        assert_eq!(evidence["status"], "awaiting_evidence");
                        assert_eq!(sensors["status"], "pending");
                        assert_eq!(sensors["cleanup"], "pending");
                        assert_eq!(rendered["sensor_session"]["state"], "waiting");
                    }
                    if diagnostic_seen && view["events"][0]["action"] == "left" {
                        assert_eq!(view["events"][0]["user"]["id"], "9007199254740993");
                        voice_after_diagnostics = true;
                        ready_to_stop = true;
                    } else if automatic_owner_seen && !diagnostic_requested {
                        diagnostic_requested = true;
                        socket.send(Message::Text(serde_json::json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send",
                        "payload":serde_json::json!({"kind":"diagnostics_check","session_id":view["intelligence"]["session_id"],"request_id":1}).to_string()}}).to_string().into())).await.unwrap();
                    }
                }
                if let Some(response) = expression
                    .strip_prefix("globalThis.__callmon?.diagnostics?.update(")
                    .and_then(|value| value.strip_suffix(')'))
                {
                    let response: serde_json::Value = serde_json::from_str(response).unwrap();
                    assert_eq!(response["request_id"], 1);
                    assert_eq!(response["source"], "host");
                    assert_eq!(
                        response["process_progress"]["status"],
                        if observe && setup_failure == 0 {
                            "no_recent_sample"
                        } else {
                            "not_connected"
                        }
                    );
                    assert_eq!(response["process_progress"]["sensor_running"], false);
                    assert!(response["process_progress"]["sample"].is_null());
                    assert_eq!(response["process_owner"]["source"], "capture_owner_outcome");
                    assert_eq!(
                        response["process_owner"]["status"],
                        if setup_failure != 0 {
                            "unavailable"
                        } else if observe {
                            "finished"
                        } else {
                            "not_configured"
                        }
                    );
                    if observe && setup_failure == 0 {
                        assert_eq!(
                            response["process_owner"]["result"]["outcome"],
                            "collector_fault"
                        );
                        assert_eq!(
                            response["process_owner"]["result"]["collector_reaped"],
                            true
                        );
                        assert_eq!(
                            response["process_owner"]["result"]["endpoint_removed"],
                            true
                        );
                    } else {
                        assert!(response["process_owner"]["result"].is_null());
                    }
                    diagnostic_seen = true;
                    socket.send(Message::Text(serde_json::json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send",
                        "payload":serde_json::json!({"kind":"snapshot","observation_source":"store_notification","capture_gap":false,
                            "channel":{"id":"100","name":"Fixture"},"users":[]}).to_string()}}).to_string().into())).await.unwrap();
                }
                if ready_to_stop {
                    if let Some(signal) = signal {
                        #[cfg(unix)]
                        assert_eq!(
                            unsafe {
                                libc::kill(
                                    child_pid.load(std::sync::atomic::Ordering::SeqCst) as i32,
                                    signal,
                                )
                            },
                            0
                        );
                    } else {
                        socket
                            .send(Message::Text(
                                serde_json::json!({"method":"Runtime.executionContextsCleared"})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                    }
                }
                if request["method"] == "Runtime.removeBinding" {
                    break;
                }
            }
            automatic_owner_seen && diagnostic_seen && voice_after_diagnostics
        };
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_intellicord"));
        command
            .args(["inject", "--port", &port.to_string()])
            .env("TMPDIR", &temporary_root)
            .env_remove("INTELLICORD_OBSERVE_HEALTH_REPORT")
            .env_remove("INTELLICORD_OBSERVE_OBJECT")
            .kill_on_drop(true);
        if observe {
            command.args([
                "--observe-object",
                "/dev/null",
                "--observe-pid",
                "1",
                "--observe-seconds",
                "1",
                "--acknowledge-injected-observation",
            ]);
        }
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = command.spawn().unwrap();
        child_pid.store(child.id().unwrap(), std::sync::atomic::Ordering::SeqCst);
        let (diagnostic_seen, output) =
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                tokio::join!(server, child.wait_with_output())
            })
            .await
            .unwrap();
        let output = output.unwrap();
        drop(renderer_input);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), renderer.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            diagnostic_seen,
            "mock diagnostics were not returned: {stderr}"
        );
        if setup_failure != 0 {
            assert!(stderr.contains("baseline tracking continues without a collector"));
            if signal.is_none() {
                assert!(stderr.contains("reloaded"));
            } else {
                assert!(stderr.contains("Injected observation could not be prepared"));
            }
            assert!(!stderr.contains("Injected observation final report"));
        } else if observe {
            let report = stderr
                .lines()
                .find_map(|line| line.strip_prefix("Injected observation final report: "))
                .unwrap();
            let report: serde_json::Value = serde_json::from_str(report).unwrap();
            assert_eq!(report["run"]["outcome"], "collector_fault");
            assert_eq!(report["run"]["collector_reaped"], true);
            assert_eq!(report["endpoint_removed"], true);
            assert_eq!(
                report["run"]["summary"]["lifecycle"]["reason"],
                "invalid_object"
            );
        } else {
            assert!(stderr.contains("reloaded"));
            assert!(!stderr.contains("Injected observation final report"));
        }
        if setup_failure == 2 {
            assert_eq!(std::fs::read(&temporary_root).unwrap(), b"preserve");
            std::fs::remove_file(&temporary_root).unwrap();
        } else if setup_failure == 1 {
            assert!(!temporary_root.exists());
        }
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        std::fs::remove_dir(directory).unwrap();
    }
}

#[test]
fn injection_observation_rejects_unsafe_options_before_devtools_access() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let complete = [
        "--observe-object",
        "/dev/null",
        "--observe-pid",
        "1",
        "--observe-seconds",
        "1",
        "--acknowledge-injected-observation",
    ];
    let mut cases = (1..complete.len())
        .map(|count| complete[..count].to_vec())
        .collect::<Vec<_>>();
    cases.push(vec!["--acknowledge-injected-observation"]);
    cases.push(vec!["--observe-pid", "--observe-seconds"]);
    for (index, replacement) in [(3, "0"), (3, "2147483648"), (5, "0"), (5, "61")] {
        let mut invalid = complete.to_vec();
        invalid[index] = replacement;
        cases.push(invalid);
    }
    if !cfg!(all(target_os = "linux", feature = "observe")) {
        cases.push(complete.to_vec());
    }
    for arguments in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["inject", "--port", &port])
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Cannot reach Discord"));
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[test]
fn launch_rejects_incomplete_or_unsafe_sensor_options_before_discord_access() {
    for arguments in [
        vec!["--port", "9223"],
        vec!["--target", "page"],
        vec!["--acoustic-sink-serial", "42"],
        vec!["--defer-acoustic-start"],
        vec!["discord://-/channels/@me", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .arg("launch")
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("Discord is already running"), "{stderr}");
        assert!(!stderr.contains("Starting Discord"), "{stderr}");
    }
}

#[test]
fn health_snapshot_helper_has_fixed_arguments_and_no_unconfigured_report() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .env_remove("INTELLICORD_OBSERVE_HEALTH_REPORT")
        .arg("health-snapshot-inspect")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"null\n");
    assert!(output.stderr.is_empty());
    for argument in ["--path", "--pid", "--object", "--attach", "extra"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["health-snapshot-inspect", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[cfg(not(all(feature = "observe", target_os = "linux")))]
#[test]
fn configured_health_snapshot_remains_unavailable_without_observe_support() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .env(
            "INTELLICORD_OBSERVE_HEALTH_REPORT",
            "/nonexistent/health.json",
        )
        .arg("check-health-snapshot")
        .output()
        .unwrap();
    assert!(output.status.success());
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snapshot["status"], "unavailable");
    assert!(snapshot["report"].is_null());
    assert_eq!(snapshot["sensor_running"], false);
}

#[test]
fn health_snapshot_cli_is_explicit_bounded_and_never_activates_a_sensor() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .env_remove("INTELLICORD_OBSERVE_HEALTH_REPORT")
        .arg("check-health-snapshot")
        .output()
        .unwrap();
    assert!(output.status.success());
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snapshot["status"], "not_configured");
    assert_eq!(snapshot["source"], "operator_supplied_snapshot");
    assert_eq!(snapshot["sensor_running"], false);
    assert!(snapshot["report"].is_null());
    for argument in ["--path", "--pid", "--object", "--attach"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["check-health-snapshot", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[cfg(all(feature = "observe", target_os = "linux"))]
#[test]
fn health_snapshot_reads_only_bounded_completed_regular_reports() {
    let directory =
        std::env::temp_dir().join(format!("intellicord-health-test-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _remove = Remove(directory.clone());
    let report = serde_json::json!({"schema_version":1,"source":"supervised_kernel_capture","sensor_running":false,
        "run":{"schema_version":1,"outcome":"completed","collector_started":true,"collector_reaped":true,
        "status":{"schema_version":1,"source":"kernel_tracepoint","records":6,"terminal_phase":"stopped",
            "reason":"duration_limit","attachment_confirmed":true,"detachment_confirmed":true},
        "terminal_health":{"schema_version":1,"accepted":2,"rejected_record":0,"rejected_clock":0,"ring_dropped":3}}});
    let file = directory.join("health.json");
    std::fs::write(&file, serde_json::to_vec(&report).unwrap()).unwrap();
    let check = |path: &std::path::Path| {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .env("INTELLICORD_OBSERVE_HEALTH_REPORT", path)
            .arg("check-health-snapshot")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.len() < 2048);
        assert!(!String::from_utf8_lossy(&output.stdout).contains(directory.to_str().unwrap()));
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let snapshot = check(&file);
    assert_eq!(snapshot["status"], "supplied");
    assert_eq!(snapshot["report"], report);
    let helper = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .env("INTELLICORD_OBSERVE_HEALTH_REPORT", &file)
        .arg("health-snapshot-inspect")
        .output()
        .unwrap();
    assert!(helper.status.success());
    assert!(helper.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&helper.stdout).unwrap(),
        report
    );
    let link = directory.join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert_eq!(check(&link)["status"], "rejected");
    assert_eq!(check(&directory)["status"], "rejected");
    for bytes in [vec![b' '; 4097], b"{}".to_vec(), b"not JSON".to_vec()] {
        std::fs::write(&file, bytes).unwrap();
        let snapshot = check(&file);
        assert_eq!(snapshot["status"], "rejected");
        assert!(snapshot["report"].is_null());
    }
    std::fs::remove_file(&file).unwrap();
    assert_eq!(check(&file)["status"], "rejected");
}

#[test]
fn delivery_replay_is_generated_and_rejects_all_activation_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("replay-process-delivery")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report,
        serde_json::to_value(intellicord::observe::delivery::generated_replay()).unwrap()
    );
    for argument in ["--pid", "--object", "--acknowledge-bpf-delivery", "extra"] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["replay-process-delivery", argument])
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn attachment_replay_is_generated_bounded_and_rejects_activation_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("replay-process-attachment")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report,
        serde_json::to_value(intellicord::observe::attachment::generated_replay()).unwrap()
    );
    assert_eq!(report.as_object().unwrap().len(), 6);
    for argument in [
        "--pid",
        "--object",
        "--attach",
        "--acknowledge-bpf-attach",
        "extra",
    ] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["replay-process-attachment", argument])
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn record_replay_is_bounded_and_rejects_activation_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("replay-process-record")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 512);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report,
        serde_json::to_value(intellicord::observe::record::generated_replay()).unwrap()
    );
    for argument in ["--pid", "--object", "--attach", "extra"] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["replay-process-record", argument])
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn producer_replay_is_generated_bounded_and_rejects_activation_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("replay-process-producer")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1536);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report,
        serde_json::to_value(intellicord::observe::producer::generated_replay()).unwrap()
    );
    for argument in ["--pid", "--object", "--attach", "extra"] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["replay-process-producer", argument])
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn selection_replay_is_generated_bounded_and_rejects_activation_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("replay-process-selection")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 5);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source"], "generated_replay");
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    assert_eq!(
        report["selection"],
        serde_json::to_value(intellicord::observe::generated_selection_checks()).unwrap()
    );
    for argument in ["--pid", "--object", "--attach", "extra"] {
        let invalid = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["replay-process-selection", argument])
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn clock_handshake_probe_never_reports_absolute_clocks_or_sensor_activity() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("probe-clock-handshake")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 8);
    assert_eq!(report["source"], "local_clock_handshake");
    assert_eq!(report["clock_domain"], "linux_monotonic");
    assert_eq!(report["max_bracket_ns"], 25_000_000);
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    #[cfg(all(feature = "observe", target_os = "linux"))]
    {
        assert!(["available", "unavailable"].contains(&report["status"].as_str().unwrap()));
        assert_eq!(report["result"]["helper_started"], true);
        assert_eq!(report["result"]["helper_reaped"], true);
        if report["status"] == "available" {
            assert_eq!(report["result"]["outcome"], "completed");
            assert!(report["result"]["bracket_width_ns"].as_u64().unwrap() <= 25_000_000);
        }
    }
    #[cfg(all(not(feature = "observe"), target_os = "linux"))]
    {
        assert_eq!(report["status"], "feature_disabled");
        assert!(report["result"].is_null());
    }
    #[cfg(not(target_os = "linux"))]
    {
        assert_eq!(report["status"], "unsupported");
        assert!(report["result"].is_null());
    }
    let text = String::from_utf8(output.stdout).unwrap();
    for private in ["source_ns", "before_ns", "after_ns", "nonce"] {
        assert!(!text.contains(private));
    }
    for command in ["probe-clock-handshake", "clock-handshake-reply"] {
        for argument in ["--pid", "--object", "--attach", "extra"] {
            let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
                .args([command, argument])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}

#[test]
fn clock_probe_is_bounded_inactive_and_rejects_activation_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("probe-clock")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 9);
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source"], "local_kernel_clock_probe");
    assert_eq!(report["clock_domain"], "linux_monotonic");
    assert_eq!(report["horizon_ns"], 1_000_000_000);
    assert_eq!(report["assumed_rate_error_ppm"], 100);
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    #[cfg(all(feature = "observe", target_os = "linux"))]
    {
        assert_eq!(report["status"], "available");
        assert!(report["bracket_width_ns"].as_u64().is_some());
    }
    #[cfg(all(not(feature = "observe"), target_os = "linux"))]
    assert_eq!(report["status"], "feature_disabled");
    #[cfg(not(target_os = "linux"))]
    assert_eq!(report["status"], "unsupported");
    if report["status"] != "available" {
        assert!(report["bracket_width_ns"].is_null());
    }
    for argument in ["--load", "--attach", "--pid", "--object", "extra"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["probe-clock", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn supervised_object_preflight_is_inactive_bounded_and_backend_configured_only() {
    for (configured, expected) in [(false, "not_configured"), (true, "unreadable")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord"));
        command
            .arg("check-object")
            .env_remove("INTELLICORD_OBSERVE_OBJECT");
        if configured {
            command.env("INTELLICORD_OBSERVE_OBJECT", "/dev/null");
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(output.stdout.len() < 2048);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report.as_object().unwrap().len(), 8);
        assert_eq!(report["source"], "offline_object_preflight");
        assert_eq!(report["sensor_running"], false);
        assert_eq!(report["load_test_performed"], false);
        assert_eq!(report["outcome"], "completed");
        assert_eq!(report["helper_started"], true);
        assert_eq!(report["helper_reaped"], true);
        assert_eq!(report["object"]["sensor_running"], false);
        assert_eq!(report["object"]["load_test_performed"], false);
        #[cfg(all(feature = "observe", target_os = "linux"))]
        assert_eq!(report["object"]["status"], expected);
        #[cfg(all(not(feature = "observe"), target_os = "linux"))]
        {
            let _ = expected;
            assert_eq!(report["object"]["status"], "feature_disabled");
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = expected;
            assert_eq!(report["object"]["status"], "unsupported");
        }
    }
    for command in ["object-preflight", "check-object"] {
        for argument in ["--load", "--attach", "--pid", "--object", "extra"] {
            let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
                .args([command, argument])
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}

#[test]
fn diagnostics_emits_one_bounded_report_without_starting_a_sensor() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
        .arg("diagnostics")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 2048);
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_object().unwrap().len(), 13);
    assert_eq!(report["schema_version"], 3);
    assert!(report["no_new_privileges"].is_null() || report["no_new_privileges"].is_boolean());
    assert!(
        ["disabled", "strict", "filter", "unknown", "unsupported"]
            .contains(&report["seccomp"].as_str().unwrap())
    );
    assert_eq!(report["sensor_running"], false);
    assert_eq!(report["load_test_performed"], false);
    assert_eq!(
        report["platform"],
        if cfg!(target_os = "linux") {
            "linux"
        } else {
            "unsupported"
        }
    );
    assert!(
        report["effective_capabilities"].is_null()
            || ["bpf", "perfmon", "sys_admin"]
                .iter()
                .all(|key| report["effective_capabilities"][key].is_boolean())
    );
}

#[test]
fn diagnostics_rejects_sensor_activation_arguments_without_a_report() {
    for argument in ["--start", "--load", "--attach", "extra"] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["diagnostics", argument])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: intellicord diagnostics"));
    }
}
