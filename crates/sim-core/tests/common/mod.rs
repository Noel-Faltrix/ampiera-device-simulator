//! Test scaffolding: a tiny mock of the Ampiera central system (CENTRAL_SYSTEM_BEHAVIOUR.md) and helpers.
//! This is not product code.

// Not every test file uses every helper. The large error type is dictated by the tungstenite handshake callback.
#![allow(dead_code, clippy::result_large_err)]

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sim_core::model::OcppStatus;
use sim_core::model::{ChargePointConfig, ChargePointSnapshot, FrameLogEntry};
use sim_core::{EventSink, Simulator};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{header, HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

pub const PASSWORD: &str = "geheim-test-passwort";
pub const IDENTITY: &str = "APTEST000001";

const STATUSES: [&str; 9] = [
    "Available",
    "Preparing",
    "Charging",
    "SuspendedEV",
    "SuspendedEVSE",
    "Finishing",
    "Reserved",
    "Unavailable",
    "Faulted",
];

#[derive(Clone, Debug)]
pub struct Logged {
    pub identity: String,
    /// `Foo` for a call of the box, `Foo.result` for the box's answer to a call of the central system.
    pub action: String,
    pub payload: Value,
    pub at: DateTime<Utc>,
}

enum ServerCmd {
    Close,
    Call(ServerCall),
}

struct ServerCall {
    action: String,
    payload: Value,
    tag: Tag,
    reply: Option<oneshot::Sender<Result<Value, String>>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Tag {
    Cfg1,
    Other,
}

struct Shared {
    password: Mutex<String>,
    boot_interval_s: u64,
    meter_interval: String,
    log: Mutex<Vec<Logged>>,
    conns: Mutex<HashMap<String, mpsc::UnboundedSender<ServerCmd>>>,
    attempts: AtomicUsize,
    next_tx: AtomicI64,
    sequence: AtomicUsize,
    /// How many StartTransaction calls are still answered with a rejection.
    reject_starts: AtomicUsize,
}

/// A running mock central system.
#[derive(Clone)]
pub struct Mock {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
}

fn now_text() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl Mock {
    pub async fn start() -> Self {
        Self::start_with(1, "1").await
    }

    /// `boot_interval_s` goes into the BootNotification answer, `meter_interval` into ChangeConfiguration.
    pub async fn start_with(boot_interval_s: u64, meter_interval: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        let shared = Arc::new(Shared {
            password: Mutex::new(PASSWORD.to_string()),
            boot_interval_s,
            meter_interval: meter_interval.to_string(),
            log: Mutex::new(Vec::new()),
            conns: Mutex::new(HashMap::new()),
            attempts: AtomicUsize::new(0),
            next_tx: AtomicI64::new(1),
            sequence: AtomicUsize::new(0),
            reject_starts: AtomicUsize::new(0),
        });
        let accept_shared = shared.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, accept_shared.clone()));
            }
        });
        Self { addr, shared }
    }

    /// The next `count` StartTransaction calls are answered with idTagInfo `Invalid`.
    pub fn reject_next_starts(&self, count: usize) {
        self.shared.reject_starts.store(count, Ordering::SeqCst);
    }

    pub fn base_url(&self) -> String {
        format!("ws://127.0.0.1:{}/ocpp", self.addr.port())
    }

    /// Number of websocket handshakes started (successful or not).
    pub fn attempts(&self) -> usize {
        self.shared.attempts.load(Ordering::SeqCst)
    }

    pub fn log(&self) -> Vec<Logged> {
        self.shared.log.lock().unwrap().clone()
    }

    pub fn calls(&self, action: &str) -> Vec<Logged> {
        self.log()
            .into_iter()
            .filter(|l| l.action == action)
            .collect()
    }

    pub fn count(&self, action: &str) -> usize {
        self.calls(action).len()
    }

    pub fn is_connected(&self, identity: &str) -> bool {
        self.shared
            .conns
            .lock()
            .unwrap()
            .get(identity)
            .is_some_and(|tx| !tx.is_closed())
    }

    /// Sends a CALL to the box and waits for its answer.
    pub async fn send_call(
        &self,
        identity: &str,
        action: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel();
        let call = ServerCall {
            action: action.to_string(),
            payload,
            tag: Tag::Other,
            reply: Some(tx),
        };
        let sender = self
            .shared
            .conns
            .lock()
            .unwrap()
            .get(identity)
            .cloned()
            .ok_or("keine Verbindung")?;
        sender
            .send(ServerCmd::Call(call))
            .map_err(|_| "Verbindung geschlossen".to_string())?;
        match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Verbindung vor der Antwort geschlossen".into()),
            Err(_) => Err("Zeitüberschreitung".into()),
        }
    }

    /// Mimics the backend's test limit: a TxDefaultProfile on connector 0 and, with a transaction, a TxProfile.
    pub async fn send_test_limit(
        &self,
        identity: &str,
        watts: f64,
        valid_to: DateTime<Utc>,
        tx_id: Option<i64>,
    ) {
        let schedule = |unit: &str, limit: f64| {
            json!({
                "startSchedule": (Utc::now() - chrono::Duration::seconds(60)).to_rfc3339_opts(SecondsFormat::Millis, true),
                "chargingRateUnit": unit,
                "chargingSchedulePeriod": [{"startPeriod": 0, "limit": limit}]
            })
        };
        let valid_to_text = valid_to.to_rfc3339_opts(SecondsFormat::Millis, true);
        let default = json!({"connectorId": 0, "csChargingProfiles": {
            "chargingProfileId": 4711, "stackLevel": 1, "chargingProfilePurpose": "TxDefaultProfile",
            "chargingProfileKind": "Absolute", "validTo": valid_to_text, "chargingSchedule": schedule("W", watts)}});
        let answer = self
            .send_call(identity, "SetChargingProfile", default)
            .await;
        assert_eq!(
            answer
                .expect("answer")
                .get("status")
                .and_then(Value::as_str),
            Some("Accepted")
        );
        if let Some(tx_id) = tx_id {
            let tx = json!({"connectorId": 1, "csChargingProfiles": {
                "chargingProfileId": 4712, "stackLevel": 1, "chargingProfilePurpose": "TxProfile", "transactionId": tx_id,
                "chargingProfileKind": "Absolute", "validTo": valid_to_text, "chargingSchedule": schedule("W", watts)}});
            let answer = self.send_call(identity, "SetChargingProfile", tx).await;
            assert_eq!(
                answer
                    .expect("answer")
                    .get("status")
                    .and_then(Value::as_str),
                Some("Accepted")
            );
        }
    }

    /// Closes the connection of this identity from the server side.
    pub fn close(&self, identity: &str) {
        if let Some(tx) = self.shared.conns.lock().unwrap().get(identity) {
            let _ = tx.send(ServerCmd::Close);
        }
    }
}

