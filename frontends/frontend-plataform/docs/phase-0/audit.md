# Fase 0 — Auditoría frontend-plataform

## Estado

Auditoría del estado previo a la primera implementación. El frontend no tenía `package.json`, lockfile, aplicación, rutas ni pruebas; el README y roadmap describían el objetivo, no una app ejecutable.

## Contrato disponible confirmado

- `POST /v1/platform/identity/session`: inicia sesión de operador y emite cookies HttpOnly/Secure de audiencia plataforma y cookie CSRF legible por la app.
- `GET /v1/platform/identity/session`: valida la sesión de operador y devuelve el identificador del operador.
- `DELETE /v1/platform/identity/session`: valida origen, CSRF y audiencia antes de revocar.
- Contrato REST/OpenAPI servido por el BFF de `bacekend-plataform`; no se inventa un endpoint frontend propio.
- Tenant registry administrativo aún no tiene rutas HTTP/gRPC ejecutables; búsqueda y cambios de ciclo de vida están bloqueados hasta que el backend dueño los publique.

## Matriz previa de integración de Fase 1

| Frontera | Decisión de implementación | Aceptación |
|---|---|---|
| Frontend→REST/OpenAPI | Cliente tipado limitado a la sesión de operador confirmada; usa rutas relativas `/v1` y cookies `credentials: include`. Vite proxy solo para desarrollo. | Tests de cliente/React y aceptación backend real de sesión. |
| Identidad y permisos | Estado de login solo representa sesión validada por `GET`; nada de roles inferidos en browser. | Login, sesión vigente, error, logout, expiración y aislamiento tenant/operator. |
| PostgreSQL | No aplica directamente al navegador; `identity_access` conserva el ownership y la persistencia. | Prueba backend usa PostgreSQL real temporal. |
| Redis | No aplica directamente al navegador; backend administra rate limit efímero. | Suite backend valida límites y TTL; frontend maneja 429 sin filtrar datos. |
| NATS/Outbox | No aplica directamente al navegador; el cliente no importa SDK ni credenciales. | Los eventos durables se integrarán al llegar el gateway realtime. |
| Realtime y recuperación | Bloqueado: el BFF aún no expone WebSocket ni REST de cursor/snapshot. No se implementa un transporte sustituto ni polling. | Fase de UI no se cierra mientras falte el contrato realtime transversal exigido. |
| CORS/despliegue | Usar same-origin mediante proxy inverso HTTPS que enrute `/v1` al BFF; en desarrollo, proxy de Vite. No se guardan tokens en storage. | Verificar cookies Secure/SameSite detrás del proxy antes de publicar. |
| Errores/telemetría | Mostrar código y mensaje seguro del envelope junto a `correlation_id`; no capturar body/password/cookies en telemetría. | Pruebas de mensajes, expiración y redacción del cliente. |

## Amenazas frontend

- XSS que lea la cookie CSRF o use la sesión: React escapa texto; no se usa HTML sin sanear ni `localStorage`.
- CSRF en mutaciones: el BFF valida Origin, cookie CSRF y header doble submit.
- Confusión de audiencia: el cliente de plataforma usa solo rutas `/v1/platform/identity/*`; la API vuelve a validar el sujeto.
- Fuga por observabilidad: no registrar formulario, password, cookie, response de login ni datos del operador.
- Enlaces/redirecciones: no aceptar URL de retorno arbitraria.

## Salida

Inventario y contrato de sesión comprobados contra el código actual del BFF. La auditoría no declara completada la Fase 0 del producto; es evidencia de alcance del frontend para su primera implementación. El realtime sigue siendo un bloqueo explícito.
