# ADR-0009: Identidad legal del tenant

- Estado: Aceptada
- Fecha: 2026-10-22
- Alcance: `tenant_registry` del backend de plataforma y contrato de onboarding tenant

## Contexto

Al registrar un comercio colombiano se debe conservar su identificación legal como NIT o cédula de ciudadanía (CC). El dato pertenece al comercio/tenant, no a la cuenta de acceso del usuario. El sistema ya separa ownership por módulos y entrega cambios visibles mediante Outbox, JetStream, journal y realtime; incluir el documento en esos contratos ampliaría innecesariamente su exposición.

## Decisión

- `tenant_registry` es el único dueño de `legal_identification_type` y `legal_identification_number`.
- El tipo admitido es `nit` o `cc`. El número se normaliza a dígitos ASCII, aceptando punto, guion y espacios como formato de entrada, y se valida con longitud de 5 a 20 dígitos.
- PostgreSQL conserva tipo/número con índice único por ambos campos. Una migración aditiva deja los registros anteriores intactos y permite que los tenants existentes aún no tengan el dato hasta una futura captura explícita.
- El BFF recibe el contrato público, lo pasa por gRPC a `tenant_registry` y no lee/escribe PostgreSQL.
- El documento no forma parte de `TenantResponse`, snapshots, Outbox, mensajes JetStream, journal realtime, WebSocket, logs ni errores. Las respuestas de conflicto son genéricas y no revelan si el tipo o número ya existe.
- Las comprobaciones del navegador mejoran el formulario; validación, normalización, unicidad y persistencia autoritativas se ejecutan en el backend.

## Consecuencias

- El onboarding solicita tipo y número además de los datos de cuenta/comercio.
- Los datos históricos permanecen válidos; los nuevos registros capturan la identificación legal requerida.
- Los cambios al contrato siguen el flujo HTTP/OpenAPI del BFF → gRPC → módulo dueño; las consultas SQL quedan dentro de `tenant_registry`.
- El dato se trata como PII: no se presenta en listados de comercio ni se propaga por eventos.
- Para cambiar tipos de documento, formato canónico o requisitos de unicidad se requiere revisar la validación, la migración y los consumidores bajo un nuevo ADR.

## Verificación

Pruebas del dominio validan tipo/formato y normalización; aceptación real verifica persistencia, unicidad y que el dato no aparece en respuesta/eventos/logs. La migración se aplica sobre una base que contiene tenants preexistentes.
