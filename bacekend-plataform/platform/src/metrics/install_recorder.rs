use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

pub fn install_recorder() -> Result<PrometheusHandle, metrics_exporter_prometheus::BuildError> {
    PrometheusBuilder::new().install_recorder()
}