fn check_auth(request: &Request, password: &str) -> Result<String, ErrorResponse> {
    let unauthorized = || {
        let mut response = ErrorResponse::new(Some("Unauthorized".to_string()));
        *response.status_mut() = StatusCode::UNAUTHORIZED;
        response
    };
    let path = request.uri().path();
    let Some(identity) = path.strip_prefix("/ocpp/") else {
        let mut response = ErrorResponse::new(Some("Not found".to_string()));
        *response.status_mut() = StatusCode::NOT_FOUND;
        return Err(response);
    };
    let header_value = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .ok_or_else(unauthorized)?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(header_value)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(unauthorized)?;
    if decoded == format!("{identity}:{password}") {
        Ok(identity.to_string())
    } else {
        Err(unauthorized())
    }
}

async fn serve(stream: TcpStream, shared: Arc<Shared>) {
    shared.attempts.fetch_add(1, Ordering::SeqCst);
    let password = shared.password.lock().unwrap().clone();
    let mut identity = None;
    let callback = |request: &Request, mut response: Response| {
        identity = Some(check_auth(request, &password)?);
        response.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("ocpp1.6"),
        );
        Ok(response)
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let Some(identity) = identity else { return };
    run_connection(ws, identity, shared).await;
}

struct Outstanding {
    id: String,
    action: String,
    tag: Tag,
    reply: Option<oneshot::Sender<Result<Value, String>>>,
}

