# ADR-0012: Separación entre facturación interna SaaS e integración fiscal externa

- Estado: resolución arquitectónica adoptada
- Fecha: 2026-11-08
- Alcance: backend de plataforma Payvia, contratos de integración de billing y límites de Fase 4
- Relacionadas: ADR-0004 (eventos/Outbox), ADR-0006 (integraciones propias de plataforma), ADR-0010 (idempotencia/Outbox)
- No autoriza: emitir documentos fiscales, transmitir a DIAN, calcular impuestos, definir numeración fiscal ni contratar/habilitar un proveedor fiscal.

## Contexto

Fase 4 incluye el futuro ciclo SaaS por el que Payvia administra suscripciones, periodos, consumo facturable, obligaciones internas, partidas, saldos y recaudo a sus tenants. Ese ciclo no es equivalente a la emisión de una factura electrónica válida ante una autoridad tributaria.

La emisión fiscal debe pertenecer a otro microservicio o proyecto independiente. Puede ser un servicio propio o una API/proveedor externo. Payvia no debe acoplar su dominio financiero a DIAN, a Factus ni a un proveedor concreto, y no debe reconstruir su motor de billing cuando se conecte ese servicio.

## Decisiones

### 1. Dos ciclos distintos y fuentes de verdad independientes

1. **Facturación interna SaaS (Payvia Billing):** plataforma es dueña de suscripciones, periodos, medición facturable, invoices internas, partidas inmutables, snapshots, ledger financiero, estado de cobro y auditoría.
2. **Facturación electrónica/fiscal:** el servicio fiscal externo es dueño de la preparación y emisión del documento fiscal, validación/transmisión ante la autoridad, numeración/autorización fiscal, representación XML/PDF, CUFE/QR, notas y demás obligaciones que le correspondan.
3. Emitir una invoice interna solo significa que Payvia registró una obligación interna según sus reglas comerciales aprobadas. **No significa que exista una factura electrónica válida ante la DIAN ni ante otra autoridad fiscal.**
4. El estado fiscal externo nunca sustituye ni modifica directamente el estado financiero interno. Por ejemplo, aceptación fiscal no prueba que el tenant haya pagado; pago registrado no prueba que el documento fiscal haya sido aceptado.
5. Una factura interna puede existir aunque la integración fiscal esté deshabilitada, pendiente o indisponible. El resultado fiscal se conserva como ciclo relacionado, no como condición implícita para crear o contabilizar la invoice interna.

### 2. Ownership y límites de módulos

- `subscriptions_billing` será dueño del ciclo de suscripción y periodos, sujeto a las decisiones comerciales aprobadas.
- `usage_metering` será dueño de la medición facturable y su procedencia, sin confundir cuotas/entitlements con un ledger financiero.
- `platform_billing` será dueño de la invoice interna, sus partidas inmutables, snapshot reproducible, ledger, estados financieros y eventos de dominio.
- El futuro servicio/proyecto de facturación electrónica será dueño de la emisión fiscal y de sus estados/artefactos fiscales.
- Un adapter/puerto de integración en plataforma podrá transportar snapshots y correlacionar respuestas. No implementará reglas fiscales, generación de XML fiscal ni lógica específica de DIAN.
- `payments` sigue siendo el bounded context de pagos de compradores de los tenants. No se agregan entidades, estados ni endpoints fiscales a ese módulo.
- `platform_billing_wompi`, si se implementa, será únicamente el adapter de recaudo de invoices SaaS de Payvia. El medio de pago no es el emisor fiscal.

### 3. Contrato neutral de integración

La integración se define mediante un contrato versionado independiente de `payments.proto`. El contrato contempla:

- Evento de salida `billing.invoice.issued`, con identificador estable de invoice interna, tenant, versión del snapshot, tipo/formato del snapshot, hash de integridad, fecha de emisión interna y correlación.
- El snapshot es una representación inmutable y versionada de la invoice interna y de los datos de facturación disponibles. Su esquema se versiona explícitamente; no se inventan impuestos, numeración fiscal, datos de emisor/receptor ni reglas comerciales que todavía no estén aprobadas.
- Evento de retorno `fiscal_document.status_changed`, correlacionado con invoice interna y con identificador externo del documento fiscal cuando exista. Los estados externos se mantienen separados de los estados de invoice y de pago internos.
- El contrato no obliga a usar un transporte específico: podrá entregarse por Outbox/JetStream, webhook autenticado o API. El adapter podrá soportar más de una modalidad sin cambiar el dominio de billing.
- Los eventos incluyen versión de esquema, event ID, correlation ID y referencias estables. Los consumidores deben tolerar entrega al menos una vez y deduplicar.

