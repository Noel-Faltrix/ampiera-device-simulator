//! Charging profile store and evaluation (OCPP 1.6 section 3.13 "Smart Charging").
//!
//! Everything here is pure: the caller passes `now`, nothing reads the clock.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{ActiveLimit, ProfilePurpose, RateUnit};

/// Grid voltage used to convert amperes to watts. Same value as the backend's `NETZSPANNUNG_V`, so a limit
/// the backend computed in W is reproduced exactly when it sent it in A.
pub const GRID_VOLTAGE_V: f64 = 230.0;

/// Most profiles a box keeps. The central system controls what arrives; without a bound it could fill memory.
pub const MAX_PROFILES: usize = 50;

/// Most periods in one schedule (same reason).
pub const MAX_PERIODS: usize = 100;

const SECONDS_PER_DAY: i64 = 86_400;
const SECONDS_PER_WEEK: i64 = 7 * SECONDS_PER_DAY;

/// OCPP `chargingProfileKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileKind {
    /// Schedule starts at `startSchedule`.
    Absolute,
    /// Schedule repeats daily or weekly from `startSchedule`.
    Recurring,
    /// Schedule starts when the transaction starts.
    Relative,
}

/// OCPP `recurrencyKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecurrencyKind {
    /// Repeats every 24 hours.
    Daily,
    /// Repeats every 7 days.
    Weekly,
}

/// One period of a charging schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulePeriod {
    /// Seconds from the schedule start at which this period begins.
    pub start_period: i64,
    /// Limit in the schedule's unit.
    pub limit: f64,
    /// Number of phases the limit applies to; the box's phases when absent.
    pub number_phases: Option<u8>,
}

/// OCPP `chargingSchedule`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargingSchedule {
    /// Length of the schedule in seconds; unbounded when absent.
    pub duration: Option<i64>,
    /// Start of the schedule.
    pub start_schedule: Option<DateTime<Utc>>,
    /// Unit of all limits.
    pub charging_rate_unit: RateUnit,
    /// Periods, ascending by `start_period`.
    pub charging_schedule_period: Vec<SchedulePeriod>,
    /// Lowest rate the box may use; informational for the simulator.
    pub min_charging_rate: Option<f64>,
}

/// OCPP `csChargingProfiles`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargingProfile {
    /// Id of the profile; setting the same id replaces it.
    pub charging_profile_id: i64,
    /// Transaction a `TxProfile` belongs to.
    pub transaction_id: Option<i64>,
    /// Higher wins within one purpose.
    pub stack_level: u32,
    /// Purpose of the profile.
    pub charging_profile_purpose: ProfilePurpose,
    /// Kind of the profile.
    pub charging_profile_kind: ProfileKind,
    /// Recurrence for `Recurring` profiles.
    pub recurrency_kind: Option<RecurrencyKind>,
    /// Profile is ignored before this time.
    pub valid_from: Option<DateTime<Utc>>,
    /// Profile is ignored from this time on.
    pub valid_to: Option<DateTime<Utc>>,
    /// The schedule.
    pub charging_schedule: ChargingSchedule,
}

/// Why a profile was refused; the German text is shown in the log and last error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileRejection {
    /// Connector id other than 0 or 1.
    UnknownConnector,
    /// ChargePointMaxProfile must target connector 0.
    MaxProfileNeedsConnectorZero,
    /// TxProfile without a running transaction or for another one.
    NoMatchingTransaction,
    /// Unit not in the box's accepted list.
    UnitNotAccepted,
    /// Box is configured to reject all profiles.
    BoxRejectsProfiles,
    /// No periods, first period not at 0, periods not ascending, or negative limit.
    InvalidSchedule,
    /// Absolute/Recurring without `startSchedule`, or Recurring without `recurrencyKind`.
    MissingStart,
    /// The store already holds the maximum number of profiles.
    TooManyProfiles,
    /// The schedule has more periods than allowed.
    TooManyPeriods,
}

