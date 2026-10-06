//! The twelve test scenarios S1..S11 (incl. S6b) from `docs/SCENARIOS.md`.
//!
//! Each scenario drives one simulated box and records German checks. `run` wraps one scenario into a report
//! and restores the box configuration afterwards.

mod app;
mod auth;
mod basic;
pub mod ctx;
mod limits;
pub mod rules;

use chrono::Utc;

pub use app::{AppProbe, AppProbeData};
pub use ctx::{ScenarioCtx, ScenarioTuning};

use crate::model::{ScenarioId, ScenarioInfo, ScenarioReport};

struct Entry {
    id: ScenarioId,
    title: &'static str,
    description: &'static str,
    live_allowed: bool,
    needs_human: bool,
    timeout_s: u64,
}

const CATALOG: [Entry; 12] = [
    Entry {
        id: ScenarioId::S1,
        title: "Anmelden, Status, Heartbeat",
        description: "Die Box meldet sich neu an (BootNotification). Geprüft wird, dass die Zentrale mit Accepted und \
                      Heartbeat-Intervall antwortet, dass die Aufrufe nach dem Start in der richtigen Reihenfolge \
                      kommen und dass ein Heartbeat mit currentTime beantwortet wird. Es ist nichts zu tun.",
        live_allowed: true,
        needs_human: false,
        timeout_s: 90,
    },
    Entry {
        id: ScenarioId::S2,
        title: "Auto anstecken und laden",
        description: "Ein Standardfahrzeug wird angesteckt. Geprüft werden Status Preparing, StartTransaction mit \
                      transactionId ab 1, Status Charging und zwei MeterValues mit Leistung größer 0. Das dauert \
                      etwa zwei Messintervalle. Es ist nichts zu tun.",
        live_allowed: true,
        needs_human: false,
        timeout_s: 180,
    },
    Entry {
        id: ScenarioId::S3,
        title: "Testgrenze aus dem Intranet",
        description: "Die Box lädt, dann muss im Intranet die „Testgrenze“ für diese Box ausgelöst werden. Geprüft \
                      wird, dass das Profil höchstens 15 Minuten gültig ist, die Box auf die Grenze (±1 %) regelt \
                      und nach validTo plus 30 s wieder mit voller Leistung lädt.",
        live_allowed: true,
        needs_human: true,
        timeout_s: 20 * 60,
    },
    Entry {
        id: ScenarioId::S4,
        title: "Server weg während einer Grenze",
        description: "Die Box lädt, dann muss im Intranet eine Testgrenze ausgelöst werden. Nach dem Profil trennt \
                      die Box die Verbindung und baut sie erst nach validTo plus 30 s wieder auf. Geprüft wird, \
                      dass die Grenze offline gilt, ohne Zentrale zum Ablaufzeitpunkt endet und die Neuanmeldung \
                      klappt.",
        live_allowed: true,
        needs_human: true,
        timeout_s: 20 * 60,
    },
    Entry {
        id: ScenarioId::S5,
        title: "StopTransaction mit transactionId 0",
        description: "Die Box sendet ein StopTransaction mit transactionId 0 (so endet ein Ladevorgang, den die \
                      Zentrale nicht gespeichert hat). Geprüft wird, dass die Zentrale mit Accepted antwortet, \
                      keinen Fehler meldet und die Verbindung hält. Es ist nichts zu tun.",
        live_allowed: true,
        needs_human: false,
        timeout_s: 60,
    },
    Entry {
        id: ScenarioId::S6,
        title: "Box lehnt Profil ab",
        description: "Die Box ist so eingestellt, dass sie Ladeprofile ablehnt. Dann muss im Intranet eine Testgrenze \
                      ausgelöst werden. Geprüft wird, dass die Box mit Rejected antwortet, die Verbindung bleibt \
                      und keine Grenze wirkt.",
        live_allowed: true,
        needs_human: true,
        timeout_s: 20 * 60,
    },
    Entry {
        id: ScenarioId::S6b,
        title: "Box ohne Ladestand (SoC)",
        description: "Die Box meldet keinen Ladestand und startet neu. Geprüft wird, dass sie die erste \
                      Konfiguration mit SoC mit Rejected beantwortet und die Zentrale danach eine Konfiguration \
                      ohne SoC schickt. Es ist nichts zu tun.",
        live_allowed: true,
        needs_human: false,
        timeout_s: 90,
    },
    Entry {
        id: ScenarioId::S7,
        title: "Zweite Verbindung derselben Kennung",
        description: "Ein zweiter Socket mit derselben Kennung wird geöffnet. Geprüft wird, dass die Zentrale die \
                      erste Verbindung schließt (Code 1000). Es ist nichts zu tun.",
        live_allowed: true,
        needs_human: false,
        timeout_s: 60,
    },
    Entry {
        id: ScenarioId::S8,
        title: "Uhr der Box geht falsch",
        description: "Die Uhr der Box geht 15 Minuten nach, während sie lädt. Geprüft wird, dass die Zentrale die \
                      MeterValues trotzdem beantwortet. Dass sie die alten Werte verwirft, ist über OCPP nicht \
                      sichtbar und steht nur als Hinweis im Bericht. Es ist nichts zu tun.",
        live_allowed: true,
        needs_human: false,
        timeout_s: 180,
    },
    Entry {
        id: ScenarioId::S9,
        title: "Fahrplan steuert die Box",
        description: "Nur lokal. Es muss ein Ladebedarf in der App oder im Intranet geöffnet sein und lokal \
                      OCPP_AKTIV=aktiv gelten. Die Box lädt und wartet auf ein Profil aus dem Fahrplan (validTo darf \
                      länger als 15 Minuten sein). Geprüft wird, dass die Grenze wirkt und das Profil bei einer \
                      Fahrplanänderung ersetzt oder gelöscht wird.",
        live_allowed: false,
        needs_human: true,
        timeout_s: 30 * 60,
    },
    Entry {
        id: ScenarioId::S10,
        title: "Falsches Passwort",
        description: "Nur lokal. Die Box verbindet sich dreimal mit falschem Passwort. Geprüft wird, dass jedes Mal \
                      HTTP 401 kommt, kein Hinweis auf den falschen Teil und dass die Box nicht automatisch \
                      weiterprobiert. Es ist nichts zu tun.",
        live_allowed: false,
        needs_human: false,
        timeout_s: 60,
    },
    Entry {
        id: ScenarioId::S11,
        title: "App-Sicht während des Ladens",
        description: "Die Box lädt mindestens 15 Minuten mit gleichbleibender Leistung, die App-Ansicht wird alle \
                      30 s abgefragt. Dafür muss in der App-Ansicht angemeldet sein. Geprüft werden Verbindung, \
                      Gerätestatus, Live-Leistung (±10 %) und Viertelstunden-kWh (±15 %). Fehler bei Live-Leistung \
                      und Gerätestatus sind ein bekannter Befund der Zentrale, kein Fehler des Simulators.",
        live_allowed: true,
        needs_human: true,
        timeout_s: 25 * 60,
    },
];

