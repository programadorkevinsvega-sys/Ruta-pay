# Frontend de plataforma — Fase 1: cliente de sesión

## Diseño y matriz previa

La app consume solo `GET|POST|DELETE /v1/platform/identity/session` del BFF. `GET` recupera la identidad de servidor; `POST` inicia login; `DELETE` envía `Origin`, cookie CSRF y header `x-csrf-token`. Las cookies permanecen HttpOnly/Secure y el cliente no obtiene ni almacena el token de sesión.

| Frontera | Aplicación | Aceptación |
|---|---|---|
| REST/OpenAPI | Cliente tipado, same-origin `/v1`, `credentials: include`, `X-Correlation-ID` aleatorio; mensajes solo del envelope seguro. | Componente consulta sesión real y login/logout contra el BFF. |
| PostgreSQL | Solo backend; la UI no accede al storage. | Backend acepta con rol runtime y PostgreSQL temporal. |
| Redis | No aplica en browser; backend limita login. | 429 visible sin retry automático ni datos privados. |
| NATS/Outbox | No aplica en browser. Bootstrap/revocación emiten Outbox backend. | Sin consumer/gateway actual; queda bloqueada la sincronización durable. |
| Realtime | Bloqueo previo documentado: no existe WebSocket ni REST de cursor/snapshot en BFF. La UI no simulará realtime ni polling. | No cerrar la fase integrada hasta implementar y probar ambas fronteras. |
| Errores/telemetría | No registrar credentials, cookies, response de login ni PII; mostrar correlation ID. | Prueba unitaria de envelope/redacción y aceptación HTTP real del backend. |
| Despliegue | `/v1` same-origin; Vite proxy solo en desarrollo. TLS termina en el proxy de despliegue. | Verificar cookie `Secure`/`SameSite` detrás del proxy. |

## Estructura de frontend

`src/api` contiene el cliente REST sin persistencia; `src/auth` será dueño del estado de sesión; TanStack Query es el cache de servidor; `src/app` compone vistas accesibles de login y sesión. Los componentes no poseen decisiones de autorización ni APIs de dominio.

## Criterios de aceptación

- Build estricto de TypeScript y React en Vite; linting de tipos y pruebas de API segura.
- Sesión recuperada mediante GET, login con cookies del navegador, logout con CSRF y cierre del estado local al recibir 401.
- La app no llama gRPC, PostgreSQL, Redis, NATS, pasarelas ni proveedores y no escribe credenciales en storage.
- La fase de interfaz integrada permanece abierta hasta que exista realtime WebSocket autenticado y recuperación REST/cursor.
