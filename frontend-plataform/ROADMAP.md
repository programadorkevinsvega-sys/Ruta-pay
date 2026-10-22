# Roadmap — Frontend de Administración de Plataforma Payvia

> Aplicación web independiente para operadores de Payvia. Usa exclusivamente REST/OpenAPI y WebSocket autenticado del BFF del backend de plataforma.
>
> Estado inicial: planificación. Ninguna fase está completada por scaffolds o documentación.

## Alcance y reglas de ejecución

El frontend administra la plataforma Payvia desde la perspectiva de operador. No es dueño de tenant, plan, precio, entitlement, billing o autorización y no contiene reglas de negocio. El backend valida identidad, permisos, separación de roles, estado y alcance de cada operación y mantiene su auditoría.

Desde el diseño de cada fase/subfase se aplican todos los requisitos pertinentes establecidos por [AGENTS.md](../AGENTS.md), [ARCHITECTURE_RULES.md](../ARCHITECTURE_RULES.md), [PROJECT_RULES.md](../PROJECT_RULES.md) y los documentos normativos de ambos backends. Los requisitos no aplicables se justifican en la matriz antes de implementar; no se difieren calidad, seguridad, pruebas ni observabilidad.

La administración es una app independiente de `frontend-pasarela`, con sesión, caché, rutas y audiencia propia. No se conecta a PostgreSQL, Redis, NATS, gRPC, módulos internos, backend de pasarelas ni proveedores. Redis/NATS no son dependencias directas del navegador; los cambios durables llegan por APIs autorizadas y WebSocket, y se reparan con REST/cursor.

## Fase 0 — Auditoría, actores y contratos

- [ ] Inspeccionar el estado real de frontend, toolchain, CI, rutas, dependencias, pruebas y configuración.
- [ ] Definir actores y permisos: operador, roles especializados y privilegios de alto impacto; documentar separación respecto a sesión tenant.
- [ ] Mapear operaciones administrativas al OpenAPI real del BFF, errores tipados y eventos realtime pertinentes.
- [ ] Definir matriz por subfase para acceso, cambios persistidos, auditoría, WebSocket y recuperación REST/cursor.
- [ ] Definir controles UX para confirmación, doble control cuando política lo requiera, y prevención de errores de producción.

**Criterio de salida:** contratos existentes identificados, mapa rol→operación revisado, matriz de integración y amenazas definidas; no inventar API ausente.

## Fase 1 — Fundaciones seguras de la app

- [ ] Establecer estructura React/TypeScript y build reproducible sobre dependencias/versiones verificadas.
- [ ] Integrar cliente REST/OpenAPI versionado del BFF de plataforma, correlación de solicitudes y envelope de error seguro.
- [ ] Implementar autenticación/sesión y protección de rutas con validación server-side como autoridad.
- [ ] Mantener identidad, caché, query keys y WebSocket separados del frontend tenant.
- [ ] Aplicar protección contra XSS, CSRF según el mecanismo de sesión, redirecciones abiertas, filtración de secretos y configuración insegura.
- [ ] Configurar telemetría cliente minimizada/redactada y páginas accesibles para error, permiso denegado, sesión expirada y degradación.

**Criterio de salida:** pruebas cubren sesión, roles, denegación, expiración, redacción, CSP/almacenamiento y contratos reales.

## Fase 2 — Coordinación realtime y estado recuperable

- [ ] Implementar coordinador WebSocket central con audiencia de plataforma, cursor, `event_id`, versión y deduplicación.
- [ ] Detectar huecos y eventos fuera de orden; reconectar con backoff+jitter y recuperar snapshot/cambios por REST autorizado.
- [ ] Invalidar/refrescar TanStack Query desde un punto central; vistas no mantienen sockets particulares.
- [ ] Limpiar socket, caché y cursor al salir, expirar sesión o cambiar la identidad operativa.
- [ ] Mostrar estado de conexión y frescura de datos en áreas administrativas críticas.

**Criterio de salida:** pruebas automáticas verifican recuperación, autorización de audiencia, deduplicación, gaps y convergencia con REST; transporte real se valida junto con el backend.

## Fase 3 — Tenants, identidad y operación de cuentas

