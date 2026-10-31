# Auditoría de pruebas — Fase 2: identidad, tenants, API Keys y realtime

## Alcance

La auditoría cubre las capacidades implementadas en Fase 2: registro e inicio de sesión, sesiones, roles y permisos, tenant registry, aislamiento, API Keys, administración de tenants, Outbox/JetStream, journal realtime y los dos frontends.

## Evidencia

- `tests/identity_access_acceptance.rs` ejecuta aceptación HTTP → BFF → gRPC contra PostgreSQL, Redis y JetStream reales.
- `tests/http_grpc_acceptance.rs` cubre el recorrido fundacional del runtime.
- Las pruebas de dominio cubren validación de correo, contraseñas, API Keys, slug, identificación legal, permisos y transiciones de estado.
- CI ejecuta Format, Clippy, Rust tests, Vitest y builds de ambos frontends, instalación de Chromium y Playwright E2E de tenant, plataforma y recuperación realtime multi-réplica.
- CI #286 aprobó todas esas etapas en el SHA de código `d1a5359f9b43e443d3168cc5c94b2dae4c9728e3`: Format, Clippy, Rust tests, ambos frontends, Chromium/Playwright, E2E tenant, E2E plataforma, recuperación realtime multi-réplica y limpieza.

## Matriz de cobertura actual

| Área | Cobertura demostrada |
|---|---|
| Registro e identidad | Registro válido, correo mal formado, contraseña débil, correo duplicado normalizado y carrera concurrente: exactamente un 201 y un 409, sin identidad ni Outbox duplicados |
| Sesiones | Login incorrecto, cookies tenant/operator, sesión vencida, sesión revocada, sesión del tipo equivocado, Origin y CSRF; las sesiones vencidas se distinguen de las sesiones vigentes sin permiso |
| Roles y actores | Los cuatro roles tenant y los tres roles operator; lectura y API Keys por rol tenant; lectura y ciclo de vida administrativo por rol operator; acceso cruzado entre sesiones tenant/operator |
| Tenant registry | Alta, slug/documento duplicado, carreras de slug e identificación legal, transiciones permitidas y denegadas, motivos y versiones inválidos, control optimista concurrente |
| DTO HTTP | Tipos y límites de credenciales, alta de tenant, API Keys, cambio de estado, paginación y realtime; valores inválidos responden 400/422 sin mutación |
| API Keys | Ambientes, scopes, límites, secreto, listado, creación, revocación, tenant cruzado y rechazo de sesiones no válidas |
| Atomicidad PostgreSQL | Fallos inyectados en registro de identidad, alta de tenant, cambio de estado y creación de API Key; se verifica rollback de filas, historial, Outbox y auditoría |
| Outbox y PostgreSQL | Fallo de claim para ambos módulos sin consumo parcial; lease vencido recuperable y fencing token que impide ACK con token obsoleto |
| Redis | El control de tasa falla de forma segura para registro e inicio de sesión cuando el usuario Redis no puede ejecutar los scripts; no se crea identidad |
| JetStream | Fallo de publicación/ACK y reintento tras restaurar el stream para ambos relays |
| Realtime | Cursor negativo, malformado, fuera de rango y futuro; audiencia y tenant inválidos; hueco retenido probado en HTTP; aislamiento, deduplicación, revocación de sesiones tenant/operator y cierre de streams |
| Frontends | Vitest, builds, Playwright de tenant y plataforma |
| Recuperación distribuida | E2E de recuperación realtime multi-réplica tras pérdida y recuperación de dependencias |

## Correcciones funcionales encontradas durante la auditoría

1. Los cambios de estado ahora comprueban primero `expected_version` bajo bloqueo de fila. Una solicitud concurrente obsoleta se clasifica como conflicto de versión, no como transición de dominio inválida.
2. La autorización de API Keys distingue sesión ausente, vencida o revocada (401) de sesión válida sin membresía/permiso (403).
3. La audiencia realtime desconocida se rechaza como entrada inválida (400), en vez de confundirse con una sesión no autenticada.
4. Los errores de Outbox no dejan mutaciones parciales en los flujos críticos probados.

## Límites de alcance

- Fase 2 no expone todavía endpoints HTTP de ejecución de pagos. Por eso la sintaxis, límites y autorización de scopes se prueban en dominio y API Keys; el consumo efectivo de cada scope corresponde a la fase que incorpore esos endpoints.
- La revocación de autorización cierra la suscripción realtime; no se debe reconectar una sesión revocada. La recuperación multi-réplica se valida por separado cuando Redis/JetStream se recuperan.
- La cobertura de fallos de dependencias se concentra en escrituras críticas y pipeline Outbox/entrega; no pretende inyectar fallos en operaciones de solo lectura que no producen efectos persistentes.

## Criterio de cierre

No se declara cobertura matemática del 100 %. La matriz de comportamiento definida para las rutas implementadas de Fase 2 está cubierta por pruebas de dominio, aceptación real y E2E. CI #286 aprobó el SHA final de código; cualquier cambio posterior de código debe volver a validarse. El PR permanece abierto y sin merge hasta la autorización correspondiente.
