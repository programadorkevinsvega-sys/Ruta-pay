# Reglas específicas del proyecto Payvia

## Autoridad y alcance

El workspace Cargo es único y está configurado en el `Cargo.toml` de la raíz, que incluye ambos backends; cada backend tiene un manifiesto de paquete miembro que hereda la configuración común; ambos backends se compilan desde esa resolución compartida como aplicaciones y despliegues independientes. No se permiten workspaces anidados por backend ni dependencias de implementación cruzadas. Los frontends viven en `frontends/frontend-pasarela/` y `frontends/frontend-plataform/`, cada uno como aplicación web separada.


- [ROADMAP.md](ROADMAP.md) define el alcance del producto, sus fases y criterios de salida.
- [ARCHITECTURE_RULES.md](ARCHITECTURE_RULES.md) contiene el contrato arquitectónico universal para el monolito modular Rust. Es una copia de [las reglas canónicas](bacekend-pasarela/reglas-estrictas-de-arquitectura-monolito-modular-rust-universal.md); cualquier cambio a reglas universales debe conservar su sincronización.
- Los prompts de [calidad](bacekend-pasarela/PROMPT_MAESTRO_ENTERPRISE_QUALITY_ENGINEERING_PARA_PROYECTOS_RUST.md) y [observabilidad](bacekend-pasarela/PROMPT_MAESTRO_IMPLEMENTACION_DE_OBSERVABILIDAD_TRAZABILIDAD_Y_MONITOREO_EMPRESARIAL_EN_RUST.md) fijan el método de verificación y diagnóstico.
- Las copias de los prompts de calidad, observabilidad y reglas modulares bajo `bacekend-plataform/` y `bacekend-pasarela/` deben mantenerse sincronizadas.
- Este archivo contiene solo decisiones específicas de Payvia; la arquitectura universal se mantiene en su documento canónico.

### Aplicación obligatoria desde el día uno

Los seis documentos listados abajo son requisitos normativos del proyecto y se implementan desde el diseño inicial en todos los módulos/crates y en toda la aplicación, incluidos workers, APIs, frontends e infraestructura, dentro del alcance aplicable a cada backend. La Fase 0 de cada backend es auditoría, decisiones y workspace, sin runtime; sus criterios son artefactos documentales y verificaciones estáticas. La calidad, seguridad, observabilidad, resiliencia, contratos y pruebas de software aplican desde la primera fase de implementación y no se aplazan después. Antes de implementar, cualquier requisito considerado no aplicable debe contar con una justificación técnica concreta y verificable en la documentación de la fase/subfase.

| Backend | Documentos normativos obligatorios |
|---|---|
| Pasarelas | [Calidad](bacekend-pasarela/PROMPT_MAESTRO_ENTERPRISE_QUALITY_ENGINEERING_PARA_PROYECTOS_RUST.md), [observabilidad](bacekend-pasarela/PROMPT_MAESTRO_IMPLEMENTACION_DE_OBSERVABILIDAD_TRAZABILIDAD_Y_MONITOREO_EMPRESARIAL_EN_RUST.md), [arquitectura modular](bacekend-pasarela/reglas-estrictas-de-arquitectura-monolito-modular-rust-universal.md) |
| Plataforma | [Calidad](bacekend-plataform/PROMPT_MAESTRO_ENTERPRISE_QUALITY_ENGINEERING_PARA_PROYECTOS_RUST.md), [observabilidad](bacekend-plataform/PROMPT_MAESTRO_IMPLEMENTACION_DE_OBSERVABILIDAD_TRAZABILIDAD_Y_MONITOREO_EMPRESARIAL_EN_RUST.md), [arquitectura modular](bacekend-plataform/reglas-estrictas-de-arquitectura-monolito-modular-rust-universal.md) |

Se aplican conjuntamente con `ARCHITECTURE_RULES.md`, los `AGENTS.md` aplicables y el `PROJECT_RULES.md` de cada backend. La arquitectura de Payvia establece ownership y fronteras; los prompts concretan las obligaciones de calidad y observabilidad. Toda incompatibilidad debe documentarse y resolverse antes de implementar el alcance afectado, sin omitir ni reducir requisitos.