fn record(shared: &Shared, identity: &str, action: &str, payload: &Value) {
    shared.log.lock().unwrap().push(Logged {
        identity: identity.to_string(),
        action: action.to_string(),
        payload: payload.clone(),
        at: Utc::now(),
    });
}

async fn run_connection(
    mut ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    identity: String,
    shared: Arc<Shared>,
) {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();
    if let Some(old) = shared
        .conns
        .lock()
        .unwrap()
        .insert(identity.clone(), cmd_tx.clone())
    {
        let _ = old.send(ServerCmd::Close);
    }
    let mut queue: VecDeque<ServerCall> = VecDeque::new();
    let mut outstanding: Option<Outstanding> = None;
    loop {
        if outstanding.is_none() {
            if let Some(call) = queue.pop_front() {
                let id = format!("srv-{}", shared.sequence.fetch_add(1, Ordering::SeqCst));
                let text = json!([2, id, call.action, call.payload]).to_string();
                outstanding = Some(Outstanding {
                    id,
                    action: call.action,
                    tag: call.tag,
                    reply: call.reply,
                });
                if ws.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        }
        tokio::select! {
            message = ws.next() => {
                let Some(Ok(message)) = message else { break };
                match message {
                    Message::Text(text) => {
                        handle_text(&mut ws, &shared, &identity, text.as_str(), &mut queue, &mut outstanding).await;
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            command = cmd_rx.recv() => match command {
                Some(ServerCmd::Close) => {
                    let frame = CloseFrame { code: CloseCode::Normal, reason: "ersetzt".into() };
                    let _ = ws.send(Message::Close(Some(frame))).await;
                    break;
                }
                Some(ServerCmd::Call(call)) => queue.push_back(call),
                None => break,
            }
        }
    }
    let mut conns = shared.conns.lock().unwrap();
    if conns
        .get(&identity)
        .is_some_and(|tx| tx.same_channel(&cmd_tx))
    {
        conns.remove(&identity);
    }
}

async fn handle_text(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    shared: &Shared,
    identity: &str,
    text: &str,
    queue: &mut VecDeque<ServerCall>,
    outstanding: &mut Option<Outstanding>,
) {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let kind = items.first().and_then(Value::as_u64);
    let id = items
        .get(1)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    match kind {
        Some(2) => {
            let action = items
                .get(2)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let payload = items.get(3).cloned().unwrap_or(Value::Null);
            record(shared, identity, &action, &payload);
            let answer = answer_call(shared, &action, &payload);
            let frame = match answer {
                Ok(result) => json!([3, id, result]),
                Err((code, description)) => json!([4, id, code, description, {}]),
            };
            let _ = ws.send(Message::text(frame.to_string())).await;
            if action == "BootNotification" {
                queue.push_back(config_call(
                    "MeterValuesSampledData",
                    "Power.Active.Import,Energy.Active.Import.Register,SoC",
                    Tag::Cfg1,
                ));
            }
        }
        Some(3) | Some(4) => {
            let Some(pending) = outstanding.take().filter(|o| o.id == id) else {
                return;
            };
            let result = if kind == Some(3) {
                Ok(items.get(2).cloned().unwrap_or(Value::Null))
            } else {
                Err(format!("CALLERROR {:?}", items.get(2)))
            };
            if let Ok(payload) = &result {
                record(
                    shared,
                    identity,
                    &format!("{}.result", pending.action),
                    payload,
                );
            }
            if pending.tag == Tag::Cfg1 {
                let accepted = result
                    .as_ref()
                    .ok()
                    .and_then(|p| p.get("status"))
                    .and_then(Value::as_str)
                    == Some("Accepted");
                if !accepted {
                    queue.push_back(config_call(
                        "MeterValuesSampledData",
                        "Power.Active.Import,Energy.Active.Import.Register",
                        Tag::Other,
                    ));
                }
                queue.push_back(config_call(
                    "MeterValueSampleInterval",
                    &shared.meter_interval,
                    Tag::Other,
                ));
                queue.push_back(ServerCall {
                    action: "TriggerMessage".into(),
                    payload: json!({"requestedMessage": "StatusNotification"}),
                    tag: Tag::Other,
                    reply: None,
                });
            }
            if let Some(reply) = pending.reply {
                let _ = reply.send(result);
            }
        }
        _ => {}
    }
}

fn config_call(key: &str, value: &str, tag: Tag) -> ServerCall {
    ServerCall {
        action: "ChangeConfiguration".into(),
        payload: json!({"key": key, "value": value}),
        tag,
        reply: None,
    }
}

fn answer_call(
    shared: &Shared,
    action: &str,
    payload: &Value,
) -> Result<Value, (&'static str, String)> {
    let text_len = |field: &str| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .map(|s| s.chars().count())
    };
    match action {
        "BootNotification" => {
            let ok = |field| text_len(field).is_some_and(|n| (1..=20).contains(&n));
            if ok("chargePointVendor") && ok("chargePointModel") {
                Ok(
                    json!({"status": "Accepted", "currentTime": now_text(), "interval": shared.boot_interval_s}),
                )
            } else {
                Err((
                    "FormationViolation",
                    "Vendor/Model fehlen oder sind zu lang".into(),
                ))
            }
        }
        "Heartbeat" => Ok(json!({"currentTime": now_text()})),
        "StatusNotification" => {
            let status = payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if STATUSES.contains(&status) {
                Ok(json!({}))
            } else {
                Err(("FormationViolation", format!("Status {status} ungültig")))
            }
        }
        "MeterValues" => Ok(json!({})),
        "Authorize" => Ok(json!({"idTagInfo": {"status": "Accepted"}})),
        "StartTransaction" => {
            let rejecting = shared
                .reject_starts
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok();
            if rejecting {
                return Ok(json!({"transactionId": 0, "idTagInfo": {"status": "Invalid"}}));
            }
            let id = shared.next_tx.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"transactionId": id, "idTagInfo": {"status": "Accepted"}}))
        }
        "StopTransaction" => Ok(json!({"idTagInfo": {"status": "Accepted"}})),
        "DataTransfer" => Ok(json!({"status": "UnknownVendorId"})),
        other => Err(("NotImplemented", format!("{other} nicht implementiert"))),
    }
}

