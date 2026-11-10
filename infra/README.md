# Infraestructura compartida de desarrollo

La infraestructura local se divide por responsabilidad en cuatro archivos Compose: `compose.postgres.yaml` contiene PostgreSQL, su exporter y pgAdmin; `compose.redis.yaml` contiene Redis, su exporter y RedisInsight; `compose.nats.yaml` contiene NATS JetStream, NATS Exporter y Surveyor; `compose.yaml` contiene la pila compartida de observabilidad. No construyen ni ejecutan `bacekend-plataform`, `bacekend-pasarela` ni los frontends: cada aplicación conserva su binario/imagen, Dockerfile, configuración, secretos y despliegue de VM/VPS independiente.

La configuración está fijada a tags de imagen versionados. PostgreSQL arranca con 200 conexiones máximas, 1 GiB de shared buffers, 4 GiB de cache estimada, work_mem de 8 MiB y workers paralelos acotados; Redis usa 4 hilos I/O, backlog TCP de 4096, hasta 20.000 clientes, memoria limitada por defecto a 2 GiB, AOF cada segundo y política `noeviction` para no expulsar claves de idempotencia; NATS limita conexiones/payload/pending y reserva cuotas explícitas de JetStream. Son valores iniciales conservadores para desarrollo con carga concurrente, ajustables en Compose mediante `PG_MAX_CONNECTIONS`, `PG_SHARED_BUFFERS`, `PG_EFFECTIVE_CACHE_SIZE`, `PG_WORK_MEM` y `PG_MAINTENANCE_WORK_MEM`; `REDIS_MAXMEMORY` controla el límite de Redis (2 GiB por defecto). Con `noeviction`, al alcanzar ese límite Redis rechazará escrituras en lugar de expulsar claves de idempotencia. Mantén el pool total de conexiones de los binarios por debajo del límite de PostgreSQL sumando todos los procesos/replicas. Los límites de NATS son cuotas máximas, no reservas de RAM/disco. El archivo de preparación genera credenciales aleatorias locales, no las imprime y no sobrescribe `bacekend-plataform/.env`; si el archivo ya existe y contiene los valores generados, puede reconstruir los archivos de servicio. La carpeta `infra/.local/` está ignorada por Git y mantiene los secretos locales con permisos restrictivos. También está excluida del contexto Docker para que nunca entre en imágenes de aplicación. No se crea ni se usa un `.env` en la raíz.

Las direcciones de `192.0.2.0/24` son placeholders reservados para documentación; reemplázalas por valores privados en la configuración local del entorno antes del despliegue. No publiques direcciones internas ni credenciales reales.

## Servicios locales

| Servicio | Dirección local | Persistencia |
|---|---|---|
| PostgreSQL | `127.0.0.1:5432` | volumen `postgres_data` |
| Redis con AOF y contraseña | `127.0.0.1:6379` | volumen `redis_data` |
| NATS con JetStream y autenticación | `127.0.0.1:4222` | volumen `nats_data`; hasta 10.000 conexiones, payload de 1 MiB, pending de 64 MiB, JetStream limitado a 512 MiB RAM y 20 GiB en disco |
| pgAdmin 4 | `127.0.0.1:5050` | volumen `pgadmin_data`; login pgAdmin |
| RedisInsight | solo red interna Compose; sin puerto publicado | volumen `redisinsight_data`; conexión Redis preconfigurada |
| Node Exporter | solo red interna Compose | métricas del host local de desarrollo |
| Prometheus | `127.0.0.1:9091` | Basic Auth, volumen `prometheus_data`, 15 días |
| Alertmanager | `127.0.0.1:9093` (perfil opcional `notifications`) | Agrupa alertas; notifica a Telegram usando secretos locales |
| Grafana | `127.0.0.1:3000` | volumen `grafana_data` |
| OTLP gRPC/HTTP Collector | `127.0.0.1:4317` / `127.0.0.1:4318` | imagen oficial Contrib 0.162.0 fijada en GHCR; exporta a Tempo, Loki y Prometheus |
| Tempo | solo red interna Compose; acceso desde Grafana/Collector | volumen `tempo_data`, 7 días |
| Loki | solo red interna Compose; acceso desde Grafana/Collector | volumen `loki_data`, 7 días |
| Exporters de PostgreSQL, Redis, NATS y NATS Surveyor | solo red interna Compose; scrape autenticado cuando corresponde | sin estado propio |

