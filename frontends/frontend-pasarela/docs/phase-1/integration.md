# Frontend tenant — Fase 1: cliente de identidad

## Diseño y matriz previa

La app consume `POST /v1/identity/register`, `POST|GET|DELETE /v1/identity/session` del BFF. Registration crea cuenta; login emite cookies tenant; GET recupera sesión; DELETE revoca con `Origin`, cookie CSRF y header `x-csrf-token`. Las cookies son la única credencial de sesión; nunca se exponen a JS salvo el token CSRF que el BFF diseñó como cookie legible.

| Frontera | Aplicación | Aceptación |
|---|---|---|
| REST/OpenAPI | Cliente tipado, same-origin `/v1`, `credentials: include`, `X-Correlation-ID`; feedback del envelope seguro. | Cliente y componentes contra contratos actuales; aceptación backend HTTP→BFF→gRPC→PostgreSQL/Redis real. |
| PostgreSQL | Sin acceso desde browser; `identity_access` persiste las cuentas. No hay tabla ni API de tenant owner/memberships todavía. | No simular tenant context; el recorrido permanece bloqueado hasta onboarding/membresías. |
| Redis | No aplica en browser; backend limita login. | 429 mostrado sin reintento automático. |
| NATS/Outbox | Sin acceso en browser; el registro y la revocación ya generan Outbox. | No hay relay/consumer/gateway; sincronización durable pendiente. |
| Realtime | Bloqueo concreto: el BFF aún no ofrece WebSocket ni REST con cursor/snapshot. No usar polling o fake socket. | No cerrar fase integrada hasta completar backend gateway y coordinador UI. |
| Errores/telemetría | No capturar password/cookies/API keys/cuerpo; no persistir credenciales. | Pruebas de errores seguros y ausencia de secretos. |
| Despliegue | API same-origin vía reverse proxy HTTPS `/v1`; proxy Vite local. | Verificar cookies Secure/SameSite en TLS. |

## Estructura de frontend

`src/api` posee el cliente HTTP; `src/auth` consulta/actualiza la sesión en TanStack Query; `src/app` contiene vistas accesibles de registro/login y el estado de cuenta autenticada. El frontend no decide permisos ni crea un tenant que el backend no haya autorizado.

La primera entrega ejecutable cubre inicio de sesión, recuperación de sesión y cierre con CSRF mediante `/v1/identity/session`. No habilita registro de comercio, selección de tenant ni API Keys mientras el backend no exponga el contrato real de onboarding y membresías. La verificación del paquete será `npm test` y `npm run build` desde este directorio. La matriz no considera cumplida la sincronización realtime: continúa bloqueada por gateway WebSocket y recuperación REST por cursor aún no implementados.

## Criterios de aceptación

- Registro real HTTP y login/logout con cookies y CSRF que provee el BFF.
- Recuperación de sesión GET tras reload; manejo de 401 y 429.
- La app no persiste tokens y no accede a gRPC, bases, Redis, NATS, proveedores o backend de pasarelas.
- Membresías, tenant, API Keys y pagos no se representan como activos hasta que el backend exponga sus contratos reales.
- La fase integrada permanece abierta hasta realtime WebSocket autenticado y recuperación REST/cursor.
