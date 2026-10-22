# Fase 2 — Identidad, tenant registry y aislamiento

## Estado

Cierre técnico completado en `feat/platform-phase-2-identity-tenants`. La evidencia de esta ejecución está registrada al final de este documento; la integración a `main` sigue el proceso de revisión de rama y Pull Request.

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

### Subfase implementada: onboarding, operaciones administrativas, entrega de eventos y E2E

El BFF permite HTTP solo para orígenes loopback exactos durante desarrollo (`localhost`, `127.0.0.1`, `::1`); en despliegue exige coincidencia HTTPS exacta y rechaza sufijos de host parecidos.

Esta subfase cerró los huecos de Fase 2. El diseño y los criterios siguientes forman la matriz normativa y se verificaron con pruebas de integración y navegador:

El gateway realtime será un módulo Rust `realtime` del mismo monolito y será dueño de su schema, journal y gRPC. Consumirá todos los eventos de integración desde un durable independiente y guardará `event_id`, cursor monotónico, versión de agregado, audiencia, tenant scope, correlation ID y envelope saneado; inserción y deduplicación preceden al ACK. `ReplayEvents` autoriza sesión/audiencia en IdentityAccess y pagina por cursor; `Subscribe` entrega señales durables tras reanudar cursor. El BFF autentica cookie y `Origin`, bridgea el stream por WebSocket y nunca accede a Redis, JetStream ni PostgreSQL. Redis Pub/Sub transmite solo cursores efímeros entre réplicas; una pérdida de señal se repara consultando journal por REST/cursor. Los clientes mantienen un coordinador TanStack Query central por aplicación, descartan duplicados/versiones viejas, detectan huecos, invalidan y recuperan por REST.

Cada frontend muestra el estado de conexión realtime y una acción de sincronización manual. La sesión y sus scopes poseen sus sockets y cursores: al cerrar sesión, cambiar identidad o perder autorización, se cierran los sockets y se borran los cursores locales de esa audiencia. En el frontend tenant se administra un socket y un cursor por cada membresía activa, sin compartir estado de reconexión entre tenants.

La ejecución local y de despliegue de migraciones será explícita y coordinada desde un único comando de plataforma, aplicando los migradores versionados de cada módulo sobre la base física compartida. La API no modifica el esquema al arrancar. El comando recibe `PLATFORM_MIGRATION_DATABASE_URL`, una identidad con permisos DDL separada de `PLATFORM_DATABASE_URL`, y no emite credenciales a stdout/stderr. La aceptación continúa usando su base efímera aislada; las pruebas destructivas no apuntan a la base persistente de desarrollo.

El runner browser reserva puertos loopback libres para los BFF y frontends efímeros y propaga los orígenes a Playwright. No debe reutilizar por error un servicio local preexistente en los puertos habituales ni detener procesos externos a la prueba.

