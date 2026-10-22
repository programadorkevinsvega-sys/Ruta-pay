# Reglas específicas — Backend de Plataforma Payvia

## Autoridad de dominio

Este monolito modular es la autoridad de negocio para tenants, identidad y permisos, API Keys, planes, precios versionados, límites/cuotas, entitlement, suscripciones, facturación que Payvia cobra por el uso, payment intents, transacciones, fees, idempotencia comercial, transiciones de pago, auditoría, webhooks salientes y APIs de recuperación.

También es dueño de sus propias integraciones para facturar el servicio Payvia: `platform_billing_wompi` recauda a los tenants en la cuenta de Payvia, `invoice_factus` integra la facturación electrónica propia y `notification_resend` entrega correos. Son distintas de las pasarelas de pago que los comercios usan para cobrar a sus compradores.

Ningún módulo de pasarelas puede cambiar estos datos ni aplicar reglas de negocio de Payvia. Los schemas son propiedad de cada módulo; no se permiten consultas, JOINs ni escrituras cruzadas con schemas de pasarelas.

## Interacción con el backend de pasarelas

- Toda operación proveedor se autoriza en plataforma antes de enviar el comando.
- El comando gRPC autenticado contiene una concesión firmada y breve ligada a identidad del servicio, tenant, entorno, cuenta, scope/operación, capacidades, monto/moneda pertinente, `jti`, expiración e IDs de trazabilidad.
- Se deniega por defecto si tenant está suspendido, plan/cuota no habilita la acción, límite excedido, concesión no válida o cualquier decisión está incompleta.
- No se hace una llamada RPC circular de vuelta a plataforma durante la ejecución del mismo comando.
- El cobro de suscripciones/facturas de Payvia se ejecuta mediante `platform_billing_wompi`, un adapter propio de este backend. No se delega al backend de pasarelas ni usa sus grants.
- Los callbacks verificados llegan por eventos versionados y durables. Plataforma verifica productor, enlace tenant/cuenta/entorno, duplicados y transición válida antes de modificar estado de negocio.
- El alta/rotación de credenciales de proveedor se autoriza en plataforma y se envía como comando gRPC síncrono cifrado al módulo `gateway_credentials` de pasarelas. El servicio de plataforma no persiste ni publica el secreto en Outbox/NATS; solo recibe resultado seguro y referencia/versionado opacos.

La conexión inter-backend usa identidad criptográfica de servicio con TLS/mTLS y certificados rotables. Plataforma verifica autenticación del servicio receptor y Pasarelas verifica caller, método, scope y concesión ligada exactamente a tenant, proveedor/cuenta, ambiente y operación. Cada lado valida tipos, versión, tamaños/rangos, deadlines e idempotencia; aplica rate limits y retries limitados con backoff+jitter solo para fallos recuperables. Una operación gRPC puede tener efecto persistente en el módulo dueño; se usa Outbox/JetStream cuando ese efecto requiera distribución durable o procesamiento asíncrono. Nunca se publican credenciales en eventos. Pruebas de aceptación cubren caller no autorizado, certificado inválido/expirado y rotación, payload inválido/grande, replay, rate limit, timeout/desconexión, recuperación y no duplicación, con eventos correlacionados y secretos redactados.

## Integraciones propias de plataforma

- `platform_billing` conserva el ownership de suscripciones, invoices, partidas, ledger y estados financieros. `platform_billing_wompi` ejecuta los cobros de Payvia con una cuenta y credenciales propias de Payvia, y recibe únicamente sus callbacks verificados en una ruta de plataforma. No se reutilizan las credenciales tenant ni el módulo `gateway_wompi` de pasarelas.
- `electronic_invoicing` conserva el estado de los documentos fiscales de Payvia y `invoice_factus` implementa el adapter a Factus. `notifications` conserva políticas, destinatarios y estado de entrega; `notification_resend` implementa el adapter de correo Resend.
- Los dominios dependen de contratos neutrales al proveedor. El composition root registra adapters por identificador/capacidad; agregar o retirar un proveedor no debe requerir cambios a reglas del dominio ni a otros adapters. Desactivar un adapter no elimina invoices, ledger, auditoría ni referencias externas históricas.
- Las credenciales de Wompi, Factus y Resend se inyectan desde configuración protegida/secretos de despliegue; no se guardan en tablas de dominio, repositorios, eventos, logs ni respuestas. Tokens temporales de Factus se manejan con expiración y renovación seguras.
- Cada adapter verifica documentación oficial vigente, autenticación, sandbox, idempotencia, límites, callbacks y errores antes de implementarse. No se afirma que cobros recurrentes automáticos estén disponibles hasta validar la capacidad concreta de la cuenta/API Wompi seleccionada.