El contrato Protobuf inicial se define en `crates/platform_contracts/proto/billing_fiscal_integration.proto`. Su existencia define una frontera de interoperabilidad; no significa que exista ya un módulo de suscripciones, invoices, un emisor fiscal o una integración runtime.

### 4. Consistencia, seguridad y recuperación

- El evento de invoice emitida se publicará mediante Outbox en la misma transacción que persista la invoice interna y sus partidas, cuando el caso de uso de billing esté implementado.
- La recepción de resultados externos usará Inbox durable e idempotente antes de aplicar cambios al registro de integración fiscal.
- La integración deberá autenticar el sistema productor y verificar firma/audience cuando use webhooks o mensajería cross-service. Un campo `producer_identity`, un header o una IP no prueban autenticidad.
- Debe existir correlación estable entre `internal_invoice_id`, `event_id` y `external_fiscal_document_id`; el identificador externo no reemplaza el ID interno.
- Los reintentos y la redelivery no pueden producir documentos fiscales duplicados. La idempotencia debe ser acordada por contrato con el servicio fiscal y reforzada por el consumidor.
- Errores, rechazos, timeouts y resultados desconocidos se conservan de forma durable y pueden reconciliarse. Un timeout no se interpreta como rechazo ni como aceptación.
- Los payloads, logs y auditoría no deben exponer credenciales, secretos, firmas ni datos sensibles innecesarios. Las credenciales del proveedor fiscal pertenecen al servicio que realiza la emisión, no al módulo `platform_billing`.

### 5. Estados separados

El estado interno de invoice (por ejemplo, borrador/emitida/anulada según las reglas aprobadas) pertenece a `platform_billing`. El estado del documento fiscal pertenece al sistema fiscal externo y se refleja como estado de integración independiente, por ejemplo:

- `not_requested`
- `pending`
- `accepted`
- `rejected`
- `delivery_failed`
- `unknown`

Estos nombres describen el ciclo de integración, no un catálogo normativo DIAN ni estados fiscales exhaustivos. Su evolución deberá ser compatible con el servicio fiscal conectado. No se permite mapear automáticamente `accepted` a pagada ni `rejected` a invoice interna anulada.

### 6. Alcance y exclusiones

Fase 4 implementa el motor de facturación interna y deja la frontera de integración preparada. No implementa en este backend:

- Emisión o transmisión de factura electrónica ante DIAN.
- Habilitación como facturador electrónico, numeración/resolución, firma, CUFE, QR, XML fiscal ni representación fiscal.
- Notas crédito/débito fiscales ni eventos tributarios.
- Integración runtime con Factus u otro proveedor fiscal, hasta que exista una decisión y un proyecto/servicio dueño de esa responsabilidad.
- Políticas económicas o tributarias no aprobadas: precios, unidades facturables, impuestos, retenciones, prorrateos, vencimientos, numeración y reglas de redondeo.

La conexión futura puede implementarse mediante eventos, webhooks o API, con el contrato versionado como frontera. El servicio fiscal podrá ser reemplazado sin alterar el dominio de pagos ni reconstruir el motor SaaS.

## Consecuencias

- Las invoices internas de Payvia pueden existir y mantener su integridad aunque no haya servicio fiscal conectado.
- El servicio fiscal puede evolucionar/desplegarse independientemente y atender más de un sistema consumidor si su propio alcance lo permite.
- La plataforma mantiene solo el estado/correlación necesarios para conocer el resultado fiscal externo; no replica el ownership de los artefactos y procesos tributarios.
- La matriz de Fase 4 debe probar la independencia de ambos ciclos y la resiliencia del contrato cuando se implemente el runtime. La definición documental/Protobuf actual no acredita esa aceptación runtime.
- No se marca facturación electrónica como implementada ni como entregable de cierre de Fase 4.

## Criterios de aceptación para la futura integración runtime

1. Publicar el evento desde Outbox junto con la persistencia de invoice/partidas.
2. Probar entrega duplicada, orden, timeout, indisponibilidad, redelivery y recuperación sin duplicar el documento externo.
3. Autenticar y verificar webhooks/resultados externos; probar firma inválida, replay, audiencia incorrecta y productor no autorizado.
4. Persistir Inbox, estado de integración, correlación y auditoría de forma atómica.
5. Demostrar que cambios en estado fiscal no alteran importes, partidas, estado de pago ni ledger financiero interno.
6. Probar tenant isolation, autorización, redacción, compatibilidad/versionado y recuperación manual/automática.
7. Ejecutar pruebas contractuales contra el servicio fiscal real o sandbox oficial antes de declarar una integración operativa.

---

**Regla normativa del proyecto:** una invoice interna emitida por Payvia no constituye por sí misma una factura electrónica válida ante la DIAN. Son dos ciclos independientes y con ownership separado.
