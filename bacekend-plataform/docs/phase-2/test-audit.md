# Auditoría de pruebas — Fase 2: identidad, tenants, API Keys y realtime

## Alcance

La auditoría cubre las capacidades realmente implementadas en Fase 2: registro/login, sesiones, roles/permisos, tenant registry, aislamiento, API Keys, administración, Outbox/JetStream, journal realtime, WebSocket y los dos frontends independientes.

## Evidencia encontrada

- `tests/identity_access_acceptance.rs` ejecuta aceptación HTTP→BFF→gRPC contra PostgreSQL, Redis y JetStream reales y aislados.
- `tests/http_grpc_acceptance.rs` cubre el recorrido fundacional de runtime.
- Hay pruebas unitarias de dominio para correo, contraseñas, API Keys, tenant slug, identificación legal y transiciones de estado.
- La aceptación comprueba registro, contraseña débil, login incorrecto, rate limits, alta de tenant, documento duplicado, API Keys, revocación, auditoría, Outbox, JetStream, aislamiento entre tenants, RLS y realtime.
- Este PR incorpora a CI Vitest, builds y Playwright de ambas aplicaciones, además del E2E de recuperación realtime multi-réplica; la aprobación depende del run del commit final del PR.
- Existen suites Vitest y Playwright para ambos frontends, pero el workflow previo solo ejecutaba Rust; la existencia de archivos E2E no equivalía a ejecución CI.
- La matriz de aceptación documenta más escenarios que los que se demostraban en una única ejecución automatizada.

## Matriz de casos

| Caso | Evidencia previa | Trabajo de este PR / estado |
|---|---|---|
| Registro válido | Integración HTTP | Cubierto |
| Password débil | Integración HTTP 400 | Cubierto |
| Correo mal formado | Unitario | Cubierto |
| Correo duplicado normalizado | Constraint UNIQUE y mapeo de conflicto | Añadida aceptación HTTP 409 + integridad de identidad/Outbox |
| Dos registros simultáneos con mismo correo | No había prueba explícita | Añadida aceptación concurrente: exactamente uno 201 y otro 409 |
| Registro rate-limited | Integración Redis/Lua | Cubierto |
| Login con credenciales incorrectas | Integración HTTP 401 | Cubierto |
| Sesión tenant usada como operador | Integración | Cubierto |
| Aislamiento tenant A/B | HTTP 403 y RLS | Cubierto en los flujos existentes |
| Documento legal duplicado | Integración | Cubierto |
| Slug duplicado | Prueba HTTP 409 y comprobación de una sola fila/evento añadida en este PR | CI pendiente |
| Todos los roles × permisos × operaciones | Solo subconjunto demostrado | Pendiente de matriz exhaustiva |
| Sesión expirada/revocada/cookie/CSRF/Origin | Algunos controles documentados y ejercitados | Pendiente de matriz exhaustiva por endpoint |
| API Key scopes vacíos/mal formados | Unitario | Cubierto parcialmente; faltan límites y combinaciones API |
| API Key test/live, secreto no repetido, revocación | Unitario + aceptación | Cubierto en flujos principales |
| Eventos Outbox/JetStream duplicados y reintentos | Aceptación de consumo idempotente | Cubierto en flujo principal; faltan variantes fuera de orden/fallo intermedio |
| Realtime cursor/replay/reconnect/huecos | E2E y aceptación de recuperación | Parcial; faltan matriz completa de cursores, autorización y reconexión |
| Frontend unit/build | Scripts npm disponibles | Deben ejecutarse en CI |
| Browser E2E comercio y administración | Playwright disponible | Deben ejecutarse en CI; la ejecución previa no los incluía |

## Pruebas añadidas en este PR

En `identity_access_acceptance.rs` se agregan dos escenarios de persistencia real:

1. Tras crear una cuenta, intentar registrarla de nuevo con el mismo correo normalizado debe producir HTTP 409 con error seguro `identity_conflict`; la base debe conservar exactamente una identidad y un evento de registro.
2. Dos peticiones simultáneas con el mismo correo deben producir exactamente un HTTP 201 y un HTTP 409, y persistir una sola identidad.

Estas pruebas validan el comportamiento observable y la integridad de persistencia, no solo la existencia del constraint SQL.

## Huecos pendientes

1. Extender las pruebas de unicidad a todos los constraints relevantes y a concurrencia de slug/documento legal.
2. Completar matriz por endpoint de autenticación, autorización, CSRF, Origin, cookie, expiración, revocación y rol.
3. Probar todas las transiciones de estado desde API, incluidos motivos ausentes, actor no autorizado, versión obsoleta y carreras concurrentes.
4. Completar límites numéricos/tamaño/tipo en cada request DTO; los campos que no acepten números negativos deben demostrar 400/422 y no mutar persistencia.
5. Completar combinaciones de scopes y ambientes de API Keys, incluido acceso insuficiente y tenant cruzado.
6. Probar fallos de PostgreSQL/Redis/JetStream en cada operación con garantías de no dejar escrituras parciales.
7. Completar recuperación realtime: cursor inválido, cursor futuro, duplicado, hueco, evento antiguo, cambio de identidad, pérdida de autorización, reconexión y múltiples réplicas.
8. Verificar que Vitest, builds, Playwright de ambas apps y E2E de recuperación multi-réplica pasan en CI; resolver cada fallo antes de integrar.

No se declara 100 % de cobertura funcional. La Fase 2 mantiene pruebas reales valiosas, pero su cierre de calidad requiere completar los huecos enumerados y ejecutar cada suite desde CI.
