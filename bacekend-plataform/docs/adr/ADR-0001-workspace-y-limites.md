# ADR-0001: Workspace raíz y límites de despliegue/modularidad

- Estado: Aceptada
- Fecha: 2026-10-22
- Alcance: backend-plataform y organización del repositorio Payvia

## Contexto

Payvia contiene dos backends desplegables, pero el repositorio adopta una resolución Cargo única. Se necesita preservar módulos con ownership y despliegues separados, y mantener tenant/admin como frontends distintos.

## Decisión

Se conserva el único `[workspace]` en `Cargo.toml` raíz, con `bacekend-plataform` y `bacekend-pasarela` como miembros. Cada backend mantiene `Cargo.toml` de paquete y composition root/binario independiente; no hay workspace anidado ni dependencia de implementación interna cruzada. Cada bounded context se convertirá en crate con comportamiento real cuando entre en su fase funcional; no se agregarán crates vacíos para simular avance. Los frontends continúan separados en `frontends/frontend-pasarela/` y `frontends/frontend-plataform/`, consumiendo BFF plataforma por REST/OpenAPI/WebSocket autenticado.

## Consecuencias

- La resolución de dependencias y `Cargo.lock` son compartidos; identidad, configuración, dominio y despliegues no.
- Cargo puede verificar los dos paquetes desde la raíz.
- Los contratos neutrales gRPC/protobuf solo se comparten con versionado y ownership explícito; el runtime siempre usa transporte autenticado.
- Cambiar a workspaces separados requiere ADR nueva y actualización de los documentos normativos.

## Verificación

Ejecutar `cargo metadata --no-deps --format-version 1`, `cargo check --workspace` y `cargo fmt --all -- --check` desde la raíz. Registrar sus resultados en `initial-audit.md`.
