# Fase 4 — Matriz de integración y evidencia

**Estado al 2026-11-09:** implementación parcial en PR #12 (`feat/phase-4-payment-lifecycle-persistence`). Su HEAD `fb5696b44483f927aa53faab423580e4abd283cb` pasó todos los jobs de [CI #42752212426](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/42752212426). Esto verifica la automatización del slice presente, no el cierre de Fase 4 ni la aceptación de proveedores externos. La continuación de los requisitos pendientes se organiza en PR #16; las decisiones comerciales y dependencias externas siguen explícitamente bloqueadas hasta su aprobación/evidencia.

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
| Transiciones iniciales, intentos, transacciones y alta de reconciliación | Payment Intent/Attempt, Payment Transaction, guards iniciales, versión optimista, auditoría y caso durable `outcome_unknown` en PR #12. | Transiciones `created → processing → outcome_unknown`, API de attempts/transactions y caso durable incluidos en el CI exitoso #42752212426 sobre el HEAD de PR #12. La aplicación de eventos externos tardíos, resolución completa del caso y aceptación de proveedor siguen pendientes según la sección 3. |

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

**Estado:** PR #12 incorpora persistencia/API de attempts y transactions, recepción autenticada de callbacks Wompi, consumo cross-backend de eventos firmados y worker durable de reconciliación. CI #42752212426 pasó sobre el HEAD exacto; las aceptaciones de Wompi Sandbox y los demás requisitos abiertos siguen pendientes y se continúan en PR #16. No se declara cerrada Fase 4.

| Área | Implementación disponible | Estado real |
|---|---|---|
| Payment Intent | La migración amplía el CHECK a estados canónicos, permite aggregate_version creciente y conserva updated_at/last_event_at. El use case aplica guards para created → processing y processing → outcome_unknown, con control optimista de versión. | Intent y attempt se actualizan en una transacción PostgreSQL; estados terminales del proveedor requieren evento verificado. Automatización validada en CI #42752212426. |
| Payment Attempt | Tabla con tenant/entorno/intent, secuencia, proveedor/cuenta, estado/version. Create/list conectados al PaymentService; SHA-256 del request + idempotency key por intent; parent row `FOR UPDATE` serializa concurrencia. | Replay de la misma clave devuelve el intento original; otra clave se rechaza hasta aprobación B-EC-10. Lecturas/mutaciones auditadas. La aceptación PostgreSQL y HTTP está incluida en CI #42752212426 sobre el HEAD de PR #12; CI #964 es evidencia histórica de una revisión anterior. |
| Payment Transaction | Tabla con FK compuestas a intent/attempt, proveedor/cuenta y consistencia de importe/moneda; trigger impide transacción sobre attempt created. List API filtra por tenant/entorno/intent. | Query conectada. Crear/actualizar transacciones continúa reservado a la frontera de evento de proveedor autenticado; no se simulan respuestas. Automatización validada en CI #42752212426. |
| Inbox normalizado | Deduplicación por tenant+entorno+proveedor+cuenta+provider_event_id, hash SHA-256, timestamp, correlación y envelope normalizado; no guarda body crudo. | El gateway verifica el checksum oficial de Wompi antes de normalizar y firma el envelope interno con HMAC. El consumer de plataforma vuelve a verificar esa firma, valida tenant/entorno/attempt/proveedor/cuenta/importe/moneda y escribe inbox + delivery + cambios + auditoría + Outbox en una transacción PostgreSQL. La aceptación firmada/replay/conflicto pasa CI #42752212426; el sandbox real sigue pendiente. |
| Trazabilidad de entregas | `provider_event_deliveries` registra accepted/duplicate/rejected y vincula reentregas con el evento canónico. | El consumer registra primera entrega y replay en el ledger; la recepción HTTP del gateway solo responde 204 tras el ACK durable de JetStream. Falta aceptación real de callback contra Wompi Sandbox y operación con credenciales de cuenta válidas. |
| Reconciliación durable | Caso por attempt con estado/lease, restricción de caso abierto e historial de consultas/evidencias. La transición a `outcome_unknown` y los conflictos/late events generan una orden Outbox firmada. | El worker gateway consume JetStream durable, verifica HMAC/key ID del productor plataforma, consulta `GET /v1/transactions/{id}` a Wompi cuando existe ID de transacción y verifica referencia de attempt/importe/moneda antes de publicar un resultado firmado. Plataforma verifica ese resultado y resuelve el caso transaccionalmente o lo conserva en manual_review. Sin transaction ID no se inventa lookup ni se reintenta el cobro. Falta verificación en Sandbox. |
| RLS / constraints | ENABLE + FORCE RLS en las seis tablas; FK compuestas de tenant/entorno/intent/attempt/monto/moneda/proveedor/cuenta; secuencia única e inbox idempotente. | Prueba PostgreSQL real con runtime role sin BYPASSRLS para visibilidad cross-tenant y constraints; automatización validada en CI #42752212426. |
| Auditoría | Creación/replay/lectura de attempts, consulta de transactions y transiciones de intent/attempt con actor API key, recurso, correlación y detalles acotados. | Auditoría escrita dentro de la transacción de negocio; rollback por fallo Outbox evita filas parciales. Automatización validada en CI #42752212426. |
| Outbox | `payment_attempt.created`, cambios de estado de intent/attempt y `payment.reconciliation.requested` con aggregate_version y correlation ID. | Escrituras atómicas con estado/auditoría; trigger PostgreSQL induce fallo Outbox y verifica rollback. El relay/ACK en JetStream real está incluido en CI #42752212426; CI #964 documenta evidencia histórica en un SHA anterior. |
| API / contratos | RPCs CreatePaymentAttempt, ListPaymentAttempts, ListPaymentTransactions y UpdatePaymentIntentStatus; BFF POST/GET attempts, GET transactions, PATCH status y OpenAPI. | Casos de uso, contratos y endpoints añadidos; aceptación HTTP real del HEAD final validada en CI #42752212426. |
| Scopes | payment_attempts.create, payment_attempts.read, payment_transactions.read y payment_intents.update. | identity_access autoriza cada RPC con su scope; test HTTP incluye scope insuficiente. Automatización validada en CI #42752212426. |
| Gateway / proveedor | Se implementó el ingress HTTP de callback Wompi y el primer cliente de consulta autoritativa. | `POST /v1/webhooks/wompi/{account_reference}` valida el checksum de Wompi, entorno y asociación del identificador de attempt; publica un evento interno HMAC firmado en JetStream. El worker consulta transacciones por ID y devuelve resultados firmados. NATS acepta certificados CA/cliente y puede exigir mTLS con `PAYVIA_REQUIRE_MTLS=true`; sin ese flag se mantiene compatibilidad de desarrollo/CI. No hay aún operación saliente de creación de cobro, vault/cifrado de credenciales, identidad SPIFFE/grants por comando ni sandbox acceptance; por tanto el gateway no se declara adapter de pago productivo completo. |

