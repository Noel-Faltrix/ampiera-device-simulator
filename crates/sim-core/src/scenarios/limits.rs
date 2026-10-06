//! Scenarios around charging profiles that need a person to trigger something: S3, S4, S6, S9.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::Value;
use tokio::sync::broadcast;

use super::ctx::{ReceivedProfile, ScenarioCtx, Wait};
use super::rules::{
    expected_power_from_payloads, parse_set_profile, power_matches, valid_to_within,
    LIMIT_TOLERANCE_PCT,
};
use crate::handle::BoxEvent;
use crate::model::{ConnectionState, FrameDirection};
use crate::ocpp::frames::Frame;

/// The backend caps test limits at 15 minutes (SCENARIOS S3).
const MAX_TEST_LIMIT_MINUTES: i64 = 15;

/// Allowed overshoot when comparing `validTo` with the arrival time: the backend computes `validTo` slightly
/// before the frame reaches the box.
const VALID_TO_SLACK_S: i64 = 5;

/// How long the new BootNotification may take after the reconnect in S4.
const BOOT_AFTER_RECONNECT_WAIT: Duration = Duration::from_secs(30);

const HINT_TRIGGER_TEST_LIMIT: &str =
    "Es kam kein SetChargingProfile an. Löse im Intranet die Testgrenze für diese Wallbox aus \
     (der Befehlsweg muss aktiv sein).";

/// Payloads of the profiles the box accepted, with the latest `validTo` among them.
struct Accepted {
    payloads: Vec<Value>,
    valid_to: Option<DateTime<Utc>>,
}

/// The answers the box gave, in order, as text for the report.
fn answers_of(received: &[ReceivedProfile]) -> Vec<String> {
    received
        .iter()
        .map(|r| {
            r.answer
                .clone()
                .unwrap_or_else(|| "keine Antwort".to_string())
        })
        .collect()
}

async fn receive_profiles(
    ctx: &mut ScenarioCtx,
    rx: &mut broadcast::Receiver<BoxEvent>,
    missing_hint: &str,
) -> Option<Vec<ReceivedProfile>> {
    let first = ctx.remaining();
    let extra = ctx.tuning.second_profile_wait;
    let received = ctx.collect_profiles(rx, first, extra).await;
    if received.is_empty() {
        if !ctx.is_aborted() {
            ctx.fail("Profil empfangen", missing_hint);
        }
        return None;
    }
    let summary: Vec<String> = received
        .iter()
        .filter_map(|r| parse_set_profile(&r.payload))
        .map(|(connector, p)| {
            format!(
                "Profil {} ({:?}, Anschluss {connector}, Stufe {})",
                p.charging_profile_id, p.charging_profile_purpose, p.stack_level
            )
        })
        .collect();
    ctx.pass(
        "Profil empfangen",
        format!(
            "{} Profil(e) angekommen: {}.",
            received.len(),
            summary.join("; ")
        ),
    );
    Some(received)
}

fn accepted_profiles(received: &[ReceivedProfile]) -> Accepted {
    let accepted: Vec<&ReceivedProfile> = received
        .iter()
        .filter(|r| r.answer.as_deref() == Some("Accepted"))
        .collect();
    let valid_to = accepted
        .iter()
        .filter_map(|r| parse_set_profile(&r.payload))
        .filter_map(|(_, p)| p.valid_to)
        .max();
    Accepted {
        payloads: accepted.iter().map(|r| r.payload.clone()).collect(),
        valid_to,
    }
}

fn check_accepted(ctx: &mut ScenarioCtx, received: &[ReceivedProfile]) {
    let answers = answers_of(received);
    let all_accepted = answers.iter().all(|a| a == "Accepted");
    ctx.check(
        all_accepted,
        "Wallbox nimmt das Profil an",
        format!("Antworten der Wallbox: {}.", answers.join(", ")),
        format!(
            "Antworten der Wallbox: {}; erwartet Accepted.",
            answers.join(", ")
        ),
    );
}

