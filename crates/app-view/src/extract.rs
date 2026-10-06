//! Pure extraction of the summary values from the backend JSON. Nothing here touches the network, so
//! every rule is covered by unit tests. Missing or mistyped values become `None`, never 0.

use crate::types::NextScheduleStep;
use chrono::{DateTime, Days, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use chrono_tz::Europe::Berlin;
use serde_json::Value;

const MAX_SCHEDULE_STEPS: usize = 4;

/// Values taken from `GET /dashboard`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DashboardSummary {
    /// Id of the chosen installation.
    pub installation_id: Option<String>,
    /// `verbindung` of the installation.
    pub connection: Option<String>,
    /// `letzter_kontakt` of the installation.
    pub last_contact: Option<String>,
    /// Status of the chosen wallbox device.
    pub device_status: Option<String>,
    /// `live.wallbox_leistung_w`.
    pub live_power_w: Option<f64>,
}

/// Values taken from the geo-position view.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GeoSummary {
    /// `zustand` of the wallbox.
    pub state: Option<String>,
    /// `ueberschrift`.
    pub headline: Option<String>,
}

/// Values taken from the quarter-hour device energy.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnergySummary {
    /// `kwhPositiv` of the latest value.
    pub kwh: Option<f64>,
    /// `zeit` of the latest value, ISO 8601 UTC.
    pub at: Option<String>,
}

fn is_wallbox(device: &Value) -> bool {
    device.get("typ").and_then(Value::as_str) == Some("wallbox")
}

fn is_ocpp_wallbox(device: &Value) -> bool {
    is_wallbox(device) && device.get("anbindung").and_then(Value::as_str) == Some("ocpp")
}

fn devices_of(installation: &Value) -> &[Value] {
    installation
        .get("geraete")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Ids arrive as strings or numbers depending on the table; both become a string.
fn id_string(value: &Value) -> Option<String> {
    match value.get("id")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Parses an ISO 8601 timestamp; a missing offset means UTC.
pub fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|naive| naive.and_utc())
}

fn iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Picks the installation that has an OCPP wallbox; falls back to the first installation.
pub fn pick_installation(dashboard: &Value) -> Option<&Value> {
    let installations = dashboard.get("anlagen")?.as_array()?;
    installations
        .iter()
        .find(|a| devices_of(a).iter().any(is_ocpp_wallbox))
        .or_else(|| installations.first())
}

/// Extracts the dashboard part of the summary.
pub fn extract_dashboard(dashboard: &Value) -> DashboardSummary {
    let Some(installation) = pick_installation(dashboard) else {
        return DashboardSummary::default();
    };
    let devices = devices_of(installation);
    let wallbox = devices
        .iter()
        .find(|d| is_ocpp_wallbox(d))
        .or_else(|| devices.iter().find(|d| is_wallbox(d)));
    DashboardSummary {
        installation_id: id_string(installation),
        connection: string_field(installation, "verbindung"),
        last_contact: string_field(installation, "letzter_kontakt"),
        device_status: wallbox.and_then(|d| string_field(d, "status")),
        live_power_w: installation
            .get("live")
            .and_then(|live| live.get("wallbox_leistung_w"))
            .and_then(Value::as_f64),
    }
}

/// Extracts state and headline from the geo-position view. The headline is read from the wallbox
/// entry first and from the top level second, because the backend has used both places.
pub fn extract_geo(geo: &Value) -> GeoSummary {
    let wallbox = geo
        .get("geraete")
        .and_then(Value::as_array)
        .and_then(|list| list.iter().find(|d| is_wallbox(d)));
    GeoSummary {
        state: wallbox.and_then(|d| string_field(d, "zustand")),
        headline: wallbox
            .and_then(|d| string_field(d, "ueberschrift"))
            .or_else(|| string_field(geo, "ueberschrift")),
    }
}

