# ADR-0013 — Procesamiento por lotes del consumo de uso con aislamiento tenant

- **Estado:** Aceptado para implementación en Fase 4
- **Fecha:** 2026-11-08
- **Ámbito:** `platform_runtime` y `platform_billing`
- **Contexto:** consumidor durable de `billing.usage.recorded` desde NATS JetStream.

## Contexto

El consumidor persistía cada evento de uso mediante una transacción PostgreSQL individual. Aunque Inbox, registro de uso y disposición `processed` ya se escriben atómicamente para un evento, el coste fijo de adquisición de conexión, round trips SQL y commit limita el throughput. Aumentar solamente la concurrencia eleva la presión sobre PostgreSQL y la memoria, sin reducir el coste por evento.

El módulo aplica Row Level Security (RLS) mediante el contexto transaccional `app.tenant_id`. Una transacción no puede mezclar eventos de tenants diferentes ni cambiar el tenant activo durante su ejecución.

## Decisión

1. El consumidor obtiene mensajes en lotes acotados y valida/deserializa cada envelope antes de persistirlo.
2. Los mensajes válidos se agrupan por `tenant_id`. Cada grupo se procesa en una transacción PostgreSQL independiente, con `set_tenant_context` aplicado una sola vez al inicio de esa transacción.
3. Inbox y usage se insertan mediante operaciones SQL por lotes. La verificación de idempotencia compara tipo, versión, hash del payload y contenido de uso; los conflictos no se convierten en duplicados aceptados.
4. La actualización de disposición Inbox a `processed` forma parte de la misma transacción que las inserciones. El commit es el único punto de durabilidad.
5. JetStream ACK se envía individualmente a cada mensaje del grupo únicamente después del commit de ese grupo. Si la transacción falla, no se ACKean sus mensajes y se solicita redelivery. Los mensajes inválidos o con conflicto permanente siguen el camino explícito de rechazo/TERM.
6. El tamaño del lote y el número de grupos concurrentes son acotados. La admisión se limita por capacidad disponible del pool PostgreSQL, presupuesto de CPU y memoria disponible; no se aumenta la concurrencia sin límites para perseguir throughput.
7. Las métricas separan: antigüedad/espera desde publicación o entrega, espera de adquisición del pool, tiempo SQL, tiempo de commit y tiempo de ACK. También se registran tamaño del lote, tenant-group count, redelivery, resultados y presión de recursos, sin etiquetas de alta cardinalidad ni datos sensibles.

## Invariantes

- Una transacción contiene eventos de un solo tenant.
- Inbox, usage y disposición `processed` se confirman o revierten juntos.
- Ningún mensaje recibe ACK antes del commit de su grupo.
- Un fallo de ACK posterior al commit puede producir redelivery; la idempotencia debe hacer que el replay sea seguro.
- Una entrega duplicada idéntica puede reconocerse; el mismo identificador con payload/tipo/versión distintos es conflicto permanente.
- Un error en un tenant no debe impedir confirmar grupos independientes de otros tenants.
- El tamaño de lote, concurrencia y memoria en vuelo tienen límites superiores configurables y límites seguros por defecto.

## Alternativas consideradas

- **Una transacción por evento con más concurrencia:** descartada como estrategia principal; aumenta la contención y mantiene los round trips por evento.
- **Una transacción compartida entre tenants:** descartada porque rompe el aislamiento RLS y amplía el alcance de rollback.
- **ACK antes de persistir:** descartada porque puede perder eventos.
- **ACK después de cada INSERT pero antes del commit:** descartada porque un rollback posterior perdería eventos ya confirmados por JetStream.
- **Concurrencia ilimitada:** descartada por riesgo de saturar el pool, CPU y memoria.

## Consecuencias

- Se reduce el coste fijo de SQL y commit por evento, a cambio de transacciones por lote y una pequeña espera de agrupación.
- Un lote fallido puede redeliverar todos sus mensajes; por eso las operaciones deben ser idempotentes y el lote debe tener tamaño acotado.
- Los grupos por tenant pueden tener tamaños desiguales. El planificador debe evitar que un tenant ruidoso monopolice la capacidad.
- La capacidad real depende del pool, PostgreSQL, CPU, memoria, configuración JetStream y número de réplicas. No se declara un objetivo de throughput cumplido sin benchmark en recursos limitados equivalentes al entorno objetivo.

## Validación requerida

- Lotes de uno y muchos eventos; múltiples tenants en el mismo fetch; lotes con un único tenant.
- Duplicados idénticos y conflictos de event_id/source_event_id dentro del mismo lote y entre redeliveries concurrentes.
- Fallos de INSERT, UPDATE y COMMIT; rollback completo y redelivery.
- ACK exitoso/fallido y reinicio entre commit y ACK.
- Aislamiento RLS verificando que ninguna transacción lee o escribe datos de otro tenant.
- Backpressure cuando el pool está ocupado, CPU está limitada o memoria se aproxima al límite.
- Medición separada de queue wait, pool acquire, SQL, commit y ACK.
- Benchmark reproducible con límites de CPU/memoria documentados, varias réplicas y PostgreSQL/NATS reales.
