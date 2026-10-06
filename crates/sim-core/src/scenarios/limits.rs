//! Scenarios around charging profiles that need a person to trigger something: S3, S4, S6, S9.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tokio::sync::broadcast;

use super::ctx::{ReceivedProfile, ScenarioCtx, Wait};
use super::rules::{
    expected_power_now, parse_set_profile, power_matches, valid_to_within, LIMIT_TOLERANCE_PCT,
};
use crate::charge_point::profiles::ChargingProfile;
use crate::handle::BoxEvent;
use crate::model::{ConnectionState, FrameDirection};
use crate::ocpp::frames::Frame;

/// The backend caps test limits at 15 minutes (SCENARIOS S3).
const MAX_TEST_LIMIT_MINUTES: i64 = 15;

/// Allowed overshoot when comparing `validTo` with the arrival time: the backend computes `validTo` slightly
/// before the frame reaches the box.
const VALID_TO_SLACK_S: i64 = 5;

const HINT_TRIGGER_TEST_LIMIT: &str =
    "Es kam kein SetChargingProfile an. Bitte im Intranet die Testgrenze für diese Box auslösen \
     (Befehlsweg muss aktiv sein).";

/// Profiles the box accepted, with the latest `validTo` among them.
struct Accepted {
    profiles: Vec<ChargingProfile>,
    valid_to: Option<DateTime<Utc>>,
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
    let profiles: Vec<ChargingProfile> = received
        .iter()
        .filter(|r| r.answer.as_deref() == Some("Accepted"))
        .filter_map(|r| parse_set_profile(&r.payload).map(|(_, p)| p))
        .collect();
    let valid_to = profiles.iter().filter_map(|p| p.valid_to).max();
    Accepted { profiles, valid_to }
}

fn check_accepted(ctx: &mut ScenarioCtx, received: &[ReceivedProfile]) {
    let answers: Vec<String> = received
        .iter()
        .map(|r| {
            r.answer
                .clone()
                .unwrap_or_else(|| "keine Antwort".to_string())
        })
        .collect();
    let all_accepted = answers.iter().all(|a| a == "Accepted");
    ctx.check(
        all_accepted,
        "Box nimmt das Profil an",
        format!("Antworten der Box: {}.", answers.join(", ")),
        format!(
            "Antworten der Box: {}; erwartet Accepted.",
            answers.join(", ")
        ),
    );
}

async fn check_power(
    ctx: &mut ScenarioCtx,
    name: &str,
    profiles: &[ChargingProfile],
    expectation: &str,
) {
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
    let expected = expected_power_now(
        profiles,
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
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let mut rx = ctx.handle.subscribe();
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
        "Box regelt auf die Grenze",
        &accepted.profiles,
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
    if !wait_until_or_fail(ctx, end, "Nach validTo lädt die Box mit voller Leistung").await {
        return;
    }
    check_power(
        ctx,
        "Nach validTo lädt die Box mit voller Leistung",
        &accepted.profiles,
        "Nach Ablauf",
    )
    .await;
}

/// S4: the server disappears during a limit; the box must keep the limit and end it on its own.
pub async fn s4_server_gone(ctx: &mut ScenarioCtx) {
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let mut rx = ctx.handle.subscribe();
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
        &accepted.profiles,
        "Offline mit Grenze",
    )
    .await;
    if is_connected(ctx) {
        ctx.fail(
            "Verbindung bleibt gesperrt",
            "Die Box ist trotz Sperre vor validTo wieder verbunden.",
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
    let still_offline = !is_connected(ctx);
    check_power(
        ctx,
        "Grenze endet zum Ablaufzeitpunkt ohne Zentrale",
        &accepted.profiles,
        "Nach validTo ohne Zentrale",
    )
    .await;
    ctx.check(
        still_offline,
        "Verbindung bleibt bis zum Ende der Sperre unterbrochen",
        "Die Box war nach validTo noch ohne Zentrale; die Grenze endete aus eigener Kraft.",
        "Die Box war schon vor dem Ende der Sperre wieder verbunden; der Offline-Nachweis gilt nicht.",
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
                "Die Box hat sich nach dem Ende der Sperre nicht wieder verbunden.",
            );
            return;
        }
        Wait::Aborted => return,
    }
    let boot = ctx
        .wait_frame(
            &mut rx,
            FrameDirection::Out,
            Duration::from_secs(30),
            |f| matches!(f, Frame::Call { action, .. } if action == "BootNotification"),
        )
        .await;
    match boot {
        Wait::Ready(_) => ctx.pass(
            "Neuanmeldung nach der Sperre",
            "Die Box hat sich mit einem neuen BootNotification angemeldet.",
        ),
        Wait::TimedOut => ctx.fail(
            "Neuanmeldung nach der Sperre",
            "Verbunden, aber kein neues BootNotification gesendet.",
        ),
        Wait::Aborted => {}
    }
}