Las credenciales locales están en `bacekend-plataform/.env`, ignorado por Git, y los hashes/archivos de servicio en `infra/.local/secrets/`. Los cuatro archivos usan ese `.env` explícito para interpolar el stack; no se crea un `.env` raíz. Puedes iniciar un componente por separado con su archivo, por ejemplo `docker compose --env-file bacekend-plataform/.env -f compose.redis.yaml up -d`. Para iniciar el stack completo, carga los cuatro archivos con el mismo proyecto y en este orden: `docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml`. Cada backend conserva su `.env` independiente, aunque repita credenciales compartidas de PostgreSQL, Redis o NATS.

| Interfaz | Usuario | Contraseña | Autenticación |
|---|---|---|---|
| Grafana | `GRAFANA_ADMIN_USER` | `GRAFANA_ADMIN_PASSWORD` | Nativa Grafana |
| pgAdmin 4 | `PGADMIN_DEFAULT_EMAIL` | `PGADMIN_DEFAULT_PASSWORD` | Nativa pgAdmin; se usa el correo como usuario |
| Prometheus | `PROMETHEUS_ADMIN_USER` | `PROMETHEUS_ADMIN_PASSWORD` | Basic Auth nativa; hash bcrypt local |
| NATS Surveyor | `NATS_SURVEYOR_USER` / `NATS_SURVEYOR_HTTP_USER` | `NATS_SURVEYOR_PASSWORD` / `NATS_SURVEYOR_HTTP_PASSWORD` | Credencial NATS de sistema y Basic Auth del endpoint de métricas |

Los valores secretos se generan y consultan localmente en `bacekend-plataform/.env`; no se imprimen en esta guía ni se versionan. NATS Surveyor es un exporter de métricas, no un panel web: sus datos se consultan en Grafana con el usuario/contraseña de Grafana. RedisInsight no tiene una cuenta local configurable equivalente a pgAdmin; por eso su interfaz no se publica al host y queda accesible solo desde la red privada de Compose. La administración de logs y trazas se realiza desde Grafana; Loki y Tempo no publican sus puertos al host.

Para registrar PostgreSQL desde el pgAdmin de Compose, usa `postgres` como host y `5432` como puerto; completa la base, el usuario y la contraseña con `POSTGRES_DB`, `POSTGRES_USER` y `POSTGRES_PASSWORD` de `bacekend-plataform/.env`. El nombre visible, por ejemplo `Payvia local`, va en la pestaña General. No uses `127.0.0.1` como host desde pgAdmin en Docker: dentro de ese contenedor apunta al propio pgAdmin. Para un cliente PostgreSQL instalado directamente en el host, el host publicado sí es `127.0.0.1:5432`.

Los paneles de Grafana cubren aplicaciones (RPS, p50/p95/p99, 5xx, solicitudes en curso), PostgreSQL, dependencias compartidas (Redis/NATS), recursos del host y pipeline de observabilidad (scrapes, logs y trazas). Las reglas Prometheus cubren disponibilidad, proporción de errores, latencia HTTP p95, saturación HTTP, recursos del host y disponibilidad de Collector/Loki/Tempo. Alertmanager envía las alertas `warning` y `critical` al chat Telegram configurado. Los targets `platform.yml` y `pasarela.yml` son para desarrollo local: usan `host.docker.internal:9090` y `host.docker.internal:9091`, respectivamente. En la VM de aplicación, las plantillas de producción usan métricas loopback `127.0.0.1:19090` y `127.0.0.1:19091`; si Prometheus está en otro host, habilita scrape remoto con `0.0.0.0` en esos puertos, bearer token de al menos 32 caracteres y firewall limitado al scraper privado.

Prometheus y Grafana requieren autenticación propia. Loki, Tempo y RedisInsight permanecen en redes privadas de Compose, sin puertos del host publicados. Usa Grafana para consultar logs y trazas; no configures una publicación directa de esos endpoints en el Nginx de entrada a Internet. Ningún puerto administrativo se expone públicamente.

Loki y Tempo son APIs, no consolas web, y sus puertos no están publicados al host. Grafana (`http://localhost:3000`) consulta ambos servicios por la red interna de Compose y es la interfaz para explorar logs y trazas. RedisInsight tampoco tiene un puerto publicado; en este Compose local se mantiene accesible únicamente desde su red interna.

## Preparar y validar

