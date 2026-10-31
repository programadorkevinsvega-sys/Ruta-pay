# Payvia: Roadmap

## Flujo de ramas y revisión

`main` debe mantenerse funcional y verificable. Los cambios importantes de código, arquitectura o seguridad se desarrollan en ramas descriptivas y se presentan mediante Pull Request con resumen, decisiones técnicas, verificaciones ejecutadas y riesgos conocidos. Los cambios triviales de documentación pueden hacerse en `main` cuando las protecciones lo permitan. No se permite force push ni reescribir historial compartido. No se integra si las verificaciones obligatorias fallan; los cambios sensibles requieren revisión manual. Consulta las reglas de agentes y proyecto para el procedimiento completo.

> Orquestador de pagos multi-pasarela en Rust.
> Última actualización: 2026-10-21 · Estado general: **planificación**

Payvia se compone de dos backends desplegables, cada uno un monolito modular Rust: [backend de plataforma](bacekend-plataform/README.md) y [backend de pasarelas](bacekend-pasarela/README.md). Sus roadmaps detallados son [plataforma](bacekend-plataform/ROADMAP.md) y [pasarelas](bacekend-pasarela/ROADMAP.md); este documento coordina la entrega integral y sus fronteras.

Plataforma también integra, en módulos propios, Wompi para cobrar a los tenants de Payvia, Factus para facturación electrónica de Payvia y Resend para correo. Estas integraciones no pertenecen a `bacekend-pasarela`, que procesa exclusivamente pagos de compradores de los comercios.

La topología de dos backends ya está fijada en `ARCHITECTURE_RULES.md`. La Fase 0 formaliza mediante ADR los contratos de servicio, concesiones de autorización, migraciones coordinadas y operación; no posterga ni reabre la decisión de tener dos backends.

Este documento describe el plan de construcción de Payvia desde el primer commit hasta la versión 1.0 y más allá. Las duraciones son **estimaciones de referencia** para un desarrollador con apoyo de IA a tiempo completo; ajústalas según tu disponibilidad real.

---

## 1. Visión y alcance

**Visión:** que un comercio integre una sola API y pueda cobrar con cualquier pasarela, con garantías de seguridad, idempotencia y tiempo real, sin reescribir su integración cada vez que cambia de proveedor.

**Dentro del alcance de la v1.0**
- Pasarelas: Wompi y Bold completas; Mercado Pago, Stripe y PayPal en nivel beta.
- API pública `/v1/payment_intents` con API Keys `live_` / `test_`.
- Webhooks entrantes autenticados con el protocolo criptográfico oficial de cada pasarela (HMAC-SHA256 cuando aplique) y salientes firmados con reintentos.
- Notificaciones en tiempo real por WebSocket.
- Dashboard con módulos No-Code, Developer y Configuración de pasarelas.
- Facturación interna básica (fees por transacción).

**Fuera del alcance de la v1.0** (ver sección 8)
- Procesamiento directo de datos de tarjeta (se delega a las pasarelas para mantener el alcance PCI mínimo).
- Suscripciones recurrentes, marketplace con splits, multi-moneda con conversión.

---

## 2. Principios de ingeniería

1. **Seguridad por defecto:** autenticar y autorizar antes de operar, verificar firmas antes de confiar en webhooks, cifrar secretos y redactarlos de logs.
2. **Idempotencia durable:** cada efecto financiero se protege con identidad idempotente y restricciones persistentes; Redis puede acelerar, pero no es autoridad.
3. **Dinero en enteros:** centavos como `i64`, nunca flotantes.
4. **Consistencia antes que velocidad:** cambio de estado y outbox en la misma transacción SQL.
5. **Observabilidad desde el día uno:** logs estructurados, métricas y trazas.
6. **Entrega incremental:** cada fase termina con algo ejecutable y probado.

---

## 3. Vista general por hitos

| Hito | Versión | Fases | Duración est. | Resultado |
|---|---|---|---|---|
| **M1: Cimientos** | `v0.1.0` | 0, 1, 2 | 1 a 2 semanas | Workspace, base de datos y contrato Protobuf |
| **M2: Núcleo de pagos** | `v0.2.0` | 3, 4 | 2 a 3 semanas | Pasarelas, idempotencia, webhooks entrantes y WebSocket |
| **M3: API y entrega** | `v0.3.0` | 5, 6 | 2 a 3 semanas | `/v1/payment_intents` y worker de webhooks salientes |
| **M4: Producto** | `v0.4.0` | 7 | 2 a 3 semanas | Dashboard completo |
| **M5: Endurecimiento** | `v0.9.0` (RC) | 8, 9 | 2 a 3 semanas | Pruebas, seguridad, CI/CD y despliegue |
| **M6: Lanzamiento** | `v1.0.0` | 10 | 1 a 2 semanas | Piloto con comercios reales y GA |
| **M7: Expansión** | `v1.x` | 11+ | continuo | Nuevas pasarelas, SDKs, reconciliación |

