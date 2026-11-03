# Phase 3 — catalog, pricing, entitlements and quotas acceptance

## Scope and evidence policy

This acceptance record covers the Rust platform modules `catalog_pricing` and `entitlements_quotas`, their PostgreSQL ownership, gRPC contracts, BFF routes, Outbox/JetStream delivery and the realtime journal boundary.

A criterion is accepted only when its executable test passes on the exact PR HEAD SHA. A test file existing, a prior green workflow, or a successful compile alone is not acceptance evidence. CI and deployment evidence are recorded separately.

## Required test environment

The acceptance suite provisions isolated PostgreSQL databases and restricted runtime roles and uses real Redis and authenticated JetStream services. It does not substitute mocks for persistence, brokers or realtime infrastructure.

Required variables:

- `PLATFORM_TEST_DATABASE_ADMIN_URL`: PostgreSQL administrator URL used only to create and remove isolated test databases and roles.
- `PLATFORM_NATS_URL`, `NATS_USER`, `NATS_PASSWORD`: authenticated JetStream endpoint and credentials.
- `PLATFORM_REDIS_URL`: authenticated Redis endpoint for realtime signal fan-out.

The CI workflow `.github/workflows/platform-quality.yml` provisions these dependencies and runs `cargo test --locked --workspace`.

## Acceptance matrix

| Criterion | Executable evidence | Acceptance condition |
|---|---|---|
| PostgreSQL ownership and migrations | `phase3_catalog_entitlements_acceptance/database.rs` | All owner migrations apply to an isolated database; runtime role has no RLS bypass |
| Price version immutability | `immutability.rs` | Real UPDATE/DELETE attempts fail with SQLSTATE `55000` |
| Capability immutability | `immutability.rs` | Real UPDATE/DELETE attempts fail with SQLSTATE `55000` |
| Usage ledger append-only | `immutability.rs` | Real UPDATE/DELETE attempts fail with SQLSTATE `55000` |
| Atomic catalog write | `atomicity.rs` | Forced Outbox failure rolls back price version and audit write |
| Atomic quota consumption | `atomicity.rs` | Forced Outbox failure rolls back usage ledger and audit write |
| Quota boundaries and policy | `quota.rs` | Below-limit, exact-limit, deny-overage, allow-overage and environment restriction are verified |
| Stable idempotent replay | `quota.rs` + migration `202610310006_usage_idempotency_result.sql` | Replay after a later same-scope consumption returns the original serialized entitlement snapshot; changed request data returns a conflict |
| Legacy idempotency rows | `consumption.rs` | A historical ledger row without a response snapshot fails closed with `IdempotencyResultUnavailable`; it is never recalculated into a misleading response |
| Concurrent quota consumption | `concurrency.rs` | Concurrent requests serialize and cannot exceed a deny-policy quota |
| Price versioning and effective dates | `versioning.rs` | Non-overlapping versions and effective-time selection are verified |
| Tenant isolation and RLS | `isolation.rs` | Tenant context cannot read or consume another tenant's entitlement or usage |
| BFF / REST / gRPC / API key | `identity_access_acceptance/phase3_catalog.rs`, `identity_access_acceptance/phase3_entitlements.rs` and the shared HTTP/gRPC acceptance harness | Real HTTP-to-BFF-to-gRPC path enforces tenant and `usage.consume` scope |
| Gateway gRPC contract | `identity_access_acceptance/phase3_entitlements.rs` + `bacekend-pasarela/src/entitlements.rs` | The real gateway client connects to the platform gRPC service, consumes quota with an API key and replays the same idempotency key; mTLS and signed-grant enforcement remain in Phase 5 |
| Outbox to JetStream | `jetstream.rs` | Real relays receive JetStream ACKs and consumers verify event ID, type, correlation ID and audience |
| Realtime journal and cursor | `jetstream.rs` | The catalog platform event and tenant quota event are accepted by the realtime journal, signalled through real Redis and replayed from their correct scoped cursors |
| W3C trace propagation | Outbox writes, relay envelope and realtime consumer | Active `traceparent` is persisted with the business transaction, carried in the JetStream envelope and extracted as the realtime consumer's parent context |
| Frontend behavior | Existing tenant/platform Playwright suites | No new product UI is introduced by this backend phase; product screens and TanStack Query presentation are owned by Phase 7. The REST/OpenAPI boundary is verified independently here |