impl ProfileRejection {
    /// German explanation.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::UnknownConnector => {
                "Die Wallbox hat nur den Anschluss 1 (und 0 für die ganze Wallbox)."
            }
            Self::MaxProfileNeedsConnectorZero => {
                "Ein ChargePointMaxProfile muss für Anschluss 0 gelten."
            }
            Self::NoMatchingTransaction => {
                "Ein TxProfile braucht eine laufende, passende Transaktion."
            }
            Self::UnitNotAccepted => {
                "Die Einheit des Ladeprofils wird von dieser Wallbox nicht akzeptiert."
            }
            Self::BoxRejectsProfiles => {
                "Die Wallbox ist so eingestellt, dass sie alle Ladeprofile ablehnt."
            }
            Self::InvalidSchedule => {
                "Der Ladeplan ist ungültig (Perioden fehlen, nicht aufsteigend oder negativ)."
            }
            Self::MissingStart => {
                "Dem Ladeprofil fehlt der Startzeitpunkt oder die Wiederholungsart."
            }
            Self::TooManyProfiles => {
                "Die Wallbox speichert höchstens 50 Ladeprofile; weitere werden abgelehnt."
            }
            Self::TooManyPeriods => {
                "Ein Ladeplan darf höchstens 100 Perioden haben; dieser hat mehr."
            }
        }
    }
}

/// What the box knows about itself when a profile arrives.
#[derive(Debug, Clone)]
pub struct AcceptContext<'a> {
    /// Units the box accepts.
    pub accepted_units: &'a [RateUnit],
    /// Reject everything.
    pub reject_profiles: bool,
    /// Running transaction id and its start time.
    pub transaction: Option<(i64, DateTime<Utc>)>,
}

/// A profile together with the connector it was set for.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredProfile {
    /// 0 = whole box, 1 = the connector.
    pub connector_id: u32,
    /// The profile.
    pub profile: ChargingProfile,
}

/// Filter of ClearChargingProfile; every given field must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearFilter {
    /// Profile id.
    pub id: Option<i64>,
    /// Connector id.
    pub connector_id: Option<u32>,
    /// Purpose.
    pub charging_profile_purpose: Option<ProfilePurpose>,
    /// Stack level.
    pub stack_level: Option<u32>,
}

/// Validates a profile against the box's capabilities (pure rule, see tests).
pub fn validate_profile(
    connector_id: u32,
    profile: &ChargingProfile,
    ctx: &AcceptContext<'_>,
) -> Result<(), ProfileRejection> {
    if ctx.reject_profiles {
        return Err(ProfileRejection::BoxRejectsProfiles);
    }
    if connector_id > 1 {
        return Err(ProfileRejection::UnknownConnector);
    }
    let purpose = profile.charging_profile_purpose;
    if purpose == ProfilePurpose::ChargePointMaxProfile && connector_id != 0 {
        return Err(ProfileRejection::MaxProfileNeedsConnectorZero);
    }
    if purpose == ProfilePurpose::TxProfile && !tx_profile_matches(connector_id, profile, ctx) {
        return Err(ProfileRejection::NoMatchingTransaction);
    }
    if !ctx
        .accepted_units
        .contains(&profile.charging_schedule.charging_rate_unit)
    {
        return Err(ProfileRejection::UnitNotAccepted);
    }
    validate_schedule(profile)
}

fn tx_profile_matches(
    connector_id: u32,
    profile: &ChargingProfile,
    ctx: &AcceptContext<'_>,
) -> bool {
    let Some((running, _)) = ctx.transaction else {
        return false;
    };
    connector_id == 1 && profile.transaction_id.is_none_or(|id| id == running)
}

fn validate_schedule(profile: &ChargingProfile) -> Result<(), ProfileRejection> {
    let periods = &profile.charging_schedule.charging_schedule_period;
    if periods.len() > MAX_PERIODS {
        return Err(ProfileRejection::TooManyPeriods);
    }
    let ascending = periods
        .windows(2)
        .all(|w| w[0].start_period < w[1].start_period);
    let first_at_zero = periods.first().is_some_and(|p| p.start_period == 0);
    let limits_ok = periods
        .iter()
        .all(|p| p.limit.is_finite() && p.limit >= 0.0);
    if !(ascending && first_at_zero && limits_ok) {
        return Err(ProfileRejection::InvalidSchedule);
    }
    let needs_start = match profile.charging_profile_kind {
        ProfileKind::Absolute => profile.charging_schedule.start_schedule.is_none(),
        ProfileKind::Recurring => {
            profile.charging_schedule.start_schedule.is_none() || profile.recurrency_kind.is_none()
        }
        ProfileKind::Relative => false,
    };
    if needs_start {
        return Err(ProfileRejection::MissingStart);
    }
    Ok(())
}

