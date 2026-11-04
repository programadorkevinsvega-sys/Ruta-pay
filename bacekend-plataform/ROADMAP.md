# Roadmap del Backend de Plataforma Payvia

## Flujo de ramas y revisión

`main` debe mantenerse funcional y verificable. Desarrolla funcionalidades, cambios arquitectónicos, correcciones complejas y modificaciones de seguridad en ramas descriptivas; presenta los cambios importantes en Pull Request con resumen, decisiones, verificaciones ejecutadas y riesgos conocidos. Los cambios triviales de documentación pueden hacerse en `main` cuando las protecciones lo permitan. No se permite force push ni reescribir historial compartido. No integres si fallan verificaciones obligatorias; los cambios sensibles requieren revisión manual.

> Este roadmap cubre únicamente el backend que gobierna tenants, planes, precios, límites, cuotas, suscripciones, facturación de Payvia, identidad, autorización y sus adapters propios de Wompi, Factus y Resend. El backend de pasarelas tiene su propio [README](../bacekend-pasarela/README.md) y [roadmap](../bacekend-pasarela/ROADMAP.md) para los pagos que los comercios procesan a sus compradores.
>
> Los backends comparten el workspace Cargo raíz solo para resolución/build. Cada uno tiene binario, imagen, configuración, secretos, red, artefactos operativos y despliegue independiente en su propia VM/VPS; los dos frontends también tienen despliegues separados. Los artefactos de esta aplicación pertenecen a `bacekend-plataform/deploy/`.
>
> Estado verificado 2026-11-03: las implementaciones de Fases 0–3 están integradas en main. PR #3, #4, #5 y #7 están fusionadas y sus CI correspondientes terminaron success. Esto no elimina los huecos que las auditorías de Fase 0 y Fase 1 todavía enumeran. PR #6 hace obligatoria la matriz de pruebas por caso de uso. No se declara conformidad total de una fase mientras quede un criterio aplicable sin evidencia. Ver PROJECT_STATUS.md para el estado consolidado.

## Autoridad de dominio

Plataforma es la fuente de verdad de tenant lifecycle, identidad/permisos, planes/precios versionados, entitlement, límites/cuotas, suscripciones, facturas y decisión de autorización. El servicio de pasarelas no puede cambiar esos datos ni inferir permiso localmente.

Las llamadas síncronas usan contratos gRPC internos autenticados. Plataforma decide autorización antes de invocar `ExecuteAuthorizedGatewayOperation` y liga una concesión firmada de corta duración a identidad de servicio, tenant, entorno, cuenta, operación, capacidades requeridas, monto/moneda, plan/versión, límites, `jti`, expiración y trazas. Pasarelas valida criptográficamente esa concesión y cada claim. No hay callback RPC circular durante la operación. Timeout de autorización, respuesta incompleta o incompatibilidad significan denegación. Los cambios y eventos entre backends tienen contratos versionados y entrega durable Outbox/JetStream cuando corresponda.

## Criterios obligatorios para cada fase y subfase

Antes de implementar una subfase, documentar su requisito, ownership y matriz de integración: PostgreSQL/schema/migración; frontend→REST/OpenAPI→BFF; BFF→módulo dueño por gRPC; interacción gRPC con backend de pasarelas; uso concreto de Redis; productores/consumidores NATS/Outbox; realtime y recuperación por REST/cursor. Cada componente no aplicable requiere justificación previa, específica y comprobable.

En la misma subfase se implementan y prueban ambos lados de los contratos afectados. Errores de validación, autenticación/autorización, conflicto, timeout/conectividad, persistencia y dependencias aplicables deben tener tipos/códigos, diagnóstico seguro, mapeo y recuperación documentados. Toda aceptación verifica evento específico de fallo y cierre HTTP con igual `correlation_id`, `trace_id` y código/categoría; incluye redacción automática de datos sensibles. Cada cambio persistido visible usa Outbox versionado → JetStream → realtime autenticado → TanStack Query → recuperación REST, salvo justificación concreta registrada previamente.

Pruebas unitarias y de propiedades aplican a la lógica; integración/aceptación ejecuta PostgreSQL, Redis y NATS reales aislados cuando corresponda y cruza frontend/API/BFF/gRPC/módulo. No se permiten fixtures, seeds o respuestas simuladas para sustituir integración real. Redis y consumers JetStream deben demostrar reconexión/re-suscripción o recuperación durable. Cada fase queda completa antes de iniciar la siguiente y registra comandos, resultados y bloqueos reales.

## Fase 0 — Auditoría, decisiones y workspace