El plan y la matriz requisito→integración→prueba para completar la instrumentación de ambos binarios, los enlaces Loki↔Tempo y Alertmanager con Telegram están en [OBSERVABILITY_IMPLEMENTATION.md](OBSERVABILITY_IMPLEMENTATION.md). Ese documento distingue lo ya configurado de lo pendiente; no implica que la entrega de alertas externas o Sentry Cloud esté validada.

Requisitos: Docker Engine con el plugin Compose, Bash y OpenSSL. `infra/prepare-dev-secrets.sh` usa Docker para generar hashes bcrypt sin imprimir contraseñas. Para el backend Rust: Rust/Cargo del toolchain fijado por el repositorio.

```bash
./infra/prepare-dev-secrets.sh
docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml up -d --wait
docker compose --env-file bacekend-plataform/.env -f compose.redis.yaml up -d --wait
docker compose --env-file bacekend-plataform/.env -f compose.nats.yaml up -d --wait
```

Cada comando inicia el servicio, su exporter y su interfaz administrativa asociada. Para iniciar además la autenticación administrativa compartida y la pila de observabilidad, valida y levanta los cuatro archivos (si ya iniciaste los componentes por separado, Compose los conserva bajo el mismo proyecto y añade el resto de servicios):

```bash
docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml config --quiet
docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml up -d --wait
```

Los servicios tienen volúmenes persistentes. No se ejecuta automáticamente como parte de los cambios de código. Para detener el stack completo sin borrar datos, usa los mismos dos archivos:

```bash
docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml down
```

No uses `docker compose down -v` salvo que quieras borrar todos los datos locales. Las pruebas de reinicio deben documentar antes el servicio afectado y solo apuntar a esta composición de desarrollo, nunca a producción.

El script genera usuarios/contraseñas administrativas, credenciales de NATS Surveyor, hashes para Prometheus/proxy y los secretos ya existentes. También crea vacíos los archivos locales de Telegram si aún no existen y les asigna permisos `0600`; no reemplaza los archivos que ya hayas completado. `PLATFORM_METRICS_TOKEN` se configura en plataforma como `APP_METRICS_BEARER_TOKEN`; `PASARELA_METRICS_TOKEN` se configura en pasarela con ese mismo nombre. Nunca van en los archivos de targets. Los targets usan direcciones alcanzables desde Docker.

Telegram está configurado como receptor de alertas de Alertmanager. Completa localmente `infra/.local/secrets/telegram_bot_token` con el token del bot y `infra/.local/secrets/telegram_chat_id` con el ID del chat. No guardes esos valores en `.env`, el repositorio ni los pases como argumentos de comandos. Ambos archivos deben conservar permisos `0600`. Inicia el perfil opcional después de configurar ambos archivos:

```bash
docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml --profile notifications up -d --wait alertmanager
```

Prometheus ya tiene configurado el destino `alertmanager:9093`; mientras el perfil `notifications` no esté iniciado, las reglas se siguen evaluando pero no se entregan avisos. Revisa `http://localhost:9093` en el entorno local y valida la entrega real solo con credenciales autorizadas.

Al regenerar `infra/.local/secrets/nats-server.conf`, aplica la configuración al NATS local con `docker compose --env-file bacekend-plataform/.env -f compose.nats.yaml up -d --force-recreate nats`. Esto interrumpe temporalmente las conexiones NATS y hace que los clientes se reconecten; conserva el volumen JetStream y solo apunta al stack local de desarrollo.

Para validar la configuración completa sin iniciar contenedores, ejecuta `docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml config --quiet`. Cada componente se puede validar por separado con su archivo, por ejemplo `docker compose --env-file bacekend-plataform/.env -f compose.redis.yaml config --quiet`. La verificación completa de ingestión, reglas, dashboards, alertas y degradación requiere levantar la infraestructura y ejecutar pruebas de integración en un entorno aislado.

Tras editar una configuración montada de Collector o Tempo, `docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml restart otel-collector tempo` reinicia únicamente esos dos servicios para que relean sus archivos, sin borrar sus volúmenes ni reiniciar PostgreSQL, Redis o NATS. La configuración actual usa el esquema de Collector 0.162 (`telemetry.metrics.readers`) y la ruta de retención de Tempo 3 (`backend_scheduler.provider.compaction`).

