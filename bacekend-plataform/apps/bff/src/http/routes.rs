use crate::http::{
    handlers::{health_live, health_ready, openapi},
    middleware::request_diagnostics,
};
use axum::{Router, middleware, routing::get};
use platform_contracts::runtime::runtime_probe_client::RuntimeProbeClient;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub grpc_client: RuntimeProbeClient<tonic::transport::Channel>,
}

pub fn router(grpc_client: RuntimeProbeClient<tonic::transport::Channel>) -> Router {
    Router::new()
        .route("/health/live", get(health_live::handle))
        .route("/health/ready", get(health_ready::handle))
        .route("/openapi.json", get(openapi::handle))
        .with_state(Arc::new(AppState { grpc_client }))
        .layer(middleware::from_fn(request_diagnostics::handle))
}