### Aceptación obligatoria de casos de uso y auditoría de pruebas

Para cada caso de uso de esta fase, documentar y automatizar caminos positivos y negativos, entradas inválidas/tipos/límites, duplicados y concurrencia, permisos/aislamiento, estados, fallos de dependencias, integridad de persistencia/Outbox y redacción, según aplique. Cada caso debe enlazar requisito→escenario→test→ejecución CI. Fase 8 no sustituye estas pruebas.

Auditoría de pruebas: [`docs/phase-0/test-audit.md`](docs/phase-0/test-audit.md). PR #3 está fusionada e incorporó la auditoría documental y su check estático de CI. La trazabilidad automatizada amenaza→control→prueba y la verificación semántica siguen pendientes; el check estático no demuestra esas propiedades.


- [x] Auditar scaffold, dependencias, configuración, CI y documentación. Evidencia: [`docs/phase-0/initial-audit.md`](docs/phase-0/initial-audit.md).
- [x] Registrar ADRs de contratos gRPC, concesiones firmadas, migraciones coordinadas, identidad de servicio, operación y pruebas cross-backend en [`docs/adr/`](docs/adr/).
- [x] Diseñar bounded contexts: identity/access, tenant registry, catalog/pricing, entitlements/quotas, subscription billing, authorization, audit, BFF y realtime. Evidencia: [`docs/phase-0/bounded-contexts.md`](docs/phase-0/bounded-contexts.md).
- [x] Fijar el acuerdo de `gateway_credentials`: plataforma autoriza y reenvía altas/rotaciones por gRPC/mTLS; pasarelas persiste ciphertext en su módulo dueño. Se prohíbe persistencia duplicada/eventos con secretos en plataforma. Evidencia: ADR-0002.
- [x] Versionar el contrato conceptual `payment_method` como unión `card|pse|crypto`, incluyendo settlement cripto decimal string separado del importe comercial entero. Evidencia: ADR-0005.
- [x] Separar el recaudo de Payvia a sus tenants de los pagos de compradores procesados por pasarelas; asignar adapters propios de plataforma: Wompi para recaudo, Factus para facturación electrónica y Resend para correo. Evidencia: ADR-0006.
- [x] Definir threat model cross-backend: identidad/scopes y mTLS con rotación, grants ligados al request, replay, tamaño/rate limits, deadlines, retries y telemetría redactada. Evidencia: [`docs/phase-0/threat-model.md`](docs/phase-0/threat-model.md).
- [x] Auditar la composición real del workspace raíz y confirmar que ambos backends son miembros independientes sin workspaces anidados.
- [x] Definir los bounded contexts, ownership, contratos y fases de creación de crates/apps funcionales sin generar paquetes vacíos.
- [x] Definir las dos apps frontend separadas: `frontends/frontend-pasarela/` para tenant y `frontends/frontend-plataform/` para administración de plataforma. Evidencia: ADR-0001 y auditoría de sus documentos actuales.
- [x] Documentar la matriz de integración y el plan de aceptación por fase en [`docs/phase-0/integration-matrix.md`](docs/phase-0/integration-matrix.md).

**Salida:** auditoría, decisiones, ownership, autorización inter-backend, matrices, contratos conceptuales, amenazas y fases documentados. Las ADRs registran decisiones ya establecidas por las reglas normativas; las decisiones de detalle no definidas por ellas siguen abiertas y no se presentan como aprobadas. El alcance documental de Fase 0 se cerró históricamente conforme a la decisión de alcance del proyecto; la auditoría adicional de trazabilidad y reproducibilidad está registrada en PR #3 y no afirma integración runtime. Las pruebas runtime HTTP→BFF→gRPC→módulo son criterio de Fase 1.

### Evidencia y límites de Fase 0

El informe de auditoría conserva los comandos ejecutados y el inventario real. No hay mutaciones persistidas visibles que originen realtime; Redis y NATS/Outbox son N/A porque Fase 0 no implementa runtime ni dominio. Por decisión de alcance del proyecto, Fase 0 es documental y se cierra al completar auditoría, ADRs, bounded contexts, threat model, matriz y verificación estática del workspace. No requiere adelantar contratos ejecutables: la aceptación HTTP→BFF→gRPC→módulo pertenece a Fase 1 y debe completarse allí antes de cerrar esa fase.

## Fase 1 — Fundaciones y observabilidad

