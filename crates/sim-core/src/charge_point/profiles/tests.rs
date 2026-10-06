use super::*;
use chrono::{Duration, TimeZone};

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap()
}

fn secs(s: i64) -> Duration {
    Duration::seconds(s)
}

fn profile(
    id: i64,
    purpose: ProfilePurpose,
    stack: u32,
    unit: RateUnit,
    limit: f64,
) -> ChargingProfile {
    ChargingProfile {
        charging_profile_id: id,
        transaction_id: None,
        stack_level: stack,
        charging_profile_purpose: purpose,
        charging_profile_kind: ProfileKind::Absolute,
        recurrency_kind: None,
        valid_from: None,
        valid_to: None,
        charging_schedule: ChargingSchedule {
            duration: None,
            start_schedule: Some(t0() - secs(60)),
            charging_rate_unit: unit,
            charging_schedule_period: vec![SchedulePeriod {
                start_period: 0,
                limit,
                number_phases: None,
            }],
            min_charging_rate: None,
        },
    }
}

fn limit_w(profiles: &[ChargingProfile], now: DateTime<Utc>) -> Option<f64> {
    effective_limit(profiles.iter(), now, 3).map(|l| l.limit_w)
}

fn ctx(units: &[RateUnit], tx: Option<(i64, DateTime<Utc>)>) -> AcceptContext<'_> {
    AcceptContext {
        accepted_units: units,
        reject_profiles: false,
        transaction: tx,
    }
}

#[test]
fn no_profiles_means_no_limit() {
    assert_eq!(limit_w(&[], t0()), None);
}

#[test]
fn limit_applies_before_valid_to_and_not_after() {
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 4000.0);
    p.valid_to = Some(t0() + secs(100));
    let list = [p];
    assert_eq!(limit_w(&list, t0()), Some(4000.0));
    assert_eq!(limit_w(&list, t0() + secs(99)), Some(4000.0));
    assert_eq!(limit_w(&list, t0() + secs(100)), None, "exclusive end");
    assert_eq!(limit_w(&list, t0() + secs(101)), None);
}

#[test]
fn valid_from_delays_the_limit() {
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 4000.0);
    p.valid_from = Some(t0() + secs(30));
    let list = [p];
    assert_eq!(limit_w(&list, t0()), None);
    assert_eq!(limit_w(&list, t0() + secs(30)), Some(4000.0));
}

#[test]
fn amperes_convert_with_voltage_and_period_phases() {
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::A, 10.0);
    let list = [p.clone()];
    assert_eq!(
        limit_w(&list, t0()),
        Some(10.0 * 230.0 * 3.0),
        "box phases when absent"
    );
    p.charging_schedule.charging_schedule_period[0].number_phases = Some(1);
    assert_eq!(limit_w(&[p], t0()), Some(2300.0), "period phases win");
    let single = profile(2, ProfilePurpose::TxDefaultProfile, 0, RateUnit::A, 10.0);
    let l = effective_limit([&single], t0(), 1).unwrap();
    assert_eq!(l.limit_w, 2300.0);
    assert_eq!(l.raw_limit, 10.0);
    assert_eq!(l.rate_unit, RateUnit::A);
}

#[test]
fn watt_limits_are_taken_as_is() {
    let p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 4321.0);
    assert_eq!(limit_w(&[p], t0()), Some(4321.0));
}

#[test]
fn higher_stack_level_wins_within_a_purpose() {
    let low = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 9000.0);
    let high = profile(2, ProfilePurpose::TxDefaultProfile, 5, RateUnit::W, 3000.0);
    assert_eq!(limit_w(&[low.clone(), high.clone()], t0()), Some(3000.0));
    assert_eq!(
        limit_w(&[high, low], t0()),
        Some(3000.0),
        "order must not matter"
    );
}

#[test]
fn expired_high_stack_level_falls_back_to_lower() {
    let low = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 9000.0);
    let mut high = profile(2, ProfilePurpose::TxDefaultProfile, 5, RateUnit::W, 3000.0);
    high.valid_to = Some(t0() + secs(10));
    let list = [low, high];
    assert_eq!(limit_w(&list, t0()), Some(3000.0));
    assert_eq!(limit_w(&list, t0() + secs(11)), Some(9000.0));
}

#[test]
fn tx_profile_overrides_tx_default_even_when_higher() {
    let default = profile(1, ProfilePurpose::TxDefaultProfile, 9, RateUnit::W, 2000.0);
    let tx = profile(2, ProfilePurpose::TxProfile, 0, RateUnit::W, 8000.0);
    let l = effective_limit([&default, &tx], t0(), 3).unwrap();
    assert_eq!(l.limit_w, 8000.0);
    assert_eq!(l.purpose, ProfilePurpose::TxProfile);
    let only_default = effective_limit([&default], t0(), 3).unwrap();
    assert_eq!(
        only_default.limit_w, 2000.0,
        "counter-check: without TxProfile the default applies"
    );
}

