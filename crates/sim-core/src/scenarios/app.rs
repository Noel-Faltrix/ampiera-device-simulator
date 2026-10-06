//! S11: what the customer app shows while the box charges, compared with what the box reports.
//!
//! The app view lives in another crate, so the shell hands in an [`AppProbe`]. The comparison rules are pure
//! functions with their own tests; a failing check is a finding about the central system, not about the simulator.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};

use super::ctx::ScenarioCtx;
use crate::model::{CheckOutcome, CheckResult};

/// Allowed deviation of the app's live power from the box power (SCENARIOS S11).
pub const LIVE_POWER_TOLERANCE_PCT: f64 = 10.0;

/// Allowed deviation of the app's quarter-hour energy from the box meter (SCENARIOS S11).
pub const QUARTER_ENERGY_TOLERANCE_PCT: f64 = 15.0;

/// Length of one energy interval in the backend (`resolution = quarter hour`).
const QUARTER_MINUTES: i64 = 15;

/// The device status the backend shows for a wallbox it cannot reach.
const STATUS_OFFLINE: &str = "offline";

/// The connection value the dashboard shows for a reachable installation.
const CONNECTION_ONLINE: &str = "online";

/// Largest variation of box power inside a quarter for which the energy comparison is still meaningful.
const CONSTANT_POWER_TOLERANCE_PCT: f64 = 5.0;

/// Values read from the customer app view; `None` means the app showed nothing (JSON null).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppProbeData {
    /// Dashboard `live.wallbox_leistung_w`.
    pub live_power_w: Option<f64>,
    /// Dashboard device status of the wallbox.
    pub device_status: Option<String>,
    /// Dashboard `anlagen[].verbindung`.
    pub connection: Option<String>,
    /// Latest wallbox quarter-hour value in kWh.
    pub last_quarter_kwh: Option<f64>,
    /// Start of that quarter hour.
    pub last_quarter_at: Option<DateTime<Utc>>,
}

/// Source of app view data, implemented by the shell on top of the `app_view` crate.
#[async_trait]
pub trait AppProbe: Send + Sync {
    /// Reads the current app view; the error is a German message.
    async fn snapshot(&self) -> Result<AppProbeData, String>;
}

/// Box meter reading at a point in time.
#[derive(Debug, Clone, PartialEq)]
pub struct EnergySample {
    /// When it was read.
    pub at: DateTime<Utc>,
    /// Meter register in Wh.
    pub energy_wh: f64,
    /// Power at that moment in W.
    pub power_w: Option<f64>,
}

fn result(name: &str, outcome: CheckOutcome, detail: String) -> CheckResult {
    CheckResult {
        name: name.to_string(),
        outcome,
        detail,
    }
}

/// Dashboard connection must be `online` while the box talks to the central system.
pub fn check_connection(connection: Option<&str>) -> CheckResult {
    const NAME: &str = "Verbindung in der App";
    match connection {
        Some(c) if c.eq_ignore_ascii_case(CONNECTION_ONLINE) => result(
            NAME,
            CheckOutcome::Passed,
            "Die App zeigt die Anlage als online.".into(),
        ),
        Some(other) => result(
            NAME,
            CheckOutcome::Failed,
            format!("Die App zeigt „{other}“, erwartet „online“."),
        ),
        None => result(
            NAME,
            CheckOutcome::Failed,
            "Die App liefert keinen Verbindungsstatus (null).".into(),
        ),
    }
}

/// The wallbox must not be shown as offline while it sends data.
pub fn check_device_status(status: Option<&str>) -> CheckResult {
    const NAME: &str = "Gerätestatus in der App";
    match status {
        Some(s) if s.eq_ignore_ascii_case(STATUS_OFFLINE) => result(
            NAME,
            CheckOutcome::Failed,
            "Die App zeigt die Wallbox als „offline“, obwohl sie Daten sendet.".into(),
        ),
        Some(s) => result(
            NAME,
            CheckOutcome::Passed,
            format!("Die App zeigt den Status „{s}“."),
        ),
        None => result(
            NAME,
            CheckOutcome::Failed,
            "Die App liefert keinen Gerätestatus (null).".into(),
        ),
    }
}

fn within_pct(app: f64, reference: f64, tolerance_pct: f64) -> bool {
    if reference == 0.0 {
        return app == 0.0;
    }
    ((app - reference) / reference).abs() * 100.0 <= tolerance_pct
}

