# ADR-0011: Modelo de intentos, transacciones y resultados normalizados — Fase 4

- Estado: resolución técnica del dominio adoptada; integración operativa pendiente
- Fecha: 2026-11-03
- Alcance: `bacekend-plataform/modules/payments`
- Relacionada: ADR-0010 (creación/consulta inicial de Payment Intents)
- No autoriza: llamadas a proveedores, captura, cancelación, expiración, reembolsos ni aceptación de callbacks no autenticados

## Contexto

ADR-0010 dejó el Payment Intent inicial en estado `created` y sin intentos ni transacciones. Fase 4 asigna a plataforma el ownership del ciclo de negocio para cobros de compradores de los tenants, mientras el backend de pasarelas es dueño de la ejecución del protocolo de cada proveedor y de la verificación criptográfica de sus callbacks.

Las decisiones B-EC-07..13 siguen sin aprobación. Además, el backend de pasarelas no implementa todavía adapters ni callback ingress; y la identidad mTLS/grants entre backends sigue asignada a Fase 5. Esta ADR resuelve únicamente decisiones técnicas que pueden fijarse sin inventar esas políticas ni debilitar la frontera de confianza.

## Decisiones técnicas

### 1. Ownership y cardinalidad

- `payments` en plataforma es el dueño del Payment Intent, los Payment Attempts, las Payment Transactions, sus transiciones de negocio, auditoría, reconciliación y eventos de dominio.
- Un Payment Intent puede tener cero o múltiples Payment Attempts. Mientras no exista una política comercial aprobada, el modelo no autoriza iniciar un intento ni reintentar automáticamente.
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
- Esta entrega agrega modelos de dominio y guards, y el mensaje Protobuf `NormalizedProviderPaymentEventV1`. No agrega todavía migraciones, persistencia, RPC de ingestión ni consumidor de eventos; no debe anunciarse como ciclo operativo implementado.

### 3. Guards y orden de eventos

- Las transiciones avanzan por estados explícitos; una regresión no se aplica. El inicio de ejecución (`created → submitted/processing`) requiere autoridad `OperationSubmitted`; un callback no puede inventar que la plataforma envió una operación. `outcome_unknown` solo se introduce por `OperationTimeout` cuando ya existe una operación enviada/en curso.
- `succeeded` y `failed` son terminales para el intento/transacción. Un hecho posterior contradictorio no sobrescribe el estado.
- Un estado `outcome_unknown` solo se resuelve con autoridad de reconciliación, nunca con un callback tardío aislado.
- Para aplicar un evento nuevo, `occurred_at` debe ser estrictamente posterior al último evento aplicado. Un timestamp menor o igual no permite inferir orden; se deriva a reconciliación.
- Un evento tardío se conserva como evidencia de recepción/deduplicación cuando exista el inbox durable, pero no altera automáticamente el estado comercial. La reconciliación decide el resultado con consulta autoritativa y guarda auditoría.
- La aplicación concurrente debe usar versión del agregado y transacción PostgreSQL; no se permite decidir orden mediante reloj de proceso, Redis o llegada al worker.

### 4. Eventos normalizados y callback verificado

- El contrato `NormalizedProviderPaymentEventV1` transporta schema version, UUID estable del sobre (`event_id`), proveedor, referencia de cuenta, ID externo estable del evento (`provider_event_id`), tenant, entorno, intent, attempt, referencia de transacción, estado neutral, timestamp de origen, correlation ID y SHA-256 del cuerpo verificado. `event_id` identifica la entrega interna; `provider_event_id` participa en la deduplicación externa.
- No transporta body crudo, secretos, firmas, credenciales ni datos de tarjeta sensibles.
- El schema Protobuf por sí solo **no prueba autenticidad**. Solo puede ser aceptado después de que el adapter del proveedor verifique el callback según el protocolo oficial, anti-replay y asociación de cuenta; además, la identidad del productor debe autenticarse en la frontera entre backends.
- La verificación criptográfica de callbacks pertenece al backend de pasarelas (su fase de webhooks). La autenticación mTLS/grants plataforma↔pasarelas pertenece a Fase 5. Ninguno de esos trabajos se implementa en esta ADR.
- Mientras ambas condiciones no existan, la plataforma no expone RPC/HTTP ni consumer que acepte este mensaje desde una fuente no autenticada. El contrato se define, pero la ingestión queda bloqueada.

### 5. Idempotencia, resultado incierto y reconciliación

- La clave de deduplicación externa será tenant + entorno + proveedor + referencia de cuenta + ID estable del evento. No se mezcla con la idempotencia comercial de creación del Payment Intent.
- El futuro inbox persistirá solo campos normalizados, hash del cuerpo original y metadatos necesarios de auditoría; no guardará payload crudo de proveedor.
- Un timeout de resultado incierto no habilita un nuevo cobro. Se conserva el intento con `outcome_unknown` y se abre una necesidad durable de reconciliación.
- La reconciliación consulta al proveedor por una referencia identificable y aplica el resultado dentro de una transacción, con control de versión/duplicado. Si no existe capacidad de consulta confiable, el caso queda en revisión; no se inventa un fallo ni se reintenta el cargo.
- La API/worker de consulta de proveedor no se implementa hasta disponer de adapter/capacidad verificada y la frontera de autorización correspondiente.

### 6. Límites comerciales explícitos

