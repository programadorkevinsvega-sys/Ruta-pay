# Roadmap — Frontend de Comercios y Pasarelas Payvia

> Esta aplicación es la experiencia del comercio para gestionar cobros e integraciones. Usa exclusivamente REST/OpenAPI y WebSocket autenticado del BFF de plataforma. No se conecta directamente al backend de pasarelas.

Ruta en el repositorio: `frontends/frontend-pasarela/`.
>
> Estado actualizado: la coordinación realtime de Fase 2 está implementada y aceptada contra el BFF real; las fases de operación de pagos siguen pendientes.

La configuración local del cliente vive en `frontends/frontend-pasarela/.env` y se documenta en su `.env.example`; no se almacena en los `.env` de los backends.

## Alcance y reglas de ejecución

El frontend es una aplicación independiente React + TypeScript para usuarios tenant. Plataforma mantiene la autoridad de identidad, autorización, aislamiento tenant, límites, estado de pagos y credenciales. La UI solo presenta esos datos y solicita operaciones al BFF. La configuración de un proveedor se enruta mediante plataforma hacia el módulo de proveedor autorizado; el browser nunca accede a gRPC o a la API del proveedor.

Desde el primer día, cada fase/subfase cumple los requisitos aplicables de calidad, seguridad, observabilidad, arquitectura modular y aceptación definidos en [AGENTS.md](../../AGENTS.md), [ARCHITECTURE_RULES.md](../../ARCHITECTURE_RULES.md), [PROJECT_RULES.md](../../PROJECT_RULES.md) y los documentos normativos de ambos backends. Todo requisito no aplicable debe tener justificación concreta y verificable registrada antes de implementar la fase/subfase.

No se introducen accesos directos del navegador a PostgreSQL, Redis, NATS, gRPC, módulos internos, almacenamiento de secretos ni proveedores. Redis y NATS son N/A como dependencias directas del frontend: el cliente solo usa los contratos públicos del BFF. La distribución durable y la recuperación del estado son responsabilidad de backend; el frontend implementa WebSocket autenticado, cursores, reconexión, deduplicación y recuperación REST según el contrato. Cada subfase documenta y verifica esta matriz; no se asume N/A para la API o realtime.

## Fase 0 — Auditoría y contratos de experiencia

- [ ] Inventariar estructura real del frontend, toolchain, CI, rutas, dependencias y estado de pruebas.
- [ ] Mapear actores, permisos tenant, entornos `test`/`live`, estados, errores y capacidades proveedor devueltos por OpenAPI.
- [ ] Definir navegación, límites de sesión y estrategia de accesibilidad, privacidad y soporte.
- [ ] Crear matriz por subfase: pantalla/acción, endpoint OpenAPI, permisos, errores, cambios persistidos, evento realtime y recuperación REST/cursor.
- [ ] Identificar bloqueos de contrato con backend; no crear endpoints hipotéticos ni simular comportamiento.

**Criterio de salida:** inventario verificable, flujos UX aprobados, matriz de contratos y dependencias reales, amenazas del cliente y criterios automatizados definidos.

## Fase 1 — Fundaciones de aplicación, API y seguridad

- [ ] Configurar aplicación tipada, build reproducible, rutas, composición, lint/formato y pruebas según toolchain elegido y verificado.
- [ ] Integrar el cliente REST con OpenAPI versionado del BFF de plataforma, propagación de correlación y manejo seguro de errores.
- [ ] Implementar sesión, guardas de ruta y contexto tenant desde identidad confirmada por backend; no confiar en tenant IDs suministrados por UI.
- [ ] Separar selección de entorno y marcar claramente `test`/`live`; prevenir acciones de producción accidentales.
- [ ] Definir telemetría cliente con redacción, consentimiento/política aplicable, límites de cardinalidad y sin tokens, secretos, PII innecesaria o payloads de pago.
- [ ] Añadir componentes base accesibles, navegación, estados de carga/vacío/error, expiración de sesión y recuperación.

**Criterio de salida:** cliente integrado con contrato real del BFF; pruebas verifican autorización visual, expiración, errores/envelopes, redacción y ausencia de secretos persistidos.

## Fase 2 — Realtime y sincronización recuperable

- [x] Implementar coordinador WebSocket central autenticado, con audiencias tenant, `event_id`, cursor, versión de agregado, deduplicación y orden.
- [x] Manejar desconexión con backoff+jitter, reconexión, eventos desconocidos, huecos, revocación de sesión y cambio de tenant.
- [x] Coordinar invalidación/refetch de TanStack Query; no duplicar sockets, polling ni lógica de eventos dentro de cada vista.
- [x] Recuperar estado autorizado por REST con cursor/snapshot; mostrar indicador de sincronización y permitir reintento manual.
- [x] Verificar que caché, query keys, cursor y socket se limpien al cerrar sesión o cambiar de tenant.

**Criterio de salida:** pruebas automatizadas de cliente y contratos validan deduplicación, eventos fuera de orden, gaps, reconexión y convergencia con REST; aceptación de backend verifica el transporte real y sus permisos.

