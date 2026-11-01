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

## Existing architecture debt and closure criteria

The Phase 3 modules and newly extracted acceptance/realtime/event-worker files are split below the mandatory 200-lines-of-code limit. The repository still contains pre-existing oversized files that this PR touches for wiring or shared setup: `apps/bff/src/http/handlers/identity.rs`, `src/main.rs`, `src/bin/migrate_platform.rs`, `tests/http_grpc_acceptance.rs` and `tests/identity_access_acceptance.rs`. Their full SRP/200-line refactor is not complete and must not be represented as repository-wide rule compliance.

Phase 3 must remain open until all of the following are true on one final HEAD SHA:

1. Every `.rs` file modified by this PR, including inherited handlers, bootstrap and integration tests, is refactored to no more than 200 lines of code. The oversized files listed above currently block this criterion; tracking them in documentation is not a waiver.
2. Format, Clippy, all Rust tests, frontend tests/builds and browser E2E are green.
3. The real PostgreSQL/Redis/JetStream acceptance test passes, including immutable result replay and event-specific realtime journal/cursor delivery.
4. The gateway shared-contract client compiles and the BFF HTTP/gRPC authorization path passes.
5. The integration matrix has no unclassified applicable boundary.
6. The PR description, roadmap and this acta reference the same final SHA and workflow.
7. Deployment/migration execution and health verification for the target environment are recorded; otherwise production readiness remains unverified.

This document intentionally does not declare Phase 3 closed before those conditions are evidenced.
