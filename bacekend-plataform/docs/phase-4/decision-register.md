# Fase 4 — Registro formal de decisiones pendientes

| Campo | Valor |
|---|---|
| Producto | Payvia |
| Componente | backend de plataforma (`bacekend-plataform`) |
| Fecha de corte | 2026-11-03 |
| Base auditada | `main` — `eb13e4fef024a3428460dbdc260bf3d254bb0eb3` |

**Estado:** decisiones pendientes; este documento no constituye aprobación comercial ni cierre de fase.

## 1. Propósito

Este registro separa las decisiones que requieren aprobación del responsable de producto, comercial, financiero o legal de las decisiones de diseño que puede resolver el arquitecto de software dentro de las reglas ya aprobadas del repositorio.

Su objetivo es que la implementación de Fase 4 no invente tarifas, impuestos, calendarios, obligaciones, permisos comerciales, capacidades de proveedores ni comportamientos financieros.

Una decisión no se considera aprobada por aparecer en un ejemplo, fixture, tipo de código, contrato conceptual o documentación de proveedor.

### 1.1. Autoridad y estados

| Clasificación | Quién decide | Regla |
|---|---|---|
| **APROBACIÓN FUNCIONAL** | Responsable de producto/comercial | Define el comportamiento del producto y sus condiciones comerciales. |
| **APROBACIÓN FINANCIERA/FISCAL** | Responsable financiero/contable y, cuando aplique, asesoría legal | Define importes, impuestos, tratamiento contable y obligaciones documentales. |
| **RESOLUCIÓN ARQUITECTÓNICA** | Arquitecto de software | Define modelos, persistencia, contratos, consistencia, concurrencia y mecanismos técnicos sin cambiar reglas comerciales. |
| **VALIDACIÓN EXTERNA** | Responsable de la integración con el proveedor, con aprobación funcional cuando afecte al producto | Comprueba documentación oficial, sandbox, capacidades y habilitación de la cuenta. |

Estados permitidos para mantener este registro:

- **Pendiente:** no existe decisión aprobada/evidencia suficiente.
- **Aprobado:** existe aprobación explícita y queda registrada su evidencia.
- **Resuelto técnicamente:** existe decisión arquitectónica documentada y revisable.
- **Bloqueado externamente:** depende de una cuenta, credencial, sandbox, contrato o confirmación del proveedor.
- **No aplicable:** requiere justificación concreta, previa y verificable.

No se asignan valores ni se seleccionan alternativas en nombre de los responsables.

## 2. Alcance de esta Fase 4

Este registro conserva exactamente los dos alcances de la Fase 4 del roadmap de plataforma:

### Alcance A — Suscripciones y facturación de uso Payvia

- Ciclo de suscripción, periodos y cambios de plan.
- Medición del consumo que pueda generar cargos.
- Cálculo de obligaciones de Payvia a sus tenants.
- Facturas financieras y partidas inmutables.
- Cobro de facturas de Payvia mediante `platform_billing_wompi`.
- Documento electrónico mediante `electronic_invoicing` / `invoice_factus`.
- Notificaciones mediante `notifications` / `notification_resend`.
- Estados financieros, idempotencia, reintentos y conciliación.
- Eventos durables y recuperación de estado cuando corresponda.

### Alcance B — Ownership del ciclo de negocio de Payment Intents y transacciones

- Ownership en plataforma de Payment Intents y transacciones de los compradores de los tenants.
- Autenticación y autorización comercial mediante API Keys.
- Idempotencia comercial.
- Modelo de transacciones, intentos y transiciones de negocio.
- Contratos normalizados con el backend de pasarelas.
- Persistencia de resultados verificados y eventos de negocio.
- Reconciliación y protección contra duplicación de efectos financieros.

### 2.1. Límites explícitos

1. El cobro de Payvia a sus tenants no es un pago de comprador procesado por el backend de pasarelas.
2. El ownership de facturas, ledger y estado financiero SaaS no se transfiere a Wompi, Factus ni Resend.
3. El backend de pasarelas ejecuta protocolos de proveedores; no gobierna tenants, planes, cuotas ni la decisión comercial de autorización.
4. No se cambia la numeración ni el alcance de otros roadmaps del repositorio.
5. Este documento no autoriza adelantar las fases independientes de webhooks, dashboard, hardening u operación.
6. Ninguna integración puede declararse operativa solo porque exista una interfaz, un cliente tipado o un contrato conceptual.

## 3. Base ya definida que no se debe volver a decidir

