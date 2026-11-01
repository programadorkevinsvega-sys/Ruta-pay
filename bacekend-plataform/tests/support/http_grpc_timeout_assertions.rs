use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub fn verify(
    capture_store: &Arc<Mutex<Vec<HashMap<String, String>>>>,
    timeout_correlation_id: &str,
) {
        let timeout_events = capture_store.lock().expect("event capture lock").clone();
        let dependency_failure = timeout_events
            .iter()
            .find(|event| {
                event
                    .get("event.name")
                    .is_some_and(|name| name.contains("bff.dependency.call.failed"))
            })
            .expect("timeout dependency failure event");
        let request_failure = timeout_events
            .iter()
            .find(|event| {
                event
                    .get("event.name")
                    .is_some_and(|name| name.contains("bff.request.failed"))
            })
            .expect("timeout request failure event");
        let timeout_closure = timeout_events
            .iter()
            .find(|event| {
                event
                    .get("event.name")
                    .is_some_and(|name| name.contains("http.request.completed"))
            })
            .expect("timeout HTTP completion event");
        for failure_event in [dependency_failure, request_failure] {
            for field in ["correlation_id", "trace_id"] {
                assert_eq!(
                    failure_event.get(field),
                    timeout_closure.get(field),
                    "timeout failure and HTTP closure must share {field}"
                );
            }
            assert!(
                failure_event
                    .get("error_category")
                    .is_some_and(|category| category.contains("timeout"))
            );
        }
        assert!(
            timeout_closure
                .get("error_code")
                .is_some_and(|code| code.contains("module_timeout"))
        );
        assert!(
            timeout_closure
                .get("error_category")
                .is_some_and(|category| category.contains("timeout"))
        );
        assert_eq!(
            timeout_closure.get("correlation_id").map(String::as_str),
            Some(timeout_correlation_id)
        );
        assert!(
            !timeout_events
                .iter()
                .flat_map(|event| event.values())
                .any(|value| value.contains("sentinel-sensitive-token")),
            "timeout diagnostics must redact authorization credentials"
        );
}
