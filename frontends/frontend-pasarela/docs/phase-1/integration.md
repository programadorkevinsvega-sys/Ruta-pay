# Frontend tenant — Fase 1: cliente de identidad

## Diseño y matriz previa

La app consume `POST /v1/identity/register`, `POST|GET|DELETE /v1/identity/session` y `POST /v1/tenants` del BFF. Las vistas tienen rutas explícitas `/register` y `/login`; `/` redirige a `/login`. El alta registra la cuenta, inicia sesión y crea el comercio con nombre, slug derivado del nombre y documento legal (NIT o CC). El tipo/número del documento se guarda en `tenant_registry`; no se devuelve en respuestas, eventos realtime ni logs. Login emite cookies tenant; GET recupera sesión; DELETE revoca con `Origin`, cookie CSRF y header `x-csrf-token`. Las cookies son la única credencial de sesión; nunca se exponen a JS salvo el token CSRF que el BFF diseñó como cookie legible.

| Frontera | Aplicación | Aceptación |
|---|---|---|
| REST/OpenAPI | Cliente tipado, same-origin `/v1`, `credentials: include`, `X-Correlation-ID`; registro envía identidad y comercio al BFF. | UI valida campos; aceptación HTTP→BFF→gRPC→PostgreSQL/Redis real cubre errores y registro correcto. |
| PostgreSQL | Sin acceso desde browser; `identity_access` persiste cuenta/password hash y `tenant_registry` persiste nombre, slug, tipo/número de documento y membresía mediante Outbox. | Migración aditiva; unicidad y validación en backend; documento omitido de vistas/eventos/logs. |
| Redis | No aplica en browser; backend limita login. | 429 mostrado sin reintento automático. |
| NATS/Outbox | Sin acceso en browser; el onboarding usa Outbox/JetStream y el realtime autorizado del BFF. | E2E verifica membresía/evento y redacción del dato legal. |
| Realtime | El backend emite `tenant.registered` sin el documento legal; el coordinador tenant obtiene el estado autorizado por WebSocket y REST/cursor. | Prueba real confirma evento/membresía y verifica que tipo/número del documento no se filtran. |
| Errores/telemetría | No capturar password/cookies/API keys/cuerpo; no persistir credenciales. | Pruebas de errores seguros y ausencia de secretos. |
| Despliegue | API same-origin vía reverse proxy HTTPS `/v1`; proxy Vite local. | Verificar cookies Secure/SameSite en TLS. |

## Estructura de frontend

`src/api.ts` posee el cliente HTTP; `src/App.tsx` presenta las vistas de registro/login y el estado autenticado; `src/realtime.ts` coordina eventos y recuperación con TanStack Query. El frontend no decide permisos ni crea un tenant que el backend no haya autorizado. Las decisiones de rutas y presentación están en el [ADR de registro](../adr/ADR-0001-flujo-y-rutas-de-registro.md).

El flujo de identidad y onboarding integrado cubre inicio/cierre de sesión, recuperación, registro de cuenta y creación de comercio mediante contratos reales del BFF. Registro vive en `/register` y login en `/login`; `/` lleva a `/login`. La contraseña exige 12–128 caracteres, una mayúscula, una minúscula, un dígito y un signo ASCII de puntuación; se confirma antes de enviar y el backend repite la validación. El slug se deriva automáticamente del nombre visible. El documento NIT/CC se valida y persiste en el backend, pero nunca se guarda en el navegador ni se incluye en respuesta/evento/log. Las decisiones separan experiencia frontend y propiedad de datos backend en sus respectivos [ADR](../adr/ADR-0001-flujo-y-rutas-de-registro.md) y [ADR de plataforma](../../../bacekend-plataform/docs/adr/ADR-0009-identidad-legal-del-tenant.md). En desarrollo Vite adapta `Origin` solo en el proxy local y usa `http://localhost:8080`, acorde con la validación del BFF; producción conserva el Origin del navegador sobre HTTPS. La verificación del paquete usa `npm test`, `npm run build` y Playwright contra BFF/servicios reales.

## Criterios de aceptación

- Registro real HTTP y login/logout con cookies y CSRF que provee el BFF; rutas separadas y persistencia de NIT/CC.
- Slug derivado automáticamente del nombre, confirmación de contraseña y la misma política de complejidad en UI/backend.
- Recuperación de sesión GET tras reload; manejo de 401 y 429.
- La app no persiste tokens y no accede a gRPC, bases, Redis, NATS, proveedores o backend de pasarelas.
- Membresías, tenant, API Keys y pagos no se representan como activos hasta que el backend exponga sus contratos reales.
- La fase integrada permanece abierta hasta realtime WebSocket autenticado y recuperación REST/cursor.
