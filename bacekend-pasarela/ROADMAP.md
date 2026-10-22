# Roadmap del Backend de Pasarelas Payvia

## Flujo de ramas y revisión

`main` debe mantenerse funcional y verificable. Desarrolla funcionalidades, cambios arquitectónicos, correcciones complejas y modificaciones de seguridad en ramas descriptivas; presenta los cambios importantes en Pull Request con resumen, decisiones, verificaciones ejecutadas y riesgos conocidos. Los cambios triviales de documentación pueden hacerse en `main` cuando las protecciones lo permitan. No se permite force push ni reescribir historial compartido. No integres si fallan verificaciones obligatorias; los cambios sensibles requieren revisión manual.

> Este roadmap cubre adapters y protocolos de Wompi, Bold y proveedores que se incorporen en módulos independientes. Tenant, plan, precios, límites, suscripción, billing de Payvia y autorización pertenecen al [backend de plataforma](../bacekend-plataform/README.md).
>
> Estado: planificación. El crate actual es un scaffold, no una integración funcional.

## Autoridad y contrato con plataforma

El servicio de plataforma es la autoridad única sobre identidad del llamador, tenant, entorno, estado de cuenta, plan, límites, cuotas, scopes y autorización. Antes de ejecutar una operación protegida, plataforma envía por gRPC autenticado un comando con concesión firmada de corta duración ligada al tenant, entorno, cuenta, acción, importe/moneda cuando aplique y clave de idempotencia. Este backend verifica firma, alcance y claims; grant ausente/inválido/expirado significa fail closed. No mantiene copia de negocio de planes/permisos ni concede permiso desde una cache. La concesión viaja con el comando y evita callback RPC circular.

Las operaciones sincrónicas usan gRPC versionado con contexto tenant/entorno/cuenta/operación/importe-moneda cuando aplique, concesión de plataforma, identidad de servicio y trazabilidad W3C. La recepción de webhooks verifica primero protocolo y firma del proveedor; los resultados verificados se persisten y se entregan a plataforma con evento versionado Outbox→NATS JetStream. No se confirma como final un evento que no esté durable.

## Criterios obligatorios para cada fase y subfase

Antes de implementar, documentar requisito, contrato y matriz: schema/migración PostgreSQL del módulo proveedor; recorrido del llamador HTTP/OpenAPI→BFF de plataforma→gRPC; llamada de autorización plataforma→pasarela; Redis o justificación N/A; productores/consumidores NATS/Outbox; exposición del resultado a realtime y recuperación REST autorizada; clases de error, observabilidad, pruebas y sandbox. Cada componente no aplicable requiere razón concreta previa.

Cada fase/subfase termina completa antes de iniciar la siguiente. Probar adapters reales en sandbox oficial, con credenciales del entorno correspondiente; no utilizar mocks, fixtures, semillas ni respuestas simuladas para declarar aceptación de integración. Las pruebas unitarias no sustituyen integración/contratos con PostgreSQL, Redis/NATS reales aislados ni recorrido BFF→gRPC→adapter cuando aplique. Reinicios se ejecutan únicamente en servicios de desarrollo documentados.

Toda aceptación de error prueba diagnóstico específico y respuesta final con igual `correlation_id`, `trace_id` y código/categoría; probar redacción automática. Deben cubrirse validación, authz, conflicto/idempotencia, timeout/conectividad, persistencia y proveedor. No cerrar con credenciales sandbox faltantes: documentar bloqueo exacto.

## Fase 0 — Auditoría y diseño de integración

- [ ] Inspeccionar crate actual, configuración, dependencias, contratos y tooling.
- [ ] Definir por ADR contrato gRPC, concesión firmada, identidad de servicio, migraciones compartidas coordinadas, subjects/eventos y aceptación cross-backend.
- [ ] Definir contrato gRPC plataforma→pasarelas `ExecuteAuthorizedGatewayOperation`: concesión firmada ligada al comando, vigencia, identidad de servicio, errores y correlación.
- [ ] Definir contratos de evento Outbox/JetStream para callback verificado y resultado proveedor.
- [ ] Completar la estructura de crates/binarios de pasarelas dentro del workspace Cargo raíz, el registro de módulos y las reglas de independencia por proveedor.
- [ ] Crear threat model: credenciales, llamadas financieras, callbacks falsos/replay, tenant incorrecto, SSRF/egress y caída del authz.

**Salida:** contratos versionados, ownership, threat model, sandbox/credenciales y matrices de integración aprobados.

## Fase 1 — Runtime, gRPC y observabilidad

- [ ] Configuración tipada, timeouts, TLS/mTLS e identidad de servicio.
- [ ] Servidor/cliente gRPC instrumentados con interceptores/span por RPC y propagación de contexto.
- [ ] Errores internos tipados y mapeo de proveedor sin exponer detalles sensibles.
- [ ] OpenTelemetry, Sentry, métricas y logs JSON con redacción y cardinalidad limitada.
- [ ] Eventos de inicio, autorización, llamada externa, fallo clasificado y cierre con duración dependencia/total.
- [ ] Readiness que compruebe dependencias necesarias y apagado ordenado.