Para comprobar degradación del exportador OTLP exclusivamente en desarrollo: `docker compose --env-file bacekend-plataform/.env -f compose.postgres.yaml -f compose.redis.yaml -f compose.nats.yaml -f compose.yaml stop otel-collector`, hacer una solicitud a la aplicación local y confirmar su respuesta, luego ejecutar el mismo comando con `start otel-collector` y comprobar que vuelve a recibir telemetría. El experimento detiene y arranca solo el Collector; no borra datos ni detiene PostgreSQL, Redis o NATS. No ejecutar contra producción.

## Sentry Cloud

Sentry es un destino externo y no forma parte de este Compose local. Configura los nombres exactos por servicio: `SENTRY_DSN` en `bacekend-plataform/.env` y `GATEWAY_SENTRY_DSN` en `bacekend-pasarela/.env`. No compartas el DSN entre servicios salvo que así lo decidas en Sentry. El script de preparación deja comentada la clave de plataforma para que la completes después; agrega el DSN de pasarela cuando tengas uno válido. Ambos aceptan opcionalmente `SENTRY_TRACES_SAMPLE_RATE` entre `0` y `1`; omite la variable mientras no elijas una tasa y nunca dejes definido un DSN vacío. La política de selección del proyecto, redacción y evidencia de recepción de plataforma está en [ADR-0007](../bacekend-plataform/docs/adr/ADR-0007-destino-sentry.md); el estado de aceptación de la Fase 1 está en [su documento de cierre](../bacekend-plataform/docs/phase-1/acceptance.md). No pongas el DSN en archivos versionados ni en esta guía.

## Requisitos de producción

Los Compose raíz son exclusivamente para desarrollo. La topología productiva se mantiene aparte bajo `infra/production/`; cada archivo Compose se ejecuta únicamente en la VM que posee sus servicios. Ningún contenedor de aplicación ni frontend se agrega a esos archivos.

| Componente | VM / IP privada | Puertos privados previstos |
|---|---|---|
| Aplicaciones, frontends, Prometheus, Grafana, Loki, Tempo y Collector | `192.0.2.70` | Backend HTTP `18080`/`18081`, métricas `19090`/`19091`, Grafana `3000`, OTLP loopback `4317`/`4318` |
| PostgreSQL y su exporter | `192.0.2.142` | PostgreSQL TLS `5432`, métricas `9187` |
| Redis y su exporter | `192.0.2.65` | Redis TLS `6379`, métricas `9121` |
| NATS JetStream y su exporter | `192.0.2.97` | NATS TLS `4222`, métricas `7777` |

El tráfico de las aplicaciones a PostgreSQL, Redis y NATS usa las IP privadas y TLS con verificación de certificados. Los tres servicios requieren identidad de cliente mediante certificados firmados por una CA privada, además de la autorización propia del servicio (rol PostgreSQL, ACL Redis o usuario/permisos NATS). Los certificados deben incluir las IP privadas correspondientes en SAN para validar la identidad del servidor. Las rutas de claves, certificados, contraseñas y archivos ACL se suministran fuera de Git; no se generan ni se reutilizan los secretos locales de desarrollo.

Prometheus, ejecutándose en la VM `192.0.2.70`, consulta los dos endpoints de métricas de aplicaciones y los exporters de las tres VM de datos por la red privada. Los exporters no se publican a Internet: el firewall de cada VM de datos permite `9187`, `9121` o `7777` solo desde `192.0.2.70`. PostgreSQL, Redis y NATS solo aceptan conexiones de aplicación desde los hosts autorizados. Grafana se publica únicamente en `192.0.2.70:3000` para que la VM Nginx pueda acceder por la red privada; el firewall debe permitir ese puerto únicamente desde la IP privada correcta de Nginx y Nginx termina HTTPS para los usuarios. No se publica Prometheus, Alertmanager, Loki, Tempo ni OTLP al exterior.

La IP de Nginx se dejó fuera de los archivos porque el valor recibido, `10.02.82`, no identifica inequívocamente una IPv4 completa. Confirma su IP privada antes de aplicar la regla de firewall entre Nginx y Grafana. El archivo Compose no reemplaza ni instala el firewall, Nginx, certificados, cuentas, backups o políticas de retención. Deben definirse y verificarse antes de aceptar producción.

La matriz de integración de esta configuración productiva es:

| Frontera | Configuración prevista | Verificación y límites |
|---|---|---|
| Frontend → Nginx → BFF HTTP | Los frontends continúan consumiendo REST por el dominio HTTPS; Nginx en la VM separada enruta a `192.0.2.70:18080`/`18081`. | No se cambia el contrato web ni se exponen IP privadas de datos al navegador. El IP de Nginx debe confirmarse para el firewall. |
| BFF → gRPC | El tráfico gRPC permanece en loopback dentro de la VM de aplicaciones; no se publica a otras VM. | Sin cambios de contrato ni acceso a almacenes desde el BFF. |
| Aplicaciones → PostgreSQL | Conexión TLS `verify-full` a `192.0.2.142:5432`; `pg_hba.conf` exige certificado cliente cuyo CN corresponde al rol; migraciones usan identidad separada. | Comprobar SAN, CA, mTLS, autenticación, permisos de roles y conectividad desde la VM app antes de desplegar. |
| Aplicaciones → Redis | Conexión `rediss://` a `192.0.2.65:6379`; cliente Rust presenta CA, certificado y clave, Redis exige mTLS y ACL; Redis no es autoridad durable de negocio. | Probar handshake, rechazo sin identidad y operaciones reales con ACL mínima antes de aceptar el despliegue. |
| Aplicaciones → NATS JetStream | Conexión TLS `verify_and_map`: SAN DNS del certificado del backend identifica al usuario NATS y sus permisos. | Probar autenticación, publicación, consumo, redelivery y reconexión; certificados sin usuario mapeado o subject no permitido deben rechazarse. |
| Exporters → PostgreSQL/Redis | Certificados clientes y roles/ACL separados; los exporters PostgreSQL y Redis presentan identidad propia. | Validar handshake y acceso de solo lectura con servicios reales; los certificados y permisos deben rotarse por separado de las aplicaciones. |
| Prometheus → aplicaciones/exporters | Bearer auth para `192.0.2.70:19090`/`19091`; scrape HTTP privado a exporters en `.142:9187`, `.65:9121`, `.97:7777`. | Firewall permite únicamente la VM Prometheus; verificar targets `UP` y métricas reales. Los exporters no tienen TLS ni auth HTTP en esta configuración y requieren firewall estricto. |
| Cambios visibles/realtime | Esta topología no escribe ni cambia estados de negocio; no añade Outbox, eventos, rutas, cursor, WebSocket ni invalidaciones TanStack Query. | N/A: no se introduce un cambio de estado persistido ni un contrato realtime. |
| Secretos/operación | Cada VM recibe un `.env` local con modo `0600`; claves y configuración sensible se montan desde rutas externas no versionadas. | Rotación, backups, restauración, firewall y pruebas de reinicio quedan a cargo del operador y no se ejecutan desde este repositorio. |

### PostgreSQL de staging

`infra/staging/postgres/` es una configuración separada para staging y **desactiva TLS expresamente**. No reutiliza ni debilita los archivos de `infra/production/postgres/`, donde TLS y autenticación por certificado siguen siendo obligatorios. PostgreSQL publica solo en `192.0.2.142`, exige SCRAM y acepta conexiones únicamente desde la VM de aplicaciones `192.0.2.70` y la propia VM para el exporter. El exporter usa red host y enlaza su listener a `192.0.2.142:9187`, de modo que UFW aplica su política de entrada. No publiques estos puertos en una interfaz pública o Tailscale.

El `.env` de staging debe estar fuera de Git y con permisos `0600`; `POSTGRES_EXPORTER_PASSWORD` debe ser aleatorio y `POSTGRES_PASSWORD_FILE` debe apuntar a un archivo de contraseña bootstrap protegido. La clave bootstrap no se reutiliza como identidad de aplicación. La primera inicialización crea el rol de métricas `postgres_exporter`; los scripts de `docker-entrypoint-initdb.d` no se vuelven a ejecutar sobre un volumen ya inicializado. Antes de ejecutar `up`, valida el Compose y comprueba que la VM tenga espacio y memoria suficientes:

```bash
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d --wait
```

Este despliegue no incluye TLS, alta disponibilidad ni una política de backup/restauración; es solo para staging y no debe declararse preparado para producción.

### Redis de staging