Las siguientes reglas están respaldadas por los documentos y el código existentes. Se registran como restricciones de diseño, no como decisiones pendientes.

### 3.1. Separación financiera y ownership

| Responsabilidad | Ownership establecido |
|---|---|
| Suscripciones, invoices financieras, partidas, ledger e idempotencia de billing | `subscriptions_billing` / `platform_billing` en plataforma |
| Recaudo de obligaciones propias de Payvia | `platform_billing_wompi` en plataforma, con cuenta y credenciales de Payvia |
| Estado y referencias fiscales de documentos electrónicos de Payvia | `electronic_invoicing` |
| Integración inicial de facturación electrónica | `invoice_factus` |
| Políticas y estado de notificaciones | `notifications` |
| Integración inicial de correo | `notification_resend` |
| Payment Intents, transacciones y transiciones de negocio de pagos de compradores | `payments` en plataforma |
| Protocolo técnico de cada pasarela | Adapter del proveedor en backend de pasarelas |
| Credenciales de cuentas de los tenants para pasarelas | `gateway_credentials` en backend de pasarelas |

### 3.2. Restricciones económicas y de datos ya existentes

- Los precios del catálogo son versionados y las versiones publicadas son inmutables.
- El catálogo guarda moneda, importe entero en unidades menores, periodo, intervalo y vigencias explícitas.
- El catálogo actual valida formato de moneda de tres letras, pero no demuestra que una moneda esté soportada por cada proveedor.
- Entitlements define capacidades, límites, unidad, entorno y política de excedente `deny` o `allow`.
- El ledger de uso es durable y PostgreSQL es su fuente de verdad.
- La política `allow` permite superar una cuota; **no define una tarifa ni autoriza por sí misma un cobro por excedente**.
- El ledger actual de consumo no equivale a una factura ni a un ledger financiero de suscripciones.
- El importe comercial de una transacción se representa como entero en unidades menores.
- El settlement cripto se representa separadamente como decimal en texto; no reemplaza el importe comercial ni usa punto flotante.
- El contrato conceptual `payment_method` contempla `card`, `pse` y `crypto`, pero los campos definitivos de wire/Protobuf/OpenAPI deben formalizarse.
- No se almacenan ni registran PAN, CVV ni datos de banda.

### 3.3. Restricciones de arquitectura ya aceptadas

- PostgreSQL es la fuente de verdad de los datos financieros.
- Cada bounded context posee su schema, migraciones, repositorios y Outbox; no se permiten consultas o escrituras SQL cruzadas entre dominios.
- Una mutación de negocio y su Outbox se confirman en una transacción local.
- JetStream entrega al menos una vez; no se promete exactly-once de extremo a extremo.
- Los consumidores deben ser idempotentes y confirmar después de persistir el efecto.
- Redis no es autoridad financiera ni de autorización; solo se usa para estado efímero/fan-out cuando corresponda.
- Los cambios durables visibles siguen Outbox → JetStream → realtime autenticado → recuperación REST autorizada, salvo N/A justificado previamente.
- Los módulos de dominio dependen de contratos neutrales al proveedor; los adapters se registran en el composition root.
- Desactivar un proveedor no elimina facturas, ledger, auditoría ni referencias históricas.
- Los secretos se inyectan desde configuración protegida; no se almacenan en tablas de dominio, repositorios, eventos, logs ni respuestas.

## 4. Alcance A — Decisiones económicas que requieren aprobación

**Todas las decisiones de esta sección están pendientes salvo que se agregue evidencia de aprobación en el apartado 8.**

