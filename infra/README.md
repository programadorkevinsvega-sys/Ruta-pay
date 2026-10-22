# Infraestructura compartida de desarrollo

El `compose.yaml` raíz provisiona PostgreSQL, Redis, NATS JetStream y la pila compartida de observabilidad para desarrollo local. No construye ni ejecuta `bacekend-plataform`, `bacekend-pasarela` ni los frontends: cada aplicación conserva su binario/imagen, Dockerfile, configuración, secretos y despliegue de VM/VPS independiente.

La configuración está fijada a tags de imagen versionados. El archivo de preparación genera credenciales aleatorias locales, no las imprime y no sobrescribe `bacekend-plataform/.env`; si el archivo ya existe y contiene los valores generados, puede reconstruir los archivos de servicio. La carpeta `infra/.local/` está ignorada por Git y mantiene los secretos locales con permisos restrictivos. También está excluida del contexto Docker para que nunca entre en imágenes de aplicación. No se crea ni se usa un `.env` en la raíz.

## Servicios locales

| Servicio | Dirección local | Persistencia |
|---|---|---|
| PostgreSQL | `127.0.0.1:5432` | volumen `postgres_data` |
| Redis con AOF y contraseña | `127.0.0.1:6379` | volumen `redis_data` |
| NATS con JetStream y autenticación | `127.0.0.1:4222` | volumen `nats_data` |
| pgAdmin 4 | `127.0.0.1:5050` | volumen `pgadmin_data`; login pgAdmin |
| RedisInsight | `127.0.0.1:5540` | volumen `redisinsight_data`; Basic Auth en proxy y conexión Redis preconfigurada |
| Node Exporter | solo red interna Compose | métricas del host local de desarrollo |
| Prometheus | `127.0.0.1:9091` | Basic Auth, volumen `prometheus_data`, 15 días |
| Grafana | `127.0.0.1:3000` | volumen `grafana_data` |
| OTLP gRPC/HTTP Collector | `127.0.0.1:4317` / `127.0.0.1:4318` | imagen oficial Contrib 0.162.0 fijada en GHCR; exporta a Tempo, Loki y Prometheus |
| Tempo | `127.0.0.1:3200` detrás de proxy Basic Auth | volumen `tempo_data`, 7 días |
| Loki | `127.0.0.1:3100` detrás de proxy Basic Auth | volumen `loki_data`, 7 días |
| Exporters de PostgreSQL, Redis, NATS y NATS Surveyor | solo red interna Compose; scrape autenticado cuando corresponde | sin estado propio |

Las credenciales locales están en `bacekend-plataform/.env`, ignorado por Git, y los hashes/archivos de servicio en `infra/.local/secrets/`. `docker compose --env-file bacekend-plataform/.env` usa ese archivo para interpolar el stack local; no se crea un `.env` raíz. Cada backend conserva su `.env` independiente, aunque repita credenciales compartidas de PostgreSQL, Redis o NATS.

| Interfaz | Usuario | Contraseña | Autenticación |
|---|---|---|---|
| Grafana | `GRAFANA_ADMIN_USER` | `GRAFANA_ADMIN_PASSWORD` | Nativa Grafana |
| pgAdmin 4 | `PGADMIN_DEFAULT_EMAIL` | `PGADMIN_DEFAULT_PASSWORD` | Nativa pgAdmin; se usa el correo como usuario |
| Prometheus | `PROMETHEUS_ADMIN_USER` | `PROMETHEUS_ADMIN_PASSWORD` | Basic Auth nativa; hash bcrypt local |
| Loki, Tempo y RedisInsight | `INFRA_UI_USER` | `INFRA_UI_PASSWORD` | Basic Auth del proxy local |
| NATS Surveyor | `NATS_SURVEYOR_USER` / `NATS_SURVEYOR_HTTP_USER` | `NATS_SURVEYOR_PASSWORD` / `NATS_SURVEYOR_HTTP_PASSWORD` | Credencial NATS de sistema y Basic Auth del endpoint de métricas |

Los valores secretos se generan y consultan localmente en `bacekend-plataform/.env`; no se imprimen en esta guía ni se versionan. NATS Surveyor es un exporter de métricas, no un panel web: sus datos se consultan en Grafana con el usuario/contraseña de Grafana. RedisInsight no tiene una cuenta local configurable equivalente a pgAdmin; su interfaz queda detrás del proxy autenticado y su conexión a Redis viene preconfigurada con el usuario `default` y la contraseña de Redis.

Los paneles de Grafana cubren aplicaciones (RPS, p50/p95/p99, 5xx, solicitudes en curso), PostgreSQL, dependencias compartidas (Redis/NATS), recursos del host y pipeline de observabilidad (scrapes, logs y trazas). Las reglas Prometheus cubren disponibilidad, proporción de errores, latencia HTTP p95, saturación HTTP, recursos del host y disponibilidad de Collector/Loki/Tempo. `platform.yml` apunta por defecto al listener nativo local en `host.docker.internal:9090`; para usarlo, enlaza el listener a `0.0.0.0:9090`, configura el token local y limita el acceso al host de desarrollo. `pasarela.yml` queda sin target hasta que su operador configure una dirección local/privada.

Prometheus autentica su interfaz web con Basic Auth. Loki y Tempo no implementan autenticación propia: sus puertos internos no se publican y el host solo llega a ellos por un proxy Basic Auth en `127.0.0.1`. RedisInsight usa el mismo proxy. Los servicios permanecen en redes privadas de Compose y ningún puerto de administración se publica fuera de loopback.

