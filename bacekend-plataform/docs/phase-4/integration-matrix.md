# Fase 4 — Matriz de integración y evidencia

**Estado:** implementación parcial en PR #11 (rama `feat/phase-4-payment-lifecycle`). El HEAD `c036f0010fa8140b1a8809ffa2d6be3c3e190f71` pasó [CI #942](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41753921841), incluidos límite de 400 líneas, formato, Clippy, tests del workspace, ambos frontends, E2E y checks documentales. La matriz distingue la base de dominio/contrato verificada de la persistencia, integración externa y operación que siguen pendientes. No declara aceptación ni cierre de Fase 4.

## 1. Regla de evidencia

Un componente se considera implementado únicamente cuando el código, la migración, el contrato y una prueba ejecutable demuestran el comportamiento sobre el SHA exacto. La presencia de una interfaz, un cliente o un archivo de test no basta. Las dependencias de proveedor requieren además sandbox/cuenta habilitados y pruebas reales.

La Fase 4 tiene dos alcances independientes:

- **A — Suscripciones y facturación de uso Payvia.**
- **B — Ownership de Payment Intents y transacciones de compradores.**

Las decisiones económicas pendientes del [registro de decisiones](decision-register.md) no se convierten en valores por defecto en esta matriz.

## 2. Alcance B — Payment Intents (slice implementado)

| Frontera | Ownership / decisión | Implementación en el PR | Evidencia y estado |
|---|---|---|---|
| PostgreSQL | `payments` es dueño de Payment Intents, claves de idempotencia, auditoría y Outbox. | Migración propia; constraints de importe, moneda, método y entorno; RLS forzada para intent, idempotencia y auditoría. El contexto `app.tenant_id` se establece dentro de la transacción. | `modules/payments/infrastructure/persistence/migrations/202611030001_payment_intents.sql`; aceptación PostgreSQL aislada con rol runtime sin BYPASSRLS. **Implementado para creación/consulta.** |
| API Key → identidad | `identity_access` sigue siendo dueño de API Keys, digest, scopes, estado y revocación. | El módulo `payments` solicita autorización de `payment_intents.create` o `payment_intents.read`; identidad valida digest, tenant, entorno, estado y scope requerido. | Código real `PaymentsApi` → `IdentityAccess`; auditoría de autorización en `identity_access`. **Implementado en la frontera gRPC interna.** |
| REST → BFF → gRPC → payments | BFF solo adapta HTTP; `payments` conserva las reglas de negocio. | `POST /v1/tenants/{tenant_id}/payment-intents` y `GET /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}`; deadlines y envelope de error estable. | `payment_intents_http_acceptance.rs` atraviesa HTTP real → BFF → gRPC → `identity_access` → PostgreSQL y verifica creación, replay, conflicto, lectura, ausencia de credencial, tenant/entorno, scopes y revocación. **Implementado y probado.** |
| Idempotencia comercial | Scope: tenant + entorno + operación versionada + idempotency key. La huella representa el payload normalizado. | Constraint único PostgreSQL; SHA-256 de representación canónica; transacción serializa solicitudes duplicadas; replay devuelve snapshot original; payload distinto devuelve conflicto. | `phase4_payments_acceptance.rs`: replay, conflicto y 12 solicitudes concurrentes; solo una crea el agregado. **Implementado y probado en servicio con PostgreSQL real.** |
| Aislamiento | Tenant y entorno son parte de cada lectura y de la clave de idempotencia. | Consulta filtra por intent ID + tenant ID + entorno; RLS aplica `app.tenant_id`. | Pruebas de lectura cross-tenant y cross-environment devuelven no encontrado. **Implementado y probado.** |
| Auditoría | Cada creación, replay y lectura registra actor API Key, recurso, resultado, correlación y detalles acotados. | Auditoría escrita en la misma transacción local que el efecto correspondiente; nunca se persiste el secreto de API Key. | Test verifica tres entradas para creación, replay y lectura; los logs/código no incluyen el secreto. **Implementado para estas operaciones.** |
| Outbox → JetStream | Outbox pertenece a `payments`; la entrega es al menos una vez, no exactly-once. | La creación escribe `payment_intent.created` en la misma transacción. Relay usa leases de 30s con token propietario y validación de expiración, `SKIP LOCKED`, timeout independiente de 10s para publish/ACK, backoff exponencial con jitter, cuarentena durable para errores permanentes y `Nats-Msg-Id`; conserva el timestamp original. | `payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime` publica en JetStream real, consume el ACK, verifica envelope/timestamp y simula el hueco ACK→confirmación Outbox. El reintento conserva `event_id` y JetStream deduplica. Las pruebas de leases, recuperación tras rechazo del stream, jitter, relays concurrentes y cuarentena están cubiertas en [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) sobre el SHA `1858766a46a5b92d6b6b75207e728ddaa6933498`. **Verificado para esos escenarios; caída/reinicio físico del proceso NATS y reconexión del socket siguen pendientes.** |
| Realtime / cursor / WebSocket | El journal realtime conserva su ownership; el BFF autentica y el cliente recupera por REST. | Realtime acepta `payment_intent.created`; el consumidor persiste el envelope en el journal y emite la señal Redis antes del ACK. La consulta de recuperación usa cursor y tenant. | La aceptación integrada verifica inserción única, persistencia, ACK posterior, cursor tenant 1 y aislamiento de otro tenant. **Journal/cursor cubiertos; WebSocket autenticado, reconexión del cliente y recuperación tras restart del consumidor siguen pendientes.** |
| Gateway / proveedor | Gateway ejecuta protocolo; plataforma mantiene la decisión de negocio. | No hay llamada de proveedor desde el slice de Payment Intent. | mTLS, identidad de servicio, signed grants y enforcement están asignados a Fase 5; el gateway aún no ofrece el flujo operativo completo requerido. **N/A por dependencia explícita; no se permite bypass, confianza por red ni mock.** |
| Redis | Redis no es fuente de verdad financiera ni de idempotencia comercial. | El slice usa PostgreSQL transaccional para idempotencia y estado. | No se usa Redis en creación/consulta de intents. **N/A por diseño para este slice.** |
| Frontend | Los frontends solo consumen REST; no acceden a gRPC/DB. | No se incorporó pantalla ni estado de cliente en este slice API-first. | La UI de producto permanece en su fase de roadmap correspondiente. **N/A en este slice; no es aceptación de frontend de Fase 4.** |

