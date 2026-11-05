# ADR-0011: Modelo de intentos, transacciones y resultados normalizados — Fase 4

- Estado: resolución técnica adoptada; ingress Wompi firmado y reconciliación cross-backend implementados parcialmente; operación de cobro y sandbox pendientes
- Fecha: 2026-11-03
- Alcance: `bacekend-plataform/modules/payments`
- Relacionada: ADR-0010 (creación/consulta inicial de Payment Intents)
- No autoriza: llamadas a proveedores, captura, cancelación, expiración, reembolsos ni aceptación de callbacks no autenticados

## Contexto

ADR-0010 dejó el Payment Intent inicial en estado `created` y sin intentos ni transacciones. Fase 4 asigna a plataforma el ownership del ciclo de negocio para cobros de compradores de los tenants, mientras el backend de pasarelas es dueño de la ejecución del protocolo de cada proveedor y de la verificación criptográfica de sus callbacks.

Las decisiones B-EC-07..13 siguen sin aprobación. El backend de pasarelas implementa un ingress Wompi con checksum oficial, publicación HMAC firmada y un worker durable de consulta por ID de transacción; plataforma valida el envelope, aplica eventos o abre reconciliación y consume resultados firmados. No existe todavía operación saliente para crear un cobro, vault/cifrado de credenciales ni aceptación oficial en Sandbox. La exigencia mTLS puede activarse por configuración (`PAYVIA_REQUIRE_MTLS=true`); la identidad/grants por comando de Fase 5 siguen pendientes.

## Decisiones técnicas

### 1. Ownership y cardinalidad

- `payments` en plataforma es el dueño del Payment Intent, los Payment Attempts, las Payment Transactions, sus transiciones de negocio, auditoría, reconciliación y eventos de dominio.
- Un Payment Intent puede tener cero o múltiples Payment Attempts. PR #12 permite registrar el primer intento y devolver su replay idempotente; mientras B-EC-10 no apruebe reintentos, una segunda clave/ejecución lógica se rechaza.
- Cada Payment Attempt identifica una ejecución lógica hacia un único proveedor y una cuenta de proveedor, tenant y entorno concretos. Los intentos son registros históricos distintos; un reintento aprobado crea otro intento, no sobrescribe el anterior.
- Un Payment Attempt puede originar cero o varias referencias de Payment Transaction. Cada transacción conserva una identidad interna y la referencia externa opaca de proveedor/cuenta. Esta cardinalidad no supone que captura, reversión o reembolso estén habilitados.
- IDs internos son UUID. Las referencias de proveedor/cuenta son opacas y no se utilizan como autoridad de tenant o autorización.

### 2. Estados canónicos y ownership de normalización

El contrato de plataforma consume estados neutrales; no contiene nombres de estado específicos de Wompi, Bold u otro proveedor. La traducción de códigos/respuestas particulares a este conjunto corresponde al adapter de cada proveedor en el backend de pasarelas.

- **Payment Intent:** `created`, `processing`, `requires_action`, `authorized`, `succeeded`, `outcome_unknown`.
- **Payment Attempt:** `created`, `submitted`, `processing`, `requires_action`, `authorized`, `succeeded`, `failed`, `outcome_unknown`.
- **Payment Transaction:** `pending`, `authorized`, `succeeded`, `failed`, `outcome_unknown`.
- `outcome_unknown` no es un estado informado por el proveedor ni una respuesta negativa: representa que plataforma no pudo conocer con certeza el resultado de una operación. Solo una consulta/reconciliación autoritativa puede resolverlo.
- El fallo de un intento no cierra automáticamente el Payment Intent. La política de reintentos B-EC-10 no está aprobada; por eso el estado `failed` pertenece al intento/transacción y no se proyecta automáticamente como un estado terminal del Payment Intent.
- PR #11 agrega modelos de dominio, guards y el mensaje Protobuf NormalizedProviderPaymentEventV1. PR #12 agrega persistencia/API inicial, ingress de callback Wompi verificado, envelopes HMAC, consumer de inbox y worker de reconciliación. No habilita la operación saliente de cobro ni reemplaza la autorización/grants por comando de Fase 5.

### 3. Guards y orden de eventos

