# Fase 4 — Matriz de integración y evidencia

**Estado:** implementación parcial. Esta matriz describe el alcance que existe en el HEAD del PR; no declara aceptación ni cierre de Fase 4.

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

| Área | Diseño actual | Estado |
|---|---|---|
| Payment Intent | Entidad persistida con status inicial `created`, importe entero minor-unit, currency, método, tenant y entorno. | Base implementada |
| Transaction / Attempt | No existe aún tabla ni agregado para intentos y transacciones. | Pendiente |
| Máquina de estados | La migración solo admite `created`; no existe todavía transición terminal/prohibida versionada. | Pendiente |
| Provider mapping / normalized result | ADR-0005 define el contrato conceptual del método; no hay mapping versionado de estados externos en este módulo. | Pendiente |
| Webhook/callback verificado | No existe ingreso de resultados externos de transacción en `payments`. | Pendiente |
| Resultado incierto y reconciliación | No se repite ninguna llamada de proveedor porque este slice todavía no invoca proveedores. El procedimiento de consulta/reconciliación externa no está implementado. | Pendiente |
| Refund / partial refund / capture | Sus políticas económicas/funcionales están pendientes en B-EC-07 y B-EC-12..14. | Bloqueado por aprobación; no implementar supuestos |

No se puede interpretar el estado inicial `created` como pago autorizado, capturado, exitoso o liquidado.

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

- Plataforma no delega el cobro SaaS al backend de pasarelas.
- El slice actual de Payment Intents no llama al gateway. La falta del mecanismo de mTLS/grants de Fase 5 no se sustituye con headers, IP allowlists, permisos locales ni mocks.
- No se comparten schemas ni se hacen joins/escrituras SQL entre dominios.
- PostgreSQL es fuente de verdad; Redis no decide importes, autorización ni idempotencia comercial.
- No se afirman adaptadores Wompi, Factus o Resend operativos sin credenciales y pruebas oficiales de sandbox.
- Despliegue/ejecución en producción queda fuera de esta evidencia de construcción.

## 6. Criterio de salida

Fase 4 solo puede declararse cerrada cuando ambos alcances tengan decisiones aprobadas, implementación completa, pruebas reales positivas/negativas/límites/concurrencia/permiso/aislamiento/estados/fallos/reintentos/integridad/redacción, integraciones externas verificadas y trazabilidad requisito → test → ejecución CI sobre el SHA final. Esta matriz deja explícitamente pendientes los puntos que aún no tienen implementación o evidencia.
