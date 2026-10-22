use opentelemetry::propagation::Injector;
use platform_contracts::runtime::{
    ReadinessRequest, ReadinessResponse, runtime_probe_client::RuntimeProbeClient as GrpcClient,
};
use tonic::{
    metadata::{MetadataKey, MetadataMap, MetadataValue},
    transport::Channel,
};

pub struct RuntimeProbeClient;

impl RuntimeProbeClient {
    pub async fn check(
        client: &GrpcClient<Channel>,
        correlation_id: &str,
    ) -> Result<ReadinessResponse, tonic::Status> {
        let mut client = client.clone();
        let mut request = tonic::Request::new(ReadinessRequest {
            correlation_id: correlation_id.to_owned(),
        });
        request.metadata_mut().insert(
            "x-correlation-id",
            MetadataValue::try_from(correlation_id)
                .map_err(|_| tonic::Status::invalid_argument("invalid correlation identifier"))?,
        );
        opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.inject(&mut MetadataInjector(request.metadata_mut()))
        });
        client
            .check_readiness(request)
            .await
            .map(tonic::Response::into_inner)
    }
}

struct MetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(key), Ok(value)) = (
            MetadataKey::from_bytes(key.as_bytes()),
            MetadataValue::try_from(value),
        ) {
            self.0.insert(key, value);
        }
    }
}