fn is_connected(ctx: &ScenarioCtx) -> bool {
    matches!(
        ctx.handle.snapshot().connection,
        ConnectionState::Connected { .. }
    )
}

/// S6: the box rejects profiles; nothing may be applied and the connection must stay up.
pub async fn s6_box_rejects(ctx: &mut ScenarioCtx) {
    let mut config = ctx.handle.snapshot().config;
    config.reject_profiles = true;
    if let Err(error) = ctx.handle.set_config(config).await {
        ctx.fail("Box auf Ablehnen einstellen", error.to_string());
        return;
    }
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let mut rx = ctx.handle.subscribe();
    let Some(received) = receive_profiles(ctx, &mut rx, HINT_TRIGGER_TEST_LIMIT).await else {
        return;
    };
    let answers: Vec<String> = received
        .iter()
        .map(|r| {
            r.answer
                .clone()
                .unwrap_or_else(|| "keine Antwort".to_string())
        })
        .collect();
    let all_rejected = answers.iter().all(|a| a == "Rejected");
    ctx.check(
        all_rejected,
        "Box antwortet mit Rejected",
        format!("Antworten der Box: {}.", answers.join(", ")),
        format!(
            "Antworten der Box: {}; erwartet Rejected.",
            answers.join(", ")
        ),
    );
    if !ctx.sleep(ctx.tuning.limit_settle).await {
        return;
    }
    ctx.check(
        is_connected(ctx),
        "Verbindung bleibt bestehen",
        "Die Verbindung war nach der Ablehnung noch offen.",
        "Die Verbindung brach nach der Ablehnung ab.",
    );
    let snapshot = ctx.handle.snapshot();
    ctx.check(
        snapshot.active_limit.is_none() && snapshot.profile_count == 0,
        "Keine Grenze wirkt",
        "Die Box hat kein Profil gespeichert und lädt ohne Grenze.",
        format!(
            "Die Box hat {} Profil(e) gespeichert (Grenze: {:?}).",
            snapshot.profile_count, snapshot.active_limit
        ),
    );
}

/// S9: the schedule steers the box; the profile must apply and later be replaced or cleared.
pub async fn s9_schedule(ctx: &mut ScenarioCtx) {
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let mut rx = ctx.handle.subscribe();
    let hint = "Es kam kein SetChargingProfile an. Bitte in der App oder im Intranet einen Ladebedarf öffnen und \
                prüfen, dass lokal OCPP_AKTIV=aktiv gilt.";
    let Some(received) = receive_profiles(ctx, &mut rx, hint).await else {
        return;
    };
    check_accepted(ctx, &received);
    let accepted = accepted_profiles(&received);
    ctx.check(
        accepted.valid_to.is_some(),
        "Profil hat validTo",
        "Das Fahrplan-Profil hat ein validTo (mehr als 15 Minuten sind hier erlaubt).",
        "Kein angenommenes Profil mit validTo; die Box wüsste nie, wann ihre Pflicht endet.",
    );
    check_power(
        ctx,
        "Grenze aus dem Fahrplan wirkt",
        &accepted.profiles,
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
            "Bis zum Ende des Szenarios kam weder ein neues Profil noch ein ClearChargingProfile. Bitte den Ladebedarf \
             ändern oder schließen, damit sich der Fahrplan ändert.",
        ),
        Wait::Aborted => {}
    }
}