/// Compares the measured power with the limit computed from the received payloads. The expectation is derived
/// from the raw SetChargingProfile JSON (A x 230 V x phases, W unchanged, capped by the box and vehicle
/// maximum), not from the box's own profile evaluation, so a bug in the box cannot confirm itself.
async fn check_power(ctx: &mut ScenarioCtx, name: &str, payloads: &[Value], expectation: &str) {
    if !ctx.sleep(ctx.tuning.limit_settle).await {
        return;
    }
    let snapshot = ctx.handle.snapshot();
    let Some(vehicle) = snapshot.vehicle.as_ref() else {
        ctx.fail(
            name,
            "Es ist kein Fahrzeug mehr angesteckt, die Leistung kann nicht beurteilt werden.",
        );
        return;
    };
    let expected = expected_power_from_payloads(
        payloads,
        Utc::now(),
        snapshot.config.phases,
        snapshot.config.max_power_w,
        &vehicle.config,
        vehicle.soc_pct,
    );
    let actual = snapshot.power_w;
    ctx.check(
        power_matches(actual, expected, LIMIT_TOLERANCE_PCT),
        name,
        format!("{expectation}: Leistung {:.0} W, erwartet {expected:.0} W (±{LIMIT_TOLERANCE_PCT} %).", actual.unwrap_or_default()),
        format!("{expectation}: Leistung {actual:?} W, erwartet {expected:.0} W (±{LIMIT_TOLERANCE_PCT} %)."),
    );
}

/// Waits until `at`; records a failed check when the scenario deadline comes first. Returns false then.
async fn wait_until_or_fail(ctx: &mut ScenarioCtx, at: DateTime<Utc>, name: &str) -> bool {
    match ctx.sleep_until_utc(at).await {
        Wait::Ready(()) => true,
        Wait::TimedOut => {
            ctx.fail(
                name,
                "Das Zeitlimit des Szenarios lief ab, bevor der Zeitpunkt erreicht war.",
            );
            false
        }
        Wait::Aborted => false,
    }
}

/// S3: test limit from the intranet; regulation and the end at `validTo`.
pub async fn s3_test_limit(ctx: &mut ScenarioCtx) {
    // Subscribe first: the backend may send the profile right after StartTransaction, while the scenario is
    // still waiting for the charging state.
    let mut rx = ctx.handle.subscribe();
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let Some(received) = receive_profiles(ctx, &mut rx, HINT_TRIGGER_TEST_LIMIT).await else {
        return;
    };
    check_accepted(ctx, &received);
    let accepted = accepted_profiles(&received);
    let Some(first_at) = received.first().map(|r| r.received_at) else {
        return;
    };
    let latest = accepted.valid_to;
    match valid_to_within(
        received
            .first()
            .and_then(|r| parse_set_profile(&r.payload))
            .and_then(|(_, p)| p.valid_to),
        first_at,
        ChronoDuration::minutes(MAX_TEST_LIMIT_MINUTES) + ChronoDuration::seconds(VALID_TO_SLACK_S),
    ) {
        Ok(remaining) => ctx.pass(
            "Gültigkeit höchstens 15 Minuten",
            format!(
                "validTo liegt {} s nach dem Eintreffen.",
                remaining.num_seconds()
            ),
        ),
        Err(problem) => ctx.fail("Gültigkeit höchstens 15 Minuten", problem),
    }
    check_power(
        ctx,
        "Wallbox regelt auf die Grenze",
        &accepted.payloads,
        "Grenze aktiv",
    )
    .await;
    let Some(valid_to) = latest else {
        ctx.skip(
            "Ende der Grenze",
            "Es wurde kein Profil mit validTo angenommen; das Ende kann nicht geprüft werden.",
        );
        return;
    };
    let end = valid_to + ChronoDuration::from_std(ctx.tuning.after_valid_to).unwrap_or_default();
    if !wait_until_or_fail(
        ctx,
        end,
        "Nach validTo lädt die Wallbox mit voller Leistung",
    )
    .await
    {
        return;
    }
    check_power(
        ctx,
        "Nach validTo lädt die Wallbox mit voller Leistung",
        &accepted.payloads,
        "Nach Ablauf",
    )
    .await;
}

