ALTER TABLE tenant_registry.tenants
    ADD COLUMN legal_identification_type TEXT,
    ADD COLUMN legal_identification_number TEXT;

ALTER TABLE tenant_registry.tenants
    ADD CONSTRAINT tenants_legal_identification_pair_check
    CHECK (
        (legal_identification_type IS NULL AND legal_identification_number IS NULL)
        OR (
            legal_identification_type IN ('nit', 'cc')
            AND legal_identification_number ~ '^[0-9]{5,20}$'
        )
    );

CREATE UNIQUE INDEX tenants_legal_identification_unique
    ON tenant_registry.tenants (legal_identification_type, legal_identification_number)
    WHERE legal_identification_type IS NOT NULL;