- Las transiciones avanzan por estados explícitos; una regresión no se aplica. El inicio de ejecución (`created → submitted/processing`) requiere autoridad `OperationSubmitted`; un callback no puede inventar que la plataforma envió una operación. `outcome_unknown` solo se introduce por `OperationTimeout` cuando ya existe una operación enviada/en curso.
- `succeeded` y `failed` son terminales para el intento/transacción. Un hecho posterior contradictorio no sobrescribe el estado.
- Un estado `outcome_unknown` solo se resuelve con autoridad de reconciliación, nunca con un callback tardío aislado.
- Para aplicar un evento nuevo, `occurred_at` debe ser estrictamente posterior al último evento aplicado. Un timestamp menor o igual no permite inferir orden; se deriva a reconciliación.
- Un evento tardío se conserva como evidencia de recepción/deduplicación en el inbox durable, pero no altera automáticamente el estado comercial. La reconciliación decide el resultado con consulta autoritativa y guarda auditoría; la entrada del evento permanece bloqueada hasta la autenticación del productor.
- La aplicación concurrente debe usar versión del agregado y transacción PostgreSQL; no se permite decidir orden mediante reloj de proceso, Redis o llegada al worker.

### 4. Eventos normalizados y callback verificado

- El contrato `NormalizedProviderPaymentEventV1` transporta schema version, UUID estable del sobre (`event_id`), proveedor, referencia de cuenta, ID externo estable del evento (`provider_event_id`), tenant, entorno, intent, attempt, referencia de transacción, estado neutral, timestamp de origen, correlation ID y SHA-256 del cuerpo verificado. `event_id` identifica la entrega interna; `provider_event_id` participa en la deduplicación externa.
- No transporta body crudo, secretos, firmas, credenciales ni datos de tarjeta sensibles.
- El schema Protobuf por sí solo **no prueba autenticidad**. Solo puede ser aceptado después de que el adapter del proveedor verifique el callback según el protocolo oficial, anti-replay y asociación de cuenta; además, la identidad del productor debe autenticarse en la frontera entre backends.
- El gateway verifica el checksum oficial del callback Wompi usando las propiedades dinámicas declaradas por el evento, timestamp y secreto de eventos; valida ambiente, referencia del attempt e importe/moneda antes de normalizar. La plataforma acepta únicamente envelopes HMAC firmados por clave/key ID configurados. La configuración NATS puede exigir mTLS; completar identidad de servicio y grants por operación sigue en Fase 5.
- La ingestión de eventos normalizados no queda expuesta como endpoint público de plataforma. El gateway transforma callback Wompi verificado en un evento interno firmado; plataforma vuelve a verificar firma y ownership antes de aplicar. No confiar en `producer_identity`, headers ni IP como prueba de autenticidad.

### 5. Idempotencia, resultado incierto y reconciliación

- La clave de deduplicación externa es tenant + entorno + proveedor + referencia de cuenta + ID estable del evento. No se mezcla con la idempotencia comercial de creación del Payment Intent.
- provider_event_inbox conserva el envelope normalizado, hash del cuerpo original y metadatos necesarios de auditoría; no guarda payload crudo de proveedor.
- Un timeout de resultado incierto no habilita un nuevo cobro. Se conserva el intento con `outcome_unknown` y se abre una necesidad durable de reconciliación.
- La reconciliación consulta al proveedor por una referencia identificable y aplica el resultado dentro de una transacción, con control de versión/duplicado. Si no existe capacidad de consulta confiable, el caso queda en revisión; no se inventa un fallo ni se reintenta el cargo.
- El worker implementado consume órdenes Outbox firmadas y consulta Wompi por `GET /v1/transactions/{id}` cuando existe un ID externo. Verifica referencia `rpay_<attempt_id>`, importe y moneda; sin ID o con evidencia inconclusa el caso queda en `manual_review`. Esta consulta no ejecuta cargos ni habilita reintentos.

### 6. Límites comerciales explícitos

No se agregan estados, comandos ni endpoints para captura, cancelación, expiración o reembolso total/parcial. Quedan bloqueados por B-EC-07..13: flujo de autorización/captura, vigencia, quién cancela y cuándo, reintentos, cambio de proveedor, habilitación y límites de reembolsos. Tampoco se inventan montos máximos, monedas habilitadas, fees, settlement o distribución de costos.

### 7. Cobro a compradores separado del SaaS de Payvia

- Los pagos de compradores pertenecen a `payments` (plataforma decide y persiste el estado de negocio) y se ejecutan técnicamente en adapters del backend de pasarelas por una operación autorizada.
- El cobro de suscripciones/facturas SaaS que Payvia realiza a sus tenants pertenece a `platform_billing_wompi`, con cuenta y credenciales propias de Payvia. No utiliza los adapters ni las credenciales tenant del backend de pasarelas.
- El backend de pasarelas dispone de ingress/lookup Wompi parciales, pero aún no implementa creación de transacción/cobro ni el contrato completo de operación autorizada. El cobro real a compradores no se declara disponible ni se simula en plataforma.

