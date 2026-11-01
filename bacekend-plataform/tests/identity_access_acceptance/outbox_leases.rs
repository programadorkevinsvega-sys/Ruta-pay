use super::*;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn verify(migration_pool: &PgPool, runtime_pool: &PgPool, created_tenant_id: Uuid) {
    // Expired Outbox leases are reclaimable, and fencing tokens prevent a
    // previous worker from acknowledging an event after another worker owns it.
    for schema in ["identity_access", "tenant_registry"] {
        if schema == "identity_access" {
            sqlx::query(
                "UPDATE identity_access.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL",
            )
            .execute(migration_pool)
            .await
            .expect("identity pending events are deferred for lease acceptance");
        } else {
            sqlx::query(
                "UPDATE tenant_registry.outbox_events SET available_at=now()+interval '1 hour' WHERE published_at IS NULL",
            )
            .execute(migration_pool)
            .await
            .expect("tenant pending events are deferred for lease acceptance");
        }
        let event_id = Uuid::new_v4();
        let aggregate_id = Uuid::new_v4();
        let stale_lease = Uuid::new_v4();
        let insert_expired = if schema == "identity_access" {
            sqlx::query(
                "INSERT INTO identity_access.outbox_events                  (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id,available_at,lease_token,lease_until,attempts)                  VALUES ($1,$2,1,'tenant.status.changed',1,'tenant_and_platform',$3,$4,now()-interval '1 second',$5,now()-interval '1 second',2)",
            )
        } else {
            sqlx::query(
                "INSERT INTO tenant_registry.outbox_events                  (event_id,aggregate_id,aggregate_version,event_type,schema_version,audience,payload,correlation_id,available_at,lease_token,lease_until,attempts)                  VALUES ($1,$2,1,'tenant.status.changed',1,'tenant_and_platform',$3,$4,now()-interval '1 second',$5,now()-interval '1 second',2)",
            )
        };
        insert_expired
            .bind(event_id)
            .bind(aggregate_id)
            .bind(serde_json::json!({"tenant_id": created_tenant_id}))
            .bind(format!("corr_{schema}_expired_lease"))
            .bind(stale_lease)
            .execute(migration_pool)
            .await
            .expect("expired synthetic Outbox lease is inserted");
        let (claimed_event_id, new_lease) = if schema == "identity_access" {
            let claimed =
                platform_identity_access::infrastructure::persistence::claim_outbox_events(
                    runtime_pool,
                    32,
                )
                .await
                .expect("identity Outbox worker reclaims an expired lease");
            assert_eq!(
                claimed.len(),
                1,
                "only the expired {schema} event is eligible"
            );
            (claimed[0].event_id, claimed[0].lease_token)
        } else {
            let claimed =
                platform_tenant_registry::infrastructure::persistence::claim_outbox_events(
                    runtime_pool,
                    32,
                )
                .await
                .expect("tenant Outbox worker reclaims an expired lease");
            assert_eq!(
                claimed.len(),
                1,
                "only the expired {schema} event is eligible"
            );
            (claimed[0].event_id, claimed[0].lease_token)
        };
        assert_eq!(claimed_event_id, event_id);
        assert_ne!(new_lease, stale_lease);
        let old_worker_acknowledged = if schema == "identity_access" {
            platform_identity_access::infrastructure::persistence::mark_outbox_published(
                runtime_pool,
                event_id,
                stale_lease,
            )
            .await
            .expect("identity stale lease acknowledgement is checked")
        } else {
            platform_tenant_registry::infrastructure::persistence::mark_outbox_published(
                runtime_pool,
                event_id,
                stale_lease,
            )
            .await
            .expect("tenant stale lease acknowledgement is checked")
        };
        assert!(!old_worker_acknowledged, "stale {schema} lease cannot ack");
        let new_worker_acknowledged = if schema == "identity_access" {
            platform_identity_access::infrastructure::persistence::mark_outbox_published(
                runtime_pool,
                event_id,
                new_lease,
            )
            .await
            .expect("identity current lease acknowledgement is checked")
        } else {
            platform_tenant_registry::infrastructure::persistence::mark_outbox_published(
                runtime_pool,
                event_id,
                new_lease,
            )
            .await
            .expect("tenant current lease acknowledgement is checked")
        };
        assert!(new_worker_acknowledged, "current {schema} lease can ack");
        let attempts: i32 = if schema == "identity_access" {
            sqlx::query_scalar(
                "SELECT attempts FROM identity_access.outbox_events WHERE event_id=$1",
            )
            .bind(event_id)
            .fetch_one(migration_pool)
            .await
        } else {
            sqlx::query_scalar(
                "SELECT attempts FROM tenant_registry.outbox_events WHERE event_id=$1",
            )
            .bind(event_id)
            .fetch_one(migration_pool)
            .await
        }
        .expect("reclaimed Outbox attempt count is queryable");
        assert_eq!(attempts, 3);
    }
}
