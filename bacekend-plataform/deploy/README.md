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

La lista completa de variables usadas por el runtime, los workers y las migraciones está en [`../.env.example`](../.env.example). Cópiala a `bacekend-plataform/.env` en el entorno de despliegue, completa las credenciales y claves reales, y fija permisos `0600`; no uses el `.env.example` directamente.

En la VM `192.0.2.123`, la inspección de listeners del 2026-11-08 encontró estos puertos ocupados por aplicaciones existentes: `3000` (frontend Next.js), `3001` (servicio Node.js) y `3002` (frontend Next.js). No los reutilices ni detengas esos procesos. Las plantillas asignan a Ruta-pay puertos distintos: plataforma HTTP `18080`, gRPC interno `15051` y métricas `19090`; pasarela HTTP `18081` y métricas `19091`. Los cinco estaban libres al verificarlos. El frontend de Ruta-pay se compila como archivos estáticos y no necesita un listener de aplicación propio; el proxy web debe servir esos archivos y enrutar `/v1` al BFF de plataforma en `18080`. Configura el reverse proxy de confianza para dirigir el tráfico del BFF a `18080` y restringe ese puerto a la red privada/proxy de entrada; no publiques el listener gRPC.

Son obligatorias las URLs de PostgreSQL/Redis, las credenciales de NATS y las claves de firma/verificación compartidas con pasarelas. `PLATFORM_MIGRATION_DATABASE_URL` y `PLATFORM_RUNTIME_DB_ROLE` se requieren al ejecutar migraciones; el rol migrador debe ser distinto del rol runtime. Si métricas escucha fuera de loopback, el token debe tener al menos 32 caracteres y la red debe estar limitada al scraper privado; para scrape remoto ajusta `APP_METRICS_ADDR` a `0.0.0.0:19090` y limita el puerto por firewall al scraper. Cuando tengas el DSN, configura `SENTRY_DSN`; no dejes la variable definida con valor vacío. Mantén un DSN distinto por backend/entorno y no lo imprimas ni lo pongas en Git. `PLATFORM_TEST_DATABASE_ADMIN_URL` es solo de pruebas y no debe incluirse en producción.

## Migraciones de PostgreSQL

Las migraciones son versionadas y propiedad de cada módulo. El binario `migrate-platform` ejecuta los migradores de `identity_access`, `tenant_registry` y `realtime` de forma coordinada sobre la base física compartida. El proceso de la API nunca crea ni altera tablas al arrancar.

Provisiona `PLATFORM_MIGRATION_DATABASE_URL` como secreto propio del backend y `PLATFORM_RUNTIME_DB_ROLE` con el nombre de la cuenta usada por `PLATFORM_DATABASE_URL`. El usuario migrador debe ser diferente del usuario runtime, no ser superusuario ni tener `BYPASSRLS`, y ser propietario de los schemas/tablas que migra o recibir los permisos DDL necesarios. El comando comprueba que las URLs pertenecen a la misma base y que los usuarios sean distintos. Tras aplicar migraciones, concede al rol runtime `USAGE`, DML y privilegios de secuencia en los schemas propiedad de plataforma y configura privilegios por defecto para futuras tablas creadas por el migrador. El runtime debe ser un rol sin `BYPASSRLS`. `PLATFORM_TEST_DATABASE_ADMIN_URL` es exclusivamente para pruebas de desarrollo aisladas y nunca se configura en producción.

El despliegue sigue este orden: construir los dos binarios, tomar y verificar un backup recuperable, detener/poner en mantenimiento las escrituras incompatibles si la migración lo requiere, ejecutar la migración una sola vez, revisar su código de salida y el ledger SQLx de cada módulo, desplegar la nueva versión de plataforma y comprobar readiness y métricas. La VM debe tener exclusivamente el `.env` de plataforma con permisos restrictivos; no uses el `.env` de pasarelas, de los frontends ni uno general.

```sh
./bacekend-plataform/build-production.sh
./bacekend-plataform/migrate-production.sh
./bacekend-plataform/run-production.sh
```

`migrate-production.sh` carga `bacekend-plataform/.env` y ejecuta el binario release ya compilado; no ejecuta Cargo ni imprime la URL. Si falla, detén el despliegue, conserva el log diagnóstico saneado y no arranques la versión nueva hasta entender el error. No reviertas automáticamente migraciones con pérdida de datos: usa cambios expand/contract (agregar, desplegar compatibilidad, migrar datos, retirar en otra versión) y restaura backup solo con un plan probado.

Para la base local existente, no se debe asumir que un schema sin filas en `_sqlx_migrations` está vacío o equivale a una instalación nueva. Antes de adoptar una versión base se verifican tablas, columnas, constraints, índices, políticas RLS, ownership y datos contra el SQL versionado; si hay diferencias, se detiene y se prepara una reconciliación específica sin borrar tablas ni datos.

El primer operador se inicializa una sola vez desde una terminal local, con la migración `identity_access` aplicada y permiso PostgreSQL para insertar el operador:

```sh
cd bacekend-plataform
set -a
source .env
set +a
cargo run -p bacekend-plataform --bin bootstrap-platform-operator -- <correo>
```

La herramienta pide y confirma el password sin mostrarlo; no lo acepta por argumento ni expone un endpoint anónimo. Ejecuta el comando en la VM privada de plataforma y no lo incorpores al arranque del servicio.

Para desplegar la imagen en una VM/VPS, configura por separado sus variables, certificados y secretos. No uses el Dockerfile de plataforma para desplegar `bacekend-pasarela`. El gRPC de `platform_runtime` es interno al proceso de plataforma y escucha en loopback; no se publica. La entrada HTTP del BFF y las dependencias remotas se configuran con la red privada y los controles TLS/mTLS correspondientes a cada entorno. Para que un scraper remoto acceda al listener de métricas, configura `APP_METRICS_ADDR=0.0.0.0:9090` y `APP_METRICS_BEARER_TOKEN` (mínimo 32 caracteres) y restringe el puerto a la red privada de observabilidad; el token debe entregarse como secreto de runtime.

La infraestructura compartida de PostgreSQL, Redis, NATS JetStream y observabilidad se configura desde [`compose.yaml`](../../compose.yaml) y [`infra/README.md`](../../infra/README.md), pasando `--env-file bacekend-plataform/.env` a Docker Compose. Esa unidad es distinta de la imagen de plataforma y no arranca ningún backend. Para producción, plataforma recibe endpoints privados y credenciales/certificados propios del entorno; esta configuración local no es un despliegue de producción.