| ID | Decisión | Pregunta que debe resolverse | Autoridad |
|---|---|---|---|
| A-EC-01 | Modelo de monetización | ¿La obligación SaaS se compone de tarifa fija, consumo variable o una combinación? | Producto/Comercial |
| A-EC-02 | Unidades facturables | ¿Qué capacidades, operaciones o métricas generan cargos? | Producto/Comercial |
| A-EC-03 | Consumo incluido | ¿Qué cantidad de cada unidad se incluye en cada versión de plan? | Producto/Comercial |
| A-EC-04 | Tarifas unitarias | ¿Qué precio corresponde a cada unidad facturable y bajo qué versión/vigencia? | Comercial/Financiera |
| A-EC-05 | Excedentes | ¿Cómo se cobran, limitan o tratan comercialmente las unidades que superan lo incluido? | Comercial |
| A-EC-06 | Entorno `test` | ¿El consumo de pruebas genera cargos, tiene límites separados o recibe otro tratamiento? | Comercial |
| A-EC-07 | Operaciones no exitosas | ¿Qué tratamiento reciben operaciones rechazadas, fallidas, canceladas, duplicadas o inciertas? | Producto/Comercial |
| A-EC-08 | Periodicidad comercial | ¿La facturación se realiza por adelantado, por consumo vencido o mediante otro ciclo aprobado? | Comercial/Financiera |
| A-EC-09 | Anclaje del periodo | ¿Qué fecha determina inicio, corte y renovación de cada suscripción? | Comercial |
| A-EC-10 | Calendario | ¿Qué regla comercial se aplica a meses de distinta duración, años bisiestos y periodos parciales? | Comercial |
| A-EC-11 | Moneda de facturación | ¿Qué monedas se habilitan para facturar a cada tenant y bajo qué condiciones? | Comercial/Financiera |
| A-EC-12 | Conversión de moneda | Si se permite conversión, ¿qué fuente, fecha, tasa y responsabilidad se aplican? | Financiera/Comercial |
| A-EC-13 | Precisión y redondeo | ¿Qué reglas se aplican a cálculos variables, subtotales, impuestos y totales? | Financiera |
| A-EC-14 | Impuestos | ¿Qué impuestos, bases, tratamientos y reglas fiscales aplican a cada obligación? | Financiera/Legal |
| A-EC-15 | Descuentos y créditos | ¿Qué descuentos, créditos promocionales o ajustes comerciales existen y cómo se aplican? | Comercial/Financiera |
| A-EC-16 | Cambios de plan | ¿Cuándo entra en vigor un upgrade/downgrade y cómo se trata la diferencia económica? | Comercial |
| A-EC-17 | Prorrateo | ¿Se aprueba? En caso afirmativo, ¿cuál es la regla de cálculo y sus casos límite? | Comercial/Financiera |
| A-EC-18 | Cancelación | ¿Cuándo es efectiva y qué obligaciones o acceso permanecen después de solicitarla? | Comercial |
| A-EC-19 | Impago | ¿Existe periodo de gracia? ¿Cuándo se suspende, limita o reactiva la cuenta? | Comercial/Financiera |
| A-EC-20 | Facturas de importe cero | ¿Se emiten, se omiten o reciben otro tratamiento financiero/fiscal? | Financiera |
| A-EC-21 | Ajustes y correcciones | ¿Cómo se corrigen facturas emitidas, saldos a favor, pagos duplicados y sobrepagos? | Financiera/Legal |
| A-EC-22 | Recaudo recurrente | ¿Se autoriza el cobro posterior/recurrente y bajo qué consentimiento y modalidad? | Comercial/Legal |
| A-EC-23 | Métodos de pago de la suscripción | ¿Qué métodos se ofrecen a los tenants para pagar sus obligaciones con Payvia? | Comercial |
| A-EC-24 | Vencimiento | ¿Qué vencimiento comercial aplica a cada obligación y cuándo se considera morosa? | Comercial/Financiera |

**Regla de bloqueo:** no se implementa un cálculo de factura o tarifa variable mientras sus unidades, base, precio, periodo, moneda y redondeo aplicables no estén aprobados.

## 5. Alcance A — Decisiones técnicas que resuelve el arquitecto

