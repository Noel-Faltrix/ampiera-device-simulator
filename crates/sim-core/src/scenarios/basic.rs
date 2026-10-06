//! Scenarios without a human: S1, S2, S5, S6b, S7, S8.

use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio::sync::broadcast;

use super::ctx::answer_status;
use super::ctx::{default_vehicle_for, received_call, ScenarioCtx, Wait};
use super::rules::{
    check_post_boot_sequence, clock_offset_matches, meter_timestamp, response_status,
    sampled_value, ReceivedCall,
};
use crate::handle::{BoxEvent, CallOutcome};
use crate::model::{FrameDirection, OcppStatus};
use crate::ocpp::client;
use crate::ocpp::frames::Frame;
use crate::ocpp::messages::{format_timestamp, MEASURAND_POWER};

/// How long the wallbox gets to answer the boot handshake.
const BOOT_WAIT: Duration = Duration::from_secs(60);

/// Time for one step of the charging start (status change, StartTransaction with its answer): a few
/// round trips plus the simulation tick.
const STEP_WAIT: Duration = Duration::from_secs(60);

/// Time to the first MeterValues after charging started: the default sample interval of 60 s plus margin.
const METER_WAIT: Duration = Duration::from_secs(150);

/// Time for the central system to answer a call: the 30 s call timeout plus margin.
const ANSWER_WAIT: Duration = Duration::from_secs(35);

/// Time the wallbox needs to become available again after unplugging.
const UNPLUG_WAIT: Duration = Duration::from_secs(15);

/// How long the connection is watched after the answer in S5; a central system that dislikes the call closes
/// the socket within moments.
const KEEP_ALIVE_WATCH: Duration = Duration::from_secs(2);

/// Time for the wallbox's own answer to a call it received (it answers within a tick).
const OWN_ANSWER_WAIT: Duration = Duration::from_secs(10);

/// Time for the central system to resend the configuration without SoC after the rejection.
const CONFIG_RETRY_WAIT: Duration = Duration::from_secs(30);

/// Clock error S8 simulates: 15 minutes slow, the case the backend's 10 minute window discards.
const S8_CLOCK_OFFSET_S: i64 = -15 * 60;

/// Allowed deviation of the reported timestamp from the simulated offset (message latency).
const S8_TIMESTAMP_TOLERANCE_S: i64 = 30;

fn is_call(action: &'static str) -> impl FnMut(&Frame) -> bool {
    move |frame| matches!(frame, Frame::Call { action: a, .. } if a == action)
}

fn call_payload(frame: Frame) -> Option<(String, serde_json::Value)> {
    match frame {
        Frame::Call { id, payload, .. } => Some((id, payload)),
        _ => None,
    }
}

/// S1: connect, BootNotification, post-boot calls, Heartbeat.
pub async fn s1_boot(ctx: &mut ScenarioCtx) {
    let mut rx = ctx.handle.subscribe();
    // Already connected: reboot, otherwise the BootNotification and the calls after it happened before we listened.
    if let Err(error) = ctx.connect_or_reboot().await {
        ctx.fail("Verbindung zur Zentrale", error.to_string());
        return;
    }
    let Some(boot) = ctx
        .expect_frame(
            &mut rx,
            FrameDirection::Out,
            BOOT_WAIT,
            is_call("BootNotification"),
            "BootNotification beantwortet",
            "Die Wallbox hat kein BootNotification gesendet; die Verbindung kam nicht zustande.",
        )
        .await
    else {
        return;
    };
    let Some((boot_id, _)) = call_payload(boot.frame) else {
        return;
    };
    let Some(answer) = ctx
        .expect_answer(
            &mut rx,
            FrameDirection::In,
            &boot_id,
            BOOT_WAIT,
            "BootNotification beantwortet",
            "Die Zentrale hat das BootNotification nicht beantwortet.",
        )
        .await
    else {
        return;
    };
    check_boot_answer(ctx, answer.frame);
    let calls = collect_post_boot_calls(ctx, &mut rx).await;
    if ctx.is_aborted() {
        return;
    }
    let soc_rejected = !ctx.handle.snapshot().config.supports_soc;
    match check_post_boot_sequence(&calls, soc_rejected) {
        Ok(detail) => ctx.pass("Aufrufe nach dem Start in richtiger Reihenfolge", detail),
        Err(problem) => ctx.fail("Aufrufe nach dem Start in richtiger Reihenfolge", problem),
    }
    check_heartbeat(ctx).await;
}