/// Static description of all scenarios for the UI.
pub fn catalog() -> Vec<ScenarioInfo> {
    CATALOG
        .iter()
        .map(|e| ScenarioInfo {
            id: e.id,
            title: e.title.to_string(),
            description: e.description.to_string(),
            live_allowed: e.live_allowed,
            needs_human: e.needs_human,
            timeout_s: e.timeout_s,
        })
        .collect()
}

/// Info of one scenario.
pub fn info(id: ScenarioId) -> ScenarioInfo {
    catalog()
        .into_iter()
        .find(|i| i.id == id)
        .expect("every ScenarioId has a catalog entry")
}

/// Runs one scenario and builds its report. The box configuration is restored afterwards, whatever happened.
pub async fn run(id: ScenarioId, mut ctx: ScenarioCtx) -> ScenarioReport {
    let started_at = Utc::now();
    let snapshot = ctx.handle.snapshot();
    let handle = ctx.handle.clone();
    let original = ctx.original_config.clone();
    if !ctx.info.live_allowed && snapshot.config.target_kind == crate::model::TargetKind::Live {
        ctx.fail(
            "Szenario auf Live-System",
            "Dieses Szenario darf nicht gegen das Live-System laufen und wurde nicht ausgeführt.",
        );
    } else {
        dispatch(id, &mut ctx).await;
    }
    if handle.snapshot().config != original {
        if let Err(error) = handle.set_config(original).await {
            ctx.fail("Einstellungen zurücksetzen", error.to_string());
        }
    }
    let (checks, aborted) = ctx.finish();
    ScenarioReport {
        scenario_id: id,
        charge_point_id: snapshot.id,
        target_kind: snapshot.config.target_kind,
        started_at,
        finished_at: Utc::now(),
        outcome: rules::outcome_of(&checks, aborted),
        checks,
    }
}

