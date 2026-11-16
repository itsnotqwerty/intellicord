mod launcher;
mod preferences;

use std::{
    collections::VecDeque,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use intellicord::{Command, History};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

const SCRIPT: &str = include_str!("../dist/client.js");
const BINDING: &str = "__callmon_send";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    id: String,
    url: String,
    #[serde(rename = "type")]
    kind: String,
    web_socket_debugger_url: Option<String>,
}

struct Cdp {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    sequence: u64,
    pending: VecDeque<Value>,
}

impl Cdp {
    async fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        self.sequence += 1;
        self.socket
            .send(Message::Text(
                json!({
                    "id": self.sequence, "method": method, "params": params
                })
                .to_string()
                .into(),
            ))
            .await?;
        Ok(self.sequence)
    }

    async fn receive(&mut self) -> Result<Value> {
        loop {
            match self.socket.next().await.context("Discord disconnected")?? {
                Message::Text(text) => return Ok(serde_json::from_str(&text)?),
                Message::Close(_) => bail!("Discord closed the debugging connection"),
                _ => {}
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params).await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let value = self.receive().await?;
                if value["id"].as_u64() == Some(id) {
                    check_response(&value)?;
                    return Ok(value["result"].clone());
                }
                self.pending.push_back(value);
            }
        })
        .await
        .context("Discord did not answer within 10 seconds")?
    }

    async fn evaluate(&mut self, expression: &str) -> Result<Value> {
        self.request(
            "Runtime.evaluate",
            json!({
                "expression": expression, "returnByValue": true, "awaitPromise": true
            }),
        )
        .await
    }
}

fn check_response(value: &Value) -> Result<()> {
    if let Some(error) = value.get("error") {
        bail!("DevTools error: {error}");
    }
    if let Some(error) = value["result"].get("exceptionDetails") {
        bail!("Client script failed: {error}");
    }
    Ok(())
}

fn discord_page(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && matches!(
                url.host_str(),
                Some("discord.com" | "canary.discord.com" | "ptb.discord.com")
            )
            && (url.path() == "/app" || url.path().starts_with("/channels/"))
    })
}

fn websocket_url(url: &str, port: u16) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "ws"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            && url.port() == Some(port)
            && url.username().is_empty()
            && url.password().is_none()
    })
}

async fn inject(port: u16, target_id: Option<&str>) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?;
    let targets: Vec<Target> = client.get(format!("http://127.0.0.1:{port}/json/list"))
        .send().await.context("Cannot reach Discord DevTools. Fully quit Discord, then launch it with --remote-debugging-port and a loopback address; see README.md")?
        .error_for_status()?.json().await?;
    let mut matches: Vec<_> = targets
        .into_iter()
        .filter(|target| {
            target.kind == "page"
                && discord_page(&target.url)
                && target_id.is_none_or(|id| target.id == id)
        })
        .collect();
    if matches.len() != 1 {
        for target in &matches {
            eprintln!("Target {}: {}", target.id, target.url);
        }
        bail!(
            "Expected one Discord page, found {}. Log in first, or specify --target ID.",
            matches.len()
        );
    }
    let target = matches.remove(0);
    let url = target
        .web_socket_debugger_url
        .context("Target has no debugging websocket")?;
    if !websocket_url(&url, port) {
        bail!("Refusing non-loopback or unexpected debugging websocket");
    }
    let (socket, _) = tokio_tungstenite::connect_async(url).await?;
    let mut cdp = Cdp {
        socket,
        sequence: 0,
        pending: VecDeque::new(),
    };
    cdp.request("Runtime.enable", json!({})).await?;
    let attached = cdp.evaluate("Boolean(globalThis.__callmon)").await?;
    if attached["result"]["value"] == true {
        bail!(
            "Intellicord is already attached. Stop the existing injector first; reload Discord if its backend exited unexpectedly."
        );
    }
    cdp.request("Runtime.addBinding", json!({ "name": BINDING }))
        .await?;
    if let Err(error) = cdp.evaluate(SCRIPT).await {
        let _ = cdp.evaluate("globalThis.__callmon?.dispose()").await;
        let _ = cdp
            .request("Runtime.removeBinding", json!({ "name": BINDING }))
            .await;
        return Err(error);
    }
    println!(
        "Intellicord attached. Open Callmon at the bottom-left of the call view. Ctrl+C detaches all features."
    );
    let result = track(&mut cdp).await;
    let _ = cdp.evaluate("globalThis.__callmon?.dispose()").await;
    let _ = cdp
        .request("Runtime.removeBinding", json!({ "name": BINDING }))
        .await;
    result
}

