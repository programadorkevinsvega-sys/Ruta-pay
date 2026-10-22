# ADR-0007: Destino de errores en Sentry

- Estado: Aceptada
- Fecha: 2026-10-22

## Contexto

El proyecto ya dispone de una instancia Sentry Cloud en `grupo-vega.sentry.io`. Mantener una distribución Sentry self-hosted local duplicaría operación y consumo de recursos sin aportar una necesidad del backend de plataforma.

## Decisión

El backend enviará eventos de error a un proyecto Sentry Cloud mediante `SENTRY_DSN`, inyectado como secreto independiente por servicio y entorno. El DSN no se guarda en Git, archivos versionados, logs ni respuestas. La integración mantiene `send_default_pii = false` y sanea el evento antes de transmitirlo. Si el DSN no está configurado, la aplicación sigue operando con logs JSON y OTLP disponibles; esto no cuenta como recepción Sentry validada.

La recepción y redacción se verifican contra el proyecto Cloud de plataforma con un evento controlado y un valor centinela no sensible. No se instala ni se ejecuta una instancia Sentry local como parte de este proyecto.

## Consecuencias y aceptación

- Cada entorno obtiene su DSN desde su gestor de secretos; las llaves y permisos se rotan por el mecanismo de Sentry.
- Las pruebas cubren que el evento llegue al proyecto esperado y que mensajes, breadcrumbs, request data y extras potencialmente sensibles se eliminen antes del envío.
- La fase que incluye Sentry no se cierra hasta verificar recepción real; si el DSN no está disponible, la fase queda abierta y se registra el bloqueo en su documento de aceptación.
- La pila local Compose no incluye Sentry: administra Collector, métricas, logs y trazas locales, mientras Sentry es un destino externo opcional.