fn check_boot_answer(ctx: &mut ScenarioCtx, frame: Frame) {
    const NAME: &str = "BootNotification beantwortet";
    match frame {
        Frame::CallResult { payload, .. } => {
            let accepted = response_status(&payload) == Some("Accepted");
            let interval = payload.get("interval").and_then(serde_json::Value::as_u64);
            match (accepted, interval) {
                (true, Some(i)) if i > 0 => {
                    ctx.pass(NAME, format!("Status Accepted, Heartbeat-Intervall {i} s."));
                }
                (true, _) => ctx.fail(NAME, "Status Accepted, aber ohne brauchbares interval."),
                (false, _) => ctx.fail(
                    NAME,
                    format!(
                        "Die Zentrale antwortete mit Status {}.",
                        response_status(&payload).unwrap_or("(fehlt)")
                    ),
                ),
            }
        }
        Frame::CallError {
            code, description, ..
        } => {
            ctx.fail(
                NAME,
                format!("Die Zentrale antwortete mit CALLERROR {code}: {description}"),
            );
        }
        Frame::Call { .. } => ctx.fail(NAME, "Statt einer Antwort kam ein Aufruf."),
    }
}

/// Reads CALLs from the central system until the TriggerMessage that ends the post-boot sequence arrives.
async fn collect_post_boot_calls(
    ctx: &mut ScenarioCtx,
    rx: &mut broadcast::Receiver<BoxEvent>,
) -> Vec<ReceivedCall> {
    let mut calls = Vec::new();
    let within = ctx.tuning.post_boot_wait;
    loop {
        let waited = ctx
            .wait_frame(rx, FrameDirection::In, within, |f| {
                matches!(f, Frame::Call { .. })
            })
            .await;
        let Wait::Ready(seen) = waited else {
            return calls;
        };
        if let Some(call) = received_call(&seen.frame) {
            let done = call.action == "TriggerMessage";
            calls.push(call);
            if done {
                return calls;
            }
        }
    }
}

async fn check_heartbeat(ctx: &mut ScenarioCtx) {
    const NAME: &str = "Heartbeat beantwortet";
    match ctx.call("Heartbeat", json!({})).await {
        Some(CallOutcome::Result(payload)) => {
            let time = payload
                .get("currentTime")
                .and_then(serde_json::Value::as_str);
            let valid = time.is_some_and(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok());
            ctx.check(
                valid,
                NAME,
                format!("currentTime {} empfangen.", time.unwrap_or_default()),
                "Die Antwort enthält kein gültiges currentTime.",
            );
        }
        Some(CallOutcome::Error { code, description }) => {
            ctx.fail(NAME, format!("CALLERROR {code}: {description}"));
        }
        Some(CallOutcome::Timeout) => {
            ctx.fail(NAME, "Keine Antwort auf den Heartbeat innerhalb von 30 s.");
        }
        Some(CallOutcome::NotConnected) | None => {}
    }
}

