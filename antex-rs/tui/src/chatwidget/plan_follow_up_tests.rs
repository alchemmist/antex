use super::*;

#[test]
fn recognizes_explicit_implementation_requests() {
    for request in [
        "Implement the plan.",
        "start implementing",
        "Реализуй этот план!",
        "приступай к реализации",
    ] {
        assert!(is_explicit_implementation_request(request), "{request}");
    }
}

#[test]
fn leaves_plan_revisions_in_plan_mode() {
    for request in [
        "Change the second step",
        "Implement a different cache strategy in the plan",
        "Доработай план",
        "Замени второй пункт",
    ] {
        assert!(!is_explicit_implementation_request(request), "{request}");
    }
}