#[test]
fn charge_point_max_caps_but_never_raises() {
    let max = profile(
        1,
        ProfilePurpose::ChargePointMaxProfile,
        0,
        RateUnit::W,
        5000.0,
    );
    let low = profile(2, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 3000.0);
    let high = profile(3, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 9000.0);
    assert_eq!(
        limit_w(&[max.clone(), low], t0()),
        Some(3000.0),
        "lower tx limit stays"
    );
    let capped = effective_limit([&max, &high], t0(), 3).unwrap();
    assert_eq!(capped.limit_w, 5000.0);
    assert_eq!(capped.purpose, ProfilePurpose::ChargePointMaxProfile);
    assert_eq!(limit_w(&[max], t0()), Some(5000.0), "cap alone is a limit");
}

#[test]
fn schedule_not_started_yet_is_ignored() {
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1000.0);
    p.charging_schedule.start_schedule = Some(t0() + secs(60));
    let list = [p];
    assert_eq!(limit_w(&list, t0()), None);
    assert_eq!(limit_w(&list, t0() + secs(60)), Some(1000.0));
}

#[test]
fn periods_switch_at_their_offsets() {
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1000.0);
    p.charging_schedule.start_schedule = Some(t0());
    p.charging_schedule
        .charging_schedule_period
        .push(SchedulePeriod {
            start_period: 600,
            limit: 6000.0,
            number_phases: None,
        });
    let list = [p];
    assert_eq!(limit_w(&list, t0() + secs(599)), Some(1000.0));
    assert_eq!(limit_w(&list, t0() + secs(600)), Some(6000.0));
    assert_eq!(
        limit_w(&list, t0() + secs(5000)),
        Some(6000.0),
        "last period continues"
    );
}

#[test]
fn duration_ends_the_schedule() {
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1000.0);
    p.charging_schedule.start_schedule = Some(t0());
    p.charging_schedule.duration = Some(300);
    let list = [p];
    assert_eq!(limit_w(&list, t0() + secs(299)), Some(1000.0));
    assert_eq!(limit_w(&list, t0() + secs(300)), None);
}

#[test]
fn zero_limit_is_a_limit_not_absence() {
    let p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::A, 0.0);
    assert_eq!(limit_w(&[p], t0()), Some(0.0));
}

#[test]
fn daily_recurrence_repeats_the_schedule() {
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1000.0);
    p.charging_profile_kind = ProfileKind::Recurring;
    p.recurrency_kind = Some(RecurrencyKind::Daily);
    p.charging_schedule.start_schedule = Some(t0());
    p.charging_schedule.duration = Some(3600);
    let list = [p];
    assert_eq!(limit_w(&list, t0() + secs(10)), Some(1000.0));
    assert_eq!(limit_w(&list, t0() + secs(7200)), None);
    assert_eq!(
        limit_w(&list, t0() + secs(SECONDS_PER_DAY + 10)),
        Some(1000.0),
        "next day again"
    );
}

#[test]
fn drop_expired_removes_only_expired_profiles() {
    let mut store = ProfileStore::new();
    let units = [RateUnit::W];
    let mut a = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1000.0);
    a.valid_to = Some(t0() + secs(10));
    let b = profile(2, ProfilePurpose::TxDefaultProfile, 1, RateUnit::W, 2000.0);
    store.set(0, a, &ctx(&units, None), t0()).unwrap();
    store.set(0, b, &ctx(&units, None), t0()).unwrap();
    assert_eq!(store.drop_expired(t0() + secs(5)), 0);
    assert_eq!(store.drop_expired(t0() + secs(10)), 1);
    assert_eq!(store.len(), 1);
}

#[test]
fn set_replaces_same_id_and_same_slot_but_not_other_slots() {
    let mut store = ProfileStore::new();
    let units = [RateUnit::W, RateUnit::A];
    let c = ctx(&units, None);
    store
        .set(
            0,
            profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1000.0),
            &c,
            t0(),
        )
        .unwrap();
    store
        .set(
            0,
            profile(1, ProfilePurpose::TxDefaultProfile, 3, RateUnit::W, 2000.0),
            &c,
            t0(),
        )
        .unwrap();
    assert_eq!(store.len(), 1, "same id replaced");
    store
        .set(
            0,
            profile(2, ProfilePurpose::TxDefaultProfile, 3, RateUnit::W, 3000.0),
            &c,
            t0(),
        )
        .unwrap();
    assert_eq!(store.len(), 1, "same purpose and stack level replaced");
    store
        .set(
            0,
            profile(3, ProfilePurpose::TxDefaultProfile, 4, RateUnit::W, 4000.0),
            &c,
            t0(),
        )
        .unwrap();
    assert_eq!(store.len(), 2, "different stack level kept");
}

