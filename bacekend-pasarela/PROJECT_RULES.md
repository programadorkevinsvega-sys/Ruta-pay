# Reglas específicas — Backend de Pasarelas Payvia

## Autoridad de dominio

Este backend es un monolito modular para integrar protocolos de proveedores. Cada proveedor es un módulo/crate independiente con ownership de adapter, capacidades realmente soportadas, contrato de credenciales, schema/migraciones no secretas, callbacks y pruebas de conformidad. El módulo `gateway_credentials` es el único dueño del almacenamiento cifrado y ciclo de vida de secretos de cuentas tenant.

No es dueño de tenants, identidades, API Keys de negocio, planes, precios, suscripciones, cuotas, entitlement, cobros que Payvia hace a tenants, payment intents, ledger ni estado comercial/transiciones de una transacción. Esos dominios y reglas pertenecen al backend de plataforma.

## Autorización emitida por plataforma

- Los comandos de tenant llegan de plataforma por gRPC autenticado; no se exponen operaciones de negocio directamente al frontend.
- Todo comando protegido trae una concesión firmada, breve, ligada a identidad de servicio, tenant, entorno, cuenta proveedor, scope/operación, capacidades, monto/moneda aplicable, `jti`, expiración e IDs de trazabilidad.
- Validar autenticidad, audiencia, expiración, uso/idempotencia y coincidencia exacta con comando antes de cualquier llamada al proveedor.
- Denegar de forma tipada si falta la concesión, está alterada, vencida, repetida o no coincide. No hay permiso implícito, bypass, ni cache local de plan/cuota.
- La cobranza de Payvia a sus tenants no pertenece a este backend: la ejecuta el adapter propio `platform_billing_wompi` de `bacekend-plataform`. Este backend solo procesa pagos de compradores para comercios y no recibe credenciales, grants ni callbacks de la cuenta Payvia.

## Seguridad entre servicios

- Exigir identidad de servicio validada criptográficamente y TLS/mTLS en ambos sentidos; nunca autorizar por IP o header autodeclarado.
- Rotar certificados y trust roots con solapamiento controlado; comprobar emisor, SAN/audience, expiración y revocación.
- Aplicar autorización por caller+método+scope y verificar cada claim firmado contra el request real. Cualquier inconsistencia produce fail-closed.
- Validar versión, tipos, longitudes, tamaño, campos permitidos, tenant, cuenta, ambiente, moneda/importe, deadline y clave idempotente antes de persistencia o llamada de proveedor.
- Aplicar rate limits, límites de concurrencia, deadlines/timeouts y retries acotados con backoff+jitter solo ante fallos clasificados como recuperables. Reconciliar resultados inciertos antes de repetir operaciones financieras.
- Usar gRPC para request/response inmediato y NATS JetStream/Outbox para hechos persistidos que requieren entrega durable/replay. gRPC no excluye persistencia local; eventos no transportan secretos ni grants completos.
- Propagar `correlation_id`/`trace_id`; registrar autorización, dependencia, fallo causal y resultado con códigos estables/duración y redacción. Nunca registrar credenciales, tokens, certificados privados, payload financiero crudo o PII innecesaria.
- Aceptación: caller denegado, certificado inválido/expirado, rotación, desconexión/timeout, replay, duplicado, rate limit y request inválido se verifican en ambos servicios, con recuperación sin duplicar efectos.

## Integraciones por proveedor

Cada módulo implementa solo contrato oficial verificado. No forzar algoritmo uniforme: preservar bytes/headers, validar firma, anti-replay, cuenta y ambiente según documentación vigente del proveedor antes de interpretar el evento. No inventar monedas, métodos, fees, capacidades o semántica de reintento.

Para sumar una pasarela, agregar su módulo/crate, schema y migraciones propias, adapter, descriptor de capacidades, verificador de callback, registro runtime y suite común de conformidad; validar en sandbox oficial. Una integración compatible no modifica adapters existentes ni añade `match` central por nombre.