`infra/staging/redis/` es independiente de `infra/production/redis/`. Desactiva TLS solo para staging, requiere ACL con credenciales aleatorias y enlaza Redis y el exporter a `192.0.2.65`; UFW limita ambos puertos a la VM de aplicaciones `192.0.2.70`. No se publican listeners en Tailscale ni en interfaces públicas. El usuario de aplicación puede operar claves y Pub/Sub, pero no ejecutar comandos administrativos o destructivos; healthcheck y exporter tienen identidades separadas. AOF habilitado, `noeviction`, memoria Redis limitada a 512 MiB y límites de proceso ajustados a la VM de staging de 1 vCPU/1.9 GiB.

Al desplegar, genera en la VM las contraseñas de aplicación, healthcheck y exporter, crea `users.acl` desde esas credenciales, y guarda cada secreto bajo `/etc/payvia/staging/redis/` con acceso restringido. El archivo de contraseña del exporter es un JSON que mapea `redis://payvia_exporter@192.0.2.65:6379` a su contraseña, según el formato del exporter oficial. No copies el `.env.example` como configuración ni publiques las credenciales en logs o chat. `REDIS_ACL_FILE` debe ser legible por el UID 999 del contenedor Redis; el directorio que contiene secretos en el host debe seguir inaccesible a usuarios no autorizados. Reiniciar o recrear el servicio no debe borrar el volumen AOF de este nuevo despliegue.

El staging no cifra el tráfico Redis: úsalo exclusivamente por la red privada aislada, con el firewall restringido. No es una configuración apta para producción. La configuración productiva en `infra/production/redis/` continúa requiriendo TLS/mTLS y no se modifica con esta variante.

### NATS de staging

`infra/staging/nats/` es independiente de `infra/production/nats/`: desactiva TLS solo para staging, pero conserva autenticación NATS por credenciales separadas para plataforma y pasarelas. NATS y su exporter usan la red del host para que UFW filtre los puertos de forma efectiva: el firewall permite `4222` y `7777` únicamente desde `192.0.2.70`; el endpoint de monitoreo `8222` solo escucha en loopback. JetStream persiste en su volumen propio con límites iniciales de 256 MiB en memoria y 4 GiB en disco, ajustados a la VM de staging de 2 vCPU/3.7 GiB. El despliegue actual está saludable en `192.0.2.97`; se verificó la autenticación y publicación permitida desde `192.0.2.70`, el rechazo de permisos no autorizados y el scrape del exporter. JetStream está habilitado, pero aún no contiene streams ni consumidores; la aplicación de plataforma debe provisionarlos. La purga o recreación del volumen elimina mensajes retenidos.

El archivo de autorización es un secreto de despliegue fuera del repositorio en `/etc/payvia/staging/nats/authorization.conf`, montado como Docker secret y con contraseñas almacenadas como hashes bcrypt. Las contraseñas aleatorias de cliente están en `/etc/payvia/staging/nats/platform.password` y `/etc/payvia/staging/nats/gateway.password`, ambas con modo `0600`; no se muestran ni se guardan en Git. `platform` publica eventos de `payvia.platform.events.v1.>` y puede administrar JetStream; `gateway` publica solo eventos de pagos/reconciliación, consume la solicitud de reconciliación y tiene acceso limitado a la información/consumidores del stream de plataforma. Ambos reciben únicamente los permisos de inbox y ACK requeridos. Las credenciales aún no se han provisionado en la VM de aplicaciones ni configurado en los procesos; hacerlo por un canal seguro es requisito antes de conectar esos procesos. No habilites usuarios anónimos ni abras `8222`. No es una configuración apta para producción: `infra/production/nats/` conserva mTLS y `verify_and_map`.

### Gestión de secretos

Los componentes obtienen secretos desde el gestor aprobado por el entorno. La configuración y operación del gestor de secretos queda fuera del alcance de esta guía.

#### Decisión de identidad de servicios de datos productivos

Se exige mTLS en las conexiones de los backends y exporters a PostgreSQL, Redis y NATS de producción. La identidad de la conexión se valida en el servicio receptor y se combina con la autorización de rol/ACL/subjects; una IP privada, contraseña o ACL por sí solas no reemplazan el certificado cliente. Esta decisión cubre únicamente los listeners de datos productivos y sus clientes de aplicación/exporter. No implementa ni adelanta la autenticación mTLS del canal gRPC Plataforma↔Pasarelas, que continúa sujeta a la Fase 5 y a la puerta de cierre de Fase 4 del roadmap.

La matriz de esta configuración es:

