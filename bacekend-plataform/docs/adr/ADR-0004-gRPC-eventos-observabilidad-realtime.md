# ADR-0004: RPC síncrono, eventos durables, contexto y realtime

- Estado: Aceptada
- Fecha: 2026-10-22

## Decisión

gRPC versionado (`tonic`/Protobuf) atiende consultas/comandos que requieren respuesta inmediata y siempre se dirige al módulo dueño. NATS JetStream con Outbox/consumidor durable distribuye hechos ya persistidos, trabajos que requieren replay o efectos desacoplados; no se usa para consultas ni como etiqueta genérica de operación delicada. Cada consumidor es idempotente por `event_id` y confirma después de persistir su efecto. No se promete exactly-once de extremo a extremo.

Cada cambio visible se persiste junto con Outbox en transacción local. El evento versionado contiene identidad y cursor durable, versión de agregado, scope/audiencia y contexto de trazabilidad mínimo; excluye secretos, grants completos y contenido no requerido. El módulo realtime de plataforma mantiene el journal/cursor autorizado y distribuye por gRPC al BFF. El BFF autentica/bridgea WebSocket; TanStack Query deduplica/ordena y recupera huecos mediante REST autorizado por cursor/snapshot. Redis Pub/Sub se limita al fan-out efímero entre réplicas y no es fuente durable.

Observabilidad usa eventos JSON estructurados y spans correlacionados, W3C Trace Context, OpenTelemetry, métricas y Sentry donde aplique. Cada frontera registra inicio, llamada a dependencia, fallo clasificado y cierre, con clase/código estable, componente, operación, dependencia, duración propia y total, `correlation_id`/`trace_id`, y detalles seguros. La telemetría redacta secretos y degrada sin derribar negocio.

## Consecuencias

- Un cambio no visible o fase sin mutaciones puede justificar no emitir realtime; debe dejar razón previa en su matriz.
- Redis y NATS solo se incorporan si el caso de uso existe; al usarse, reconexión y recuperación son criterios de aceptación inmediatos.
- Contratos de error se mapean módulo→gRPC→BFF sin perder código/categoría ni filtrar datos internos.
