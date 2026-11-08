# ADR-0006: Integraciones propias de cobro, billing y correo de plataforma

> **Enmienda de alcance:** la responsabilidad de emisión fiscal electrónica descrita originalmente aquí se separa del backend de plataforma por [ADR-0012](ADR-0012-separacion-billing-interno-integracion-fiscal.md). Esta ADR conserva las decisiones de billing interno, recaudo y notificaciones; no asigna la emisión DIAN a un módulo de este backend.

- Estado: aceptada como decisión arquitectónica
- Contexto: Fase 0 — Auditoría, decisiones y workspace
- Alcance: únicamente `bacekend-plataform`

## Contexto

Payvia necesita cobrar a sus tenants por el uso de la plataforma y enviar correos operativos. La factura interna SaaS es responsabilidad de plataforma; la emisión de documentos fiscales electrónicos se realiza en un microservicio/proyecto independiente. Estas responsabilidades no deben confundirse con el procesamiento de pagos que los comercios ofrecen a sus compradores mediante `bacekend-pasarela`.

## Decisión

- `subscriptions_billing`/`platform_billing` mantiene el ownership de suscripciones, invoices, partidas, ledger, estados financieros e idempotencia de negocio.
- `platform_billing_wompi` es el adapter Wompi de plataforma. Usa cuenta y credenciales pertenecientes a Payvia, procesa cobros propios y recibe sus callbacks verificados en plataforma. No invoca `bacekend-pasarela` ni comparte sus credenciales, grants o estado con `gateway_wompi`.
- La emisión fiscal electrónica y sus artefactos pertenecen a un servicio/proyecto independiente, no a `bacekend-plataform`. Plataforma conserva únicamente la correlación/estado de integración que necesite para enlazar la invoice interna con el resultado externo, mediante el contrato neutral de ADR-0012. No se crea `electronic_invoicing` ni `invoice_factus` en este backend.
- `notifications` es dueño de políticas/estado de notificación; `notification_resend` es el adapter inicial para el envío mediante Resend.
- Los dominios llaman contratos neutrales al proveedor. El composition root registra los adapters propios por identificador/capacidades. Añadir o retirar un proveedor requiere implementar/desactivar su adapter y cambiar su registro/configuración, sin modificar las reglas generales de dominio ni los otros adapters. La conexión al servicio fiscal externo sigue el límite de integración de ADR-0012.
- Desactivar un proveedor impide nuevas operaciones según una transición explícita; conserva ledger, invoices, auditoría, identificadores externos y capacidad de consulta/reconciliación histórica.
- Los secretos de proveedor se inyectan desde configuración protegida del despliegue y no se guardan en PostgreSQL de dominio, repositorios, eventos, logs ni respuestas. Los tokens OAuth temporales se mantienen con expiración/renovación seguras.
- Cada adapter verifica en su subfase la documentación oficial vigente, sandbox, autenticación, idempotencia, límites, callbacks y comportamiento ante resultados inciertos. No se asume soporte de cobro recurrente automático hasta verificarlo para la cuenta/API concreta.

## Consecuencias

- La facturación interna SaaS no depende del backend de pagos de comercios y tiene ownership, auditoría y ciclo de despliegue propios. La emisión fiscal es independiente y puede conectarse por eventos, webhook o API.
- El estado financiero, el documento fiscal y el estado de entrega de correo son responsabilidades separadas, conectadas mediante contratos internos y eventos durables cuando aplique.
- Nuevos proveedores pueden reemplazar o complementar a los iniciales mediante adapters, sin ramificaciones por proveedor en las reglas centrales.
- La implementación y la validación sandbox siguen pendientes; esta ADR no declara capacidades disponibles.

## Referencias oficiales a verificar nuevamente al implementar

- [Wompi — ambientes y llaves](https://docs.wompi.co/docs/colombia/ambientes-y-llaves/)

- [Resend — envío de correo](https://resend.com/docs/api-reference/emails/send-email)
- [Resend — claves de idempotencia](https://resend.com/changelog/idempotency-keys)
- [ADR-0012 — Separación entre facturación interna e integración fiscal](ADR-0012-separacion-billing-interno-integracion-fiscal.md)