## Topología de los dos backends

El sistema tiene dos backends desplegables independientes y cada uno es internamente un monolito modular Rust:

- [`bacekend-plataform`](bacekend-plataform/README.md) es autoridad de tenant registry, identidad, planes, precios, límites, cuotas, suscripciones, facturación por uso de Payvia y reglas de negocio de payment intents/transacciones. Expone BFF REST/OpenAPI y decide la autorización que se adjunta como concesión firmada a comandos gRPC de pasarelas. También usa adapters propios en ese backend para cobrar a los tenants (Wompi), emitir facturas electrónicas de Payvia (Factus) y enviar correos operativos (Resend).
- [`bacekend-pasarela`](bacekend-pasarela/README.md) implementa únicamente integración/protocolo técnico de Wompi, Bold y proveedores futuros, un módulo/crate por proveedor. No posee política comercial, planes, cuotas ni transiciones de negocio de pagos.

Plataforma decide antes de operar y envía un comando gRPC autenticado con una concesión firmada, breve y ligada a tenant, entorno, cuenta, acción e importe. Pasarelas verifica concesión y claims; una concesión ausente, alterada, expirada o incompatible significa fail closed. Los eventos verificados de proveedor se entregan de forma durable a plataforma, que decide y persiste el resultado de negocio. Los detalles, fases y aceptación pertenecen a los [roadmaps separados](bacekend-plataform/ROADMAP.md) y [roadmap de pasarelas](bacekend-pasarela/ROADMAP.md).

La topología y los límites de responsabilidad están definidos en `ARCHITECTURE_RULES.md`. Antes de implementar cada backend, sus ADRs detallan contrato gRPC, concesión de autorización, ownership, migraciones coordinadas, operación y pruebas de integración; no cambian las reglas transversales de seguridad, calidad u observabilidad.

## Identidad del producto

Payvia es un servicio SaaS multi-comercio de orquestación de pagos. Cada comercio integra una API común para crear intenciones de pago, recibe estados normalizados y configura una o varias cuentas de pasarela. El backend administra credenciales, idempotencia, webhooks, notificaciones, facturación interna y auditoría.

La primera versión contempla Wompi y Bold como integraciones prioritarias. Mercado Pago, Stripe y PayPal se incorporan únicamente tras validar sus APIs, métodos, monedas, cuentas elegibles, esquemas de autenticación, firmas de webhook y entornos de prueba. No se declaran adapters completos por tener solo tipos o scaffolds.

## Decisiones de arquitectura

| Área | Decisión del proyecto |
|---|---|
| Lenguaje y async | Rust estable con versión fijada; Tokio |
| Despliegue | Dos aplicaciones independientes; cada backend es un monolito modular Rust |
| Front door | BFF Axum REST/OpenAPI en plataforma; pasarelas no recibe tráfico tenant desde frontend |
| Comunicación interna | gRPC versionado dentro de cada backend y plataforma→pasarelas; concesiones de autorización se verifican en cada comando |
| Comunicación asíncrona | NATS JetStream con eventos versionados y Outbox transaccional para cambios durables entre dominios/backends |
| Persistencia | Una base física PostgreSQL compartida por ambos monolitos; schema y migraciones propiedad de cada módulo, sin acceso SQL cruzado; migraciones coordinadas |
| Redis | Cache, rate-limit o coordinación técnica efímera con TTL; nunca autoridad del estado de pago |
| Pasarelas | Un crate/módulo independiente por proveedor y un módulo `gateway_credentials` dueño del ciclo de vida/cifrado de credenciales; contratos gRPC comunes y versionados; secretos nunca entregados al navegador |
| Integraciones propias de plataforma | Módulos/adapters independientes `platform_billing_wompi`, `invoice_factus` y `notification_resend`; cuentas/credenciales de Payvia separadas de las credenciales de tenants; cada adapter implementa el contrato de su dominio dueño |
| Frontends | Aplicaciones React + TypeScript independientes para comercio y administración de plataforma; ambas consumen solo REST/OpenAPI y WebSocket autenticado del BFF |
| Dinero | Enteros en unidades menores de la moneda; no usar punto flotante para importes |
| Observabilidad | `tracing`, OpenTelemetry, Sentry, métricas y logs estructurados con redacción segura |

