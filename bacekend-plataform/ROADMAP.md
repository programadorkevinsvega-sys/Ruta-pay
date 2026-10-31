# Roadmap del Backend de Plataforma Payvia

## Flujo de ramas y revisión

`main` debe mantenerse funcional y verificable. Desarrolla funcionalidades, cambios arquitectónicos, correcciones complejas y modificaciones de seguridad en ramas descriptivas; presenta los cambios importantes en Pull Request con resumen, decisiones, verificaciones ejecutadas y riesgos conocidos. Los cambios triviales de documentación pueden hacerse en `main` cuando las protecciones lo permitan. No se permite force push ni reescribir historial compartido. No integres si fallan verificaciones obligatorias; los cambios sensibles requieren revisión manual.

> Este roadmap cubre únicamente el backend que gobierna tenants, planes, precios, límites, cuotas, suscripciones, facturación de Payvia, identidad, autorización y sus adapters propios de Wompi, Factus y Resend. El backend de pasarelas tiene su propio [README](../bacekend-pasarela/README.md) y [roadmap](../bacekend-pasarela/ROADMAP.md) para los pagos que los comercios procesan a sus compradores.
>
> Los backends comparten el workspace Cargo raíz solo para resolución/build. Cada uno tiene binario, imagen, configuración, secretos, red, artefactos operativos y despliegue independiente en su propia VM/VPS; los dos frontends también tienen despliegues separados. Los artefactos de esta aplicación pertenecen a `bacekend-plataform/deploy/`.
>
> Estado: las implementaciones de Fases 0, 1 y 2 están integradas en `main`; la Fase 2 se integró mediante PR #2 tras pasar Format, Clippy y Tests en CI. Ese cierre histórico acredita el alcance implementado y aceptado entonces, pero no certifica la cobertura exhaustiva exigida por la nueva política de pruebas. La auditoría de calidad por casos de uso está abierta mediante PRs #3, #4 y #5; hasta que sus pruebas y CI estén aprobadas, no se declara conformidad completa con la nueva Definition of Done. Las fases posteriores siguen pendientes.

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

Auditoría de pruebas: [`docs/phase-0/test-audit.md`](docs/phase-0/test-audit.md). Estado: revisión documental; trazabilidad automatizada amenaza→control→prueba pendiente del PR #3.


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

Para cada caso de uso de esta fase, documentar y automatizar caminos positivos y negativos, entradas inválidas/tipos/límites, duplicados y concurrencia, permisos/aislamiento, estados, fallos de dependencias, integridad de persistencia/Outbox y redacción, según aplique. Cada caso debe enlazar requisito→escenario→test→ejecución CI. Fase 8 no sustituye estas pruebas.

Auditoría de pruebas: [`docs/phase-1/test-audit.md`](docs/phase-1/test-audit.md). Estado: pruebas de runtime existentes y ampliación de límites en PR #4; matriz exhaustiva de configuración/readiness aún pendiente.


La subfase 1.1 implementa el primer corte operativo: configuración tipada y validada; BFF Axum con OpenAPI y endpoints operacionales; contrato y servidor gRPC del módulo runtime; propagación W3C/correlation; errores tipados y envelopes seguros; logs JSON, OpenTelemetry/OTLP, métricas y Sentry con redacción; readiness verificada, apagado ordenado y diagnósticos de cada frontera. Su matriz previa, con N/A justificados, está en [`docs/phase-0/integration-matrix.md`](docs/phase-0/integration-matrix.md). El health endpoint es una interfaz operacional; por eso no crea una pantalla de producto ni requiere UI React. La aceptación debe usar cliente HTTP real y cruzar el límite de red gRPC. El Dockerfile y configuración de observabilidad son específicos del backend y se encuentran bajo `bacekend-plataform/deploy/`; su build usa la raíz como contexto Cargo, pero no contiene ni despliega el binario de pasarelas.

Criterios de cierre 1.1: pruebas reales HTTP→BFF→gRPC→módulo; traza y correlation ID presentes en el servidor RPC; éxito, desconexión y timeout automatizados; validación/configuración y autorización donde corresponda; evento causal y respuesta HTTP correlacionados con código/categoría y duraciones; redacción verificada; health/readiness distinta y basada en dependencias existentes; apagado ordenado probado; formato, clippy y pruebas ejecutados. El timeout se verifica en el mismo recorrido HTTP con un fallo de transporte TCP controlado, sin respuestas gRPC simuladas. No se exige PostgreSQL, Redis, JetStream, Outbox ni realtime en este corte, según las justificaciones de la matriz. Sentry y OTLP se instrumentan y configuran desde el inicio; su recepción externa solo se reporta como comprobada con envío verificable, evidenciado en el documento de aceptación.

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

