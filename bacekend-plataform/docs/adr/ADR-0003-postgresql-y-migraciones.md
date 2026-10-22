# ADR-0003: PostgreSQL físico compartido y ownership de schema

- Estado: Aceptada
- Fecha: 2026-10-22

## Decisión

Plataforma y pasarelas utilizan una sola instancia/base física PostgreSQL, con schemas, permisos, tablas, índices y migraciones versionadas propiedad de cada bounded context. Un módulo no consulta, modifica ni hace JOIN con tablas/schema de otro módulo. La aplicación de migraciones se coordina por release con orden y compatibilidad documentados; cada migración debe poder validarse desde base vacía y desde la versión anterior soportada. La Outbox del evento se escribe en la misma transacción local que el cambio dueño; no se simula transacción distribuida.

## Justificación

La base física compartida satisface la topología actual, mientras ownership y privilegios reducen acoplamiento. Los módulos deben conservar una futura posibilidad de despliegue independiente.

## Consecuencias y controles

- Cada módulo tendrá rol DB limitado al schema que posee; BFF no recibe acceso a PostgreSQL.
- La identidad organizacional no permite acceso entre tenants por sí sola; autorización e aislamiento se validan en el módulo dueño.
- RLS podrá ser defensa adicional si el módulo y contexto de sesión pueden evitar bypass por el rol de aplicación.
- Rollback/compatibilidad y estrategia de backup/restore deben verificarse antes de migraciones de producción.

## No aplicable en Fase 0

No se crean tablas ni migraciones porque no se implementan datos persistidos en esta fase. El primer schema se entrega con el módulo dueño, acompañado de integración con PostgreSQL real.