**Total estimado hasta v1.0:** 10 a 16 semanas.

Las duraciones son orientativas y no sustituyen los criterios de aceptación. El alcance de cada pasarela se confirma contra su documentación oficial, cuenta elegible y sandbox antes de declararla disponible.

---

## 4. Definition of Done (aplica a cada fase)

Una fase se considera terminada solo cuando cumple **todo** lo siguiente:

- [ ] Compila sin warnings (`cargo clippy -- -D warnings`) y está formateada (`cargo fmt`).
- [ ] Tiene tests unitarios y de integración para la lógica crítica.
- [ ] No contiene `TODO`, `unimplemented!()` ni `unwrap()` fuera de tests.
- [ ] Las migraciones aplican y revierten sin errores.
- [ ] La API pública está documentada (`///` y OpenAPI cuando aplique).
- [ ] No hay secretos en logs ni en el repositorio.
- [ ] El CHANGELOG está actualizado y la fase está etiquetada en Git.
- [ ] El checklist de aceptación de la fase está completo.
- [ ] El cambio fue comprobado en las fronteras reales afectadas y sus eventos operativos/cierre HTTP conservan `correlation_id`, `trace_id` y el código de error.
- [ ] Se documentaron persistencia/migraciones, REST/OpenAPI, gRPC, Redis, NATS/Outbox y WebSocket/recuperación; cada componente no aplicable tiene justificación concreta.
- [ ] La fase contiene una matriz de integración por cada fase/subfase antes de implementar: PostgreSQL y migraciones; frontend→HTTP/REST/OpenAPI→BFF; BFF→módulo dueño por gRPC; decisión concreta sobre Redis; productores/consumidores NATS JetStream y Outbox; WebSocket/realtime y recuperación. Lo no aplicable queda justificado antes de codificar y se verifica como parte de la misma subfase.
- [ ] Se evalúan por separado los frontends de comercio y administración de plataforma. Si la subfase cruza una interfaz, se completa el recorrido por el BFF y las aplicaciones independientes; una no sustituye a la otra. Cambios persistidos visibles tienen Outbox→JetStream→realtime→WebSocket→TanStack Query y recuperación REST autorizada, o justificación concreta, verificable y previa de por qué no deben emitir/consumir realtime.
- [ ] Se prueban errores de validación, autorización, conflicto, timeout/conectividad, persistencia y proveedor aplicable; los no aplicables se justifican. Logs internos conservan diagnóstico seguro (SQLSTATE/constraint/tabla cuando exista), los errores REST son tipados y accionables, y la redacción se verifica automáticamente.
- [ ] La aceptación comprueba eventos de inicio, llamada a dependencia, fallo clasificado y respuesta final con contexto correlacionado, duración propia de dependencia y total, resultado/código y `retryable` solo si la política lo confirma. Evento de fallo y cierre HTTP comparten `correlation_id`/`trace_id` y código/categoría.
- [ ] Redis y JetStream se prueban con servicios reales aislados cuando se usan: reconexión/re-suscripción y recuperación idempotente de mensajes pendientes. Redis nunca es autoridad; PostgreSQL y REST por cursor reparan señales efímeras.
- [ ] No se usan mocks, fixtures, seeds o respuestas simuladas para declarar integración/aceptación. Los tests de integración crean y limpian registros válidos con la aplicación; proveedores externos se validan con sandbox oficial y credenciales reales apropiadas.
- [ ] Calidad y operación se evidencian con unitarias, propiedades cuando aporten valor, integración/contrato/E2E, seguridad, resiliencia, carga/benchmark/fuzzing según aplicabilidad, `cargo fmt`, Clippy, dependencias y CI. Se registran comandos y resultados reales; no se cierra fase con verificaciones bloqueadas sin explicar el bloqueo.

---

### Matriz obligatoria de pruebas de casos de uso

Además de los criterios anteriores, cada fase/subfase debe entregar una matriz trazable para cada caso de uso y cada acción implementada. Debe cubrir caminos positivos y negativos, entradas ausentes/nulas/malformadas, tipos incorrectos, valores cero/negativos y límites, duplicados secuenciales y concurrentes, idempotencia, autenticación/autorización, permisos/aislamiento, estados/transiciones, fallos/timeout/reintentos, integridad transaccional, efectos secundarios, auditoría y redacción, según aplique.