/// Live power of the app against the box power (±10 %). A missing box value skips the check; a missing app
/// value fails it, because that is exactly the gap this scenario looks for.
pub fn check_live_power(app_w: Option<f64>, box_w: Option<f64>) -> CheckResult {
    const NAME: &str = "Live-Leistung in der App";
    let Some(box_w) = box_w else {
        return result(
            NAME,
            CheckOutcome::Skipped,
            "Die Box meldet gerade keine Leistung, ein Vergleich ist nicht möglich.".into(),
        );
    };
    let Some(app_w) = app_w else {
        return result(
            NAME,
            CheckOutcome::Failed,
            format!("Die App liefert keine Live-Leistung (null), die Box lädt mit {box_w:.0} W."),
        );
    };
    let ok = within_pct(app_w, box_w, LIVE_POWER_TOLERANCE_PCT);
    let outcome = if ok {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Failed
    };
    result(
        NAME,
        outcome,
        format!("App {app_w:.0} W, Box {box_w:.0} W (erlaubt ±{LIVE_POWER_TOLERANCE_PCT} %)."),
    )
}

/// Quarter-hour energy of the app against the box meter (±15 %).
pub fn check_quarter_energy(app_kwh: Option<f64>, box_kwh: f64) -> CheckResult {
    const NAME: &str = "Viertelstunden-kWh in der App";
    let Some(app_kwh) = app_kwh else {
        return result(
            NAME,
            CheckOutcome::Failed,
            format!("Die App liefert keinen Viertelstundenwert (null), die Box hat {box_kwh:.3} kWh gemessen."),
        );
    };
    let ok = within_pct(app_kwh, box_kwh, QUARTER_ENERGY_TOLERANCE_PCT);
    let outcome = if ok {
        CheckOutcome::Passed
    } else {
        CheckOutcome::Failed
    };
    result(
        NAME,
        outcome,
        format!("App {app_kwh:.3} kWh, Box {box_kwh:.3} kWh (erlaubt ±{QUARTER_ENERGY_TOLERANCE_PCT} %)."),
    )
}

/// The quarter hour that starts at `at`. The backend labels a quarter with its start time (assumption taken
/// from the interval start convention of `geraete_messwerte`); if that proves wrong, only this function changes.
pub fn quarter_window(at: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    (at, at + ChronoDuration::minutes(QUARTER_MINUTES))
}

fn interpolate(samples: &[EnergySample], at: DateTime<Utc>) -> Option<f64> {
    let first = samples.first()?;
    let last = samples.last()?;
    if at < first.at || at > last.at {
        return None;
    }
    let upper = samples.iter().position(|s| s.at >= at)?;
    let hi = &samples[upper];
    if hi.at == at || upper == 0 {
        return Some(hi.energy_wh);
    }
    let lo = &samples[upper - 1];
    let span = (hi.at - lo.at).num_milliseconds() as f64;
    let fraction = (at - lo.at).num_milliseconds() as f64 / span;
    Some(lo.energy_wh + (hi.energy_wh - lo.energy_wh) * fraction)
}

