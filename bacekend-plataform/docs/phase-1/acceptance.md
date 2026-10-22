# Fase 1 — Fundaciones y observabilidad

## Alcance y estado

La Fase 1 implementa la base ejecutable de plataforma: composition root, configuración tipada, BFF Axum/OpenAPI, contrato Protobuf/Tonic y módulo `platform_runtime` con límites de dominio, aplicación e infraestructura; readiness atraviesa HTTP→BFF→gRPC→módulo. También instrumenta correlación, métricas Prometheus, exportación OTLP, apagado ordenado y diagnóstico seguro. No agrega reglas comerciales, tenants, autenticación de usuario, mutaciones persistidas ni frontends funcionales.

**Estado: cerrada — 2026-10-22, tras revalidar configuración real.** La revisión detectó que `bacekend-plataform/.env` carecía de variables de runtime y que la evidencia anterior había usado variables exportadas desde la terminal. Se añadieron las variables faltantes al `.env` local ignorado por Git y se volvió a ejecutar `run-production.sh` sin overrides. Readiness y petición HTTP respondieron 200, el endpoint de métricas rechazó acceso sin token (401) y permitió el scrape autenticado (200), Prometheus reportó el target de plataforma `up=1`, Loki recibió logs estructurados correlacionados y Tempo devolvió tres spans para la traza de aceptación. Sentry Cloud ya había aceptado una prueba de ingestión HTTP 200; la SDK del backend se inicializa con el DSN del mismo `.env` y aplica el saneador probado unitariamente.

Las decisiones de arquitectura viven en [`../adr/`](../adr/), incluida [ADR-0007: destino Sentry](../adr/ADR-0007-destino-sentry.md) y [ADR-0008: autenticación de interfaces locales de infraestructura](../adr/ADR-0008-autenticacion-infraestructura-local.md). El alcance funcional y los gates completos continúan en el [roadmap](../../ROADMAP.md).

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
| Sentry Cloud | Se usa el proyecto Cloud del grupo; el DSN es secreto del backend local/de despliegue. | El endpoint Cloud aceptó una prueba controlada (HTTP 200); el payload era mínimo y no contenía datos privados. El binario release también emitió su evento de fallo de bind con el DSN configurado. La prueba unitaria verifica el saneador antes del envío. |
| Despliegue | Backend de plataforma tiene su Dockerfile, `.env` ignorado y scripts de build/run independientes; no existe `.env` en la raíz. | `build-production.sh` compila solo `bacekend-plataform`; `run-production.sh` carga `bacekend-plataform/.env` y ejecuta el binario `target/release`. La infraestructura local usa ese mismo archivo de forma explícita con `docker compose --env-file bacekend-plataform/.env`. No despliega pasarela ni frontends. |
| Interfaces administrativas de infraestructura local | pgAdmin 4 y RedisInsight para los almacenes de desarrollo; Prometheus protegido con Basic Auth; Loki y Tempo accesibles por proxy autenticado; NATS Surveyor interno como exporter (sin UI propia). Grafana conserva su login y datasource Prometheus autenticado. | Se valida rechazo anónimo y acceso autenticado en Prometheus, proxy de Loki/Tempo/RedisInsight; readiness de Loki/Tempo; UI HTTP de pgAdmin/Grafana; health del datasource de Prometheus y target `nats-surveyor` en estado `up`. Los usuarios y contraseñas solo viven en el `.env` ignorado y archivos locales generados. Diseño y límites en ADR-0008. |

### Revalidación de configuración de runtime

`bacekend-plataform/.env` define `APP_HTTP_ADDR`, `APP_MODULE_GRPC_ADDR`, `APP_METRICS_ADDR`, `APP_METRICS_BEARER_TOKEN`, `APP_ENV`, `OTEL_EXPORTER_OTLP_ENDPOINT`, `RUST_LOG` y `SENTRY_DSN`. El bearer de métricas de la app coincide con `PLATFORM_METRICS_TOKEN`, del que se aprovisiona el secreto del scrape Prometheus. Prometheus, Loki y Tempo son destinos de la infraestructura; no requieren credenciales de conexión directas en la app porque el Collector enruta la telemetría. La app exporta OTLP al Collector; el Collector envía trazas a Tempo, logs a Loki y métricas al endpoint scrapeado por Prometheus. No se copian valores secretos a la documentación ni al repositorio.

## Evidencia de aceptación ejecutada — 2026-10-22

