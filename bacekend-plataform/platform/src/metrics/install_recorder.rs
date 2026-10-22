use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

const LATENCY_BUCKETS_SECONDS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

pub fn install_recorder() -> Result<PrometheusHandle, metrics_exporter_prometheus::BuildError> {
    PrometheusBuilder::new()
        .set_buckets(LATENCY_BUCKETS_SECONDS)?
        .install_recorder()
}