Para cada caso de uso de esta fase, documentar y automatizar caminos positivos y negativos, entradas inválidas/tipos/límites, duplicados y concurrencia, permisos/aislamiento, estados, fallos de dependencias, integridad de persistencia/Outbox y redacción, según aplique. Cada caso debe enlazar requisito→escenario→test→ejecución CI. Fase 8 no sustituye estas pruebas.

Auditoría de pruebas: [`docs/phase-2/test-audit.md`](docs/phase-2/test-audit.md). Estado: pruebas Rust y frontend existentes; duplicidad/concurrencia de email y ejecución de suites frontend/Playwright se incorporan en PR #5, pendiente de CI.


La implementación se ejecuta conforme a la matriz previa de [`docs/phase-2/acceptance.md`](docs/phase-2/acceptance.md). El primer corte incluye identidad/sesiones separadas para tenants y operadores, revisión manual de tenant, ciclo de vida de API Keys y las fronteras PostgreSQL/gRPC/Redis/JetStream/realtime y ambas aplicaciones frontend que aplican a estos cambios visibles.

- [x] Schemas/migraciones propiedad de identity/access, tenant registry y realtime, coordinadas sobre PostgreSQL.
- [x] Registro, verificación, activación, suspensión, reactivación, cierre y auditoría de tenant.
- [x] Usuarios, sesiones, membresías propietarias, roles, permisos y cuentas de operadores independientes.
- [x] API Keys `test`/`live`: entropía, digest, prefijo, scopes, revocación y visualización única.
- [x] Contexto tenant autorizado de extremo a extremo; límites, unicidad y RLS entre tenants.
- [x] Superficie REST/OpenAPI para ambas apps detrás del BFF y contratos internos gRPC.
- [x] Outbox→JetStream, membresía idempotente, WebSocket autenticado, replay por cursor y recuperación REST.
- [x] E2E Playwright de ambas apps y recuperación multi-réplica tras reiniciar Redis/NATS de desarrollo.

**Salida funcional histórica:** cumplida dentro de los criterios implementados entonces. Aceptación HTTP→BFF→gRPC→módulo verifica identidad, permisos, tenant cruzado, API Keys, auditoría, errores y redacción con PostgreSQL/Redis/NATS reales de desarrollo; Playwright recorre ambas aplicaciones. Evidencia en [`docs/phase-2/acceptance.md`](docs/phase-2/acceptance.md). La conformidad con la nueva matriz obligatoria de casos positivos/negativos, concurrencia y CI frontend/E2E queda pendiente del PR #5 y su ejecución aprobada.

## Fase 3 — Catálogo de planes, precios, límites y entitlement

- [ ] Planes y versiones inmutables de precio/moneda/periodo; cambios con vigencias explícitas.
- [ ] Capacidades, límites, cuotas, políticas de excedente y restricciones por entorno.
- [ ] Evaluación determinista del entitlement por tenant, plan y fecha efectiva.
- [ ] Ledger durable de consumo/cuotas y protección contra concurrencia y replay.
- [ ] API administrativa de catálogo y consulta tenant de su plan/consumo sin filtrar datos internos.
- [ ] Contrato de entitlement/autorización consumible por gateway mediante gRPC.

**Salida:** pruebas reales cubren límites exactos, excedentes, concurrencia, cambio versionado de plan, fechas y aislamiento.

## Fase 4 — Suscripciones y facturación de uso Payvia

- [ ] Ownership del ciclo de negocio de payment intents/transacciones: API Keys, idempotency scope, transiciones válidas y eventos normalizados; el backend de pasarelas ejecuta solamente llamadas de protocolo autorizadas.
- [ ] Cobro a compradores de los tenants según el flujo configurado, separado del cobro que Payvia realiza a los tenants por usar la plataforma.
- [ ] Ciclo de suscripción, periodos, prorrateo si se aprueba, facturas y partidas inmutables.
- [ ] Cálculo de cobros de Payvia independiente de montos y fees de una transacción procesada para el tenant.
- [ ] Cobro de invoices de Payvia por `platform_billing_wompi`, con cuenta/credenciales de Payvia y callbacks propios verificados; no atraviesa `bacekend-pasarela` ni comparte credenciales tenant.
- [ ] Integración `invoice_factus` para emisión/consulta de documentos electrónicos de Payvia, manteniendo `platform_billing` como dueño de invoices, ledger y estados financieros.
- [ ] Integración `notification_resend` para correo transaccional/operativo, despachada desde Outbox con idempotencia, reintentos clasificados y trazabilidad.
- [ ] Estados de pago/reintentos y conciliación de la factura, sin duplicar efectos financieros.
- [ ] Eventos Outbox de invoice, pago, mora, suspensión y cambios de suscripción.
- [ ] Reglas de notificación y recuperación realtime a las apps pertinentes.

