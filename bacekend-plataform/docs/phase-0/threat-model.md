# Modelo de amenazas — plataforma ↔ pasarelas

Alcance: decisiones y límites de confianza de Fase 0. No es una evaluación de seguridad de un runtime implementado. La probabilidad se reevaluará al desplegar; no se inventan garantías criptográficas.

| Amenaza / actor | Impacto | Control arquitectónico requerido | Evidencia de aceptación futura |
|---|---|---|---|
| Servicio no autorizado invoca RPC de pasarelas | Operación proveedor/cobro no permitida | Identidad criptográfica mTLS y autorización por método/scope/caller; IP/header no bastan. | Certificado/caller inválido rechazado; scopes permitidos y denegados comprobados. |
| Servicio autenticado modifica tenant, monto, cuenta u operación | Cobro o acceso cruzado | Grant firmado corto, audiencia, claims ligados a comando, tipos/rangos fuertes; verificación independiente de pasarelas. | Alterar cada claim invalida; identidad, traza y operación coinciden. |
| Robo o replay de grant | Repetición de efecto financiero | `jti` único, expiración, enlace al comando e idempotencia persistente; alcance/almacenamiento del replay control definido. | Reusar grant se rechaza sin efecto duplicado. |
| Certificado vencido/comprometido o rotación parcial | Interrupción o suplantación | Emisión/rotación con solapamiento acotado, revocación, identidad mínima y auditoría; formato por definir en ADR de implementación. | Certificado inválido/expirado rechazado y rotación durante operación compatible. |
| Plataforma indisponible o autorización incompleta | Operación no autorizada o bloqueo | Fail closed ante timeout, respuesta incompleta, revocación no comprobable o versión incompatible; deadline finito. | Timeout/desconexión produce deny seguro y evento correlacionado. |
| Secretos de gateway filtrados desde plataforma | Compromiso de cuentas de proveedor | `gateway_credentials` dueño en pasarelas; plataforma reenvía por canal cifrado y no persiste ni publica secretos. Redacción en cada frontera. | Inspección de DB/eventos/logs/traces/browser no encuentra secreto; adapter autorizado lo usa en memoria. |
| Payload malicioso/grande o abuso RPC | Agotamiento, parser exploit, bypass de validación | Tipos fuertes, límite de bytes/campos, validación, timeouts, rate/concurrency limits. | Mensaje inválido/grande se rechaza antes de efecto y deja diagnóstico seguro. |
| Reintento tras timeout con resultado incierto | Pago duplicado | Retry solo con clasificación retryable; idempotencia y reconciliación antes de repetir efecto financiero. | Desconexión en cada punto no duplica operación; resultado incierto es recuperable. |
| PostgreSQL compartido y consultas cross-schema | Fuga de datos entre módulos/tenants | Ownership por módulo/schema, privilegios mínimos, no SQL/JOIN cruzado; tenant scope validado por el dueño. | Manipulación de IDs y pruebas con tenants distintos contra PostgreSQL real. |
| Evento falso, duplicado, desordenado o con secreto | Transición fraudulenta, duplicidad/fuga | Producer autenticado, contrato versionado, Outbox, validación de fuente/asociación, deduplicación durable, sin secretos en eventos. | Publicación no permitida, replay, redelivery y evento inválido rechazados/idempotentes. |
| Pérdida de señal realtime o Redis | UI obsoleta o falsa fuente de verdad | PostgreSQL/journal durable + JetStream replay; Redis Pub/Sub solo fan-out y frontend repara con cursor/snapshot REST autorizado. | Reinicios/reconexión reales, hueco detectado y reparación desde fuente autorizada. |
| Fuga por logs/Sentry/métricas | Exposición de credenciales, datos personales o pagos | Allowlist de campos, redacción, cardinalidad controlada, errores cliente seguros, telemetría autenticada. | Fallo de frontera correlacionado sin secreto en logs/exportadores. |
| Robo de credenciales Wompi de Payvia o falsificación/replay de callback | Cobros indebidos, alteración de invoices o filtración de cuenta Payvia | Credenciales solo en configuración protegida; cuenta de plataforma separada de tenants; verificación oficial de callback, timestamp/replay si el protocolo lo soporta, deduplicación durable e idempotencia. | Pruebas sandbox de firma inválida, cuerpo alterado, callback repetido, ambiente/cuenta incorrectos y rotación de credenciales. |
| Robo/filtración de token OAuth o credenciales de Factus | Emisión no autorizada, acceso a documentos fiscales o indisponibilidad | Secretos fuera de PostgreSQL/logs; tokens temporales con expiración; scopes mínimos, renovación segura y redacción de documentos/PII. | Token vencido/inválido, renovación, revocación y recuperación; auditoría sin contenido fiscal sensible. |
| Repetición o duplicación de emisión Factus | Documentos fiscales duplicados o divergentes del invoice de Payvia | Referencia externa idempotente, estado fiscal separado del ledger financiero y reconciliación antes de reintentar resultados inciertos. | Repetir solicitud con misma referencia no duplica emisión; timeout recupera estado desde Factus. |
| Abuso de API key o webhook de Resend | Envío de correo no autorizado, duplicado o fuga de datos personales | API key protegida; alcance de envío mínimo; Outbox durable, clave de idempotencia, límites/reintentos; validar firma del webhook sobre cuerpo original si se habilitan webhooks. | Key inválida/revocada, replay, idempotencia, redacción de destinatarios/contenido y webhook con firma incorrecta. |
| Retirar un proveedor rompe la lógica financiera/documental | Pérdida de capacidad de reconciliar historial o cambio accidental de negocio | Contratos neutrales y adapters registrados por composición; preservar identificadores, estado, ledger y auditoría históricos al desactivar. | Prueba de conformidad de adapter y verificación de que la baja solo bloquea operaciones nuevas, manteniendo consultas/reconciliación histórica. |

## Límites de confianza

1. El frontend/browser es no confiable; no puede fijar tenant, permiso, capacidad ni estado de autorización.
2. El BFF es borde REST de plataforma, no dueño de negocio ni cliente directo de stores.
3. Cada módulo valida identidad, autorización y estado de su dominio aunque el request venga de otro módulo autenticado.
4. El canal cifrado protege tránsito, no garantiza autorización ni elimina riesgo de compromiso del proceso/host.
5. PostgreSQL puede almacenar ciphertext de credenciales solo en el módulo dueño de pasarelas; la DB física compartida no elimina aislamiento lógico ni controles de permisos.
6. Ningún control de arquitectura garantiza protección frente al compromiso total del host o memoria del proceso autorizado; esa limitación debe declararse al diseñar la custodia criptográfica.

## Riesgos y bloqueos no resueltos en Fase 0

- Algoritmo/formato del grant, custodia y rotación de sus claves, antireplay entre réplicas, esquema PKI y política de revocación necesitan contratos implementables.
- No hay infraestructura local ni credenciales de proveedor para probar conectividad real. Esto no bloquea el diseño, pero bloquea cualquier declaración de integración.
- No existe runtime para demostrar redacción, trazas, denegación o resiliencia; estas pruebas no se marcan como aprobadas.