**Salida:** aceptación prueba propagación HTTP/BFF→gRPC→servicio pasarela, mTLS, caller permitido/denegado, certificado inválido/expirado, timeout/desconexión, error y cierre correlacionados, métricas y ausencia de secretos.

## Fase 2 — Contrato neutral y harness de conformidad

- [ ] Definir operaciones normalizadas (crear/consultar/cancelar/reembolsar únicamente cuando soporte real).
- [ ] Definir tipos de importe entero, moneda, método, estado, referencia, error y descriptor de capacidades.
- [ ] Definir trait interno y servicio gRPC versionado, con compatibilidad backward.
- [ ] Definir validación de límites/capacidades sin policy de plan o entitlement.
- [ ] Crear suite de conformidad reutilizable y pruebas contractuales de errores, cancelación y trazabilidad.

**Salida:** suite ejecutable contra implementaciones reales, contratos sin operaciones vacías y compatibilidad verificada.

## Fase 3 — Cuentas proveedor y secretos

- [ ] Crear `gateway_credentials` como módulo/crate dueño del schema/migraciones de credenciales tenant, scopeados por tenant, proveedor, cuenta y ambiente.
- [ ] Definir contratos/tipos por proveedor y comandos internos gRPC para almacenar, rotar, revocar, validar y entregar en memoria credenciales al adapter autorizado.
- [ ] Configurar API interna para altas/rotaciones solo mediante comandos autorizados de plataforma sobre mTLS; nunca exponer endpoints directos al frontend.
- [ ] Aplicar cifrado autenticado envelope con llave maestra en KMS/secret manager externo, versionado/rotación y metadata mínima; persistir ciphertext, nunca secretos en claro.
- [ ] No exponer secretos a frontend; plataforma conserva autorización de quién configura y para qué tenant.
- [ ] No incluir credenciales/ciphertext en eventos, logs, trazas, errores o respuestas; eventos de rotación solo contienen referencias opacas y estado/version.
- [ ] Estados de conexión y errores operativos tipados sin guardar credenciales.

**Salida:** operaciones autorizadas, aislamiento tenant/ambiente, cifrado/rotación, acceso mínimo del adapter y redacción probados con PostgreSQL/KMS reales; credenciales no aparecen en NATS ni frontend.

## Fase 4 — Adaptadores independientes

- [ ] `gateway_wompi`: contrato oficial, operaciones/capacidades verificadas, errores, timeouts y sandbox Colombia.
- [ ] `gateway_bold`: contrato oficial, operaciones/capacidades verificadas, errores, timeouts y sandbox Colombia.
- [ ] Un crate por proveedor futuro; no introducir ramas centrales por nombre.
- [ ] Validar cuenta/entorno/capacidad real antes de cada llamada externa.
- [ ] Reintentos solo para fallos definidos como retryable por proveedor y operación idempotente; resolver resultado incierto consultando/reconciliando.
- [ ] Registrar llamadas, latencia, resultado y dependencia sin payload sensible.

**Salida:** cada adapter real pasa suite común con sandbox oficial, credenciales apropiadas y evidencia de capacidades; no se declara soporte por scaffold.

## Fase 5 — Verificación de concesión de plataforma por operación

- [ ] Recibir cada operación protegida solo mediante comando autenticado de plataforma con concesión firmada.
- [ ] Propagar identity service, `correlation_id`, `trace_id`, versión y deadline.
- [ ] Validar firma, audience/servicio, tenant, entorno, cuenta, scope, importe/moneda, `jti`, vigencia y clave idempotente; ningún cache local suplanta authz.
- [ ] Validar mTLS e identidad de servicio, rotación/expiración de certificados, allowlist caller+método, tipos/tamaño/rangos y versión antes de producir cualquier efecto.
- [ ] Denegar de forma tipada si falta concesión, está alterada/expirada/reutilizada o no corresponde al comando.
- [ ] Aplicar deadlines, timeouts, rate limits y concurrencia acotada; retries finitos con backoff+jitter solo si la operación/error son recuperables e idempotentes.
- [ ] Proteger transporte y registrar diagnóstico correlacionado en ambos servicios; no registrar credenciales ni datos financieros sensibles.
- [ ] Scope administrativo específico y auditado para cobros de facturas de plataforma, sin privilegio comodín.

**Salida:** pruebas de contrato reales demuestran que no sale llamada financiera al proveedor sin grant válido y que el allow corresponde exactamente al tenant/acción/comando autorizados.

## Decisión de transporte entre backends