## Commands

From the repository workspace root:

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --workspace
```

Focused Phase 3 acceptance:

```bash
cargo test -p bacekend-plataform --test phase3_catalog_entitlements_acceptance
```

The full workspace workflow is authoritative because it also runs the BFF, gateway client, frontend tests/builds and browser E2E.

## Compatibility and operational boundaries

- PostgreSQL is the source of truth for price versions, capability definitions, tenant assignments and usage.
- Redis is not used for quota accounting or authorization decisions. It is used only for realtime wake-up fan-out after the durable journal write.
- JetStream delivery is at-least-once; consumers deduplicate by immutable `event_id`.
- Platform Outbox audience `platform` is translated to the realtime journal's `platform_operator` scope. Tenant events remain tenant-scoped and must carry a valid `tenant_id`.
- Gateway has a typed client for the platform entitlement contract. mTLS identity, signed grants, fail-closed authorization and the live gateway request path are Phase 5 acceptance and must not be claimed complete in Phase 3.
- Legacy usage rows created before the response-snapshot migration cannot be replayed as if the original response were known. They fail explicitly rather than returning current state.

## Architecture debt and closure criteria

The repository file-size rule was revised in PR #8 and is now consistent across the root and both backend architecture rules:

- **Hard limit:** 400 physical lines per Rust source file, counting imports, comments and blank lines.
- **Preferred size:** approximately 200 physical lines when responsibility boundaries and cohesion permit.
- Files are split by responsibility; code is not fragmented artificially to satisfy the preferred size.
- CI enforces the hard limit across `bacekend-plataform`, `bacekend-pasarela` and `crates`.

This PR refactors the oversized identity service, identity gRPC API, platform bootstrap, tenant BFF handler and acceptance-test harnesses into cohesive modules. The acceptance-test scenarios remain executable through the existing Cargo integration-test targets; the test environment continues to use isolated PostgreSQL plus real Redis and authenticated JetStream.

The previous paragraph listing `identity.rs` and `src/bin/migrate_platform.rs` as files modified by this PR was inaccurate: neither path is part of PR #8's diff. The actual changed-file inventory is the PR's GitHub files view and must be used for review.

Phase 3 must remain open until all of the following are true on one final HEAD SHA:

1. Every Rust source file under the platform, gateway and shared-crate source trees is at or below 400 physical lines. The CI guard must pass on the exact final SHA; approximately 200 lines remains the preferred target where cohesion allows.
2. Format, Clippy, all Rust tests, frontend tests/builds and browser E2E are green on that same SHA.
3. The real PostgreSQL/Redis/JetStream acceptance tests pass, including immutable idempotent-result replay and event-specific realtime journal/cursor delivery.
4. The gateway shared-contract client compiles and the BFF HTTP/gRPC authorization path passes.
5. The integration matrix has no unclassified applicable boundary.
6. The PR description, roadmap and this acceptance record identify the same final SHA and workflow run.
7. Deployment/migration execution and health verification for the target environment are recorded; otherwise production readiness remains unverified.

The last code-validation run before this acceptance-record correction passed all CI jobs on SHA `7d96c1f6fbfd9cd37fba8d0ebeda1ec929682650` (workflow run [41565888509](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41565888509)). This documentation change must itself be revalidated before the PR is considered ready.

This document intentionally does not declare Phase 3 closed before all phase-level conditions, including deployment evidence, are satisfied.
