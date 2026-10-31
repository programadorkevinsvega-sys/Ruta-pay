# Architecture Roles and Quality Responsibilities

## Purpose

This document assigns responsibilities for architecture decisions, implementation, verification and phase acceptance in Payvia. It complements `ARCHITECTURE_RULES.md` (architectural constraints), `PROJECT_RULES.md` (project-specific rules), the backend roadmaps and `AGENTS.md` (agent execution instructions).

## Roles and accountability

| Role | Owns | Mandatory evidence |
|---|---|---|
| Domain / Product Owner | Use-case intent, actors, business rules, allowed and forbidden outcomes, error semantics | Use-case inventory, acceptance examples, business invariants and explicit decisions |
| Architecture Owner | Bounded-context ownership, trust boundaries, contracts, data ownership, integration decisions and N/A justifications | ADRs, context map, contract decisions, threat/control mapping and integration matrix |
| Implementer | Production code and tests for every changed behavior; input validation, authorization, transactions and safe error mapping | Unit tests, integration/contract/E2E tests at applicable boundaries, migration and API evidence |
| Quality / Test Owner | Independent challenge of positive, negative, boundary, concurrency and failure scenarios; test traceability and gap reporting | Test matrix, test IDs, execution reports, coverage and mutation/negative-test evidence where available |
| Security Reviewer | Authentication, authorization, tenant isolation, secrets, replay, abuse limits, privacy and failure-closed behavior | Security test scenarios, findings, residual risks and review approval for sensitive changes |
| CI / Release Owner | Reproducible automation, required checks, phase gates and evidence integrity | Workflow results for exact commit, required job/step status, release checklist and rollback plan |
| AI / Automation Agent | Follow normative docs, inspect actual repository state, implement authorized work and report evidence honestly | Branch/PR, exact commands and outcomes, known gaps, no unsupported completion claims |

A person or agent may hold multiple roles in a small team, but the responsibilities and evidence remain distinct. Security-sensitive changes still require the review defined by project rules; an agent must not self-approve or merge them.

## Mandatory use-case verification

Every implemented use case in every phase/subphase must have a traceable test matrix before its phase can be accepted. The matrix must include applicable:

1. Valid/positive path and expected persisted state.
2. Invalid/missing/null/malformed values and wrong types.
3. Boundary partitions: minimum, maximum, below/above range, zero, negative values, excessive length, precision and overflow.
4. Duplicate and idempotency behavior, including normalized identifiers and concurrent requests.
5. Authentication and authorization failures, including missing, invalid, expired and revoked credentials, wrong roles/scopes and cross-tenant access.
6. Valid and invalid state transitions, stale versions and concurrent updates.
7. Dependency failures, timeout, disconnection, retry and recovery.
8. Data integrity: transaction rollback, no partial writes, no duplicate side effects, durable audit and Outbox behavior.
9. Privacy and observability: safe responses, logs, traces and metrics without credentials or unnecessary personal data.

Choose the test level that proves the behavior: unit/property for deterministic domain rules, integration against real owned dependencies for persistence and coordination, contract tests for service boundaries, and browser E2E for user journeys. Mocks may isolate unit tests but never replace required real integration acceptance.

For numeric inputs, the matrix must explicitly decide whether negative and zero values are allowed. When disallowed, prove rejection at the server/API boundary and prove that no persistent or external side effect occurred. For uniqueness constraints, prove sequential and concurrent duplicate requests, stable conflict mapping, and exactly one durable effect.

## Phase acceptance and coverage semantics

- Each test case has an identifier, requirement/use-case reference, expected result, test implementation path, test level and latest execution evidence.
- Every N/A is justified before implementation with the owner and future verification phase; it cannot be used to defer a test for behavior already implemented.
- CI runs all applicable tests, frontend builds, unit suites, integration suites and browser E2E required by the affected phase.
- A phase cannot be marked complete when a required test is missing, not executed, failing, flaky without disposition, or supported only by a document claim.
- Code coverage percentages are diagnostic, not proof of functional completeness. “100% of defined use cases” means every documented use case and applicable scenario partition is mapped to passing evidence; it does not mean every possible byte string or input combination has been exhaustively enumerated.
- Phase 8 adds cross-cutting security hardening, fuzzing, load, performance, saturation and resilience campaigns. It does not replace or postpone use-case tests required in earlier phases.

## Change control

The phase roadmap owns phase-specific scope and acceptance. `PROJECT_RULES.md` owns the mandatory project policy. `AGENTS.md` tells agents how to apply it. If these documents conflict, stop the affected work, identify the conflict and resolve it through a reviewed documentation change before proceeding.