El servicio puede efectuar validaciones técnicas, traducciones protocolarias e idempotencia del callback. No decide si una operación es comercialmente permitida ni actualiza por sí mismo el estado de negocio de pago. Las notificaciones verificados se guardan en Outbox y se publican por NATS JetStream; plataforma valida asociación/tenant y aplica las reglas de negocio.

## Fronteras, persistencia y eventos

Los frontends solo usan REST/OpenAPI/WebSocket de plataforma; nunca llaman a este backend directamente. La comunicación síncrona es gRPC autenticado con concesión. Resultados asíncronos se entregan con Outbox/JetStream y contratos versionados.

La comunicación plataforma→pasarelas usa identidad mTLS rotatoria, TLS, autenticación de servicio, autorización por método/scope y concesión firmada breve ligada a cada operación. Cada RPC valida versión, tenant, cuenta, ambiente, importe, tamaño/rango, deadline e idempotencia, con rate limit, timeout, retry/backoff acotado y fail-closed. gRPC sirve operaciones request/response aunque el módulo destino persista el resultado. Outbox/JetStream se reserva para hechos/workflows durables asíncronos; los eventos nunca contienen secretos ni concesiones completas.

Ambos backends utilizan una única base física PostgreSQL. Cada módulo de proveedor posee su propio schema y migraciones coordinadas; no lee ni escribe tablas de plataforma y no hace JOINs entre dominios. Redis es exclusivamente efímero con TTL, nunca fuente de autorización/idempotencia de negocio.

Todo cambio visible sincroniza por evento durable, realtime autenticado y recuperación REST autorizada; los eventos incluyen cursor/versión/audiencia y deduplicación. Redis Pub/Sub solo hace fan-out entre réplicas. Probar reconexión Redis y recuperación de consumer JetStream cuando se use.

## Errores, telemetría y pruebas

Modelar errores tipados por firma inválida, replay, capacidad no soportada, deny, expiración de concesión, conectividad/timeout, rate limit, persistencia y proveedor. Cada frontera registra logs JSON estructurados de inicio, dependencia, fallo clasificado y resultado con código/categoría, `correlation_id`, `trace_id`, `span_id` disponible, duración propia y total. Las métricas permiten p50/p95/p99, solicitudes por segundo, tasa de error, saturación y salud/duración de dependencias con cardinalidad acotada. Reutilizar [`payvia-observability`](../crates/observability/) del workspace; exportación Sentry/OTLP no debe bloquear solicitudes. Infraestructura, alertas y dashboards se versionan en `infra/`, sin fusionar el despliegue de aplicaciones. No registrar secretos, concesiones completas, headers privados ni payload sensible. Verificar propagación, redacción y degradación de extremo a extremo.

### Módulo `gateway_credentials`

- Recibe altas/rotaciones solo por comandos autenticados de plataforma; liga cada registro a tenant, proveedor, cuenta y ambiente `test`/`live`.
- Cifra secretos con cifrado autenticado y envelope encryption; las claves maestras viven en KMS/secret manager fuera de PostgreSQL. Versiona claves, audita cambios sin incluir valores y permite rotación/revocación con solapamiento controlado.
- Persiste ciphertext y metadata estrictamente necesaria; usa mappers/tipos que no implementan `Debug` con material secreto. Minimiza tiempo en memoria y limpia buffers cuando sea posible.
- Solo el adapter del proveedor correspondiente puede solicitar el secreto requerido para una operación con scope vigente; la respuesta se mantiene en memoria, nunca se devuelve al frontend y nunca pasa por NATS/Outbox.
- Los eventos opcionales de cambio publican únicamente identificadores opacos, tenant, provider, environment, versión y estado (sin secret, ciphertext, longitud ni huella reutilizable).
- Los errores para el usuario confirman resultado/estado sin revelar si un secreto específico existe, su contenido o detalles criptográficos.
- El transporte de configuración desde plataforma usa mTLS y un scope de escritura explícito; el vault valida que tenant, provider, account y ambiente coincidan con la concesión firmada.

