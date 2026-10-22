# Bounded contexts de backend-plataform

Los siguientes límites son decisiones de dominio para organizar crates independientes del workspace raíz. No se crean crates sin comportamiento en Fase 0. Cada contexto será dueño de su API pública gRPC, schema/migraciones, Outbox y telemetría. Un módulo no consulta tablas de otro módulo.

| Contexto | Ownership | No posee | Consumers/canales |
|---|---|---|---|
| `identity_access` | Usuarios, sesiones, membresías y permisos tenant/operador | Tenant lifecycle, planes, pago proveedor | BFF vía gRPC; eventos versionados cuando un cambio de permisos requiera fan-out durable/realtime. |
| `tenant_registry` | Alta, verificación, estado y cierre de tenants | Credenciales de pasarela; membresías de usuario | BFF vía gRPC; eventos de cambios visibles por Outbox. |
| `catalog_pricing` | Planes, versiones de precios, moneda y vigencias | Asignación de consumo e invoices | API administrativa vía BFF/gRPC; eventos versionados. |
| `entitlements_quotas` | Capacidades, evaluación de límites y ledger de consumo | Definición del catálogo o cobro | Llamadas request-response gRPC; ledger durable en PostgreSQL. Redis solo puede acelerar estado efímero con TTL y nunca decidir autorización por sí solo. |
| `subscriptions_billing` | Suscripción, periodo, facturas/partidas y cobro de Payvia al tenant | Cargos del comprador del comercio procesados por pasarelas | gRPC para comandos inmediatos; Outbox/JetStream para cambios durables. |
| `payments` | Payment intents, transacciones y transiciones de negocio del comercio | Protocolo HTTP del proveedor o almacenamiento de sus credenciales | Autorización previa + RPC gRPC a pasarelas; eventos verificados entran por JetStream. |
| `authorization` | Decisión allow/deny, emisión y auditoría de concesiones cortas | Ejecución del protocolo del proveedor | RPC gRPC de respuesta inmediata; no se cachea decisión como autoridad. |
| `audit` | Historial durable de acciones administrativas y de seguridad | Logs técnicos efímeros o payloads secretos | Eventos/cambios append-only; consulta autorizada vía gRPC. |
| `realtime` | Journal/cursor, autorización de audiencia y streaming interno | Fuente de verdad de dominio | Outbox/JetStream de cambios visibles; gRPC streaming al BFF; REST cursor/snapshot a través del BFF. Redis Pub/Sub solo fan-out efímero multi-réplica. |
| `webhook_delivery` | Endpoints, secreto de firma del webhook de Payvia, intentos/reintentos | Firma/verificación de callback de proveedor | Outbox/JetStream durable y worker; payload mínimo, SSRF controls y entrega auditable. |
| `bff` (aplicación) | REST/OpenAPI, autenticación de sesión de borde, adaptación y composición | Reglas de dominio, SQL, Redis o NATS directo | Consume únicamente contratos gRPC de módulos; puentea WebSocket autenticado con `realtime`. |

## Composición y contratos

- El paquete de backend plataforma conserva su binario/composition root y no importa implementación de backend pasarelas.
- Cada bounded context se materializa como crate cuando se inicia su implementación funcional; solo expone `module_public_api`/contratos gRPC tipados.
- BFF es una aplicación separada dentro del mismo backend desplegable; no constituye un módulo con acceso directo a persistencia.
- `gateway_credentials` pertenece exclusivamente a backend-pasarela. Plataforma autoriza y reenvía la credencial por mTLS/gRPC sin guardarla ni ponerla en eventos.
- BFF, módulos, workers y realtime conservan configuración, identidad y permisos explícitos. Ningún crate genérico se convierte en dueño de reglas de un dominio.

## Ownership de datos

Existe una única base física PostgreSQL compartida por decisión del proyecto. Cada contexto obtiene schema propio, migraciones versionadas propias y permisos DB mínimos. Migraciones coordinadas se ejecutan por release; claves foráneas compuestas solo dentro de ownership acordado, sin JOINs ni escrituras cross-schema.