/// S2: plug in the default vehicle and watch the transaction start and two MeterValues.
pub async fn s2_plug_in_and_charge(ctx: &mut ScenarioCtx) {
    if !ctx.ensure_connected().await {
        return;
    }
    if ctx.handle.snapshot().vehicle.is_some() && !unplug_first(ctx).await {
        return;
    }
    let mut rx = ctx.handle.subscribe();
    let vehicle = default_vehicle_for(&ctx.handle.snapshot().config);
    if let Err(error) = ctx.handle.plug_in(vehicle).await {
        ctx.fail("Fahrzeug anstecken", error.to_string());
        return;
    }
    let step = STEP_WAIT;
    if expect_status(ctx, &mut rx, "Preparing", step)
        .await
        .is_none()
    {
        return;
    }
    let Some(tx_id) = expect_transaction(ctx, &mut rx, step).await else {
        return;
    };
    if expect_status(ctx, &mut rx, "Charging", step)
        .await
        .is_none()
    {
        return;
    }
    let mut powers = Vec::new();
    for round in 1..=2 {
        let Some(power) = expect_meter_values(ctx, &mut rx, round).await else {
            return;
        };
        powers.push(power);
    }
    let name = "Leistung größer 0 gemeldet";
    let positive = powers.iter().all(|p| p.is_some_and(|w| w > 0.0));
    ctx.check(
        positive,
        name,
        format!("Gemeldete Leistung in W: {powers:?} (Transaktion {tx_id})."),
        format!("Mindestens ein MeterValues ohne Leistung größer 0: {powers:?}."),
    );
}

async fn unplug_first(ctx: &mut ScenarioCtx) -> bool {
    if let Err(error) = ctx.handle.unplug().await {
        ctx.fail("Fahrzeug abstecken", error.to_string());
        return false;
    }
    match ctx
        .wait_snapshot(UNPLUG_WAIT, |s| s.status == OcppStatus::Available)
        .await
    {
        Wait::Ready(_) => true,
        Wait::TimedOut => {
            ctx.fail(
                "Fahrzeug abstecken",
                "Die Wallbox wurde nach dem Abstecken nicht wieder frei (Available).",
            );
            false
        }
        Wait::Aborted => false,
    }
}

async fn expect_status(
    ctx: &mut ScenarioCtx,
    rx: &mut broadcast::Receiver<BoxEvent>,
    status: &'static str,
    within: Duration,
) -> Option<()> {
    let name = format!("StatusNotification {status}");
    ctx.expect_frame(
        rx,
        FrameDirection::Out,
        within,
        move |f| {
            matches!(f, Frame::Call { action, payload, .. }
                if action == "StatusNotification" && payload.get("status").and_then(serde_json::Value::as_str) == Some(status))
        },
        &name,
        &format!("Die Wallbox hat nicht innerhalb von {} s den Status {status} gemeldet.", within.as_secs()),
    )
    .await?;
    ctx.pass(&name, "Status gemeldet.");
    Some(())
}

async fn expect_transaction(
    ctx: &mut ScenarioCtx,
    rx: &mut broadcast::Receiver<BoxEvent>,
    within: Duration,
) -> Option<i64> {
    const NAME: &str = "transactionId ab 1";
    let start = ctx
        .expect_frame(
            rx,
            FrameDirection::Out,
            within,
            is_call("StartTransaction"),
            NAME,
            "Die Wallbox hat kein StartTransaction gesendet.",
        )
        .await?;
    let (id, _) = call_payload(start.frame)?;
    let answer = ctx
        .expect_answer(
            rx,
            FrameDirection::In,
            &id,
            within,
            NAME,
            "Die Zentrale hat StartTransaction nicht beantwortet.",
        )
        .await?;
    match answer.frame {
        Frame::CallResult { payload, .. } => {
            let tx = payload
                .get("transactionId")
                .and_then(serde_json::Value::as_i64);
            match tx {
                Some(t) if t >= 1 => {
                    ctx.pass(NAME, format!("transactionId {t} erhalten."));
                    Some(t)
                }
                other => {
                    ctx.fail(NAME, format!("transactionId {other:?} ist kleiner als 1; die Zentrale speichert nichts."));
                    None
                }
            }
        }
        Frame::CallError {
            code, description, ..
        } => {
            ctx.fail(
                NAME,
                format!("StartTransaction wurde mit {code} abgelehnt: {description}"),
            );
            None
        }
        Frame::Call { .. } => None,
    }
}