Por fase/subfase, documentar y probar la matriz PostgreSQL/migración, REST/BFF, gRPC, autorización concesión, Redis, JetStream/Outbox, realtime/recuperación, errores y sandbox. Probar integraciones reales contra servicios aislados y sandbox oficial; no usar mocks/stubs/fixtures para declarar aceptación. Reinicios solo sobre infraestructura de desarrollo identificada previamente. No declarar cerrada una fase con bloqueo o verificación no ejecutada.

## Documentos normativos y bloqueo

Aplicar desde el día uno y en todos los módulos/crates, workers, APIs, infraestructura y fases/subfases de este backend todo requisito aplicable de:

- [ROADMAP.md](ROADMAP.md), para alcance funcional y criterios de aceptación.
- [Prompt maestro de calidad](PROMPT_MAESTRO_ENTERPRISE_QUALITY_ENGINEERING_PARA_PROYECTOS_RUST.md).
- [Prompt maestro de observabilidad](PROMPT_MAESTRO_IMPLEMENTACION_DE_OBSERVABILIDAD_TRAZABILIDAD_Y_MONITOREO_EMPRESARIAL_EN_RUST.md).
- [Reglas estrictas de arquitectura modular](reglas-estrictas-de-arquitectura-monolito-modular-rust-universal.md).
- [Reglas de arquitectura de Payvia](../ARCHITECTURE_RULES.md) e instrucciones superiores de [`AGENTS.md`](../AGENTS.md).
- Documentos equivalentes del [backend de plataforma](../bacekend-plataform/PROJECT_RULES.md), que deben permanecer sincronizados con los prompts y reglas normativas comunes.

Estas obligaciones aplican en cada módulo y cambio desde su diseño inicial; no se aplazan a una etapa posterior ni se consideran satisfechas por scaffolds o verificaciones no ejecutadas. Si un requisito no aplica, su justificación técnica concreta y verificable debe quedar documentada antes de implementar la fase/subfase. Una incompatibilidad real se explica y resuelve antes de continuar, sin omitir ni debilitar requisitos. La topología de dos backends y el ownership de realtime en plataforma están establecidos en `ARCHITECTURE_RULES.md`; las ADRs detallan el contrato y la operación sin cambiar esa decisión.

## Higiene del repositorio y README principal

No crear archivos, carpetas o documentos Markdown innecesarios, duplicados o sin propósito verificable para este backend. Actualizar primero la documentación existente y mantener cada detalle en el documento dueño. El README raíz debe conservar únicamente la orientación general necesaria de Payvia —propósito, estado real, arquitectura resumida, instrucciones verificadas y enlaces—; los detalles internos de pasarelas, contratos y fases permanecen en los documentos especializados.

## Ramas, verificación e integración

`main` es exclusivamente la rama principal de integración; debe mantenerse funcional y verificable y no es una rama de trabajo. Todo cambio de código, y todo cambio importante de documentación, se desarrolla en una rama descriptiva. El flujo obligatorio es completar el cambio en la rama, ejecutar las pruebas y verificaciones aplicables, comprobar todos los criterios de aceptación, documentar resultados y riesgos, abrir el Pull Request y esperar verificaciones exitosas y aprobación del usuario antes de integrarlo en `main`. No se presenta como terminado un cambio con criterios pendientes ni se integra si fallan verificaciones obligatorias. Solo las correcciones editoriales triviales de documentación pueden confirmarse directamente en `main` cuando las protecciones lo permitan; si existe duda sobre el alcance, se usa una rama. El agente no integra Pull Requests automáticamente. No hagas force push ni reescribas historial compartido. Mantén commits pequeños y coherentes, revisa diffs y secretos; los cambios sensibles de cifrado/autenticación/autorización requieren la revisión exigida. Conserva historial auditable y reversión; no añadas proceso innecesario a cambios pequeños.