| Flujo | Ownership y contrato | Persistencia/Redis/NATS/realtime | Verificación requerida |
|---|---|---|---|
| Onboarding y membresía propietaria | `identity_access` crea usuario y posee membresías/roles; `tenant_registry` crea tenant y posee estado. El BFF coordina por REST→gRPC. No hay SQL cruzado ni transacción distribuida. Un evento versionado `tenant.registered` del Outbox de `tenant_registry` es consumido idempotentemente por `identity_access` para crear la membresía inicial `owner`; la respuesta de onboarding solo se considera lista cuando el BFF puede confirmar la membresía por el contrato de identidad. | PostgreSQL escribe tenant+Outbox atómicamente en `tenant_registry`, y membresía+auditoría+Outbox atómicamente en `identity_access`. NATS JetStream entrega con consumer durable y deduplicación por `event_id`. Redis no autoriza onboarding; no aplica salvo límites efímeros ya existentes. La membresía emitirá evento realtime de audiencia tenant después de persistir. | Alta completa por browser contra BFF real; slug/email duplicado, redelivery y retry no duplican tenant/membership; usuario no puede reclamar tenant de otro; fallo parcial es recuperable y observable. |
| Aislamiento real entre tenants | Cada acceso primero valida la audiencia de la sesión. Una sesión tenant válida que carece de membresía recibe denegación de autorización; solo una sesión que no es tenant puede continuar la comprobación de audiencia de operador. Las consultas de recursos conservan RLS y contexto tenant. | El usuario B y su tenant se crean por la API real; el relay JetStream y consumer crean su membresía. El usuario B intenta leer y listar llaves del tenant A; el rol runtime consulta RLS bajo el contexto B. | La sesión B recibe 403 para recursos A; SQL bajo contexto B no devuelve llaves A. Credenciales inválidas siguen recibiendo 401. |
| API Keys del tenant | `identity_access` genera llaves test/live, conserva digest y metadatos, aplica `api_keys.manage` y crea auditoría/Outbox para creación y revocación. El BFF solo transporta el secreto en la respuesta única de creación. | PostgreSQL persiste digest/estado bajo RLS tenant; Outbox publica el cambio durable; Redis no almacena llaves. | La aceptación HTTP verifica creación `live`, prefijo/digest SHA-256, respuesta única, listado sin secreto, revocación durable/auditable y rechazo cross-tenant; tests unitarios cubren el prefijo `test_` y validaciones. |
| Operaciones administrativas | `tenant_registry` autoriza y ejecuta listar/buscar/verificar/activar/suspender/reactivar/cerrar; `identity_access` valida sesión/CSRF, rol y permiso. Cada operación usa tenant ID, motivo y versión esperada para evitar updates perdidos. BFF no consulta datos ni implementa reglas. | La transición, history, auditoría local y Outbox se confirman juntas; el relay publica estado versionado. Redis no es fuente de autorización. Gateway registra cambios para audiencia de plataforma y tenant según permiso. | E2E browser cubre verificar/activar/suspender/reactivar/iniciar cierre/confirmar cierre; aceptación API cubre permiso denegado, transiciones inválidas, conflicto por versión, motivo, auditoría y aislamiento. |
| Relay Outbox→JetStream | Worker dentro de la aplicación/monolito modular; repositorios de Outbox siguen dentro del módulo productor. Lease con expiración, entrega al menos una vez, `event_id` como clave de deduplicación, límite/reintento y telemetría correlacionada. No se marca publicado antes del ack durable de JetStream. | Cada módulo procesa solo su Outbox. Consumers crean membresías/journal en transacciones propias, con unique constraints idempotentes. NATS conserva eventos; PostgreSQL Outbox sigue siendo autoridad ante indisponibilidad. Redis no media publicación durable. | Prueba contra PostgreSQL, Redis y JetStream de desarrollo reales: publicación, duplicado, caída/reconexión, pending redelivery, recuperación de lease y ausencia de pérdida confirmada; sin crear/reiniciar contenedores. |
| Gateway realtime y cursor | Módulo `realtime` posee journal durable, cursor, audiencia y autorización de suscripción. BFF autentica sesión antes del handshake WebSocket y revalida permisos; snapshot REST usa cursor autorizado. Frontends usan coordinador central TanStack Query separado por audiencia. | Consumers JetStream persisten journal/deduplicación; BFF fan-out multi-réplica usa Redis Pub/Sub efímero con resuscripción; REST recupera desde PostgreSQL por cursor/snapshot. Ningún estado de autorización vive en Redis. | Aceptación de dos sesiones/audiencias y dos instancias cuando el entorno existente lo permita; evento ordenado, hueco, repetición, cursor inválido, desconexión, recuperación REST y rechazo de scope cruzado. |
| E2E de ambas aplicaciones | Playwright ejecuta navegador real de `frontend-pasarela` y `frontend-plataform`, cada uno contra su servidor de desarrollo y BFF real. No se sustituyen BFF/gRPC/módulos/DB por mocks. | Stack de prueba reutiliza servicios existentes; PostgreSQL de aceptación efímero, datos transitorios generados vía aplicación, Redis/NATS reales, limpieza de recursos propios. | Tenant: registro→sesión→membresía→REST snapshot→evento WebSocket y recuperación. Administración: bootstrap/sesión→búsqueda/revisión/activación/suspensión→auditoría/eventos. Ningún flujo depende solo de la prueba manual en navegador. |

**Orden aplicado:** contratos y autorización; onboarding y consumer idempotente; operaciones administrativas y UI; relay Outbox→JetStream; journal/gateway WebSocket y REST/cursor; coordinadores TanStack Query y Playwright E2E para ambas apps; pruebas de reinicio/reconexión y diagnósticos.

### Prueba destructiva de reconexión en desarrollo

