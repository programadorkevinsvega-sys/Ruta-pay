# ADR-0008: Autenticación de las interfaces locales de infraestructura

- **Estado:** Aceptada para desarrollo local
- **Fecha:** 2026-10-22
- **Ámbito:** Compose compartido de desarrollo; no define el despliegue productivo de los backends

## Contexto

La pila local expone herramientas administrativas para inspeccionar PostgreSQL, Redis, métricas, logs, trazas y mensajes NATS. pgAdmin 4, RedisInsight y NATS Surveyor no estaban disponibles; Prometheus, Loki y Tempo tampoco tenían autenticación de entrada en la composición. Aunque varios puertos ya estaban enlazados a loopback, la falta de autenticación dejaba las interfaces sin una barrera de credenciales para procesos y usuarios locales. Loki y Tempo son APIs, por lo que sus rutas raíz devolvían `404` al abrirlas como si fueran consolas.

La infraestructura compartida se inicia explícitamente con `docker compose --env-file bacekend-plataform/.env`. Cada backend conserva su propio `.env`; no se introduce uno en la raíz.

## Decisión

1. Añadir pgAdmin 4 con su autenticación nativa y persistencia en un volumen propio. El usuario es el correo configurado mediante `PGADMIN_DEFAULT_EMAIL`; la contraseña se configura mediante `PGADMIN_DEFAULT_PASSWORD`.
2. Añadir RedisInsight con el acceso a Redis preconfigurado. Como RedisInsight no aporta en esta configuración una cuenta nativa de inicio de sesión, publicar su interfaz únicamente detrás de Nginx Basic Auth. Mantener el puerto interno fuera de los puertos del host.
3. Habilitar Basic Auth nativa en Prometheus mediante `--web.config.file` y hash bcrypt. Grafana consulta Prometheus con las mismas credenciales de forma provisionada como dato seguro.
4. Mantener Loki y Tempo en la red privada de Compose, sin publicar directamente sus puertos. Nginx aplica Basic Auth antes de reenviar solicitudes. Sus raíces muestran una página informativa que dirige a Grafana; `/ready` es la ruta de disponibilidad.
5. Añadir NATS Surveyor como exporter interno, con autenticación propia de NATS y Basic Auth en su endpoint de métricas. No se publica su puerto al host: la interfaz de consulta es Grafana, no Surveyor. El usuario de Surveyor pertenece a la cuenta de sistema NATS y tiene privilegios de sistema; por ello, esta configuración se limita al stack local, no se presenta como una política de mínimo privilegio de producción.
6. Generar credenciales faltantes localmente con `infra/prepare-dev-secrets.sh`, escribirlas solo en `bacekend-plataform/.env` ignorado y crear los hashes/archivos en `infra/.local/secrets/` ignorado. El script no imprime valores y no reemplaza credenciales ya configuradas.
7. Enlazar interfaces administrativas publicadas exclusivamente a `127.0.0.1`. Esta configuración local no ofrece TLS y no debe exponerse a una red ni reutilizarse como manifiesto de producción.

La política de entrada usa los mecanismos documentados por [pgAdmin para contenedores](https://www.pgadmin.org/docs/pgadmin4/latest/container_deployment.html), [Prometheus Basic Auth](https://prometheus.io/docs/prometheus/latest/configuration/https/), [autenticación de Loki](https://grafana.com/docs/loki/latest/operations/authentication/), [autenticación de Tempo](https://grafana.com/docs/tempo/latest/operations/authentication/), [configuración de RedisInsight](https://redis.io/docs/latest/operate/redisinsight/configuration/) y [NATS Surveyor](https://github.com/nats-io/nats-surveyor).

## Consecuencias

- Las credenciales de desarrollo no se almacenan en Git, documentación, imágenes ni `.env` raíz. Cada desarrollador consulta sus valores localmente en `bacekend-plataform/.env`.
- Los puertos directos de Loki y Tempo dejan de estar disponibles desde el host; las consultas locales usan sus rutas protegidas o Grafana.
- NATS Surveyor añade observabilidad de NATS y requiere una identidad con acceso a la cuenta de sistema. El privilegio queda confinado a la red Compose local y su puerto HTTP no se publica.
- Nginx es una dependencia adicional para acceder a RedisInsight, Loki y Tempo desde el host. Si se cae, esos servicios siguen en su red privada pero sus interfaces protegidas no son accesibles desde el host.
- Prometheus, Loki y Tempo no se convierten en servicios de identidad completa: Basic Auth es una barrera local de desarrollo y no reemplaza TLS, gestión central de identidades, autorización por rol ni controles de red productivos.
- El `404` de la raíz de Loki/Tempo era una respuesta esperable de sus APIs. La página informativa resuelve la navegación humana sin alterar `/ready`, las rutas API ni el acceso interno del Collector/Grafana.

## Verificación asociada

La evidencia ejecutada de Compose, controles de autenticación, disponibilidad de servicios y límites de esta decisión se registra en [aceptación de Fase 1](../phase-1/acceptance.md). No se declara validado el despliegue productivo ni el acceso interactivo con un navegador.
