use crate::{
    application::tenant_service::{RegisterTenantCommand, TenantService, TenantServiceError},
    domain::tenant::TenantStatus,
};
use payvia_observability::{current_span_id, current_trace_id};
use platform_contracts::tenant_registry::{
    ChangeTenantStatusRequest, GetTenantRequest, ListTenantsRequest, ListTenantsResponse,
    RegisterTenantRequest, TenantResponse, tenant_registry_server::TenantRegistry,
};
use tonic::{Request, Response, Status};
use tracing::Instrument;
use uuid::Uuid;

#[derive(Clone)]
pub struct TenantRegistryApi {
    service: TenantService,
}

impl TenantRegistryApi {
    pub fn new(service: TenantService) -> Self {
        Self { service }
    }
}

#[tonic::async_trait]
impl TenantRegistry for TenantRegistryApi {
    async fn register_tenant(
        &self,
        request: Request<RegisterTenantRequest>,
    ) -> Result<Response<TenantResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let span = tracing::info_span!(
            "grpc.module.tenant_registry",
            operation = "register_tenant",
            correlation_id
        );
        async move {
            let started = std::time::Instant::now();
            match self.service.register(RegisterTenantCommand {
                session_token: &body.session_token,
                csrf_token: &body.csrf_token,
                display_name: &body.display_name,
                slug: &body.slug,
                identification_type: &body.legal_identification_type,
                identification_number: &body.legal_identification_number,
                correlation_id: &correlation_id,
            }).await {
                Ok(tenant) => {
                    tracing::info!(event.name = "grpc.module.response", component = "tenant_registry", operation = "register_tenant", result = "success", tenant_id = %tenant.id, duration_total_ms = started.elapsed().as_millis() as u64, correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(tenant_response(tenant)))
                }
                Err(error) => Err(map_error(error, "register_tenant", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    async fn change_tenant_status(
        &self,
        request: Request<ChangeTenantStatusRequest>,
    ) -> Result<Response<TenantResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let span = tracing::info_span!(
            "grpc.module.tenant_registry",
            operation = "change_tenant_status",
            correlation_id
        );
        async move {
            let started = std::time::Instant::now();
            let tenant_id = parse_uuid(&body.tenant_id)?;
            let next = TenantStatus::parse(&body.status).map_err(|_| Status::invalid_argument("tenant status is invalid"))?;
            match self.service.change_status(crate::application::tenant_service::ChangeTenantStatusCommand {
                session_token: &body.session_token,
                csrf_token: &body.csrf_token,
                tenant_id,
                next,
                reason: &body.reason,
                expected_version: body.expected_version,
                correlation_id: &correlation_id,
            }).await {
                Ok(tenant) => {
                    tracing::info!(event.name = "grpc.module.response", component = "tenant_registry", operation = "change_tenant_status", result = "success", tenant_id = %tenant.id, duration_total_ms = started.elapsed().as_millis() as u64, correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id());
                    Ok(Response::new(tenant_response(tenant)))
                }
                Err(error) => Err(map_error(error, "change_tenant_status", &correlation_id, started.elapsed().as_millis() as u64)),
            }
        }.instrument(span).await
    }

    async fn get_tenant(
        &self,
        request: Request<GetTenantRequest>,
    ) -> Result<Response<TenantResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let tenant_id = parse_uuid(&body.tenant_id)?;
        match self
            .service
            .get(
                &body.session_token,
                &body.csrf_token,
                tenant_id,
                &correlation_id,
            )
            .await
        {
            Ok(tenant) => Ok(Response::new(tenant_response(tenant))),
            Err(error) => Err(map_error(error, "get_tenant", &correlation_id, 0)),
        }
    }

    async fn list_tenants(
        &self,
        request: Request<ListTenantsRequest>,
    ) -> Result<Response<ListTenantsResponse>, Status> {
        let body = request.into_inner();
        let correlation_id = body.correlation_id.clone();
        let started = std::time::Instant::now();
        match self
            .service
            .list_for_operator(
                &body.session_token,
                &body.csrf_token,
                &body.query,
                i64::from(body.limit),
                i64::from(body.offset),
                &correlation_id,
            )
            .await
        {
            Ok((tenants, total)) => Ok(Response::new(ListTenantsResponse {
                tenants: tenants.into_iter().map(tenant_response).collect(),
                total,
            })),
            Err(error) => Err(map_error(
                error,
                "list_tenants",
                &correlation_id,
                started.elapsed().as_millis() as u64,
            )),
        }
    }
}

fn tenant_response(tenant: crate::domain::tenant::Tenant) -> TenantResponse {
    TenantResponse {
        tenant_id: tenant.id.to_string(),
        display_name: tenant.display_name,
        slug: tenant.slug,
        status: tenant.status.as_str().to_owned(),
        status_version: tenant.status_version,
        created_at: tenant.created_at.to_rfc3339(),
        updated_at: tenant.updated_at.to_rfc3339(),
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("tenant identifier is invalid"))
}

fn map_error(
    error: TenantServiceError,
    operation: &'static str,
    correlation_id: &str,
    duration_ms: u64,
) -> Status {
    let (code, category, status) = match &error {
        TenantServiceError::Domain(_)
        | TenantServiceError::InvalidQuery
        | TenantServiceError::InvalidTransition => (
            "tenant_input_invalid",
            "validation",
            Status::invalid_argument("tenant request is invalid"),
        ),
        TenantServiceError::Identity(
            crate::application::identity_access_port::IdentityAccessError::Authentication,
        ) => (
            "authentication_failed",
            "authentication",
            Status::unauthenticated("authentication failed"),
        ),
        TenantServiceError::Identity(
            crate::application::identity_access_port::IdentityAccessError::Authorization,
        ) => (
            "authorization_denied",
            "authorization",
            Status::permission_denied("operation is not permitted"),
        ),
        TenantServiceError::Identity(
            crate::application::identity_access_port::IdentityAccessError::Invalid,
        ) => (
            "identity_request_invalid",
            "validation",
            Status::invalid_argument("identity request is invalid"),
        ),
        TenantServiceError::Identity(
            crate::application::identity_access_port::IdentityAccessError::Unavailable,
        ) => (
            "identity_dependency_unavailable",
            "dependency",
            Status::unavailable("tenant operation could not be completed"),
        ),
        TenantServiceError::NotFound
        | TenantServiceError::Repository(
            crate::infrastructure::persistence::TenantRepositoryError::NotFound,
        ) => (
            "tenant_not_found",
            "not_found",
            Status::not_found("tenant was not found"),
        ),
        TenantServiceError::Repository(
            crate::infrastructure::persistence::TenantRepositoryError::VersionConflict,
        ) => (
            "tenant_version_conflict",
            "conflict",
            Status::aborted("tenant changed; refresh and retry"),
        ),
        TenantServiceError::Repository(
            crate::infrastructure::persistence::TenantRepositoryError::Domain(_),
        ) => (
            "tenant_transition_invalid",
            "conflict",
            Status::failed_precondition("tenant status transition is not allowed"),
        ),
        TenantServiceError::Repository(
            crate::infrastructure::persistence::TenantRepositoryError::Database(database),
        )
        | TenantServiceError::Database(database) => {
            let sqlstate = database
                .as_database_error()
                .and_then(|value| value.code())
                .unwrap_or_default();
            let conflict = sqlstate == "23505";
            tracing::error!(event.name = "grpc.module.request.failed", component = "tenant_registry", operation, dependency = "postgresql", error_code = if conflict { "tenant_unique_conflict" } else { "tenant_persistence_failed" }, error_category = if conflict { "conflict" } else { "persistence" }, sqlstate = %sqlstate, constraint = database.as_database_error().and_then(|value| value.constraint()).unwrap_or("unknown"), dependency_duration_ms = duration_ms, correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id(), retryable = false);
            return if conflict {
                Status::already_exists("tenant identifier is already in use")
            } else {
                Status::internal("tenant operation could not be completed")
            };
        }
    };
    tracing::warn!(event.name = "grpc.module.request.failed", component = "tenant_registry", operation, error_code = code, error_category = category, dependency_duration_ms = duration_ms, correlation_id, trace_id = %current_trace_id(), span_id = %current_span_id(), retryable = false);
    status
}
