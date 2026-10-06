//! Pure state of one simulated charge point: lifecycle, vehicle, meter and profiles. No IO, no clock access.

use chrono::{DateTime, Utc};

use super::profiles::{
    effective_limit, AcceptContext, ChargingProfile, ClearFilter, ProfileRejection, ProfileStore,
};
use super::status::{derive_status, Phase};
use super::vehicle::{charging_power_w, integrate, is_full, Meter};
use crate::error::SimError;
use crate::model::{
    ActiveLimit, ChargePointConfig, ChargePointSnapshot, ConnectionState, OcppStatus,
    VehicleConfig, VehicleSnapshot,
};

/// idTag sent with transactions the simulator starts itself. The backend accepts any tag up to 20 characters.
pub const SIMULATOR_ID_TAG: &str = "SIMULATOR";

/// MeterValueSampleInterval before the central system configures one. 60 s is the value the backend sets
/// right after boot, so a box that never receives the call behaves the same.
pub const DEFAULT_METER_INTERVAL_S: u32 = 60;

/// Measurands the box reports until the central system changes `MeterValuesSampledData`.
pub const DEFAULT_SAMPLED_DATA: &str = "Power.Active.Import,Energy.Active.Import.Register";

/// Smallest interval (heartbeat, MeterValues) the central system can impose on the box. Without a floor a
/// misconfigured or hostile server could make the box send every few milliseconds. 0 stays "off" for MeterValues.
pub const MIN_INTERVAL_S: u32 = 5;

/// Longest simulated step. A task that was starved (suspended laptop) would otherwise integrate minutes of
/// charging in one go and make the meter jump.
pub const MAX_STEP_S: f64 = 10.0;

/// Why a transaction ends; the OCPP `reason` values the simulator uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Vehicle unplugged.
    EVDisconnected,
    /// Stopped by RemoteStopTransaction.
    Remote,
    /// Soft Reset from the central system.
    SoftReset,
    /// Hard Reset from the central system.
    HardReset,
    /// Restart triggered from the UI.
    Reboot,
}

impl StopReason {
    /// OCPP spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EVDisconnected => "EVDisconnected",
            Self::Remote => "Remote",
            Self::SoftReset => "SoftReset",
            Self::HardReset => "HardReset",
            Self::Reboot => "Reboot",
        }
    }
}

/// A running transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    /// Id given by the central system.
    pub id: i64,
    /// Start time (true UTC, not the box clock).
    pub started_at: DateTime<Utc>,
    /// idTag it was started with.
    pub id_tag: String,
}

/// What StopTransaction needs after the transaction was taken out of the state.
#[derive(Debug, Clone, PartialEq)]
pub struct EndedTransaction {
    /// Transaction id.
    pub id: i64,
    /// Meter reading at the end in Wh.
    pub meter_stop_wh: i64,
    /// idTag of the transaction.
    pub id_tag: String,
}

#[derive(Debug, Clone)]
struct VehicleState {
    config: VehicleConfig,
    soc_pct: f64,
}

/// Everything the box knows, minus the network.
#[derive(Debug, Clone)]
pub struct BoxState {
    /// Local id (uuid).
    pub id: String,
    /// Configuration; scenarios may swap it temporarily.
    pub config: ChargePointConfig,
    /// Connection state, maintained by the actor.
    pub connection: ConnectionState,
    /// Heartbeat interval from the BootNotification response.
    pub heartbeat_interval_s: Option<u32>,
    /// Last error worth showing, German.
    pub last_error: Option<String>,
    /// Time of the last published change.
    pub updated_at: DateTime<Utc>,
    /// MeterValueSampleInterval; 0 disables periodic MeterValues.
    pub meter_interval_s: u32,
    /// Floor for intervals set by the central system ([`MIN_INTERVAL_S`] unless a test lowers it).
    pub min_interval_s: u32,
    /// MeterValuesSampledData as a list.
    pub sampled_data: Vec<String>,
    /// idTag for the next transaction start.
    pub next_id_tag: String,
    /// Charging profiles.
    pub store: ProfileStore,
    phase: Phase,
    vehicle: Option<VehicleState>,
    transaction: Option<Transaction>,
    meter: Meter,
    power_w: Option<f64>,
    active_limit: Option<ActiveLimit>,
}

impl BoxState {
    /// A fresh, unplugged box.
    pub fn new(id: String, config: ChargePointConfig) -> Self {
        Self {
            id,
            config,
            connection: ConnectionState::Disconnected,
            heartbeat_interval_s: None,
            last_error: None,
            updated_at: Utc::now(),
            meter_interval_s: DEFAULT_METER_INTERVAL_S,
            min_interval_s: MIN_INTERVAL_S,
            sampled_data: DEFAULT_SAMPLED_DATA
                .split(',')
                .map(str::to_string)
                .collect(),
            next_id_tag: SIMULATOR_ID_TAG.to_string(),
            store: ProfileStore::new(),
            phase: Phase::Idle,
            vehicle: None,
            transaction: None,
            meter: Meter::new(),
            power_w: None,
            active_limit: None,
        }
    }