La prueba se ejecuta en el nivel que demuestra la garantía: unitario/propiedad, integración con dependencias reales aisladas, contrato o E2E de navegador. Cada escenario enlaza requisito, test y ejecución CI del commit. Los N/A se justifican antes de implementar. No se cierra una fase con escenarios obligatorios ausentes, sin ejecutar o fallidos. Fase 8 añade fuzzing, seguridad transversal, carga y resiliencia; no sustituye las pruebas funcionales y negativas de cada fase. El porcentaje de cobertura de líneas no demuestra por sí solo cobertura de casos de uso.

## 5. Fases detalladas

### Fase 0: Arquitectura y estructura del proyecto
**Objetivo:** base técnica reproducible. **Duración:** 2 a 3 días.

- [ ] **0.1 Workspace Cargo único:** verificar los paquetes de backend ya registrados en el workspace Cargo de la raíz, completar la estructura de sus crates/binarios y mantener una sola resolución/`Cargo.lock`. Mantener aplicaciones y despliegues independientes, ownership modular propio y sin dependencias de implementación cruzadas. Plataforma contiene tenants, identidad, planes, precios, límites, suscripciones, facturación y dominio de pagos; pasarelas contiene `gateway_wompi`, `gateway_bold` y futuros `gateway_<provider>`. Evitar crates genéricos `core`/`db` como dueños de dominio.
- [ ] **0.2 Infraestructura compartida local:** `compose.yaml` raíz con PostgreSQL, Redis, NATS JetStream, Prometheus, OpenTelemetry Collector, Loki, Tempo y Grafana; Sentry se provisiona con el instalador de su release oficial fijada porque su distribución autohospedada mantiene un Compose con sus propias dependencias. Incluye `crates/observability`, alertas, paneles y pruebas de correlación/redacción/degradación. Evidencia y comandos: [`infra/README.md`](infra/README.md). La configuración no unifica los despliegues de los backends ni frontends.
- [ ] **0.3 Configuración tipada:** carga de `.env`, falla rápido si falta una variable.
- [ ] **0.4 Convenciones:** errores tipados (`thiserror`), logging estructurado con `tracing`, `correlation_id` y `trace_id` por petición.
- [ ] **0.5 Tooling:** `justfile` con `migrate`, `test`, `lint`, `fmt`; hooks pre-commit.
- [ ] **0.6 Límites y amenazas:** diagrama de contexto, flujos de pago/webhook, trust boundaries, manejo de secretos y responsabilidades por módulo.
- [ ] **0.7 Repositorio:** README, LICENSE, SECURITY.md, CONTRIBUTING.md, plantillas de issues y PR.
- [ ] **0.8 Separación de backends:** ADR de topología, contratos gRPC plataforma→pasarelas, concesión de autorización y publicación de eventos pasarelas→plataforma, límites de datos, identidad de servicio, eventos y procedimiento de migración/despliegue.
- [ ] **0.9 Aplicaciones frontend:** definir tenant y administración de plataforma como aplicaciones React + TypeScript independientes en `frontends/frontend-pasarela/` y `frontends/frontend-plataform/`, ambas detrás del BFF REST/OpenAPI; establecer sesión, permisos, cliente API y coordinación TanStack Query.
- [ ] **0.10 Telemetría base:** fijar esquema de logs/eventos, propagación W3C, métricas para p50/p95/p99, RPS, errores, saturación y dependencias, exportación OpenTelemetry, captura Sentry best-effort, redacción, alertas, paneles y salud/degradación del propio pipeline mediante `crates/observability` compartido.

**Criterio de salida:** ambos backends compilan desde el workspace Cargo único de la raíz como binarios/aplicaciones monolíticas modulares y despliegues independientes; servicios locales requeridos tienen configuración documentada; ownership, contratos/autorización entre backends, ambas aplicaciones frontend, flujo frontend→BFF→gRPC, telemetría y límites de confianza quedan definidos. Las ADRs de contratos, ownership y operación están aprobadas. No se considera producto ejecutable por compilar un workspace vacío.

---

### Fase 1: Base de datos
**Objetivo:** esquema multi-tenant sólido. **Duración:** 4 a 6 días.