Loki y Tempo son APIs, no consolas web. Por eso `/` podía devolver `404 page not found`; eso no significa que la ingesta esté caída. Usa `/ready` para comprobar disponibilidad y Grafana (`http://localhost:3000`) para explorar logs y trazas. La raíz local muestra una página explicativa y enlaza a Grafana; el proxy solicita autenticación antes de servirla.

## Preparar y validar

Requisitos: Docker Engine con el plugin Compose, Bash y OpenSSL. `infra/prepare-dev-secrets.sh` usa Docker para generar hashes bcrypt sin imprimir contraseñas. Para el backend Rust: Rust/Cargo del toolchain fijado por el repositorio.

```bash
./infra/prepare-dev-secrets.sh
docker compose --env-file bacekend-plataform/.env config --quiet
docker compose --env-file bacekend-plataform/.env up -d
```

El último comando inicia servicios persistentes de desarrollo. No se ejecuta automáticamente como parte de los cambios de código. Para detener servicios sin borrar datos:

```bash
docker compose --env-file bacekend-plataform/.env down
```

No uses `docker compose down -v` salvo que quieras borrar todos los datos locales. Las pruebas de reinicio deben documentar antes el servicio afectado y solo apuntar a esta composición de desarrollo, nunca a producción.

El script genera usuarios/contraseñas administrativas, credenciales de NATS Surveyor, hashes para Prometheus/proxy y los secretos ya existentes. `PLATFORM_METRICS_TOKEN` y `PASARELA_METRICS_TOKEN` se entregan a cada aplicación como `APP_METRICS_BEARER_TOKEN`; nunca van en los archivos de targets. Los targets usan direcciones alcanzables desde Docker.

Al regenerar `infra/.local/secrets/nats-server.conf`, aplica la configuración al NATS local con `docker compose --env-file bacekend-plataform/.env up -d --force-recreate nats`. Esto interrumpe temporalmente las conexiones NATS y hace que los clientes se reconecten; conserva el volumen JetStream y solo apunta al stack local de desarrollo.

Para validar configuración Compose sin iniciar contenedores, ejecuta `docker compose --env-file bacekend-plataform/.env config --quiet`. La verificación completa de ingestión, reglas, dashboards, alertas y degradación requiere levantar la infraestructura y ejecutar pruebas de integración en un entorno aislado.

Tras editar una configuración montada de Collector o Tempo, `docker compose restart otel-collector tempo` reinicia únicamente esos dos servicios para que relean sus archivos, sin borrar sus volúmenes ni reiniciar PostgreSQL, Redis o NATS. La configuración actual usa el esquema de Collector 0.162 (`telemetry.metrics.readers`) y la ruta de retención de Tempo 3 (`backend_scheduler.provider.compaction`).

Para comprobar degradación del exportador OTLP exclusivamente en desarrollo: `docker compose stop otel-collector`, hacer una solicitud a la aplicación local y confirmar su respuesta, luego `docker compose start otel-collector` y comprobar que vuelve a recibir telemetría. El experimento detiene y arranca solo el Collector; no borra datos ni detiene PostgreSQL, Redis o NATS. No ejecutar contra producción.

## Sentry Cloud

Sentry es un destino externo y no forma parte de este Compose local. Cada backend recibe su `SENTRY_DSN` desde su propio `.env` ignorado en desarrollo y desde el gestor de secretos del entorno desplegado. No copies el DSN de un backend al otro: cada uno puede usar su proyecto Sentry. La política de selección del proyecto, redacción y evidencia de recepción de plataforma está en [ADR-0007](../bacekend-plataform/docs/adr/ADR-0007-destino-sentry.md); el estado de aceptación de la Fase 1 está en [su documento de cierre](../bacekend-plataform/docs/phase-1/acceptance.md). No pongas el DSN en archivos versionados ni en esta guía.

## Requisitos de producción

Esta pila es una base reproducible de desarrollo, no una receta de producción. En producción, instala el plano de datos/observabilidad en hosts controlados, redes privadas y endpoints cifrados; usa TLS/mTLS según el límite de confianza, secretos gestionados y rotables, autenticación delante de Grafana/Loki/Tempo, retención/backups probados, control de acceso y alertas hacia un destino operativo. Configura cada backend por separado en su VM/VPS. No agregues bins de aplicación a este Compose ni publiques puertos de administración a Internet.

## Límites actuales

El Collector acepta OTLP traces/metrics/logs. La biblioteca `crates/observability` usa el [appender oficial de OpenTelemetry para `tracing`](https://docs.rs/opentelemetry-appender-tracing/0.33.0/opentelemetry_appender_tracing/) y exporta spans/logs por lotes OTLP cuando `OTEL_EXPORTER_OTLP_ENDPOINT` está configurado; los logs JSON en stdout siguen activos como destino local de respaldo. La aplicación no está agregada como contenedor; su target Prometheus local usa `platform.yml`. La imagen del Collector usa la distribución oficial publicada en [GHCR](https://opentelemetry.io/docs/collector/install/docker/), ya que el tag fijado no estaba disponible en Docker Hub. La recepción real por Loki, Tempo y Prometheus se valida en [la aceptación de Fase 1](../bacekend-plataform/docs/phase-1/acceptance.md); Sentry Cloud requiere un DSN de proyecto separado y no forma parte de este Compose.