El comando `cd frontends/frontend-plataform && npm run test:e2e:recovery` valida dos réplicas del backend: abre WebSockets contra ambas, cambia el tenant desde una y verifica fan-out Redis y sincronización en la otra. También comprueba reconexión tras reiniciar Redis y JetStream mientras ambas aplicaciones están activas. Antes de reiniciar, el script valida que `PLATFORM_TEST_DATABASE_ADMIN_URL`, `PLATFORM_REDIS_URL` y `PLATFORM_NATS_URL` de `bacekend-plataform/.env` apunten a loopback, y que existan los contenedores de desarrollo nombrados abajo. No crea contenedores ni permite endpoints remotos.

Durante la prueba se ejecuta `docker restart payvia-shared-infra-redis-1` y `docker restart payvia-shared-infra-nats-1`. Esto interrumpe Redis/NATS y afecta temporalmente a cualquier otro proceso local conectado a esos servicios; Redis pierde solo sus conexiones/suscripciones efímeras, JetStream conserva el stream y los mensajes durables en su volumen. Se levantan dos instancias del backend y dos servidores Vite locales sobre puertos loopback libres elegidos para la ejecución; el wrapper apaga solo sus procesos y elimina la base, el rol y las credenciales efímeras al finalizar. No ejecutarlo contra infraestructura de producción.

`identity_access.AuthorizeOperatorOperation` valida token y CSRF, autoriza según permisos persistidos y audita el resultado con correlación. El contrato no devuelve roles al cliente; `tenant_registry` lo consulta antes de ejecutar operaciones administrativas.

| Frontera | Diseño e implementación verificados | Evidencia |
|---|---|---|
| PostgreSQL/migraciones | Una base física; migraciones coordinadas y schemas propiedad de `identity_access`, `tenant_registry` y `realtime`; RLS y rol runtime sin `BYPASSRLS`; Outbox en transacción local. | Aceptación crea/elimina base y rol efímeros, aplica migraciones versionadas y genera datos por la aplicación real. |
| Frontend tenant → REST/OpenAPI → BFF | Onboarding, membership, contexto tenant y realtime en `frontends/frontend-pasarela`. Configuración Vite reside en `frontends/frontend-pasarela/.env`; no se introduce `VITE_*` al `.env` de los backends. | `npm test`, build y Playwright reales aprobados; onboarding termina con membership propietaria y evento realtime. |
| Frontend administración → REST/OpenAPI → BFF | Sesión independiente, revisión y ciclo de vida de tenants en `frontends/frontend-plataform`; no accede directamente a almacenes ni gRPC. Su `.env` es propiedad de esa app. | `npm test`, build y Playwright reales aprobados; operaciones administrativas y eventos visibles en UI. |
| BFF → módulos gRPC | BFF enruta identidad y tenant registry por contratos Protobuf; los módulos dueños validan autorización, persistencia y reglas. | Aceptación HTTP→BFF→gRPC→módulo cubre éxito, errores, traza/correlación y redacción. |
| Redis | Rate limit temporal con TTL y fan-out Pub/Sub efímero; nunca autoridad de negocio. | Umbrales/TTL y restart/reconnect/resuscripción verificados contra Redis real de desarrollo. |
| NATS/Outbox | Relay por módulo, ACK durable, consumidores idempotentes para membership y journal realtime. | JetStream real verificado con evento, redelivery/restart y recuperación. |
| Realtime y errores | WebSocket autenticado por BFF, cursores y recuperación REST; errores tipados/correlacionados en fronteras. | Browser E2E, replay por cursor, aislamiento tenant, recuperación cross-replica y errores HTTP probados. |

## Flujos de aceptación

1. Crear usuario tenant, crear tenant pendiente, observar el evento durable y completar la membresía propietaria idempotente sin transacción SQL entre módulos.
2. Login tenant y operador con cookies/audiencias distintas; logout, expiración, revocación, CSRF, origen inválido, cuenta inválida y límite de intentos.
3. Operador verifica y activa tenant con permiso y motivo; suspensión/cierre revocan acceso en cada request y generan auditoría/evento.
4. Usuario autorizado crea API Key `test` o `live`, ve el secreto una vez, consulta metadata sin secreto, revoca y demuestra que la llave revocada no se puede usar/consultar. Scopes/env/tenant no se pueden alterar.
5. Dos tenants y dos operadores independientes intentan leer, modificar, enumerar, revocar y recibir eventos de recursos ajenos. Se verifica directamente PostgreSQL con credenciales runtime de mínimo privilegio.
6. Replay autorizado desde cursor incluye los eventos reales; pedir desde el cursor reconocido devuelve cero duplicados; duplicar un evento real de JetStream mantiene una sola fila en el journal; el scope ajeno recibe 403; browser E2E comprueba WebSocket y convergencia en dos réplicas tras reiniciar Redis/NATS.