// ----- helpers for the tests -----

#[derive(Default)]
pub struct RecordingSink {
    pub updates: Mutex<Vec<(std::time::Instant, ChargePointSnapshot)>>,
    pub frames: Mutex<Vec<FrameLogEntry>>,
    pub removed: Mutex<Vec<String>>,
}

impl EventSink for RecordingSink {
    fn charge_point_updated(&self, snapshot: &ChargePointSnapshot) {
        self.updates
            .lock()
            .unwrap()
            .push((std::time::Instant::now(), snapshot.clone()));
    }

    fn frame_logged(&self, entry: &FrameLogEntry) {
        self.frames.lock().unwrap().push(entry.clone());
    }

    fn charge_point_removed(&self, id: &str) {
        self.removed.lock().unwrap().push(id.to_string());
    }
}

/// Simulator settings with short times so tests finish in seconds.
pub fn fast_settings() -> sim_core::Settings {
    let mut settings = sim_core::Settings::default();
    settings.timings.tick = Duration::from_millis(100);
    settings.timings.backoff_base = Duration::from_millis(200);
    settings.timings.connect_timeout = Duration::from_secs(5);
    // The mock sends 1 s intervals; the production floor of 5 s would stretch every test.
    settings.timings.min_interval = Duration::from_secs(1);
    settings.timings.cooldown = Duration::from_millis(600);
    settings.timings.stable_after = Duration::from_secs(1);
    settings.timings.start_retry = Duration::from_secs(1);
    settings.tuning.cooldown_margin = Duration::from_millis(100);
    settings.tuning.after_valid_to = Duration::from_secs(2);
    settings.tuning.limit_settle = Duration::from_millis(800);
    settings.tuning.second_profile_wait = Duration::from_secs(2);
    settings.tuning.no_retry_observation = Duration::from_secs(2);
    settings.tuning.close_wait = Duration::from_secs(5);
    settings.tuning.post_boot_wait = Duration::from_secs(10);
    settings.tuning.connect_wait = Duration::from_secs(10);
    settings
}