/// Waits for one MeterValues call and its answer; returns the reported power (outer `None` = failure).
async fn expect_meter_values(
    ctx: &mut ScenarioCtx,
    rx: &mut broadcast::Receiver<BoxEvent>,
    round: usize,
) -> Option<Option<f64>> {
    let name = format!("MeterValues {round} beantwortet");
    let call = ctx
        .expect_frame(
            rx,
            FrameDirection::Out,
            METER_WAIT,
            is_call("MeterValues"),
            &name,
            "Die Wallbox hat kein MeterValues gesendet. Prüfe das Messintervall.",
        )
        .await?;
    let (id, payload) = call_payload(call.frame)?;
    let answer = ctx
        .expect_answer(
            rx,
            FrameDirection::In,
            &id,
            ANSWER_WAIT,
            &name,
            "Die Zentrale hat MeterValues nicht beantwortet.",
        )
        .await?;
    match answer.frame {
        Frame::CallResult { .. } => {
            ctx.pass(&name, "Von der Zentrale beantwortet.");
            Some(sampled_value(&payload, MEASURAND_POWER))
        }
        Frame::CallError {
            code, description, ..
        } => {
            ctx.fail(&name, format!("CALLERROR {code}: {description}"));
            None
        }
        Frame::Call { .. } => None,
    }
}

/// S5: StopTransaction with transactionId 0 must be accepted and keep the connection.
pub async fn s5_stop_transaction_zero(ctx: &mut ScenarioCtx) {
    const NAME: &str = "StopTransaction mit transactionId 0 akzeptiert";
    if !ctx.ensure_connected().await {
        return;
    }
    let snapshot = ctx.handle.snapshot();
    let payload = json!({
        "transactionId": 0,
        "meterStop": snapshot.energy_wh.round() as i64,
        "timestamp": format_timestamp(Utc::now()),
        "reason": "Local",
    });
    let Some(outcome) = ctx.call("StopTransaction", payload).await else {
        return;
    };
    match outcome {
        CallOutcome::Result(answer) => {
            let status = answer
                .pointer("/idTagInfo/status")
                .and_then(serde_json::Value::as_str);
            ctx.check(
                status.is_none_or(|s| s == "Accepted"),
                NAME,
                format!(
                    "Antwort ohne Fehler, idTagInfo.status: {}.",
                    status.unwrap_or("nicht enthalten")
                ),
                format!(
                    "idTagInfo.status ist {}, erwartet Accepted.",
                    status.unwrap_or_default()
                ),
            );
        }
        CallOutcome::Error { code, description } => {
            ctx.fail(
                NAME,
                format!("Die Zentrale antwortete mit CALLERROR {code}: {description}"),
            );
        }
        CallOutcome::Timeout => ctx.fail(NAME, "Keine Antwort innerhalb von 30 s."),
        CallOutcome::NotConnected => ctx.fail(NAME, "Die Verbindung brach vor der Antwort ab."),
    }
    if !ctx.sleep(KEEP_ALIVE_WATCH).await {
        return;
    }
    ctx.check(
        ctx.is_connected(),
        "Verbindung bleibt bestehen",
        format!(
            "Die Verbindung war auch {} s nach der Antwort noch offen.",
            KEEP_ALIVE_WATCH.as_secs()
        ),
        "Die Zentrale hat die Verbindung nach dem StopTransaction beendet.",
    );
}