#[test]
fn clear_by_id_purpose_stack_level_and_everything() {
    let units = [RateUnit::W];
    let fill = || {
        let mut store = ProfileStore::new();
        let c = ctx(&units, Some((7, t0())));
        store
            .set(
                0,
                profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1.0),
                &c,
                t0(),
            )
            .unwrap();
        store
            .set(
                0,
                profile(
                    2,
                    ProfilePurpose::ChargePointMaxProfile,
                    1,
                    RateUnit::W,
                    1.0,
                ),
                &c,
                t0(),
            )
            .unwrap();
        store
            .set(
                1,
                profile(3, ProfilePurpose::TxProfile, 2, RateUnit::W, 1.0),
                &c,
                t0(),
            )
            .unwrap();
        store
    };
    let mut s = fill();
    assert_eq!(
        s.clear(&ClearFilter {
            id: Some(2),
            ..Default::default()
        }),
        1
    );
    assert_eq!(s.len(), 2);
    let mut s = fill();
    let by_purpose = ClearFilter {
        charging_profile_purpose: Some(ProfilePurpose::TxProfile),
        ..Default::default()
    };
    assert_eq!(s.clear(&by_purpose), 1);
    let mut s = fill();
    assert_eq!(
        s.clear(&ClearFilter {
            stack_level: Some(1),
            ..Default::default()
        }),
        1
    );
    let mut s = fill();
    let combined = ClearFilter {
        id: Some(1),
        stack_level: Some(9),
        ..Default::default()
    };
    assert_eq!(s.clear(&combined), 0, "all given fields must match");
    assert_eq!(
        s.clear(&ClearFilter::default()),
        3,
        "empty filter clears all"
    );
    assert_eq!(
        s.clear(&ClearFilter::default()),
        0,
        "nothing left means Unknown"
    );
}

#[test]
fn drop_tx_profiles_keeps_the_others() {
    let units = [RateUnit::W];
    let mut store = ProfileStore::new();
    let c = ctx(&units, Some((7, t0())));
    store
        .set(
            0,
            profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1.0),
            &c,
            t0(),
        )
        .unwrap();
    store
        .set(
            1,
            profile(3, ProfilePurpose::TxProfile, 2, RateUnit::W, 1.0),
            &c,
            t0(),
        )
        .unwrap();
    store.drop_tx_profiles();
    assert_eq!(store.len(), 1);
    assert_eq!(store.profiles().next().unwrap().charging_profile_id, 1);
}

#[test]
fn validation_rules_each_have_a_counter_case() {
    let both = [RateUnit::W, RateUnit::A];
    let only_w = [RateUnit::W];
    let tx = Some((7, t0()));
    let default = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::A, 10.0);
    assert!(validate_profile(0, &default, &ctx(&both, None)).is_ok());
    assert_eq!(
        validate_profile(0, &default, &ctx(&only_w, None)),
        Err(ProfileRejection::UnitNotAccepted)
    );
    let reject_all = AcceptContext {
        accepted_units: &both,
        reject_profiles: true,
        transaction: None,
    };
    assert_eq!(
        validate_profile(0, &default, &reject_all),
        Err(ProfileRejection::BoxRejectsProfiles)
    );
    assert_eq!(
        validate_profile(2, &default, &ctx(&both, None)),
        Err(ProfileRejection::UnknownConnector)
    );
    let max = profile(
        2,
        ProfilePurpose::ChargePointMaxProfile,
        0,
        RateUnit::W,
        1.0,
    );
    assert!(validate_profile(0, &max, &ctx(&both, None)).is_ok());
    assert_eq!(
        validate_profile(1, &max, &ctx(&both, None)),
        Err(ProfileRejection::MaxProfileNeedsConnectorZero)
    );
    let mut tx_profile = profile(3, ProfilePurpose::TxProfile, 0, RateUnit::W, 1.0);
    assert_eq!(
        validate_profile(1, &tx_profile, &ctx(&both, None)),
        Err(ProfileRejection::NoMatchingTransaction)
    );
    assert!(validate_profile(1, &tx_profile, &ctx(&both, tx)).is_ok());
    tx_profile.transaction_id = Some(8);
    assert_eq!(
        validate_profile(1, &tx_profile, &ctx(&both, tx)),
        Err(ProfileRejection::NoMatchingTransaction),
        "another transaction"
    );
    tx_profile.transaction_id = Some(7);
    assert!(validate_profile(1, &tx_profile, &ctx(&both, tx)).is_ok());
}