- [ ] **1.1 Tenant y acceso:** comercios, usuarios/membresías/roles necesarios y aislamiento por `merchant_id`; la autorización se comprueba en cada caso de uso.
- [ ] **1.2 API Keys:** hash, prefijo, entorno (`test`/`live`), permisos, estado, creación/revocación y `last_used_at`; secretos visibles una sola vez.
- [ ] **1.3 Pasarelas:** cuentas por comercio, credenciales y secretos cifrados/versionados, entorno y estado de conexión.
- [ ] **1.4 Pagos:** `payment_intents`, `transactions`, `transaction_events` append-only e idempotencia durable con unicidad por comercio/entorno/clave.
- [ ] **1.5 Webhooks:** registro/deduplicación de eventos entrantes, endpoints de comercio y `outgoing_webhook_deliveries` con intentos y próximo intento.
- [ ] **1.6 Facturación:** reglas/fees aplicados, facturas y partidas; separar monto de pago, fee de Payvia y costos de pasarela.
- [ ] **1.7 Outbox:** Outbox propiedad del módulo productor para cambios que deban publicarse de forma durable.
- [ ] **1.8 Calidad:** migraciones versionadas por módulo, checks de montos/estados, índices justificados, diagrama ER y pruebas de aislamiento tenant/entorno. No añadir datasets ficticios.

**Criterio de salida:** migraciones aplican sobre PostgreSQL aislado y su estrategia de reversión está documentada; restricciones de unicidad e aislamiento se prueban directamente. Los dos backends usan una única DB física compartida; cada módulo posee exclusivamente su schema y migraciones, sin acceso cruzado.

---

### Fase 2: Contrato Protobuf
**Objetivo:** contrato estable entre servicios. **Duración:** 2 a 3 días.

- [ ] **2.1** `payment.proto` con `PaymentIntent`, `Transaction`, `PaymentStatus`, `GatewayEvent`, `WebhookEvent`, `Money`.
- [ ] **2.2** Versionado `v1`, campos reservados y reglas de compatibilidad.
- [ ] **2.3** Generación con `prost-build` y conversiones `From/TryFrom` al dominio.
- [ ] **2.4** Mapeo Protobuf ↔ payload JSON público unificado.

**Criterio de salida:** tests de roundtrip y `buf lint` (o equivalente) sin errores.

---

### Fase 3: Dominio de pagos en plataforma e integración técnica de pasarelas
**Objetivo:** reglas de negocio de pago en plataforma y adapters aislados en el backend pasarelas. **Duración:** 8 a 10 días.

- [ ] **3.1 Ownership de negocio:** payment intents, transacciones, idempotencia comercial, fees y máquina de estados pertenecen al módulo `payments` del backend de plataforma.
- [ ] **3.2 Contrato inter-backend:** gRPC versionado para operaciones normalizadas y `ExecuteAuthorizedGatewayOperation` con concesión firmada; identidad de servicio, tenant/entorno/cuenta, operación, monto/moneda, deadline, errores tipados y trazabilidad.
- [ ] **3.3 `gateway_credentials`:** módulo/crate dueño del storage ciphertext, schema/migraciones y ciclo de vida de credenciales tenant, con envelope encryption, KMS externo, rotación, revocación y lectura solo por adapter autorizado.
- [ ] **3.4 `gateway_wompi`:** módulo/crate independiente con adapter real, contrato de credenciales requerido (sin persistir secretos), mapeos, firma oficial, sandbox y suite de conformidad.
- [ ] **3.5 `gateway_bold`:** módulo/crate independiente con adapter real, contrato de credenciales requerido (sin persistir secretos), mapeos, firma oficial, sandbox y suite de conformidad.
- [ ] **3.6 `gateway_<provider>`:** cada proveedor futuro tiene crate/schema técnico/suite independientes y describe campos secretos requeridos a `gateway_credentials`; habilitación solo tras verificación oficial de contrato, cuenta, moneda, métodos y sandbox.
- [ ] **3.6 Autorización obligatoria:** el backend pasarelas consulta a plataforma antes de operaciones protegidas y falla cerrado ante deny, timeout, revocación o contrato incompatible; no mantiene reglas de plan/cuota ni bypass local.
- [ ] **3.7 Extensión y capacidades:** registrar crate en el runtime del backend pasarelas; capabilities reales consultables por plataforma. Agregar un proveedor no cambia adapters existentes ni contiene `match` central por nombre.
- [ ] **3.8 Idempotencia/estado:** PostgreSQL como autoridad de negocio de pagos en plataforma; Redis no es barrera única ni fuente de autorización.
- [ ] **3.9 Transporte seguro:** altas/rotaciones viajan por comando gRPC sobre mTLS con caller/método/scope y concesión firmada; plataforma no persiste ni publica secretos en eventos.
- [ ] **3.10 Errores/reintentos:** política por proveedor y operación; en resultado incierto consultar/reconciliar antes de repetir operación financiera; diagnósticos correlacionados ambos lados del RPC.
- [ ] **3.11 Contrato normalizado de pago:** `payment_method` usa unión discriminada versionada `card|pse|crypto`; settlement cripto es decimal string aparte del amount comercial entero y no copia payloads crudos.
- [ ] **3.12 Seguridad inter-backend:** identidad de servicio mTLS rotatoria, autorización por método/scope, validación tipos/tamaño/claims, timeouts, rate limits, retries idempotentes y pruebas negativas/recuperación.