| ID | Decisión técnica | Resultado que debe documentarse |
|---|---|---|
| A-AR-01 | Modelo de suscripción | Agregado, identidad, ciclo de vida y relación con tenant y versión de precio |
| A-AR-02 | Relación con asignaciones actuales | Integración con `tenant_plan_assignments` sin duplicar ownership de entitlements |
| A-AR-03 | Periodos y calendario | Representación temporal, límites inclusivos/exclusivos, zona de referencia y cálculo determinista conforme a política aprobada |
| A-AR-04 | Medición facturable | Modelo de lectura/registro de consumo y origen de cada unidad |
| A-AR-05 | Procedencia del consumo | Referencia a operación/recurso que permite auditar la medición sin duplicar el ledger de pagos |
| A-AR-06 | Snapshot económico | Congelar precio, versión, cantidades, moneda y parámetros utilizados para producir cada obligación |
| A-AR-07 | Modelo de invoice | Cabecera, partidas, estados, identificadores y relaciones financieras |
| A-AR-08 | Inmutabilidad | Separar borrador/cálculo de obligación emitida/confirmada; corregir mediante operaciones compensatorias según política aprobada |
| A-AR-09 | Ledger financiero | Movimientos, referencias, unicidad e integridad transaccional |
| A-AR-10 | Cálculo reproducible | Poder reconstruir el cálculo con datos históricos y versión de reglas |
| A-AR-11 | Idempotencia de billing | Scope, huella, unicidad, persistencia del resultado y recuperación concurrente |
| A-AR-12 | Estados de suscripción | Máquina de estados y guards derivados de las políticas comerciales aprobadas |
| A-AR-13 | Estados de invoice y pago | Separar obligación, intento de cobro, resultado de proveedor y documento fiscal |
| A-AR-14 | Wompi plataforma | Contrato neutral, adapter propio, configuración y referencias externas |
| A-AR-15 | Resultado incierto | Consulta/reconciliación antes de repetir una operación financiera de resultado desconocido |
| A-AR-16 | Callbacks de cobro Payvia | Vinculación segura a invoice/intent de billing, autenticidad, deduplicación y transición válida |
| A-AR-17 | Factus | Contrato fiscal separado, OAuth/tokens, estados externos, referencias e idempotencia |
| A-AR-18 | Resend | Contrato de notificación, Outbox, deduplicación, errores clasificados y estado recuperable |
| A-AR-19 | Reintentos | Clasificación retryable por operación/proveedor; no repetir efectos inciertos sin reconciliar |
| A-AR-20 | Auditoría | Registrar quién/qué originó cambios financieros y conservar referencias verificables |
| A-AR-21 | Migraciones | Schemas, roles mínimos, constraints, índices, RLS cuando aplique y compatibilidad |
| A-AR-22 | Contratos | gRPC/Protobuf tipado y versionado; REST/OpenAPI donde aplique |
| A-AR-23 | Observabilidad | Correlación de operación, DB, proveedor, Outbox y consumidor; redacción de secretos/PII |
| A-AR-24 | Retención y limpieza | Política técnica de conservación/depuración coherente con obligaciones legales y financieras aprobadas |

La resolución técnica puede seleccionar mecanismos e invariantes, pero debe dejar explícito qué regla comercial aprobada implementa. No puede rellenar ninguna decisión de la sección 4.

## 6. Alcance B — Decisiones económicas y funcionales que requieren aprobación

| ID | Decisión | Pregunta que debe resolverse | Autoridad |
|---|---|---|---|
| B-EC-01 | Importe comercial | ¿Qué representa el importe exigible al comprador y qué conceptos incluye? | Producto/Comercial |
| B-EC-02 | Monedas | ¿Qué monedas se ofrecen por mercado, tenant, proveedor y método de pago? | Comercial |
| B-EC-03 | Fees de Payvia | ¿Payvia cobra una tarifa por transacción? ¿Qué base y condiciones tiene? | Comercial/Financiera |
| B-EC-04 | Costos del proveedor | ¿Cómo se asignan o muestran los costos de las pasarelas? | Comercial/Financiera |
| B-EC-05 | Responsable de cargos | ¿Qué parte asume cada comisión, costo o impuesto? | Comercial/Financiera |
| B-EC-06 | Límites comerciales | ¿Qué montos mínimos/máximos aplican por método, moneda, cuenta y proveedor? | Comercial |
| B-EC-07 | Autorización/captura | ¿Qué flujo comercial se ofrece por método y proveedor? | Producto/Comercial |
| B-EC-08 | Vigencia del intento | ¿Cuánto tiempo puede permanecer abierta una intención y qué consecuencia comercial tiene su expiración? | Producto |
| B-EC-09 | Cancelación | ¿Quién puede solicitarla, cuándo es efectiva y qué condiciones aplican? | Producto/Comercial |
| B-EC-10 | Reintentos del comprador | ¿Cuándo se permite volver a intentar un pago y qué experiencia se presenta? | Producto |
| B-EC-11 | Cambio de pasarela | ¿Se permite cambiar de proveedor para la misma intención y bajo qué condiciones? | Producto/Comercial |
| B-EC-12 | Reembolsos | ¿Se habilitan reembolsos? ¿Totales, parciales o ambos? | Comercial/Financiera |
| B-EC-13 | Reembolsos múltiples | ¿Qué límites y condiciones se aplican sobre el importe original? | Comercial/Financiera |
| B-EC-14 | Diferencias de settlement | ¿Cómo se tratan diferencias entre importe comercial y monto liquidado? | Financiera |
| B-EC-15 | Cripto | ¿Qué activos, monedas, redes y condiciones comerciales están habilitados, si aplica? | Comercial/Financiera |
| B-EC-16 | Impuestos del pago | ¿Qué responsabilidades fiscales aplican al importe y a los cargos asociados? | Financiera/Legal |
| B-EC-17 | Datos del comprador | ¿Qué datos son necesarios y con qué propósito se solicitan/comparten? | Producto/Legal |

