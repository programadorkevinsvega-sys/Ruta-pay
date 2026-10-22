# Fase 2 — Identidad, tenant registry y aislamiento

## Estado

En implementación en `feat/platform-phase-2-identity-tenants`. Esta matriz guía la implementación y el cierre. No se marcará la fase completa hasta cubrir los criterios de salida automatizados y las dependencias reales.

## Alcance vertical

La fase entrega registro/autenticación diferenciados para usuarios de tenant y operadores, sesiones revocables, membresías/roles/permisos, revisión y ciclo de vida de tenants, y ciclo de vida de API Keys `test`/`live`. La superficie de ambas aplicaciones React usa exclusivamente REST/OpenAPI del BFF. Las mutaciones visibles se registran en Outbox, se distribuyen por JetStream y llegan a las aplicaciones por WebSocket autorizado; Redis solo aporta rate limit efímero y fan-out.

### Decisiones de seguridad y dominio

- `identity_access` posee usuarios de tenant, cuentas de operador, sesiones, membresías, roles/permisos, API Keys, autenticación y rate limit de acceso. `tenant_registry` posee el agregado tenant y sus estados. Ninguno consulta tablas del otro módulo.
- Cuentas de operador y de tenant se almacenan en conjuntos de tablas separados y usan audiencias/sesiones diferentes. Una sesión tenant nunca concede acceso de operador ni al contrario.
- Las contraseñas se almacenan con Argon2id. Los tokens de sesión son aleatorios, opacos, expirables y revocables; PostgreSQL conserva su digest, nunca el token. Las cookies son `HttpOnly`, `Secure` en despliegue TLS, `SameSite=Lax`; las mutaciones validan origen y token CSRF.
- Una API Key contiene 256 bits aleatorios, un prefijo público `test_` o `live_`, identificador y scopes. PostgreSQL guarda solo SHA-256 del secreto, tenant, ambiente, scopes, estado y fechas. El secreto completo se devuelve una vez y no aparece en eventos/logs/errores. Esta fase administra las llaves; su uso para payment intents queda en la fase que implementa esa API.
- El alta crea un tenant en `pending_verification`. La verificación es revisión explícita por operador con motivo auditable; no depende de correo ni de un adapter Resend aún no implementado. Solo un operador con permiso específico puede activar/suspender/cerrar; el tenant cerrado no se reactiva mediante una operación genérica.
- El primer operador se crea mediante una operación local de bootstrap de un solo uso que exige una instalación sin operadores existentes y no publica credenciales en una ruta pública. No se insertan usuarios de demo o fixtures.
- Todas las consultas tenant se filtran por contexto autorizado, usan claves/constraints compuestas y políticas RLS como defensa en profundidad. El rol runtime no será superusuario ni propietario bypass de las tablas sujetas a RLS.
- Los errores de autenticación evitan enumerar cuentas. Los errores internos preservan diagnóstico seguro; jamás incluyen passwords, cookies, sesiones, API Keys completas, CSRF tokens ni datos personales innecesarios.

## Matriz de integración obligatoria