**Criterio de salida:** contratos inter-backend, vault de credenciales y autorización pasan aceptación; credenciales nunca aparecen en claro o en eventos; cada adapter habilitado pasa suite común contra sandbox/protocolo oficial; métodos card/PSE/crypto normalizados y versionados; concurrencia de idempotencia no duplica efectos. Añadir un proveedor no modifica adapters existentes y ningún request cruza la frontera sin autorización vigente.

---

### Fase 4: Webhooks entrantes y WebSockets
**Objetivo:** recibir pagos y reflejarlos en tiempo real. **Duración:** 8 a 10 días.

- [ ] **4.1 Endpoint HTTPS** por proveedor, preservando body crudo y headers firmados; el enrutamiento no confía en un `merchant_id` controlado por el emisor.
- [ ] **4.2 Verificación criptográfica** del protocolo oficial específico antes de aceptar o aplicar el evento. Conservar bytes crudos y permitir solo la extracción estructural mínima que exija el protocolo para verificar; no mapear ni ejecutar transiciones de negocio antes de autenticar. HMAC-SHA256 con comparación segura cuando el contrato lo define, más anti-replay/frescura cuando aplique. No normalizar algoritmos incompatibles a una firma inventada.
- [ ] **4.3 Registro** completo en schema `incoming_webhook_logs` propiedad del módulo de pasarela que recibe el callback.
- [ ] **4.4 Entrega entre backends:** pasarela persiste recepción verificada e idempotente más su Outbox; JetStream entrega el evento durable a plataforma, que aplica transición de pago y escribe su propio Outbox en una transacción local. No se simula una transacción SQL distribuida.
- [ ] **4.5 Eventos fuera de orden y duplicados:** reglas documentadas, deduplicación durable y protección contra regresiones de estado.
- [ ] **4.6 Servidor WebSocket:** canal autenticado y autorizado a nivel de comercio/transacción. No exponer tokens en URL; proteger contra enumeración de IDs.
- [ ] **4.7 Hub Tokio/realtime de plataforma:** suscripciones por transacción autorizada, heartbeat, límites por comercio, cierre y backpressure; el estado durable se recupera por REST.
- [ ] **4.8 Resiliencia:** enviar el estado actual al conectar; descartar clientes lentos (backpressure).

**Criterio de salida:** aceptación de proveedor/sandbox o vector oficial de firma verifica webhook válido → Outbox durable en backend pasarela → consumo idempotente y transición durable en plataforma → Outbox → WebSocket autorizado; firma inválida, duplicado, evento obsoleto, otro entorno y otro comercio no cambian el pago. Códigos de respuesta/reintento siguen el contrato del proveedor.

