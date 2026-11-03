# Estado verificado de Ruta-pay

**Corte:** 2026-11-03  |  **Rama:** main  |  **HEAD:** 1efe6e0be4099587bf4f6b510dadede320daea86

Este documento consolida el estado por componente a partir de las PR fusionadas y las ejecuciones CI consultadas. No reemplaza los roadmaps ni las actas de aceptación.

## Numeración de fases

Ruta-pay tiene roadmaps separados para el producto completo, backend de plataforma, backend de pasarelas y dos frontends. La numeración es local a cada roadmap. La Fase 3 de plataforma (catálogo y entitlements) no equivale a la Fase 3 global de pagos ni a una fase del backend de pasarelas.

Una PR fusionada o CI verde demuestra integración/verificación del SHA correspondiente; no cierra automáticamente todos los criterios del roadmap. Los pendientes de las auditorías permanecen abiertos hasta contar con evidencia.

## Estado por componente

| Componente | Estado en main | Límite de la afirmación |
|---|---|---|
| Backend de plataforma | Implementaciones de Fases 0–3 integradas. Fases 2–3 cuentan con ampliación de aceptación/CI. Fases 0–1 conservan huecos explícitos de auditoría. | No significa DoD total de Fases 0–1 ni backend terminado. |
| Backend de pasarelas | No hay fase propia completa certificada como cerrada. PR #7 añadió un cliente gRPC tipado de entitlements. | No cierra vault de credenciales, adapters, webhooks ni enforcement mTLS/grants. |
| Frontend de comercios | Slice realtime/onboarding de la integración vertical de plataforma probado contra BFF real. | No cierra todas las fases propias; las tareas de fundación/aceptación conservan pendientes. |
| Frontend de administración | Slice realtime y operaciones de tenant probadas contra BFF real. | No cierra todas las fases propias; las tareas de fundación/aceptación conservan pendientes. |
| Producto completo | En desarrollo; las fases globales del roadmap raíz no están certificadas como completas. | No se marcan completas por sumar fases locales de plataforma. |

## Fases del backend de plataforma

| Fase | Dominio | Estado y evidencia |
|---|---|---|
| 0 | Auditoría, decisiones, workspace y bounded contexts | Alcance documental integrado. PR #3 y CI 41579115662 exitosos. Quedan pendientes trazabilidad automatizada amenaza→control→test, consistencia semántica y revalidación reproducible del HEAD vigente. |
| 1 | Runtime, BFF, gRPC y observabilidad fundacional | Implementación y pruebas ampliadas integradas. PR #4 y CI 41578015530 exitosos. Quedan pendientes matriz completa de configuración/readiness, DSN Sentry, APP_VERSION y pruebas repetibles Collector/shutdown. |
| 2 | Identidad, sesiones, tenant registry, API Keys, aislamiento y realtime | Implementación PR #2 y ampliación PR #5 integradas. CI 41237702253 exitoso sobre HEAD de PR #5; incluye Rust, ambos frontends y Playwright E2E. |
| 3 | Catálogo, precios, límites, cuotas y entitlements | Implementación integrada mediante PR #7; CI 41319415556 exitoso sobre su HEAD exacto. La aceptación formal sigue abierta hasta conciliar los criterios 1–7 en un único HEAD final, conforme al acta de aceptación. No incluye despliegue productivo ni enforcement mTLS/grants de pasarelas. |
| 4–9 | Billing, pagos, autorización cross-backend, hardening y operación | Pendientes. No avanzar mientras existan criterios pendientes de fases anteriores. |

## PR y evidencia CI

Todas las PR listadas están cerradas con merged=true; cada run indicado terminó completed/success sobre el HEAD de la PR.

| PR | Entrega | HEAD de PR | CI | Merge commit |
|---|---|---|---|---|
| #2 | Fase 2 identidad/tenants inicial | 3050a9e260de02c1edb8b48b63d595bfada2f397 | [41196070301](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41196070301) | bc94a54a6dd1ae0da217cb4c10ebeec2a24c7123 |
| #3 | Auditoría documental Fase 0 | 7b08022da0d796b1890c7e452776847f84b7239b | [41579115662](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41579115662) | 1efe6e0be4099587bf4f6b510dadede320daea86 |
| #4 | Límites/runtime Fase 1 | db6e822e10f048ae918435aa8754322b685283df | [41578015530](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41578015530) | de3cdc9e3daf43f6270f6253314c88bd37e9b110 |
| #5 | Duplicidad/concurrencia y suites frontend/E2E Fase 2 | 45cb77502ef96009750177f184ea241f3792817f | [41237702253](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41237702253) | 4a69f5d7e62bfedc9a4400ef5b180cfd29c4d233 |
| #6 | Política obligatoria de pruebas por caso de uso | 7f28ae3738c0956d0ac1d592eb00fa6508a33de9 | [41577445274](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41577445274) | c0a6e28ab973e6f2ce93b808585475b9b80b320f |
| #7 | Catálogo y entitlements — Fase 3 plataforma | 67489f2e9cb689b1ad3356853e1ce23dc8c96f18 | [41319415556](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41319415556) | a73b595d58a59996c39709338e39a2ca77a97a63 |
| #8 | Máximo 400 líneas Rust; 200 deseable | 15b9bc6fa43e7b82b7b4ffe4ca2b352fbe3c1f31 | [41574748701](https://github.com/programadorkevinsvega-sys/Ruta-pay/actions/runs/41574748701) | d4f6da3e97c89c87f5fee276e73728ec30be5db0 |

PR #6 hace obligatoria la matriz de escenarios por caso de uso en todas las fases/subfases. PR #8 establece el máximo absoluto de 400 líneas físicas por archivo Rust, con 200 como objetivo preferido cuando la cohesión permita dividir el código naturalmente.

## Bloqueos y siguiente puerta

1. Cerrar trazabilidad amenaza→control→caso→test→CI, consistencia semántica y reproducibilidad en la auditoría de Fase 0.
2. Completar matriz de configuración/readiness y escenarios repetibles de Fase 1.
3. Solo después de cerrar y evidenciar esas brechas, reevaluar el inicio de Fase 4 de plataforma.
4. Mantener separados los roadmaps de pasarelas y frontends; sus slices integrados no cierran fases locales.

Cada PR que cambie funcionalidades o aceptación debe actualizar en la misma entrega roadmap, acta, auditoría y este resumen.