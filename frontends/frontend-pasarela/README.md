<div align="center">

# Ruta-pay

**Orquestador de pagos multi-pasarela en Rust.**

Ruta en el repositorio: `frontends/frontend-pasarela/`.
Aplicación web del comercio para operar pagos, links, credenciales e integraciones habilitadas.

![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-15+-blue?logo=postgresql)
![Redis](https://img.shields.io/badge/Redis-idempotencia-red?logo=redis)
![NATS](https://img.shields.io/badge/NATS-JetStream-green)
![React](https://img.shields.io/badge/React-TypeScript-61dafb?logo=react)
![Estado](https://img.shields.io/badge/estado-en%20desarrollo-yellow)

</div>

## Propósito

`frontend-pasarela` es la aplicación React + TypeScript para los comercios (tenants) que usan Payvia. El nombre identifica la experiencia de gestión de pagos y conexiones con pasarelas; **no significa que el navegador se conecte al backend de pasarelas**. La aplicación consume únicamente REST/OpenAPI y WebSocket autenticado del BFF del backend de plataforma.

El backend de plataforma autentica al usuario, autoriza cada acción y aplica las políticas del tenant. Si una operación requiere un proveedor, plataforma coordina por gRPC con el backend de pasarelas y devuelve al frontend únicamente datos autorizados y seguros.

## Funcionalidades objetivo

- Inicio de sesión, contexto de comercio y selección segura del entorno de prueba o producción.
- Resumen de actividad, consultas y detalle de payment intents y transacciones.
- Creación de links de pago con monto, moneda soportada, descripción y metadata permitida; presentación del QR generado por el backend.
- Administración de API Keys de prueba y producción: creación con visualización única, scopes y revocación, sin recuperar ni persistir el secreto en el navegador.
- Configuración de endpoints de webhook del comercio y consulta de entregas, resultados de intentos y acciones de reenvío autorizadas.
- Alta y configuración de cuentas de Wompi, Bold y proveedores habilitados mediante las operaciones seguras de la API de plataforma. El frontend no almacena credenciales de proveedor después de enviarlas ni las vuelve a mostrar.
- Estado de cuenta, plan, límites y uso visibles según los permisos del usuario.
- Actualización reactiva de pagos, configuración y entregas; ante desconexión o huecos, recuperación mediante REST autorizado.

Las capacidades se muestran según los descriptores y permisos entregados por la API. El frontend no promete monedas, métodos o acciones que el proveedor, cuenta o backend no haya habilitado.

## Arquitectura y flujo de pago

La app presenta la interfaz del comercio y consume solo el BFF REST/OpenAPI/WebSocket de plataforma. Plataforma valida usuario, tenant, API Key, plan, permisos y límites; si el pago requiere un proveedor, manda un comando gRPC con concesión de autorización al backend de pasarelas. Pasarelas llama al proveedor y publica el resultado verificado; plataforma persiste la transición de negocio y la entrega de webhook. El browser nunca llama a pasarelas o al proveedor.

## Requisitos e instalación

Requisitos verificados: Node.js 22.12 o superior y npm. La primera app ejecutable implementa login/logout de usuario de comercio y recuperación de sesión usando REST del BFF. Aún no ofrece onboarding de tenant, selección de tenant ni API Keys porque sus contratos backend no están disponibles.

```bash
npm ci
npm test
npm run build
npm run dev
```

En desarrollo, Vite reenvía `/v1` al BFF `http://localhost:8080` y adapta `Origin` únicamente en el proxy local para cumplir la validación same-origin del BFF. Configura `VITE_PLATFORM_BFF_URL` localmente si hace falta. Producción requiere enrutar `/v1` por HTTPS al BFF sin alterar `Origin`.

## Uso de la API

El frontend llamará a la API pública a través del BFF. La forma objetivo de crear un pago es `POST /v1/payment_intents`; un ejemplo de integración de servidor está en el [README principal](../../README.md#uso-de-la-api). La app no debe incrustar una API Key secreta de servidor ni hacer llamadas financieras privilegiadas desde el browser. El backend emite el link/QR final y la interfaz solo lo presenta.

El resultado usa un `payment_method` normalizado discriminado: `card`, `pse` o `crypto`, con detalles seguros específicos de ese tipo. El selector de método/cuenta en una solicitud es un contrato de entrada separado y se limitará a las capacidades publicadas por la API; no enviar detalles de liquidación o IDs de transacción del proveedor como datos de entrada. Revisa los ejemplos card/PSE/cripto y el contrato de webhook en el [README principal](../../README.md#webhooks-hacia-tu-servidor).

## Escuchar el estado en tiempo real

Un coordinador central mantendrá el WebSocket autenticado del BFF para la sesión y tenant activos. Los eventos autorizados por transacción actualizan TanStack Query; el cliente deduplica por `event_id`, controla cursor/versión, reconecta con backoff y pide snapshot REST para reparar eventos perdidos. La ruta y contrato de suscripción se documentarán cuando estén publicados. La app aún no abre una conexión real.

## Webhooks hacia tu servidor

El comercio configura su endpoint desde el dashboard. La entrega es responsabilidad del worker de plataforma, no del frontend ni del backend de adapters. Payvia hará un `POST` HTTPS con `X-Payvia-Signature: t=<timestamp>,v1=<hmac_sha256>` y `User-Agent: Payvia-Worker/1.0.0`; el [README principal](../../README.md#webhooks-hacia-tu-servidor) describe el payload propuesto.

La interfaz objetivo permite generar/rotar el secreto de endpoint con visualización única, inspeccionar entregas e intentos y solicitar reenvío si el usuario tiene permiso. No almacena el secreto y no marca una entrega exitosa por una señal visual: presenta el resultado durable comunicado por el backend. El flujo todavía no está implementado.

## Credenciales de tus pasarelas

El comercio puede registrar sus credenciales de Wompi, Bold u otra pasarela habilitada desde el dashboard. El formulario muestra únicamente los campos que requiere el adapter seleccionado y el entorno elegido. Al guardar, el secreto se envía una sola vez al BFF de plataforma; el backend verifica permisos y lo reenvía de forma cifrada al módulo `gateway_credentials` de pasarelas. El valor no se almacena en el estado del navegador después del envío ni se devuelve al consultar la cuenta. El dashboard muestra estado de configuración, fecha/versión de rotación y resultado de conexión, nunca la credencial.

La plataforma usa identidad de servicio y mTLS para comunicar el comando al backend de pasarelas. La autorización está ligada al tenant, cuenta, proveedor, entorno y operación. Esta UI no llama a la pasarela directamente y no almacena llaves permanentes.

## Límites de seguridad y arquitectura

- Aplicación independiente de `frontend-plataform`; sesiones, rutas, almacenamiento de caché y permisos no se comparten entre apps.
- Consume el BFF de plataforma; no llama a módulos Rust, endpoints gRPC, backend de pasarelas, proveedores, PostgreSQL, Redis ni NATS.
- No contiene reglas de negocio o decisiones de autorización. La validación del navegador mejora la experiencia, pero la API vuelve a validar todo.
- No incluye PAN, CVV ni formularios que capturen datos completos de tarjeta. El checkout usa las superficies autorizadas de los proveedores.
- No guarda API Keys, secretos, tokens de sesión ni credenciales de pasarela en `localStorage`, logs, telemetría o URLs. Las sesiones usan el mecanismo seguro definido por el BFF.
- WebSocket autenticado solo transporta señales autorizadas. La consulta REST sigue siendo la fuente para leer estado, recuperar cursores y corregir eventos perdidos.
- Errores se muestran con mensajes seguros, accionables y referencia de soporte; nunca se presenta SQL, stack trace, secretos ni contenido sensible.

## Stack de interfaz objetivo

React 19, TypeScript 7, Vite 8 y TanStack Query 5 con versiones fijadas en `package-lock.json`. Vitest cubre la forma segura de los mensajes de error. La UI actual se limita al primer flujo de identidad.

## Estado

En implementación. Login/logout y recuperación de sesión compilan y tienen pruebas unitarias. Onboarding/membresías, WebSocket realtime y recuperación REST por cursor siguen pendientes y no se presentan como funcionalidad disponible.

## Documentación relacionada

- [Roadmap de frontend-pasarela](ROADMAP.md)
- [Backend de plataforma y su API](../../bacekend-plataform/README.md)
- [Backend técnico de pasarelas](../../bacekend-pasarela/README.md)
- [Arquitectura del producto](../../ARCHITECTURE_RULES.md)
- [Reglas específicas del proyecto](../../PROJECT_RULES.md)
- [README principal](../../README.md)