#[test]
fn schedule_validation_catches_bad_periods_and_missing_start() {
    let both = [RateUnit::W];
    let c = ctx(&both, None);
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1.0);
    assert!(validate_profile(0, &p, &c).is_ok());
    p.charging_schedule.charging_schedule_period[0].start_period = 5;
    assert_eq!(
        validate_profile(0, &p, &c),
        Err(ProfileRejection::InvalidSchedule),
        "must start at 0"
    );
    p.charging_schedule.charging_schedule_period.clear();
    assert_eq!(
        validate_profile(0, &p, &c),
        Err(ProfileRejection::InvalidSchedule),
        "empty"
    );
    let mut p = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, -1.0);
    assert_eq!(
        validate_profile(0, &p, &c),
        Err(ProfileRejection::InvalidSchedule),
        "negative"
    );
    p.charging_schedule.charging_schedule_period[0].limit = 1.0;
    p.charging_schedule.start_schedule = None;
    assert_eq!(
        validate_profile(0, &p, &c),
        Err(ProfileRejection::MissingStart)
    );
    p.charging_profile_kind = ProfileKind::Relative;
    assert!(
        validate_profile(0, &p, &c).is_ok(),
        "relative needs no start"
    );
}

#[test]
fn relative_profile_starts_with_the_transaction() {
    let units = [RateUnit::W];
    let mut p = profile(1, ProfilePurpose::TxProfile, 0, RateUnit::W, 1500.0);
    p.charging_profile_kind = ProfileKind::Relative;
    p.charging_schedule.start_schedule = None;
    let mut store = ProfileStore::new();
    store
        .set(1, p, &ctx(&units, Some((7, t0() - secs(120)))), t0())
        .unwrap();
    assert_eq!(
        store
            .profiles()
            .next()
            .unwrap()
            .charging_schedule
            .start_schedule,
        Some(t0() - secs(120))
    );
    assert_eq!(
        effective_limit(store.profiles(), t0(), 3).map(|l| l.limit_w),
        Some(1500.0)
    );
}

#[test]
fn profile_json_from_the_backend_parses() {
    let json = serde_json::json!({
        "chargingProfileId": 4711, "stackLevel": 1,
        "chargingProfilePurpose": "TxDefaultProfile", "chargingProfileKind": "Absolute",
        "validTo": "2026-10-06T12:15:00.000Z",
        "chargingSchedule": { "startSchedule": "2026-10-06T11:59:00.000Z", "chargingRateUnit": "A",
          "chargingSchedulePeriod": [ { "startPeriod": 0, "limit": 10.7, "numberPhases": 3 } ] }
    });
    let p: ChargingProfile = serde_json::from_value(json).unwrap();
    assert_eq!(p.charging_profile_id, 4711);
    assert_eq!(p.valid_to.map(|v| v - t0()), Some(secs(900)));
    let l = effective_limit([&p], t0(), 3).unwrap();
    assert!((l.limit_w - 10.7 * 690.0).abs() < 1e-9);
}

#[test]
fn store_is_bounded_in_profiles_and_periods() {
    let units = [RateUnit::W];
    let c = ctx(&units, None);
    let mut store = ProfileStore::new();
    for n in 0..MAX_PROFILES as i64 {
        let p = profile(
            n,
            ProfilePurpose::TxDefaultProfile,
            n as u32,
            RateUnit::W,
            1.0,
        );
        store.set(0, p, &c, t0()).unwrap();
    }
    let extra = profile(
        1000,
        ProfilePurpose::TxDefaultProfile,
        999,
        RateUnit::W,
        1.0,
    );
    assert_eq!(
        store.set(0, extra, &c, t0()),
        Err(ProfileRejection::TooManyProfiles)
    );
    assert_eq!(store.len(), MAX_PROFILES);
    let replacing = profile(0, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 2.0);
    assert!(
        store.set(0, replacing, &c, t0()).is_ok(),
        "counter-check: replacing does not grow the store"
    );

    let mut many = profile(1, ProfilePurpose::TxDefaultProfile, 0, RateUnit::W, 1.0);
    many.charging_schedule.charging_schedule_period = (0..=MAX_PERIODS as i64)
        .map(|n| SchedulePeriod {
            start_period: n * 10,
            limit: 1.0,
            number_phases: None,
        })
        .collect();
    assert_eq!(
        validate_profile(0, &many, &ctx(&units, None)),
        Err(ProfileRejection::TooManyPeriods)
    );
    many.charging_schedule.charging_schedule_period.pop();
    assert!(validate_profile(0, &many, &ctx(&units, None)).is_ok());
}
