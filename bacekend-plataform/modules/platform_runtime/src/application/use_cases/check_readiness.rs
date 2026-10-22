use crate::domain::{
    entities::runtime_readiness::RuntimeReadiness,
    errors::correlation_id_error::{CorrelationId, CorrelationIdError},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadinessResult {
    pub component: &'static str,
    pub readiness: RuntimeReadiness,
    pub correlation_id: String,
    pub trace_id: String,
}

pub fn execute(
    correlation_id: String,
    trace_id: String,
) -> Result<ReadinessResult, CorrelationIdError> {
    let correlation_id = CorrelationId::parse(correlation_id)?;
    Ok(ReadinessResult {
        component: "platform_runtime",
        readiness: RuntimeReadiness::Ready,
        correlation_id: correlation_id.as_str().to_owned(),
        trace_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_result_preserves_valid_correlation_and_trace() {
        let result =
            execute("corr_123".to_owned(), "trace-456".to_owned()).expect("valid identifiers");
        assert_eq!(result.component, "platform_runtime");
        assert_eq!(result.readiness, RuntimeReadiness::Ready);
        assert_eq!(result.correlation_id, "corr_123");
        assert_eq!(result.trace_id, "trace-456");
    }

    #[test]
    fn readiness_rejects_invalid_correlation() {
        assert!(execute("bad value".to_owned(), "trace-456".to_owned()).is_err());
    }
}
