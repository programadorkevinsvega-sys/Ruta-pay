# ADR-0002: Autorización de operaciones y custodia de credenciales de pasarela

- Estado: Aceptada
- Fecha: 2026-10-22
- Alcance: plataforma ↔ backend-pasarela

## Contexto

Autenticar la conexión entre procesos no autoriza todas sus operaciones. Plataforma gobierna tenant, plan, cuota, permisos y límites; pasarelas ejecuta protocolos técnicos y guarda credenciales de cuentas.

## Decisión

La comunicación síncrona usa gRPC sobre TLS, con mTLS/identidad criptográfica de servicio rotatoria. Ambas partes autentican el peer y autorizan por identidad, método y scope. Plataforma decide antes de cada operación protegida y entrega una concesión breve, firmada y de uso único ligada a caller/audiencia, tenant, entorno, cuenta, operación, capacidades, monto/moneda cuando proceda, versión, `jti`, expiración e IDs de trazabilidad. Pasarelas verifica cada claim y deniega por defecto ante identidad, firma, scope, claim, deadline o versión inválidos; no hace callback RPC circular.

El payload se valida con tipos fuertes y límites de tamaño/rango. Se aplican deadlines, concurrencia y rate limits, retries acotados solo en errores/operaciones retryable, idempotencia y reconciliación ante resultado financiero incierto. Las credenciales tenant se aceptan en BFF plataforma, se autorizan y reenvían por gRPC mTLS al bounded context dueño `gateway_credentials` de pasarelas. Plataforma no persiste plaintext/ciphertext ni publica el secreto o grant completo en Outbox/JetStream. Pasarelas guarda solo ciphertext/versiones y devuelve referencia/resultado sanitizado.

## Consecuencias

- Identidad del servicio y autorización de operación son controles distintos.
- El formato, algoritmo, rotación/solapamiento de claves de grants y protección antireplay se detallan en el contrato versionado antes de Fase 5.
- La caída o respuesta incompleta de plataforma impide la operación.
- mTLS no sustituye autorización por método/scope ni la validación de concesión.

## Aceptación

Pruebas bilaterales reales deben cubrir certificado inválido/expirado/rotado, caller/método/scope no permitido, grant manipulado/expirado/reutilizado, request grande/malformado, rate limit, deadline/desconexión, recuperación e idempotencia; correlación y redacción se verifican en ambas partes.