/// All profiles of one box.
#[derive(Debug, Clone, Default)]
pub struct ProfileStore {
    entries: Vec<StoredProfile>,
}

impl ProfileStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored profiles.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no profile is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over the profiles without connector information.
    pub fn profiles(&self) -> impl Iterator<Item = &ChargingProfile> {
        self.entries.iter().map(|e| &e.profile)
    }

    /// Validates and stores a profile. A profile with the same id, or with the same purpose, stack level and
    /// connector, is replaced (OCPP 1.6 3.13.2). A `Relative` profile gets its start fixed to the transaction
    /// start (or `now`), so evaluation needs no transaction context.
    pub fn set(
        &mut self,
        connector_id: u32,
        mut profile: ChargingProfile,
        ctx: &AcceptContext<'_>,
        now: DateTime<Utc>,
    ) -> Result<(), ProfileRejection> {
        validate_profile(connector_id, &profile, ctx)?;
        if profile.charging_profile_kind == ProfileKind::Relative
            && profile.charging_schedule.start_schedule.is_none()
        {
            profile.charging_schedule.start_schedule =
                Some(ctx.transaction.map_or(now, |(_, started)| started));
        }
        let replaced_by_new = |e: &StoredProfile| {
            let same_id = e.profile.charging_profile_id == profile.charging_profile_id;
            let same_slot = e.connector_id == connector_id
                && e.profile.charging_profile_purpose == profile.charging_profile_purpose
                && e.profile.stack_level == profile.stack_level;
            same_id || same_slot
        };
        let remaining = self.entries.iter().filter(|e| !replaced_by_new(e)).count();
        if remaining >= MAX_PROFILES {
            return Err(ProfileRejection::TooManyProfiles);
        }
        self.entries.retain(|e| !replaced_by_new(e));
        self.entries.push(StoredProfile {
            connector_id,
            profile,
        });
        Ok(())
    }

    /// Removes the profiles matching every given field of the filter (all profiles for an empty filter).
    /// Returns how many were removed; 0 means ClearChargingProfile answers `Unknown`.
    pub fn clear(&mut self, filter: &ClearFilter) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !matches_filter(e, filter));
        before - self.entries.len()
    }

    /// Drops profiles whose `validTo` has passed; an expired profile must never apply again.
    pub fn drop_expired(&mut self, now: DateTime<Utc>) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|e| e.profile.valid_to.is_none_or(|valid_to| now < valid_to));
        before - self.entries.len()
    }

    /// Removes every `TxProfile`; they end with their transaction (OCPP 1.6 3.13.1).
    pub fn drop_tx_profiles(&mut self) {
        self.entries
            .retain(|e| e.profile.charging_profile_purpose != ProfilePurpose::TxProfile);
    }
}

fn matches_filter(entry: &StoredProfile, filter: &ClearFilter) -> bool {
    filter
        .id
        .is_none_or(|id| id == entry.profile.charging_profile_id)
        && filter.connector_id.is_none_or(|c| c == entry.connector_id)
        && filter
            .charging_profile_purpose
            .is_none_or(|p| p == entry.profile.charging_profile_purpose)
        && filter
            .stack_level
            .is_none_or(|s| s == entry.profile.stack_level)
}

