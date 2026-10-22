<div align="center">

# Ruta-pay

**Orquestador de pagos multi-pasarela en Rust.**
Backend de pasarelas: adapters técnicos independientes para Wompi, Bold y proveedores futuros.

![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-15+-blue?logo=postgresql)
![Redis](https://img.shields.io/badge/Redis-idempotencia-red?logo=redis)
![NATS](https://img.shields.io/badge/NATS-JetStream-green)
![React](https://img.shields.io/badge/React-TypeScript-61dafb?logo=react)
![Estado](https://img.shields.io/badge/estado-en%20desarrollo-yellow)

</div>

## Backend de Pasarelas Payvia

## Propósito

Este backend es el plano de ejecución técnica de las integraciones de pago con proveedores externos. Es un backend desplegable independiente, organizado internamente como monolito modular Rust. Cada proveedor vive en su módulo/crate (`gateway_wompi`, `gateway_bold`, `gateway_<provider>`); el módulo transversal `gateway_credentials` es el único dueño de los secretos de cuentas tenant. Añadir un proveedor requiere implementar su contrato técnico, registrar el módulo y superar la suite común de conformidad, sin editar adapters existentes.

Este backend no es dueño del registro de tenant, usuarios, planes, precios, suscripciones, cobros de Payvia, cuotas ni autorización comercial. La autoridad de esos datos y decisiones es [`bacekend-plataform`](../bacekend-plataform/README.md).

## Responsabilidades

- Implementar APIs de proveedor, autenticación, traducción y normalización de requests/responses.
- Administrar adaptadores separados y capacidades verificadas; `gateway_credentials` almacena credenciales de cuenta tenant y cada adapter declara su configuración requerida.
- Crear/consultar/ejecutar acciones financieras técnicas que solicite la plataforma bajo una autorización válida.
- Recibir callbacks/webhooks públicos de cada proveedor, preservar bytes/headers necesarios, verificar protocolo criptográfico oficial y anti-replay antes de confiar en el evento.
- Aplicar idempotencia técnica al protocolo/callback, registrar intentos y producir eventos versionados verificados.
- Comunicar resultados por contrato interno autenticado y entrega durable Outbox/JetStream; no decidir planes, cuotas ni estados de cuenta de plataforma.
- Mantener el servicio interno de credenciales con cifrado envelope, rotación, revocación y acceso restringido al adapter del proveedor correspondiente.
- Exponer health/readiness, métricas de proveedor, logs diagnósticos y trazas sin secretos ni payloads sensibles.

## Características

- Un crate independiente por proveedor (`gateway_wompi`, `gateway_bold`, `gateway_<provider>`), con capacidades declaradas y verificadas.
- Clientes HTTP/adapters, autenticación de requests, mapeo de respuestas y errores específicos del proveedor.
- Gestión cifrada de credenciales por cuenta y entorno bajo autorización de plataforma.
- Webhooks entrantes públicos: recepción de bytes originales, verificación criptográfica oficial, control anti-replay, persistencia técnica e idempotencia.
- Contrato gRPC interno para ejecutar comandos autorizados; Outbox/JetStream para entregar resultados verificados a plataforma.

## Arquitectura y flujo técnico

El backend es un monolito modular Rust separado del backend de plataforma. Plataforma es el único caller de las operaciones protegidas: valida negocio y permisos, después envía por gRPC autenticado una concesión firmada, breve y ligada al comando. Pasarelas valida identidad de servicio, firma, tenant, entorno, cuenta, operación e importe aplicable antes de contactar al proveedor.

En callbacks, el proveedor llama el endpoint público del adapter correspondiente. El módulo conserva los bytes/headers requeridos por el protocolo, valida autenticidad y anti-replay antes de interpretar el evento, persiste el resultado técnico con Outbox y publica un evento durable. Plataforma asocia ese evento a tenant/cuenta y decide el estado de negocio. El frontend no se conecta a este backend.

## Stack y requisitos

Stack objetivo: Rust/Tokio, `tonic`/`prost`, clientes HTTP async, PostgreSQL/SQLx, NATS JetStream/Outbox, Redis solo cuando se justifique con TTL, OpenTelemetry y Sentry. Cada integración real requiere cuenta elegible y credenciales del sandbox oficial del proveedor.

El código presente es scaffold: no hay workspace, adapters, migraciones ni servicio HTTP/gRPC listos para operar. Por eso no existe aún una instalación o comando de ejecución verificable. Los ejemplos siguientes describen responsabilidades/contratos planeados, no rutas activas.

## API de servicio y callbacks

No hay API pública de tenant en este backend. Plataforma consume los contratos gRPC internos versionados para ejecutar una operación con concesión. La ruta pública de callback se define de forma independiente por proveedor y solo se publica tras comprobar su documentación oficial; no se inventa un endpoint común ni un algoritmo universal.

La autenticación del webhook entrante depende de la pasarela: se verifica el método oficial sobre el contenido y headers requeridos, ventana de tiempo/anti-replay y cuenta/ambiente. Por ejemplo, no se debe reemplazar un checksum de un proveedor por HMAC-SHA256 solo para uniformar la implementación. Un webhook verificado se entrega por evento durable a plataforma; no constituye por sí mismo el estado comercial final.

## Normalización de métodos de pago

El adapter traduce las respuestas verificadas al contrato común de plataforma: `payment_method.type=card|pse|crypto` con `details` tipado. PSE puede reportar código/nombre del banco y referencia permitida; el adapter cripto puede reportar activo y monto de settlement decimal como string, además de IDs de orden/transacción disponibles. La red y el transaction ID on-chain son opcionales. Los importes nunca usan punto flotante y el callback completo no cruza a plataforma.

Cada protocolo conserva su autenticación propia. Binance Pay documenta una firma de API HMAC-SHA512 (distinta de la firma saliente HMAC-SHA256 de Payvia) y datos de orden como moneda, importe decimal `totalFee`, `transactionId` y estado. Seguir su [documentación oficial de firma](https://developers.binance.com/en/docs/products/binance-pay-merchant/api-common) y [consulta de órdenes](https://developers.binance.com/en/docs/products/binance-pay-merchant/api-order-query); confirmar capacidades concretas con el sandbox antes de declarar soporte.

## Tiempo real y webhooks salientes

Este backend no abre WebSockets para los usuarios ni envía los webhooks de negocio a los servidores de comercios. Publica el resultado técnico verificado hacia plataforma por Outbox/NATS JetStream; plataforma procesa la transición y su módulo `realtime` notifica a los clientes vía BFF. La entrega de eventos salientes al comercio y la firma `X-Payvia-Signature` pertenecen al backend de plataforma.

La plataforma remite altas, rotaciones y revocaciones de credenciales al contrato gRPC del backend de pasarelas por un canal TLS/mTLS con identidad de servicio rotatable y autorización por método y alcance. El módulo `gateway_credentials` valida y cifra las credenciales; solo el adapter autorizado las obtiene en memoria para ejecutar la operación. No se envían secretos por NATS ni se guardan en el backend de plataforma.

## Instalación

Todavía no hay comandos ejecutables verificados. Para completar una prueba real se necesitan el workspace/servicios de desarrollo, migraciones y credenciales sandbox oficiales. No marques un adapter listo hasta ejecutar contrato y flujo con el entorno oficial; mocks o tipos por sí solos no verifican integración.

## Límite de negocio

Plataforma autentica/autoriza al tenant, decide plan, scopes, límites y elegibilidad, y solo entonces llama por gRPC autenticado a `ExecuteAuthorizedGatewayOperation` con una concesión firmada, breve y ligada al comando. El contexto incluye identidad de servicio, tenant, entorno, cuenta proveedor, operación/capacidad, importe/moneda si aplica, `jti`, vigencia e IDs de correlación. Este backend verifica firma, claims y correspondencia con el comando antes de llamar al proveedor. Una concesión ausente, alterada, expirada o con scope/tenant/monto distinto produce rechazo cerrado; no hay bypass por cache Redis ni permisos locales. Plataforma y pasarelas no hacen callbacks RPC circulares dentro de una operación.

Este servicio no modifica el catálogo o estado de tenants y no crea una segunda política de autorización. Sus validaciones locales se limitan a verificar la concesión de plataforma y la seguridad del protocolo y consistencia técnica (firma, formato, idempotencia de callbacks, límites de request, capacidades del adapter). No inventa métodos, monedas, fees ni garantías que el proveedor no documente.

Los webhooks entrantes no están preautorizados por el comercio: primero se verifica la firma oficial del proveedor. Luego se guarda durablemente y se publica el evento verificado con contexto de cuenta/entorno en Outbox/JetStream. Si no puede resolverse el vínculo, el evento se conserva en estado no procesado para revisión/reintento seguro; el backend plataforma valida la asociación, autorización y reglas de negocio y cambia el estado autoritativo.

## Módulos por pasarela

Cada proveedor posee adapter, contratos/mapeos, descriptor de capacidades, verificador de webhook, clasificación de errores y pruebas de conformidad/sandbox. `gateway_credentials` es propietario del ciphertext, metadata mínima y ciclo de vida de las credenciales; cada adapter declara el esquema de campos requerido y recibe solo su secreto en memoria para una operación autorizada. El crate común contiene contrato neutral, errores compartidos, tipos de dinero/identidad técnica y harness de conformidad; no contiene lógica condicional central por nombre de proveedor.

Agregar una pasarela: crear el crate/módulo dueño, definir su descriptor y esquema tipado de credenciales para `gateway_credentials`, implementar el contrato gRPC versionado, registrar composición/configuración, documentar capacidades basadas en contrato oficial y pasar pruebas reales en sandbox. No se modifica el módulo existente de otro proveedor. La activación requiere autorización/configuración de plataforma y no habilita capacidades que el proveedor/cuenta no soporte.

## Integración y persistencia

- El frontend nunca se conecta directamente a este backend. Las solicitudes de pago llegan desde BFF/módulo autorizado de plataforma por gRPC interno autenticado.
- gRPC es síncrono para comandos de plataforma con concesión y ejecución proveedor. Eventos verificados de callback/resultados se publican durablemente por Outbox y NATS JetStream; no se usa mensajería para consultas síncronas.
- Ambos backends usan una única base física PostgreSQL conforme a la norma vigente. Cada adapter es dueño de su schema y migraciones; no consulta tablas de plataforma por SQL ni realiza joins entre dominios. Las migraciones se aplican coordinadamente.
- Redis solo se usa para estado técnico efímero con TTL si se justifica. Idempotencia financiera, autorización y resultado nunca dependen únicamente de Redis.
- Resultados y callbacks que afecten pantallas se conectan a realtime de plataforma mediante eventos durable versionados; el usuario recupera estado por REST autorizado de plataforma.

## Seguridad y observabilidad

Verificar protocolo oficial por proveedor; no asumir HMAC uniforme. Preservar bytes originales, usar comparación segura, tolerar/rechazar replay conforme al contrato, limitar tamaño, validar entorno/cuenta y no procesar payload de negocio antes de autenticar. Credenciales se cifran y rotan; no se devuelven a browser ni aparecen en logs.

Entre plataforma y este backend se exige TLS/mTLS con identidad de servicio verificable, certificados rotables y autorización por método y alcance. Cada RPC valida tenant, entorno, cuenta, operación, concesión firmada y límites del mensaje; aplica deadline, rate limit, idempotencia y reintentos acotados. La recuperación tras desconexión no duplica operaciones. Las solicitudes síncronas gRPC pueden ejecutar persistencia del dominio; Outbox/JetStream se reserva para hechos durables, flujos diferidos y replay. Los eventos contienen referencias opacas, nunca secretos.

Cada frontera HTTP/gRPC/SQL/Redis/NATS/proveedor emite eventos de inicio, dependencia, fallo clasificado y respuesta con `correlation_id`, `trace_id`, categoría/código estable, duración de dependencia/total y retryable solo por política confirmada. Telemetría OpenTelemetry/Sentry/métricas se redacta. Los errores externos se convierten en errores tipados internos y respuestas seguras.

## Estado

El crate actual es un scaffold Rust (`Cargo.toml`/`src/main.rs`); no implementa adapters, autorización, migraciones, webhooks ni workers. Las pasarelas no se consideran operativas hasta probar adapters reales y sandbox oficial con credenciales válidas.

## Roadmap y reglas

- [Roadmap específico de pasarelas](ROADMAP.md)
- [Reglas específicas de pasarelas](PROJECT_RULES.md)
- [Roadmap integrado de Payvia](../ROADMAP.md)
- [Reglas específicas del proyecto](../PROJECT_RULES.md)
- [Reglas de arquitectura](../ARCHITECTURE_RULES.md)
- Prompts de calidad y observabilidad incluidos en este directorio.

## Alineación de arquitectura

La topología de dos backends está definida en [las reglas de arquitectura](../ARCHITECTURE_RULES.md). Este backend conserva monolito modular interno, pero es un límite de despliegue separado del backend plataforma. Antes de habilitar tráfico real, cada integración debe completar sus contratos, concesión de autorización, migraciones coordinadas, observabilidad y aceptación cross-backend.