## 8. Persistencia relacional del lifecycle

La rama feat/phase-4-payment-lifecycle-persistence añade una migración aditiva para materializar el ownership ya definido por esta ADR. Sustituye el CHECK de estado de Payment Intent limitado a created por el conjunto canónico, permite incrementar aggregate_version y conserva updated_at/last_event_at. PR #12 conecta repositorios/casos de uso y API para las operaciones iniciales descritas en la sección 9:

- `payments.payment_attempts` conserva tenant/entorno/intent, secuencia positiva, proveedor/cuenta, estado, versión del agregado y último timestamp aplicado. Una clave única serializa la secuencia por Payment Intent.
- `payments.payment_transactions` enlaza intent y attempt mediante claves foráneas compuestas. La relación incluye tenant, entorno, proveedor y cuenta; el importe y moneda se referencian también contra el Payment Intent. Un trigger rechaza crear una transacción cuando el attempt todavía está en `created`.
- `payments.provider_event_inbox` conserva solo el envelope normalizado, hash SHA-256 y metadatos necesarios; deduplica por tenant + entorno + proveedor + cuenta + provider_event_id. No almacena el body crudo, firmas ni secretos.
- `payments.provider_event_deliveries` conserva la traza por entrega. Las entregas aceptadas/duplicadas se vinculan por tenant/entorno al inbox canónico aunque el event_id de una reentrega difiera; las entregas rechazadas pueden conservarse sin crear un registro de evento confiable.
- `payments.payment_reconciliation_cases` y `payment_reconciliation_attempts` almacenan casos pendientes/leased/manual review, leases recuperables e historial de consultas/resultados/evidencia.
- Las seis tablas nuevas aplican ENABLE y FORCE RLS sobre `app.tenant_id`; las relaciones usan tenant y entorno en sus claves. La prueba de integración usa un rol runtime sin BYPASSRLS.

El esquema y los casos de uso iniciales se conectan ahora a un flujo autenticado: el gateway verifica callbacks Wompi y firma el envelope interno; la plataforma verifica HMAC/key ID, valida tenant/entorno/provider/account/monto y aplica el cambio o abre reconciliación dentro de transacciones PostgreSQL. El worker de pasarela consulta por ID de transacción y publica resultados firmados; plataforma los valida y resuelve o conserva el caso en revisión. La migración es aditiva; una reversión de aplicación conserva los registros financieros y no ejecuta DROP automático.

**Estado de persistencia/eventos:** migraciones aditivas e integración de eventos firmados incluidas en PR #12. El test `signed_provider_events_apply_once_and_terminal_conflicts_require_reconciliation` comprueba aplicación, replay, conflicto terminal e integridad HMAC sobre PostgreSQL; debe pasar el CI del HEAD final para considerarse verificado.

## Evidencia y trabajo que queda pendiente