La aprobación de estos puntos no implica que todos los métodos o proveedores tengan una capacidad técnica equivalente.

## 7. Alcance B — Decisiones técnicas que resuelve el arquitecto

| ID | Decisión técnica | Resultado que debe documentarse |
|---|---|---|
| B-AR-01 | Agregado Payment Intent | Identidad, ownership, invariantes y ciclo de vida en `payments` de plataforma |
| B-AR-02 | Relación con transacciones | Cardinalidad entre intención, intentos y transacciones; historial sin sobrescrituras ambiguas |
| B-AR-03 | Identificadores | IDs internos, referencias del comercio, IDs externos y claves de correlación |
| B-AR-04 | Máquina de estados | Estados de intent y transacción, guards, transiciones, terminalidad y actores permitidos |
| B-AR-05 | Eventos tardíos | Tratamiento de respuestas/callbacks que llegan después de cancelación, expiración o resultado previo |
| B-AR-06 | Mapeo externo | Tabla versionada de estados/errores por proveedor a resultados normalizados |
| B-AR-07 | API Keys | Integrar autenticación existente, tenant, entorno, revocación y scopes; no ampliar permisos implícitamente |
| B-AR-08 | Matriz de scopes | Relacionar cada endpoint/operación con el permiso requerido |
| B-AR-09 | Idempotencia comercial | Scope, huella versionada, canonicalización, unicidad, concurrencia y replay del resultado original |
| B-AR-10 | Operación en curso | Representar la reclamación y estado persistido de una solicitud concurrente/en progreso |
| B-AR-11 | Timeout y resultado incierto | Proceso de recuperación y consulta externa antes de repetir efectos |
| B-AR-12 | Intentos | Persistencia de cada intento, proveedor elegido, referencia externa y resultado técnico |
| B-AR-13 | Dinero | Tipo monetario entero de unidades menores, moneda explícita y validación de rango |
| B-AR-14 | Settlement cripto | Decimal como string y campos separados del importe comercial, sin punto flotante |
| B-AR-15 | Método de pago | Contrato discriminado versionado; entrada y salida separadas cuando sus datos difieran |
| B-AR-16 | Descriptor de proveedor | Capacidades verificadas por cuenta/entorno/método/operación; no inferir capacidades |
| B-AR-17 | Contrato inter-backend | RPC tipado, versión, deadlines, errores, contexto tenant/entorno/cuenta y trazabilidad |
| B-AR-18 | Seguridad de frontera | Consumir únicamente mecanismo de autorización realmente disponible y verificado; no crear bypass local |
| B-AR-19 | Eventos normalizados | `event_id`, tipo/versión, agregado/versión, productor, origen, audiencia, referencias y trazabilidad |
| B-AR-20 | Deduplicación | Claves técnicas para eventos externos y eventos internos sin confundir sus ámbitos |
| B-AR-21 | Outbox y consumers | Persistencia atómica local, entrega al menos una vez, ACK después de persistir y redelivery idempotente |
| B-AR-22 | Errores | Taxonomía estable de validación, autenticación, autorización, conflicto, proveedor, timeout y persistencia |
| B-AR-23 | Reconciliación | Consultar y resolver resultados inciertos sin duplicar cobros ni alterar estados terminales indebidamente |
| B-AR-24 | Aislamiento | Restricciones tenant/entorno en consultas, claves únicas, RLS y pruebas contra PostgreSQL real |
| B-AR-25 | Auditoría y redacción | Evidencia de autorización/transición sin secretos, PAN/CVV ni payloads crudos sensibles |
| B-AR-26 | Compatibilidad | Evolución compatible de contratos Protobuf/OpenAPI y eventos versionados |

### 7.1. Alcance de autorización y dependencia de Fase 5

ADR-0002 exige identidad de servicio, mTLS, autorización por método/scope y concesiones firmadas ligadas a la operación. La matriz de aceptación de Fase 3 establece que mTLS, signed grants y enforcement del gateway pertenecen a Fase 5.

Por tanto:

- Fase 4 no puede declarar una operación cross-backend segura basándose solamente en que existe un cliente gRPC tipado.
- Fase 4 no puede introducir un bypass, permiso local equivalente ni confianza por IP/header.
- El arquitecto debe definir los contratos y los puntos de integración compatibles con ADR-0002.
- La aceptación de operaciones protegidas debe depender de un mecanismo real, fail-closed y verificable.
- La implementación/enforcement formal de mTLS y grants se conserva en la fase que el roadmap asigna a esa capacidad; este registro no cambia Fase 5.