El BFF de plataforma adapta REST a gRPC y no accede directamente a PostgreSQL, Redis o NATS. Plataforma contiene las reglas de negocio. Pasarelas traduce operaciones autorizadas a protocolos externos y no decide planes, cuotas o transiciones de pagos. Los módulos poseen sus datos; no se permiten JOINs entre schemas ni escrituras cruzadas.

Los frontends de comercio y administración de plataforma son aplicaciones separadas, con autenticación, autorización y navegación propias. Ninguno llama directamente a módulos, proveedores, gRPC, PostgreSQL, Redis o NATS.

## Bounded contexts objetivo

El conjunto se ajusta si el análisis de dominio encuentra un límite mejor; no se crean crates solo por conveniencia técnica.

| Módulo | Responsabilidad exclusiva | Datos principales |
|---|---|---|
| `identity_access` (plataforma) | Identidades, sesiones, membresías y permisos tenant/admin | identidades y membresías |
| `tenant_registry` (plataforma) | Registro, verificación, estado y ciclo de vida del tenant | tenants y cambios auditados |
| `plan_catalog` (plataforma) | Planes, precios versionados, límites, cuotas y entitlement | catálogo/versions/consumo |
| `platform_billing` (plataforma) | Suscripción e invoices por uso de Payvia | suscripciones, cargos e invoices |
| `platform_billing_wompi` (plataforma) | Adapter técnico de Wompi para cobrar facturas/suscripciones propias de Payvia y validar sus callbacks | referencias de cobro Wompi; secretos Payvia solo mediante configuración protegida |
| `electronic_invoicing` (plataforma) | Estado de emisión y referencias de facturas electrónicas que Payvia emite a tenants | documentos electrónicos, estados, referencia externa y auditoría |
| `invoice_factus` (plataforma) | Adapter API de Factus tras el contrato dueño de facturación electrónica | sin ownership de la factura de negocio ni secretos persistidos en tablas de dominio |
| `notifications` / `notification_resend` (plataforma) | Reglas, plantillas y registro de notificaciones, con Resend como adapter inicial de envío | estado/outbox de notificación; credenciales fuera de PostgreSQL |
| `payments` (plataforma) | Payment intents, transacciones, idempotencia y transiciones de negocio | intents, transacciones, ledger/outbox |
| `gateway_credentials` (pasarelas) | Ingreso autorizado, cifrado, rotación, estado y entrega en memoria de secretos a adapters autorizados | schema/ciphertext/versiones de clave y metadata mínima |
| `gateway_wompi` (pasarelas) | Protocolo/API, contrato de credenciales requerido, callback y verificación oficial Wompi | schema Wompi no secreto |
| `gateway_bold` (pasarelas) | Protocolo/API, contrato de credenciales requerido, callback y verificación oficial Bold | schema Bold no secreto |
| `gateway_<provider>` (pasarelas) | Un crate por proveedor adicional, sin mezclar adapters; define sus campos requeridos | schema del proveedor no secreto |
| `audit`, `realtime`, `webhook_delivery` (plataforma) | Auditoría, distribución autorizada y notificación de sistemas tenant | datos append-only, cursores y entregas |

Las credenciales enviadas por un tenant se guardan exclusivamente en `gateway_credentials`, no en la plataforma, el frontend, eventos, logs ni tablas en claro. El módulo de credenciales es técnico y no decide permisos comerciales; plataforma autoriza quién puede configurarlas y para qué tenant, y el adapter correcto solo las recibe en memoria para la operación autorizada. Cada módulo posee sus propias tablas/migraciones; ningún crate comparte acceso SQL ni hay un `db` genérico dueño de datos.

## Extensión por pasarela

Cada pasarela es un bounded context/crate independiente que implementa el contrato público gRPC de gateway y expone su identificador, versión y capacidades reales (métodos, monedas, operaciones, ambientes y parámetros configurables). El módulo `payments` selecciona y llama al módulo proveedor por descriptor/registro de runtime; no contiene `match` por proveedor, SDK del proveedor ni ramas de negocio específicas. El contrato compartido define operaciones normalizadas, errores tipados, contexto tenant/entorno/traza y compatibilidad de versiones.

