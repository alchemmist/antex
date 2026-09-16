use super::*;

#[test]
fn new_process_starts_on_standard_tier_without_overwriting_other_tiers() {
    for service_tier in [None, Some("fast".to_string()), Some("priority".to_string())] {
        assert_eq!(
            service_tier_for_new_process(service_tier).as_deref(),
            Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE)
        );
    }

    assert_eq!(
        service_tier_for_new_process(Some("flex".to_string())).as_deref(),
        Some("flex")
    );
}
