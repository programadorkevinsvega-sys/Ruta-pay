# ADR-0005: Unión versionada `payment_method`

- Estado: Aceptada como contrato conceptual v1; campos wire quedan pendientes de contrato protobuf/OpenAPI
- Fecha: 2026-10-22

## Decisión

El envelope de transacción conserva `amount` como entero en unidades menores de la moneda comercial. `payment_method` es una unión discriminada por `type`; las variantes iniciales son `card`, `pse` y `crypto`, con `details` tipado específico. Solo se exponen datos mínimos permitidos y normalizados; nunca se serializan callbacks crudos del proveedor.

- `card`: marca y últimos cuatro dígitos, si el proveedor los entrega y el contrato autoriza su exposición. Nunca PAN/CVV.
- `pse`: código/nombre de banco y referencia permitida, cada uno opcional según proveedor y evento.
- `crypto`: activo, moneda de settlement, monto decimal en string y referencias de proveedor; la red e ID on-chain son opcionales y solo se informan si corresponden al flujo real.

## Consecuencias

- El monto de settlement cripto no se representa con float ni sustituye al importe comercial.
- Cada adapter valida campos disponibles; ausencia de un campo no se rellena con valores inventados.
- La versión wire, campos requeridos/opcionales, compatibilidad y vectores se definen en contrato versionado antes de implementarse en protobuf/OpenAPI.
- La firma de un proveedor no se normaliza al mecanismo de webhook saliente de Payvia.