/// Returns up to four wallbox steps that start at or after `now`, earliest first. Steps without a
/// parsable start, an action or a numeric target power are left out instead of being filled with
/// made-up values.
pub fn extract_schedule(schedule: &Value, now: DateTime<Utc>) -> Vec<NextScheduleStep> {
    let Some(steps) = schedule
        .get("fahrplan")
        .and_then(|f| f.get("schritte"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut upcoming: Vec<(DateTime<Utc>, NextScheduleStep)> = steps
        .iter()
        .filter(|s| s.get("geraete_typ").and_then(Value::as_str) == Some("wallbox"))
        .filter_map(|s| {
            let start = parse_time(s.get("beginn")?.as_str()?)?;
            if start < now {
                return None;
            }
            let step = NextScheduleStep {
                start: iso(start),
                action: string_field(s, "aktion")?,
                target_power_w: s.get("soll_leistung_w")?.as_f64()?,
            };
            Some((start, step))
        })
        .collect();
    upcoming.sort_by_key(|(start, _)| *start);
    upcoming
        .into_iter()
        .take(MAX_SCHEDULE_STEPS)
        .map(|(_, step)| step)
        .collect()
}

/// Reads the latest quarter-hour value of the first wallbox. If that value has no numeric
/// `kwhPositiv`, the energy stays `None`; an older value is not substituted.
pub fn extract_energy(energy: &Value) -> EnergySummary {
    let latest = energy
        .get("geraete")
        .and_then(Value::as_array)
        .and_then(|list| list.iter().find(|d| is_wallbox(d)))
        .and_then(|d| d.get("werte"))
        .and_then(Value::as_array)
        .and_then(|values| {
            values
                .iter()
                .filter_map(|v| Some((parse_time(v.get("zeit")?.as_str()?)?, v)))
                .max_by_key(|(at, _)| *at)
        });
    match latest {
        Some((at, value)) => EnergySummary {
            kwh: value.get("kwhPositiv").and_then(Value::as_f64),
            at: Some(iso(at)),
        },
        None => EnergySummary::default(),
    }
}

/// Dates for the energy request: yesterday and today in Berlin local time, as the backend groups
/// quarter hours by Berlin day.
pub fn berlin_date_range(now: DateTime<Utc>) -> (NaiveDate, NaiveDate) {
    let today = now.with_timezone(&Berlin).date_naive();
    let yesterday = today.checked_sub_days(Days::new(1)).unwrap_or(today);
    (yesterday, today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(s: &str) -> DateTime<Utc> {
        parse_time(s).expect("test timestamp")
    }

    fn dashboard() -> Value {
        json!({"anlagen": [
            {"id": "a-1", "bezeichnung": "Haus ohne Wallbox", "verbindung": "offline",
             "letzter_kontakt": null,
             "geraete": [{"id": "g1", "typ": "pv", "status": "ok", "anbindung": "api"}],
             "live": {"wallbox_leistung_w": 999}},
            {"id": "a-2", "bezeichnung": "Haus mit OCPP", "verbindung": "online",
             "letzter_kontakt": "2026-10-06T10:00:00Z",
             "geraete": [
                {"id": "g2", "typ": "speicher", "status": "fehler", "anbindung": "ocpp"},
                {"id": "g3", "typ": "wallbox", "status": "offline", "anbindung": "ocpp"}],
             "live": {"wallbox_leistung_w": 7360}}
        ], "wetter": null})
    }

    #[test]
    fn picks_installation_with_ocpp_wallbox_not_the_first() {
        let d = dashboard();
        assert_eq!(pick_installation(&d).unwrap()["id"], "a-2");
    }

    #[test]
    fn falls_back_to_first_installation_without_ocpp_wallbox() {
        let d = json!({"anlagen": [
            {"id": 7, "verbindung": "online", "letzter_kontakt": null, "geraete": [
                {"typ": "wallbox", "status": "ok", "anbindung": "api"}], "live": null},
            {"id": 8, "geraete": []}]});
        let s = extract_dashboard(&d);
        assert_eq!(s.installation_id.as_deref(), Some("7"));
        assert_eq!(s.device_status.as_deref(), Some("ok"));
    }

    #[test]
    fn dashboard_summary_uses_wallbox_not_other_device_types() {
        let s = extract_dashboard(&dashboard());
        assert_eq!(s.installation_id.as_deref(), Some("a-2"));
        assert_eq!(s.connection.as_deref(), Some("online"));
        assert_eq!(s.last_contact.as_deref(), Some("2026-10-06T10:00:00Z"));
        assert_eq!(s.device_status.as_deref(), Some("offline"));
        assert_eq!(s.live_power_w, Some(7360.0));
    }

    #[test]
    fn non_wallbox_device_is_never_picked_for_status() {
        let d = json!({"anlagen": [{"id": "x", "geraete": [
            {"typ": "speicher", "status": "ok", "anbindung": "ocpp"}]}]});
        assert_eq!(extract_dashboard(&d).device_status, None);
    }

    #[test]
    fn null_or_missing_live_stays_none_not_zero() {
        let null_live = json!({"anlagen": [{"id": "x", "live": null, "geraete": []}]});
        let null_value = json!({"anlagen": [{"id": "x", "live": {"wallbox_leistung_w": null}}]});
        let no_live = json!({"anlagen": [{"id": "x"}]});
        let text = json!({"anlagen": [{"id": "x", "live": {"wallbox_leistung_w": "7360"}}]});
        for d in [null_live, null_value, no_live, text] {
            assert_eq!(extract_dashboard(&d).live_power_w, None);
        }
    }

    #[test]
    fn real_zero_power_is_kept_as_zero() {
        let d = json!({"anlagen": [{"id": "x", "live": {"wallbox_leistung_w": 0}}]});
        assert_eq!(extract_dashboard(&d).live_power_w, Some(0.0));
    }

    #[test]
    fn empty_or_malformed_dashboard_gives_empty_summary() {
        for d in [
            json!({}),
            json!({"anlagen": []}),
            json!({"anlagen": "x"}),
            json!(null),
        ] {
            assert_eq!(extract_dashboard(&d), DashboardSummary::default());
        }
    }

    #[test]
    fn geo_reads_wallbox_entry_and_ignores_other_types() {
        let geo = json!({"geraete": [
            {"typ": "pv", "zustand": "erzeugt", "ueberschrift": "Sonne"},
            {"typ": "wallbox", "zustand": "laedt", "leistungW": 7000, "ueberschrift": "Auto lädt"}]});
        let s = extract_geo(&geo);
        assert_eq!(s.state.as_deref(), Some("laedt"));
        assert_eq!(s.headline.as_deref(), Some("Auto lädt"));
    }

    #[test]
    fn geo_headline_falls_back_to_top_level_but_state_does_not_fall_back_to_other_devices() {
        let geo = json!({"ueberschrift": "Alles ruhig",
            "geraete": [{"typ": "pv", "zustand": "erzeugt"}]});
        let s = extract_geo(&geo);
        assert_eq!(s.state, None);
        assert_eq!(s.headline.as_deref(), Some("Alles ruhig"));
    }

    #[test]
    fn schedule_keeps_future_wallbox_steps_sorted_and_capped_at_four() {
        let now = t("2026-10-06T12:00:00Z");
        let mut steps = vec![
            json!({"beginn": "2026-10-06T11:00:00Z", "geraete_typ": "wallbox", "aktion": "laden", "soll_leistung_w": 1}),
            json!({"beginn": "2026-10-06T13:00:00Z", "geraete_typ": "speicher", "aktion": "laden", "soll_leistung_w": 2}),
        ];
        for h in [18, 16, 14, 15, 17, 12] {
            steps.push(
                json!({"beginn": format!("2026-10-06T{h}:00:00Z"), "geraete_id": "g",
                "geraete_typ": "wallbox", "aktion": "laden", "soll_leistung_w": h * 100}),
            );
        }
        let out = extract_schedule(
            &json!({"fahrplan": {"schritte": steps}, "hinweise": []}),
            now,
        );
        let starts: Vec<&str> = out.iter().map(|s| s.start.as_str()).collect();
        assert_eq!(
            starts,
            [
                "2026-10-06T12:00:00Z",
                "2026-10-06T14:00:00Z",
                "2026-10-06T15:00:00Z",
                "2026-10-06T16:00:00Z"
            ]
        );
        assert_eq!(out[1].target_power_w, 1400.0);
    }

    #[test]
    fn schedule_handles_offsets_and_skips_incomplete_steps() {
        let now = t("2026-10-06T12:00:00Z");
        let s = json!({"fahrplan": {"schritte": [
            {"beginn": "2026-10-06T16:00:00+02:00", "geraete_typ": "wallbox", "aktion": "halten", "soll_leistung_w": 0},
            {"beginn": "2026-10-06T16:00:00Z", "geraete_typ": "wallbox", "aktion": "laden", "soll_leistung_w": null},
            {"beginn": "kaputt", "geraete_typ": "wallbox", "aktion": "laden", "soll_leistung_w": 5},
            {"beginn": "2026-10-06T17:00:00Z", "geraete_typ": "wallbox", "soll_leistung_w": 5}]}});
        let out = extract_schedule(&s, now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start, "2026-10-06T14:00:00Z");
        assert_eq!(out[0].target_power_w, 0.0);
    }

    #[test]
    fn schedule_null_plan_or_garbage_gives_no_steps() {
        let now = t("2026-10-06T12:00:00Z");
        for s in [
            json!({"fahrplan": null, "hinweise": []}),
            json!({}),
            json!({"fahrplan": {"schritte": 3}}),
        ] {
            assert!(extract_schedule(&s, now).is_empty());
        }
    }

    #[test]
    fn energy_takes_latest_value_of_the_wallbox_only() {
        let e = json!({"geraete": [
            {"typ": "pv", "werte": [{"zeit": "2026-10-06T10:30:00Z", "kwhPositiv": 9.9}]},
            {"typ": "wallbox", "werte": [
                {"zeit": "2026-10-06T10:15:00Z", "kwhPositiv": 1.5, "kwhNegativ": 0, "abdeckungPct": 100},
                {"zeit": "2026-10-06T10:00:00Z", "kwhPositiv": 1.0},
                {"zeit": "2026-10-06T10:30:00Z", "kwhPositiv": 1.75}]}]});
        let s = extract_energy(&e);
        assert_eq!(s.kwh, Some(1.75));
        assert_eq!(s.at.as_deref(), Some("2026-10-06T10:30:00Z"));
    }

    #[test]
    fn energy_with_null_latest_value_does_not_use_an_older_one() {
        let e = json!({"geraete": [{"typ": "wallbox", "werte": [
            {"zeit": "2026-10-06T10:00:00Z", "kwhPositiv": 1.0},
            {"zeit": "2026-10-06T10:15:00Z", "kwhPositiv": null}]}]});
        let s = extract_energy(&e);
        assert_eq!(s.kwh, None);
        assert_eq!(s.at.as_deref(), Some("2026-10-06T10:15:00Z"));
    }

    #[test]
    fn energy_without_wallbox_or_values_is_empty() {
        for e in [
            json!({"geraete": [{"typ": "pv", "werte": [{"zeit": "2026-10-06T10:00:00Z", "kwhPositiv": 1}]}]}),
            json!({"geraete": [{"typ": "wallbox", "werte": []}]}),
            json!({}),
        ] {
            assert_eq!(extract_energy(&e), EnergySummary::default());
        }
    }

    #[test]
    fn berlin_dates_follow_local_midnight_in_summer_and_winter() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        // 22:30 UTC is already the next day in Berlin during CEST (+2).
        assert_eq!(
            berlin_date_range(t("2026-10-06T22:30:00Z")),
            (d(2026, 10, 6), d(2026, 10, 7))
        );
        // In CET (+1) the same UTC time is still the same day only until 23:00 UTC.
        assert_eq!(
            berlin_date_range(t("2026-12-06T22:30:00Z")),
            (d(2026, 12, 5), d(2026, 12, 6))
        );
        assert_eq!(
            berlin_date_range(t("2026-12-06T23:30:00Z")),
            (d(2026, 12, 6), d(2026, 12, 7))
        );
        // Month boundary.
        assert_eq!(
            berlin_date_range(t("2026-11-01T10:00:00Z")),
            (d(2026, 10, 31), d(2026, 11, 1))
        );
    }

    #[test]
    fn parse_time_assumes_utc_without_offset_and_rejects_garbage() {
        assert_eq!(t("2026-10-06T10:00:00"), t("2026-10-06T10:00:00Z"));
        assert!(parse_time("gestern").is_none());
    }
}