| Frontera | Decisión | Criterios de aceptación |
|---|---|---|
| Backend → PostgreSQL | `sslmode=verify-full`, certificado cliente del rol y `pg_hba.conf` con autenticación `cert`; exporter con identidad y rol `pg_monitor` propios. | Rechazo de conexión sin certificado, CA no confiable, certificado vencido, CN/rol distinto o SAN de servidor incorrecto; aceptar solo roles autorizados. Migraciones conservan identidad separada. |
| Backend → Redis | `rediss://`, verificación de CA/nombre, certificado cliente propio y ACL de aplicación; no se permite conexión sin TLS. | Rechazo de cliente sin cert, cert/CA inválidos o ACL insuficiente; operaciones válidas con cuenta restringida; reconexión con el mismo nivel de autenticación. |
| Exporters → PostgreSQL/Redis | Certificado distinto por exporter, permisos de solo lectura y endpoint de métricas limitado a Prometheus por firewall. | Scrape real autenticado; rechazar cert/rol/ACL inválido y mantener no accesibles los exporters desde Internet. |
| Backend → NATS JetStream | TLS `verify_and_map` mapea SAN DNS a identidad NATS individual; permisos mínimos por subjects. | Rechazo de cert, CA o identidad no mapeada y subject no permitido; publicación, consumo, redelivery y reconexión con identidad autorizada. |
| Frontend → Nginx → BFF, BFF → gRPC | Sin cambios: esta decisión no agrega acceso de navegador a almacenes ni autentica el canal gRPC entre backends. | N/A: no se altera contrato, ruta ni estado de dominio; gRPC mTLS permanece en Fase 5. |
| Estado visible / realtime | No se modifica estado de negocio ni contratos de eventos, WebSocket, cursor o TanStack Query. | N/A: no hay mutación persistida visible que requiera Outbox/JetStream/realtime adicional. |

Las pruebas criptográficas de aceptación se deben ejecutar en infraestructura aislada con certificados de prueba emitidos para esos tests, servicios reales y sin secretos versionados. Incluyen identidad válida, ausencia de certificado, CA/SAN/validez incorrectos, privilegios insuficientes, reinicio/reconexión de Redis/NATS/PostgreSQL y recuperación de operaciones persistidas. No se reinician servicios de producción. Compose no está disponible en este entorno y no se han generado certificados; por tanto, la aceptación de handshakes reales sigue pendiente hasta poder ejecutar esa matriz.

### Política TLS y rotación

PostgreSQL fija `ssl_min_protocol_version=TLSv1.2` y `ssl_max_protocol_version=TLSv1.3`; Redis permite únicamente `TLSv1.2 TLSv1.3`; NATS fija `min_version: "1.2"` y su runtime Go negocia la versión más alta compatible, TLS 1.3. Los clientes Rust usan Rustls/OpenSSL con TLS 1.3 preferido y TLS 1.2 como único fallback permitido por esta política. No se habilita modo TLS inseguro ni se omite validación de CA, vigencia o nombre/IP SAN. URLs PostgreSQL usan `sslmode=verify-full`, Redis usa `rediss://`, y los clientes NATS reciben CA y certificado/clave de cliente. La feature Redis habilitada excluye expresamente `tls-rustls-insecure`.

Usa una CA privada con emisión separada para servidor y clientes, EKU/SAN correctos, claves privadas restringidas al proceso, inventario de expiración y renovación anticipada. Los certificados servidor PostgreSQL deben incluir `192.0.2.142` y `postgres`; Redis debe incluir `192.0.2.65` y `redis`; NATS debe incluir `192.0.2.97`. Emite certificados cliente separados para el rol runtime y migración de plataforma, y para plataforma y pasarela en NATS; configura CN/mapping de PostgreSQL si se usa `clientcert=verify-full`. Las rutas de TLS de los servidores se montan como directorios de solo lectura para que la rotación atómica de archivos sea visible dentro del contenedor. Renueva con un periodo de solapamiento: instala el nuevo certificado y cadena, valida la cadena/SAN y la clave antes de aplicarlo, conserva la CA anterior durante la transición, y retírala solo después de comprobar que ya no hay consumidores con certificados anteriores.