async fn dispatch(id: ScenarioId, ctx: &mut ScenarioCtx) {
    match id {
        ScenarioId::S1 => basic::s1_boot(ctx).await,
        ScenarioId::S2 => basic::s2_plug_in_and_charge(ctx).await,
        ScenarioId::S3 => limits::s3_test_limit(ctx).await,
        ScenarioId::S4 => limits::s4_server_gone(ctx).await,
        ScenarioId::S5 => basic::s5_stop_transaction_zero(ctx).await,
        ScenarioId::S6 => limits::s6_box_rejects(ctx).await,
        ScenarioId::S6b => basic::s6b_no_soc(ctx).await,
        ScenarioId::S7 => basic::s7_second_connection(ctx).await,
        ScenarioId::S8 => basic::s8_wrong_clock(ctx).await,
        ScenarioId::S9 => limits::s9_schedule(ctx).await,
        ScenarioId::S10 => auth::s10_wrong_password(ctx).await,
        ScenarioId::S11 => app::s11_app_view(ctx).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_matches_scenarios_md() {
        let all = catalog();
        assert_eq!(all.len(), 12);
        let flags = |id| {
            let i = all.iter().find(|i| i.id == id).unwrap();
            (i.live_allowed, i.needs_human, i.timeout_s)
        };
        assert_eq!(flags(ScenarioId::S1), (true, false, 90));
        assert_eq!(flags(ScenarioId::S3), (true, true, 1200));
        assert_eq!(flags(ScenarioId::S9), (false, true, 1800));
        assert_eq!(flags(ScenarioId::S10), (false, false, 60));
        assert_eq!(flags(ScenarioId::S11), (true, true, 1500));
        let live_forbidden: Vec<_> = all
            .iter()
            .filter(|i| !i.live_allowed)
            .map(|i| i.id)
            .collect();
        assert_eq!(live_forbidden, vec![ScenarioId::S9, ScenarioId::S10]);
    }

    #[test]
    fn texts_use_real_umlauts_not_transliterations() {
        let transliterations = [
            "fuer ",
            "ueber",
            "gueltig",
            "Gueltig",
            "Aenderung",
            "pruef",
            "muessen",
            "koennen",
        ];
        for i in catalog() {
            for text in [&i.title, &i.description] {
                assert!(
                    !transliterations.iter().any(|t| text.contains(t)),
                    "{}: transliterated umlaut in {text}",
                    i.title
                );
            }
        }
        assert!(info(ScenarioId::S3).description.contains("„Testgrenze“"));
    }

    #[test]
    fn every_id_has_info() {
        for id in [ScenarioId::S6b, ScenarioId::S11] {
            assert_eq!(info(id).id, id);
        }
    }
}
