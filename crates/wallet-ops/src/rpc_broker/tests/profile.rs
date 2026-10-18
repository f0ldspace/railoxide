use super::*;

#[test]
fn health_cooldown_escalates_without_resetting() {
    let mut profile = EndpointProfile::default();
    let now = Instant::now();
    for _ in 0..HEALTH_STRIKE_THRESHOLD.saturating_sub(1) {
        assert!(!profile.record_failure(now));
    }
    assert!(profile.record_failure(now));
    assert!(profile.is_withdrawn(now));
    let first = profile.withdrawn_until.expect("withdrawal");
    assert!(profile.restore_if_ready(first));
    for _ in 0..HEALTH_STRIKE_THRESHOLD.saturating_sub(1) {
        assert!(!profile.record_failure(first));
    }
    assert!(profile.record_failure(first));
    let second = profile.withdrawn_until.expect("second withdrawal");
    assert!(second > first);
    assert!(second - first > HEALTH_WITHDRAWAL_BASE);
}

#[test]
fn warmup_strike_withdraws_at_once_without_shortening_an_active_cooldown() {
    let mut profile = EndpointProfile::default();
    let now = Instant::now();
    let first = now + HEALTH_WITHDRAWAL_BASE;
    assert!(profile.withdraw_after_warmup_strike(now));
    assert_eq!(profile.withdrawn_until, Some(first));
    // A strike during the cooldown neither shortens nor escalates it.
    assert!(!profile.withdraw_after_warmup_strike(now + HEALTH_WITHDRAWAL_BASE / 2));
    assert_eq!(profile.withdrawn_until, Some(first));
    assert_eq!(profile.withdrawal_level, 1);
    // Expiry restores the endpoint, and the next withdrawal escalates as an ordinary one would.
    assert!(profile.restore_if_ready(first));
    assert!(!profile.is_withdrawn(first));
    assert!(profile.withdraw_after_warmup_strike(first));
    assert_eq!(
        profile.withdrawn_until,
        Some(first + HEALTH_WITHDRAWAL_BASE * 2)
    );
}
