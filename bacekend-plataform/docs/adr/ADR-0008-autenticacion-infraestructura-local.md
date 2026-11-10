# ADR-0008: Autenticación de las interfaces locales de infraestructura

- **Estado:** Aceptada para desarrollo local
- **Fecha:** 2026-10-22
- **Ámbito:** Compose compartido de desarrollo; no define el despliegue productivo de los backends

## Contexto

La pila local incluye herramientas administrativas para PostgreSQL y Grafana/Prometheus. pgAdmin y Grafana cuentan con autenticación propia; RedisInsight no ofrece una cuenta local configurable en esta composición, así que no debe publicarse al host. Loki y Tempo son APIs, no consolas web: se mantienen en redes privadas de Compose y se consultan desde Grafana. El Nginx de entrada del despliegue productivo vive fuera de esta infraestructura de desarrollo y no debe duplicarse en sus Compose.

La infraestructura compartida se inicia explícitamente con `docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml`. `compose.postgres.yaml` contiene PostgreSQL, su exporter y pgAdmin; `compose.redis.yaml` contiene Redis, su exporter y RedisInsight; `compose.nats.yaml` contiene NATS JetStream y sus exporters; `compose.yaml` contiene la pila de observabilidad. Todos se ejecutan bajo el mismo proyecto para compartir redes/servicios. Cada backend conserva su propio `.env`; no se introduce uno en la raíz. Para iniciar un componente por separado se usa su archivo, por ejemplo `docker compose --env-file bacekend-plataform/.env -f compose.redis.yaml`.

## Decisión

1. Añadir pgAdmin 4 con su autenticación nativa y persistencia en un volumen propio. El usuario es el correo configurado mediante `PGADMIN_DEFAULT_EMAIL`; la contraseña se configura mediante `PGADMIN_DEFAULT_PASSWORD`.
   - Al registrar PostgreSQL desde este pgAdmin de Compose, usar `postgres` como host, `5432` como puerto, los valores de `POSTGRES_DB` y `POSTGRES_USER`, y `POSTGRES_PASSWORD` como contraseña. `127.0.0.1` dentro del contenedor apunta a pgAdmin, no a PostgreSQL. El nombre visible, por ejemplo `Payvia local`, se escribe en la pestaña General y no como host.
2. Añadir RedisInsight con el acceso a Redis preconfigurado. Como RedisInsight no aporta en esta configuración una cuenta nativa de inicio de sesión y no se requiere acceso desde el host, mantener la interfaz únicamente en la red privada de Compose, sin publicar su puerto.
3. Habilitar Basic Auth nativa en Prometheus mediante `--web.config.file` y hash bcrypt. Grafana consulta Prometheus con las mismas credenciales de forma provisionada como dato seguro.
4. Mantener Loki y Tempo en la red privada de Compose y sin publicar sus puertos al host. Grafana y el Collector acceden por la red interna; Grafana es la interfaz de consulta. No se incluye un proxy Nginx en el Compose.
5. Añadir NATS Surveyor como exporter interno, con autenticación propia de NATS y Basic Auth en su endpoint de métricas. No se publica su puerto al host: la interfaz de consulta es Grafana, no Surveyor. El usuario de Surveyor pertenece a la cuenta de sistema NATS y tiene privilegios de sistema; por ello, esta configuración se limita al stack local, no se presenta como una política de mínimo privilegio de producción.
6. Generar credenciales faltantes localmente con `infra/prepare-dev-secrets.sh`, escribirlas solo en `bacekend-plataform/.env` ignorado y crear los hashes/archivos en `infra/.local/secrets/` ignorado. El script no imprime valores y no reemplaza credenciales ya configuradas.
7. Enlazar a `127.0.0.1` las interfaces administrativas que sí se publiquen al host, como Grafana, Prometheus y pgAdmin. RedisInsight, Loki y Tempo permanecen sin puertos publicados. Esta configuración local no debe exponerse a una red ni reutilizarse como manifiesto de producción.

La política de entrada usa los mecanismos documentados por [pgAdmin para contenedores](https://www.pgadmin.org/docs/pgadmin4/latest/container_deployment.html), [Prometheus Basic Auth](https://prometheus.io/docs/prometheus/latest/configuration/https/), [autenticación de Loki](https://grafana.com/docs/loki/latest/operations/authentication/), [autenticación de Tempo](https://grafana.com/docs/tempo/latest/operations/authentication/), [configuración de RedisInsight](https://redis.io/docs/latest/operate/redisinsight/configuration/) y [NATS Surveyor](https://github.com/nats-io/nats-surveyor).

## Consecuencias

- Las credenciales de desarrollo no se almacenan en Git, documentación, imágenes ni `.env` raíz. Cada desarrollador consulta sus valores localmente en `bacekend-plataform/.env`.
- pgAdmin y PostgreSQL comparten la red Compose `postgres_admin`; pgAdmin resuelve PostgreSQL por el DNS interno `postgres`. El puerto `127.0.0.1:5432` corresponde a clientes ejecutados directamente en el host, no a clientes dentro del contenedor pgAdmin.
- Los puertos de RedisInsight, Loki y Tempo no están disponibles desde el host; las consultas de logs/trazas se realizan desde Grafana.
- NATS Surveyor añade observabilidad de NATS y requiere una identidad con acceso a la cuenta de sistema. El privilegio queda confinado a la red Compose local y su puerto HTTP no se publica.
- RedisInsight, Loki y Tempo no son accesibles directamente desde el host; Grafana conserva el acceso de consulta a Loki/Tempo por la red Compose. No se añade Nginx al Compose porque el proxy de entrada del entorno desplegado vive fuera de esta infraestructura local.
- Prometheus usa Basic Auth local. Loki y Tempo no tienen autenticación propia en esta topología de desarrollo, por lo que su aislamiento depende de no publicar puertos y mantener sus redes Compose privadas. Esto no reemplaza controles de acceso en un eventual despliegue diferente.

## Verificación asociada

La evidencia ejecutada de Compose, controles de autenticación, disponibilidad de servicios y límites de esta decisión se registra en [aceptación de Fase 1](../phase-1/acceptance.md). No se declara validado el despliegue productivo ni el acceso interactivo con un navegador.
