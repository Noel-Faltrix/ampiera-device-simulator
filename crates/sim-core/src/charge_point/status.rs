//! OCPP connector status derived from what the box is doing.

use crate::model::OcppStatus;

/// Where the connector is in the plug-in / transaction lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Nothing plugged in.
    Idle,
    /// Vehicle plugged in, transaction not running (yet).
    Preparing,
    /// Transaction running.
    InTransaction,
    /// Transaction ended; the vehicle may still be plugged in.
    Finishing,
}

/// Derives the status. Inside a transaction a full vehicle wins over a zero limit: the box could offer power
/// (limit permitting) only if the vehicle wanted it, and `SuspendedEV` tells the operator the vehicle is the reason.
pub fn derive_status(phase: Phase, limit_w: Option<f64>, vehicle_full: bool) -> OcppStatus {
    match phase {
        Phase::Idle => OcppStatus::Available,
        Phase::Preparing => OcppStatus::Preparing,
        Phase::Finishing => OcppStatus::Finishing,
        Phase::InTransaction => {
            if vehicle_full {
                OcppStatus::SuspendedEV
            } else if limit_w.is_some_and(|limit| limit <= 0.0) {
                OcppStatus::SuspendedEVSE
            } else {
                OcppStatus::Charging
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_phases_map_to_their_statuses() {
        assert_eq!(
            derive_status(Phase::Idle, None, false),
            OcppStatus::Available
        );
        assert_eq!(
            derive_status(Phase::Preparing, None, false),
            OcppStatus::Preparing
        );
        assert_eq!(
            derive_status(Phase::Finishing, None, false),
            OcppStatus::Finishing
        );
        assert_eq!(
            derive_status(Phase::InTransaction, None, false),
            OcppStatus::Charging
        );
    }

    #[test]
    fn zero_limit_suspends_evse_but_a_positive_limit_does_not() {
        assert_eq!(
            derive_status(Phase::InTransaction, Some(0.0), false),
            OcppStatus::SuspendedEVSE
        );
        assert_eq!(
            derive_status(Phase::InTransaction, Some(1.0), false),
            OcppStatus::Charging
        );
    }

    #[test]
    fn full_vehicle_suspends_ev_and_wins_over_zero_limit() {
        assert_eq!(
            derive_status(Phase::InTransaction, None, true),
            OcppStatus::SuspendedEV
        );
        assert_eq!(
            derive_status(Phase::InTransaction, Some(0.0), true),
            OcppStatus::SuspendedEV
        );
    }

    #[test]
    fn limit_and_full_flag_do_not_change_other_phases() {
        assert_eq!(
            derive_status(Phase::Preparing, Some(0.0), true),
            OcppStatus::Preparing
        );
        assert_eq!(
            derive_status(Phase::Idle, Some(0.0), true),
            OcppStatus::Available
        );
    }
}