### Aceptación obligatoria de casos de uso y auditoría de pruebas
La subfase 1.1 implementa el primer corte operativo: configuración tipada y validada; BFF Axum con OpenAPI y endpoints operacionales; contrato y servidor gRPC del módulo runtime; propagación W3C/correlation; errores tipados y envelopes seguros; logs JSON, OpenTelemetry/OTLP, métricas y Sentry con redacción; readiness verificada, apagado ordenado y diagnósticos de cada frontera. Su matriz previa, con N/A justificados, está en [`docs/phase-0/integration-matrix.md`](docs/phase-0/integration-matrix.md). El health endpoint es una interfaz operacional; por eso no crea una pantalla de producto ni requiere UI React. La aceptación debe usar cliente HTTP real y cruzar el límite de red gRPC. El Dockerfile y configuración de observabilidad son específicos del backend y se encuentran bajo `bacekend-plataform/deploy/`; su build usa la raíz como contexto Cargo, pero no contiene ni despliega el binario de pasarelas.

Para cada caso de uso de esta fase, documentar y automatizar caminos positivos y negativos, entradas inválidas/tipos/límites, duplicados y concurrencia, permisos/aislamiento, estados, fallos de dependencias, integridad de persistencia/Outbox y redacción, según aplique. Cada caso debe enlazar requisito→escenario→test→ejecución CI. Fase 8 no sustituye estas pruebas.
Criterios de cierre 1.1: pruebas reales HTTP→BFF→gRPC→módulo; traza y correlation ID presentes en el servidor RPC; éxito, desconexión y timeout automatizados; validación/configuración y autorización donde corresponda; evento causal y respuesta HTTP correlacionados con código/categoría y duraciones; redacción verificada; health/readiness distinta y basada en dependencias existentes; apagado ordenado probado; formato, clippy y pruebas ejecutados. El timeout se verifica en el mismo recorrido HTTP con un fallo de transporte TCP controlado, sin respuestas gRPC simuladas. No se exige PostgreSQL, Redis, JetStream, Outbox ni realtime en este corte, según las justificaciones de la matriz. Sentry y OTLP se instrumentan y configuran desde el inicio; su recepción externa solo se reporta como comprobada con envío verificable, evidenciado en el documento de aceptación.

Auditoría de pruebas: [`docs/phase-1/test-audit.md`](docs/phase-1/test-audit.md). PR #4 está fusionada y su CI está aprobada. La matriz exhaustiva de configuración/readiness, DSN Sentry, semántica de APP_VERSION y pruebas repetibles de degradación/shutdown continúan pendientes.


- [x] Configuración tipada y secretos por entorno, fallando de forma segura.
- [x] Tokio/Axum BFF, gRPC interno, OpenAPI y propagación W3C.
- [x] Errores tipados y envelopes seguros.
- [x] Logs JSON estructurados; OpenTelemetry, métricas y Sentry con redacción.
- [x] Eventos diagnósticos en cada frontera: inicio, dependencia, fallo y cierre; duración propia/total y clasificación retryable fundada.
- [x] Health/readiness, apagado ordenado, dashboards/alertas y salud del pipeline de telemetría.
- [x] Artefacto de despliegue exclusivo para `bacekend-plataform`, con configuración operativa bajo `bacekend-plataform/deploy/`; build y ejecución no inician el backend de pasarelas.

**Salida:** integración automatizada prueba contexto HTTP→gRPC→módulo, éxito, desconexión y timeout, eventos correlacionados y ausencia de secretos. El alcance de implementación de Fase 1 se cerró históricamente con evidencia en [`docs/phase-1/acceptance.md`](docs/phase-1/acceptance.md); su auditoría ampliada de casos de uso y límites está en PR #4 y queda sujeta a aprobación.

## Fase 2 — Identidad, tenant registry y aislamiento

### Aceptación obligatoria de casos de uso y auditoría de pruebas
La implementación se ejecuta conforme a la matriz previa de [`docs/phase-2/acceptance.md`](docs/phase-2/acceptance.md). El primer corte incluye identidad/sesiones separadas para tenants y operadores, revisión manual de tenant, ciclo de vida de API Keys y las fronteras PostgreSQL/gRPC/Redis/JetStream/realtime y ambas aplicaciones frontend que aplican a estos cambios visibles.

Para cada caso de uso de esta fase, documentar y automatizar caminos positivos y negativos, entradas inválidas/tipos/límites, duplicados y concurrencia, permisos/aislamiento, estados, fallos de dependencias, integridad de persistencia/Outbox y redacción, según aplique. Cada caso debe enlazar requisito→escenario→test→ejecución CI. Fase 8 no sustituye estas pruebas.

Auditoría de pruebas: [`docs/phase-2/test-audit.md`](docs/phase-2/test-audit.md). PR #5 está fusionada y su CI run 41237702253 aprobó Rust, ambos frontends y Playwright E2E; la matriz de cobertura y sus límites de alcance están en el acta de auditoría.


