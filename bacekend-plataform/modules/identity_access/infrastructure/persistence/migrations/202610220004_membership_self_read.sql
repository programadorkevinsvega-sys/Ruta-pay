DROP POLICY tenant_memberships_tenant_isolation ON identity_access.tenant_memberships;

CREATE POLICY tenant_memberships_tenant_isolation
    ON identity_access.tenant_memberships
    USING (
        tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid
        OR user_id = nullif(current_setting('app.actor_id', true), '')::uuid
    )
    WITH CHECK (
        tenant_id = nullif(current_setting('app.tenant_id', true), '')::uuid
    );
