# Auditoría de pruebas — Fase 1: runtime y observabilidad

## Alcance

Esta auditoría cubre exclusivamente el runtime fundacional: configuración, validación de contexto HTTP, readiness, transporte HTTP→BFF→gRPC→módulo, deadlines/desconexión y observabilidad. No atribuye a Fase 1 casos de identidad, tenants o pagos.

## Evidencia ejecutable encontrada en main

- `tests/http_grpc_acceptance.rs` ejecuta un recorrido de aceptación HTTP→BFF→gRPC→módulo y comprueba éxito, módulo desconectado y timeout de transporte.
- El recorrido comprueba envelopes de error, retryability, correlación entre eventos de fallo y cierre HTTP, y redacción.
- Hay pruebas unitarias de configuración y validación de correlation ID.
- La CI Rust ejecuta `cargo fmt`, Clippy y `cargo test --workspace`; usa PostgreSQL, Redis y JetStream autenticados para los tests que los requieren.
- El test de aceptación HTTP/gRPC es un test de integración con fronteras reales, no un browser E2E de producto.

## Casos y estado

| Caso | Tipo esperado | Evidencia / estado |
|---|---|---|
| Correlation ID válido | Unitario | Cubierto |
| Correlation ID vacío | Unitario | Cubierto |
| Correlation ID >128 bytes | Unitario | Añadido en este PR |
| Caracteres no ASCII/control/separadores no permitidos | Unitario | Añadido en este PR |
| Runtime probe exitoso | Integración | Cubierto por `http_grpc_acceptance` |
| Módulo gRPC desconectado | Integración | Cubierto |
| Timeout de transporte | Integración | Cubierto |
| Error correlacionado con cierre HTTP | Integración | Cubierto |
| Configuración válida/inválida | Unitario | Hay cobertura; falta inventario campo por campo |
| Readiness con cada dependencia crítica degradada | Integración | No demostrado para una matriz exhaustiva |
| Caída de exportador/Collector y recuperación | Integración/operación | Evidencia histórica en acta; no se demuestra por la suite de CI actual |
| Matriz de límites de cada variable de configuración | Unitario | Pendiente de inventario completo |
| Build y browser E2E frontend | Build/E2E | No aplican a la superficie entregada en Fase 1; frontend real entra en Fase 2 |

## Inventario de configuración y escenarios

El inventario estático de `AppConfig::from_env` identifica estas variables y reglas:

| Variable | Regla implementada | Escenarios automatizados / pendientes |
|---|---|---|
| `PLATFORM_DATABASE_URL` | Obligatoria; URL PostgreSQL con host, usuario y password; bloquea parámetros `password`/`sslpassword` y valores de query >32 caracteres | Positivo PostgreSQL; negativos de esquema, host/usuario/password ausentes, query sensible y longitud |
| `PLATFORM_REDIS_URL` | Obligatoria; `redis`/`rediss`, host y password; sin query/fragment | Positivos ambos esquemas; negativos password ausente, esquema, query y fragment |
| `PLATFORM_NATS_URL` | Opcional con default local; `nats`/`tls`, host, sin credenciales/query/fragment | Positivos ambos esquemas; negativos de esquema, credenciales embebidas, query y fragment |
| `NATS_USER` | Obligatoria, no vacía/solo espacios | Validación vacía/espacios añadida |
| `NATS_PASSWORD` | Obligatoria; mínimo 32 caracteres | 31 rechazado, 32 aceptado |
| `APP_HTTP_ADDR` | Default `0.0.0.0:8080`; debe parsear como SocketAddr | Dirección inválida con error estable; límites de puerto/rango siguen pendientes |
| `APP_MODULE_GRPC_ADDR` | Default loopback; debe ser loopback | Listener remoto rechazado; loopback aceptado |
| `APP_METRICS_ADDR` | Default loopback; listener remoto exige token | Loopback sin token; remoto sin token/31 caracteres rechazado y 32 aceptado |
| `APP_METRICS_BEARER_TOKEN` | Opcional; requerido con longitud mínima 32 para métricas remotas | Frontera 31/32 añadida |
| `APP_ENV` | Default `development`; allowlist development/test/staging/production | Los cuatro aceptados; vacío, alias, case incorrecto y desconocido rechazados |
| `APP_VERSION` | Default versión Cargo | No tiene validación semántica implementada; pendiente decidir si debe limitarse |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Opcional; HTTP/HTTPS, host, sin credenciales/query/fragment | Positivos HTTP/HTTPS y negativos de esquema, credenciales, query y fragment |
| `SENTRY_DSN` | Opcional; parseo mediante tipo DSN de Sentry | La validación del parser existe; matriz unitaria de DSN válido/malformado pendiente |

Se agregan pruebas unitarias puras en `platform/src/config/app_config.rs` para las reglas de entorno, credenciales NATS, límites de token de métricas, URL PostgreSQL/Redis/NATS y endpoint OTLP. Los tests nuevos usan configuración aislada y no dependen de modificar variables globales de entorno.

## Cambios de prueba en este PR

Se amplía `CorrelationId` con pruebas explícitas de longitud mínima/máxima, longitud excedida, valor vacío, espacios, controles, separadores, Unicode y alfabeto permitido. Estos tests se ejecutan automáticamente mediante `cargo test --workspace`.

## Huecos pendientes y criterio de cierre de auditoría

1. Completar límites de puerto/rango para `SocketAddr`, matriz de DSN Sentry y semántica de `APP_VERSION`; revisar si la longitud de password NATS debe medirse en bytes o caracteres.
2. Determinar formalmente qué dependencias son críticas para readiness y probar cada combinación relevante de estados.
3. Convertir las pruebas históricas de degradación del Collector y shutdown en verificaciones repetibles en CI o en una suite de aceptación controlada.
4. Mantener el test de integración HTTP→BFF→gRPC como requisito obligatorio de CI.
5. Registrar commit, comandos, ejecución y resultado real para cada aceptación.

No se declara cobertura funcional del 100 %. La Fase 1 conserva su alcance implementado, pero los huecos enumerados siguen abiertos como trabajo de calidad de esta fase; no se posponen a Fase 8.