/// S4: the server disappears during a limit; the box must keep the limit and end it on its own.
pub async fn s4_server_gone(ctx: &mut ScenarioCtx) {
    let mut rx = ctx.handle.subscribe();
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let Some(received) = receive_profiles(ctx, &mut rx, HINT_TRIGGER_TEST_LIMIT).await else {
        return;
    };
    check_accepted(ctx, &received);
    let accepted = accepted_profiles(&received);
    let Some(valid_to) = accepted.valid_to else {
        ctx.fail("Profil mit validTo", "Kein angenommenes Profil mit validTo; der Verbindungsabbruch ist nicht sinnvoll prüfbar.");
        return;
    };
    let reconnect_at =
        valid_to + ChronoDuration::from_std(ctx.tuning.after_valid_to).unwrap_or_default();
    if let Err(error) = ctx.handle.drop_connection(Some(reconnect_at)).await {
        ctx.fail("Verbindung trennen", error.to_string());
        return;
    }
    check_power(
        ctx,
        "Grenze gilt ohne Verbindung",
        &accepted.payloads,
        "Offline mit Grenze",
    )
    .await;
    if ctx.is_connected() {
        ctx.fail(
            "Verbindung bleibt gesperrt",
            "Die Wallbox ist trotz Sperre vor validTo wieder verbunden.",
        );
        return;
    }
    let ends_at = valid_to + ChronoDuration::from_std(ctx.tuning.limit_settle).unwrap_or_default();
    if !wait_until_or_fail(
        ctx,
        ends_at,
        "Grenze endet zum Ablaufzeitpunkt ohne Zentrale",
    )
    .await
    {
        return;
    }
    let still_offline = !ctx.is_connected();
    check_power(
        ctx,
        "Grenze endet zum Ablaufzeitpunkt ohne Zentrale",
        &accepted.payloads,
        "Nach validTo ohne Zentrale",
    )
    .await;
    ctx.check(
        still_offline,
        "Verbindung bleibt bis zum Ende der Sperre unterbrochen",
        "Die Wallbox war nach validTo noch ohne Zentrale; die Grenze endete aus eigener Kraft.",
        "Die Wallbox war schon vor dem Ende der Sperre wieder verbunden; der Nachweis ohne Zentrale gilt nicht.",
    );
    let within = (reconnect_at - Utc::now()).to_std().unwrap_or_default() + ctx.tuning.connect_wait;
    match ctx
        .wait_snapshot(within, |s| {
            matches!(s.connection, ConnectionState::Connected { .. })
        })
        .await
    {
        Wait::Ready(_) => {}
        Wait::TimedOut => {
            ctx.fail(
                "Neuanmeldung nach der Sperre",
                "Die Wallbox hat sich nach dem Ende der Sperre nicht wieder verbunden.",
            );
            return;
        }
        Wait::Aborted => return,
    }
    let boot = ctx
        .wait_frame(
            &mut rx,
            FrameDirection::Out,
            BOOT_AFTER_RECONNECT_WAIT,
            |f| matches!(f, Frame::Call { action, .. } if action == "BootNotification"),
        )
        .await;
    match boot {
        Wait::Ready(_) => ctx.pass(
            "Neuanmeldung nach der Sperre",
            "Die Wallbox hat sich mit einem neuen BootNotification angemeldet.",
        ),
        Wait::TimedOut => ctx.fail(
            "Neuanmeldung nach der Sperre",
            "Verbunden, aber kein neues BootNotification gesendet.",
        ),
        Wait::Aborted => {}
    }
}

