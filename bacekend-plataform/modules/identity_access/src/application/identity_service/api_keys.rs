use sqlx::Row;
use uuid::Uuid;

use crate::domain::api_key::{create_api_key, ApiKeyEnvironment};
use super::{ApiKeyMetadata, CreatedApiKey, IdentityError, IdentityService};
use super::support::{insert_audit, set_tenant_context};

impl IdentityService {
    pub async fn create_api_key(
        &self,
        session_token: &str,
        tenant_id: Uuid,
        name: &str,
        environment: &str,
        scopes: &[String],
        correlation_id: &str,
    ) -> Result<CreatedApiKey, IdentityError> {
        let actor_id = self
            .authorized_tenant_actor(session_token, tenant_id)
            .await?;
        let environment = match environment {
            "test" => ApiKeyEnvironment::Test,
            "live" => ApiKeyEnvironment::Live,
            _ => return Err(IdentityError::ApiKeyInputInvalid),
        };
        let key = create_api_key(name, environment, scopes)
            .map_err(|_| IdentityError::ApiKeyInputInvalid)?;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id, actor_id).await?;
        sqlx::query(
            "INSERT INTO identity_access.api_keys (id, tenant_id, name, environment, prefix, secret_digest, scopes, status, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,'active',$8)",
        )
        .bind(key.id)
        .bind(tenant_id)
        .bind(&key.name)
        .bind(key.environment.as_str())
        .bind(&key.prefix)
        .bind(key.digest.as_slice())
        .bind(&key.scopes)
        .bind(actor_id)
        .execute(&mut *transaction)
        .await?;
        let payload = serde_json::json!({"tenant_id": tenant_id, "api_key_id": key.id, "status": "active", "environment": key.environment.as_str()});
        sqlx::query(
            "INSERT INTO identity_access.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,1,'api_key.status.changed',1,'tenant',$3,$4)",
        )
        .bind(Uuid::new_v4())
        .bind(key.id)
        .bind(payload)
        .bind(correlation_id)
        .execute(&mut *transaction)
        .await?;
        insert_audit(
            &mut transaction,
            "api_key.created",
            actor_id,
            tenant_id,
            key.id,
            correlation_id,
        )
        .await?;
        transaction.commit().await?;
        Ok(CreatedApiKey { tenant_id, key })
    }

    pub async fn list_api_keys(
        &self,
        session_token: &str,
        tenant_id: Uuid,
    ) -> Result<Vec<ApiKeyMetadata>, IdentityError> {
        let actor_id = self
            .authorized_tenant_actor(session_token, tenant_id)
            .await?;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id, actor_id).await?;
        let rows = sqlx::query(
            "SELECT id,name,prefix,environment,scopes,status,created_at,revoked_at FROM identity_access.api_keys WHERE tenant_id=$1 ORDER BY created_at DESC",
        )
        .bind(tenant_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            keys.push(ApiKeyMetadata {
                id: row.try_get("id").map_err(IdentityError::Database)?,
                name: row.try_get("name").map_err(IdentityError::Database)?,
                prefix: row.try_get("prefix").map_err(IdentityError::Database)?,
                environment: row
                    .try_get("environment")
                    .map_err(IdentityError::Database)?,
                scopes: row.try_get("scopes").map_err(IdentityError::Database)?,
                status: row.try_get("status").map_err(IdentityError::Database)?,
                created_at: row.try_get("created_at").map_err(IdentityError::Database)?,
                revoked_at: row.try_get("revoked_at").map_err(IdentityError::Database)?,
            });
        }
        transaction.commit().await?;
        Ok(keys)
    }

    pub async fn revoke_api_key(
        &self,
        session_token: &str,
        tenant_id: Uuid,
        key_id: Uuid,
        correlation_id: &str,
    ) -> Result<(), IdentityError> {
        let actor_id = self
            .authorized_tenant_actor(session_token, tenant_id)
            .await?;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id, actor_id).await?;
        let result = sqlx::query("UPDATE identity_access.api_keys SET status='revoked',revoked_at=now() WHERE id=$1 AND tenant_id=$2 AND status='active'")
            .bind(key_id).bind(tenant_id).execute(&mut *transaction).await?;
        if result.rows_affected() == 0 {
            return Err(IdentityError::ApiKeyNotFound);
        }
        let payload =
            serde_json::json!({"tenant_id": tenant_id, "api_key_id": key_id, "status": "revoked"});
        sqlx::query("INSERT INTO identity_access.outbox_events (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id) VALUES ($1,$2,2,'api_key.status.changed',1,'tenant',$3,$4)")
            .bind(Uuid::new_v4()).bind(key_id).bind(payload).bind(correlation_id).execute(&mut *transaction).await?;
        insert_audit(
            &mut transaction,
            "api_key.revoked",
            actor_id,
            tenant_id,
            key_id,
            correlation_id,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}