El incremento inicial de dominio de PR #11 contiene:
- modelos Rust `PaymentAttempt` y `PaymentTransaction` con validaciones de identidad/monto;
- enums de estados normalizados y guards de transición/orden;
- contrato Protobuf neutral `NormalizedProviderPaymentEventV1` sin método de ingestión;
- pruebas unitarias de modelos, mapping neutral, transiciones, estado terminal, ownership entre intent/attempt, resultado incierto y orden de timestamps. `transaction_status_is_guarded_and_terminal_state_is_immutable` verifica `Pending → Authorized → Succeeded` y rechaza el cambio contradictorio después del estado terminal. **Verificado en el HEAD `c036f0010fa8140b1a8809ffa2d6be3c3e190f71` por [CI #942](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41753921841)**: límite de 400 líneas, fmt, Clippy, tests del workspace, ambos frontends, E2E tenant/plataforma/multi-réplica y checks documentales.

Continúa pendiente para cerrar el ciclo:
1. Ejecutar CI sobre el HEAD final de PR #12 y corregir cualquier fallo de migración, Rust, contratos, aceptación o documentación.
2. Aceptación oficial Wompi Sandbox del callback y reconciliación, con credenciales/cuenta válidas y vectores/respuestas reales; completar anti-replay temporal y observabilidad operacional.
3. Completar identidad de servicio y grants firmados por comando entre backends (Fase 5); hoy los envelopes de eventos y órdenes usan HMAC/key ID y mTLS configurable sobre JetStream.
4. Completar adapter saliente de creación/consulta de transacción autorizado por plataforma, políticas de reintento aprobadas y verificación productiva de resolución de outcome_unknown. El worker actual consulta únicamente IDs existentes.
5. Aprobación de B-EC-07..13 antes de implementar captura, cancelación, expiración, reintentos comerciales, cambio de proveedor o refunds.
6. Sandbox y credenciales oficiales de proveedor antes de declarar el cobro a compradores operativo.
7. Completar la matriz requisito→test→CI para el HEAD final y los escenarios de proveedor/inbox que continúan bloqueados.

La matriz `docs/phase-4/integration-matrix.md` conserva los estados y la evidencia requisito → test → CI. Esta ADR no declara completa Fase 4 ni una integración de proveedor.


## 9. Casos de uso y API inicial del ciclo de vida

PR #12 agrega la primera superficie operativa que no requiere confiar en un callback externo ni inventar reglas comerciales:

- Crear el primer Payment Attempt desde un Payment Intent created, con idempotencia por tenant + entorno + intent + idempotency key y SHA-256 de la petición normalizada. El parent intent se bloquea para serializar secuencia/creación. Hasta que B-EC-10 autorice reintentos, un segundo intento con otra clave se rechaza; una repetición de la misma clave devuelve el intento original.
- Consultar intentos y transacciones con filtros explícitos de tenant, entorno e intent y límite acotado. Cada lectura autorizada genera auditoría.
- Actualizar el estado inicial por OperationSubmitted (created → processing) y marcar resultado incierto por OperationTimeout (processing → outcome_unknown). Ambas filas (intent y attempt) cambian con control de aggregate_version y dentro de una transacción.
- Cada mutación escribe auditoría y eventos Outbox versionados en la misma transacción que el estado. Un fallo de Outbox revierte el cambio, las versiones y las filas de auditoría.
- outcome_unknown abre un caso durable de reconciliación, vincula una transacción existente cuando la hay y publica payment.reconciliation.requested. La tabla de casos impide más de un caso abierto por attempt.
- La API pública no permite que el tenant declare authorized, succeeded o failed. Estos estados requieren un evento autenticado y normalizado de proveedor o una reconciliación autorizada.

Contratos gRPC agregados:
- CreatePaymentAttempt
- ListPaymentAttempts
- ListPaymentTransactions
- UpdatePaymentIntentStatus

Endpoints REST del BFF:
- POST /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/attempts
- GET /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/attempts
- GET /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/transactions
- PATCH /v1/tenants/{tenant_id}/payment-intents/{payment_intent_id}/status

Scopes independientes:
- payment_attempts.create
- payment_attempts.read
- payment_transactions.read
- payment_intents.update

La matriz de integración registra las pruebas reales añadidas: concurrencia de creación/replay de attempts, deduplicación concurrente de inbox, transición concurrente e idempotente, conflicto de versión, lectura cross-tenant, scope insuficiente, rechazo de estados finales falsificados, rollback inducido por fallo Outbox y ACK de lifecycle event types en JetStream real. La aceptación HTTP recorre BFF → gRPC → identity_access → payments → PostgreSQL. La ejecución CI del HEAD final de PR #12 es obligatoria para verificar este incremento.

### Límites que permanecen cerrados

La tabla provider_event_inbox ya recibe eventos desde el consumer autenticado por HMAC de plataforma, después de que gateway verifica callback Wompi. La columna producer_identity sigue siendo metadato de auditoría y no prueba identidad. No se acepta un evento normalizado arbitrario desde internet; el trust se basa en verificación del proveedor y firma interna, con mTLS configurable en NATS.

El caso de reconciliación se registra de forma durable y el worker consulta Wompi cuando el case contiene una referencia de transacción identificable. Si no hay ID, se deja en revisión manual; el resultado firmado se valida desde la plataforma y nunca se acepta desde una API key del tenant. Falta validar en Sandbox y completar la operación de cobro saliente.

**Estado de PR #12:** código de schema/repositorios/casos de uso/contratos/BFF, ingress Wompi verificado, worker de consulta/reconciliación, consumers firmados y aceptación PostgreSQL añadido. El HEAD final debe pasar CI para confirmar la integración. PR #12 no cierra Fase 4 ni el alcance A de billing SaaS.