### 2.1. Trazabilidad de pruebas del slice

| Requisito / comportamiento | Test actual | Estado |
|---|---|---|
| Crear Payment Intent válido y persistir importe/moneda/método/entorno | `payment_intent_creation_replay_conflict_concurrency_and_tenant_isolation` | Cubierto |
| Repetir la misma solicitud y devolver el snapshot original | Mismo test, replay exacto | Cubierto |
| Reutilizar idempotency key con payload distinto | Mismo test, importe modificado | Cubierto |
| Scope de idempotencia incluye entorno | Mismo test; la misma clave puede crear una intención `live` distinta de la intención `test` | Cubierto |
| Concurrencia de 12 solicitudes con la misma clave | Mismo test; exactamente una creación inicial | Cubierto |
| Impedir lectura desde otro tenant | Mismo test, segundo tenant | Cubierto |
| Impedir lectura desde otro entorno | Mismo test, `live` frente a intent `test` | Cubierto |
| Una fila Outbox por Payment Intent | Mismo test; incluye intención test, live y concurrente | Cubierto |
| Auditar creación, replay y lectura | Mismo test | Cubierto |
| Rechazar importes, monedas, entornos, métodos, claves y referencias inválidas sin escrituras | `invalid_payment_intents_are_rejected_before_any_business_write`; verifica intents, idempotency records y auditoría en cero | Cubierto en servicio con PostgreSQL real |
| Validación unitaria de importe, moneda, entorno, método, idempotency key y merchant reference | Tests unitarios de `domain/payment_intent.rs` | Cubierto a nivel de dominio |
| Scope ausente/revocado, auth HTTP inválida y denegación por cada frontera | `payment_intents_http_path_enforces_auth_scope_isolation_and_idempotency` | Cubierto en HTTP real: bearer ausente, API key revocada, scope de solo lectura, ruta cross-tenant y entorno incorrecto. |
| Fallo/rollback PostgreSQL, timeout, recuperación de relay, redelivery y deduplicación tras reinicio | `payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime` + `outbox_lease_expiry_is_reclaimed_and_stale_owner_is_rejected` + `outbox_retries_after_stream_becomes_available_with_jittered_backoff` + `concurrent_relays_publish_all_claimed_events_without_losing_rows` | Crash-window ACK→confirmación, recuperación de lease, backoff y recuperación tras stream ausente, y concurrencia verificados con PostgreSQL/JetStream reales en [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) sobre SHA `1858766a46a5b92d6b6b75207e728ddaa6933498`; caída/reinicio físico de NATS, reconexión del socket y fallo total de PostgreSQL siguen pendientes |
| Publicación JetStream y journal/realtime/cursor autenticados | `payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime` | Cubierto con JetStream, Redis y PostgreSQL reales; la prueba verifica audience tenant, cursor y ausencia de datos de otro tenant. La sesión WebSocket autenticada no está cubierta por este test. |
| Transiciones de estado, intentos, transacciones, eventos tardíos y reconciliación | No existen todavía los agregados/acciones completos | Pendiente |