- [x] Schemas/migraciones propiedad de identity/access, tenant registry y realtime, coordinadas sobre PostgreSQL.
- [x] Registro, verificación, activación, suspensión, reactivación, cierre y auditoría de tenant.
- [x] Usuarios, sesiones, membresías propietarias, roles, permisos y cuentas de operadores independientes.
- [x] API Keys `test`/`live`: entropía, digest, prefijo, scopes, revocación y visualización única.
- [x] Contexto tenant autorizado de extremo a extremo; límites, unicidad y RLS entre tenants.
- [x] Superficie REST/OpenAPI para ambas apps detrás del BFF y contratos internos gRPC.
- [x] Outbox→JetStream, membresía idempotente, WebSocket autenticado, replay por cursor y recuperación REST.
- [x] E2E Playwright de ambas apps y recuperación multi-réplica tras reiniciar Redis/NATS de desarrollo.

Salida: la implementación y la ampliación de pruebas de Fase 2 están integradas. La aceptación HTTP→BFF→gRPC→módulo verifica identidad, permisos, aislamiento, API Keys, auditoría, errores y redacción con PostgreSQL/Redis/NATS reales; Playwright recorre ambas aplicaciones y la recuperación multi-réplica. PR #5 quedó fusionada con CI aprobada (run 41237702253). La cobertura y los límites están en docs/phase-2/test-audit.md.

## Fase 3 — Catálogo de planes, precios, límites y entitlement

**Pruebas obligatorias de esta fase:** cada caso de uso y cada acción debe tener escenarios positivos/negativos, límites y tipos inválidos, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción aplicables. Registrar requisito→test→ejecución CI; no se aplaza a Fase 8.


- [x] Planes y versiones inmutables de precio/moneda/periodo; cambios con vigencias explícitas.
- [x] Capacidades, límites, cuotas, políticas de excedente y restricciones por entorno.
- [x] Evaluación determinista del entitlement por tenant, plan y fecha efectiva.
- [x] Ledger durable de consumo/cuotas y protección contra concurrencia y replay, incluyendo replay estable de la respuesta original después de consumos posteriores y rechazo seguro de filas históricas sin snapshot.
- [x] API administrativa de catálogo y consulta tenant de su plan/consumo sin filtrar datos internos.
- [x] Contrato gRPC versionado y cliente tipado en gateway; mTLS, grants firmados y enforcement runtime pertenecen a Fase 5.
- [x] Outbox de catálogo/entitlements → JetStream → realtime journal, cursor de plataforma/tenant y señal Redis; traceparent W3C propagado.
- [x] Matriz de integración previa, límites N/A y evidencia de aceptación registrados.

Acta: docs/phase-3/acceptance.md. Matriz específica: docs/phase-3/integration-matrix.md. La aceptación usa PostgreSQL, Redis y JetStream reales; prueba inmutabilidad, rollback, replay original, concurrencia, RLS y entrega de eventos de catálogo/cuota al journal realtime. PR #7 se integró tras CI run 41319415556 en success sobre su HEAD exacto. Esta fase corresponde a construcción y no exige despliegue productivo.

**Salida:** pruebas reales cubren límites exactos, excedentes, replay original, concurrencia, cambio versionado de plan, fechas, aislamiento, BFF/gRPC, gateway contract client y entrega realtime con cursor.

## Fase 4 — Suscripciones y facturación de uso Payvia

**Estado:** implementación parcial. El PR #10 está integrado en `main` y entregó el slice inicial de creación/consulta de Payment Intents, Outbox y journal/cursor realtime. El PR #11 continúa abierto en `feat/phase-4-payment-lifecycle`; su base de dominio y contrato neutral del lifecycle está implementada y verificada en el HEAD `c036f0010fa8140b1a8809ffa2d6be3c3e190f71` por [CI #942](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41753921841), con quality, documentación, ambos frontends y E2E en success. Esta evidencia no implementa la persistencia ni el ciclo operativo de intentos/transacciones. El Alcance A (suscripciones y facturación SaaS de Payvia) continúa pendiente. La matriz de integración conserva el estado por frontera y la trazabilidad requisito → test → ejecución CI.

**Registro de decisiones pendientes:** [docs/phase-4/decision-register.md](docs/phase-4/decision-register.md). Separa las decisiones económicas que requieren aprobación de las resoluciones técnicas. No aprueba precios, impuestos, políticas comerciales ni capacidades de proveedores.

