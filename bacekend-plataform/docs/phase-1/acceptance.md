# Fase 1 — Fundaciones y observabilidad

## Alcance y estado

La Fase 1 implementa la base ejecutable de plataforma: composition root, configuración tipada, BFF Axum/OpenAPI, contrato Protobuf/Tonic y módulo `platform_runtime` con límites de dominio, aplicación e infraestructura; readiness atraviesa HTTP→BFF→gRPC→módulo. También instrumenta correlación, métricas Prometheus, exportación OTLP, apagado ordenado y diagnóstico seguro. No agrega reglas comerciales, tenants, autenticación de usuario, mutaciones persistidas ni frontends funcionales.

**Estado: en validación, no cerrada.** La recepción en Loki, Tempo y Prometheus está comprobada en infraestructura local. El binario real acepta el DSN de Sentry y generó el evento de prueba de inicio; la confirmación de recepción y redacción en el proyecto Sentry Cloud todavía requiere inspección del evento en Sentry. No declarar la fase completa hasta cubrir ese criterio.

Las decisiones de arquitectura viven en [`../adr/`](../adr/), incluida [ADR-0007: destino Sentry](../adr/ADR-0007-destino-sentry.md). El alcance funcional y los gates completos continúan en el [roadmap](../../ROADMAP.md).

## Matriz de integración

| Frontera | Aplicación en Fase 1 | Verificación y razón de no aplicabilidad |
|---|---|---|
| PostgreSQL / migraciones | No se persiste estado de negocio. | Health/readiness no consulta PostgreSQL; no se agregan schemas o tablas sin ownership funcional. |
| Frontend → REST/OpenAPI → BFF | Endpoints operacionales `/health/live`, `/health/ready` y `/openapi.json`; no hay frontend ejecutable. | Los endpoints no son UX tenant/admin. Se acepta el BFF con cliente HTTP real y contrato OpenAPI; no se crea UI de salud. |
| BFF → módulo gRPC | Axum llama a `platform_runtime` por contrato Protobuf/Tonic; el probe de inicio también propaga contexto. | Aceptación real HTTP→Axum→Tonic client/server→módulo; comprobar trace/correlation, éxito, error, timeout y envelope seguro. |
| Redis | No se requiere caché ni estado efímero en health/readiness. | No conectar ni reiniciar Redis como condición artificial de esta fase; primera subfase que lo use implementará `ConnectionManager`, backoff y Pub/Sub si aplica. |
| JetStream / Outbox | No hay mutaciones ni efectos durables de dominio. | No emitir eventos de health. Outbox/consumer durable aplican en la primera mutación persistida y visible. |
| Realtime | Health es lectura operacional puntual, sin audiencia ni transición persistida. | No corresponde emitir realtime; se documenta para evitar agregar Redis Pub/Sub como autoridad. |
| Errores y observabilidad | Eventos estructurados de inicio, dependencia, fallo y cierre; OTLP, Sentry opcional, métricas Prometheus privadas y redacción. | Aceptar campos correlation/trace, error estable, categoría/código, duraciones y ausencia de canario sensible en los sinks. |
| Sentry Cloud | Se usa el proyecto Cloud del grupo; el DSN es secreto del backend local/de despliegue. | El proceso real validó la configuración y emitió un evento de inicio de prueba; pendiente comprobar su recepción y redacción en el proyecto. No se sustituye por mocks. |
| Despliegue | Backend de plataforma tiene su Dockerfile, `.env` ignorado y scripts de build/run independientes; no existe `.env` en la raíz. | `build-production.sh` compila solo `bacekend-plataform`; `run-production.sh` carga `bacekend-plataform/.env` y ejecuta el binario `target/release`. La infraestructura local usa ese mismo archivo de forma explícita con `docker compose --env-file bacekend-plataform/.env`. No despliega pasarela ni frontends. |

## Evidencia de aceptación ejecutada — 2026-10-22

- `cargo fmt --all -- --check`: pasó.
- `cargo clippy --workspace --all-targets -- -D warnings`: pasó.
- `cargo test --workspace --all-targets`: pasó; incluye aceptación HTTP→BFF→gRPC→módulo y pruebas de configuración/observabilidad.
- `docker build --file bacekend-plataform/deploy/Dockerfile --tag payvia-platform:phase1-check .`: pasó; construyó el binario del backend de plataforma en imagen de runtime independiente.
- `docker compose config --quiet`: pasó.
- `docker compose exec -T prometheus promtool check config /etc/prometheus/prometheus.yml`: pasó.
- `docker compose exec -T prometheus promtool check rules /etc/prometheus/alerts.yml`: pasó; encontró 11 reglas.
- Prueba de runtime: health devolvió HTTP 200; propagó correlation y trace; Prometheus scrape `up=1` y consultó histograma; Loki recibió eventos correlacionados; Tempo devolvió spans del trace. Canary sensible no apareció en Loki.
- Degradación Collector: con Collector detenido, health siguió devolviendo 200; después de reiniciarlo, Loki recibió telemetría nueva y Tempo una nueva tanda.
- Apagado: SIGTERM al binario real terminó limpiamente y emitió eventos correlacionados de apagado.
- Configuración de Compose y `git diff --check`: pasaron.
- `bacekend-plataform/build-production.sh`: compiló el binario release del backend de plataforma.
- `bacekend-plataform/run-production.sh`: ejecutó el binario ya compilado, cargó el `.env` propio del backend y generó `service.startup.bind.failed` ante un puerto local ocupado; el DSN no apareció en la salida y el proceso terminó, permitiendo al guard de Sentry vaciar su cola. La aceptación del evento en Sentry Cloud no se puede confirmar desde el proceso local.
- `infra/prepare-dev-secrets.sh`: validó y preparó los secretos de infraestructura conservando byte por byte `bacekend-plataform/.env`; Compose validó la configuración con `--env-file bacekend-plataform/.env`. No existe `.env` en la raíz.
- Los dos scripts del backend y el script de preparación pasaron `bash -n`; `cargo fmt`, Clippy, todas las pruebas del workspace, la compilación release, Compose config y `git diff --check` pasaron.
- El stack compartido de desarrollo quedó iniciado de nuevo después de las pruebas; PostgreSQL y Redis aparecen saludables.

## Bloqueos y próximos criterios de cierre

1. En Sentry, verifica que el evento de prueba se recibió en el proyecto de plataforma y que no contiene mensaje, detalles de error, datos de request ni valores sensibles. El saneamiento elimina deliberadamente tags y campos de contexto no trazables; no se debe exigir un tag `service` que el evento no conserva.
2. Mantener fuera de la fase los errores de PostgreSQL, auth de tenant, conflictos de dominio, Redis, JetStream y proveedores: no existen operaciones que los invoquen; cada fase de dominio debe documentar y aceptar sus errores aplicables antes de implementar.
