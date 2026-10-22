<div align="center">

# Ruta-pay

**Orquestador de pagos multi-pasarela en Rust.**

Ruta en el repositorio: `frontends/frontend-plataform/`.
Aplicación de administración de la plataforma SaaS Payvia.

![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-15+-blue?logo=postgresql)
![Redis](https://img.shields.io/badge/Redis-idempotencia-red?logo=redis)
![NATS](https://img.shields.io/badge/NATS-JetStream-green)
![React](https://img.shields.io/badge/React-TypeScript-61dafb?logo=react)
![Estado](https://img.shields.io/badge/estado-en%20desarrollo-yellow)

</div>

## Propósito

`frontend-plataform` es la aplicación independiente React + TypeScript para operadores autorizados de Payvia. Permite administrar el ciclo de vida de tenants, catálogo y precios, límites/cuotas, suscripciones, facturación de la plataforma, políticas de autorización y auditoría.

Esta interfaz no es el backend de plataforma: consume exclusivamente REST/OpenAPI y WebSocket autenticado del BFF de plataforma. Las decisiones, permisos, cambios de estado y datos persistentes pertenecen al backend.

## Funcionalidades objetivo

- Inicio de sesión de operador, autenticación reforzada según política, sesiones y permisos separados de los comercios.
- Registro, verificación, activación, suspensión y cierre auditado de tenants.
- Administración versionada de planes, precios, moneda, límites, cuotas, periodos y vigencias.
- Consulta y operación autorizada de suscripciones, facturas, cobros de uso, estados de cuenta y conciliación.
- Visualización de consumo y estado de autorización con controles explícitos para acciones de alto impacto.
- Auditoría consultable con filtros autorizados y datos sensibles minimizados.
- Paneles operativos de errores, webhooks y estado de proveedores que exponga la API de plataforma; el navegador no consulta directamente pasarelas o infraestructura.
- Actualizaciones realtime de cambios visibles, con recuperación por API REST si se interrumpe la conexión.

## Arquitectura y flujo administrativo

La consola es un cliente sin autoridad de negocio. Envía requests REST/OpenAPI al BFF de plataforma, que autentica al operador y los módulos validan roles, scopes y estado actual. El módulo dueño persiste el cambio y su Outbox; eventos autorizados llegan al WebSocket y el coordinador frontend invalida/refresca TanStack Query. Ante desconexión, se recupera por REST. La consola nunca llama directamente a módulos, gRPC, base de datos, mensajería, backend de pasarelas o proveedores.

## Requisitos e instalación

Stack objetivo: Node.js con versión fijada, React, TypeScript, Vite y TanStack Query. La app requiere acceso al BFF de desarrollo y una identidad de operador provisionada de forma segura.

Esta carpeta aún no tiene `package.json`, lockfile ni aplicación ejecutable. Por lo tanto no hay comandos de instalación, desarrollo o build que puedan afirmarse como verificados. Las versiones, variables públicas y comandos se añadirán cuando exista el scaffold funcional.

## Uso de APIs administrativas

La consola usa únicamente endpoints administrativos publicados por OpenAPI del BFF de plataforma. Los recursos objetivo incluyen tenants, catálogo/versiones de precios, límites, suscripciones, facturas, uso y auditoría. No se debe construir un endpoint por intuición: rutas, request/response y permisos deben provenir del contrato publicado.

Las operaciones de impacto requieren mostrar objetivo y efecto antes de confirmar. La autorización y auditoría se vuelven a validar en backend; esconder botones o confiar en roles guardados en el browser nunca sustituye esos controles.

## Tiempo real y monitoreo de webhooks

La app escuchará señales de plataforma por un WebSocket autenticado con audiencia de administración. Puede presentar el estado operacional y entregas de webhooks a comercios que el operador esté autorizado a consultar; no recibe la credencial ni el secreto del comercio. Los detalles se recuperan por REST/cursor para reconciliar huecos. La ruta WebSocket y los recursos de soporte se definirán en el contrato backend; todavía no están disponibles.

Los webhooks de negocio a los servidores de comercios los genera plataforma con firma `X-Payvia-Signature` y `User-Agent: Payvia-Worker/1.0.0`. Su payload y controles de verificación están descritos en el [README principal](../../README.md#webhooks-hacia-tu-servidor). Esta consola solo ofrece supervisión autorizada, no despacho desde el navegador.

## Controles de comunicación inter-backend

Las operaciones de consola no acceden a pasarelas directamente. Plataforma autentica y autoriza al operador y, cuando requiere una acción técnica, se comunica por gRPC con identidad de servicio TLS/mTLS, permisos por método/scope y concesión firmada ligada a tenant/cuenta/operación. Errores, expiraciones, límites y estados se presentan de forma segura; la UI nunca recibe grants ni credenciales.

## Límites de seguridad y arquitectura

- Aplicación separada de `frontend-pasarela`: identidad, rutas, caché, contexto y navegación de administración no se comparten con tenants.
- Consume solo endpoints REST/OpenAPI y WebSocket autenticado del BFF de plataforma; no accede a módulos, gRPC, PostgreSQL, Redis, NATS, APIs de proveedores o backend de pasarelas.
- El backend vuelve a autenticar y autorizar cada acción. Ocultar un botón no constituye un control de acceso.
- Operaciones administrativas sensibles muestran alcance y efecto antes de enviar; el backend registra actor, decisión y auditoría.
- No se guarda credencial de proveedor, API Key, token, cookie ni dato sensible en `localStorage`, telemetría o URL.
- Errores de infraestructura o autorización se presentan de manera clara sin revelar SQL, stack traces, secretos o políticas internas.

## Stack y estado

Stack objetivo: React, TypeScript, Vite, cliente API alineado con OpenAPI, TanStack Query y coordinador realtime central. Las versiones y comandos se publicarán cuando existan y hayan sido verificados.

Planificación. No se considera implementada una operación administrativa por existir únicamente una vista o componente: debe contar con contrato real, autorización, auditoría, estados de error, sincronización y pruebas correspondientes.

## Documentación relacionada

- [Roadmap de frontend-plataform](ROADMAP.md)
- [Backend de plataforma](../../bacekend-plataform/README.md)
- [Backend de pasarelas](../../bacekend-pasarela/README.md)
- [Arquitectura del producto](../../ARCHITECTURE_RULES.md)
- [Reglas específicas del proyecto](../../PROJECT_RULES.md)
- [README principal](../../README.md)
