# Auditoría de pruebas — Fase 0

## Alcance correcto

La Fase 0 es exclusivamente de auditoría, diseño, decisiones, ownership, amenazas, contratos conceptuales y verificación estática del workspace. No contiene runtime ni casos de uso de negocio ejecutables. Por tanto, su cobertura no se mide con pruebas HTTP, gRPC, base de datos o navegador que pertenecen a fases de implementación.

## Matriz de aceptación de Fase 0

| Requisito | Evidencia exigida | Verificación | Estado |
|---|---|---|---|
| Composición del workspace | Manifiesto raíz, miembros y ausencia de workspaces anidados | `cargo metadata --no-deps --format-version 1` + revisión estructural | Documentado en auditoría inicial; revalidar en CI |
| Ownership de dominios | Mapa bounded-context → propietario de datos/API | Revisión de bounded contexts y reglas | Documentado |
| Límites entre backends | Contratos y responsabilidades de plataforma/pasarelas | Revisión de ADR y arquitectura | Documentado |
| Amenazas | Activo, actor, amenaza, control, evidencia y riesgo residual | Revisión del threat model | Documentado; requiere trazabilidad explícita amenaza→control→test |
| Integraciones futuras | Decisión/N/A justificado por frontera | Revisión de matriz de integración | Documentado |
| Decisiones arquitectónicas | ADR con contexto, decisión, consecuencias y estado | Validación de inventario y enlaces | Documentado |
| Reproducibilidad | Comandos exactos, versiones y resultados observados | Ejecución limpia en CI o evidencia verificable | Evidencia histórica; revalidación automatizada pendiente |
| Consistencia documental | Roadmap, PROJECT_RULES, AGENTS y arquitectura sin contradicciones | Script/check automatizado + revisión | Pendiente de automatización |

## Reglas de evidencia

- Una casilla documental no demuestra una integración runtime.
- Los N/A deben incluir el motivo, el propietario y la fase/subfase donde se verificará la frontera.
- Cada amenaza identificada debe enlazar con un control preventivo/detectivo y con los casos de prueba que demostrarán ese control cuando se implemente.
- La línea base histórica de 2026-10-22 no se presenta como ejecución actual. Los resultados deben incluir commit, workflow, comando, salida y fecha.
- Fase 0 solo se considera aceptada si el inventario documental es completo, las decisiones están aprobadas y las verificaciones estáticas aplicables se ejecutan correctamente.

## Hallazgos de auditoría

1. El acta inicial contiene una línea base histórica válida para su fecha, pero no equivale a evidencia reproducida en el commit actual.
2. La matriz de integración explica correctamente por qué las dependencias runtime no aplican en Fase 0.
3. No existe todavía una trazabilidad automatizada completa entre amenazas, controles y pruebas implementadas en fases posteriores.
4. El control de consistencia entre documentos normativos y roadmaps debe automatizarse para evitar divergencias.

## Criterio de cierre

La Fase 0 mantiene su cierre de alcance documental, sujeto a los controles de trazabilidad y reproducibilidad indicados. Los huecos de pruebas runtime se asignan a las fases que implementan cada operación y no se trasladan a Fase 8.
