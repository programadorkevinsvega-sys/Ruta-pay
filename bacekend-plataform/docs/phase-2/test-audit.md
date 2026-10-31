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
| Slug duplicado | HTTP 409 secuencial y carrera concurrente: exactamente un 201/409, una fila y un evento Outbox | Añadido en este PR; CI del head actualizado pendiente |
| Identificación legal duplicada concurrente | Constraint único y normalización del documento | Añadida carrera concurrente con slugs distintos: exactamente un 201/409, una fila y un evento Outbox |
| Todos los roles × permisos × operaciones | RPC por permiso para 4 roles tenant y 3 operator; HTTP tenant read/API-key list/create/revoke por rol; HTTP operator list y lifecycle allowed/denied | Parcial: faltan todas las operaciones administrativas por cada rol y una matriz exhaustiva endpoint×actor |
| Sesión expirada/revocada/cookie/CSRF/Origin | Expiración tenant, revocación tenant/operator, cookie, CSRF, Origin y cierre de stream realtime tras revocar operador | Parcial: falta cruzar cada endpoint mutante/lectura con ambos tipos de sesión, ausencia/expiración/revocación y Origin/CSRF |
| API Key scopes y autorización | Unitario de sintaxis/límites; autorización gRPC positiva/negativa, ambiente incorrecto, tenant cruzado y clave revocada | Cubierto para la superficie de autorización de Fase 2. No existe endpoint HTTP de consumo de operaciones de pago en esta fase; validar scopes HTTP corresponde a la fase que exponga esas operaciones |
| API Key test/live, secreto no repetido, revocación | Unitario + aceptación | Cubierto en flujos principales |
| Eventos Outbox/JetStream duplicados y reintentos | Outbox atómico, redelivery idempotente y journal deduplicado | Parcial: faltan fallos de publicación/ACK, lease vencido, reintentos y eventos fuera de orden |
| Realtime cursor/replay/reconnect/huecos | Replay desde cursor, cursor actual vacío, cursor futuro HTTP 416, tenant cruzado, eventos de plataforma y cierre de suscripción al revocar operador; tests unitarios de secuencia/gaps y E2E multi-réplica | Parcial: falta cubrir cursor malformado/negativo, gaps/duplicados/antiguos en el borde HTTP, cambio de identidad y reconexión tras pérdida de autorización |
| Frontend unit/build | Vitest y builds de ambas aplicaciones | Workflow CI configurado; falta evidencia verde en el SHA final del PR |
| Browser E2E comercio y administración | Playwright para ambas aplicaciones y recuperación realtime multi-réplica | Workflow CI configurado; falta evidencia verde en el SHA final del PR |

## Pruebas añadidas en este PR

- `identity_access_acceptance.rs`: registra dos solicitudes simultáneas con el mismo slug y dos con la misma identificación legal normalizada; cada carrera debe producir exactamente un HTTP 201 y un HTTP 409, una fila de tenant y un único evento Outbox para el agregado ganador.
- `identity_access_acceptance.rs`: rechaza desde HTTP una transición de estado sin motivo válido (400) y una transición con `expected_version` obsoleta (409), verificando que el estado y la versión persistidos no cambien.
- `api_key.rs`: nombre mínimo/máximo, nombre inválido/control, scopes 1–16, scope de 64 bytes, exceso de longitud/cantidad, valores vacíos, mayúsculas, Unicode y separadores no permitidos; prefijos y secretos test/live distintos.
- `tenant.rs`: matriz completa de 5 estados × 5 estados frente a las ocho transiciones permitidas; razones de 3/500 caracteres; actor operador obligatorio en transiciones sensibles; parseo de estados conocidos/desconocidos.
- `identity_access_acceptance.rs`: comparación exacta de los grants sembrados para los cuatro roles tenant y tres roles operator. Además, se inyecta un fallo controlado en la escritura Outbox durante un registro HTTP y se verifica que no persistan identidad, evento Outbox ni auditoría: la transacción no deja efectos parciales.

En `identity_access_acceptance.rs` también se agregan dos escenarios de persistencia real:

1. Tras crear una cuenta, intentar registrarla de nuevo con el mismo correo normalizado debe producir HTTP 409 con error seguro `identity_conflict`; la base debe conservar exactamente una identidad y un evento de registro.
2. Dos peticiones simultáneas con el mismo correo deben producir exactamente un HTTP 201 y un HTTP 409, y persistir una sola identidad.

Estas pruebas validan el comportamiento observable y la integridad de persistencia, no solo la existencia del constraint SQL.

## Huecos pendientes

1. Las carreras concurrentes de slug e identificación legal ya están añadidas en `identity_access_acceptance.rs`; falta validar el resultado en CI del SHA final y ampliar a cualquier otro constraint único expuesto por la API.
2. Completar matriz por endpoint de autenticación, autorización, CSRF, Origin, cookie, expiración, revocación y rol; la prueba actual de grants no sustituye probar cada operación real con cada rol.
3. Desde API ya se prueban motivos ausentes, versión obsoleta sin mutación y actor no autorizado; falta una carrera concurrente explícita entre cambios de estado con la misma versión esperada y completar la matriz de estados inválidos por endpoint.
4. Completar límites numéricos/tamaño/tipo en cada request DTO; los campos que no acepten números negativos deben demostrar 400/422 y no mutar persistencia.
5. Completar combinaciones de scopes y ambientes de API Keys en la frontera HTTP, incluido acceso insuficiente, clave revocada y tenant cruzado; las validaciones del dominio ya cubren sintaxis y límites.
6. Probar fallos de PostgreSQL/Redis/JetStream en cada operación con garantías de no dejar escrituras parciales.
7. Completar recuperación realtime: cursor inválido, cursor futuro, duplicado, hueco, evento antiguo, cambio de identidad, pérdida de autorización, reconexión y múltiples réplicas.
8. Verificar que Vitest, builds, Playwright de ambas apps y E2E de recuperación multi-réplica pasan en CI; resolver cada fallo antes de integrar.

No se declara 100 % de cobertura funcional. La Fase 2 mantiene pruebas reales valiosas, pero su cierre de calidad requiere completar los huecos enumerados y ejecutar cada suite desde CI.
