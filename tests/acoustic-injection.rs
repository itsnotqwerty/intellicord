#![cfg(target_os = "linux")]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, UnixListener},
    process::Command,
};
use tokio_tungstenite::tungstenite::Message;

#[test]
fn incomplete_output_consent_never_reaches_devtools() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let complete = [
        "--acoustic-sink-serial",
        "42",
        "--acoustic-seconds",
        "1",
        "--acknowledge-injected-output-capture",
    ];
    for count in 1..complete.len() {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .args(["inject", "--port", &port])
            .args(&complete[..count])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Cannot reach Discord"));
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[tokio::test]
async fn capture_voice_and_menu_share_one_session_and_await_teardown() {
    let mut cases = vec![
        ("stream", None, false),
        ("duration", None, false),
        ("stream", Some(libc::SIGINT), false),
        ("stream", Some(libc::SIGTERM), false),
        ("wrong-source", None, false),
        ("source-replaced", None, false),
        ("source-disappeared", None, false),
        ("invalid", None, false),
        ("slow-discovery", Some(libc::SIGTERM), false),
        ("stop-stream", None, false),
        ("stop-discovery", None, false),
        ("analysis", None, false),
        ("deferred-stream", None, false),
        ("deferred-call-end", None, false),
        ("deferred-never", None, false),
    ];
    if cfg!(feature = "observe") {
        cases.push(("stream", None, true));
        cases.push(("stop-stream", None, true));
        cases.push(("analysis", None, true));
        cases.push(("deferred-stream", None, true));
    }
    for (index, (mode, signal, observe)) in cases.into_iter().enumerate() {
        let directory =
            std::env::temp_dir().join(format!("ic-audio-{}-{index}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        for tool in ["pw-dump", "pw-record"] {
            let file = directory.join(tool);
            std::fs::write(&file, include_str!("fixtures/pipewire-tool.mjs")).unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let events = UnixListener::bind(directory.join("events")).unwrap();
        let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
        let collector = observed.clone();
        let events_task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = events.accept().await.unwrap();
                let mut bytes = Vec::new();
                socket.read_to_end(&mut bytes).await.unwrap();
                collector
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(&bytes).unwrap());
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut renderer = Command::new("node")
            .arg("--max-old-space-size=256")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/observe-renderer.mjs"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut renderer_input = renderer.stdin.take().unwrap();
        let mut renderer_output = BufReader::new(renderer.stdout.take().unwrap());
        let search = std::env::join_paths(std::iter::once(directory.clone()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord"));
        command
            .args([
                "inject",
                "--port",
                &port.to_string(),
                "--acoustic-sink-serial",
                "42",
                "--acoustic-seconds",
                if mode == "duration" { "1" } else { "30" },
                "--acknowledge-injected-output-capture",
            ])
            .env("PATH", search)
            .env("TMPDIR", &directory)
            .env(
                "INTELLICORD_FIXTURE_MODE",
                match mode {
                    "duration" | "stop-stream" | "deferred-stream" | "deferred-call-end"
                    | "deferred-never" => "stream",
                    "stop-discovery" => "slow-discovery",
                    _ => mode,
                },
            )
            .env("INTELLICORD_FIXTURE_SOCKET", directory.join("events"))
            .env_remove("INTELLICORD_OBSERVE_HEALTH_REPORT")
            .env_remove("INTELLICORD_OBSERVE_OBJECT")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if mode.starts_with("deferred-") {
            command.arg("--defer-acoustic-start");
        }
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
        let child = command.spawn().unwrap();
        let child_pid = child.id().unwrap();
        let server = async {
            let (mut http, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                http.read_exact(&mut byte).await.unwrap();
                headers.push(byte[0]);
                assert!(headers.len() < 8192);
            }
            let targets =
                json!([{"id":"fixture", "type":"page", "url":"https://discord.com/channels/@me",
                "webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/page/fixture")}])
                .to_string();
            http.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{targets}", targets.len()).as_bytes()).await.unwrap();
            drop(http);
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut acoustic_seen = false;
            let mut departure_requested = false;
            let mut stopped = false;
            let mut stop_requested = false;
            let mut start_requested = false;
            let mut call_end_requested = false;
            let mut stale_frames = None;
            let snapshot = |users: Value| {
                json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send",
                "payload":json!({"kind":"snapshot","channel":{"id":"100","name":"Fixture"},"users":users,
                "observation_source":"store_notification","capture_gap":false}).to_string()}})
            };
            let disconnected = || {
                json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send",
                "payload":json!({"kind":"snapshot","channel":null,"users":[],
                "observation_source":"store_notification","capture_gap":false}).to_string()}})
            };
            loop {
                let Some(Ok(Message::Text(text))) = socket.next().await else {
                    panic!("missing DevTools request for {mode}")
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                let expression = request["params"]["expression"].as_str().unwrap_or_default();
                let result = if expression == "Boolean(globalThis.__callmon)" {
                    json!({"result":{"value":false}})
                } else {
                    json!({})
                };
                socket
                    .send(Message::Text(
                        json!({"id":request["id"],"result":result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                if request["method"] == "Runtime.addBinding" {
                    socket.send(Message::Text(snapshot(json!([{"id":"9007199254740993","username":"morgan","display_name":"Morgan"}])).to_string().into())).await.unwrap();
                }
                if let Some(view) = expression
                    .strip_prefix("globalThis.__callmon?.update(")
                    .and_then(|text| text.strip_suffix(')'))
                {
                    let view: Value = serde_json::from_str(view).unwrap();
                    let acoustic = &view["intelligence"]["acoustic"];
                    let acoustic_owner = &view["intelligence"]["acoustic_owner"];
                    let session_evidence = &view["intelligence"]["session_evidence"];
                    let sensor_session = &view["intelligence"]["session_sensors"];
                    let session_timing = &view["intelligence"]["session_timing"];
                    let process_progress = &view["intelligence"]["process_progress"];
                    assert_eq!(acoustic_owner["schema_version"], 2);
                    assert_eq!(acoustic_owner["call_bound"], mode.starts_with("deferred-"));
                    assert_eq!(sensor_session["source"], "owner_lifecycle_aggregate");
                    assert_eq!(sensor_session["configured"], if observe { 2 } else { 1 });
                    assert_eq!(
                        session_evidence["source"],
                        "backend_session_evidence_aggregate"
                    );
                    assert_eq!(session_evidence["cross_sensor_correlation"], "unavailable");
                    assert_eq!(process_progress["source"], "owner_progress_subscription");
                    assert_eq!(process_progress["sensor_running"], false);
                    assert_eq!(session_timing["source"], "backend_session_clock_inventory");
                    assert_eq!(session_timing["cross_sensor_correlation"], "unavailable");
                    assert_eq!(
                        session_timing["process_timing"],
                        if observe {
                            "receipt_freshness_only"
                        } else {
                            "unavailable"
                        }
                    );
                    let is_stop = mode.starts_with("stop-");
                    let is_deferred = mode.starts_with("deferred-");
                    let stop_ready = if mode == "stop-discovery" {
                        acoustic["status"] == "starting"
                    } else {
                        acoustic["frames"].as_u64().unwrap_or(0) >= 2
                    } && (!observe
                        || view["intelligence"]["process_owner"]["status"] == "finished");
                    let call_active = view["current_call"].is_object()
                        && view["current_call"]["ended_at"].is_null();
                    let mut action = "none";
                    if is_deferred && acoustic.is_null() && !start_requested {
                        assert_eq!(acoustic_owner["status"], "prepared");
                        assert!(acoustic_owner["result"].is_null());
                        assert_eq!(view["intelligence"]["acoustic_stop_available"], false);
                        assert_eq!(
                            view["intelligence"]["acoustic_start_available"],
                            call_active
                        );
                        if call_active {
                            for payload in [
                                json!({"kind":"acoustic_start","session_id":"stale-session"}),
                                json!({"kind":"acoustic_start","session_id":view["intelligence"]["session_id"],"extra":true}),
                                json!({"kind":"acoustic_start"}),
                            ] {
                                socket.send(Message::Text(json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send","payload":payload.to_string()}}).to_string().into())).await.unwrap();
                            }
                            start_requested = true;
                            if mode == "deferred-stream" || mode == "deferred-call-end" {
                                action = "start_acoustic";
                            } else {
                                acoustic_seen = true;
                            }
                        }
                    }
                    if is_stop && stop_ready && !stop_requested {
                        if mode == "stop-stream" && stale_frames.is_none() {
                            for payload in [
                                json!({"kind":"acoustic_stop","session_id":"stale-session"}),
                                json!({"kind":"acoustic_stop","session_id":view["intelligence"]["session_id"],"extra":true}),
                                json!({"kind":"acoustic_stop"}),
                            ] {
                                socket.send(Message::Text(json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send","payload":payload.to_string()}}).to_string().into())).await.unwrap();
                            }
                            stale_frames = acoustic["frames"].as_u64();
                        } else if mode == "stop-discovery"
                            || acoustic["frames"].as_u64() > stale_frames
                        {
                            assert_eq!(view["intelligence"]["acoustic_stop_available"], true);
                            action = "stop_acoustic";
                            stop_requested = true;
                        }
                    }
                    renderer_input
                        .write_all(format!("{}\n", json!({"view":view,"action":action})).as_bytes())
                        .await
                        .unwrap();
                    let mut rendered = String::new();
                    assert!(renderer_output.read_line(&mut rendered).await.unwrap() > 0);
                    let rendered: Value = serde_json::from_str(&rendered).unwrap();
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
                    assert_ne!(
                        rendered["process_telemetry"]["feature"]["state"],
                        "unsupported"
                    );
                    assert_eq!(rendered["process_telemetry"]["liveness"], "Not established");
                    if is_deferred && acoustic.is_null() {
                        if acoustic_owner["status"] == "prepared" {
                            assert_eq!(rendered["acoustic"]["start_visible"], call_active);
                            assert_eq!(
                                rendered["acoustic"]["call_binding"],
                                if call_active {
                                    "Active voice call required; capture binds on start"
                                } else {
                                    "Waiting for an active voice call"
                                }
                            );
                        } else {
                            assert_eq!(acoustic_owner["status"], "running");
                            assert_eq!(rendered["acoustic"]["start_visible"], true);
                            assert_eq!(rendered["acoustic"]["start_disabled"], true);
                            assert_eq!(
                                rendered["acoustic"]["call_binding"],
                                "Bound to originating voice call; stops on end or switch"
                            );
                        }
                    }
                    if action == "stop_acoustic" {
                        let commands = rendered["commands"].as_array().unwrap();
                        assert_eq!(
                            commands,
                            &[
                                json!({"kind":"acoustic_stop","session_id":view["intelligence"]["session_id"]})
                            ]
                        );
                        assert_eq!(rendered["acoustic"]["stop_disabled"], true);
                        for _ in 0..2 {
                            socket.send(Message::Text(json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send","payload":commands[0].to_string()}}).to_string().into())).await.unwrap();
                        }
                    }
                    if action == "start_acoustic" {
                        let commands = rendered["commands"].as_array().unwrap();
                        assert_eq!(
                            commands,
                            &[
                                json!({"kind":"acoustic_start","session_id":view["intelligence"]["session_id"]})
                            ]
                        );
                        assert_eq!(rendered["acoustic"]["start_disabled"], true);
                        socket.send(Message::Text(json!({"method":"Runtime.bindingCalled","params":{"name":"__callmon_send","payload":commands[0].to_string()}}).to_string().into())).await.unwrap();
                    }
                    if !acoustic.is_null() {
                        assert_eq!(acoustic["source"], "mixed_output");
                        assert_eq!(
                            acoustic["schema_version"],
                            if acoustic.get("processing_health").is_some() {
                                4
                            } else if acoustic["latest"].is_null() {
                                2
                            } else {
                                3
                            }
                        );
                        assert_eq!(
                            rendered["acoustic"]["source"],
                            "Selected mixed output; all applications"
                        );
                        assert_eq!(
                            rendered["acoustic"]["attribution"],
                            "Unattributed; not participant evidence"
                        );
                        assert_eq!(rendered["acoustic"]["liveness"], "Not established");
                        if let Some(health) = acoustic.get("processing_health") {
                            assert_eq!(health["schema_version"], 1);
                            assert_eq!(
                                health["processed"]["frames"],
                                health["delivered"]["frames"].as_u64().unwrap()
                                    + health["discarded"]["frames"].as_u64().unwrap()
                                    + health["pending"]["frames"].as_u64().unwrap()
                            );
                            assert_eq!(
                                rendered["acoustic"]["processing_scope"],
                                "In-process queue accounting; not source or device liveness"
                            );
                            assert_eq!(
                                rendered["acoustic"]["processed"],
                                health["processed"]["frames"].as_u64().unwrap().to_string()
                            );
                            assert_eq!(rendered["acoustic"]["overflows"], "0");
                        }
                        if !acoustic["latest"].is_null() {
                            if session_evidence["status"] != "degraded" {
                                assert_eq!(session_evidence["acoustic"], "frame_available");
                                assert!(matches!(
                                    session_evidence["status"].as_str(),
                                    Some("partial" | "independent_evidence")
                                ));
                            }
                            let interval = &acoustic["latest"]["session_start_interval_ns"];
                            assert!(
                                interval["earliest_ns"].as_u64().unwrap()
                                    <= interval["latest_ns"].as_u64().unwrap()
                            );
                            assert_eq!(
                                rendered["acoustic"]["clock"],
                                "Backend session interval; recorder startup bounded, source clock uncalibrated"
                            );
                            assert!(
                                rendered["acoustic"]["interval"]
                                    .as_str()
                                    .is_some_and(|value| value.ends_with(" ms"))
                            );
                            assert_eq!(
                                session_timing["acoustic_frame_clock"],
                                "backend_session_interval"
                            );
                            assert_eq!(
                                rendered["session_timing"]["acoustic"],
                                "Bounded backend session interval"
                            );
                            assert_eq!(
                                rendered["session_timing"]["correlation"],
                                "Unavailable; no calibrated synchronization"
                            );
                        }
                        let ready = match mode {
                            "stop-stream" | "stop-discovery" => {
                                acoustic["status"] == "stopped" && stop_requested
                            }
                            "wrong-source" | "source-replaced" | "source-disappeared"
                            | "invalid" => acoustic["status"] == "faulted",
                            "slow-discovery" => acoustic["status"] == "starting",
                            "duration" => {
                                acoustic["status"] == "stopped"
                                    && acoustic["frames"].as_u64().unwrap() > 0
                            }
                            "analysis" => {
                                acoustic["frames"].as_u64().unwrap() >= 10
                                    && !acoustic["latest"]["analysis"]["best_similarity"].is_null()
                            }
                            _ => acoustic["frames"].as_u64().unwrap() >= 2,
                        };
                        if ready
                            && (!observe
                                || view["intelligence"]["process_owner"]["status"] == "finished")
                        {
                            acoustic_seen = true;
                            let terminal =
                                matches!(acoustic["status"].as_str(), Some("stopped" | "faulted"));
                            assert_eq!(
                                acoustic_owner["status"],
                                if terminal { "finished" } else { "running" }
                            );
                            if terminal {
                                assert_eq!(acoustic_owner["result"]["cleanup_confirmed"], true);
                                assert_eq!(rendered["acoustic"]["cleanup"], "Confirmed");
                                assert_eq!(
                                    acoustic_owner["result"]["outcome"],
                                    if acoustic["status"] == "faulted" {
                                        "faulted"
                                    } else {
                                        "completed"
                                    }
                                );
                                let expected_reason = match mode {
                                    "wrong-source" | "source-replaced" | "source-disappeared" => {
                                        Some("source_rejected")
                                    }
                                    "invalid" => Some("processing_failed"),
                                    _ => None,
                                };
                                assert_eq!(
                                    acoustic_owner["result"]["reason"].as_str(),
                                    expected_reason
                                );
                                assert_eq!(
                                    rendered["acoustic"]["owner_reason"],
                                    expected_reason.map_or(
                                        "Normal or owner-requested completion",
                                        |reason| if reason == "source_rejected" {
                                            "source rejected"
                                        } else {
                                            "processing failed"
                                        }
                                    )
                                );
                            }
                            if is_stop {
                                assert_eq!(view["intelligence"]["acoustic_stop_available"], false);
                                assert_eq!(
                                    rendered["acoustic"]["stop_request"],
                                    "Capture stopped."
                                );
                                for event in observed.lock().unwrap().iter() {
                                    assert!(
                                        !std::path::Path::new(&format!(
                                            "/proc/{}",
                                            event["pid"].as_u64().unwrap()
                                        ))
                                        .exists(),
                                        "tool survived stop acknowledgement"
                                    );
                                }
                            }
                            if mode == "stream" || mode == "duration" {
                                assert_eq!(rendered["acoustic"]["rms"], "0.000");
                            }
                            if mode == "analysis" {
                                assert_eq!(rendered["acoustic"]["centroid"], "1000.0 Hz");
                                assert_eq!(rendered["acoustic"]["repetition"], "1.000");
                                let analysis = acoustic["latest"]["analysis"].as_object().unwrap();
                                assert_eq!(analysis.len(), 6);
                                assert!(
                                    view["intelligence"]["observations"]
                                        .as_array()
                                        .unwrap()
                                        .iter()
                                        .all(|observation| observation
                                            .to_string()
                                            .find("acoustic")
                                            .is_none())
                                );
                            }
                        }
                    }
                    if acoustic_seen && !departure_requested {
                        departure_requested = true;
                        socket
                            .send(Message::Text(snapshot(json!([])).to_string().into()))
                            .await
                            .unwrap();
                    }
                    if acoustic_seen && view["events"][0]["action"] == "left" && !stopped {
                        assert_eq!(view["events"][0]["user"]["id"], "9007199254740993");
                        if mode == "deferred-call-end" && !call_end_requested {
                            call_end_requested = true;
                            socket
                                .send(Message::Text(disconnected().to_string().into()))
                                .await
                                .unwrap();
                        } else if mode != "deferred-call-end" || acoustic["status"] == "stopped" {
                            stopped = true;
                            if let Some(signal) = signal {
                                assert_eq!(unsafe { libc::kill(child_pid as i32, signal) }, 0);
                            } else {
                                socket
                                    .send(Message::Text(
                                        json!({"method":"Runtime.executionContextsCleared"})
                                            .to_string()
                                            .into(),
                                    ))
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                }
                if request["method"] == "Runtime.removeBinding" {
                    for event in observed.lock().unwrap().iter() {
                        assert!(
                            !std::path::Path::new(&format!(
                                "/proc/{}",
                                event["pid"].as_u64().unwrap()
                            ))
                            .exists(),
                            "tool survived UI detach"
                        );
                    }
                    break stopped && acoustic_seen;
                }
            }
        };
        let (matched, output) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(server, child.wait_with_output())
        })
        .await
        .unwrap();
        let output = output.unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            matched,
            "missing integrated acoustic view for {mode}: {stderr}"
        );
        if observe {
            assert!(
                stderr.contains("Injected observation did not complete"),
                "{stderr}"
            );
        } else if signal.is_none() {
            assert!(stderr.contains("reloaded"), "{stderr}");
        }
        if mode == "wrong-source"
            || mode == "source-replaced"
            || mode == "source-disappeared"
            || mode == "invalid"
        {
            assert!(stderr.contains("voice tracking continues without retry"));
        }
        drop(renderer_input);
        assert!(
            tokio::time::timeout(Duration::from_secs(2), renderer.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        events_task.abort();
        let _ = events_task.await;
        let events = observed.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event["tool"] == "pw-record")
                .count(),
            usize::from(
                mode != "wrong-source"
                    && mode != "slow-discovery"
                    && mode != "stop-discovery"
                    && mode != "deferred-never"
            )
        );
        for event in events.iter() {
            assert!(
                !std::path::Path::new(&format!("/proc/{}", event["pid"].as_u64().unwrap()))
                    .exists()
            );
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
