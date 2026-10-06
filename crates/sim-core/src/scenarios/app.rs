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

/// Appended to every statement about the quarter-hour value: the labelling convention is an assumption.
const LABEL_ASSUMPTION: &str =
    "Annahme: Die App beschriftet eine Viertelstunde mit ihrem Beginn. Das lässt sich \
                                aus dem Code der Zentrale nicht prüfen.";

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

/// Wallbox meter reading at a point in time.
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

/// Current power of the app against the wallbox power (±10 %). A missing wallbox value skips the check; a
/// missing app value fails it, because that is exactly the gap this scenario looks for.
pub fn check_live_power(app_w: Option<f64>, box_w: Option<f64>) -> CheckResult {
    const NAME: &str = "Aktuelle Leistung in der App";
    let Some(box_w) = box_w else {
        return result(
            NAME,
            CheckOutcome::Skipped,
            "Die Wallbox meldet gerade keine Leistung, ein Vergleich ist nicht möglich.".into(),
        );
    };
    let Some(app_w) = app_w else {
        return result(
            NAME,
            CheckOutcome::Failed,
            format!("Die App liefert keine aktuelle Leistung (null), die Wallbox lädt mit {box_w:.0} W."),
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
        format!("App {app_w:.0} W, Wallbox {box_w:.0} W (erlaubt ±{LIVE_POWER_TOLERANCE_PCT} %)."),
    )
}

/// Quarter-hour energy of the app against the box meter (±15 %).
pub fn check_quarter_energy(app_kwh: Option<f64>, box_kwh: f64) -> CheckResult {
    const NAME: &str = "Viertelstunden-kWh in der App";
    let Some(app_kwh) = app_kwh else {
        return result(
            NAME,
            CheckOutcome::Failed,
            format!("Die App liefert keinen Viertelstundenwert (null), die Wallbox hat {box_kwh:.3} kWh gemessen."),
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
        format!("App {app_kwh:.3} kWh, Wallbox {box_kwh:.3} kWh (erlaubt ±{QUARTER_ENERGY_TOLERANCE_PCT} %)."),
    )
}

/// The quarter hour that starts at `at`. This is the single place of the convention "the label of a quarter is
/// its START" (assumed from the interval start convention of `geraete_messwerte`; it cannot be verified from the
/// backend code). If it proves wrong, only this function changes.
pub fn quarter_window(at: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    (at, at + ChronoDuration::minutes(QUARTER_MINUTES))
}

/// The first clock-aligned quarter hour that starts at or after `at`.
pub fn next_quarter_start(at: DateTime<Utc>) -> DateTime<Utc> {
    let quarter_s = QUARTER_MINUTES * 60;
    let ts = at.timestamp();
    let aligned = (ts + quarter_s - 1).div_euclid(quarter_s) * quarter_s;
    DateTime::from_timestamp(aligned, 0).unwrap_or(at)
}

/// When the app's quarter value can be judged: the first full quarter after `charge_start` has ended and the
/// backend's job has had `backend_job_wait` to compute it. Returns that quarter's start and the time.
pub fn quarter_ready_at(
    charge_start: DateTime<Utc>,
    backend_job_wait: Duration,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = next_quarter_start(charge_start);
    let wait = ChronoDuration::from_std(backend_job_wait).unwrap_or_default();
    (start, quarter_window(start).1 + wait)
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

/// Decides the quarter-hour check from the latest app value and the wallbox samples. Every detail states the
/// labelling assumption (see [`quarter_window`]).
pub fn judge_quarter(data: &AppProbeData, samples: &[EnergySample]) -> CheckResult {
    let mut checked = judge_quarter_inner(data, samples);
    checked.detail = format!("{} {LABEL_ASSUMPTION}", checked.detail);
    checked
}

fn judge_quarter_inner(data: &AppProbeData, samples: &[EnergySample]) -> CheckResult {
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
                "Die App zeigt die Viertelstunde ab {}, aber die Wallbox hat das Fenster nicht vollständig gemessen \
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

/// The checks S11 cannot run without an app login.
const S11_CHECK_NAMES: [&str; 4] = [
    "Verbindung in der App",
    "Gerätestatus in der App",
    "Aktuelle Leistung in der App",
    "Viertelstunden-kWh in der App",
];

/// S11: charge at constant power from the start and poll the app view until one full clock-aligned quarter hour
/// is measured and the backend's job has run.
pub async fn s11_app_view(ctx: &mut ScenarioCtx) {
    let Some(probe) = ctx.probe.clone() else {
        for name in S11_CHECK_NAMES {
            ctx.skip(
                name,
                "Die App-Sicht ist nicht angemeldet. Melde dich zuerst in der App-Sicht an und starte das \
                 Szenario dann neu.",
            );
        }
        return;
    };
    if !ctx.ensure_connected().await || !ctx.ensure_charging().await {
        return;
    }
    let charge_start = Utc::now();
    let (quarter_start, ready_at) = quarter_ready_at(charge_start, ctx.tuning.backend_job_wait);
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
        let out_of_time = ctx.remaining() <= ctx.tuning.app_poll_interval
            || ctx
                .tuning
                .app_max_wait
                .is_some_and(|max| started.elapsed() >= max);
        if Utc::now() >= ready_at || out_of_time {
            break;
        }
        let interval: Duration = ctx.tuning.app_poll_interval;
        if !ctx.sleep(interval).await {
            return;
        }
    }
    let Some((data, box_power)) = latest else {
        ctx.fail(
            "App-Sicht abrufen",
            format!(
                "Die App-Sicht konnte nie abgerufen werden: {}",
                last_error.unwrap_or_else(|| "keine Fehlermeldung".to_string())
            ),
        );
        return;
    };
    ctx.push_result(check_connection(data.connection.as_deref()));
    ctx.push_result(check_device_status(data.device_status.as_deref()));
    ctx.push_result(check_live_power(data.live_power_w, box_power));
    let mut quarter = judge_quarter(&data, &samples);
    if data.last_quarter_at.is_some_and(|at| at != quarter_start) {
        quarter.detail = format!(
            "Gewartet wurde auf die Viertelstunde ab {}. {}",
            quarter_start.format("%H:%M"),
            quarter.detail
        );
    }
    ctx.push_result(quarter);
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
    }

    #[test]
    fn quarter_window_is_fifteen_minutes() {
        let (from, to) = quarter_window(t(15));
        assert_eq!(to - from, ChronoDuration::minutes(15));
    }

    #[test]
    fn quarter_details_state_the_labelling_assumption() {
        let samples = constant_samples(0, 40);
        let ok = AppProbeData {
            last_quarter_kwh: Some(1.75),
            last_quarter_at: Some(t(15)),
            ..Default::default()
        };
        assert!(judge_quarter(&ok, &samples)
            .detail
            .contains("beschriftet eine Viertelstunde mit ihrem Beginn"));
        let none = AppProbeData::default();
        assert!(judge_quarter(&none, &samples).detail.contains("Annahme"));
    }

    #[test]
    fn next_quarter_start_is_clock_aligned_and_never_earlier() {
        assert_eq!(next_quarter_start(t(0)), t(0), "already aligned");
        assert_eq!(next_quarter_start(t(1)), t(15));
        assert_eq!(
            next_quarter_start(t(14) + ChronoDuration::seconds(59)),
            t(15)
        );
        assert_eq!(
            next_quarter_start(t(15) + ChronoDuration::seconds(1)),
            t(30)
        );
    }

    #[test]
    fn ready_time_is_one_full_quarter_plus_the_backend_job() {
        let start = t(7);
        let (quarter, ready) = quarter_ready_at(start, Duration::from_secs(240));
        assert_eq!(quarter, t(15));
        assert_eq!(ready, t(30) + ChronoDuration::minutes(4));
        let worst_case =
            quarter_ready_at(t(0) + ChronoDuration::seconds(1), Duration::from_secs(240));
        assert!(
            worst_case.1 - (t(0) + ChronoDuration::seconds(1)) < ChronoDuration::minutes(40),
            "counter-check: even the worst alignment fits into the 40 minute scenario timeout"
        );
    }
}
