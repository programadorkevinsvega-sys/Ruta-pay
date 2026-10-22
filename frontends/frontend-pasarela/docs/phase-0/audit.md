# Fase 0 — Auditoría frontend-pasarela

## Estado

El estado anterior era documental: no había `package.json`, lockfile, componentes, rutas, toolchain ni pruebas.

## Contrato disponible confirmado

- `POST /v1/identity/register`: registra una cuenta tenant.
- `POST /v1/identity/session`: inicia sesión tenant y emite cookies de audiencia tenant.
- `GET /v1/identity/session`: valida la sesión tenant al recargar y devuelve solo el ID de usuario.
- `DELETE /v1/identity/session`: valida sesión, Origin y CSRF y revoca.
- Endpoints para crear tenant, alta de propietario, membresías y perfil tenant aún no están publicados por el backend. Por ello no se implementa selector de tenant, API Keys, pagos ni configuración de pasarelas.

## Matriz previa de integración de Fase 1

| Frontera | Decisión de implementación | Aceptación |
|---|---|---|
| Frontend→REST/OpenAPI | Cliente solo para registro/login/logout existentes; rutas relativas `/v1`, cookies `credentials: include`; proxy Vite en desarrollo. | Unitarias/componentes y flujo real backend con PostgreSQL/Redis. |
| Tenant/autorización | La UI no interpreta IDs ni crea estado tenant que el backend aún no emite. | Los endpoints futuros deben validar membresía y tenant en el módulo dueño. |
| PostgreSQL | Sin acceso desde navegador; backend identity/tenant posee tablas y migraciones. | Aceptación backend real. |
| Redis | Sin cliente Redis en browser; el backend aplica rate limiting y TTL. | Mensaje claro y retry-after al responder 429. |
| NATS/Outbox | Sin cliente NATS en browser; eventos los entrega el BFF. | Eventos no están disponibles todavía. |
| Realtime/recuperación | Bloqueado: no hay WebSocket BFF ni REST con cursor/snapshot. No se sustituye con polling. | No cerrar la fase de interfaz integrada hasta tener gateway y recuperación reales. |
| CORS/despliegue | Same-origin detrás de proxy inverso HTTPS `/v1`; Vite proxy en desarrollo. | Cookies Secure/SameSite verificadas en despliegue TLS. |
| Errores/telemetría | Envelope seguro, correlation ID; nunca password, cookies, API keys ni cuerpos de pago en logs. | Pruebas automáticas de redacción y errores. |

## Amenazas frontend

- XSS y lectura de cookies: nada de HTML arbitrario ni persistencia de credenciales en storage.
- CSRF: usar doble submit/origen que exige el BFF y no crear mutaciones sin contrato.
- Aislamiento tenant/operator: sesión tenant únicamente; el backend es autoridad de membership.
- Rate limit/errores: no reintentar login automáticamente; presentar el código seguro del BFF.
- Realtime sin autorización: no abrir socket hasta que exista contrato autenticado y recuperación REST.

## Salida

El inventario confirmó el contrato y registró GET sesión como gap; el BFF ya expone ese contrato real y la prueba se documenta en `../phase-1/integration.md`. Endpoints tenant restantes siguen bloqueados. Esta auditoría no declara completa ninguna fase de producto.