**Matriz de integración y evidencia:** [docs/phase-4/integration-matrix.md](docs/phase-4/integration-matrix.md). La matriz conserva los límites de cada prueba y los componentes que aún no cuentan con implementación o evidencia.

**Pruebas obligatorias de esta fase:** cada caso de uso y cada acción debe tener escenarios positivos/negativos, límites y tipos inválidos, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción aplicables. Registrar requisito → test → ejecución CI; no se aplaza a Fase 8.

### Entregables implementados y verificados — Alcance B

Los siguientes checks se limitan al slice inicial de creación/consulta de Payment Intents y a los escenarios Outbox/realtime expresamente probados. No certifican el ciclo completo de transacciones ni el procesamiento de pagos por un proveedor.

- [x] Agregado inicial Payment Intent propiedad de `payments`: UUID, tenant y entorno, importe entero positivo en unidades menores, validación sintáctica de moneda y método, referencia opaca opcional y único estado persistido `created`. La creación/consulta funciona mediante REST → BFF → gRPC → módulo; el estado `created` no representa autorización, captura, éxito ni liquidación.
- [x] Autorización de creación y consulta mediante los scopes existentes `payment_intents.create` y `payment_intents.read`, conservando API Keys bajo `identity_access`. Aceptación HTTP real cubre credencial ausente, clave revocada, scope insuficiente, tenant y entorno.
- [x] Idempotencia de creación por tenant + entorno + operación versionada + clave; huella SHA-256 del payload normalizado, replay del snapshot original y conflicto ante payload diferente. Constraint PostgreSQL y prueba con 12 solicitudes concurrentes verifican una sola creación.
- [x] Aislamiento de lectura y persistencia por tenant/entorno mediante filtros explícitos y RLS forzada; pruebas PostgreSQL reales rechazan accesos cross-tenant y cross-environment.
- [x] Auditoría de creación, replay y lectura en la misma transacción que la operación correspondiente, con actor, recurso, resultado y correlación; las pruebas verifican que el secreto de API Key no se persiste.
- [x] Escritura atómica de Payment Intent, registro idempotente, snapshot, auditoría y evento inicial `payment_intent.created` en Outbox.
- [x] Relay Outbox con claim `FOR UPDATE SKIP LOCKED`, lease y token propietario; los propietarios vencidos/obsoletos no pueden renovar, confirmar, reprogramar ni poner eventos en cuarentena.
- [x] Timeouts independientes de 10 segundos para publicación y ACK; backoff exponencial con jitter acotado; `Nats-Msg-Id` estable y confirmación PostgreSQL solo después del ACK JetStream.
- [x] Cuarentena durable de errores permanentes de contrato, conservando la fila/payload y excluyéndolos de claims automáticos. Están probados con PostgreSQL/JetStream reales los escenarios de lease vencido, recuperación tras rechazo del stream, dos relays concurrentes y cuarentena permanente.
- [x] Integración inicial con journal realtime: persistencia del envelope antes del ACK, señal Redis y recuperación por cursor con aislamiento tenant, según el test de aceptación integrado.