    /// Lifecycle phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// True while a vehicle is plugged in.
    pub fn is_plugged(&self) -> bool {
        self.vehicle.is_some()
    }

    /// Running transaction.
    pub fn transaction(&self) -> Option<&Transaction> {
        self.transaction.as_ref()
    }

    /// Meter reading in Wh as transported by OCPP.
    pub fn meter_register_wh(&self) -> i64 {
        self.meter.register_wh()
    }

    /// Current power; `None` when nothing is plugged in.
    pub fn power_w(&self) -> Option<f64> {
        self.power_w
    }

    /// State of charge of the plugged-in vehicle.
    pub fn soc_pct(&self) -> Option<f64> {
        self.vehicle.as_ref().map(|v| v.soc_pct)
    }

    /// Limit in force.
    pub fn active_limit(&self) -> Option<&ActiveLimit> {
        self.active_limit.as_ref()
    }

    /// Connector status.
    pub fn status(&self) -> OcppStatus {
        let full = self.vehicle.as_ref().is_some_and(|v| is_full(v.soc_pct));
        derive_status(
            self.phase,
            self.active_limit.as_ref().map(|l| l.limit_w),
            full,
        )
    }

    /// Validates and stores a profile from SetChargingProfile, then re-evaluates the limit.
    pub fn set_profile(
        &mut self,
        connector_id: u32,
        profile: ChargingProfile,
        now: DateTime<Utc>,
    ) -> Result<(), ProfileRejection> {
        let ctx = AcceptContext {
            accepted_units: &self.config.accepted_rate_units,
            reject_profiles: self.config.reject_profiles,
            transaction: self.transaction.as_ref().map(|t| (t.id, t.started_at)),
        };
        self.store.set(connector_id, profile, &ctx, now)?;
        self.refresh(now);
        Ok(())
    }

    /// Removes profiles for ClearChargingProfile and re-evaluates the limit. Returns how many were removed.
    pub fn clear_profiles(&mut self, filter: &ClearFilter, now: DateTime<Utc>) -> usize {
        let removed = self.store.clear(filter);
        self.refresh(now);
        removed
    }

    /// Plugs a vehicle in: the connector becomes `Preparing`.
    pub fn plug_in(&mut self, vehicle: VehicleConfig, now: DateTime<Utc>) -> Result<(), SimError> {
        if self.vehicle.is_some() {
            return Err(SimError::VehicleAlreadyPlugged);
        }
        let soc_pct = vehicle.soc_pct;
        self.vehicle = Some(VehicleState {
            config: vehicle,
            soc_pct,
        });
        self.phase = Phase::Preparing;
        self.refresh(now);
        Ok(())
    }

    /// Unplugs the vehicle. A running transaction ends and is returned so the caller can send StopTransaction;
    /// the phase is `Finishing` until the central system has answered, `Idle` when there was nothing to stop.
    pub fn unplug(&mut self, now: DateTime<Utc>) -> Result<Option<EndedTransaction>, SimError> {
        if self.vehicle.is_none() {
            return Err(SimError::NoVehicle);
        }
        let ended = self.end_transaction();
        self.vehicle = None;
        if ended.is_none() {
            self.phase = Phase::Idle;
        }
        self.refresh(now);
        Ok(ended)
    }

    /// Records the transaction id from the StartTransaction response and begins charging.
    pub fn begin_transaction(&mut self, id: i64, id_tag: &str, now: DateTime<Utc>) {
        self.transaction = Some(Transaction {
            id,
            started_at: now,
            id_tag: id_tag.to_string(),
        });
        self.phase = Phase::InTransaction;
        self.refresh(now);
    }

    /// Ends the running transaction (if any). TxProfiles end with it. The phase becomes `Finishing`.
    pub fn end_transaction(&mut self) -> Option<EndedTransaction> {
        let tx = self.transaction.take()?;
        self.store.drop_tx_profiles();
        self.phase = Phase::Finishing;
        Some(EndedTransaction {
            id: tx.id,
            meter_stop_wh: self.meter.register_wh(),
            id_tag: tx.id_tag,
        })
    }

    /// Called when the central system has answered StopTransaction: without a vehicle the connector is free.
    pub fn finish_if_unplugged(&mut self) {
        if self.phase == Phase::Finishing && self.vehicle.is_none() {
            self.phase = Phase::Idle;
        }
    }