| Frontera | Diseño e implementación de Fase 2 | Aceptación requerida |
|---|---|---|
| PostgreSQL/migraciones | Una base física compartida; schemas/migraciones independientes de `identity_access`, `tenant_registry` y `realtime`. Cada módulo conserva una tabla SQLx de historial propia para que el runner no confunda versiones pertenecientes a otros módulos; el despliegue aplica coordinadamente las migraciones versionadas. Transacciones locales por módulo; relaciones tenant con claves compuestas, RLS forzada y rol runtime de mínimo privilegio. Outbox en el mismo commit que cambios visibles. | La aceptación crea una base temporal aislada en el PostgreSQL de desarrollo ya iniciado, ejecuta las migraciones reales con historial separado por módulo, crea un rol runtime sin `BYPASSRLS` y elimina la base temporal al terminar. Los registros de prueba nacen por HTTP→BFF→gRPC→módulo; no se usan fixtures ni se crean contenedores en el test. |
| Frontend tenant → REST/OpenAPI → BFF | `frontends/frontend-pasarela` tiene app React/Vite y cliente REST con login/logout/recuperación de sesión. No tiene onboarding de tenant, membresías, selección de tenant ni API Keys UI; no se presentan como activas. | `npm test` y `npm run build` pasan. Pendiente flujo automatizado browser/app contra BFF real, CSRF en navegador, tenant context/memberships, API Key de una sola lectura y realtime. |
| Frontend administración → REST/OpenAPI → BFF | `frontends/frontend-plataform` tiene app React/Vite y cliente REST con login/logout/recuperación de sesión de operador. Búsqueda y ciclo de vida de tenants aún no cuentan con handlers BFF/runtime. | `npm test` y `npm run build` pasan. Pendiente aceptación browser/app contra BFF real, autorización de roles y operaciones administrativas reales. |
| BFF → módulos gRPC | `identity_access` y `tenant_registry` exponen servicios/versiones Protobuf propios; BFF solo enruta/valida transporte/mapea errores. Aserciones de identidad/contexto se verifican en el módulo dueño. | HTTP→BFF→gRPC→módulo real; propagación W3C/correlation, authn, autorización, deadline, errores tipados y redacción. |
| Redis | `identity_access` usa contadores `INCR`/`EXPIRE` de 60 s, segmentados por IP y digest SHA-256 codificado de cuenta; más de 10 intentos por cuenta o 50 por IP se rechazan, y Redis no disponible falla cerrado. `ConnectionManager` reintenta conexión con backoff limitado. No almacena roles, sesiones autorizadas ni estado de tenant. `realtime` usará Pub/Sub solo para fan-out efímero multi-réplica. | El flujo real ya verifica con Redis de desarrollo los umbrales por cuenta e IP y que ambos contadores tengan TTL 1–60 s; siguen pendientes fallo cerrado ante Redis no disponible y recuperación tras reinicio. Re-suscripción corresponde cuando se implemente Pub/Sub realtime. Reinicios solo después de registrar comando/impacto. |
| NATS JetStream/Outbox | Outbox por módulo dueño para `identity.tenant_user.registered`, `identity.operator.bootstrap.completed`, `identity.session.revoked`, `tenant.registered`, `tenant.status.changed`, `membership.changed` y `api_key.status.changed`. Eventos versionados contienen IDs/scope/metadatos mínimos, nunca secretos o credenciales. Consumer durable de realtime persiste journal/cursor e idempotencia. | La aceptación verifica eventos de registro, bootstrap y revocación de sesión junto con su auditoría en PostgreSQL; el relay, publicación a JetStream, consumers durables, redelivery/restart y deduplicación siguen pendientes. |
| Realtime/recuperación | Gateway modular `realtime`, BFF WebSocket con autenticación y audiencia tenant/operator separadas, cursor durable, deduplicación/orden, reconexión y recuperación REST autorizada. Redis Pub/Sub solo distribuye a réplicas; PostgreSQL/journal y JetStream son durables. | Cambios de tenant, membresía y API Key aparecen únicamente en scope autorizado; cubrir huecos, cursor inválido, re-conexión, réplica y recuperación REST. |
| Inicialización del operador | Binario local `bootstrap-platform-operator`, limitado al primer operador, con password confirmado e introducido sin eco, hash Argon2id, auditoría y Outbox. No se publica endpoint anónimo ni se guarda un password por defecto. La sesión de operador usa cookies y validación gRPC separadas de tenant. | La aceptación invoca el servicio real con el rol runtime de la base temporal, confirma bootstrap único, login/logout/replay y rechazo cruzado de audiencias. La interacción TTY del binario no se ejecuta en CI; el binario debe compilar. No se pasan secretos como argumentos/logs. |
| Proveedores externos | Ninguno requerido: revisión de tenant es decisión administrativa. Resend corresponde a una fase posterior. | Verificar que ninguna operación dependa de credenciales sandbox ni prometa correo no disponible. |
| Observabilidad/errores | Eventos de entrada, llamada RPC/DB/cache/broker, fallo causal y respuesta final con `correlation_id`, `trace_id`, duración, código/categoría y metadatos técnicos seguros. | Cada aceptación de error verifica evento causal + cierre HTTP correlacionados y código/categoría preservados; pruebas anti-leak de logs, spans, métricas, NATS y respuestas. |

## Flujos de aceptación