**Referencias oficiales que deben volver a verificarse al implementar cada adapter:** [eventos de Wompi Colombia](https://docs.wompi.co/docs/colombia/eventos/) y [webhooks de Bold Colombia](https://developers.bold.co/webhook). Wompi documenta un checksum SHA-256 construido con propiedades declaradas, timestamp y secreto; Bold documenta HMAC-SHA256 con `x-bold-signature` sobre el cuerpo codificado en Base64. No se debe imponer a ambos el mismo cálculo.

---

### Fase 5: API pública de intenciones de pago
**Objetivo:** superficie pública estable. **Duración:** 6 a 8 días.

- [ ] **5.1 Middleware de API Key:** búsqueda por prefijo + hash, aislamiento tenant y `live/test`, revocación, permisos y `last_used_at`.
- [ ] **5.2 Endpoints:** `POST /v1/payment_intents` (con `Idempotency-Key`), `GET /v1/payment_intents/{id}`, `POST .../cancel`.
- [ ] **5.3 Validación:** monto positivo dentro de los límites del proveedor, moneda/método realmente habilitados, límites y allowlist de `metadata`; no aceptar capacidades inferidas por el frontend.
- [ ] **5.4 Rate limiting** por API Key (token bucket).
- [ ] **5.5 Errores** con formato uniforme `{ error: { type, code, message } }`.
- [ ] **5.6 OpenAPI** generado (`utoipa`) y contrato versionado de requests/responses.
- [ ] **5.7 Admin API** interna para el dashboard (llaves, endpoints, pasarelas).

**Criterio de salida:** pruebas HTTP→BFF→gRPC→módulo prueban autenticación, autorización tenant, API Key inválida/revocada, aislamiento `test/live`, idempotencia concurrente, validación, rate limit y errores seguros.

---

### Fase 6: Worker de webhooks salientes
**Objetivo:** entrega fiable al comercio. **Duración:** 8 a 10 días.

- [ ] **6.1 Payload unificado versionado** (`id`, `object`, `type`, `created_at`, `api_version`, `data.transaction`, `data.customer` opcional y `metadata` limitada); serialización estable, datos mínimos y sin PAN/CVV.
- [ ] **6.2 Relay del outbox** a NATS JetStream.
- [ ] **6.3 Consumer durable:** ack explícito, `max_deliver`, DLQ para mensajes envenenados.
- [ ] **6.4 Firma:** `X-Payvia-Signature: t=<ts>,v1=<hmac_sha256>` sobre bytes exactos conforme a contrato versionado; `User-Agent` identificable/versionado (por ejemplo, `Payvia-Worker/1.0.0`) y vectores de firma compartidos.
- [ ] **6.5 Cliente HTTP seguro:** timeouts, TLS obligatorio, sin redirecciones, **protección SSRF**.
- [ ] **6.6 Reintentos acotados:** 2 min → 5 min → 15 min → 1 h como calendario inicial, con jitter, timeouts, máximo de intentos configurable y estado terminal inspeccionable; no marcar como fallido antes de completar los intentos permitidos.
- [ ] **6.7 Registro** de cada intento en `outgoing_webhook_deliveries`.
- [ ] **6.8 Reintento manual** y evento de prueba desde el dashboard.
- [ ] **6.9 Métricas:** intentos, latencia y tasa de éxito.

**Criterio de salida:** aceptación del worker sobre NATS/PostgreSQL y receptor HTTP aislado comprueba `2xx`, `4xx`, `5xx`, timeout, firma, reentrega/redelivery, persistencia de cada intento, SSRF y recuperación tras reinicio. Las pruebas de proveedor usan sandbox oficial; un receptor local no valida una pasarela externa.

---

### Fase 7: Dashboard (React + TypeScript)
**Objetivo:** panel para el comercio. **Duración:** 10 a 14 días.

- [ ] **7.1 Base:** estructura por features, router, cliente API tipado, sesión, layout, estados de carga y error.
- [ ] **7.2 Módulo No-Code:** links de pago (monto, descripción, referencia), QR descargable, listado.
- [ ] **7.3 Módulo Developer:** credenciales enmascaradas, generar y revocar `live_key`/`test_key` (visible una sola vez), URL y secreto del webhook, evento de prueba, tabla de entregas.
- [ ] **7.4 Módulo de Pasarelas:** formularios seguros (campos tipo password, solo últimos 4 caracteres), validación y estado de conexión.
- [ ] **7.5 Hook `useTransactionSocket`:** reconexión exponencial, heartbeat, cleanup, estados tipados.
- [ ] **7.6 Vista de transacciones:** filtros, detalle y línea de tiempo de eventos.
- [ ] **7.7 Calidad:** Vitest + Testing Library, accesibilidad básica, TypeScript estricto sin `any`.
- [ ] **7.8 Frontend de administración de plataforma:** aplicación separada para operaciones de plataforma con autenticación/roles propios, auditoría y flujos administrativos expuestos únicamente por REST/OpenAPI del BFF; nunca reutiliza permisos tenant como autoridad.
- [ ] **7.9 Sincronización central:** coordinadores TanStack Query independientes en tenant y administración, deduplicación/orden por `event_id` y cursor, detección de huecos, reconexión y recuperación REST autorizada.

**Criterio de salida:** ambas aplicaciones independientes completan sus flujos contra HTTP→BFF→gRPC→módulo; aceptación de extremo a extremo (crear intención → evento auténtico de sandbox → cambio persistido/Outbox → JetStream → WebSocket autorizado → caché UI actualizada) y recuperación por REST tras pérdida/reinicio/conexión. Se prueban permisos separados, errores seguros y propagación/redacción de telemetría. Una respuesta simulada no valida integración con proveedor.

---

### Fase 8: Pruebas, seguridad y hardening
**Objetivo:** confianza para producción. **Duración:** 6 a 8 días.

- [ ] **8.1 Integración E2E** con PostgreSQL, Redis y NATS JetStream reales en entorno aislado de pruebas, sin acceso a producción.
- [ ] **8.2 Modelo de amenazas (STRIDE):** replay, firma falsificada, SSRF, enumeración de IDs, fuga de secretos.
- [ ] **8.3 Carga:** k6 u `oha` sobre webhooks e idempotencia bajo concurrencia.
- [ ] **8.4 Dependencias:** `cargo audit`, `cargo deny`, `npm audit`.
- [ ] **8.5 Fuzzing** del parser de webhooks y del verificador de firmas.
- [ ] **8.6 Pruebas de resiliencia/reinicio:** caída de Redis, NATS y PostgreSQL en servicios de desarrollo autorizados; documentar antes el comando/efecto y verificar reconexión, replay, recuperación y ausencia de duplicados. Nunca reiniciar producción.
- [ ] **8.7 Revisión de seguridad** externa o par (checklist OWASP ASVS nivel 2).
- [ ] **8.8 Telemetría y pruebas de errores:** verificación de Sentry en entorno de prueba aislado, logs/trazas/métricas, cardinalidad controlada, dashboards/alertas SLO y fallo/degradación del pipeline sin filtrar datos sensibles.
- [ ] **8.9 Cobertura de contratos y calidad:** REST/gRPC/OpenAPI/protobuf compatibles, pruebas de propiedad/concurrencia, fuzzing de parsers/firmas, dependencias y reporte de cobertura de rutas/errores críticas.

**Criterio de salida:** sin hallazgos críticos o altos abiertos; errores operativos trazables de punta a punta, recuperación de dependencias verificada, matrices/cobertura aceptadas y evidencia reproducible disponible.

---

### Fase 9: Despliegue y operación
**Objetivo:** operar con seguridad. **Duración:** 6 a 8 días.

- [ ] **9.1 Contenedores:** Dockerfiles multi-stage, imágenes mínimas, usuario no root.
- [ ] **9.2 CI/CD** (GitHub Actions): fmt, clippy, tests, migraciones, build y escaneo de imágenes.
- [ ] **9.3 Entornos:** `dev`, `staging` y `prod` separados, con secretos gestionados (no en el repo).
- [ ] **9.4 Salud:** healthchecks, readiness, apagado ordenado de API, WS y worker.
- [ ] **9.5 Observabilidad:** métricas (Prometheus), trazas (OpenTelemetry), dashboards y alertas.
- [ ] **9.6 Backups:** Postgres con restauración probada, política de retención.
- [ ] **9.7 Runbooks:** rotación de secretos, reproceso de webhooks fallidos, incidentes.

**Criterio de salida:** despliegue automatizado a staging y restauración de backup verificada.

---

### Fase 10: Piloto y lanzamiento (v1.0)
**Objetivo:** validar con comercios reales. **Duración:** 1 a 2 semanas.

- [ ] **10.1 Piloto cerrado** con 1 a 3 comercios en modo `test`, luego `live` con montos bajos.
- [ ] **10.2 Conciliación** manual de los primeros pagos contra los reportes de las pasarelas.
- [ ] **10.3 Documentación pública:** guía de inicio, referencia de API, guía de webhooks, FAQ.
- [ ] **10.4 Legal:** términos de servicio, política de privacidad y tratamiento de datos (Ley 1581 / Habeas Data).
- [ ] **10.5 Soporte:** canal de contacto, plantilla de incidentes y página de estado.
- [ ] **10.6 Lanzamiento:** etiqueta `v1.0.0`, notas de versión y comunicado.

**Criterio de salida:** cero incidentes de pérdida o duplicación de pagos durante el piloto.

---

## 6. Calidad, seguridad y cumplimiento

### Objetivos de nivel de servicio (SLO) sugeridos

| Indicador | Objetivo |
|---|---|
| Disponibilidad de la API pública | ≥ 99,9 % mensual |
| Latencia p95 de `POST /v1/payment_intents` | < 300 ms (sin contar la pasarela) |
| Latencia de webhook entrante → mensaje WS | < 200 ms p95 |
| Entrega de webhooks salientes (primer intento) | ≥ 98 % |
| Pérdida de eventos | 0 eventos confirmados perdidos; verificar mediante Outbox/JetStream y conciliación |

Estos valores son objetivos iniciales propuestos, no SLO comprometidos hasta definir ventana de medición, presupuesto de error, capacidad, instrumentación y aceptación operativa antes del piloto.

### Cumplimiento

- **PCI DSS:** mantener el alcance mínimo; no almacenar PAN, CVV ni datos de banda. Usar tokenización y checkout de las pasarelas. Evaluar el cuestionario SAQ aplicable antes del lanzamiento.
- **Habeas Data (Ley 1581 de 2012):** minimizar datos personales (`holder_name`, `phone`, `legal_id`), hacerlos opcionales o configurables por comercio y definir retención.
- **Auditoría:** `transaction_events` y logs de webhooks inmutables (append-only).

---

## 7. Riesgos y mitigaciones

| Riesgo | Probabilidad | Impacto | Mitigación |
|---|---|---|---|
| Cambios en la API o firma de una pasarela | Media | Alto | Trait de abstracción, tests de contrato, monitoreo de changelogs |
| Webhooks duplicados o fuera de orden | Alta | Alto | Idempotencia en dos capas y máquina de estados estricta |
| Fuga de credenciales de pasarela | Baja | Crítico | Cifrado en reposo, rotación, acceso mínimo, sin logs |
| SSRF vía URL de webhook del comercio | Media | Alto | Bloqueo de rangos privados, revalidación DNS, sin redirecciones |
| Caída de NATS o Redis | Media | Medio | Outbox transaccional, reintentos, degradación controlada |
| Alcance excesivo | Alta | Medio | Hitos cortos, Definition of Done, funcionalidades fuera de v1.0 explícitas |
| Incumplimiento regulatorio | Media | Alto | Revisión legal antes del piloto, minimización de datos |

---

## 8. Después de la v1.0

### v1.1: Pasarelas y pagos
- [ ] Mercado Pago, Stripe y PayPal a nivel estable.
- [ ] Reembolsos totales y parciales desde API y dashboard.
- [ ] PSE, Nequi, Daviplata y efectivo (Efecty, Baloto) según disponibilidad de cada pasarela.
- [ ] Enrutamiento inteligente: reglas por monto, método o disponibilidad, con failover entre pasarelas.

### v1.2: Experiencia del desarrollador
- [ ] SDKs oficiales: Node/TypeScript, Python, PHP y Go.
- [ ] Plugins para Shopify, WooCommerce y PrestaShop.
- [ ] CLI para probar webhooks en local (túnel + reenvío).
- [ ] Sandbox con escenarios simulados (aprobado, rechazado, expirado).

### v1.3: Finanzas
- [ ] Conciliación automática contra reportes de las pasarelas.
- [ ] Liquidaciones y reportes exportables (CSV, PDF).
- [ ] Facturación electrónica (DIAN) a través de un proveedor.
- [ ] Multi-moneda con tasas de cambio.

### v2.0: Expansión de pagos
- [ ] Suscripciones y cobros recurrentes.
- [ ] Marketplace con splits y pagos a terceros.
- [ ] SSO y roles avanzados de equipo (el aislamiento tenant y permisos básicos son requisitos de la v1.0).
- [ ] Detección de fraude con reglas y scoring.
- [ ] Multi-región y alta disponibilidad activa-activa.

---

## 9. Seguimiento del progreso

| Fase | Estado | Inicio | Fin | Notas |
|---|---|---|---|---|
| 0. Arquitectura | ⬜ Pendiente | | | |
| 1. Base de datos | ⬜ Pendiente | | | |
| 2. Protobuf | ⬜ Pendiente | | | |
| 3. Core | ⬜ Pendiente | | | |
| 4. Webhooks + WS | ⬜ Pendiente | | | |
| 5. API pública | ⬜ Pendiente | | | |
| 6. Worker saliente | ⬜ Pendiente | | | |
| 7. Dashboard | ⬜ Pendiente | | | |
| 8. Hardening | ⬜ Pendiente | | | |
| 9. Despliegue | ⬜ Pendiente | | | |
| 10. Piloto y v1.0 | ⬜ Pendiente | | | |

Leyenda: ⬜ Pendiente · 🟨 En curso · 🟩 Completada · 🟥 Bloqueada

---

## 10. Gobierno del roadmap

- Este documento se revisa **al cierre de cada hito** y se versiona en Git.
- Los cambios de alcance se registran como ADR (Architecture Decision Record) en `docs/adr/`.
- Las etiquetas de Git siguen SemVer: `v0.1.0`, `v0.2.0`, … `v1.0.0`.
- Los issues se organizan en milestones M1 a M7, uno por hito.