pub fn local_config(mock: &Mock) -> ChargePointConfig {
    let mut config = ChargePointConfig::default_local(IDENTITY);
    config.base_url = mock.base_url();
    config
}

/// Polls `condition` every 25 ms until it holds or `timeout` passes; panics with `what` then.
pub async fn wait_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < timeout,
            "Zeitüberschreitung beim Warten auf: {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Like `wait_until`, but for conditions that need to await (e.g. reading the simulator).
pub async fn wait_until_async<F, Fut>(what: &str, timeout: Duration, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    while !condition().await {
        assert!(
            start.elapsed() < timeout,
            "Zeitüberschreitung beim Warten auf: {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ----- harness shared by the integration test files -----

const WAIT: Duration = Duration::from_secs(10);

pub struct Harness {
    pub mock: Mock,
    pub sim: Simulator,
    pub sink: Arc<RecordingSink>,
}

impl Harness {
    pub async fn new() -> Self {
        let mock = Mock::start().await;
        let sink = Arc::new(RecordingSink::default());
        let sim = Simulator::with_settings(sink.clone(), fast_settings());
        Self { mock, sim, sink }
    }

    pub async fn add_box(&self) -> String {
        self.sim
            .add(local_config(&self.mock), PASSWORD.to_string(), false)
            .await
            .expect("add box")
    }

    pub async fn snapshot(&self, id: &str) -> sim_core::model::ChargePointSnapshot {
        self.sim
            .list()
            .await
            .into_iter()
            .find(|s| s.id == id)
            .expect("box exists")
    }

    pub async fn connect_and_wait(&self, id: &str) {
        self.sim.connect(id).await.expect("connect");
        let mock = self.mock.clone();
        // The sequence ends with the TriggerMessage; waiting for it keeps later assertions free of races.
        wait_until("Verbindung und Konfiguration nach dem Start", WAIT, || {
            mock.is_connected(IDENTITY)
                && mock.count("TriggerMessage.result") >= 1
                && mock.count("StatusNotification") >= 2
        })
        .await;
    }

    pub async fn wait_status(&self, id: &str, status: OcppStatus) {
        let sim = self.sim.clone();
        let id = id.to_string();
        wait_until_async(&format!("Status {status:?}"), WAIT, move || {
            let sim = sim.clone();
            let id = id.clone();
            async move {
                sim.list()
                    .await
                    .iter()
                    .any(|s| s.id == id && s.status == status)
            }
        })
        .await;
    }
}

pub fn status_of(payload: &Value) -> &str {
    payload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub fn meter_power(payload: &Value) -> Option<f64> {
    payload["meterValue"][0]["sampledValue"]
        .as_array()?
        .iter()
        .find(|v| v["measurand"] == "Power.Active.Import")
        .and_then(|v| v["value"].as_str()?.parse().ok())
}
