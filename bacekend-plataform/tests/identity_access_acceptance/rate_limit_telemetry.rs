use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub fn verify(captured_events: &Arc<Mutex<Vec<HashMap<String, String>>>>) {
    let rate_limit_events = captured_events.lock().expect("event capture lock").clone();
    let rate_limit_module_failure = rate_limit_events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("grpc.module.request.failed"))
                && event
                    .get("error_code")
                    .is_some_and(|code| code.contains("identity_rate_limited"))
                && event
                    .get("correlation_id")
                    .is_some_and(|value| value.contains("corr_identity_rate_limit_8"))
        })
        .expect("causal rate limit event is recorded");
    let rate_limit_http_closure = rate_limit_events
        .iter()
        .find(|event| {
            event
                .get("event.name")
                .is_some_and(|name| name.contains("http.request.completed"))
                && event
                    .get("correlation_id")
                    .is_some_and(|value| value.contains("corr_identity_rate_limit_8"))
        })
        .expect("rate limit HTTP closure event is recorded");
    for field in ["correlation_id", "trace_id"] {
        assert_eq!(
            rate_limit_module_failure.get(field),
            rate_limit_http_closure.get(field)
        );
    }
    assert!(
        rate_limit_http_closure
            .get("error_code")
            .is_some_and(|code| code.contains("identity_rate_limited"))
    );
    assert!(
        rate_limit_http_closure
            .get("error_category")
            .is_some_and(|category| category.contains("rate_limit"))
    );
}