/// S6b: a wallbox without SoC rejects the first ChangeConfiguration and receives one without SoC.
pub async fn s6b_no_soc(ctx: &mut ScenarioCtx) {
    let mut config = ctx.handle.snapshot().config;
    config.supports_soc = false;
    if let Err(error) = ctx.handle.set_config(config).await {
        ctx.fail("Wallbox ohne SoC einstellen", error.to_string());
        return;
    }
    let mut rx = ctx.handle.subscribe();
    if let Err(error) = ctx.connect_or_reboot().await {
        ctx.fail("Verbindung zur Zentrale", error.to_string());
        return;
    }
    let sampled = |with_soc: bool| {
        move |f: &Frame| match f {
            Frame::Call {
                action, payload, ..
            } if action == "ChangeConfiguration" => {
                let key = payload.get("key").and_then(serde_json::Value::as_str);
                let value = payload
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                key == Some("MeterValuesSampledData")
                    && value.split(',').any(|m| m.trim() == "SoC") == with_soc
            }
            _ => false,
        }
    };
    let first_name = "Erste Konfiguration (mit SoC) abgelehnt";
    let Some(first) = ctx
        .expect_frame(
            &mut rx,
            FrameDirection::In,
            BOOT_WAIT,
            sampled(true),
            first_name,
            "Die Zentrale hat keine Konfiguration mit SoC geschickt.",
        )
        .await
    else {
        return;
    };
    let Some((first_id, _)) = call_payload(first.frame) else {
        return;
    };
    let Some(answer) = ctx
        .expect_answer(
            &mut rx,
            FrameDirection::Out,
            &first_id,
            OWN_ANSWER_WAIT,
            first_name,
            "Die Wallbox hat die Konfiguration nicht beantwortet.",
        )
        .await
    else {
        return;
    };
    let status = answer_status(&answer.frame);
    ctx.check(
        status.as_deref() == Some("Rejected"),
        first_name,
        "Die Wallbox antwortete mit Rejected.",
        format!(
            "Die Wallbox antwortete mit {} statt Rejected.",
            status.unwrap_or_default()
        ),
    );
    let second = ctx
        .wait_frame(
            &mut rx,
            FrameDirection::In,
            CONFIG_RETRY_WAIT,
            sampled(false),
        )
        .await;
    match second {
        Wait::Ready(_) => ctx.pass(
            "Zweite Konfiguration ohne SoC kommt an",
            "Die Zentrale hat die Liste ohne SoC erneut geschickt.",
        ),
        Wait::TimedOut => ctx.fail(
            "Zweite Konfiguration ohne SoC kommt an",
            format!(
                "Nach der Ablehnung kam innerhalb von {} s keine Konfiguration ohne SoC.",
                CONFIG_RETRY_WAIT.as_secs()
            ),
        ),
        Wait::Aborted => {}
    }
}

/// S7: a second connection with the same identity makes the central system close the first one.
///
/// The second connection is opened directly and bypasses the connect gate, so on a live box the scenario is
/// refused (German check) while the live limit of three connected boxes is used up.
pub async fn s7_second_connection(ctx: &mut ScenarioCtx) {
    const NAME: &str = "Erste Verbindung wird geschlossen";
    if !ctx.ensure_connected().await {
        return;
    }
    // A completed round trip proves the central system has registered the first session; opening the second
    // connection earlier could race with that registration and make the replacement invisible.
    if !matches!(
        ctx.call("Heartbeat", json!({})).await,
        Some(CallOutcome::Result(_))
    ) {
        ctx.fail(NAME, "Die erste Verbindung antwortet nicht auf einen Heartbeat; der Test ist nicht aussagekräftig.");
        return;
    }
    // The second connection bypasses the connect gate, so the live limit is checked here: with three live
    // boxes connected it would be a fourth live connection.
    if let Err(error) = ctx.handle.check_extra_connection() {
        ctx.fail("Zweite Verbindung öffnen", error.to_string());
        return;
    }
    let snapshot = ctx.handle.snapshot();
    let mut rx = ctx.handle.subscribe();
    let second = client::connect(
        &snapshot.config.base_url,
        &snapshot.config.identity,
        ctx.handle.password(),
        ctx.handle.timings().connect_timeout,
    )
    .await;
    let second_socket = match second {
        Ok(socket) => socket,
        Err(failure) => {
            ctx.fail("Zweite Verbindung öffnen", failure.reason);
            return;
        }
    };
    let within = ctx.tuning.close_wait;
    let closed = ctx
        .wait_event(&mut rx, within, |event| match event {
            BoxEvent::Closed { code, reason } => Some((*code, reason.clone())),
            _ => None,
        })
        .await;
    // The second socket has done its job; dropping it now keeps it from kicking the reconnecting box out again.
    drop(second_socket);
    match closed {
        Wait::Ready((Some(1000), reason)) => ctx.pass(NAME, format!("Close-Code 1000, Grund „{reason}“.")),
        Wait::Ready((code, reason)) => ctx.fail(
            NAME,
            format!("Die erste Verbindung wurde mit Code {code:?} (Grund „{reason}“) geschlossen, erwartet 1000."),
        ),
        Wait::TimedOut => ctx.fail(
            NAME,
            format!("Die erste Verbindung blieb {} s nach dem Öffnen der zweiten offen.", within.as_secs()),
        ),
        Wait::Aborted => {}
    }
}