- `cargo fmt --all -- --check`: pasó.
- `cargo clippy --workspace --all-targets -- -D warnings`: pasó.
- `cargo test --workspace --all-targets`: pasó; incluye aceptación HTTP→BFF→gRPC→módulo y pruebas de configuración/observabilidad.
- Inventario de pruebas del workspace: 12 pruebas unitarias y 1 prueba automatizada de integración/aceptación HTTP→BFF→gRPC→módulo. No existe una suite separada de browser E2E ni frontend en esta fase; la interfaz de producto no forma parte del alcance de Fase 1. La aceptación de backend sí cruza HTTP y gRPC reales.
- `docker build --file bacekend-plataform/deploy/Dockerfile --tag payvia-platform:phase1-check .`: pasó; construyó el binario del backend de plataforma en imagen de runtime independiente.
- `docker compose config --quiet`: pasó.
- `docker compose exec -T prometheus promtool check config /etc/prometheus/prometheus.yml`: pasó.
- `docker compose exec -T prometheus promtool check rules /etc/prometheus/alerts.yml`: pasó; encontró 11 reglas.
- Prueba inicial de runtime: health devolvió HTTP 200; propagó correlation y trace; Prometheus scrape `up=1` y consultó histograma; Loki recibió eventos correlacionados; Tempo devolvió spans del trace. Canary sensible no apareció en Loki. Esa prueba usó variables exportadas y por sí sola no validaba la carga del `.env`.
- Degradación Collector: con Collector detenido, health siguió devolviendo 200; después de reiniciarlo, Loki recibió telemetría nueva y Tempo una nueva tanda.
- Apagado: SIGTERM al binario real terminó limpiamente y emitió eventos correlacionados de apagado.
- Configuración de Compose y `git diff --check`: pasaron.
- `bacekend-plataform/build-production.sh`: compiló el binario release del backend de plataforma.
- `bacekend-plataform/run-production.sh`: ejecutó el binario ya compilado, cargó el `.env` propio del backend y generó `service.startup.bind.failed` ante un puerto local ocupado; el DSN no apareció en la salida y el proceso terminó, permitiendo al guard de Sentry vaciar su cola. La recepción específica de ese evento SDK no se identifica desde la salida del proceso; la ingestión Cloud se comprobó con la prueba controlada registrada abajo.
- `infra/prepare-dev-secrets.sh`: validó y preparó los secretos de infraestructura conservando byte por byte `bacekend-plataform/.env`; Compose validó la configuración con `--env-file bacekend-plataform/.env`. No existe `.env` en la raíz.
- Los dos scripts del backend y el script de preparación pasaron `bash -n`; `cargo fmt`, Clippy, todas las pruebas del workspace, la compilación release, Compose config y `git diff --check` pasaron.
- El stack compartido quedó iniciado con `docker compose --env-file bacekend-plataform/.env up -d`. PostgreSQL y Redis reportaron `healthy`; Prometheus, Grafana, Loki y Tempo respondieron HTTP 200. Loki puede devolver 503 durante su ventana de calentamiento de 15 segundos tras el inicio; al terminarla respondió 200.
- Prueba adicional del binario release: readiness HTTP→gRPC devolvió 200 y propagó el `correlation_id` y `trace_id` suministrados; el endpoint de métricas devolvió 401 sin bearer y 200 con el token; Prometheus marcó `up{job="payvia-platform"}=1`.
- Loki encontró seis eventos con el `correlation_id` de aceptación; Tempo devolvió tres spans para el `trace_id`. La consulta de series de Loki mostró únicamente `service_name` como etiqueta indexada; IDs de correlación/traza permanecen disponibles como metadatos para diagnóstico sin crear series de alta cardinalidad.
- SIGTERM al binario release terminó con código 0.
- Sentry Cloud aceptó un evento mínimo de aceptación por HTTPS, con respuesta HTTP 200 e ID `23352e0de5d3a7dc38ffec579538449b`; el código del proyecto valida el DSN y la prueba del saneador cubre campos sensibles.
- Revalidación con el `.env` del backend: `run-production.sh` arrancó el binario release sin variables manuales adicionales; readiness/request HTTP `200`, `/metrics` sin bearer `401` y con bearer `200`, target `payvia-platform` en Prometheus `up=1`, consulta de Loki `200` con entradas correlacionadas y consulta de Tempo `200` con tres spans. El proceso se detuvo con SIGTERM al terminar la comprobación.
- Verificación automatizada repetida durante esta revalidación: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo test --workspace --all-targets`: pasaron; el conjunto ejecutó 13 pruebas (12 unitarias y 1 integración/aceptación).

### Ampliación de interfaces administrativas locales — 2026-10-22

Esta ampliación cubre el acceso administrativo a la pila local de desarrollo y no añade persistencia ni comportamiento de dominio al backend. Se documenta aquí porque el stack compartido respalda la observabilidad y verificación de la Fase 1.

- `docker compose --env-file bacekend-plataform/.env config --quiet`: pasó tras añadir los servicios y la autenticación.
- `bash -n infra/prepare-dev-secrets.sh` y `git diff --check`: pasaron.
- `docker compose --env-file bacekend-plataform/.env up -d` dejó los servicios de desarrollo en ejecución; PostgreSQL y Redis quedaron `healthy`. No se borraron volúmenes.
- Prometheus rechazó una petición sin credenciales (`401`) y aceptó la autenticada (`200`); `promtool check config` y `promtool check web-config` pasaron.
- El proxy rechazó solicitudes anónimas (`401`) y admitió solicitudes autenticadas (`200`) en Loki, Tempo y RedisInsight. Loki y Tempo devolvieron `200` en `/ready`.
- pgAdmin sirvió su formulario de login (`200`); Grafana sirvió su endpoint de salud (`200`) y su datasource Prometheus reportó `OK`.
- Prometheus reportó `nats-surveyor` como `up`. El exporter no publica una consola web: sus métricas se consultan por Grafana.
- Las raíces `/` de Loki y Tempo ahora muestran una página informativa protegida. Antes el `404` era el comportamiento de una API sin página raíz; `/ready` es la prueba de disponibilidad.
- La contraseña HTTP de scrape de NATS Surveyor se rotó después de aparecer en la salida de un comando auxiliar; los servicios se recrearon con el valor actualizado. No se registran valores de credenciales en esta evidencia.
- Esta aceptación no valida login interactivo de usuario en navegador, despliegue productivo ni recepción OTLP después de reinicios destructivos. Las interfaces se enlazan a loopback y la configuración se limita a desarrollo local.

## Resultado del cierre

La Fase 1 queda cerrada para su alcance documentado. PostgreSQL, Redis, NATS JetStream/Outbox, autorización de tenant y realtime no aplican porque esta fase no persiste ni muta estado de negocio; sus justificaciones están en la matriz de integración. Sus pruebas e integraciones pertenecen a las fases que introducen esos dominios. La suite de esta fase no es una suite E2E del navegador ni cubre fallos/reconexión de servicios de negocio.