1. Crear usuario tenant, crear tenant pendiente, observar el evento durable y completar la membresía propietaria idempotente sin transacción SQL entre módulos.
2. Login tenant y operador con cookies/audiencias distintas; logout, expiración, revocación, CSRF, origen inválido, cuenta inválida y límite de intentos.
3. Operador verifica y activa tenant con permiso y motivo; suspensión/cierre revocan acceso en cada request y generan auditoría/evento.
4. Usuario autorizado crea API Key `test` o `live`, ve el secreto una vez, consulta metadata sin secreto, revoca y demuestra que la llave revocada no se puede usar/consultar. Scopes/env/tenant no se pueden alterar.
5. Dos tenants y dos operadores independientes intentan leer, modificar, enumerar, revocar y recibir eventos de recursos ajenos. Se verifica directamente PostgreSQL con credenciales runtime de mínimo privilegio.
6. REST y WebSocket reanudan desde cursor tras desconexión, duplicación, evento fuera de orden y reinicio de Redis/NATS; JetStream recupera pendientes y REST repara huecos.

## Errores que deben probarse

Validación de campos/email/password/scope; autenticación inválida, expirada o revocada; autorización denegada y tenant cruzado; conflicto de email/nombre; estado de tenant inválido; rate limit; timeout/conectividad de gRPC/PostgreSQL/Redis/NATS; constraint/persistencia; replay/redelivery; fallo de realtime y cursor. Los errores no aplicables a una operación específica se justifican en su prueba/matriz y no se simulan.

## Criterios de cierre

- Todos los ítems de Fase 2 del [roadmap](../../ROADMAP.md) implementados con ownership DDD, Clean Architecture/Hexagonal, contratos y migraciones reales.
- Ambas aplicaciones frontend compilan y recorren sus flujos contra el BFF; no acceden directamente a módulos o almacenes.
- Unitarias, integración y aceptación/E2E automatizadas ejecutadas con PostgreSQL, Redis y NATS reales de desarrollo cuando aplican; sin fixtures/mock que sustituyan dependencias.
- Pruebas de aislamiento verifican consultas y datos persistidos, roles y scopes, revocación, evento/realtime y recuperación.
- `cargo fmt`, Clippy, tests, builds de frontend, análisis y controles aplicables pasan; se registran comandos/resultados.
- Evidencia actualizada aquí; estado del README, roadmap de backend y roadmaps de ambos frontends consistente.

## Evidencia ejecutada hasta el corte actual

- `cargo fmt --all -- --check`: aprobado tras aplicar formato al test de recuperación de sesión.
- `cargo clippy --workspace --all-targets -- -D warnings`: aprobado.
- `bash -lc 'set -a; source bacekend-plataform/.env; set +a; cargo test --workspace'`: aprobado con 26 pruebas unitarias e integración, incluyendo recuperación de sesión tenant.
- Desde cada directorio frontend: `npm test` (3 pruebas) y `npm run build`: aprobados para ambas aplicaciones; `npm install` reportó cero vulnerabilidades en ambos paquetes.
- `identity_access_acceptance` usa los servicios PostgreSQL y Redis de desarrollo reales; crea una base aleatoria temporal, aplica los migradores reales con historiales separados en los schemas dueños, crea rol de runtime sin `BYPASSRLS`, recorre HTTP→BFF→gRPC, verifica registro/outbox/auditoría, Argon2id, sesión tenant y sesión operador separadas, cookies, CSRF, logout/replay, bootstrap único con privilegios de runtime, rechazo cruzado de audiencias, Outbox/auditoría de revocación, límites Redis por cuenta e IP y TTL 1–60 s, correlación de errores y ausencia de credenciales en logs; después elimina base y rol temporales. La prueba de rate limit elimina únicamente el contador IP con digest exclusivo que creó para esta ejecución antes de medir el umbral. La verificación funcional de aislamiento RLS entre dos tenants sigue pendiente porque todavía no hay flujo real de tenant onboarding/membresías.
- No se iniciaron ni reiniciaron contenedores. La aceptación no verifica aún Redis inaccesible/recuperado, NATS/JetStream, relay Outbox, WebSocket, membresías/tenant onboarding, operaciones de UI de operadores ni aislamiento completo entre dos tenants. Ambas apps tienen únicamente pantallas de identidad; no se probó un recorrido browser/app contra servicios reales. El bootstrap se probó mediante el servicio en la base temporal; la lectura TTY del binario no se ejecutó.

Este es un corte parcial de Fase 2 y no constituye su acta de cierre.