**Evidencia:** [CI #913](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41716834260) verificó los tests reales del slice y resiliencia Outbox sobre el SHA de código `1858766a46a5b92d6b6b75207e728ddaa6933498`. [CI #914](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41717663987) verificó el HEAD documental previo del PR (`e1d2c4281e70d117719398d9bad0730e393f1169`): detección de alcance, quality, documentación y YAML, todos en success. La matriz describe qué escenarios concretos cubre cada ejecución y qué no demuestra.

### Base técnica del ciclo de pagos — avance parcial

El código de esta rama introduce la base de dominio y el contrato neutral documentados en [ADR-0011](docs/adr/ADR-0011-payment-lifecycle-and-normalized-provider-events.md). Estos elementos no equivalen a una integración operativa ni cierran los entregables completos:

- [x] Modelos de dominio `PaymentAttempt` y `PaymentTransaction`; validaciones de identidad, ownership, secuencia, proveedor, referencia e importe; estados canónicos separados; guards forward-only y terminales; transición `Pending → Authorized → Succeeded`; control unitario de timestamps y `outcome_unknown`; contrato Protobuf neutral `NormalizedProviderPaymentEventV1`. **Verificado en el HEAD actual:** [CI #942](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41753921841), SHA `c036f0010fa8140b1a8809ffa2d6be3c3e190f71`. Pasaron límite de 400 líneas, `cargo fmt`, Clippy, `cargo test --locked --workspace`, ambos frontends, E2E tenant/plataforma/multi-réplica y documentación. Este check certifica únicamente los modelos, guards, contrato y pruebas incluidas; no acredita persistencia ni operación de pagos.
- [ ] Persistencia PostgreSQL con RLS/constraints, inbox idempotente, casos de reconciliación, auditoría, Outbox y pruebas reales de aislamiento/concurrencia/rollback.
- [ ] API/contratos de creación de intentos, consulta de intentos/transacciones y actualización del estado del intent, con scopes y aceptación HTTP→BFF→gRPC→módulo.
- [ ] Ingestión de eventos normalizados solo después de verificar el callback en el adapter y autenticar al productor cross-backend. El mensaje Protobuf no autentica por sí mismo y no se añadió un RPC/consumer de ingreso.
- [ ] Consultas reales al proveedor, resolución durable de resultado incierto y conciliación; no repetir un cobro ante timeout sin resultado autoritativo.

### Entregables de Fase 4 aún pendientes o bloqueados

- [ ] Completar el ciclo de negocio de Payment Intents/transacciones de extremo a extremo: persistencia de intentos/transacciones, estados y guards aplicados transaccionalmente, eventos normalizados consumidos desde una frontera autenticada, manejo durable de eventos tardíos, outcome unknown y reconciliación contra el proveedor.
- [ ] Definir e implementar captura, cancelación, expiración y reembolsos totales/parciales únicamente después de aprobar B-EC-07..13 (autorización/captura, vigencia, cancelación, reintentos, cambio de proveedor y política/límites de reembolsos). No se crean comandos, endpoints ni estados para estas operaciones mientras sigan sin aprobación.
- [ ] Cobro a compradores de los tenants mediante el flujo configurado, separado del recaudo SaaS que Payvia realiza a sus tenants. Bloqueado por decisiones funcionales aplicables y por adapters/callbacks reales del backend de pasarelas; el scaffold de gateway no se sustituye con una simulación.
- [ ] Implementar el ciclo de suscripción, periodos, medición facturable, prorrateo si se aprueba, invoices y partidas inmutables.
- [ ] Implementar cálculo de cobros de Payvia independiente de montos y fees de transacciones procesadas para el tenant, con snapshot económico reproducible y ledger financiero.
- [ ] Implementar cobro de invoices de Payvia por `platform_billing_wompi`, con cuenta/credenciales de Payvia, callbacks propios verificados y reconciliación; no atraviesa `bacekend-pasarela` ni comparte credenciales tenant.
- [ ] Implementar `invoice_factus` para emisión/consulta de documentos electrónicos de Payvia, manteniendo `platform_billing` como dueño de invoices, ledger y estados financieros.
- [ ] Implementar `notification_resend` para correo transaccional/operativo, despachado desde Outbox con idempotencia, reintentos clasificados y trazabilidad.
- [ ] Completar estados de pago/reintentos y conciliación de la factura, sin duplicar efectos financieros.
- [ ] Completar eventos Outbox de invoice, pago, mora, suspensión y cambios de suscripción.
- [ ] Completar notificaciones y recuperación realtime para las aplicaciones pertinentes.
- [ ] Completar pruebas de caída/reinicio físico del proceso NATS, reconexión de socket, restart del consumidor, WebSocket autenticado y recuperación de cliente; la recuperación ante stream ausente/restaurado ya probada no demuestra estos escenarios.
- [ ] Completar pruebas de fallo/rollback total de PostgreSQL y los demás escenarios de fallos aún señalados en la matriz.

**Bloqueos y dependencias:** las decisiones económicas A-EC/B-EC que correspondan deben aprobarse antes de implementar cálculos o políticas comerciales; las cuentas, credenciales y sandboxes oficiales de Wompi, Factus y Resend deben validarse antes de certificar sus adapters. mTLS, signed grants y enforcement cross-backend permanecen asignados a Fase 5 y no se implementan ni se sustituyen en esta fase.

**Salida de Fase 4:** solo podrá declararse cerrada cuando ambos alcances tengan decisiones aprobadas, implementación completa, pruebas reales positivas/negativas/límites/concurrencia/permisos/aislamiento/estados/fallos/reintentos/integridad/redacción, integraciones externas verificadas y trazabilidad requisito → test → ejecución CI sobre el SHA final. Los checks completados arriba no eliminan los pendientes de esta sección.

### Extensibilidad de proveedores propios

Los módulos de dominio invocan contratos neutrales al proveedor y el composition root registra adapters por identidad y capacidades. `platform_billing_wompi`, `invoice_factus` y `notification_resend` son adapters iniciales separados. Incorporar o retirar un proveedor modifica su adapter y su registro/configuración, no las reglas generales de billing, facturación electrónica, notificaciones, ni adapters existentes. La baja conserva historial financiero, auditoría y referencias externas; solo bloquea nuevas operaciones según política de transición y reconciliación.

Referencias oficiales para la implementación: [Wompi — ambientes y llaves](https://docs.wompi.co/docs/colombia/ambientes-y-llaves/), [Factus API](https://developers.factus.com.co/), [Resend — envío de correo](https://resend.com/docs/api-reference/emails/send-email). Revalidar contratos y capacidades durante cada adapter; no asumir una forma común de autenticación, firma o reintento.

## Fase 5 — Decisión de autorización y concesiones para el backend de pasarelas

**Pruebas obligatorias de esta fase:** cada caso de uso y cada acción debe tener escenarios positivos/negativos, límites y tipos inválidos, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción aplicables. Registrar requisito→test→ejecución CI; no se aplaza a Fase 8.


- [ ] Contrato gRPC versionado `ExecuteAuthorizedGatewayOperation` y `ReportVerifiedProviderEvent` (o contratos versionados equivalentes).
- [ ] Autenticación mutua TLS/mTLS, identidad de servicio rotatoria y autorización por método/scope.
- [ ] Política por tenant, API Key, entorno, cuenta proveedor, plan, cuota, operación, monto/moneda y estado de cuenta.
- [ ] Concesión firmada de corta duración y uso único/Idempotency-Key, ligada al comando y verificable por pasarelas.
- [ ] Decisiones tipadas: allow/deny y razón estable segura; no exponer reglas internas o datos de otros tenants.
- [ ] Denegación cerrada ante timeout de decisión, revocación, concesión expirada o alterada y versión incompatible.
- [ ] Pruebas de contrato/conformidad compartidas y propagación de `correlation_id`/`trace_id`.
- [ ] Pruebas negativas y resiliencia: certificado inválido/expirado/rotado, caller no autorizado, payload grande/malformado, rate limit, replay, deadline, desconexión y recuperación idempotente.
- [ ] Configuración de credenciales tenant llega por BFF→plataforma→gRPC mTLS→`gateway_credentials`; no se persiste en plataforma ni se publica en NATS.

**Salida:** plataforma solo envía operaciones que ha autorizado y pasarelas no puede ejecutarlas sin concesión vigente válida; matriz de errores y recuperación aprobada.

## Fase 6 — Auditoría, eventos y realtime

**Pruebas obligatorias de esta fase:** cada caso de uso y cada acción debe tener escenarios positivos/negativos, límites y tipos inválidos, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción aplicables. Registrar requisito→test→ejecución CI; no se aplaza a Fase 8.


- [ ] Auditoría inmutable de administración, authz, tenant, plan, precio, límite y facturación.
- [ ] Outbox en la transacción de negocio y relay durable JetStream.
- [ ] Consumers durable idempotentes y DLQ/replay con permisos operativos.
- [ ] Realtime multi-replica con `event_id`, cursor, aggregate version, scope, deduplicación, orden y huecos.
- [ ] WebSocket por BFF autenticado; TanStack Query central en tenant/admin y recuperación REST por cursor/snapshot.
- [ ] Redis Pub/Sub solo fan-out efímero, reconexión acotada y resuscripción.

**Salida:** restart/reconnect tests sobre PostgreSQL/Redis/NATS reales de desarrollo prueban replay, reparación REST, permisos, orden y no pérdida confirmada.

## Fase 7 — Frontends independientes

**Pruebas obligatorias de esta fase:** cada caso de uso y cada acción debe tener escenarios positivos/negativos, límites y tipos inválidos, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción aplicables. Registrar requisito→test→ejecución CI; no se aplaza a Fase 8.
- [ ] Aplicación tenant: registro/invitación, plan/uso, suscripción/facturas, API Keys y estado de cuenta.
- [ ] Aplicación administración: tenants, catálogo/versiones, precios, límites, consumo, facturación, autorización y auditoría.
- [ ] Sesiones, permisos, navegación, cliente API y caché separados; ambas apps consumen exclusivamente BFF REST/OpenAPI/WebSocket autenticado.
- [ ] UX de denegación, cuota agotada, error recuperable, conexión perdida y datos actualizándose.
- [ ] Pruebas UI propias sin usarlas como sustituto de aceptación HTTP→BFF→gRPC real.

**Salida:** pruebas de permisos y contratos para ambas aplicaciones; frontend no accede a PostgreSQL, Redis, NATS, gRPC interno o servicio pasarela directamente.

## Fase 8 — Calidad, seguridad, resiliencia y rendimiento

**Pruebas obligatorias de esta fase:** cada caso de uso y cada acción debe tener escenarios positivos/negativos, límites y tipos inválidos, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción aplicables. Registrar requisito→test→ejecución CI; no se aplaza a Fase 8.


- [ ] Unitarias, properties, integración, E2E de frontera, contratos, aislamiento tenant y autorización.
- [ ] Pruebas reales de caídas/reinicios PostgreSQL/Redis/JetStream en servicios de desarrollo documentados; nunca producción.
- [ ] Seguridad de sesiones, API Keys, SSRF, rate limit, replay, auditoría, secretos y separación de roles.
- [ ] Carga/concurrencia k6, benchmark de decisiones authz y pruebas de saturación/recuperación.
- [ ] Fuzzing de parsers expuestos y revisión de dependencias, SAST y configuración.
- [ ] SLO, dashboards, alertas, runbooks y reporte de cobertura de flujos/errores críticos.

**Salida:** no hay defectos críticos/altos abiertos ni criterios bloqueados presentados como pasados; evidencia reproducible y limitaciones registradas.

## Fase 9 — Operación y piloto

**Pruebas obligatorias de esta fase:** cada caso de uso y cada acción debe tener escenarios positivos/negativos, límites y tipos inválidos, duplicados/concurrencia, permisos/aislamiento, estados, fallos/reintentos, integridad y redacción aplicables. Registrar requisito→test→ejecución CI; no se aplaza a Fase 8.
- [ ] Despliegue seguro, entornos separados, backups/restauración, migraciones coordinadas y rotación de claves.
- [ ] Runbooks de tenant suspendido, deuda, reintento/reconciliación, authz caída, replay y recuperación realtime.
- [ ] Piloto, soporte, términos/privacidad y cumplimiento aplicable.
- [ ] Lanzamiento solo tras demostrar aislamiento, billing, autorización, trazabilidad, recuperación y conciliación.

## Registro de integración obligatorio por subfase

| Frontera | Decisión/diseño previo | Evidencia de aceptación |
|---|---|---|
| PostgreSQL y migraciones | schema dueño, transacción, rollback/compatibilidad | migración aplicada y pruebas reales de aislamiento/consistencia |
| Frontend → HTTP/OpenAPI → BFF | apps y usuario/permiso | aceptación automatizada del endpoint y envelope |
| BFF → módulo plataforma (gRPC) | método, contrato y contexto | propagación de identidad, error y traza |
| Plataforma → backend pasarela | decisión, concesión ligada al comando, mTLS y versión | grant válido/alterado/expirado, denegación previa y contrato real |
| Redis | uso TTL o justificación N/A | degradación/reconexión/re-suscripción |
| NATS/Outbox | evento, dueño, consumer, replay o justificación N/A | redelivery/idempotencia y correlación |
| Realtime/recuperación | audiencia, cursor, WebSocket, REST snapshot | orden, huecos, reconexión y autorización |
| Errores/telemetría | clase, código, redacción, duración | fallo diagnóstico + cierre correlacionados |

## Estado verificado — 2026-11-03

- Fase 0 — Alcance documental integrado; PR #3 y CI 41579115662 exitosos. Siguen abiertos trazabilidad automatizada amenaza→control→prueba, consistencia semántica y revalidación reproducible, según docs/phase-0/test-audit.md.
- Fase 1 — Runtime y pruebas ampliadas integrados; PR #4 y CI 41578015530 exitosos. Siguen abiertos matriz de configuración/readiness, DSN Sentry, APP_VERSION y pruebas repetibles de degradación/shutdown, según docs/phase-1/test-audit.md.
- Fase 2 — Identidad, tenants, aislamiento y realtime integrados mediante PR #2 y PR #5. CI 41237702253 exitoso con Rust, frontends y Playwright E2E.
- Fase 3 — Catálogo, precios, límites y entitlements integrados mediante PR #7; CI 41319415556 exitoso sobre el HEAD de PR. La aceptación formal permanece abierta hasta reconciliar los criterios 1–7 en un único HEAD final, como exige docs/phase-3/acceptance.md.
- Fases 4–9 — Pendientes. No comenzar una fase siguiente mientras queden criterios aplicables pendientes en las anteriores.
- PR #6 establece matrices obligatorias de pruebas por caso de uso. PR #8 fija máximo absoluto 400 líneas físicas por archivo Rust; 200 es preferido cuando la cohesión lo permita.

Consulta PROJECT_STATUS.md para SHAs, merge commits, runs y estado de los demás componentes. La integración de Fases 0–3 no implica que pagos, proveedores, billing ni el backend de pasarelas estén terminados.