- [ ] Buscar/consultar tenants con filtros autorizados, detalle y auditoría de cambios.
- [ ] Flujos de verificación, activación, suspensión, reactivación y cierre según permisos backend.
- [ ] Visualizar usuarios, membresías, estado y acceso con minimización de datos personales.
- [ ] Presentar confirmaciones, razones y resultado persistido para acciones de impacto.
- [ ] Sincronizar cambios por realtime y recuperar por REST ante interrupciones.

**Criterio de salida:** UI/contratos cubren acceso cruzado, autorización denegada, conflictos de estado, auditoría y recuperación.

## Fase 4 — Catálogo, precios, límites y facturación

- [ ] Administrar planes y versiones inmutables de precio, moneda, periodo y fechas efectivas.
- [ ] Configurar límites, cuotas, capacidades y vigencias mediante comandos autorizados.
- [ ] Consultar uso, entitlement, estado de cuenta y explicación segura de denegaciones.
- [ ] Administrar suscripciones, invoices y operaciones de cobro permitidas, con confirmación explícita.
- [ ] Mostrar conciliación, intentos y errores de facturación según contratos reales, sin suponer estados.

**Criterio de salida:** UI refleja contratos versionados y resultados durables; pruebas comprueban permisos, límites, conflictos, errores y eventos realtime relacionados.

## Fase 5 — Autorización, proveedores y auditoría

- [ ] Consultar políticas y decisiones disponibles sin exponer secretos, tokens de concesión o detalles internos innecesarios.
- [ ] Consultar estado de conexión/capacidades de proveedor publicado por el backend de plataforma.
- [ ] Mostrar registro de auditoría con actor, acción, resultado y correlación según permisos.
- [ ] Proporcionar acciones operativas controladas (por ejemplo, revisión o reenvío autorizado) solo si existe contrato explícito.
- [ ] No habilitar operaciones de gateway mediante llamadas directas desde el navegador.

**Criterio de salida:** cada operación sensible tiene permiso backend, actor y auditoría; las capacidades externas son las efectivamente declaradas.

## Fase 6 — Calidad, accesibilidad y preparación operativa

- [ ] Pruebas unitarias, componentes, flujo UI, contratos OpenAPI y escenarios de roles.
- [ ] Auditoría de accesibilidad, teclado, contraste, semántica y foco en diálogos de confirmación.
- [ ] Pruebas de seguridad de cliente, dependencias, cabeceras, CSP y ausencia de exposición de credenciales.
- [ ] Verificar rendimiento, paginación, filtros, grandes historiales y manejo de APIs degradadas.
- [ ] Despliegue por ambiente, configuración pública segura, rollback y telemetría redactada.
- [ ] Registrar evidencia, comandos ejecutados, resultados, defectos y bloqueos.

**Criterio de salida:** flujos de operador críticos tienen aceptación backend real, seguridad revisada, accesibilidad y recuperación probadas; ningún pendiente se presenta como cerrado.

## Matriz de integración por subfase

| Frontera | Diseño requerido | Aceptación |
|---|---|---|
| Frontend → REST/OpenAPI | Solo BFF plataforma, contrato versionado y contexto de solicitud | Pruebas de contrato, autorización y errores reales |
| Identidad/roles | Sesión/audiencia de plataforma independiente; backend autoriza cada comando | Roles, escalamiento, revocación y sesión vencida |
| PostgreSQL | Sin acceso desde el cliente; ownership backend | Prueba backend en schema dueño y persistencia real |
| Redis | No hay cliente Redis en frontend; cualquier uso pertenece al backend y requiere TTL | Revisión de bundle/configuración y degradación de API |
| NATS/Outbox | No hay cliente NATS en frontend; eventos llegan mediante WebSocket BFF autorizado | Eventos durable/backend, deduplicación y recuperación REST |
| Realtime | Cursor/scope de plataforma, reconexión, refetch y borrado de sesión | Gap/replay/reconexión y convergencia verificadas |
| Auditoría/errores | Mutaciones sensibles auditables; mensajes cliente seguros | Evento operativo correlacionado, envelope seguro y redacción |
| Seguridad y calidad | Amenazas, casos negativos, accesibilidad y privacidad por pantalla | Suites automatizadas y reporte de evidencia real |

## Estado de fases

Todas las fases permanecen pendientes hasta que sus criterios se implementen y validen con evidencia. Una pantalla o navegación visual no sustituye operación autorizada, persistencia, auditoría, realtime ni pruebas de backend.