/// S8: a clock that is 15 minutes slow; the MeterValues must still be answered.
pub async fn s8_wrong_clock(ctx: &mut ScenarioCtx) {
    const NAME: &str = "MeterValues trotz falscher Uhr beantwortet";
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let mut config = ctx.handle.snapshot().config;
    config.clock_offset_s = S8_CLOCK_OFFSET_S;
    if let Err(error) = ctx.handle.set_config(config).await {
        ctx.fail("Uhrenfehler einstellen", error.to_string());
        return;
    }
    let mut rx = ctx.handle.subscribe();
    let Some(call) = ctx
        .expect_frame(
            &mut rx,
            FrameDirection::Out,
            METER_WAIT,
            is_call("MeterValues"),
            NAME,
            "Die Wallbox hat kein MeterValues gesendet. Prüfe das Messintervall.",
        )
        .await
    else {
        return;
    };
    let seen_at = call.at;
    let Some((id, payload)) = call_payload(call.frame) else {
        return;
    };
    match meter_timestamp(&payload) {
        Some(ts) => ctx.check(
            clock_offset_matches(ts, seen_at, S8_CLOCK_OFFSET_S, S8_TIMESTAMP_TOLERANCE_S),
            "Zeitstempel der Wallbox geht 15 Minuten nach",
            format!(
                "Zeitstempel {} liegt 15 Minuten vor der echten Zeit.",
                format_timestamp(ts)
            ),
            format!(
                "Zeitstempel {} weicht nicht um 15 Minuten ab.",
                format_timestamp(ts)
            ),
        ),
        None => ctx.fail(
            "Zeitstempel der Wallbox geht 15 Minuten nach",
            "Das MeterValues enthält keinen lesbaren Zeitstempel.",
        ),
    }
    let Some(answer) = ctx
        .expect_answer(
            &mut rx,
            FrameDirection::In,
            &id,
            ANSWER_WAIT,
            NAME,
            "Die Zentrale hat das MeterValues nicht beantwortet.",
        )
        .await
    else {
        return;
    };
    match answer.frame {
        Frame::CallResult { payload, .. } => ctx.check(
            payload == json!({}),
            NAME,
            "Antwort `{}` wie erwartet.",
            format!("Antwort {payload} statt `{{}}`."),
        ),
        Frame::CallError {
            code, description, ..
        } => ctx.fail(NAME, format!("CALLERROR {code}: {description}")),
        Frame::Call { .. } => {}
    }
    ctx.skip(
        "Zentrale verwirft die alten Werte",
        "Die Zentrale verwirft Messwerte, die älter als 10 Minuten sind (CENTRAL_SYSTEM_BEHAVIOUR.md). Das ist über \
         OCPP nicht sichtbar und wird hier nicht geprüft, nur dokumentiert.",
    );
}