Para incorporar un proveedor se añade su crate/módulo, su schema y migraciones propias, el adapter/validación de webhook, su registro en el workspace/runtime y sus pruebas de conformidad en sandbox. No se modifica la lógica de `payments` ni los módulos de pasarela existentes para agregar una implementación compatible. La composición debe permitir habilitarlo por configuración y el dashboard debe leer su descriptor para exponer solo sus capacidades. Las extensiones incompatibles del contrato requieren versionado compatible y no pueden romper adapters existentes.

## Integraciones propias de plataforma

El recaudo de las suscripciones/facturas de Payvia no pasa por [`bacekend-pasarela`](bacekend-pasarela/README.md): `platform_billing` mantiene la obligación y el estado financiero, y un módulo proveedor independiente (`platform_billing_wompi` inicialmente) ejecuta el protocolo de la cuenta de Payvia. El Wompi de plataforma y el `gateway_wompi` de pasarelas son módulos distintos, con identidades, credenciales, callbacks, auditoría y ciclo de despliegue del backend propietario. No se comparten credenciales tenant ni se mezclan pagos de consumidores.

`electronic_invoicing` conserva el estado de las facturas electrónicas de Payvia emitidas a sus tenants; `invoice_factus` implementa la integración Factus. `notifications` conserva reglas y mensajes; `notification_resend` implementa el envío por Resend. Las claves/API secrets de Wompi, Factus y Resend se inyectan desde configuración protegida del entorno, no se guardan en PostgreSQL, repositorios, eventos ni logs.

Cada integración se consume por el contrato gRPC del módulo dueño. Se registra en el composition root mediante descriptor y configuración, no con `match` por proveedor dentro de la lógica comercial. Para agregar o retirar un proveedor se añade/retira su adapter y su registro, se revisan credenciales/contratos y datos externos; no se reescribe `platform_billing`, el dominio fiscal ni `notifications`. Ningún cambio elimina los documentos contables/fiscales o el historial durable que el proveedor anterior ya generó.

## Multitenancy y aislamiento

- El comercio es el tenant del producto. Identidad, API Key y configuración de pasarelas deben resolver explícitamente a un comercio y un entorno (`test` o `live`).
- Cada operación valida autenticación, estado de la cuenta, permiso y propiedad del recurso dentro del contexto del comercio. Un ID conocido no concede acceso.
- Las lecturas, escrituras, búsquedas idempotentes, rate limits y claves únicas se segmentan por comercio y entorno cuando corresponda.
- Las API Keys se generan con entropía suficiente, se muestran una sola vez, se almacenan solo como hash y se pueden revocar. El prefijo sirve para localizar la llave; no autentica por sí solo.
- Credenciales de proveedor y secretos de webhook se cifran en reposo, tienen rotación/versionado y no aparecen en logs, errores ni respuestas de dashboard.
- La administración de plataforma usa identidad y autorización separadas de las sesiones de comercio y deja auditoría.

## Contratos funcionales objetivo

