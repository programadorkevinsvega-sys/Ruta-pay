# Fase 0 — Matriz de integración y plan de aceptación

## Alcance de la fase

Fase 0 audita, formaliza decisiones, límites, amenazas y workspace. No crea estado de negocio persistido, mutaciones visibles, usuarios, tenants, pagos, credenciales, contratos RPC ejecutables ni eventos de dominio. La matriz no declara integraciones runtime inexistentes como aprobadas.

| Frontera | Decisión previa a implementación | Aplicación en Fase 0 | Evidencia / condición de aceptación |
|---|---|---|---|
| PostgreSQL/schema/migraciones | Una DB física; ownership por contexto, migraciones coordinadas, prohibido SQL cruzado. | N/A: no se implementa estado de dominio ni schema en esta fase; se fijan ownership y proceso en ADR-0003. | Revisión ADR-0003; migraciones reales pertenecen a la primera subfase que persista datos y se prueban con PostgreSQL real. |
| Frontend → HTTP/REST/OpenAPI → BFF | Dos apps independientes; BFF plataforma es la única frontera REST y no tiene negocio/DB/cache/broker. | N/A runtime: no hay apps fuente, BFF, endpoints ni sesión en el scaffold; ubicaciones documentadas y actuales. | Los contratos se implementan en la fase que crea la operación. Bloquea cerrar Fase 0 si la norma superior exige aceptación runtime desde esta fase. |
| BFF → módulo por gRPC | Cada módulo expone API tipada; context propagation W3C y errores diagnósticos. | N/A runtime: no hay módulo API ni contrato protobuf/tonic actual. ADR-0001/0004 fija versión, identidad y semántica. | Primera verificación real health/readiness en Fase 1 debe ejecutar REST→BFF→gRPC→módulo y probar correlation/trace/error/redacción. |
| Plataforma → pasarelas gRPC | mTLS, identidad rotatoria, autorización por método/scope y grant ligado al comando; fallo cerrado; sin callback circular. | Diseño fijado en ADR-0002; no hay RPC ni certificados configurados. | Conformidad bilateral en fase de autorización; certificado inválido, caller/scope/claims inválidos, replay, timeout, límites y reconexión. |
| Redis | Solo estado técnico/caché efímera con TTL; no autoridad comercial. En realtime, fan-out Pub/Sub no durable. | N/A: no hay datos/cache, quota runtime o sockets multi-réplica que acelerar. | Cuando se incorpore, `ConnectionManager`, backoff y resuscripción; restart test real contra desarrollo, claves aisladas y fallback seguro. |
| NATS JetStream / Outbox | Outbox transaccional dueño; eventos persistidos/versionados, durable, sin secretos; gRPC no se sustituye. | N/A: no se persiste ni ocurre un hecho de dominio de Fase 0 que requiera propagación. | En primera mutación visible: Outbox junto a transacción, consumer durable e idempotencia; prueba PostgreSQL/JetStream real, redelivery/recovery. |
| Realtime / WebSocket / recuperación | Módulo `realtime` de plataforma; scope/cursor durable; BFF autentica y puentea; REST autorizado repara huecos. | N/A: Fase 0 no cambia estado visible ni tiene frontends runtime. Arquitectura y recovery se fijan en ADR-0004. | Primera mutación visible incluye Outbox→JetStream→realtime→WS→TanStack Query→REST cursor/snapshot, o justificación específica previa. |
| Errores / telemetría | Correlación W3C, eventos de inicio/dependencia/fallo/cierre, metadatos seguros y redacción. | No hay runtime donde instrumentar; se registra diseño. No hay exportadores configurados. | En Fase 1 probar evento de fallo y cierre HTTP con el mismo `correlation_id`, `trace_id`, código/categoría y duraciones; prueba anti-leak. |
| Pruebas / infraestructura | No mocks para aceptación; PostgreSQL/Redis/NATS reales aislados cuando aplican; jamás reinicio de producción. | Solo verificaciones estáticas del workspace aplican. No se levantan servicios ni se hacen reinicios. | Registrar comandos y resultados al final. Reinicios futuros requieren documentar antes `.env` de desarrollo, comando y efecto. |

## Conflicto de alcance y resolución

`AGENTS.md`/reglas estrictas exigen en cada fase aceptar el recorrido real frontend→BFF→gRPC→módulo; el roadmap específico define Fase 0 como auditoría y decisiones y reserva el BFF/gRPC/OpenAPI ejecutable para Fase 1. No existe runtime en el scaffold. La solución compatible propuesta es cerrar Fase 0 solo como auditoría/workspace y usar el primer recorrido técnico de health/readiness como la primera subfase de Fase 1, sin afirmar que fue ejecutado en Fase 0. Si prevalece la lectura literal de integración runtime por cada fase, esta Fase 0 queda BLOQUEADA hasta adelantar BFF/gRPC y su aceptación real; el roadmap y las reglas normativas no se modifican para ocultar el conflicto.

## Plan de aceptación posterior

| Área | Aceptación mínima de la primera fase ejecutable | Dependencias reales |
|---|---|---|
| Workspace | `cargo metadata`, `cargo check --workspace`, `cargo fmt --all -- --check`; bins separados; sin workspaces anidados. | toolchain Rust. |
| Health/readiness | HTTP real del BFF → RPC real por tonic → API de módulo; propagation W3C/correlation; errores tipados hasta envelope. | runtime del BFF y módulo. |
| PostgreSQL | migraciones del módulo dueño, aislamiento y rollback; detalles SQLSTATE/constraint seguros. | PostgreSQL desarrollo aislado. |
| Redis | solo si la subfase integra estado efímero; TTL, caída, reconexión/backoff, Pub/Sub resubscribe. | Redis desarrollo declarado. |
| JetStream/Outbox | solo cuando el evento durable corresponda al dominio; persistencia, ack/redelivery, deduplicación, reconnect. | PostgreSQL y NATS desarrollo declarados. |
| Observabilidad | logs JSON, eventos de frontera, OpenTelemetry/Sentry configurados externamente, fallo de exportador degradable, redacción. | collector/Sentry de test aislados según integración. |
| Aceptación cross-backend | caller y certificado válidos/rotados y negativos; autorización grant; límite/deadline/replay/recovery/idempotencia. | ambas aplicaciones y configuración mTLS de desarrollo. |
