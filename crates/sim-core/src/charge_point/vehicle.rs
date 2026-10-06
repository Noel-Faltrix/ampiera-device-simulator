//! Vehicle charging model and meter integration.
//!
//! Simplifications, all deliberate: the battery takes exactly the energy the meter counts (no charging losses),
//! and the vehicle follows the taper curve below. The scenarios compare box and backend values, so a
//! simple, predictable model is worth more than a realistic one.

use crate::model::VehicleConfig;

/// Above this state of charge the vehicle reduces its power. 80 % is the usual knee of lithium-ion charge curves.
pub const TAPER_START_SOC_PCT: f64 = 80.0;

/// Fraction of the power still drawn at 100 % SoC. The linear ramp from 80 % to 100 % ends at a quarter of the
/// power, which approximates the constant-voltage phase without modelling cell chemistry.
pub const TAPER_END_POWER_FACTOR: f64 = 0.25;

/// From this state of charge on the vehicle is full and stops drawing power (status `SuspendedEV`).
pub const FULL_SOC_PCT: f64 = 100.0;

const WH_PER_KWH: f64 = 1000.0;
const SECONDS_PER_HOUR: f64 = 3600.0;

/// Fraction of the allowed power the vehicle actually draws at this state of charge: 1.0 up to 80 %, then
/// linear down to [`TAPER_END_POWER_FACTOR`] at 100 %.
pub fn taper_factor(soc_pct: f64) -> f64 {
    if soc_pct <= TAPER_START_SOC_PCT {
        return 1.0;
    }
    let progress =
        ((soc_pct - TAPER_START_SOC_PCT) / (FULL_SOC_PCT - TAPER_START_SOC_PCT)).min(1.0);
    1.0 - (1.0 - TAPER_END_POWER_FACTOR) * progress
}

/// True when the vehicle will not draw any more energy.
pub fn is_full(soc_pct: f64) -> bool {
    soc_pct >= FULL_SOC_PCT
}

/// Charging power in W: the smallest of box maximum, vehicle maximum and limit, times the taper factor.
/// Zero when the vehicle is full or the limit is zero.
pub fn charging_power_w(
    box_max_w: f64,
    vehicle_max_w: f64,
    limit_w: Option<f64>,
    soc_pct: f64,
) -> f64 {
    if is_full(soc_pct) {
        return 0.0;
    }
    let hardware = box_max_w.min(vehicle_max_w);
    let allowed = limit_w.map_or(hardware, |limit| hardware.min(limit));
    allowed.max(0.0) * taper_factor(soc_pct)
}

/// Result of integrating one time step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Integration {
    /// New state of charge, at most 100.
    pub soc_pct: f64,
    /// Energy that actually flowed in Wh (less than power times time when the battery fills up).
    pub energy_wh: f64,
}

/// Integrates `power_w` over `dt_s` seconds into the battery. The energy is capped at what is needed to fill
/// the battery, so the meter never counts more than the vehicle took.
pub fn integrate(vehicle: &VehicleConfig, soc_pct: f64, power_w: f64, dt_s: f64) -> Integration {
    let capacity_wh = vehicle.capacity_kwh * WH_PER_KWH;
    let wanted_wh = power_w.max(0.0) * dt_s.max(0.0) / SECONDS_PER_HOUR;
    let room_wh = (FULL_SOC_PCT - soc_pct).max(0.0) / 100.0 * capacity_wh;
    let energy_wh = wanted_wh.min(room_wh);
    let new_soc = (soc_pct + energy_wh / capacity_wh * 100.0).min(FULL_SOC_PCT);
    Integration {
        soc_pct: new_soc,
        energy_wh,
    }
}

/// Active energy import register of the box. Monotonic: it only ever grows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meter {
    energy_wh: f64,
}