## Errores que deben probarse

Validación de campos/email/password/scope; autenticación inválida, expirada o revocada; autorización denegada y tenant cruzado; conflicto de email/nombre; estado de tenant inválido; rate limit; timeout/conectividad de gRPC/PostgreSQL/Redis/NATS; constraint/persistencia; replay/redelivery; fallo de realtime y cursor. Los errores no aplicables a una operación específica se justifican en su prueba/matriz y no se simulan.

## Criterios de cierre

- Todos los ítems de Fase 2 del [roadmap](../../ROADMAP.md) implementados con ownership DDD, Clean Architecture/Hexagonal, contratos y migraciones reales.
- Ambas aplicaciones frontend compilan y recorren sus flujos contra el BFF; no acceden directamente a módulos o almacenes.
- Unitarias, integración y aceptación/E2E automatizadas ejecutadas con PostgreSQL, Redis y NATS reales de desarrollo cuando aplican; sin fixtures/mock que sustituyan dependencias.
- Pruebas de aislamiento verifican consultas y datos persistidos, roles y scopes, revocación, evento/realtime y recuperación.
- `cargo fmt`, Clippy, tests, builds de frontend, análisis y controles aplicables pasan; se registran comandos/resultados.
- Evidencia actualizada aquí; estado del README, roadmap de backend y roadmaps de ambos frontends consistente.

## Acta de cierre y evidencia ejecutada — 2026-10-22

Fase 2 está completa en esta rama. Los recorridos implementados son identidad diferenciada, onboarding y membership propietaria mediante Outbox/JetStream, aislamiento tenant/RLS, API Keys test/live y operaciones administrativas de tenant; los cambios visibles se entregan por realtime y se recuperan por cursor/REST.

- `cargo fmt --all -- --check`: aprobado.
- `cargo clippy -p bacekend-plataform --all-targets -- -D warnings`: aprobado.
- `bash -lc 'set -a; source bacekend-plataform/.env; set +a; cargo test -p bacekend-plataform -p payvia-platform-core -p platform-identity-access -p platform-tenant-registry -p platform-realtime -p platform-runtime -p payvia-platform-bff'`: aprobado; incluye 22 pruebas unitarias y dos aceptaciones HTTP→BFF→gRPC con PostgreSQL/Redis reales, base temporal, rol runtime con RLS y limpieza de recursos.
- En `frontends/frontend-pasarela`: `npm test` (3 pruebas), `npm run build` y `npm run test:e2e`: aprobados. Playwright recorrió onboarding por BFF real y verificó membership propietaria y WebSocket conectado.
- En `frontends/frontend-plataform`: `npm test` (3 pruebas), `npm run build` y `npm run test:e2e`: aprobados. Playwright verificó sesión de operador y transiciones administrativas de tenant con cambios realtime.
- `cd frontends/frontend-plataform && npm run test:e2e:recovery`: aprobado. Dos réplicas del BFF mantuvieron sesiones y WebSockets; el test reinició únicamente Redis y NATS JetStream de desarrollo, verificó fan-out tras reinicio y recuperación durable/REST. El script usó puertos libres y no detuvo servicios preexistentes.
- La aceptación backend creó dos tenants/usuarios por APIs reales, comprobó 403 y filtrado RLS para recursos cross-tenant, consumo idempotente de registro y deduplicación del journal usando un evento real de JetStream, API Key `live` (prefijo/digest/listado/revocación) y replay por cursor con denegación de audiencia ajena.
- La configuración local permanece separada: cada frontend tiene su `.env` ignorado y `.env.example` propio; los `.env` de backend no contienen variables frontend `VITE_*`.

La aceptación de esta fase no valida integraciones de Wompi, Factus o Resend ni funcionalidades de planes y billing; pertenecen a fases posteriores. No se declara completado el roadmap total del backend. El cierre se limita al alcance y criterios de Fase 2 descritos arriba.