    /// Allows a new transaction after a stop (remote stop, reset) while the vehicle is still plugged in and not
    /// full: a full vehicle would only produce an empty transaction.
    pub fn prepare_new_transaction(&mut self) {
        let full = self.vehicle.as_ref().is_some_and(|v| is_full(v.soc_pct));
        if self.vehicle.is_some() && !full && self.phase == Phase::Finishing {
            self.phase = Phase::Preparing;
        }
    }

    /// True when the box is plugged in, has no transaction and none is being started.
    pub fn wants_transaction(&self) -> bool {
        self.phase == Phase::Preparing && self.transaction.is_none()
    }

    /// Re-evaluates profiles and the current power without advancing time.
    pub fn refresh(&mut self, now: DateTime<Utc>) {
        self.store.drop_expired(now);
        self.active_limit = effective_limit(self.store.profiles(), now, self.config.phases);
        self.power_w = self.vehicle.as_ref().map(|v| {
            if self.phase == Phase::InTransaction {
                let limit = self.active_limit.as_ref().map(|l| l.limit_w);
                charging_power_w(
                    self.config.max_power_w,
                    v.config.max_power_w,
                    limit,
                    v.soc_pct,
                )
            } else {
                0.0
            }
        });
    }

    /// Advances the simulation by `dt_s` seconds ending at `now`.
    pub fn tick(&mut self, now: DateTime<Utc>, dt_s: f64) {
        self.refresh(now);
        let (Some(vehicle), Some(power_w)) = (self.vehicle.as_mut(), self.power_w) else {
            return;
        };
        let step = integrate(
            &vehicle.config,
            vehicle.soc_pct,
            power_w,
            dt_s.min(MAX_STEP_S),
        );
        vehicle.soc_pct = step.soc_pct;
        self.meter.add_wh(step.energy_wh);
        if is_full(vehicle.soc_pct) {
            self.power_w = Some(0.0);
        }
    }

