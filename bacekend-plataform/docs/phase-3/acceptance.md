# Fase 3 — acta de aceptación y verificaciones

## Alcance

Esta acta acompaña el PR #7 y documenta pruebas de integración ejecutables sobre PostgreSQL y JetStream reales. La evidencia de CI debe asociarse siempre al SHA exacto del HEAD del PR; un run verde de un SHA anterior no valida cambios posteriores.

## Entorno requerido

El test de aceptación crea una base de datos y un rol runtime aislados, aplica las migraciones reales de los módulos y elimina los recursos al finalizar. Requiere PostgreSQL con permisos para crear/eliminar bases y roles, y JetStream con autenticación:

- `PLATFORM_TEST_DATABASE_ADMIN_URL`: URL administrativa PostgreSQL.
- `PLATFORM_NATS_URL`: URL NATS.
- `NATS_USER` y `NATS_PASSWORD`: credenciales NATS/JetStream.

En CI, el workflow `.github/workflows/platform-quality.yml` provisiona PostgreSQL y JetStream autenticado y ejecuta `cargo test --workspace`.

## Matriz de aceptación

| Criterio | Verificación automatizada | Evidencia / estado |
|---|---|---|
| Precio/versiones inmutables | `immutability.rs`: UPDATE y DELETE de `price_versions` deben fallar con SQLSTATE `55000` | Prueba añadida; CI del HEAD pendiente |
| Capacidades inmutables | `immutability.rs`: UPDATE y DELETE de `plan_capabilities` deben fallar con SQLSTATE `55000` | Prueba añadida; CI del HEAD pendiente |
| Ledger append-only | `immutability.rs`: UPDATE y DELETE de `usage_ledger` deben fallar con SQLSTATE `55000` | Prueba añadida; CI del HEAD pendiente |
| Atomicidad catálogo | `atomicity.rs`: una falla forzada en Outbox revierte versión de precio y auditoría | Prueba añadida; CI del HEAD pendiente |
| Atomicidad consumo | `atomicity.rs`: una falla forzada en Outbox revierte ledger y auditoría | Prueba añadida; CI del HEAD pendiente |
| Entrega JetStream catálogo | `jetstream.rs`: relay real, ACK de publicación y lectura del envelope con event ID/tipo/correlation ID | Prueba añadida; CI del HEAD pendiente |
| Entrega JetStream entitlement | `jetstream.rs`: relay real, ACK de publicación y lectura del envelope con event ID/tipo/correlation ID | Prueba añadida; CI del HEAD pendiente |
| Cuotas y límites exactos | `quota.rs`: consumo inferior, exacto, excedente, overage, entorno e idempotencia | Cubierto por prueba existente |
| Concurrencia | `concurrency.rs`: dos consumos concurrentes compiten por cuota limitada; solo uno se confirma | Cubierto por prueba existente |
| Versionado y fechas | `versioning.rs`: versión futura, vigencia anterior/nueva y snapshot de capacidades | Cubierto por prueba existente |
| Aislamiento multi-tenant / RLS | `isolation.rs`: tenant no asignado y visibilidad de ledger/asignaciones ajenas | Cubierto por prueba existente |
| BFF/HTTP/gRPC y autorización de consumo | `identity_access_acceptance.rs`: recorrido HTTP, BFF, gRPC y API key con scope `usage.consume` | Cubierto por prueba existente; no sustituye autorización de gateway de Fase 5 |

## Comandos

Desde la raíz del workspace:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Para ejecutar solo la aceptación de catálogo/cuotas:

```bash
cargo test -p bacekend-plataform --test phase3_catalog_entitlements_acceptance
```

En CI se usa `cargo test --workspace` como comando autoritativo.

## Criterio de cierre

No declarar Fase 3 cerrada hasta que:
1. Format, Clippy, todas las pruebas Rust, pruebas frontend y E2E del workflow estén verdes sobre el SHA actual del PR.
2. La ejecución de esta aceptación demuestre PostgreSQL y JetStream reales, incluyendo las nuevas verificaciones de integridad, rollback y entrega.
3. Los contratos y endpoints HTTP/BFF/gRPC existentes sigan pasando.
4. La matriz se actualice con el enlace y SHA de CI definitivo, sin convertir una prueba añadida pero aún no ejecutada en evidencia aprobada.

Las casillas del roadmap permanecen abiertas hasta confirmar los puntos anteriores. Esta acta no declara aprobada la Fase 3 por el mero hecho de incorporar los tests.
