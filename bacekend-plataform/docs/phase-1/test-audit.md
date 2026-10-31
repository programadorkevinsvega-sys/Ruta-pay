# Auditoría de pruebas — Fase 1: runtime y observabilidad

## Alcance

Esta auditoría cubre exclusivamente el runtime fundacional: configuración, validación de contexto HTTP, readiness, transporte HTTP→BFF→gRPC→módulo, deadlines/desconexión y observabilidad. No atribuye a Fase 1 casos de identidad, tenants o pagos.

## Evidencia ejecutable encontrada en main

- `tests/http_grpc_acceptance.rs` ejecuta un recorrido de aceptación HTTP→BFF→gRPC→módulo y comprueba éxito, módulo desconectado y timeout de transporte.
- El recorrido comprueba envelopes de error, retryability, correlación entre eventos de fallo y cierre HTTP, y redacción.
- Hay pruebas unitarias de configuración y validación de correlation ID.
- La CI Rust ejecuta `cargo fmt`, Clippy y `cargo test --workspace`; usa PostgreSQL, Redis y JetStream autenticados para los tests que los requieren.
- El test de aceptación HTTP/gRPC es un test de integración con fronteras reales, no un browser E2E de producto.

## Casos y estado

| Caso | Tipo esperado | Evidencia / estado |
|---|---|---|
| Correlation ID válido | Unitario | Cubierto |
| Correlation ID vacío | Unitario | Cubierto |
| Correlation ID >128 bytes | Unitario | Añadido en este PR |
| Caracteres no ASCII/control/separadores no permitidos | Unitario | Añadido en este PR |
| Runtime probe exitoso | Integración | Cubierto por `http_grpc_acceptance` |
| Módulo gRPC desconectado | Integración | Cubierto |
| Timeout de transporte | Integración | Cubierto |
| Error correlacionado con cierre HTTP | Integración | Cubierto |
| Configuración válida/inválida | Unitario | Hay cobertura; falta inventario campo por campo |
| Readiness con cada dependencia crítica degradada | Integración | No demostrado para una matriz exhaustiva |
| Caída de exportador/Collector y recuperación | Integración/operación | Evidencia histórica en acta; no se demuestra por la suite de CI actual |
| Matriz de límites de cada variable de configuración | Unitario | Pendiente de inventario completo |
| Build y browser E2E frontend | Build/E2E | No aplican a la superficie entregada en Fase 1; frontend real entra en Fase 2 |

## Cambios de prueba en este PR

Se amplía `CorrelationId` con pruebas explícitas de longitud mínima/máxima, longitud excedida, valor vacío, espacios, controles, separadores, Unicode y alfabeto permitido. Estos tests se ejecutan automáticamente mediante `cargo test --workspace`.

## Huecos pendientes y criterio de cierre de auditoría

1. Crear inventario exhaustivo de cada variable de configuración y sus valores válidos, inválidos y límites.
2. Determinar formalmente qué dependencias son críticas para readiness y probar cada combinación relevante de estados.
3. Convertir las pruebas históricas de degradación del Collector y shutdown en verificaciones repetibles en CI o en una suite de aceptación controlada.
4. Mantener el test de integración HTTP→BFF→gRPC como requisito obligatorio de CI.
5. Registrar commit, comandos, ejecución y resultado real para cada aceptación.

No se declara cobertura funcional del 100 %. La Fase 1 conserva su alcance implementado, pero los huecos enumerados siguen abiertos como trabajo de calidad de esta fase; no se posponen a Fase 8.