    /// Snapshot for the UI.
    pub fn snapshot(&self) -> ChargePointSnapshot {
        ChargePointSnapshot {
            id: self.id.clone(),
            config: self.config.clone(),
            connection: self.connection.clone(),
            status: self.status(),
            vehicle: self.vehicle.as_ref().map(|v| VehicleSnapshot {
                config: v.config.clone(),
                soc_pct: v.soc_pct,
                plugged: true,
            }),
            power_w: self.power_w,
            energy_wh: self.meter.energy_wh(),
            transaction_id: self.transaction.as_ref().map(|t| t.id),
            active_limit: self.active_limit.clone(),
            profile_count: self.store.len(),
            heartbeat_interval_s: self.heartbeat_interval_s,
            last_error: self.last_error.clone(),
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charge_point::profiles::{
        ChargingProfile, ChargingSchedule, ProfileKind, SchedulePeriod,
    };
    use crate::model::{ProfilePurpose, RateUnit};
    use chrono::{Duration, TimeZone};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
    }

    fn new_box() -> BoxState {
        BoxState::new("id-1".into(), ChargePointConfig::default_local("AP1"))
    }

    fn limit_profile(
        id: i64,
        purpose: ProfilePurpose,
        watts: f64,
        valid_to: Option<DateTime<Utc>>,
    ) -> ChargingProfile {
        ChargingProfile {
            charging_profile_id: id,
            transaction_id: None,
            stack_level: 0,
            charging_profile_purpose: purpose,
            charging_profile_kind: ProfileKind::Absolute,
            recurrency_kind: None,
            valid_from: None,
            valid_to,
            charging_schedule: ChargingSchedule {
                duration: None,
                start_schedule: Some(t0() - Duration::seconds(60)),
                charging_rate_unit: RateUnit::W,
                charging_schedule_period: vec![SchedulePeriod {
                    start_period: 0,
                    limit: watts,
                    number_phases: None,
                }],
                min_charging_rate: None,
            },
        }
    }

    fn charging_box() -> BoxState {
        let mut b = new_box();
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        b.begin_transaction(5, SIMULATOR_ID_TAG, t0());
        b
    }

    #[test]
    fn lifecycle_goes_available_preparing_charging_finishing_available() {
        let mut b = new_box();
        assert_eq!(b.status(), OcppStatus::Available);
        assert_eq!(b.power_w(), None, "nothing plugged in: nothing to measure");
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        assert_eq!(b.status(), OcppStatus::Preparing);
        assert_eq!(b.power_w(), Some(0.0));
        assert!(b.wants_transaction());
        b.begin_transaction(5, SIMULATOR_ID_TAG, t0());
        assert_eq!(b.status(), OcppStatus::Charging);
        assert!(!b.wants_transaction());
        let ended = b.unplug(t0()).unwrap().expect("transaction ended");
        assert_eq!(ended.id, 5);
        assert_eq!(b.status(), OcppStatus::Finishing);
        b.finish_if_unplugged();
        assert_eq!(b.status(), OcppStatus::Available);
        assert_eq!(b.power_w(), None);
    }

    #[test]
    fn unplug_without_transaction_goes_straight_to_available() {
        let mut b = new_box();
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        assert_eq!(b.unplug(t0()).unwrap(), None);
        assert_eq!(b.status(), OcppStatus::Available);
    }

    #[test]
    fn plugging_twice_and_unplugging_empty_are_errors() {
        let mut b = new_box();
        assert_eq!(b.unplug(t0()), Err(SimError::NoVehicle));
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        assert_eq!(
            b.plug_in(VehicleConfig::default(), t0()),
            Err(SimError::VehicleAlreadyPlugged)
        );
    }

    #[test]
    fn remote_stop_keeps_finishing_until_unplug() {
        let mut b = charging_box();
        b.end_transaction().unwrap();
        b.finish_if_unplugged();
        assert_eq!(b.status(), OcppStatus::Finishing, "vehicle still plugged");
        b.prepare_new_transaction();
        assert_eq!(b.status(), OcppStatus::Preparing);
    }

    #[test]
    fn charging_integrates_power_into_meter_and_soc() {
        let mut b = charging_box();
        for i in 1..=10 {
            b.tick(t0() + Duration::seconds(i), 1.0);
        }
        assert_eq!(b.power_w(), Some(11_000.0));
        let expected_wh = 11_000.0 * 10.0 / 3600.0;
        assert!((b.snapshot().energy_wh - expected_wh).abs() < 1e-9);
        assert!(b.soc_pct().unwrap() > 20.0);
    }

    #[test]
    fn meter_does_not_run_without_a_transaction() {
        let mut b = new_box();
        b.plug_in(VehicleConfig::default(), t0()).unwrap();
        b.tick(t0() + Duration::seconds(5), 5.0);
        assert_eq!(b.snapshot().energy_wh, 0.0);
    }

    #[test]
    fn profile_limits_power_and_stops_after_valid_to() {
        let mut b = charging_box();
        let valid_to = t0() + Duration::seconds(30);
        let ctx_profile =
            limit_profile(1, ProfilePurpose::TxDefaultProfile, 4000.0, Some(valid_to));
        b.set_profile(0, ctx_profile, t0()).unwrap();
        b.tick(t0() + Duration::seconds(1), 1.0);
        assert_eq!(b.power_w(), Some(4000.0));
        assert_eq!(b.active_limit().unwrap().limit_w, 4000.0);
        b.tick(t0() + Duration::seconds(29), 1.0);
        assert_eq!(
            b.power_w(),
            Some(4000.0),
            "still limited just before validTo"
        );
        b.tick(t0() + Duration::seconds(30), 1.0);
        assert_eq!(
            b.power_w(),
            Some(11_000.0),
            "box charges at its own maximum again"
        );
        assert!(b.active_limit().is_none());
        assert_eq!(b.store.len(), 0, "expired profile was dropped");
    }

    #[test]
    fn zero_limit_suspends_evse_and_full_vehicle_suspends_ev() {
        let mut b = charging_box();
        b.set_profile(
            0,
            limit_profile(1, ProfilePurpose::TxDefaultProfile, 0.0, None),
            t0(),
        )
        .unwrap();
        b.refresh(t0());
        assert_eq!(b.status(), OcppStatus::SuspendedEVSE);
        assert_eq!(b.power_w(), Some(0.0));

        let mut full = new_box();
        full.plug_in(
            VehicleConfig {
                soc_pct: 100.0,
                ..VehicleConfig::default()
            },
            t0(),
        )
        .unwrap();
        full.begin_transaction(1, SIMULATOR_ID_TAG, t0());
        assert_eq!(full.status(), OcppStatus::SuspendedEV);
        assert_eq!(full.power_w(), Some(0.0));
    }

    #[test]
    fn tx_profiles_end_with_the_transaction() {
        let mut b = charging_box();
        b.set_profile(
            1,
            limit_profile(2, ProfilePurpose::TxProfile, 2000.0, None),
            t0(),
        )
        .unwrap();
        assert_eq!(b.store.len(), 1);
        b.end_transaction();
        assert_eq!(b.store.len(), 0);
    }

    #[test]
    fn long_gaps_are_capped_so_the_meter_does_not_jump() {
        let mut b = charging_box();
        b.tick(t0() + Duration::seconds(600), 600.0);
        let expected = 11_000.0 * MAX_STEP_S / 3600.0;
        assert!((b.snapshot().energy_wh - expected).abs() < 1e-9);
    }

    #[test]
    fn snapshot_reports_missing_measurements_as_none() {
        let b = new_box();
        let s = b.snapshot();
        assert_eq!(s.power_w, None);
        assert_eq!(s.transaction_id, None);
        assert!(s.vehicle.is_none());
        let json = serde_json::to_value(&s).unwrap();
        assert!(json["powerW"].is_null());
        assert!(json["activeLimit"].is_null());
    }
}