La aceptación del camino HTTP→BFF→gRPC y las denegaciones de autorización están cubiertas por `payment_intents_http_acceptance.rs`. La aceptación Outbox→JetStream→journal/cursor está cubierta por `payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime`. Estas pruebas no completan el ciclo de vida de transacciones ni la sesión WebSocket de pagos.

### 2.2. Resiliencia del relay Outbox

| Requisito | Test | Infraestructura/evidencia | Estado |
|---|---|---|---|
| Lease vencido recuperado y propietario anterior rechazado | outbox_lease_expiry_is_reclaimed_and_stale_owner_is_rejected | PostgreSQL real; token nuevo, renovación y ACK del propietario anterior rechazados | Implementado y verificado en [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) sobre SHA `1858766a46a5b92d6b6b75207e728ddaa6933498` |
| Error temporal de JetStream, backoff con jitter y recuperación al restaurar el stream | outbox_retries_after_stream_becomes_available_with_jittered_backoff | PostgreSQL y JetStream reales; NACK por ausencia de stream, programación diferida, restauración y publicación posterior | Implementado y verificado en [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) sobre SHA `1858766a46a5b92d6b6b75207e728ddaa6933498` |
| Dos relays concurrentes sin perder eventos reclamados | concurrent_relays_publish_all_claimed_events_without_losing_rows | PostgreSQL y JetStream reales; 40 eventos, dos ejecuciones concurrentes, ACK y conteo del stream | Implementado y verificado en [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) sobre SHA `1858766a46a5b92d6b6b75207e728ddaa6933498` |
| Error permanente de contrato sin bucle de reintentos | permanent_outbox_errors_are_quarantined_without_retrying_forever | PostgreSQL real; failed_at durable, lease liberado y exclusión de claims automáticos | Implementado y verificado en [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) sobre SHA `1858766a46a5b92d6b6b75207e728ddaa6933498` |
| Ventana ACK JetStream → confirmación Outbox y deduplicación del consumidor | payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime | PostgreSQL, Redis y JetStream reales; re-publicación con Nats-Msg-Id estable y journal idempotente | Cubierto y verificado en [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) sobre SHA `1858766a46a5b92d6b6b75207e728ddaa6933498` |

La indisponibilidad de un stream/subject se prueba mediante rechazo real de JetStream y su recuperación al crear el stream. [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) ejecutó esas pruebas sobre el SHA `1858766a46a5b92d6b6b75207e728ddaa6933498` con resultado exitoso. Esta prueba no equivale a detener/reiniciar el proceso servidor NATS ni a demostrar reconexión de socket tras una caída física. Los leases recuperables tampoco implican entrega exactly-once: la garantía sigue siendo al menos una vez y los consumidores deben deduplicar por event_id.

## 3. Alcance B — Transacciones e intentos

**Estado:** la base de dominio/contrato fue verificada por CI #942 sobre `c036f0010fa8140b1a8809ffa2d6be3c3e190f71`. En la rama `feat/phase-4-payment-lifecycle-persistence` se añadió la migración `202611040001_payment_lifecycle_storage.sql` y la aceptación PostgreSQL `lifecycle_storage_enforces_ownership_deduplication_and_reconciliation_integrity`. El código de esta ampliación aún está **pendiente de ejecución CI**; no se presenta como verificado ni operativo.