**Salida:** aceptación integrada comprueba invoice persistida → adapter Wompi de plataforma → callback oficial verificado → ledger/Outbox, incluyendo timeout, redelivery y reconciliación. También verifica emisión Factus y entrega Resend con claves de idempotencia, errores clasificados y estados recuperables. Los entornos sandbox/cuentas oficiales y credenciales reales apropiadas son requisito antes de declarar cada adapter validado. La capacidad de cobro recurrente automático se verifica antes de diseñar ese comportamiento.

### Extensibilidad de proveedores propios

Los módulos de dominio invocan contratos neutrales al proveedor y el composition root registra adapters por identidad y capacidades. `platform_billing_wompi`, `invoice_factus` y `notification_resend` son adapters iniciales separados. Incorporar o retirar un proveedor modifica su adapter y su registro/configuración, no las reglas generales de billing, facturación electrónica, notificaciones, ni adapters existentes. La baja conserva historial financiero, auditoría y referencias externas; solo bloquea nuevas operaciones según política de transición y reconciliación.

Referencias oficiales para la implementación: [Wompi — ambientes y llaves](https://docs.wompi.co/docs/colombia/ambientes-y-llaves/), [Factus API](https://developers.factus.com.co/), [Resend — envío de correo](https://resend.com/docs/api-reference/emails/send-email). Revalidar contratos y capacidades durante cada adapter; no asumir una forma común de autenticación, firma o reintento.

## Fase 5 — Decisión de autorización y concesiones para el backend de pasarelas

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

- [ ] Auditoría inmutable de administración, authz, tenant, plan, precio, límite y facturación.
- [ ] Outbox en la transacción de negocio y relay durable JetStream.
- [ ] Consumers durable idempotentes y DLQ/replay con permisos operativos.
- [ ] Realtime multi-replica con `event_id`, cursor, aggregate version, scope, deduplicación, orden y huecos.
- [ ] WebSocket por BFF autenticado; TanStack Query central en tenant/admin y recuperación REST por cursor/snapshot.
- [ ] Redis Pub/Sub solo fan-out efímero, reconexión acotada y resuscripción.

**Salida:** restart/reconnect tests sobre PostgreSQL/Redis/NATS reales de desarrollo prueban replay, reparación REST, permisos, orden y no pérdida confirmada.

## Fase 7 — Frontends independientes

- [ ] Aplicación tenant: registro/invitación, plan/uso, suscripción/facturas, API Keys y estado de cuenta.
- [ ] Aplicación administración: tenants, catálogo/versiones, precios, límites, consumo, facturación, autorización y auditoría.
- [ ] Sesiones, permisos, navegación, cliente API y caché separados; ambas apps consumen exclusivamente BFF REST/OpenAPI/WebSocket autenticado.
- [ ] UX de denegación, cuota agotada, error recuperable, conexión perdida y datos actualizándose.
- [ ] Pruebas UI propias sin usarlas como sustituto de aceptación HTTP→BFF→gRPC real.

**Salida:** pruebas de permisos y contratos para ambas aplicaciones; frontend no accede a PostgreSQL, Redis, NATS, gRPC interno o servicio pasarela directamente.

## Fase 8 — Calidad, seguridad, resiliencia y rendimiento

- [ ] Unitarias, properties, integración, E2E de frontera, contratos, aislamiento tenant y autorización.
- [ ] Pruebas reales de caídas/reinicios PostgreSQL/Redis/JetStream en servicios de desarrollo documentados; nunca producción.
- [ ] Seguridad de sesiones, API Keys, SSRF, rate limit, replay, auditoría, secretos y separación de roles.
- [ ] Carga/concurrencia k6, benchmark de decisiones authz y pruebas de saturación/recuperación.
- [ ] Fuzzing de parsers expuestos y revisión de dependencias, SAST y configuración.
- [ ] SLO, dashboards, alertas, runbooks y reporte de cobertura de flujos/errores críticos.

**Salida:** no hay defectos críticos/altos abiertos ni criterios bloqueados presentados como pasados; evidencia reproducible y limitaciones registradas.

## Fase 9 — Operación y piloto

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

## Estado

Fases 0, 1 y 2 están cerradas dentro de sus alcances documentados. Las evidencias están en [`docs/phase-0/`](docs/phase-0/), [`docs/phase-1/acceptance.md`](docs/phase-1/acceptance.md) y [`docs/phase-2/acceptance.md`](docs/phase-2/acceptance.md). Fase 2 está integrada en `main` mediante PR #2 (squash merge). Las fases posteriores siguen pendientes; cuentas sandbox, credenciales o decisiones legales faltantes se documentan como bloqueos concretos, sin simulaciones.
