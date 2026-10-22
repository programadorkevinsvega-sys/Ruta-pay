<div align="center">

# Ruta-pay

**Orquestador de pagos multi-pasarela en Rust.**
Backend de plataforma: tenants, planes, facturación, autorización y reglas de negocio de pagos.

![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-15+-blue?logo=postgresql)
![Redis](https://img.shields.io/badge/Redis-idempotencia-red?logo=redis)
![NATS](https://img.shields.io/badge/NATS-JetStream-green)
![React](https://img.shields.io/badge/React-TypeScript-61dafb?logo=react)
![Estado](https://img.shields.io/badge/estado-en%20desarrollo-yellow)

</div>

## Backend de Plataforma Payvia

## Propósito

Este backend es el plano de control y el dueño de las reglas de la plataforma SaaS. Mantiene el registro y ciclo de vida de los tenants/comercios, los usuarios y permisos, el catálogo de planes, precios y versiones, los límites y cuotas, las suscripciones y la facturación que Payvia cobra por el uso del sistema.

También es la autoridad de autorización para operaciones del backend de pasarelas. Una solicitud no queda autorizada por llegar desde otro servicio: la plataforma evalúa tenant, identidad de servicio, entorno, operación, estado de suscripción, plan, permisos, cuotas y límites vigentes. El resultado se entrega por contrato interno autenticado y correlacionado.

## Responsabilidades

- Registro, verificación, activación, suspensión y cierre de tenants.
- Identidad, sesiones, membresías, roles y permisos de comercios y operadores de plataforma.
- Planes, precios versionados, periodos, límites, cuotas, uso medible y cambios efectivos.
- Suscripciones, invoices, cobro de tarifas de Payvia, estado de cuenta, mora y auditoría.
- API Keys de tenant: creación, hash, entorno, scopes, revocación y exposición del secreto una sola vez.
- API pública y reglas de negocio de payment intents, transacciones, idempotencia, fees de plataforma y transiciones de estado.
- Decisiones de autorización para operaciones de pasarela, incluida la aplicación de plan, límites y cuotas.
- BFF REST/OpenAPI para los frontends independientes de tenant y administración de plataforma.
- Recepción idempotente de eventos durables normalizados de pasarela, aplicación de reglas de cuenta, auditoría y publicación de cambios visibles.
- Entrega durable de webhooks salientes a comercios, con payload de negocio versionado, firma, reintentos y auditoría.
- APIs REST de recuperación autorizada, cursores y snapshots para realtime.

## Características del backend

- Fuente de verdad de tenants, identidad, permisos, API Keys, planes, precios, cuotas, suscripciones y facturación de Payvia.
- API pública versionada para payment intents y consultas de transacciones.
- Autorización por tenant, entorno, cuenta, operación, monto, plan y límites antes de contactar una pasarela.
- BFF REST/OpenAPI y WebSocket para los dos frontends, con identidad y permisos separados para comercio y administración.
- Eventos de dominio durables mediante Outbox/NATS JetStream y entrega de webhooks salientes con firma, reintentos y auditoría.

## Arquitectura y flujo de pago

1. El BFF recibe el request del comercio, valida autenticación/contexto y llama por gRPC al módulo propietario.
2. El módulo `payments` valida API Key, idempotencia, tenant, moneda, monto, plan y permiso, y persiste la intención con su Outbox cuando corresponda.
3. Plataforma crea una concesión breve, firmada y ligada al comando y llama por gRPC autenticado al backend de pasarelas.
4. Pasarelas opera el protocolo del proveedor y publica el resultado verificado mediante evento durable.
5. Plataforma valida asociación, duplicado y transición de negocio, actualiza PostgreSQL y emite el evento para realtime y webhook delivery.

Los módulos de plataforma son crates con ownership propio. El BFF no contiene negocio ni accede directamente a PostgreSQL, Redis o NATS. La base PostgreSQL física es compartida por decisión del proyecto, pero cada módulo controla únicamente su schema, tablas y migraciones.

## Stack y requisitos

Stack objetivo: Rust/Tokio, Axum, `tonic`/`prost`, SQLx, PostgreSQL 15+, Redis con TTL, NATS JetStream/Outbox, OpenTelemetry, Sentry, React/TypeScript para los clientes. Para ejecutar una instalación local completa se necesitarán toolchains fijados, servicios de desarrollo, secretos de aplicación y, al probar pagos, credenciales oficiales sandbox.

El crate actual es un scaffold y aún no existe configuración reproducible de workspace, base de datos, NATS, Redis o BFF. Por eso todavía no hay comandos de instalación/arranque que se puedan declarar verificados. No uses los siguientes contratos de ejemplo como endpoints disponibles hasta que OpenAPI esté publicado.

## Uso de la API pública (contrato objetivo)

La operación inicial prevista es `POST /v1/payment_intents`. Ejemplo de request conceptual:

```bash
curl --request POST "$PAYVIA_API_URL/v1/payment_intents" \
  --header "Authorization: Bearer $PAYVIA_TEST_KEY" \
  --header "Idempotency-Key: order-2026-99481" \
  --header "Content-Type: application/json" \
  --data '{
    "amount": 8500000,
    "currency": "COP",
    "reference": "ORD-2026-99481",
    "description": "Compra en tienda",
    "metadata": {"cart_id": "sh_cart_99812"}
  }'
```

El backend valida API Key, tenant, ambiente, moneda/capacidad, monto, permiso, plan y `Idempotency-Key`. `amount` es entero en unidades menores; la interpretación de moneda depende del contrato y proveedor real. La API y el esquema final todavía no están publicados.

## Escuchar el estado en tiempo real

Los clientes obtienen un WebSocket autenticado a través del BFF de plataforma. La identidad, audiencia, tenant y acceso a la transacción se verifican en servidor. El contrato de conexión y suscripción se publicará junto con OpenAPI; no se documenta aquí una ruta todavía inexistente.

Los eventos llevan ID estable, cursor, versión de agregado y scope. El frontend deduplica y detecta huecos, reconecta y recupera estado mediante REST autorizado. PostgreSQL conserva la fuente durable, JetStream permite replay y Redis se limita a fan-out efímero entre réplicas.

## Webhooks hacia el servidor del comercio

El módulo `webhook_delivery` de plataforma crea entregas durables cuando existe un evento notificable. El worker manda POST a la URL HTTPS configurada por el comercio, registra cada intento y programa reintentos con backoff (política inicial prevista: 2, 5, 15 y 60 minutos, con jitter y límites sujetos al contrato final).

| Header | Valor/propósito |
|---|---|
| `X-Payvia-Signature` | `t=<timestamp>,v1=<hmac_sha256>` calculado con el secreto del endpoint y la regla versionada de bytes firmados. |
| `User-Agent` | Identifica el worker; ejemplo `Payvia-Worker/1.0.0`. |
| `Content-Type` | `application/json`. |

Payload de ejemplo:

```json
{
  "id": "evt_7f3b2a9c1e5d48a0",
  "object": "event",
  "type": "payment.successful",
  "created_at": "2026-10-20T05:05:23Z",
  "api_version": "v1",
  "data": {
    "transaction": {
      "id": "tx_901234567890abcdef",
      "reference": "ORD-2026-99481",
      "amount": 8500000,
      "currency": "COP",
      "status": "approved",
      "payment_method": {"type": "card", "details": {"brand": "visa", "last4": "4242"}},
      "gateway": {"name": "bold", "receipt_id": "bold_rec_9938102"}
    },
    "customer": {"id": "cust_33421", "email": "juan.perez@email.com"},
    "metadata": {"cart_id": "sh_cart_99812"}
  }
}
```

El receptor debe verificar firma y timestamp sobre el cuerpo original conforme al contrato, comparar en tiempo constante, deduplicar por `id` y devolver el código HTTP definido para cada intento. Datos personales son opcionales/minimizados; nunca enviar PAN o CVV. La representación y la política criptográfica aún requieren contrato versionado y vectores de prueba. La recepción de callbacks entrantes de Wompi/Bold es distinta y usa el protocolo oficial de cada proveedor.

`payment_method` es una unión discriminada común. `type=card` incluye solo marca/últimos cuatro dígitos; `type=pse` puede incluir código/nombre de banco y referencia permitida; `type=crypto` puede incluir `asset`, `settlement_currency`, `settlement_amount` como string decimal y referencias del proveedor. Ejemplos normalizados:

```json
{
  "type": "pse",
  "details": {
    "bank_code": "BANK_CODE",
    "bank_name": "Banco de ejemplo",
    "payment_reference": "provider-reference-if-available"
  }
}
```

```json
{
  "type": "crypto",
  "details": {
    "asset": "USDT",
    "settlement_currency": "USDT",
    "settlement_amount": "85.00000000",
    "network": null,
    "provider_order_id": "binance_order_id",
    "provider_transaction_id": "binance_transaction_id",
    "network_transaction_id": null
  }
}
```

El importe `transaction.amount` permanece como el importe comercial en unidades menores enteras; el importe cripto se conserva aparte como string decimal. `network` e ID on-chain son opcionales y no deben inferirse para Binance Pay. El adapter traduce estados/campos específicos de proveedor a este union versionado y nunca expone el callback crudo. Binance Pay tiene su propio protocolo de firma y campos de orden: seguir su [documentación oficial de autenticación](https://developers.binance.com/en/docs/products/binance-pay-merchant/api-common) y [consulta de órdenes](https://developers.binance.com/en/docs/products/binance-pay-merchant/api-order-query); no reutilizar la firma saliente HMAC-SHA256 de Payvia para autenticar al proveedor.

## Instalación y estado actual

No hay instalación ejecutable verificada. Antes de publicar comandos deben existir manifests/workspace, versiones fijadas, configuración de servicios locales y migraciones aplicables. Sigue el [roadmap](ROADMAP.md); el scaffold actual no implementa esta API, worker, WebSocket ni webhooks.

## Fuera de responsabilidad

Este backend no implementa SDKs, firma de solicitudes de proveedor, verificación criptográfica específica de webhooks entrantes ni mapeos particulares de Wompi, Bold u otras pasarelas. Esos adapters pertenecen exclusivamente a [`bacekend-pasarela`](../bacekend-pasarela/README.md). La plataforma define si una operación está permitida; el backend de pasarela ejecuta el protocolo autorizado.

## Arquitectura

Es una aplicación Rust/Tokio organizada como monolito modular: cada bounded context es dueño de sus reglas, contratos, schema/tablas/migraciones y Outbox. Las llamadas síncronas a módulos internos usan gRPC versionado; los cambios durables entre contextos usan Outbox y NATS JetStream. El BFF es la frontera REST/OpenAPI y no accede directamente a PostgreSQL, Redis o NATS.

La autorización del servicio de pasarelas se realiza mediante gRPC interno autenticado (mTLS/identidad de servicio). Plataforma decide antes de cada operación tenant, entorno, cuenta, permiso, plan, cuota, acción y monto/moneda; solo entonces invoca el método del gateway y entrega una autorización firmada, breve y ligada a esa operación. Pasarelas verifica identidad, firma, scope, tenant, cuenta, importe y vigencia y rechaza por defecto cualquier mismatch. No se cachean permisos de negocio en Redis. Las consultas de solo lectura también requieren scope explícito. Así se evita un callback RPC circular desde pasarelas a plataforma durante la misma operación.

Para registrar/rotar credenciales, el usuario las envía al BFF de plataforma; plataforma comprueba el permiso y las reenvía en un comando gRPC sobre mTLS al módulo `gateway_credentials` de pasarelas. Plataforma no persiste el valor ni lo publica en Outbox/NATS. El vault lo cifra y solo entrega el secreto en memoria al adapter correcto para una operación autorizada. El navegador nunca puede volver a leerlo.

El backend de pasarelas no recibe autoridad permanente ni puede modificar planes, cuotas o estados de suscripción. Las llamadas técnicas de plataforma para cobrar sus propias facturas usan una identidad de servicio y un scope explícito de `platform_billing_collection`, con límites/auditoría; no existe bypass administrativo implícito.

En cada cruce plataforma↔pasarelas se validan autenticación mutua, certificado y rotación, autorización por servicio/método/scope, claims ligados al request, versión/tipos/tamaño/rangos, deadline, rate limit, idempotencia y contexto de trazabilidad. Requests con caller/certificado no permitido, replay o grant alterado se rechazan cerradamente. Timeouts/reintentos son acotados y no repiten operaciones financieras inciertas sin reconciliación. gRPC da resultado síncrono aunque un comando persista; Outbox/JetStream se reserva para cambios durables y no lleva secretos. Ambos servicios registran eventos de diagnóstico correlacionados con redacción.

Webhooks de proveedores primero se autentican y verifican en el backend de pasarelas. Si la validación pasa, este publica un evento versionado y durable con su Outbox. Plataforma verifica identidad de productor, tenant/cuenta/entorno registrados, deduplicación y transición permitida antes de aplicar efectos de negocio a payment intents/transacciones. Si plataforma o JetStream no están disponibles, el evento no se descarta ni se confirma como procesado. La plataforma emite luego sus propios eventos de dominio y entrega webhooks a los sistemas de los comercios.

## Persistencia, eventos y realtime

- Los dos backends usan una única base física PostgreSQL conforme a la norma vigente. Los módulos de plataforma poseen schemas y migraciones versionadas propias; no consultan tablas de pasarelas por SQL ni hacen JOINs entre dominios. Las migraciones se aplican coordinadamente.
- Redis se reserva para cache/estado efímero con TTL y se documenta su degradación; no es autoridad de tenant, entitlement, plan ni pago.
- NATS JetStream transporta eventos durables con `event_id`, versión, tenant/scope y contexto de trazabilidad. Los consumidores deduplican y recuperan mensajes pendientes.
- Los cambios visibles de tenant, permisos, planes, límites, suscripciones y facturas se publican por Outbox → JetStream → gateway realtime → WebSocket autenticado. TanStack Query aplica orden/deduplicación y recupera huecos por REST autorizado con cursor/snapshot.
- Los frontends tenant y administración son aplicaciones independientes, con autorización separada. Ambos acceden únicamente al BFF REST/OpenAPI y WebSocket autenticado.

## Seguridad y diagnóstico

Cada frontera produce eventos correlacionados de inicio, llamada a dependencia, fallo clasificado y resultado final. Logs incluyen nombre estable, componente, operación, dependencia, código/categoría, duración, `correlation_id` y `trace_id`, más metadatos técnicos seguros disponibles. Sentry, OpenTelemetry y métricas no reciben secretos, API Keys, cookies, firmas completas, payloads sensibles ni URL firmadas. Las respuestas REST usan envelopes seguros y accionables.

Toda operación administrativa y decisión de autorización es auditable. Se aplican tenant isolation, least privilege, protección de secretos, rate limits y control de replay/idempotencia. No se confía en IDs, tenant claims o scopes proporcionados por el frontend sin validación del backend.

## Estado

El crate actual es un scaffold `Cargo.toml`/`src/main.rs`; no implementa estas capacidades. No se debe declarar una funcionalidad disponible hasta contar con migraciones, lógica, contratos, integración real, pruebas y evidencia de operación.

## Roadmap y reglas

- [Roadmap específico de plataforma](ROADMAP.md)
- [Reglas específicas de plataforma](PROJECT_RULES.md)
- [Roadmap integrado de Payvia](../ROADMAP.md)
- [Reglas específicas del proyecto](../PROJECT_RULES.md)
- [Reglas de arquitectura](../ARCHITECTURE_RULES.md)
- Prompts de calidad y observabilidad incluidos en este directorio.

## Alineación de arquitectura

La topología de dos backends está definida en [las reglas de arquitectura](../ARCHITECTURE_RULES.md). Plataforma contiene realtime y reglas de negocio; pasarelas es un backend desplegable separado con adapters técnicos. Antes de habilitar tráfico real, cada integración debe completar sus contratos, autorización firmada, migraciones coordinadas, observabilidad y aceptación cross-backend.