/// Seconds since the schedule start that apply at `now`, or `None` when the schedule is not running
/// (not started yet, or past its `duration`).
fn schedule_elapsed_s(profile: &ChargingProfile, now: DateTime<Utc>) -> Option<i64> {
    let start = profile.charging_schedule.start_schedule?;
    if now < start {
        return None;
    }
    let raw = (now - start).num_seconds();
    let elapsed = match (profile.charging_profile_kind, profile.recurrency_kind) {
        (ProfileKind::Recurring, Some(RecurrencyKind::Daily)) => raw.rem_euclid(SECONDS_PER_DAY),
        (ProfileKind::Recurring, Some(RecurrencyKind::Weekly)) => raw.rem_euclid(SECONDS_PER_WEEK),
        (ProfileKind::Recurring, None) => return None,
        _ => raw,
    };
    match profile.charging_schedule.duration {
        Some(duration) if elapsed >= duration => None,
        _ => Some(elapsed),
    }
}

/// True when `now` lies inside `[validFrom, validTo)`. The end is exclusive: at exactly `validTo` the box must
/// charge as without the profile.
fn is_valid_at(profile: &ChargingProfile, now: DateTime<Utc>) -> bool {
    profile.valid_from.is_none_or(|from| now >= from) && profile.valid_to.is_none_or(|to| now < to)
}

/// A profile that applies right now with its limit.
#[derive(Debug, Clone)]
struct Candidate<'a> {
    profile: &'a ChargingProfile,
    limit_w: f64,
    raw_limit: f64,
}

fn candidate_at<'a>(
    profile: &'a ChargingProfile,
    now: DateTime<Utc>,
    box_phases: u8,
) -> Option<Candidate<'a>> {
    if !is_valid_at(profile, now) {
        return None;
    }
    let elapsed = schedule_elapsed_s(profile, now)?;
    let period = profile
        .charging_schedule
        .charging_schedule_period
        .iter()
        .rev()
        .find(|p| p.start_period <= elapsed)?;
    let limit_w = match profile.charging_schedule.charging_rate_unit {
        RateUnit::W => period.limit,
        RateUnit::A => {
            period.limit * GRID_VOLTAGE_V * f64::from(period.number_phases.unwrap_or(box_phases))
        }
    };
    Some(Candidate {
        profile,
        limit_w,
        raw_limit: period.limit,
    })
}

/// Highest stack level among the profiles of one purpose that apply now.
fn best_of_purpose<'a>(
    profiles: &[&'a ChargingProfile],
    purpose: ProfilePurpose,
    now: DateTime<Utc>,
    box_phases: u8,
) -> Option<Candidate<'a>> {
    profiles
        .iter()
        .filter(|p| p.charging_profile_purpose == purpose)
        .filter_map(|p| candidate_at(p, now, box_phases))
        .max_by_key(|c| c.profile.stack_level)
}

/// Computes the limit in force.
///
/// Precedence: a running `TxProfile` replaces `TxDefaultProfile`; `ChargePointMaxProfile` caps the result.
/// Within one purpose the highest stack level that currently applies wins. Profiles outside their validity
/// window or schedule are ignored. Returns `None` when nothing applies, i.e. the box charges at its own maximum.
pub fn effective_limit<'a>(
    profiles: impl IntoIterator<Item = &'a ChargingProfile>,
    now: DateTime<Utc>,
    box_phases: u8,
) -> Option<ActiveLimit> {
    let all: Vec<&ChargingProfile> = profiles.into_iter().collect();
    let tx_level = best_of_purpose(&all, ProfilePurpose::TxProfile, now, box_phases)
        .or_else(|| best_of_purpose(&all, ProfilePurpose::TxDefaultProfile, now, box_phases));
    let cap = best_of_purpose(&all, ProfilePurpose::ChargePointMaxProfile, now, box_phases);
    let winner = match (tx_level, cap) {
        (Some(tx), Some(cap)) => {
            if cap.limit_w < tx.limit_w {
                cap
            } else {
                tx
            }
        }
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => return None,
    };
    Some(ActiveLimit {
        limit_w: winner.limit_w,
        raw_limit: winner.raw_limit,
        rate_unit: winner.profile.charging_schedule.charging_rate_unit,
        profile_id: winner.profile.charging_profile_id,
        purpose: winner.profile.charging_profile_purpose,
        valid_to: winner.profile.valid_to,
    })
}

#[cfg(test)]
mod tests;