Si la dependencia impide ejecutar una aceptación completa, debe registrarse como bloqueo explícito. No se reemplaza con mocks ni con una declaración de conformidad.

## 8. Registro de aprobaciones

Este apartado se mantiene vacío hasta recibir aprobaciones reales. No se deben completar nombres, fechas, importes ni políticas por inferencia.

| ID/rango | Decisión aprobada | Responsable/rol | Fecha | Evidencia (ADR, acta o referencia) |
|---|---|---|---|---|
| Pendiente | | | | |

Una aprobación parcial debe identificar exactamente qué decisión y qué escenarios cubre. No se extiende automáticamente a otros tenants, monedas, proveedores o métodos.

## 9. Registro de resoluciones arquitectónicas

| ID/rango | Resolución técnica | Responsable | Documento/ADR | Evidencia de revisión |
|---|---|---|---|---|
| B-AR-01 (slice inicial) | `payments` posee el agregado Payment Intent; UUID interno y estado inicial `created`. No implica una transacción ni define el ciclo de vida completo. | Arquitectura de plataforma | [ADR-0010](../adr/ADR-0010-payment-intent-idempotency-and-outbox.md) | Código de dominio y migración propia del módulo. |
| B-AR-07 / B-AR-08 (create/read) | API Keys siguen bajo `identity_access`; creación/lectura exigen respectivamente `payment_intents.create` y `payment_intents.read`. No se amplían scopes. | Arquitectura de plataforma | [ADR-0010](../adr/ADR-0010-payment-intent-idempotency-and-outbox.md) | `payment_intents_http_path_enforces_auth_scope_isolation_and_idempotency` atraviesa HTTP→BFF→gRPC→identidad→PostgreSQL y verifica creación/replay/conflicto/lectura, bearer ausente, API key revocada, scope insuficiente, tenant y entorno. **Frontera create/read probada; no adelanta mTLS/grants de Fase 5.** |
| B-AR-09 (create) | Idempotencia por tenant + entorno + operación versionada + clave; huella SHA-256 de campos explícitos; replay devuelve snapshot original y payload diferente produce conflicto. | Arquitectura de plataforma | [ADR-0010](../adr/ADR-0010-payment-intent-idempotency-and-outbox.md) | Constraint PostgreSQL, transacción y test real con replay/conflicto/12 solicitudes concurrentes. |
| B-AR-13 (importe inicial) | Importe positivo como entero en unidades menores; moneda validada sintácticamente como tres letras mayúsculas, sin declarar monedas habilitadas comercialmente. | Arquitectura de plataforma | [ADR-0010](../adr/ADR-0010-payment-intent-idempotency-and-outbox.md) | Dominio, constraint SQL y tests unitarios. |
| B-AR-21 (producer/relay y resiliencia) | Outbox transaccional con lease propietario renovable, timeout de publish/ACK, entrega al menos una vez, backoff exponencial con jitter y cuarentena durable de errores de contrato permanentes. | Arquitectura de plataforma | [ADR-0010](../adr/ADR-0010-payment-intent-idempotency-and-outbox.md) | Tests reales: outbox_lease_expiry_is_reclaimed_and_stale_owner_is_rejected; outbox_retries_after_stream_becomes_available_with_jittered_backoff; concurrent_relays_publish_all_claimed_events_without_losing_rows; permanent_outbox_errors_are_quarantined_without_retrying_forever; aceptación publish/ACK + crash-window en jetstream.rs. **Verificado:** [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260), SHA `1858766a46a5b92d6b6b75207e728ddaa6933498`, todos los jobs aprobados (detección, YAML, documentación y quality). |
| B-AR-24 / B-AR-25 (create/read) | RLS forzada y filtros por tenant/entorno; auditoría por API Key, correlación y detalles acotados sin secreto. | Arquitectura de plataforma | [ADR-0010](../adr/ADR-0010-payment-intent-idempotency-and-outbox.md) | Test PostgreSQL real de separación tenant/entorno y auditoría. |