- gRPC autenticado es el canal request/response para decisiones, consultas y comandos que requieren respuesta inmediata. Un comando puede persistir en el módulo propietario y usar idempotencia/Outbox local; sincronía describe la interacción, no la ausencia de escritura.
- Outbox→NATS JetStream se usa para hechos ya persistidos, procesamiento diferido, retry durable, replay y desacoplamiento. No se elige NATS solo porque una operación sea sensible o compleja; secretos, concesiones completas y payloads crudos nunca se publican.
- Los eventos transportan ID estable, versión, audiencia/scope, referencias opacas y contexto de correlación. El consumidor deduplica, valida productor/tenant y ACKea después de persistir su efecto.
- Aceptación cross-backend cubre acceso indebido, firma/grant alterado/expirado/replay, certificado inválido y rotado, límite de tasa, tamaño inválido, deadline, timeout, desconexión, recuperación y ausencia de duplicados o secretos.

## Fase 6 — Webhooks entrantes y eventos verificados

- [ ] Endpoint HTTPS público con ruta/versionado por proveedor y tamaño limitado; conservar bytes crudos/headers exigidos.
- [ ] Implementar verificación criptográfica oficial individual, frescura/anti-replay y comparación segura antes de confiar en contenido.
- [ ] Registrar recepción técnica durable e idempotente, sin crear estado comercial autoritativo.
- [ ] Resolver vínculo de cuenta/tenant/entorno con autorización de plataforma después de verificar firma.
- [ ] Persistir evento normalizado en Outbox y publicar por JetStream; consumers durable/idempotentes y manejo de redelivery/DLQ.
- [ ] En fallo temporal de authz o mensajería, no descartar ni marcar como completado; aplicar política de retry/backoff del proveedor.

**Salida:** aceptación sandbox/vector oficial prueba firma válida, inválida, alterada, replay, duplicado, cuenta incorrecta, plataforma no disponible, persistencia y publicación durable.

## Fase 7 — Recuperación, resiliencia y operaciones

- [ ] Reconexión PostgreSQL/Redis/NATS y gRPC con backoff acotado; no bloquear Tokio.
- [ ] Redis `ConnectionManager`/PubSub resuscribe si se usa; JetStream reconstruye consumer durable y recupera mensajes sin duplicar efecto.
- [ ] Pruebas de reinicio solo contra infraestructura local de desarrollo documentada, sin contenedores nuevos ni producción.
- [ ] Runbooks de credenciales, caída authz, cambios de API proveedor, timeout incierto, replay y DLQ.
- [ ] Alertas para fallos por proveedor, firma inválida, authz deny/latencia, cola atrasada, webhook pendiente y saturación.

**Salida:** evidencia de reinicio/replay/recuperación con PostgreSQL, Redis/NATS reales y sandbox si aplica; no hay eventos confirmados perdidos.

## Fase 8 — Seguridad, calidad y lanzamiento

- [ ] Unitarias/property tests de parser, normalización, firma, idempotencia y estados técnicos.
- [ ] E2E HTTP de webhook→verificador→Outbox/NATS→consumidor plataforma y operación BFF→gRPC→authz→sandbox proveedor.
- [ ] Fuzzing de parsers/firmas, revisión de SSRF/egress y dependencias.
- [ ] Pruebas de carga/concurrencia, timeout, circuit breaking y saturación por proveedor.
- [ ] Confirmar contratos oficiales, cuenta elegible, monedas y métodos de cada integración.
- [ ] Publicar disponibilidad por capacidades probadas; documentar limitaciones y runbooks.

**Salida:** adapters listos para operar con evidencia de sandbox, aceptación cross-backend, errores/telemetría seguros, compatibilidad y resiliencia. Cualquier credencial o sandbox faltante bloquea solo el proveedor afectado y queda registrado.

## Matriz de integración por subfase

| Frontera | Decisión/diseño previo | Evidencia requerida |
|---|---|---|
| PostgreSQL/migraciones | schema dueño de adapter, Outbox y transacción | migración/constraints/aislamiento en DB real |
| BFF plataforma → gRPC pasarelas | caller, método, timeout, versión y contexto | aceptación cross-backend y compatibilidad |
| gRPC plataforma → pasarelas | mTLS, concesión ligada al comando, versión | concesión válida, alterada, expirada y scope mismatch |
| gateway_credentials | ciphertext PostgreSQL, KMS envelope, ACL de adapter | aislamiento, rotación, acceso indebido denegado y cero secretos en logs/eventos |
| Redis | uso TTL o justificación N/A | reconexión/re-suscripción y degradación |
| NATS JetStream/Outbox | evento, subject, durable consumer, dedup | ack/replay/restart sin perder idempotencia |
| Proveedor externo | contrato oficial, sandbox, credencial y capability | llamada real al sandbox, no mock |
| Webhook entrante | cuerpo crudo, firma, replay y persistencia | vectores oficiales/sandbox y eventos correlacionados |
| Realtime | evento a plataforma y recuperación REST autorizada | cursor/snapshot, orden y huecos |
| Errores/observabilidad | clases/códigos, redacción, eventos por frontera | fallo y cierre con IDs/código coincidentes |

## Registro de avance

Todas las fases están pendientes al crear este documento. Cada avance debe registrar commit, cambios, comandos realmente ejecutados, resultados, limitaciones y bloqueos. No iniciar una fase posterior mientras la fase actual tenga criterios pendientes.