- API pública versionada: `POST /v1/payment_intents`, `GET /v1/payment_intents/{id}` y operaciones de cancelación si el proveedor/estado las permite. Cada request autenticado valida API Key, entorno, tenant, esquema, monto, moneda, metadata y `Idempotency-Key`.
- Webhooks entrantes: endpoint HTTPS por proveedor que conserva bytes crudos y headers necesarios, valida el protocolo de firma oficial y anti-replay antes de confiar en el payload, deduplica por identificador estable y aplica una transición válida.
- Webhooks salientes: payload JSON versionado con `id` (`evt_`), `object`, `type` (por ejemplo `payment.successful`), `created_at`, `api_version` y `data`. `data.transaction` incluye `id`, `reference`, `amount`, `currency`, `status`, método normalizado, gateway/recibo y `fee_internal_cents`; `data.customer` es opcional; `metadata` se valida, limita y devuelve sin alterar sus valores aceptados. Incluir datos de cliente solo si el contrato y propósito lo requieren; nunca PAN/CVV.
- `payment_method` es una unión discriminada versionada: `type=card|pse|crypto` y `details` contiene solo el esquema permitido para ese tipo. Tarjeta expone marca y últimos cuatro dígitos; PSE, código/nombre de banco y referencia permitida; cripto, activo, moneda y cantidad decimal como string, además de identificadores de orden/transacción disponibles. No enviar objetos crudos de proveedor, secretos, direcciones/IDs personales innecesarios ni PAN/CVV.
- El `payment_method` de salida del evento no es automáticamente el schema de entrada de `payment_intents`: la API de creación acepta únicamente el selector y los datos que el checkout/backend realmente necesite según OpenAPI/capacidades; datos de settlement e IDs emitidos por el proveedor son output-only.
- El `amount` común de transacción conserva el entero en unidades menores de la moneda facturada. El monto cripto de liquidación se representa aparte como decimal string y moneda/activo para no asumir exponentes iguales ni usar punto flotante.
- Firma saliente: header `X-Payvia-Signature: t=<timestamp>,v1=<hmac_sha256>` y `User-Agent` versionado (por ejemplo, `Payvia-Worker/1.0.0`). El esquema exacto de bytes firmados se define en el contrato y se prueba con vectores compartidos antes de publicar la integración.
- Reintentos salientes iniciales: 2, 5, 15 y 60 minutos, con jitter configurable, límites, registro de cada intento y mecanismo autorizado de reenvío.
- WebSocket: el usuario obtiene acceso a la transacción únicamente después de validar sesión, tenant y permiso. El canal comunica cambios mínimos y no reemplaza la consulta REST autorizada.
- Errores REST: envelope estable, seguro y accionable con código de dominio, detalles de validación permitidos y correlation ID; nunca incluye SQL, secretos o stack traces.

## Política de firmas de proveedores

La autenticación de webhooks es obligatoria, pero el algoritmo y la entrada firmada son propios de cada proveedor. No se fuerza una implementación HMAC común si el proveedor define otro protocolo. Por ejemplo, la documentación de Bold describe HMAC-SHA256 aplicado a un cuerpo en Base64 y `x-bold-signature`; Wompi describe un checksum SHA-256 basado en propiedades del evento, timestamp y secreto. Estos formatos no son intercambiables. Antes de cada adapter se verifica la documentación oficial vigente, se registra la versión/fecha de consulta y se prueban casos válidos, inválidos, alterados, repetidos y de entorno incorrecto.