| B-AR-02 / B-AR-03 (modelo y persistencia inicial) | Un Payment Intent mantiene múltiples Payment Attempts históricos; cada intento conserva tenant, entorno, proveedor/cuenta, secuencia y clave idempotente; las transacciones se relacionan mediante claves compuestas y no sobrescriben historial. | Arquitectura de plataforma | [ADR-0011](../adr/ADR-0011-payment-lifecycle-and-normalized-provider-events.md) | PR #12 añade migración aditiva para attempts/transactions, FK compuestas, unicidad, RLS ENABLE+FORCE y casos de uso de creación/listado. Aceptación PostgreSQL y HTTP→BFF→gRPC ejecutada en [CI #964](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41937106523), SHA `e92123be7bb7762d08fe537e42997a466146824e`. **Persistencia y API inicial verificadas; el segundo intento con clave nueva permanece bloqueado por B-EC-10.** |
| B-AR-04 / B-AR-05 (guards y transiciones iniciales) | Estados neutrales separados para intent, attempt y transaction; guards forward-only y terminalidad inmutable. El fallo de un intento no cierra automáticamente el intent mientras B-EC-10 no defina reintentos. Los eventos externos tardíos o contradictorios requieren inbox autenticado y reconciliación autoritativa; no se aplican por llegada. | Arquitectura de plataforma | [ADR-0011](../adr/ADR-0011-payment-lifecycle-and-normalized-provider-events.md) | PR #12 conecta transiciones iniciales `created → processing → outcome_unknown` con control de versión, auditoría, Outbox y caso durable de reconciliación; aceptación de concurrencia, replay y rollback ejecutada en [CI #964](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41937106523), SHA `e92123be7bb7762d08fe537e42997a466146824e`. **Aplicación inicial verificada; aplicación de callbacks tardíos sigue bloqueada hasta autenticación de productor y adapter verificador.** |
| B-AR-06 / B-AR-19 (normalización) | El backend de pasarelas debe traducir estados/códigos de cada proveedor a resultados neutrales; `outcome_unknown` sigue siendo una condición interna de plataforma, nunca un estado reportado por proveedor. | Arquitectura de plataforma + adaptadores de pasarela | [ADR-0011](../adr/ADR-0011-payment-lifecycle-and-normalized-provider-events.md) | `NormalizedProviderPaymentEventV1` y el enum neutral existen en `payments.proto`; el inbox PostgreSQL y la deduplicación están implementados en PR #12. **No hay mapping operativo por proveedor ni RPC/consumer de ingreso: el árbol de `bacekend-pasarela` solo contiene el scaffold y el cliente de entitlements, sin adapters, callback verifier ni runtime de webhooks.** CI #964 verifica el slice de persistencia/API, no una integración de proveedor. |
| B-AR-10 / B-AR-11 / B-AR-12 (intentos y resultado incierto) | El intento se persiste antes de cualquier ejecución futura. Un timeout de una operación enviada pasa a `outcome_unknown`; ningún timeout autoriza un segundo cobro. La resolución requiere consulta autoritativa correlacionada con la misma cuenta/proveedor/tenant/entorno e importe/moneda. | Arquitectura de plataforma | [ADR-0011](../adr/ADR-0011-payment-lifecycle-and-normalized-provider-events.md) | PR #12 verifica persistencia/idempotencia de attempts y transición transaccional a `outcome_unknown`, con caso de reconciliación y Outbox durable ([CI #964](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41937106523), SHA `e92123be7bb7762d08fe537e42997a466146824e`). **Pendiente:** worker de consulta y resolución, bloqueado por ausencia de adapters/capacidad de consulta en pasarelas y autenticación cross-backend de Fase 5. Reintentos comerciales con otra clave siguen bloqueados por aprobación B-EC-10; replay de la misma clave devuelve el intento original. |
| B-AR-18 / B-AR-20 / B-AR-23 (confianza, deduplicación y reconciliación) | Solo se consume un evento después de verificar el callback en el adapter y autenticar el productor en la frontera cross-backend. La deduplicación externa es tenant + entorno + proveedor + cuenta + `provider_event_id`; eventos tardíos/conflictivos no sobrescriben estado. | Arquitectura de plataforma; callback en pasarelas; identidad de servicio en Fase 5 | [ADR-0011](../adr/ADR-0011-payment-lifecycle-and-normalized-provider-events.md) | PR #12 añade inbox/delivery ledger, constraints de ownership, RLS y deduplicación concurrente; verificado con PostgreSQL real en [CI #964](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41937106523). `producer_identity` continúa siendo solo metadato de auditoría. **Ingreso de eventos sigue fail-closed y sin endpoint/consumer**, porque `bacekend-pasarela` aún no tiene callback verifier/adapter y la autenticación mTLS/grants está asignada a Fase 5. La reconciliación durable se crea; el worker de consulta autoritativa aún no existe. |