impl Meter {
    /// A meter at 0 Wh.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds energy; negative or non-finite values are ignored because a register cannot run backwards.
    pub fn add_wh(&mut self, wh: f64) {
        if wh.is_finite() && wh > 0.0 {
            self.energy_wh += wh;
        }
    }

    /// Current reading in Wh.
    pub fn energy_wh(&self) -> f64 {
        self.energy_wh
    }

    /// Reading as the integer Wh that OCPP transports.
    pub fn register_wh(&self) -> i64 {
        self.energy_wh.round() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car() -> VehicleConfig {
        VehicleConfig {
            capacity_kwh: 50.0,
            soc_pct: 20.0,
            max_power_w: 11_000.0,
            phases: 3,
        }
    }

    #[test]
    fn taper_is_flat_until_80_then_linear_to_a_quarter() {
        assert_eq!(taper_factor(0.0), 1.0);
        assert_eq!(taper_factor(80.0), 1.0);
        assert!((taper_factor(90.0) - 0.625).abs() < 1e-12);
        assert!((taper_factor(100.0) - 0.25).abs() < 1e-12);
        assert!(
            taper_factor(80.1) < 1.0,
            "counter-check: it must drop right after 80 %"
        );
    }

    #[test]
    fn power_is_the_minimum_of_all_caps() {
        assert_eq!(
            charging_power_w(11_000.0, 7_400.0, None, 50.0),
            7_400.0,
            "vehicle is the bottleneck"
        );
        assert_eq!(
            charging_power_w(3_700.0, 7_400.0, None, 50.0),
            3_700.0,
            "box is the bottleneck"
        );
        assert_eq!(
            charging_power_w(11_000.0, 11_000.0, Some(4_000.0), 50.0),
            4_000.0,
            "limit is the bottleneck"
        );
        assert_eq!(
            charging_power_w(11_000.0, 11_000.0, Some(20_000.0), 50.0),
            11_000.0,
            "limit never raises"
        );
    }

    #[test]
    fn zero_limit_and_full_battery_stop_charging() {
        assert_eq!(charging_power_w(11_000.0, 11_000.0, Some(0.0), 50.0), 0.0);
        assert_eq!(charging_power_w(11_000.0, 11_000.0, None, 100.0), 0.0);
        assert!(charging_power_w(11_000.0, 11_000.0, None, 99.0) > 0.0);
    }

    #[test]
    fn taper_reduces_the_limited_power_too() {
        let p = charging_power_w(11_000.0, 11_000.0, Some(4_000.0), 90.0);
        assert!((p - 2_500.0).abs() < 1e-9);
    }

    #[test]
    fn negative_limit_never_gives_negative_power() {
        assert_eq!(charging_power_w(11_000.0, 11_000.0, Some(-5.0), 50.0), 0.0);
    }

    #[test]
    fn integration_adds_energy_and_soc() {
        let r = integrate(&car(), 20.0, 10_000.0, 360.0);
        assert!((r.energy_wh - 1_000.0).abs() < 1e-9);
        assert!(
            (r.soc_pct - 22.0).abs() < 1e-9,
            "1 kWh of 50 kWh is 2 points"
        );
    }

    #[test]
    fn integration_stops_at_full_and_caps_energy() {
        let r = integrate(&car(), 99.9, 11_000.0, 3600.0);
        assert_eq!(r.soc_pct, 100.0);
        assert!(
            (r.energy_wh - 50.0).abs() < 1e-6,
            "only 0.1 % of 50 kWh fits"
        );
        let full = integrate(&car(), 100.0, 11_000.0, 10.0);
        assert_eq!(full.energy_wh, 0.0);
    }

    #[test]
    fn meter_is_monotonic() {
        let mut m = Meter::new();
        m.add_wh(10.4);
        m.add_wh(-5.0);
        m.add_wh(f64::NAN);
        m.add_wh(0.2);
        assert!((m.energy_wh() - 10.6).abs() < 1e-12);
        assert_eq!(m.register_wh(), 11);
    }
}
