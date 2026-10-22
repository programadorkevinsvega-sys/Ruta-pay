# Despliegue independiente de plataforma

Este directorio contiene artefactos operativos exclusivos de `bacekend-plataform`. El backend se construye como un binario/imagen propia y se ejecuta en su VM/VPS. `bacekend-pasarela`, `frontend-pasarela` y `frontend-plataform` tienen procesos, imágenes o artefactos, configuración, secretos, redes y despliegues independientes.

El `Cargo.toml` y `Cargo.lock` raíz comparten resolución de dependencias y compilación en el repositorio. No combinan los procesos ni exigen desplegar los backends en el mismo host. El Dockerfile compila únicamente `-p bacekend-plataform`; la imagen final contiene únicamente ese binario. El contexto de build es la raíz del repositorio por ser un workspace Cargo único:

```bash
docker build -f bacekend-plataform/deploy/Dockerfile -t payvia-platform:local .
```

## Binario de producción

Los scripts de este backend son independientes del backend de pasarelas y resuelven el manifiesto Cargo compartido desde la raíz:

```bash
./bacekend-plataform/build-production.sh
./bacekend-plataform/run-production.sh
```

`build-production.sh` ejecuta `cargo build --locked --release -p bacekend-plataform`. `run-production.sh` carga con `set -a` el archivo ignorado `bacekend-plataform/.env` y ejecuta `target/release/bacekend-plataform`; no ejecuta Cargo. En una VM, coloca ese `.env` como secreto del backend y configura permisos restrictivos. Plataforma mantiene sus propias variables, incluidas las credenciales de dependencias que use. Pasarela debe mantener su propio archivo independiente con copias propias de las variables compartidas que necesite. El repositorio no usa un `.env` en la raíz.

Variables leídas por el binario: `APP_HTTP_ADDR`, `APP_MODULE_GRPC_ADDR`, `APP_METRICS_ADDR`, `APP_METRICS_BEARER_TOKEN`, `APP_ENV`, `APP_VERSION`, `OTEL_EXPORTER_OTLP_ENDPOINT` y `SENTRY_DSN`. Las direcciones, entorno y versión tienen defaults en la configuración de Rust; OTLP, DSN y token son opcionales según la topología. Si métricas escucha fuera de loopback, el token debe tener al menos 32 caracteres y la red debe estar limitada al scraper privado. Usa un DSN distinto por backend/entorno y no lo imprimas ni lo pongas en Git.

Para desplegar la imagen en una VM/VPS, configura por separado sus variables, certificados y secretos. No uses el Dockerfile de plataforma para desplegar `bacekend-pasarela`. El gRPC de `platform_runtime` es interno al proceso de plataforma y escucha en loopback; no se publica. La entrada HTTP del BFF y las dependencias remotas se configuran con la red privada y los controles TLS/mTLS correspondientes a cada entorno. Para que un scraper remoto acceda al listener de métricas, configura `APP_METRICS_ADDR=0.0.0.0:9090` y `APP_METRICS_BEARER_TOKEN` (mínimo 32 caracteres) y restringe el puerto a la red privada de observabilidad; el token debe entregarse como secreto de runtime.

La infraestructura compartida de PostgreSQL, Redis, NATS JetStream y observabilidad se configura desde [`compose.yaml`](../../compose.yaml) y [`infra/README.md`](../../infra/README.md), pasando `--env-file bacekend-plataform/.env` a Docker Compose. Esa unidad es distinta de la imagen de plataforma y no arranca ningún backend. Para producción, plataforma recibe endpoints privados y credenciales/certificados propios del entorno; esta configuración local no es un despliegue de producción.
