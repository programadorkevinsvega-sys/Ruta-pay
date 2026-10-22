# Despliegue independiente de plataforma

Este directorio contiene artefactos operativos exclusivos de `bacekend-plataform`. El backend se construye como un binario/imagen propia y se ejecuta en su VM/VPS. `bacekend-pasarela`, `frontend-pasarela` y `frontend-plataform` tienen procesos, imágenes o artefactos, configuración, secretos, redes y despliegues independientes.

El `Cargo.toml` y `Cargo.lock` raíz comparten resolución de dependencias y compilación en el repositorio. No combinan los procesos ni exigen desplegar los backends en el mismo host. El Dockerfile compila únicamente `-p bacekend-plataform`; la imagen final contiene únicamente ese binario. El contexto de build es la raíz del repositorio por ser un workspace Cargo único:

```bash
docker build -f bacekend-plataform/deploy/Dockerfile -t payvia-platform:local .
```

Para desplegar la imagen en una VM/VPS, configura por separado sus variables, certificados y secretos. No uses el Dockerfile de plataforma para desplegar `bacekend-pasarela`. El gRPC de `platform_runtime` es interno al proceso de plataforma y escucha en loopback; no se publica. La entrada HTTP del BFF y las dependencias remotas se configuran con la red privada y los controles TLS/mTLS correspondientes a cada entorno. Para que un scraper remoto acceda al listener de métricas, configura `APP_METRICS_ADDR=0.0.0.0:9090` y `APP_METRICS_BEARER_TOKEN` (mínimo 32 caracteres) y restringe el puerto a la red privada de observabilidad; el token debe entregarse como secreto de runtime.

La infraestructura compartida de PostgreSQL, Redis, NATS JetStream y observabilidad se configura desde [`compose.yaml`](../../compose.yaml) y [`infra/README.md`](../../infra/README.md). Esa unidad es distinta de la imagen de plataforma y no arranca ningún backend. Para producción, plataforma recibe endpoints privados y credenciales/certificados propios del entorno; esta configuración local no es un despliegue de producción.