## Fronteras y datos

El BFF REST/OpenAPI es la única frontera HTTP de tenant y administración. Los dos frontends son apps separadas con autorización distinta. Ningún frontend accede a gRPC interno, módulos, proveedores, PostgreSQL, Redis o NATS.

Ambos backends utilizan una única base física PostgreSQL; cada módulo es dueño exclusivo de su schema y migraciones, aplicadas coordinadamente. El BFF no accede a stores. gRPC es síncrono; NATS JetStream/Outbox es para eventos durables; Redis solo cache/estado efímero con TTL, nunca autoridad comercial.

## Calidad, errores y realtime

Cada fase/subfase empieza con matriz documentada: PostgreSQL/migraciones, ambas aplicaciones REST→BFF, BFF→módulo gRPC, gRPC a pasarelas/concesiones, Redis, NATS/Outbox, realtime/WebSocket y recuperación REST/cursor. Justificar previamente cualquier N/A y probar la matriz en la misma entrega.

Aplicar errores tipados y diagnosticables, autorización tenant en cada operación, redacción, eventos de inicio/dependencia/fallo/cierre con `correlation_id`, `trace_id`, categoría/código, duración propia y total. La aceptación verifica que fallo y cierre HTTP correlacionen y conserven el error, además del envelope seguro y ausencia de datos sensibles.

Todo cambio persistido visible sigue Outbox versionado → JetStream → realtime autenticado → TanStack Query → REST autorizado por cursor/snapshot, salvo justificación previa verificable. Redis/NATS se prueban con servicios reales aislados y recuperación tras reinicio cuando se usen.

## Contrato de pago normalizado

El envelope de webhook conserva un `payment_method` discriminado por `type` (`card`, `pse` o `crypto`) y `details` tipado para el método. `transaction.amount` es el importe facturado en unidades menores; el settlement cripto se expresa aparte con monto decimal string y moneda/activo, sin `float`. El mapeo proveedor→contrato normalizado pertenece al módulo de pagos, valida cada dato del evento verificado y no expone payloads crudos de proveedor. Campos concretos y evolución de esquema se versionan en OpenAPI/event contract.

## Calidad y cierre

Aplicar desde el día uno y en todos los módulos/crates, workers, APIs, frontends, infraestructura y fases/subfases de este backend todo requisito aplicable de:

- [ROADMAP.md](ROADMAP.md), para alcance funcional y criterios de aceptación.
- [Prompt maestro de calidad](PROMPT_MAESTRO_ENTERPRISE_QUALITY_ENGINEERING_PARA_PROYECTOS_RUST.md).
- [Prompt maestro de observabilidad](PROMPT_MAESTRO_IMPLEMENTACION_DE_OBSERVABILIDAD_TRAZABILIDAD_Y_MONITOREO_EMPRESARIAL_EN_RUST.md).
- [Reglas estrictas de arquitectura modular](reglas-estrictas-de-arquitectura-monolito-modular-rust-universal.md).
- [Reglas de arquitectura de Payvia](../ARCHITECTURE_RULES.md) e instrucciones superiores de [`AGENTS.md`](../AGENTS.md).
- Documentos equivalentes del [backend de pasarelas](../bacekend-pasarela/PROJECT_RULES.md), que deben permanecer sincronizados con los prompts y reglas normativas comunes.

Estas obligaciones aplican en cada módulo y cambio desde su diseño inicial; no se aplazan a una etapa posterior ni se consideran satisfechas por scaffolds o verificaciones no ejecutadas. Si un requisito no aplica, su justificación técnica concreta y verificable debe quedar documentada antes de implementar la fase/subfase. Una incompatibilidad real se explica y resuelve antes de continuar, sin omitir ni debilitar requisitos. No cerrar una fase por endpoint, tipo o prueba no ejecutada; reportar evidencia y bloqueos concretos.

## Límite de despliegue

Este backend es independiente del backend de pasarelas; ambos cumplen las reglas de monolito modular y los contratos cross-backend de `ARCHITECTURE_RULES.md`. Las ADRs de implementación definen mensajes, claves, límites, timeouts y migraciones sin modificar ownership de dominio.