/// Energy the box meter counted between `from` and `to` in kWh, interpolated linearly between samples.
/// `None` when the samples do not cover the whole window (partial windows would understate the energy).
pub fn energy_in_window(
    samples: &[EnergySample],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Option<f64> {
    let start = interpolate(samples, from)?;
    let end = interpolate(samples, to)?;
    Some((end - start) / 1000.0)
}

/// True when the box power inside the window stayed within 5 % of its mean; otherwise the comparison with a
/// quarter-hour value is not about "constant power" any more.
pub fn power_is_constant(samples: &[EnergySample], from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
    let powers: Vec<f64> = samples
        .iter()
        .filter(|s| s.at >= from && s.at <= to)
        .filter_map(|s| s.power_w)
        .collect();
    if powers.is_empty() {
        return false;
    }
    let mean = powers.iter().sum::<f64>() / powers.len() as f64;
    powers
        .iter()
        .all(|p| within_pct(*p, mean, CONSTANT_POWER_TOLERANCE_PCT))
}

/// Decides the quarter-hour check from the latest app value and the box samples.
pub fn judge_quarter(data: &AppProbeData, samples: &[EnergySample]) -> CheckResult {
    const NAME: &str = "Viertelstunden-kWh in der App";
    let Some(at) = data.last_quarter_at else {
        return result(
            NAME,
            CheckOutcome::Failed,
            "Die App liefert noch keinen Viertelstundenwert für die Wallbox (null).".into(),
        );
    };
    let (from, to) = quarter_window(at);
    let Some(box_kwh) = energy_in_window(samples, from, to) else {
        return result(
            NAME,
            CheckOutcome::Skipped,
            format!(
                "Die App zeigt die Viertelstunde ab {}, aber die Box hat das Fenster nicht vollständig gemessen \
                 (Ladebeginn oder Abruf lagen dazwischen); ein Vergleich wäre verfälscht.",
                at.format("%H:%M")
            ),
        );
    };
    if !power_is_constant(samples, from, to) {
        return result(
            NAME,
            CheckOutcome::Skipped,
            "Die Leistung war in dieser Viertelstunde nicht konstant; der Vergleich gilt nur für gleichbleibende Leistung.".into(),
        );
    }
    check_quarter_energy(data.last_quarter_kwh, box_kwh)
}

/// True when the app's quarter value can be judged against the samples collected so far.
fn quarter_is_decidable(data: &AppProbeData, samples: &[EnergySample]) -> bool {
    data.last_quarter_at.is_some_and(|at| {
        energy_in_window(samples, quarter_window(at).0, quarter_window(at).1).is_some()
    })
}

/// S11: charge at constant power and poll the app view.
pub async fn s11_app_view(ctx: &mut ScenarioCtx) {
    let Some(probe) = ctx.probe.clone() else {
        for name in [
            "Verbindung in der App",
            "Gerätestatus in der App",
            "Live-Leistung in der App",
            "Viertelstunden-kWh in der App",
        ] {
            ctx.skip(
                name,
                "Die App-Ansicht ist nicht angemeldet. Bitte zuerst in der App-Ansicht anmelden, dann das Szenario neu starten.",
            );
        }
        return;
    };
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let started = tokio::time::Instant::now();
    let mut samples: Vec<EnergySample> = Vec::new();
    let mut latest: Option<(AppProbeData, Option<f64>)> = None;
    let mut last_error: Option<String> = None;
    loop {
        let box_snapshot = ctx.handle.snapshot();
        samples.push(EnergySample {
            at: Utc::now(),
            energy_wh: box_snapshot.energy_wh,
            power_w: box_snapshot.power_w,
        });
        match probe.snapshot().await {
            Ok(data) => latest = Some((data, box_snapshot.power_w)),
            Err(message) => last_error = Some(message),
        }
        let long_enough = started.elapsed() >= ctx.tuning.min_charge_duration;
        let decided = latest
            .as_ref()
            .is_some_and(|(data, _)| quarter_is_decidable(data, &samples));
        let out_of_time = ctx.remaining() <= ctx.tuning.app_poll_interval
            || ctx
                .tuning
                .app_max_wait
                .is_some_and(|max| started.elapsed() >= max);
        if (long_enough && decided) || out_of_time {
            break;
        }
        let interval: Duration = ctx.tuning.app_poll_interval;
        if !ctx.sleep(interval).await {
            return;
        }
    }
    let Some((data, box_power)) = latest else {
        ctx.fail(
            "App-Ansicht abrufen",
            format!(
                "Die App-Ansicht konnte nie abgerufen werden: {}",
                last_error.unwrap_or_else(|| "keine Fehlermeldung".to_string())
            ),
        );
        return;
    };
    ctx.push_result(check_connection(data.connection.as_deref()));
    ctx.push_result(check_device_status(data.device_status.as_deref()));
    ctx.push_result(check_live_power(data.live_power_w, box_power));
    ctx.push_result(judge_quarter(&data, &samples));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 10, 0, 0).unwrap() + ChronoDuration::minutes(minute)
    }

    /// 7.2 kW constant: 120 Wh per minute.
    fn constant_samples(from_minute: i64, to_minute: i64) -> Vec<EnergySample> {
        (from_minute..=to_minute)
            .map(|m| EnergySample {
                at: t(m),
                energy_wh: (m - from_minute) as f64 * 120.0,
                power_w: Some(7200.0),
            })
            .collect()
    }

    #[test]
    fn connection_must_be_online() {
        assert_eq!(
            check_connection(Some("online")).outcome,
            CheckOutcome::Passed
        );
        assert_eq!(
            check_connection(Some("offline")).outcome,
            CheckOutcome::Failed
        );
        assert_eq!(
            check_connection(None).outcome,
            CheckOutcome::Failed,
            "null is not online"
        );
    }

    #[test]
    fn device_status_offline_or_missing_fails() {
        assert_eq!(
            check_device_status(Some("laedt")).outcome,
            CheckOutcome::Passed
        );
        assert_eq!(
            check_device_status(Some("offline")).outcome,
            CheckOutcome::Failed
        );
        assert_eq!(check_device_status(None).outcome, CheckOutcome::Failed);
    }

    #[test]
    fn live_power_tolerance_is_ten_percent_and_null_is_a_finding() {
        assert_eq!(
            check_live_power(Some(7000.0), Some(7200.0)).outcome,
            CheckOutcome::Passed
        );
        assert_eq!(
            check_live_power(Some(7920.0), Some(7200.0)).outcome,
            CheckOutcome::Passed,
            "exactly +10 %"
        );
        assert_eq!(
            check_live_power(Some(7921.0), Some(7200.0)).outcome,
            CheckOutcome::Failed
        );
        assert_eq!(
            check_live_power(Some(6000.0), Some(7200.0)).outcome,
            CheckOutcome::Failed
        );
        assert_eq!(
            check_live_power(None, Some(7200.0)).outcome,
            CheckOutcome::Failed
        );
        assert_eq!(
            check_live_power(Some(7000.0), None).outcome,
            CheckOutcome::Skipped
        );
    }

    #[test]
    fn quarter_energy_tolerance_is_fifteen_percent() {
        assert_eq!(
            check_quarter_energy(Some(1.8), 1.8).outcome,
            CheckOutcome::Passed
        );
        assert_eq!(
            check_quarter_energy(Some(2.0), 1.8).outcome,
            CheckOutcome::Passed
        );
        assert_eq!(
            check_quarter_energy(Some(2.2), 1.8).outcome,
            CheckOutcome::Failed
        );
        assert_eq!(
            check_quarter_energy(Some(1.5), 1.8).outcome,
            CheckOutcome::Failed
        );
        assert_eq!(
            check_quarter_energy(None, 1.8).outcome,
            CheckOutcome::Failed
        );
    }

    #[test]
    fn window_energy_is_interpolated_between_samples() {
        let samples = constant_samples(0, 30);
        let kwh = energy_in_window(&samples, t(5), t(20)).unwrap();
        assert!((kwh - 1.8).abs() < 1e-9, "15 minutes at 7.2 kW are 1.8 kWh");
        let half = EnergySample {
            at: t(40),
            energy_wh: 30.0 * 120.0 + 1200.0,
            power_w: Some(7200.0),
        };
        let mut sparse = vec![samples[0].clone(), samples[30].clone(), half];
        sparse.sort_by_key(|s| s.at);
        let kwh = energy_in_window(
            &sparse,
            t(0) + ChronoDuration::seconds(150),
            t(15) + ChronoDuration::seconds(150),
        )
        .unwrap();
        assert!(
            (kwh - 1.8).abs() < 1e-9,
            "linear interpolation across sparse samples"
        );
    }

    #[test]
    fn window_outside_the_samples_is_not_decidable() {
        let samples = constant_samples(5, 25);
        assert!(
            energy_in_window(&samples, t(0), t(15)).is_none(),
            "starts before the first sample"
        );
        assert!(
            energy_in_window(&samples, t(15), t(30)).is_none(),
            "ends after the last sample"
        );
        assert!(energy_in_window(&samples, t(5), t(20)).is_some());
        assert!(energy_in_window(&[], t(5), t(20)).is_none());
    }

    #[test]
    fn constant_power_rule_has_a_counter_case() {
        let mut samples = constant_samples(0, 30);
        assert!(power_is_constant(&samples, t(0), t(15)));
        samples[8].power_w = Some(3000.0);
        assert!(!power_is_constant(&samples, t(0), t(15)));
        assert!(
            power_is_constant(&samples, t(15), t(30)),
            "the variation lies outside this window"
        );
    }

    #[test]
    fn judging_the_quarter_covers_pass_fail_skip() {
        let samples = constant_samples(0, 40);
        let ok = AppProbeData {
            last_quarter_kwh: Some(1.75),
            last_quarter_at: Some(t(15)),
            ..Default::default()
        };
        assert_eq!(judge_quarter(&ok, &samples).outcome, CheckOutcome::Passed);
        let bad = AppProbeData {
            last_quarter_kwh: Some(0.5),
            last_quarter_at: Some(t(15)),
            ..Default::default()
        };
        assert_eq!(judge_quarter(&bad, &samples).outcome, CheckOutcome::Failed);
        let none = AppProbeData::default();
        assert_eq!(
            judge_quarter(&none, &samples).outcome,
            CheckOutcome::Failed,
            "no quarter value is a finding"
        );
        let uncovered = AppProbeData {
            last_quarter_kwh: Some(1.8),
            last_quarter_at: Some(t(30)),
            ..Default::default()
        };
        assert_eq!(
            judge_quarter(&uncovered, &samples).outcome,
            CheckOutcome::Skipped
        );
        assert!(!quarter_is_decidable(&uncovered, &samples));
        assert!(quarter_is_decidable(&ok, &samples));
    }

    #[test]
    fn quarter_window_is_fifteen_minutes() {
        let (from, to) = quarter_window(t(15));
        assert_eq!(to - from, ChronoDuration::minutes(15));
    }
}