No se agregan estados, comandos ni endpoints para captura, cancelación, expiración o reembolso total/parcial. Quedan bloqueados por B-EC-07..13: flujo de autorización/captura, vigencia, quién cancela y cuándo, reintentos, cambio de proveedor, habilitación y límites de reembolsos. Tampoco se inventan montos máximos, monedas habilitadas, fees, settlement o distribución de costos.

### 7. Cobro a compradores separado del SaaS de Payvia

- Los pagos de compradores pertenecen a `payments` (plataforma decide y persiste el estado de negocio) y se ejecutan técnicamente en adapters del backend de pasarelas por una operación autorizada.
- El cobro de suscripciones/facturas SaaS que Payvia realiza a sus tenants pertenece a `platform_billing_wompi`, con cuenta y credenciales propias de Payvia. No utiliza los adapters ni las credenciales tenant del backend de pasarelas.
- El backend de pasarelas continúa siendo scaffold sin adapters operativos; por tanto, el cobro real a compradores no se declara disponible ni se simula en plataforma.

## 8. Persistencia relacional del lifecycle

La rama `feat/phase-4-payment-lifecycle-persistence` añade una migración aditiva para materializar el ownership ya definido por esta ADR:

- `payments.payment_attempts` conserva tenant/entorno/intent, secuencia positiva, proveedor/cuenta, estado, versión del agregado y último timestamp aplicado. Una clave única serializa la secuencia por Payment Intent.
- `payments.payment_transactions` enlaza intent y attempt mediante claves foráneas compuestas. La relación incluye tenant, entorno, proveedor y cuenta; el importe y moneda se referencian también contra el Payment Intent. Un trigger rechaza crear una transacción cuando el attempt todavía está en `created`.
- `payments.provider_event_inbox` conserva solo el envelope normalizado, hash SHA-256 y metadatos necesarios; deduplica por tenant + entorno + proveedor + cuenta + provider_event_id. No almacena el body crudo, firmas ni secretos.
- `payments.provider_event_deliveries` conserva la traza por entrega. Las entregas aceptadas/duplicadas se vinculan por tenant/entorno al inbox canónico aunque el event_id de una reentrega difiera; las entregas rechazadas pueden conservarse sin crear un registro de evento confiable.
- `payments.payment_reconciliation_cases` y `payment_reconciliation_attempts` almacenan casos pendientes/leased/manual review, leases recuperables e historial de consultas/resultados/evidencia.
- Las seis tablas nuevas aplican ENABLE y FORCE RLS sobre `app.tenant_id`; las relaciones usan tenant y entorno en sus claves. La prueba de integración usa un rol runtime sin BYPASSRLS.

El esquema por sí solo no autentica productores, no verifica callbacks, no procesa transiciones, no ejecuta consultas de proveedor ni resuelve resultados inciertos. No se expone una entrada de eventos ni se inicia reconciliación hasta implementar las fronteras autenticadas y las capacidades reales. La migración es aditiva; una reversión de aplicación conserva los registros financieros y no ejecuta DROP automático.

**Estado de esta ampliación:** código de migración y aceptación añadidos en `feat/phase-4-payment-lifecycle-persistence`; ejecución CI pendiente. No se declara todavía como verificada.

## Evidencia y trabajo que queda pendiente

Esta entrega contiene únicamente:
- modelos Rust `PaymentAttempt` y `PaymentTransaction` con validaciones de identidad/monto;
- enums de estados normalizados y guards de transición/orden;
- contrato Protobuf neutral `NormalizedProviderPaymentEventV1` sin método de ingestión;
- pruebas unitarias de modelos, mapping neutral, transiciones, estado terminal, ownership entre intent/attempt, resultado incierto y orden de timestamps. `transaction_status_is_guarded_and_terminal_state_is_immutable` verifica `Pending → Authorized → Succeeded` y rechaza el cambio contradictorio después del estado terminal. **Verificado en el HEAD `c036f0010fa8140b1a8809ffa2d6be3c3e190f71` por [CI #942](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41753921841)**: límite de 400 líneas, fmt, Clippy, tests del workspace, ambos frontends, E2E tenant/plataforma/multi-réplica y checks documentales.

Continúa pendiente para cerrar el ciclo:
1. La migración aditiva y la aceptación de esquema están añadidas en `feat/phase-4-payment-lifecycle-persistence` y pendientes de CI; siguen pendientes repositorios/casos de uso, aplicación transaccional de estados, auditoría/Outbox del lifecycle y procesamiento del inbox.
2. Use cases y API de consulta de intentos/transacciones, autorización/scope, auditoría y Outbox/realtime.
3. Operación autorizada plataforma→gateway, cuando corresponda, tras completar la frontera de confianza asignada.
4. Callback verification/anti-replay y publicación del evento desde adapters reales del backend de pasarelas.
5. Consumer autenticado en plataforma, pruebas cross-backend reales y reconciliación contra sandbox/cuenta oficial.
6. Aprobación de B-EC-07..13 antes de implementar captura, cancelación, expiración, reintentos comerciales, cambio de proveedor o refunds.
7. Sandbox y credenciales oficiales de proveedor antes de declarar el cobro a compradores operativo.

La matriz `docs/phase-4/integration-matrix.md` conserva los estados y la evidencia requisito → test → CI. Esta ADR no declara completa Fase 4 ni una integración de proveedor.