/// S6: the box rejects profiles; nothing may be applied and the connection must stay up.
pub async fn s6_box_rejects(ctx: &mut ScenarioCtx) {
    let mut config = ctx.handle.snapshot().config;
    config.reject_profiles = true;
    if let Err(error) = ctx.handle.set_config(config).await {
        ctx.fail("Wallbox auf Ablehnen einstellen", error.to_string());
        return;
    }
    let mut rx = ctx.handle.subscribe();
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let Some(received) = receive_profiles(ctx, &mut rx, HINT_TRIGGER_TEST_LIMIT).await else {
        return;
    };
    let answers = answers_of(&received);
    let all_rejected = answers.iter().all(|a| a == "Rejected");
    ctx.check(
        all_rejected,
        "Wallbox antwortet mit Rejected",
        format!("Antworten der Wallbox: {}.", answers.join(", ")),
        format!(
            "Antworten der Wallbox: {}; erwartet Rejected.",
            answers.join(", ")
        ),
    );
    if !ctx.sleep(ctx.tuning.limit_settle).await {
        return;
    }
    ctx.check(
        ctx.is_connected(),
        "Verbindung bleibt bestehen",
        "Die Verbindung war nach der Ablehnung noch offen.",
        "Die Verbindung brach nach der Ablehnung ab.",
    );
    let snapshot = ctx.handle.snapshot();
    ctx.check(
        snapshot.active_limit.is_none() && snapshot.profile_count == 0,
        "Keine Grenze wirkt",
        "Die Wallbox hat kein Profil gespeichert und lädt ohne Grenze.",
        format!(
            "Die Wallbox hat {} Profil(e) gespeichert (Grenze: {:?}).",
            snapshot.profile_count, snapshot.active_limit
        ),
    );
}

/// S9: the schedule steers the box; the profile must apply and later be replaced or cleared.
pub async fn s9_schedule(ctx: &mut ScenarioCtx) {
    let mut rx = ctx.handle.subscribe();
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let hint = "Es kam kein SetChargingProfile an. Öffne in der App oder im Intranet einen Ladebedarf und \
                prüfe, dass lokal OCPP_AKTIV=aktiv gilt.";
    let Some(received) = receive_profiles(ctx, &mut rx, hint).await else {
        return;
    };
    check_accepted(ctx, &received);
    let accepted = accepted_profiles(&received);
    ctx.check(
        accepted.valid_to.is_some(),
        "Profil hat validTo",
        "Das Fahrplan-Profil hat ein validTo (mehr als 15 Minuten sind hier erlaubt).",
        "Kein angenommenes Profil mit validTo; die Wallbox wüsste nie, wann ihre Pflicht endet.",
    );
    check_power(
        ctx,
        "Grenze aus dem Fahrplan wirkt",
        &accepted.payloads,
        "Fahrplan-Grenze",
    )
    .await;
    let change = ctx
        .wait_frame(&mut rx, FrameDirection::In, ctx.remaining(), |f| {
            matches!(f, Frame::Call { action, .. } if action == "SetChargingProfile" || action == "ClearChargingProfile")
        })
        .await;
    match change {
        Wait::Ready(seen) => {
            let action = match seen.frame {
                Frame::Call { action, .. } => action,
                _ => String::new(),
            };
            ctx.pass("Profil wird bei Fahrplanänderung ersetzt oder gelöscht", format!("{action} kam an."));
        }
        Wait::TimedOut => ctx.fail(
            "Profil wird bei Fahrplanänderung ersetzt oder gelöscht",
            "Bis zum Ende des Szenarios kam weder ein neues Profil noch ein ClearChargingProfile. Ändere oder \
             schließe den Ladebedarf, damit sich der Fahrplan ändert.",
        ),
        Wait::Aborted => {}
    }
}