| Área | Implementación disponible | Estado real |
|---|---|---|
| Payment Intent | Migración de PR #10; solo persiste `created`. La migración nueva añade la clave compuesta tenant/entorno/id/importe/moneda requerida por la relación transaccional. | Crear/consultar PI sigue implementado; sus transiciones persistidas siguen pendientes. |
| Payment Attempt | Nueva tabla con tenant, entorno, intent, secuencia positiva, proveedor/cuenta opacos, estado canónico, versión y timestamp del último evento; FK compuesta y unicidad de secuencia por intent. | Schema y RLS añadidos en esta rama; integración CI pendiente; repositorio/use case de escritura/consulta todavía no existe. |
| Payment Transaction | Nueva tabla con relación compuesta a intent y attempt, proveedor/cuenta, referencia externa, importe/moneda y estados canónicos. El importe/moneda deben coincidir con el intent. Un trigger impide insertar/cambiar la relación hacia un attempt que siga en `created`. | Constraints y RLS añadidos; aceptación CI pendiente. Los guards de transición aún no se aplican desde un reducer PostgreSQL. |
| Inbox normalizado | Nueva tabla con event_id interno, dedupe por proveedor+cuenta+provider_event_id, hash SHA-256, timestamp de proveedor, correlación e identidad declarada del productor; no almacena body crudo ni `outcome_unknown` como estado del proveedor. | Schema y RLS añadidos; todavía no hay RPC/consumer ni código que escriba el inbox. El campo de identidad es dato de auditoría, no autenticación. |
| Trazabilidad de entregas | Tabla separada para registrar cada entrega vinculada a su evento canónico, con productor y disposición; FK compuesta garantiza que inbox_id y event_id coincidan. | Schema añadido; no hay handler de entregas autenticado. |
| Reconciliación durable | Caso por attempt con lease/status/next_attempt_at y restricción de un caso abierto; historial de consultas con resultado clasificado, hash de evidencia y error seguro. | Schema añadido; no hay worker, provider client, consulta autoritativa ni resolución de estado implementados. |
| RLS | Las seis tablas nuevas aplican ENABLE + FORCE RLS con `app.tenant_id`; los FKs incluyen tenant/entorno para impedir relaciones cross-tenant o cross-environment. | Test PostgreSQL real añadido para visibilidad cross-tenant en las seis tablas, sujeto a CI. |
| API y contratos | Protobuf/RPC y HTTP solo exponen create/get Payment Intent existentes. | Crear/consultar attempts/transactions y actualizar estados persistidos continúan pendientes. |
| Callback y proveedor | Gateway continúa siendo scaffold sin adapters ni callback ingress. mTLS/grants están asignados a Fase 5. | Fail-closed: no se expone ingestión; no hay integración/sandbox real que certificar. |

### 3.1. Trazabilidad de pruebas del modelo de dominio

| Requisito técnico | Test | Evidencia/limitación |
|---|---|---|
| Mapear estados canónicos sin códigos específicos de proveedor | `normalized_status_maps_without_provider_specific_codes` | Unit test verificado en CI #942; no valida mapping en adapter. |
| Transiciones forward-only y conflictos terminales | `transitions_are_forward_only_and_terminal_conflicts_need_reconciliation`; `transaction_status_is_guarded_and_terminal_state_is_immutable` | Unit tests verificados en CI #942; todavía no hay reducer persistente. |
| Resolver `outcome_unknown` solo por reconciliación | `outcome_unknown_requires_reconciliation_to_resolve`; `only_platform_timeout_can_enter_outcome_unknown` | Unit tests verificados en CI #942; no consultan a proveedor real. |
| Rechazar timestamps iguales/anteriores | `provider_event_timestamps_must_be_strictly_monotonic` | Unit test verificado en CI #942; depende de timestamp de un adapter verificado. |
| Persistir intent, attempt, transaction, inbox, delivery y reconciliation en PostgreSQL | `lifecycle_storage_enforces_ownership_deduplication_and_reconciliation_integrity` | Test de aceptación añadido en esta rama: escritura transaccional en PostgreSQL real, RLS cross-tenant sobre seis tablas, dedupe externo, entregas aceptadas/rechazadas, relación de ownership, rechazo de transacción sobre attempt `created`, unicidad de caso abierto y ausencia de fila tras rollback. **Pendiente de ejecución CI.** |
| Conservar importe/moneda y ownership | Constraints compuestas y trigger de `payment_transactions` | Implementados en SQL; pendientes de ejecución de migración/aceptación en CI. |
| Inbox seguro/idempotente | Constraint provider+account+provider_event_id | La persistencia deduplica claves externas; la autenticación del productor y el procesamiento idempotente aún no existen. No aceptar eventos hasta que exista frontera autenticada. |

### 3.2. Lo que esta ampliación no habilita

- No añade endpoints/RPC ni casos de uso de attempts o transactions.
- No aplica transiciones de estado, auditoría y Outbox de ciclo de vida desde un servicio transaccional; la migración reutiliza las tablas de auditoría/Outbox existentes sin afirmar que el flujo esté conectado.
- No habilita RPC/consumer de `NormalizedProviderPaymentEventV1`. Callback verification/anti-replay pertenece al adapter real de pasarela; mTLS/grants de productor continúan en Fase 5.
- No ejecuta consultas al proveedor ni resuelve `outcome_unknown`. El caso durable y el historial de consulta son almacenamiento, no una reconciliación operativa.
- No habilita cobros, captura, cancelación, expiración, reintentos comerciales, cambio de proveedor ni refunds. B-EC-07..13 siguen sujetos a aprobación.
- No certifica el alcance A de suscripciones y facturación SaaS.

