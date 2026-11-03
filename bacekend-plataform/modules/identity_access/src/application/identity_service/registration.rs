use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{IdentityError, IdentityService};
use crate::domain::{
    email_address::EmailAddress, password::validate_password, password_hash::hash_password,
};

const REGISTRATION_WINDOW_SECONDS: u64 = 300;
const REGISTRATION_ACCOUNT_LIMIT: i64 = 5;
const REGISTRATION_IP_LIMIT: i64 = 20;

impl IdentityService {
    pub async fn register_tenant_user(
        &self,
        email: &str,
        password: &str,
        source_ip: &str,
        correlation_id: &str,
        traceparent: Option<&str>,
    ) -> Result<Uuid, IdentityError> {
        let email = EmailAddress::parse(email)?;
        self.enforce_registration_rate_limit(email.as_str(), source_ip)
            .await?;
        validate_password(password)?;
        let password_owned = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || hash_password(&password_owned))
            .await
            .map_err(|_| IdentityError::HashingTaskFailed)??;
        let user_id = Uuid::new_v4();
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "INSERT INTO identity_access.tenant_users (id, email_normalized, password_hash, status) VALUES ($1, $2, $3, 'active')",
        )
        .bind(user_id)
        .bind(email.as_str())
        .bind(password_hash)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = result {
            if error
                .as_database_error()
                .and_then(|db| db.code())
                .as_deref()
                == Some("23505")
            {
                return Err(IdentityError::AccountConflict);
            }
            return Err(IdentityError::Database(error));
        }
        let event_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO identity_access.outbox_events (event_id, aggregate_id, aggregate_version, event_type, schema_version, audience, payload, traceparent, correlation_id) VALUES ($1, $2, 1, 'identity.tenant_user.registered', 1, 'platform_operator', $3, $4, $5)",
        )
        .bind(event_id)
        .bind(user_id)
        .bind(serde_json::json!({ "user_id": user_id, "status": "active" }))
        .bind(traceparent)
        .bind(correlation_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO identity_access.audit_events (event_id, actor_kind, actor_id, action, resource_id, outcome, correlation_id, trace_id) VALUES ($1, 'tenant_user', $2, 'tenant_user.registered', $2, 'completed', $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(correlation_id)
        .bind(payvia_observability::current_trace_id())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(user_id)
    }

    async fn enforce_registration_rate_limit(
        &self,
        email: &str,
        source_ip: &str,
    ) -> Result<(), IdentityError> {
        let source_ip = source_ip
            .parse::<std::net::IpAddr>()
            .map_err(|_| IdentityError::SourceAddressInvalid)?;
        let account_digest = Sha256::digest(email.as_bytes());
        let ip_digest = Sha256::digest(source_ip.to_string().as_bytes());
        let account_key = format!(
            "platform:identity:registration:account:{}",
            URL_SAFE_NO_PAD.encode(account_digest)
        );
        let ip_key = format!(
            "platform:identity:registration:ip:{}",
            URL_SAFE_NO_PAD.encode(ip_digest)
        );

        // The account and peer-IP counters share one fixed five-minute window.
        // Increment both counters and set their expirations atomically so concurrent
        // registration requests cannot bypass either ceiling.
        const RATE_LIMIT_SCRIPT: &str = r#"
            local account_count = redis.call('INCR', KEYS[1])
            if account_count == 1 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            local ip_count = redis.call('INCR', KEYS[2])
            if ip_count == 1 then
                redis.call('EXPIRE', KEYS[2], ARGV[1])
            end
            return {account_count, ip_count}
        "#;
        let mut connection = self.redis.clone();
        let (account_count, ip_count): (i64, i64) = redis::cmd("EVAL")
            .arg(RATE_LIMIT_SCRIPT)
            .arg(2)
            .arg(&account_key)
            .arg(&ip_key)
            .arg(REGISTRATION_WINDOW_SECONDS)
            .query_async(&mut connection)
            .await
            .map_err(IdentityError::Redis)?;

        if account_count > REGISTRATION_ACCOUNT_LIMIT || ip_count > REGISTRATION_IP_LIMIT {
            return Err(IdentityError::RateLimited);
        }
        Ok(())
    }
}
