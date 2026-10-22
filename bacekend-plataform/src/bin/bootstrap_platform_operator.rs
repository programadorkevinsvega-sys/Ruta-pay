use platform_identity_access::application::operator_bootstrap::OperatorBootstrapService;
use sqlx::postgres::PgPoolOptions;
use std::{env, io};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let email = arguments
        .next()
        .ok_or("usage: bootstrap-platform-operator <operator-email>")?;
    if arguments.next().is_some() {
        return Err("usage: bootstrap-platform-operator <operator-email>".into());
    }
    let database_url = env::var("PLATFORM_DATABASE_URL")
        .map_err(|_| "PLATFORM_DATABASE_URL must be configured in bacekend-plataform/.env")?;
    let password = rpassword::prompt_password("Initial operator password: ")?;
    let confirmation = rpassword::prompt_password("Confirm password: ")?;
    if password != confirmation {
        return Err("password confirmation does not match".into());
    }
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .map_err(|_| "operator bootstrap could not connect to configured PostgreSQL")?;
    let correlation_id = Uuid::new_v4().to_string();
    let service = OperatorBootstrapService::new(pool.clone());
    let operator_id = service
        .bootstrap_first_operator(&email, &password, &correlation_id)
        .await
        .map_err(|error| match error {
            platform_identity_access::application::identity_service::IdentityError::OperatorBootstrapClosed => {
                io::Error::new(io::ErrorKind::PermissionDenied, "operator bootstrap was already completed")
            }
            platform_identity_access::application::identity_service::IdentityError::Database(_) => {
                io::Error::other("operator bootstrap failed while persisting the operator")
            }
            platform_identity_access::application::identity_service::IdentityError::Email(_) => {
                io::Error::new(io::ErrorKind::InvalidInput, "operator email is invalid")
            }
            platform_identity_access::application::identity_service::IdentityError::Password(_) => {
                io::Error::new(io::ErrorKind::InvalidInput, "operator password does not meet the password policy")
            }
            platform_identity_access::application::identity_service::IdentityError::PasswordHash(_) => {
                io::Error::other("operator password hashing failed")
            }
            platform_identity_access::application::identity_service::IdentityError::HashingTaskFailed => {
                io::Error::other("operator password hashing task failed")
            }
            _ => io::Error::other("operator bootstrap failed"),
        })?;
    pool.close().await;
    println!(
        "{}",
        serde_json::json!({
            "event.name": "platform.operator.bootstrap.completed",
            "component": "identity_access",
            "operation": "bootstrap_first_operator",
            "operator_id": operator_id,
            "correlation_id": correlation_id,
            "result": "success"
        })
    );
    Ok(())
}