Referencias oficiales de partida: [Bold](https://developers.bold.co/webhook), [Wompi](https://docs.wompi.co/docs/colombia/eventos/). Estas referencias no sustituyen volver a comprobar el contrato durante la implementación.

## Persistencia, mensajería y tiempo real

- El estado de pago, idempotencia crítica y resultado de negocio son durables en PostgreSQL. Redis puede acelerar el claim, pero fallos o pérdidas de Redis no permiten duplicar cargos ni aceptar estados no confirmados.
- Una transición que deba producir un evento guarda el cambio y el registro Outbox en una transacción local del módulo dueño. El relay publica en JetStream de forma al menos una vez; consumidores idempotentes hacen ACK después de persistir el efecto.
- El webhook saliente es trabajo durable: fallo del worker, timeout, reinicio o redelivery no pierde el evento ni crea intentos inconsistentes.
- Todo cambio persistido visible para comercios u operadores se publica desde Outbox versionado del módulo dueño por NATS JetStream; el gateway realtime modular lo distribuye por WebSocket autenticado del BFF a los frontends pertinentes. El cliente coordina caché/estado con TanStack Query y recupera huecos o reconexiones por REST autorizado con cursor/snapshot.
- Los contratos realtime preservan `event_id`, cursor durable, versión de agregado, audiencia/scope, deduplicación, orden, detección de huecos, permisos y sincronización de recuperación. PostgreSQL conserva la verdad durable y JetStream permite replay; Redis Pub/Sub solo hace fan-out efímero entre réplicas.
- BFF/runtime, consumidores, Redis, JetStream, PostgreSQL y frontends deben tolerar múltiples réplicas, reinicios y reconexiones; no se depende de sticky sessions ni de memoria local como autoridad. Redis usa reconexión acotada y re-suscripción; los consumers reconstruyen sus durables y recuperan pendientes de forma idempotente.
- En cada fase y subfase se documenta previamente una matriz de PostgreSQL/migraciones, frontend→REST/OpenAPI→BFF, BFF→módulo gRPC, Redis, JetStream/Outbox, WebSocket y recuperación. Se justifica cada componente no aplicable y se implementa/verifica la matriz en esa misma fase.

## Contratos seguros entre backends

- Plataforma y pasarelas se autentican con identidad criptográfica de servicio sobre gRPC cifrado y mTLS. Certificados/credenciales tienen rotación con periodo de solapamiento; una IP o header declarativo no acredita identidad.
- Autenticación no equivale a autorización: cada RPC exige allowlist de servicio/método y scope mínimo. Las concesiones firmadas son breves, con audience, tenant, entorno, cuenta, operación, importe cuando aplica, `jti`, expiración e IDs de traza; el receptor deniega por defecto ante cualquier mismatch.
- El gateway valida tipos, campos obligatorios, límites de tamaño, versión, cuenta/tenant, rango monetario y deadline. Protege contra replay y duplicados mediante `jti`/idempotencia durable, rate limits por caller/operación/tenant, timeout acotado, retry limitado con backoff+jitter solo para fallos recuperables y circuit breaker/bulkhead según dependencia.
- gRPC se usa para consultas/comandos síncronos que requieren resultado inmediato. Puede iniciar una escritura durable en el módulo dueño; la elección de transporte no elimina la persistencia que exige el dominio. Outbox/JetStream se usa para hechos persistidos que requieren distribución durable, procesamiento diferido, replay o desacoplamiento. Los eventos no transportan credenciales, concesiones completas, payloads crudos de pago ni secretos.
- La configuración/rotación de credenciales tenant usa comando gRPC síncrono y cifrado de transporte hasta `gateway_credentials`; solo el módulo de credenciales persiste el ciphertext. Si el cambio produce workflows/eventos, estos contienen referencias/versiones y estado, nunca el secreto.
- Ambos lados emiten eventos diagnósticos correlacionados de autorización, dependencia, clasificación de fallo y cierre con `correlation_id`, `trace_id`, duración, código estable y resultado. Se redactan certificados, tokens, concesiones, credenciales, PII y payloads.
- La aceptación cubre caller autorizado/no autorizado, certificado inválido/expirado, rotación sin downtime, timeout/desconexión y recuperación, request malformado/grande, replay, límite de tasa, idempotencia y redacción extremo a extremo.

## Seguridad de pagos y privacidad

- No almacenar ni registrar PAN, CVV, track data ni credenciales de autenticación de tarjeta. Delegar captura/procesamiento a superficies de las pasarelas y validar el alcance PCI antes de producción.
- Aceptar únicamente importes enteros positivos dentro de límites y monedas realmente soportados. La moneda no se deduce de un valor del frontend sin validación de cuenta/proveedor.
- Los webhooks entrantes validan firma con comparación segura cuando aplique, límites de tamaño, antigüedad/replay, entorno, merchant/provider linkage, orden de eventos e idempotencia.
- Los webhooks salientes fuerzan HTTPS, timeouts, sin redirects, defensa SSRF contra loopback/privadas/link-local, revalidación DNS y controles de egress.
- El payload de webhook minimiza datos personales. `email`, `phone`, `legal_id` y `holder_name` son opcionales y solo se emiten con base contractual y necesidad documentada.
- Toda respuesta de pago externa se trata como no confiable hasta verificar autenticidad, pertenencia y estado mediante reglas del dominio.

## Errores y observabilidad

Cada frontera registra inicio, llamada de dependencia, fallo causal clasificado y resultado final, propagando `trace_id` y `correlation_id`. Los eventos incluyen componente, operación, dependencia, clase/código estable, resultado, duración de dependencia y total, además de metadatos técnicos seguros como SQLSTATE/constraint cuando existan. `retryable` se registra solo si la política lo confirma.

Cada operación correlaciona HTTP, gRPC, SQL, llamadas a proveedores y trabajo asíncrono con propagación W3C cuando corresponda. Los logs JSON incluyen `correlation_id`, `trace_id` y `span_id` disponible, nombre estable, componente, operación, dependencia, resultado, categoría/código y duración de dependencia/total; usan rutas normalizadas, cardinalidad acotada y redacción de secretos/PII. `crates/observability` estandariza estas primitivas para ambos backends y no posee lógica de dominio. Métricas permiten calcular latencia p50/p95/p99, solicitudes por segundo, tasa de error, saturación y salud/duración de dependencias. Sentry/OpenTelemetry se exportan fuera del camino crítico: una caída de Sentry o Collector no bloquea operaciones. La infraestructura versiona dashboards de aplicación, PostgreSQL, dependencias, recursos e ingestión, además de alertas por errores, latencia, disponibilidad y saturación.

La aceptación verifica que el evento específico de fallo operativo y el cierre HTTP comparten `correlation_id` y `trace_id`, cada evento conserva su `span_id` correcto, y el cierre mantiene código/categoría. Se prueban propagación y redacción a través de BFF→gRPC→módulo y consumidores, incluidos destinos de telemetría en pruebas aisladas. Los errores de proveedor, conectividad, base de datos, autorización, validación, conflicto y timeout tienen traducción y recuperación documentadas; el cliente recibe un envelope seguro y accionable, nunca SQL, stack trace o secretos. La infraestructura se versiona bajo `infra/`; el Compose raíz es para desarrollo compartido, mientras los backends permanecen desplegables de forma independiente en sus VM/VPS.

Cada fase incluye pruebas unitarias y de propiedades cuando aporten valor, integración con PostgreSQL/Redis/NATS reales aislados, aceptación HTTP→BFF→gRPC→módulo, contratos/versionado, seguridad, concurrencia, errores, resiliencia y rendimiento según aplique. No se usan mocks/datasets prefabricados para sustituir recorridos de integración o proveedores reales; las pruebas crean y limpian sus datos transitorios. Cada cierre informa comandos y resultados reales, limitaciones y bloqueos sin declarar como pasada una prueba no ejecutada.

## Documentación y estado

### Higiene de archivos y README principal

Está prohibido crear archivos, carpetas o documentos Markdown innecesarios, duplicados o sin relación verificable con un requisito, entregable, decisión técnica o mantenimiento del proyecto. Antes de añadir documentación, revisa si debe actualizarse un archivo existente y conserva cada tema en su documento dueño. El `README.md` raíz debe contener solo la presentación y orientación general necesarias: propósito, estado real, arquitectura resumida, requisitos/instrucciones verificadas y enlaces a guías específicas. Los contratos extensos, decisiones, planes detallados y contenido especulativo pertenecen a sus documentos especializados, no al README principal.

### Ramas, verificación e integración

Mantén `main` funcional y verificable. Usa ramas descriptivas para funcionalidades, refactorizaciones importantes, cambios arquitectónicos, correcciones complejas y modificaciones de seguridad; los cambios triviales de documentación pueden ir directamente a `main` si las protecciones lo permiten. No hagas force push ni reescribas historial compartido. Mantén commits pequeños y coherentes, revisa diffs y secretos y ejecuta las verificaciones pertinentes antes de integrar; compilar no basta. Los cambios importantes se proponen en un Pull Request con resumen, decisiones, pruebas y riesgos. No integres con verificaciones obligatorias fallidas ni fusiones automáticamente cambios sensibles de cifrado/autenticación/autorización sin la revisión requerida. Conserva historial auditable y reversión; no añadas proceso innecesario a cambios pequeños.

El código actual es un scaffold mínimo. Antes de cada fase y subfase, su documentación debe definir requisito, ownership, migración, REST/OpenAPI, gRPC, tratamiento Redis/NATS/Outbox, seguridad, errores, integración realtime/recuperación, pruebas y aceptación. Se actualiza el estado solo con evidencia ejecutada; un endpoint, tipo, mock o documento por sí solo no cierra una capacidad. No se inicia la siguiente fase hasta completar todos los criterios aplicables o documentar un bloqueo externo concreto dentro de la fase actual.

Pendientes externos —cuentas, credenciales sandbox, países/monedas habilitados, tarifas y requisitos legales— se anotan por proveedor y bloquean solo las subfases que realmente dependan de ellos. Nunca se sustituyen con capacidades inventadas.