- PostgreSQL lee certificado, clave y CA al iniciar y al recargar configuración; `pg_ctl reload`/SIGHUP actualiza nuevas conexiones sin cerrar las existentes.
- Redis permite actualizar `tls-cert-file` y `tls-key-file` con `CONFIG SET`; instala el par nuevo bajo nombres versionados en el directorio montado, aplica esos paths con `CONFIG SET` y sincroniza `redis.conf` para el próximo arranque. Los clientes conectados conservan su sesión TLS; las conexiones nuevas validan y negocian con el certificado actualizado.
- NATS marca la configuración TLS como hot-reloadable; la recarga aplica el certificado a conexiones nuevas y las sesiones TLS existentes conservan el certificado anterior.

Los pools de aplicación son duraderos por proceso (`PgPool`, `ConnectionManager` de Redis y el cliente NATS compartido): no se crea una conexión TLS por request. La dependencia Redis usa Rustls sin la feature insegura; valida cadena y hostname contra la CA configurada y negocia TLS 1.3 primero, con TLS 1.2 como fallback. Mantén las sesiones establecidas, abre conexiones nuevas gradualmente y drena las antiguas antes de revocar o dejar expirar el certificado viejo. La topología aquí especifica una instancia por VM y no configura réplicas/failover. La recarga automática de certificados cliente en procesos ya iniciados no queda resuelta por cargar PEM desde archivos al arranque. La rotación sin interrupción requiere un mecanismo de recarga que renueve/reemplace pools/conexiones, o un despliegue solapado de réplicas; no se afirma que esté automatizada ni validada. Para garantizar ese requisito en producción hay que definir y probar capacidad de réplica, health/readiness y drenaje antes de revocar el certificado anterior.

La aceptación criptográfica pendiente debe demostrar handshake válido y rechazo de CA desconocida, certificado expirado, identidad/SAN incorrectos y certificados de cliente no autorizados, además de recarga con conexiones existentes y apertura de nuevas conexiones tras rotación. No se han generado certificados ni ejecutado estas pruebas en las VM.

Los exporters de PostgreSQL, Redis y NATS publican métricas HTTP sin TLS en la red privada. No contienen credenciales de negocio en sus respuestas, pero exponen información operativa: el firewall permite sus puertos solamente desde Prometheus (`192.0.2.70`) y no desde Internet. Los dos endpoints de métricas de aplicación usan bearer tokens. La VM de aplicaciones debe tener reglas de firewall equivalentes para limitar `19090`/`19091` a Prometheus y `3000` a Nginx.

La configuración de producción de observabilidad reutiliza las configuraciones versionadas de Collector, Loki, Tempo, datasources, dashboards y reglas; Loki/Tempo usan almacenamiento local de una sola réplica y retención inicial de siete días, Prometheus conserva quince días. Esto no configura HA, backups, cifrado de discos, SLO ni capacidad suficiente: dimensiona almacenamiento y memoria, respalda y prueba restauración antes del servicio productivo. El acceso de usuarios debe pasar por Nginx con HTTPS; Prometheus, Alertmanager y los puertos OTLP no se publican en el host.

Los frontends se compilan y despliegan separadamente. Sus `.env.example` actuales configuran únicamente proxy de desarrollo; en producción el navegador usa el dominio HTTPS y Nginx enruta a los backends en `.70`. No se incluyen credenciales ni endpoints privados de datos en bundles frontend.

## Límites actuales

El Collector acepta OTLP traces/metrics/logs. La biblioteca `crates/observability` usa el [appender oficial de OpenTelemetry para `tracing`](https://docs.rs/opentelemetry-appender-tracing/0.33.0/opentelemetry_appender_tracing/) y exporta spans/logs por lotes OTLP cuando `OTEL_EXPORTER_OTLP_ENDPOINT` está configurado; los logs JSON en stdout siguen activos como destino local de respaldo. La aplicación no está agregada como contenedor; se ejecuta como binario nativo en el host y se conecta a PostgreSQL/Redis/NATS por `127.0.0.1`; su target Prometheus local usa `platform.yml`. Los límites configurados evitan crecimiento ilimitado de conexiones/mensajes, pero el throughput real debe medirse con el benchmark de la aplicación y dimensionarse según RAM, CPU y almacenamiento disponibles. La imagen del Collector usa la distribución oficial publicada en [GHCR](https://opentelemetry.io/docs/collector/install/docker/), ya que el tag fijado no estaba disponible en Docker Hub. La recepción real por Loki, Tempo y Prometheus se valida en [la aceptación de Fase 1](../bacekend-plataform/docs/phase-1/acceptance.md); Sentry Cloud requiere un DSN de proyecto separado y no forma parte de este Compose.