La migración es aditiva: el rollback de una versión de aplicación consiste en volver al binario anterior y conservar las tablas/registros financieros; no se ejecuta un DROP automático de datos de ciclo de pagos. Antes de activar cualquier flujo nuevo se requieren repositorios, casos de uso, API, autenticación y aceptación adicional.

## 4. Alcance A — Suscripciones y facturación SaaS de Payvia

| Frontera | Estado actual | Qué falta antes de aceptación |
|---|---|---|
| Catálogo y precios | `catalog_pricing` ya posee versiones de precio inmutables. | Aprobación del modelo comercial, unidades, importes, periodo, moneda, impuestos, redondeo y vigencias. |
| Entitlements y usage ledger | `entitlements_quotas` posee asignaciones, cuotas y medición durable de consumo. | Procedencia por operación/recurso, contrato de medición facturable y reglas aprobadas de inclusión/excedente. El ledger de cuotas no es una factura ni un ledger financiero. |
| Subscription aggregate | No existe módulo de suscripciones ni ciclo de vida persistido. | Modelo técnico, relación con asignaciones y reglas de ciclo de vida aprobadas. |
| Billing invoices / invoice lines | No existe módulo financiero de facturación SaaS. | Modelo de obligación inmutable, snapshot reproducible, ledger financiero y reglas comerciales/fiscales aprobadas. |
| Platform Wompi | No existe adapter propio `platform_billing_wompi`. | Cuenta/credenciales Payvia, sandbox, métodos, moneda, autenticación, callbacks oficiales y validación de idempotencia/resultado incierto. |
| Factus | No existe módulo `electronic_invoicing` ni adapter `invoice_factus`. | Contrato fiscal, autenticación, ambiente, tipos de documento, ciclo de estados y credenciales/sandbox. |
| Resend | No existe módulo `notifications` ni adapter `notification_resend`. | Política de notificación, destinatarios, Outbox/delivery state, clave de deduplicación y credenciales. |
| Recaudo / ledger | No hay intentos ni resultados de cobro de invoices SaaS. | Flujo Wompi de Payvia, callback verificado, ledger y reconciliación con PostgreSQL como fuente de verdad. |

**Regla de bloqueo económico:** no calcular facturas, tarifas variables, excedentes, prorrateos, impuestos, FX, comisiones, vencimientos ni cargos recurrentes mientras las decisiones aplicables A-EC estén pendientes. Registrar la aprobación real antes de habilitar cada comportamiento.

## 5. Restricciones cross-backend y despliegue

- El cobro a compradores por cuenta de los tenants y el cobro SaaS que Payvia realiza a sus tenants son flujos distintos. El primero pertenece al ownership de Payment Intents/transacciones en `payments` y requiere ejecución protocolar autorizada del gateway; el segundo pertenece a los módulos/adapters de billing de plataforma y usa la cuenta/credenciales propias de Payvia. No se comparten credenciales, ledgers ni ownership. Ninguno se declara operativo por la mera existencia del contrato neutral.
- Plataforma no delega el cobro SaaS al backend de pasarelas.
- El slice actual de Payment Intents no llama al gateway. El nuevo mensaje `NormalizedProviderPaymentEventV1` es solo un contrato; no se expone endpoint ni consumer para recibirlo sin autenticación.
- El callback del proveedor debe ser verificado en el adapter gateway y el productor debe autenticarse en la frontera cross-backend. La ausencia de esas capacidades bloquea la ingestión; no se sustituye con headers, IP allowlists, permisos locales ni mocks.
- No se comparten schemas ni se hacen joins/escrituras SQL entre dominios.
- PostgreSQL es fuente de verdad; Redis no decide importes, autorización ni idempotencia comercial.
- No se afirman adaptadores Wompi, Factus o Resend operativos sin credenciales y pruebas oficiales de sandbox.
- Despliegue/ejecución en producción queda fuera de esta evidencia de construcción.

## 6. Criterio de salida

Fase 4 solo puede declararse cerrada cuando ambos alcances tengan decisiones aprobadas, implementación completa, pruebas reales positivas/negativas/límites/concurrencia/permiso/aislamiento/estados/fallos/reintentos/integridad/redacción, integraciones externas verificadas y trazabilidad requisito → test → ejecución CI sobre el SHA final. Esta matriz deja explícitamente pendientes los puntos que aún no tienen implementación o evidencia.
