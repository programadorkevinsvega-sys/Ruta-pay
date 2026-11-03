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

- El evento payment_intent.created se escribe en la misma transacción local que el agregado.
- El relay reclama filas con leases y FOR UPDATE SKIP LOCKED. Antes de publicar renueva el lease usando el token propietario; un worker que perdió el token no puede renovar, reprogramar, poner en cuarentena ni marcar como publicado el evento.
- Cada publicación y su ACK tienen timeout individual de 10 segundos, dentro del lease renovado de 30 segundos. Esto limita la duración de una operación bloqueada; una publicación cuyo resultado sea incierto puede repetirse y sigue siendo al menos una vez.
- JetStream usa Nats-Msg-Id derivado del event_id. El ACK se confirma en PostgreSQL solo después del ACK de JetStream y únicamente para el token vigente.
- Errores transitorios liberan el lease y programan un backoff exponencial con jitter, acotado a 300 segundos. Los errores permanentes del envelope/contrato registran failed_at y last_error_code, liberan el lease y excluyen el evento de claims automáticos. La fila y su payload permanecen durables para diagnóstico; no se descartan silenciosamente.
- Los eventos en cuarentena requieren inspección/acción operativa explícita; no existe una API de redrive automática en este slice.
- El envelope publica el timestamp persistido del Outbox, no la hora de reintento/publicación.
- El consumidor realtime admite payment_intent.created; persiste el envelope en el journal y emite la señal Redis antes de confirmar el ACK de JetStream.
- La aceptación integrada payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime verifica publish/ACK real, timestamp original, persistencia en journal, replay por cursor tenant, aislamiento y crash-window ACK→confirmación Outbox con republicación deduplicada.
- Las pruebas outbox_lease_expiry_is_reclaimed_and_stale_owner_is_rejected, outbox_retries_after_stream_becomes_available_with_jittered_backoff, concurrent_relays_publish_all_claimed_events_without_losing_rows y permanent_outbox_errors_are_quarantined_without_retrying_forever verifican leases, token obsoleto, recuperación tras rechazo real de JetStream, backoff, concurrencia y cuarentena sobre PostgreSQL/JetStream reales.
- Las pruebas no simulan una caída física/reinicio del proceso NATS ni certifican reconexión del socket después de ese reinicio. Esa brecha permanece explícita junto con restart del consumidor y WebSocket autenticado.

## Consecuencias y límites

- El slice ofrece crear/consultar Payment Intents de forma autenticada, aislada e idempotente.
- No crea transacciones ni intentos, no modifica el estado más allá de `created`, no selecciona proveedor y no llama al gateway.
- No implementa captura, cancelación, expiración, refund, settlement, fees ni conciliación.
- No define importe mínimo/máximo comercial, moneda habilitada, método permitido por proveedor ni flujo de autorización/captura.
- La aceptación HTTP→BFF→gRPC y las denegaciones por scope/tenant/entorno están verificadas en `payment_intents_http_acceptance.rs`.
- La integración publish/ACK Outbox→JetStream→journal/cursor y su crash-window de republicación deduplicada están verificadas en `payment_intent_outbox_is_published_deduplicated_and_recoverable_in_realtime` con dependencias reales.
- Siguen pendientes los fallos de PostgreSQL/timeout, outage y restart NATS/consumer, backoff exhaustivo, WebSocket autenticado, y el ciclo de vida de transacciones/intentos. La matriz de `docs/phase-4/integration-matrix.md` conserva la evidencia y los límites.
- Cualquier expansión del ciclo de vida requiere ampliar esta ADR o una ADR nueva, definir estados/transiciones y añadir pruebas de aceptación con dependencias reales antes de declarar su comportamiento operativo.
