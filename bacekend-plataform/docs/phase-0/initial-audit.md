# Fase 0 — Auditoría inicial de backend-plataform

Fecha de auditoría: 2026-10-22. Rama observada: `main`.

## Alcance y fuentes

Se contrastaron el roadmap y README de raíz, `AGENTS.md`, `ARCHITECTURE_RULES.md`, `PROJECT_RULES.md`, el roadmap, README, reglas específicas y los prompts normativos de calidad y observabilidad de plataforma. Las reglas estrictas de plataforma y `ARCHITECTURE_RULES.md` son idénticas (SHA-256 `a63e5a3fc4e064526e4f741d2a15ab1fd8f782e1177dce5173d323cfd820d48b`).

## Inventario observado

| Área | Estado observado | Riesgo / decisión |
|---|---|---|
| Workspace Cargo | `Cargo.toml` raíz con resolver 3 y dos miembros; los manifests de ambos backends heredan versión y edition. No hay workspace anidado. | Se conserva un único `Cargo.lock` generado desde la raíz. |
| Backend plataforma | Un paquete con `src/main.rs` que solo imprime `Hello, world!`; sin módulos funcionales, configuración, dependencias ni pruebas. | No se atribuyen capacidades del README al scaffold. |
| Backend pasarelas | Miembro separado con scaffold mínimo y documentación propia. | Su runtime/ownership no se incorpora a este backend. |
| PostgreSQL / migraciones | No se encontraron migraciones ni configuración local versionada. | Fase 0 no crea schemas; ownership y coordinación quedan definidos en ADR-0003. |
| Redis / NATS | No hay cliente, configuración o consumers/productores en plataforma. | Sin mutación de negocio ni evento durable en Fase 0; decisiones y aplicación posterior registradas en matriz. |
| HTTP / BFF / gRPC | No hay routers, OpenAPI, protos, servidores o clientes gRPC. | Fase 1 construirá y probará la frontera técnica de health/readiness; aún no hay camino ejecutable. |
| Observabilidad | No hay inicialización `tracing`, OpenTelemetry, métricas ni Sentry. | Aplicación desde la primera implementación runtime en Fase 1, con fallos de telemetría degradables. |
| Frontends | Existen README y ROADMAP separados en `frontends/frontend-pasarela/` y `frontends/frontend-plataform/`, pero no manifests ni apps fuente. | Se conserva su separación; los contratos BFF quedan para fases correspondientes. |
| CI/despliegue | No se hallaron workflows, Compose, configuración de entorno o migraciones de plataforma. | No se declara instalación integrada disponible. Fase 0 define límites y Fase 1/9 aporta implementación según roadmap. |
| Dependencias | Los manifests de los backends no declaran dependencias de aplicación. | No se añadieron dependencias solo para crear scaffolds sin comportamiento. |

## Comprobaciones de línea base

Registrar únicamente los comandos realmente ejecutados al cierre de esta fase:

| Comando | Estado | Resultado |
|---|---|---|
| `git status --short --branch` | APROBADO | Rama `main`, árbol limpio al iniciar la tarea. |
| `rustc --version` / `cargo --version` | APROBADO | Rust `1.99.1`, Cargo `1.99.1`. |
| `docker --version` / `docker compose version` | APROBADO | Docker `29.9.1`, Compose `v5.6.0`; no se iniciaron servicios. |
| `cargo metadata --no-deps --format-version 1` | APROBADO | Resuelve dos miembros/binarios (`bacekend-plataform`, `bacekend-pasarela`) del workspace raíz. |
| `cargo check --workspace` | APROBADO | Ambos paquetes compilan; no demuestra funcionalidades de negocio. |
| `cargo fmt --all -- --check` | APROBADO | Sin diferencias de formato. |
| `cargo test --workspace` | APROBADO, SIN COBERTURA | Los targets reportaron 0 pruebas; no hay pruebas funcionales. |
| `cargo clippy --workspace --all-targets -- -D warnings` | APROBADO | Ambos paquetes sin warnings Clippy. |

No había pruebas de negocio antes de la fase. No se afirma integración con PostgreSQL, Redis, NATS, BFF, gRPC, OpenTelemetry o Sentry por la mera existencia de documentos.

## Discrepancias y tratamiento

1. Fase 0 se define como auditoría, decisiones y workspace, sin runtime. Por decisión explícita del proyecto, la aceptación HTTP→BFF→gRPC se asigna a la primera fase de implementación (Fase 1). El scaffold auditado carecía de esas fronteras; este documento no afirma haberlas probado.
2. El workspace contiene los dos paquetes/binarios iniciales y no tiene módulos funcionales. La Fase 0 documenta límites; crates/apps se incorporan con comportamiento real en las fases que los necesitan, evitando crates vacíos y stubs.
3. El README de plataforma indicaba que el workspace compartido aún no existía; se actualizó para reflejar el workspace único raíz ya configurado y diferenciarlo de las capacidades funcionales que siguen pendientes.

## Riesgos priorizados

| Riesgo | Impacto | Control/artefacto |
|---|---|---|
| Una llamada técnica de gateway se confunde con autorización comercial | Crítico | ADR-0002, autorización previa, concesión ligada al comando y denegación cerrada. |
| Fuga de credenciales tenant en plataforma/eventos/logs | Crítico | ADR-0002: pasarelas es el único dueño de persistencia; plataforma reenvía sin retener/publicar secreto. |
| Cruce de schema entre módulos en una DB física | Alto | ADR-0003: ownership por schema, sin SQL cruzado, migraciones coordinadas. |
| Replay/reintento produce efectos duplicados | Alto | ADR-0004: gRPC para request-response; Outbox/JetStream e idempotencia durable para eventos. |
| Certificado válido pero método no autorizado | Alto | ADR-0002: identidad criptográfica y autorización por operación/método/scope separadas. |
| Contexto de trazabilidad o datos sensibles perdidos entre fronteras | Alto | ADR-0004 y matriz: W3C, IDs correlacionados, eventos diagnósticos y redacción. |

## Estado

Auditoría documental y verificaciones estáticas del workspace completadas. **Estado: CERRADA (Fase 0 documental).** No se levantaron dependencias ni se probaron integraciones runtime; estas corresponden a Fase 1. Los crates/apps y contratos ejecutables se crearán con comportamiento real en las fases que los requieren, sin scaffolds vacíos.