### 3.1. Trazabilidad de pruebas

| Requisito | Test | Cobertura / estado |
|---|---|---|
| Ownership, RLS, constraints, inbox duplicado, deliveries, caso único, attempt no enviado y rollback | `lifecycle_storage_enforces_ownership_deduplication_and_reconciliation_integrity` y `concurrent_provider_event_deliveries_create_one_canonical_inbox_row` | PostgreSQL real, seis tablas, rol runtime restringido, replay y carrera concurrente sobre la clave externa del inbox. **Incluido en el CI exitoso** [#42752212426](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/42752212426) del HEAD de PR #12; CI #964 es evidencia histórica. |
| Creación de attempt, idempotency key/fingerprint, replay y retry guard | attempt_creation_is_idempotent_under_concurrency_and_disallows_unapproved_retries | 12 llamadas concurrentes, una sola creación, replay estable, conflicto por request distinta y segundo intento bloqueado; automatización validada en CI #42752212426. |
| Estado concurrente, aggregate_version y caso outcome_unknown | `status_transitions_are_guarded_atomic_and_create_durable_reconciliation` | Actualización concurrente idempotente, transacción asociada, caso durable y Outbox. **Incluido en el CI exitoso #42752212426** sobre el HEAD de PR #12; CI #964 corresponde a una revisión anterior. |
| Rollback de estado/auditoría/Outbox | failed_outbox_write_rolls_back_attempt_intent_and_audit_changes | Trigger PostgreSQL induce fallo real; verifica que intent, attempt, versiones y auditoría no queden parcialmente modificados; automatización validada en CI #42752212426. |
| API HTTP→BFF→gRPC→identity_access→payments | payment_attempt_and_transaction_apis_cross_http_bff_grpc_and_payments_module | Create/replay/list attempts, list transactions, PATCH status, scopes, cross-tenant, versión obsoleta y resultado final falsificado; automatización validada en CI #42752212426. |
| Callback firmado Wompi, replay y conflicto terminal | `signed_provider_events_apply_once_and_terminal_conflicts_require_reconciliation` | PostgreSQL real: firma HMAC cross-backend, primera aplicación, replay deduplicado, estado terminal preservado y creación durable de caso `terminal_conflict`; automatización validada en CI #42752212426. Los vectores unitarios de checksum Wompi comprueban la concatenación de propiedades dinámicas, timestamp y secreto. El sandbox del proveedor sigue pendiente. |
| Relay Outbox/JetStream | `payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime`, `outbox_resilience` y `lifecycle_outbox_event_types_are_acknowledged_by_real_jetstream` | Verifica relay/leases/ACK y publica tipos de eventos del lifecycle y reconciliación en JetStream real. CI #42752212426 valida la automatización del HEAD de PR #12; no prueba el sandbox ni la caída física/reinicio de NATS descrita en los pendientes. |

### 3.2. Límites que permanecen cerrados

- El consumer de plataforma solo acepta eventos internos firmados; `producer_identity` sigue siendo metadato de auditoría, no autenticación. El webhook público se verifica primero con el contrato Wompi; no se acepta un NormalizedProviderPaymentEventV1 arbitrario desde internet.
- El worker consulta Wompi cuando hay ID externo de transacción. Si falta, hay mismatch o la evidencia no es concluyente, la conciliación queda en `manual_review`; no se crea otro cobro.
- La API del tenant solo permite created → processing (OperationSubmitted) y processing → outcome_unknown (OperationTimeout), con aggregate_version esperado. No permite afirmar authorized, succeeded o failed con una API key de tenant.
- No se habilitan captura, cancelación, expiración, reintentos comerciales, cambio de proveedor ni refunds mientras B-EC-07..13 sigan pendientes.
- El alcance A de suscripciones/facturación SaaS continúa independiente y pendiente.
- Callback checksum verification Wompi y autenticación HMAC de eventos/órdenes están implementados; mTLS se puede exigir con `PAYVIA_REQUIRE_MTLS=true`. Falta probar callback/lookup en Sandbox, desplegar certs reales y completar identidad de servicio/grants ligados a operación de cobro de Fase 5.

### 3.3. API, contratos y scopes

| Operación | RPC | Endpoint BFF | Scope |
|---|---|---|---|
| Crear Payment Attempt | CreatePaymentAttempt | POST /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/attempts | payment_attempts.create |
| Consultar attempts | ListPaymentAttempts | GET /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/attempts | payment_attempts.read |
| Consultar transactions | ListPaymentTransactions | GET /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/transactions | payment_transactions.read |
| Actualizar estado del intent | UpdatePaymentIntentStatus | PATCH /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/status | payment_intents.update |

La prueba `payment_attempt_and_transaction_apis_cross_http_bff_grpc_and_payments_module` atraviesa HTTP real → BFF → gRPC → identity_access → payments → PostgreSQL. La matriz registra requisito → test → ejecución CI. Está incluida en el CI exitoso #42752212426 del HEAD de PR #12; CI #964 sobre `e92123be7bb7762d08fe537e42997a466146824e` es evidencia histórica.

## 4. Alcance A — Suscripciones y facturación SaaS de Payvia

| Frontera | Estado actual | Qué falta antes de aceptación |
|---|---|---|
| Catálogo y precios | `catalog_pricing` ya posee versiones de precio inmutables. | Aprobación del modelo comercial, unidades, importes, periodo, moneda, impuestos, redondeo y vigencias. |
| Entitlements y usage ledger | `entitlements_quotas` posee asignaciones, cuotas y medición durable de consumo. | Procedencia por operación/recurso, contrato de medición facturable y reglas aprobadas de inclusión/excedente. El ledger de cuotas no es una factura ni un ledger financiero. |
| Subscription aggregate | No existe módulo de suscripciones ni ciclo de vida persistido. | Modelo técnico, relación con asignaciones y reglas de ciclo de vida aprobadas; consumer idempotente de uso y scheduler siguen pendientes. |
| Billing invoices / invoice lines | PR #12 añade el módulo `platform_billing` con consumo registrado, drafts, líneas, snapshot reproducible y ledger balanceado. | Completar el ciclo comercial solo tras aprobar unidades, precios, periodos, prorrateo, impuestos y reglas de vigencia; integrar consumo/periodicidad y validar contabilidad. |
| Platform Wompi | No existe adapter propio `platform_billing_wompi`. | Cuenta/credenciales Payvia, sandbox, métodos, moneda, autenticación, callbacks oficiales y validación de idempotencia/resultado incierto. |
| Integración fiscal externa | No existe emisor fiscal ni integración runtime en este backend. ADR-0012 y `billing_fiscal_integration.proto` definen la frontera versionada para una invoice interna y el retorno de estado fiscal. | Seleccionar el microservicio/proyecto fiscal; acordar contrato operativo, autenticación, transporte (Outbox/eventos, webhook o API), idempotencia, sandbox y pruebas contractuales. La emisión y los artefactos fiscales pertenecen al sistema externo, no a `platform_billing`. |
| Resend | No existe módulo `notifications` ni adapter `notification_resend`. | Política de notificación, destinatarios, Outbox/delivery state, clave de deduplicación y credenciales. |
| Recaudo / ledger | PR #12 añade `collection_attempts` aislados de los intents de compradores y asientos balanceados para recaudo registrado. | Adapter Wompi de Payvia, callbacks verificados, consultas autoritativas y reconciliación siguen pendientes; no se ejecutan cobros sin decisiones y credenciales aprobadas. |

**Regla de bloqueo económico:** no calcular facturas, tarifas variables, excedentes, prorrateos, impuestos, FX, comisiones, vencimientos ni cargos recurrentes mientras las decisiones aplicables A-EC estén pendientes. Registrar la aprobación real antes de habilitar cada comportamiento.

## 5. Restricciones cross-backend y despliegue

- El cobro a compradores por cuenta de los tenants y el cobro SaaS que Payvia realiza a sus tenants son flujos distintos. El primero pertenece al ownership de Payment Intents/transacciones en `payments` y requiere ejecución protocolar autorizada del gateway; el segundo pertenece a los módulos/adapters de billing de plataforma y usa la cuenta/credenciales propias de Payvia. No se comparten credenciales, ledgers ni ownership. Ninguno se declara operativo por la mera existencia del contrato neutral.
- Plataforma no delega el cobro SaaS al backend de pasarelas.
- El slice actual de Payment Intents todavía no inicia un cobro hacia el gateway; la integración añadida procesa callbacks y reconciliación de transacciones identificadas.
- El gateway valida el callback Wompi antes de publicar. Plataforma solo consume envelopes HMAC firmados; JetStream entrega durablemente y ambos lados soportan mTLS configurable. No se aceptan callbacks arbitrarios ni se confía en headers/IP como identidad del productor.
- No se comparten schemas ni se hacen joins/escrituras SQL entre dominios.
- PostgreSQL es fuente de verdad; Redis no decide importes, autorización ni idempotencia comercial.
- No se afirman adapters propios de Wompi o Resend operativos sin credenciales y pruebas oficiales de sandbox. La emisión fiscal es externa y no se declara integrada ni operativa por la existencia del contrato.
- Despliegue/ejecución en producción queda fuera de esta evidencia de construcción.

## 6. Criterio de salida

Fase 4 solo puede declararse cerrada cuando ambos alcances tengan decisiones aprobadas, implementación completa, pruebas reales positivas/negativas/límites/concurrencia/permiso/aislamiento/estados/fallos/reintentos/integridad/redacción, integraciones externas verificadas y trazabilidad requisito → test → ejecución CI sobre el SHA final. Esta matriz deja explícitamente pendientes los puntos que aún no tienen implementación o evidencia.

## 4.1. Frontera entre invoice interna y documento fiscal externo

**Decisión:** una invoice interna emitida por Payvia no es una factura electrónica válida ante DIAN. Son dos ciclos independientes con ownership, estados y fuentes de verdad separados. Ver [ADR-0012](../adr/ADR-0012-separacion-billing-interno-integracion-fiscal.md).

| Requisito | Contrato / responsabilidad | Estado real |
|---|---|---|
| Invoice interna | `platform_billing` es dueño de invoice interna, partidas inmutables, snapshot, ledger y estado financiero. | El slice inicial está implementado en PR #12; faltan decisiones y reglas comerciales para automatizar su ciclo completo. |
| Evento de salida | `BillingInvoiceIssuedV1`: ID interno estable, tenant, versión/formato del snapshot inmutable, hash y correlación. | Contrato Protobuf y Outbox transaccional implementados; relay JetStream runtime conectado y validado en CI #42752212426. |
| Resultado fiscal externo | `FiscalDocumentStatusChangedV1`: ID interno, ID fiscal externo, estado de integración, referencia de autoridad, código/detalle seguro y correlación. | Existe contrato y soporte de Inbox/deduplicación genérico; no existe endpoint/webhook autenticado ni consumer que procese respuestas del sistema fiscal. |
| Transporte futuro | Outbox/JetStream, webhook autenticado o API, sin acoplar el dominio al proveedor. | Outbox/relay de invoice interna implementado; falta seleccionar el servicio externo y definir, implementar y probar su transporte/autenticación. |
| Idempotencia y correlación | Deduplicar por event ID y por invoice interna + operación/versionado; conservar correlación con documento externo. | El Inbox genérico soporta deduplicación y detección de replay conflictivo; no hay integración runtime ni evidencia de sandbox fiscal. |
| Separación de estados | Estado interno de invoice, estado de pago y estado fiscal externo permanecen independientes. | Decisión documentada. Debe probarse que una respuesta fiscal no modifica ledger, partidas ni estado de pago cuando se implemente el runtime. |

### Criterios de aceptación de esta frontera

- La emisión de la invoice interna no requiere que el servicio fiscal esté disponible.
- Un evento fiscal duplicado o reentregado no crea dos documentos externos ni duplica cambios internos.
- Timeout o falta de respuesta se conserva como resultado desconocido/pendiente; nunca se interpreta automáticamente como aceptación o rechazo.
- Webhooks/resultados externos requieren autenticación criptográfica y autorización de productor; `producer_identity`, headers o IP no son prueba de autenticidad.
- La recepción se persiste de forma durable e idempotente antes de confirmar el ACK.
- La aceptación/rechazo fiscal no altera por sí sola los importes, partidas, ledger ni estado de cobro de la invoice interna.
- La integración se prueba con el servicio fiscal real o su sandbox antes de declararse operativa.

**Fuera de alcance de este backend:** emisión DIAN, habilitación como facturador, numeración/resolución fiscal, firma, XML, CUFE/QR, notas crédito/débito fiscales y credenciales del proveedor fiscal. No crear `invoice_factus` ni marcar facturación electrónica como implementada en Fase 4.

## 5. Alcance A — Billing SaaS Payvia (implementación parcial en PR #12)

| Requisito | Implementación añadida | Evidencia / límite actual |
|---|---|---|
| Ownership y persistencia | Nuevo crate `platform-billing`, schema propio, migración coordinada y RLS forzada. No depende del schema `payments`. | Migración `modules/platform_billing/infrastructure/persistence/migrations/202611080001_platform_billing.sql`; conectado al binario `migrate-platform`. |
| Medición facturable | Registro append-only de eventos de consumo por tenant, deduplicación por sistema/evento de origen y agregación por meter/unidad/periodo. | `BillingService::record_usage` y `summarize_usage`. La ingesta desde el evento `quota.consumed` aún requiere un consumer runtime; no se asume que toda cuota sea facturable. |
| Facturas internas | Draft, partidas con importe en unidades menores, moneda y vínculo opcional a evento de uso. Una invoice emitida conserva partidas, periodo, importes y snapshot. | `BillingService::create_draft`, `add_line`, `issue_invoice`. No se implementa todavía el ciclo de suscripciones, generación periódica ni reglas de pricing/prorrateo sin aprobación. |
| Snapshot reproducible | JSON versionado, hash SHA-256 y replay idempotente de emisión. | Test `usage_invoice_snapshot_ledger_and_financial_immutability_are_enforced`; se compara hash recalculado y almacenado. |
| Ledger | Journal con líneas de débito/crédito, cuentas explícitas, moneda y constraint diferido que rechaza asientos no balanceados. Emisión registra CxC/ingreso; recaudo exitoso registra clearing/CxC. | Test de balance y rollback de journal incompleto. El modelo contable requiere validación funcional/contable antes de contabilizar impuestos u otras partidas no definidas. |
| Outbox | Evento `billing.invoice.issued` en la misma transacción de emisión; eventos de creación y cambio de estado de recaudo; claim con `SKIP LOCKED`, lease/token, ACK y retry con backoff. | Relay JetStream runtime ya está conectado a `platform_runtime`; prueba PostgreSQL/JetStream real valida envelope fiscal versionado, ACK, crash-window ACK→Outbox y deduplicación por `Nats-Msg-Id`. Consumer Inbox externo y webhook autenticado aún pendientes. |
| Inbox | Dedupe durable por tenant + productor + event ID; compara tipo, versión y hash de payload para detectar replay conflictivo. | Pruebas de primera recepción, duplicado exacto y mismo event ID con payload alterado. La autenticación criptográfica del productor/webhook debe realizarse antes de llamar a `receive`; no está implementado el endpoint/consumer externo. |
| Recaudo SaaS | `collection_attempts` dentro de `platform_billing`, una única solicitud por invoice hasta aprobación de política de reintentos, importe/moneda exactos de la invoice, idempotencia y estados forward-only. No FK ni reutilización de Payment Intents de compradores. | Pruebas de concurrencia, replay, bloqueo de nueva clave, transición terminal y journal de recaudo. Adapter Wompi de Payvia, callbacks autenticados y reconciliación siguen pendientes. |
| Aislamiento / recuperación | RLS por tenant y worker scope explícito; transacciones para consumo, invoice, ledger, Outbox/Inbox y collection. | Test PostgreSQL aislado con rol runtime sin BYPASSRLS, aislamiento cross-tenant, concurrencia y rollback de journal desequilibrado; incluido en CI #42752212426 sobre el HEAD de PR #12. |

**Límites deliberados:** no hay precios, tarifas, impuestos, unidades facturables, vencimientos ni prorrateos por defecto. El caller debe suministrar el precio explícito aprobado para cada línea. La facturación fiscal sigue fuera de este backend y se rige por ADR-0012. No se marca el Alcance A como terminado.

## 6. Identidad mTLS de dependencias productivas — decisión acotada

Este cambio operativo exige certificados cliente independientes para PostgreSQL, Redis y NATS, además de la autorización existente de cada dependencia. No implementa ni adelanta gRPC mTLS Plataforma↔Pasarelas, grants o scopes de Fase 5; la puerta de Fase 4 y su estado parcial no cambian.

| Frontera | Diseño registrado antes del cambio | Escenarios de aceptación requeridos |
|---|---|---|
| Plataforma → PostgreSQL | `verify-full`, CA y certificado/clave del rol en URL; `pg_hba.conf` productivo autentica por certificado. Migraciones usan su propio certificado/rol. | Aceptar SAN/CA/CN válidos; rechazar cert ausente, expirado, CA/SAN/CN incorrectos; verificar no conexión sin TLS y no acceso fuera de permisos. |
| Plataforma y workers → Redis | Un único `redis::Client` con CA y certificado cliente explícitos se comparte con `ConnectionManager` y workers, para conservar identidad en reconexiones; producción falla cerrado si falta mTLS. | Aceptar cert y ACL válidos; rechazar identidad ausente/inválida y ACL insuficiente; reinicio y reconexión de Redis sin perder autenticación. |
| Plataforma/Pasarelas → NATS | TLS y certificado individual por backend, junto a usuario y permisos NATS; configuración de producción exige `tls://` y credenciales mTLS completas. | Rechazar cliente sin cert, cert/CA inválidos, usuario/subject no permitido; publicar/consumir y recuperar mensajes tras reinicio con identidad válida. |
| PostgreSQL/Redis exporter → almacén | Cada exporter usa certificado distinto y rol/ACL de observabilidad de solo lectura. Healthcheck Redis usa su propio certificado y permiso `PING`. | Scrapes/healthchecks reales con identidad autorizada; fallar cerrados al retirar cert, CA o privilegio. |
| Frontend → HTTP/BFF → gRPC | Sin cambio de endpoint, contrato, autorización o almacenamiento desde el BFF. | N/A: no se altera ese recorrido. gRPC entre backends permanece pendiente en Fase 5. |
| Outbox / JetStream / realtime | Se mantiene el flujo de eventos existente; el cambio solo configura identidad de transporte y no crea ni modifica estado visible. | N/A: no se añade mutación ni evento de dominio; pruebas de entrega/recovery existentes siguen aplicando. |

La rotación de certificados cliente no se considera automatizada: los archivos se cargan al inicializar clientes/pools. Rotación sin interrupción requiere renovación/reemplazo de las conexiones o despliegue solapado con capacidad de réplicas y drenaje; esa garantía queda pendiente de diseño, infraestructura y pruebas reales. La matriz criptográfica tampoco se considera aprobada hasta ejecutarse contra servicios aislados con certificados reales de prueba. Ver el procedimiento y los límites en [`infra/README.md`](../../../infra/README.md).

## 7. Alcance de continuación de Fase 4 — PR #16

PR #12 conserva el slice implementado y verificable; este PR continúa únicamente los requisitos de Fase 4 que aún no tienen implementación, decisiones aprobadas o evidencia de aceptación. Las tablas de las secciones 3–5 y el backlog de [`ROADMAP.md`](../../ROADMAP.md) siguen siendo la especificación detallada; esta tabla delimita su secuencia y bloqueos, no reduce sus criterios.

| Paquete de trabajo | Incluye | Dependencia / criterio de aceptación |
|---|---|---|
| Aceptación de pagos Wompi existentes | Validar callback, consulta autoritativa y reconciliación del slice actual en el Sandbox oficial. | Cuenta/credenciales de Sandbox autorizadas; ejecutar pruebas reales de discrepancia, timeout y resultado incierto. No usar respuestas simuladas ni declarar proveedor aceptado sin esa evidencia. |
| Cobro a compradores | Implementar el flujo saliente y su operación de proveedor solo después de resolver B-EC-07..13, conservando el ownership de `payments` y la autorización del gateway. | Decisiones comerciales aprobadas, adapter real, contrato cross-backend autorizado y escenarios reales de idempotencia, resultado incierto, reconciliación y no duplicación financiera. Sin decisiones, los comandos y transiciones correspondientes permanecen bloqueados. |
| Suscripciones y facturación de uso Payvia | Ciclo de suscripción/periodos, consumer idempotente de `quota.consumed`, scheduler y emisión periódica de invoices desde la base `platform_billing` de PR #12. | Aprobar las decisiones A-EC aplicables antes de determinar unidades, periodos, precios, excedentes, impuestos o reintentos; verificar persistencia, duplicados/concurrencia, aislamiento, rollback y eventos Outbox/realtime. |
| Recaudo y notificaciones de Payvia | Adapter `platform_billing_wompi`, callback/reconciliación de recaudo propio de Payvia y `notification_resend`. | Cuenta y Sandbox de Payvia; credenciales administradas fuera del código; probar fallos, idempotencia y recuperación con servicios/proveedores reales. Mantenerlo separado del cobro a compradores. |
| Integración fiscal externa | Seleccionar el sistema fiscal dueño y completar el transporte/authenticación del contrato ADR-0012. | Bloqueado hasta que se acuerde el servicio/proyecto externo y su contrato operativo/Sandbox. No implementar `invoice_factus`, emisión DIAN ni artefactos fiscales en plataforma. |
| Recuperación y realtime | Completar pruebas reales de restart/reconexión de Redis/JetStream y consumidor, WebSocket autenticado y recuperación del cliente por cursor/snapshot. | Servicios aislados de desarrollo y comandos/efectos documentados antes de pruebas destructivas; PostgreSQL sigue siendo fuente durable. La aceptación debe verificar idempotencia, permisos, huecos y propagación/correlación. |

**Fuera del alcance de PR #16:** los grants firmados, enforcement cross-backend y mTLS obligatorio de gRPC entre plataforma y pasarelas pertenecen a Fase 5; el despliegue productivo no se autoriza por cerrar este PR. Fase 4 seguirá parcial hasta que cada criterio aplicable de ambos alcances tenga una decisión, implementación y ejecución de aceptación sobre el SHA final.