async fn track(cdp: &mut Cdp) -> Result<()> {
    let mut history = History::default();
    let mut preferences =
        preferences::PreferenceStore::new(launcher::state_directory()?.join("preferences.json"));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    loop {
        let value = if let Some(value) = cdp.pending.pop_front() {
            value
        } else {
            tokio::select! {
                result = cdp.receive() => result?,
                _ = heartbeat.tick() => {
                    cdp.send("Runtime.evaluate", json!({"expression": "globalThis.__callmon?.heartbeat()"})).await?;
                    continue;
                }
                result = tokio::signal::ctrl_c() => { result?; return Ok(()); }
            }
        };
        check_response(&value)?;
        match value["method"].as_str() {
            Some("Runtime.bindingCalled") if value["params"]["name"] == BINDING => {
                let payload = value["params"]["payload"]
                    .as_str()
                    .context("Missing binding payload")?;
                if payload.len() > 2_000_000 {
                    bail!("Client snapshot exceeds size limit");
                }
                let value: Value =
                    serde_json::from_str(payload).context("Invalid client command")?;
                if matches!(
                    value["kind"].as_str(),
                    Some("preferences_get" | "preferences_set")
                ) {
                    if value["kind"] == "preferences_set" {
                        let disabled = value["disabled"]
                            .as_bool()
                            .context("Content preference must be boolean")?;
                        preferences.set(disabled);
                    }
                    let expression = format!(
                        "globalThis.__intellicord?.preferences.update({})",
                        preferences.response()
                    );
                    cdp.send("Runtime.evaluate", json!({"expression":expression}))
                        .await?;
                    continue;
                }
                let command: Command =
                    serde_json::from_value(value).context("Invalid client command")?;
                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
                history
                    .apply(command, timestamp)
                    .map_err(anyhow::Error::msg)?;
                let expression = format!(
                    "globalThis.__callmon?.update({})",
                    serde_json::to_string(&history)?
                );
                cdp.send("Runtime.evaluate", json!({"expression": expression}))
                    .await?;
            }
            Some("Runtime.executionContextsCleared") => {
                bail!("Discord reloaded. Run the injector again to attach to the new page.")
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("launch") {
        if args.len() > 2 {
            bail!("Usage: intellicord launch [discord://URL]");
        }
        return launcher::launch(args.get(1).map(String::as_str)).await;
    }
    if args.first().map(String::as_str) != Some("inject") {
        println!(
            "Intellicord 0.1.0\n\nUsage: intellicord launch [discord://URL]\n       intellicord inject [--port 9222] [--target ID]\n\nLaunch starts Discord with loopback-only DevTools and attaches automatically. Includes Callmon voice-call history.\nUI preview: open preview.html. See README.md for compatibility and privacy notes."
        );
        return Ok(());
    }
    let mut port = 9222;
    let mut target = None;
    let (options, remainder) = args[1..].as_chunks::<2>();
    for option in options {
        match option[0].as_str() {
            "--port" => {
                port = option[1].parse::<u16>().context("Invalid port")?;
            }
            "--target" => target = Some(option[1].as_str()),
            _ => bail!("Unknown option: {}", option[0]),
        }
    }
    if !remainder.is_empty() || port == 0 {
        bail!("Options require values and a nonzero port");
    }
    inject(port, target).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_discord_and_loopback_targets_are_allowed() {
        assert!(discord_page("https://discord.com/channels/@me"));
        assert!(discord_page("https://canary.discord.com/app"));
        assert!(!discord_page(
            "https://discord.com.attacker.test/channels/@me"
        ));
        assert!(!discord_page("https://discord.com/login"));
        assert!(websocket_url("ws://127.0.0.1:9222/devtools/page/1", 9222));
        assert!(!websocket_url(
            "ws://192.168.1.2:9222/devtools/page/1",
            9222
        ));
        assert!(!websocket_url("ws://127.0.0.1:9223/devtools/page/1", 9222));
    }

    #[test]
    fn script_exceptions_are_not_treated_as_success() {
        assert!(
            check_response(&json!({"result": {"exceptionDetails": {"text": "Error"}}})).is_err()
        );
        assert!(check_response(&json!({"error": {"message": "Missing method"}})).is_err());
        assert!(check_response(&json!({"result": {"result": {"value": true}}})).is_ok());
    }

    #[tokio::test]
    async fn transport_preserves_early_snapshots_and_emits_history() -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            let Some(Ok(Message::Text(request))) = socket.next().await else {
                bail!("missing bootstrap");
            };
            let request: Value = serde_json::from_str(&request)?;
            let snapshot = |ids: &[&str]| {
                json!({
                    "method": "Runtime.bindingCalled", "params": { "name": BINDING,
                        "payload": json!({"kind":"snapshot", "channel":{"id":"100", "name":"General"},
                            "users": ids.iter().map(|id| json!({"id":id,"username":"morgan","display_name":"Morgan"})).collect::<Vec<_>>()
                        }).to_string()
                    }
                })
            };
            socket
                .send(Message::Text(
                    snapshot(&["9007199254740993"]).to_string().into(),
                ))
                .await?;
            socket
                .send(Message::Text(
                    json!({"id":request["id"], "result": {}}).to_string().into(),
                ))
                .await?;
            let mut views = Vec::new();
            while views.len() < 3 {
                let Some(Ok(Message::Text(text))) = socket.next().await else {
                    bail!("missing view");
                };
                let request: Value = serde_json::from_str(&text)?;
                let expression = request["params"]["expression"].as_str().unwrap_or_default();
                if let Some(view) = expression
                    .strip_prefix("globalThis.__callmon?.update(")
                    .and_then(|value| value.strip_suffix(')'))
                {
                    views.push(serde_json::from_str::<Value>(view)?);
                    if views.len() == 1 {
                        socket
                            .send(Message::Text(snapshot(&[]).to_string().into()))
                            .await?;
                    } else if views.len() == 2 {
                        socket
                            .send(Message::Text(
                                json!({"method":"Runtime.bindingCalled", "params":{
                                    "name":BINDING,"payload":"{\"kind\":\"clear\"}"
                                }})
                                .to_string()
                                .into(),
                            ))
                            .await?;
                    }
                }
                socket
                    .send(Message::Text(
                        json!({"id":request["id"], "result":{}}).to_string().into(),
                    ))
                    .await?;
            }
            socket
                .send(Message::Text(
                    json!({"method":"Runtime.executionContextsCleared"})
                        .to_string()
                        .into(),
                ))
                .await?;
            Ok::<_, anyhow::Error>(views)
        });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}")).await?;
        let mut cdp = Cdp {
            socket,
            sequence: 0,
            pending: VecDeque::new(),
        };
        cdp.request("Runtime.evaluate", json!({"expression":"bootstrap"}))
            .await?;
        assert_eq!(cdp.pending.len(), 1);
        let result = tokio::time::timeout(Duration::from_secs(5), track(&mut cdp)).await?;
        assert!(result.unwrap_err().to_string().contains("reloaded"));
        let views = server.await??;
        assert!(views[0]["events"].as_array().unwrap().is_empty());
        assert_eq!(views[1]["events"][0]["action"], "left");
        assert_eq!(views[1]["events"][0]["user"]["id"], "9007199254740993");
        assert!(views[2]["events"].as_array().unwrap().is_empty());
        Ok(())
    }
}