## Fase 3 — Operación de pagos y links/QR

- [ ] Panel tenant con transacciones/payment intents consultados por REST y filtros autorizados.
- [ ] Detalle de pago con monto/unidad menor, moneda, estado, referencia, método y proveedor solo según contrato permitido.
- [ ] Formulario de link con validación de presentación; el backend valida monto, moneda, límites, permisos e idempotencia.
- [ ] Renderizar QR y enlace exclusivamente desde la respuesta validada del backend; cubrir estados de expiración y error.
- [ ] Mostrar cambios recientes por realtime y corregir estado por REST tras reconexión.

**Criterio de salida:** flujos UI completos enlazados a endpoints reales y sus pruebas de aceptación; cobertura de permisos tenant, doble envío, conflictos, errores y actualización/reactivación.

## Fase 4 — Credenciales, webhooks e integraciones

- [ ] Administrar API Keys test/live con creación y visualización del secreto una sola vez, scopes explícitos, copia segura y revocación.
- [ ] Configurar URL de webhook del comercio y consultar eventos, intentos, fallos y reenvíos que permita la API.
- [ ] Configurar cuentas de proveedor a través de plataforma; enviar secretos en transporte seguro y descartarlos de estado, logs y analítica al terminar.
- [ ] Mostrar capacidades disponibles usando datos del backend, sin inferir métodos/monedas ni marcar una cuenta como conectada sin comprobación real.
- [ ] Asegurar prevención SSRF y validación de destino en backend; frontend no reemplaza esas protecciones.

**Criterio de salida:** aceptación verifica flujos backend→BFF reales, scopes, revocación, visualización única, errores por proveedor y redacción de secretos; sandbox se valida en los backends correspondientes.

## Fase 5 — Cuenta, uso, ayuda y accesibilidad

- [ ] Mostrar plan, límites, consumo, estado de cuenta, facturas y eventos de uso autorizados.
- [ ] Explicar de forma segura límite alcanzado, cuenta suspendida, autorización denegada y acción de recuperación disponible.
- [ ] Completar navegación por teclado, etiquetas, semántica, contraste, escalado y estados accesibles.
- [ ] Añadir ayuda contextual y referencias de solicitud para soporte sin exponer diagnósticos internos.

**Criterio de salida:** auditoría de accesibilidad definida y pruebas, permisos tenant y estados de error/sincronización cubiertos.

## Fase 6 — Calidad, seguridad y lanzamiento

- [ ] Unitarias/componentes, flujos end-to-end de UI, contratos OpenAPI, accesibilidad y pruebas de seguridad del cliente.
- [ ] Verificar XSS, CSRF conforme a sesión, navegación/redirección, dependencias, CSP, almacenamiento y filtración de secretos.
- [ ] Medir rendimiento de carga/rutas, tamaño de bundles, límites de WebSocket y comportamiento ante BFF degradado.
- [ ] Validar despliegue por entorno, configuración pública/privada, rollback, sourcemaps protegidos y monitoreo sin PII innecesaria.
- [ ] Documentar comandos efectivamente probados, criterios cumplidos, defectos y bloqueos.

**Criterio de salida:** ningún requisito o prueba pendiente se reporta como completo; lanzamiento sujeto a aceptación de backend real, seguridad, calidad, accesibilidad y operación.

## Matriz de integración requerida por subfase

| Área | Decisión/diseño obligatorio | Verificación |
|---|---|---|
| REST/OpenAPI | Cliente consume solo contrato versionado del BFF de plataforma | Contrato generado/validado y pruebas contra endpoints reales |
| Autorización/tenant | Backend decide acceso; UI refleja permisos sin confiar en estado local | Pruebas de roles, cambio de tenant, expiración y denegación |
| PostgreSQL | No existe acceso del browser; el módulo dueño persiste | Aceptación backend con migración/DB real según fase del backend |
| Redis | N/A directo en browser; posible backend solo con uso documentado | El frontend no incluye cliente ni secretos Redis |
| NATS/Outbox | N/A directo en browser; estado durable detrás de contratos del BFF | Evento real llega por WebSocket autenticado; recovery REST repara gaps |
| Realtime | WebSocket autenticado del BFF, cursor, orden, dedup, reconexión y snapshot | Pruebas de huecos, reconexión, sesión/tenant y convergencia REST |
| Credenciales | Nunca persistir secretos o credenciales en browser/logs/URLs | Pruebas de redacción y almacenamiento; flujo backend seguro |
| Errores/telemetría | Mensajes accionables sin datos internos; correlación con solicitud | Verificación automática de redacción y referencia segura |

## Estado de fases

Fase 2 de realtime y sincronización recuperable está verificada como parte de la integración vertical del backend de plataforma: Playwright recorrió onboarding, membership propietaria, sesión y WebSocket autenticado contra el BFF real; los detalles están en [`bacekend-plataform/docs/phase-2/acceptance.md`](../../bacekend-plataform/docs/phase-2/acceptance.md). Pagos e integraciones del tenant siguen en fases posteriores.
