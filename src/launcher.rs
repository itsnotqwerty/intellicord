use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

const PORT: u16 = 9222;

fn discord_arguments(url: Option<&str>) -> Result<Vec<String>> {
    let mut arguments = vec![
        "--remote-debugging-address=127.0.0.1".into(),
        format!("--remote-debugging-port={PORT}"),
    ];
    if let Some(url) = url {
        let parsed = reqwest::Url::parse(url).context("Invalid Discord launch URL")?;
        if parsed.scheme() != "discord" {
            bail!("Only discord:// launch URLs are accepted");
        }
        arguments.extend(["--url".into(), "--".into(), url.into()]);
    }
    Ok(arguments)
}

pub(super) fn state_directory() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| Path::new(value).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .context("No user state directory available")?;
    Ok(base.join("intellicord"))
}

fn discord_running() -> bool {
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let Ok(bytes) = fs::read(entry.path().join("cmdline")) else {
                return false;
            };
            let arguments: Vec<_> = bytes.split(|byte| *byte == 0).collect();
            let Some(executable) = arguments.first() else {
                return false;
            };
            let name = executable.rsplit(|byte| *byte == b'/').next();
            matches!(name, Some(b"Discord" | b"discord"))
                && !arguments
                    .iter()
                    .any(|argument| argument.starts_with(b"--type="))
        })
}

async fn ready(client: &reqwest::Client, port: u16) -> Result<bool> {
    let targets: Vec<super::Target> = client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let Some(target) = targets
        .into_iter()
        .find(|target| target.kind == "page" && super::discord_page(&target.url))
    else {
        return Ok(false);
    };
    let websocket = target
        .web_socket_debugger_url
        .context("Discord target has no websocket")?;
    if !super::websocket_url(&websocket, port) {
        bail!("Refusing non-loopback debugging target");
    }
    let (socket, _) = tokio_tungstenite::connect_async(websocket).await?;
    let mut cdp = super::Cdp {
        socket,
        sequence: 0,
        pending: Default::default(),
    };
    let result = cdp
        .evaluate("Array.isArray(globalThis.webpackChunkdiscord_app)")
        .await?;
    Ok(result["result"]["value"] == true)
}

fn record(log: &mut File, message: &str) {
    let _ = writeln!(log, "{message}");
    eprintln!("{message}");
}

pub async fn launch(url: Option<&str>) -> Result<()> {
    let arguments = discord_arguments(url)?;
    let directory = state_directory()?;
    fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    fs::set_permissions(
        &directory,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("launch.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            if url.is_some() {
                Command::new("/usr/bin/discord")
                    .args(arguments)
                    .spawn()
                    .context("Unable to forward Discord URL")?;
            }
            return Ok(());
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    let mut log = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(directory.join("launch.log"))?;
    let result = run(arguments, &mut log).await;
    if let Err(error) = &result {
        record(&mut log, &format!("Intellicord: {error:#}"));
        let _ = Command::new("notify-send")
            .args([
                "--app-name=Intellicord",
                "Intellicord could not attach",
                &format!("{error:#}"),
            ])
            .status();
    }
    drop(lock);
    result
}

async fn run(arguments: Vec<String>, log: &mut File) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    let running = discord_running();
    if running
        && !tokio::time::timeout(Duration::from_secs(4), ready(&client, PORT))
            .await
            .is_ok_and(|value| value.unwrap_or(false))
    {
        bail!(
            "Discord is already running without a ready loopback debugging endpoint. Fully quit Discord (including its tray), then open the installed Discord launcher again. Intellicord will not close your current session."
        );
    }
    let mut child = if !running {
        record(
            log,
            "Starting Discord with loopback-only debugging on port 9222",
        );
        Some(
            Command::new("/usr/bin/discord")
                .args(&arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("Unable to launch /usr/bin/discord")?,
        )
    } else {
        if arguments.len() > 2 {
            Command::new("/usr/bin/discord").args(&arguments).spawn()?;
        }
        None
    };
    let mut started = Instant::now();
    let mut retry = tokio::time::interval(Duration::from_millis(500));
    loop {
        loop {
            tokio::select! {
                result = tokio::signal::ctrl_c() => { result?; return Ok(()); }
                _ = retry.tick() => {}
            }
            if child
                .as_mut()
                .map(|child| child.try_wait())
                .transpose()?
                .flatten()
                .is_some()
            {
                bail!("Discord exited before attachment");
            }
            if tokio::time::timeout(Duration::from_secs(4), ready(&client, PORT))
                .await
                .is_ok_and(|value| value.unwrap_or(false))
            {
                break;
            }
            if started.elapsed() > Duration::from_secs(120) {
                bail!(
                    "Discord did not become ready within two minutes. Log in normally, then retry the launcher. No authentication information is read by Intellicord."
                );
            }
        }
        record(log, "Attaching Intellicord");
        match super::inject(PORT, None).await {
            Ok(()) => return Ok(()),
            Err(error) if error.to_string().contains("Discord reloaded") => {
                record(
                    log,
                    "Discord reloaded; reattaching with a new memory-only history session",
                );
                started = Instant::now();
            }
            Err(error)
                if started.elapsed() < Duration::from_secs(120)
                    && [
                        "required voice/user/channel stores were not found",
                        "module loader is unavailable",
                        "module cache is unavailable",
                        "no Discord runtime was captured",
                    ]
                    .iter()
                    .any(|message| error.to_string().contains(message)) =>
            {
                record(log, "Discord stores are still loading; retrying attachment");
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_is_loopback_only_and_preserves_discord_links() {
        assert_eq!(
            discord_arguments(None).unwrap(),
            vec![
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=9222"
            ]
        );
        let arguments = discord_arguments(Some("discord://-/channels/@me")).unwrap();
        assert_eq!(&arguments[2..], ["--url", "--", "discord://-/channels/@me"]);
        assert!(discord_arguments(Some("--no-sandbox")).is_err());
        assert!(discord_arguments(Some("https://discord.com/channels/@me")).is_err());
    }

    #[test]
    fn launcher_lock_prevents_duplicate_backend_ownership() {
        let file =
            std::env::temp_dir().join(format!("intellicord-launch-test-{}", std::process::id()));
        let first = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&file)
            .unwrap();
        first.try_lock().unwrap();
        let second = OpenOptions::new().write(true).open(&file).unwrap();
        assert!(second.try_lock().is_err());
        first.unlock().unwrap();
        second.try_lock().unwrap();
        second.unlock().unwrap();
        fs::remove_file(file).unwrap();
    }

    #[tokio::test]
    async fn readiness_checks_only_the_loopback_discord_loader() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut http, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let count = http.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /json/list "));
            let body = serde_json::json!([{"id":"fixture", "type":"page", "url":"https://discord.com/channels/@me", "webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/page/fixture")}]).to_string();
            http.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            drop(http);
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let request = socket.next().await.unwrap().unwrap();
            let value: serde_json::Value =
                serde_json::from_str(request.to_text().unwrap()).unwrap();
            assert_eq!(value["method"], "Runtime.evaluate");
            assert_eq!(
                value["params"]["expression"],
                "Array.isArray(globalThis.webpackChunkdiscord_app)"
            );
            socket.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::json!({"id":value["id"], "result":{"result":{"type":"boolean", "value":true}}}).to_string().into())).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), ready(&client, port))
                .await
                .unwrap()
                .unwrap()
        );
        server.await.unwrap();
    }
}
