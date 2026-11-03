# ADR-0010: Ownership e idempotencia de Payment Intents — slice inicial de Fase 4

- Estado: Aceptada para el slice inicial de creación y consulta de Payment Intents
- Fecha: 2026-11-03
- Alcance: `bacekend-plataform`, bounded context `payments`
- No cubre: ejecución de proveedores, intentos/transacciones, refunds, suscripciones ni billing SaaS

## Contexto

La Fase 4 asigna a plataforma el ownership del estado de negocio de Payment Intents y transacciones de compradores. El backend de pasarelas mantiene el ownership de los protocolos técnicos y credenciales de proveedor. La primera entrega debe ofrecer una intención persistida y consultable sin adelantar el enforcement cross-backend de Fase 5 ni inventar decisiones comerciales todavía pendientes.

## Decisiones

### Ownership y frontera

- El bounded context `payments` en plataforma es dueño del agregado Payment Intent, sus claves de idempotencia para creación, auditoría y Outbox.
- `identity_access` conserva el ownership de API Keys, sus digests, scopes, estado y revocación.
- El BFF es la única frontera REST. Llama al módulo `payments` por gRPC y no consulta su schema.
- La autorización de creación/lectura se comprueba en `identity_access` con los scopes exactos `payment_intents.create` y `payment_intents.read`.
- Esta ADR no autoriza una llamada al backend de pasarelas. mTLS, grants firmados y enforcement de la operación protegida continúan asignados a Fase 5.

### Modelo inicial del agregado

- El identificador interno es UUID.
- El agregado pertenece a un tenant y a un entorno `test` o `live`; ambos forman parte de la identidad de lectura.
- El importe comercial se almacena como entero positivo en unidades menores (`amount_minor`). No se usa punto flotante.
- La moneda se valida sintácticamente como tres letras ASCII mayúsculas. Esto no constituye una lista de monedas habilitadas para proveedores/mercados.
- El método inicial se valida como discriminador `card`, `pse` o `crypto`. Los detalles específicos de cada método, settlement y su contrato wire completo no quedan resueltos por este slice.
- El único estado persistido permitido actualmente es `created`. Crear una intención no significa que exista un intento externo ni que el pago esté autorizado, capturado, exitoso o liquidado.
- `merchant_reference` es opcional y se conserva como referencia opaca del comercio; esta entrega no le atribuye unicidad ni semántica comercial.

### Idempotencia de creación

La clave se identifica por la tupla:

`(tenant_id, environment, operation, idempotency_key)`

La operación actual tiene el nombre versionado `payment_intent.create.v1`.

- PostgreSQL impone unicidad sobre esa tupla.
- La huella SHA-256 se calcula sobre una estructura serializada explícita que contiene tenant, entorno, importe, moneda, tipo de método y merchant reference. La clave no se incluye en el contenido de la huella porque ya forma parte de la clave única.
- La creación de idempotency record, Payment Intent, snapshot de respuesta, auditoría y Outbox ocurre en una transacción PostgreSQL.
- Ante la misma clave y la misma huella se devuelve el snapshot original persistido, no una reconstrucción a partir del estado actual.
- Ante la misma clave y una huella distinta se devuelve conflicto.
- La transacción y la constraint única coordinan solicitudes concurrentes; no se usa Redis como autoridad de idempotencia.
- No se define expiración ni borrado automático de claves en esta entrega. La política de retención sigue pendiente de decisión técnica/financiera aplicable.

### Aislamiento y auditoría

- Cada operación establece `app.tenant_id` en la transacción y las tablas de intent, idempotencia y auditoría aplican RLS forzada.
- Las lecturas filtran además por tenant, entorno e ID.
- La auditoría conserva actor API Key, acción, recurso, resultado, correlación, trace ID y detalles acotados.
- El secreto de API Key no se almacena en el módulo de pagos ni se escribe en auditoría, Outbox o logs.

### Outbox

- El evento `payment_intent.created` se escribe en la misma transacción local que el agregado.
- El relay reclama filas con leases y `FOR UPDATE SKIP LOCKED`, publica con JetStream `Nats-Msg-Id` y marca como publicado solo después del ACK.
- Los errores liberan el lease y programan retry con backoff. La entrega es al menos una vez; los consumidores deben deduplicar por `event_id`.
- El envelope publica el timestamp persistido del Outbox, no la hora de reintento/publicación.
- Esta ADR no declara completada la integración con journal realtime ni la recuperación por WebSocket.

## Consecuencias y límites

- El slice ofrece crear/consultar Payment Intents de forma autenticada, aislada e idempotente.
- No crea transacciones ni intentos, no modifica el estado más allá de `created`, no selecciona proveedor y no llama al gateway.
- No implementa captura, cancelación, expiración, refund, settlement, fees ni conciliación.
- No define importe mínimo/máximo comercial, moneda habilitada, método permitido por proveedor ni flujo de autorización/captura.
- La aceptación específica de HTTP→BFF→gRPC, denegaciones por scope, fallos PostgreSQL/JetStream, redelivery, y journal/realtime sigue pendiente y está registrada en `docs/phase-4/integration-matrix.md`.
- Cualquier expansión del ciclo de vida requiere ampliar esta ADR o una ADR nueva, definir estados/transiciones y añadir pruebas de aceptación con dependencias reales antes de declarar su comportamiento operativo.