Estas resoluciones aplican al slice de creación/consulta y al modelo de dominio inicial de lifecycle definido en ADR-0011. No acreditan persistencia de intentos/transacciones, ingestión verificada de callbacks, procesamiento de eventos ni reconciliación operativa. Capture/cancel/expiration/refunds siguen bloqueados por B-EC-07..13 y el cobro a compradores no es operativo mientras los adapters y la frontera de pasarelas no existan.

Las decisiones que alteren límites de bounded contexts, ownership financiero, seguridad o contratos compartidos deben registrarse mediante ADR y revisión correspondiente.

## 10. Dependencias y bloqueos conocidos

1. **Puertas de fases anteriores:** el estado verificado del repositorio mantiene abiertas brechas de auditoría en Fase 0, runtime/observabilidad en Fase 1 y aceptación formal de Fase 3. Este registro no cambia ni cierra esos criterios.
2. **Fase 5 de plataforma:** mTLS, grants firmados y enforcement cross-backend tienen su asignación propia. Fase 4 no los declara implementados ni los sustituye.
3. **Backend de pasarelas:** su README indica que continúa como scaffold sin adapters/migraciones/servicio operativo completo. Las pruebas de integración reales dependen de su implementación y contratos correspondientes.
4. **Wompi plataforma:** la cuenta, credenciales sandbox, métodos, monedas y modalidad de cobro deben validarse para la cuenta concreta de Payvia.
5. **Factus:** los contratos fiscales, autenticación, ambiente y capacidades deben comprobarse con documentación oficial y credenciales apropiadas.
6. **Resend:** la semántica de idempotencia del proveedor no sustituye Outbox ni estado durable propio.
7. **Política económica:** sin aprobación no se calculan tarifas, excedentes, impuestos, prorrateos, comisiones ni vencimientos.
8. **Numeración de fases:** la Fase 4 de este documento es exclusivamente la Fase 4 del backend de plataforma; no renumera la Fase 4 del roadmap del backend de pasarelas.

## 11. Criterio de preparación de implementación

Un caso de uso puede pasar a implementación cuando:

- Su regla funcional/económica aplicable está aprobada y referenciada.
- Su ownership y matriz de integración están definidos.
- El arquitecto ha documentado modelos, contratos, persistencia, idempotencia, errores, seguridad y recuperación aplicables.
- Las dependencias externas necesarias tienen contrato/capacidad verificable o bloqueo explícito.
- Los casos de prueba cubren caminos positivos/negativos, tipos inválidos, límites, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción, según aplique.
- Existe trazabilidad requisito → decisión → caso de prueba → ejecución CI.
- Las integraciones reales usan dependencias aisladas y sandbox oficial cuando corresponda; no se sustituyen por mocks para declarar aceptación.

Una decisión pendiente bloquea únicamente el comportamiento que depende de ella, sin inventar un valor alternativo. No se marca una capacidad como finalizada mientras una regla aplicable carezca de aprobación o evidencia.

## 12. Referencias de propiedad documental

- [Roadmap del backend de plataforma](../../ROADMAP.md)
- [Reglas del backend de plataforma](../../PROJECT_RULES.md)
- [Bounded contexts](../phase-0/bounded-contexts.md)
- [Matriz de integración de Fase 0](../phase-0/integration-matrix.md)
- [Threat model](../phase-0/threat-model.md)
- [ADR-0002 — Autorización y credenciales cross-backend](../adr/ADR-0002-autorizacion-y-credenciales-cross-backend.md)
- [ADR-0003 — PostgreSQL y migraciones](../adr/ADR-0003-postgresql-y-migraciones.md)
- [ADR-0004 — gRPC, eventos, observabilidad y realtime](../adr/ADR-0004-gRPC-eventos-observabilidad-realtime.md)
- [ADR-0005 — Contrato normalizado de pago](../adr/ADR-0005-contrato-normalizado-de-pago.md)
- [ADR-0006 — Integraciones propias de plataforma](../adr/ADR-0006-integraciones-propias-de-plataforma.md)
- [ADR-0010 — Ownership e idempotencia inicial de Payment Intents](../adr/ADR-0010-payment-intent-idempotency-and-outbox.md)
- [Aceptación de Fase 3](../phase-3/acceptance.md)
- [Estado verificado del proyecto](../../../PROJECT_STATUS.md)

---

**Nota de control:** este documento registra decisiones pendientes y restricciones existentes. No constituye una aprobación de precios, impuestos, comisiones, modalidades de cobro, contratos de proveedor ni políticas comerciales. Tampoco certifica el inicio, aceptación o cierre de Fase 4.
