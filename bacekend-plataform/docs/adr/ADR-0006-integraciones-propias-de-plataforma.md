# ADR-0006: Integraciones propias de cobro, facturación y correo de plataforma

- Estado: aceptada como decisión arquitectónica
- Contexto: Fase 0 — Auditoría, decisiones y workspace
- Alcance: únicamente `bacekend-plataform`

## Contexto

Payvia necesita cobrar a sus tenants por el uso de la plataforma, emitir sus documentos electrónicos y enviar correos operativos. Estas son obligaciones propias del SaaS y no deben confundirse con el procesamiento de pagos que los comercios ofrecen a sus compradores mediante `bacekend-pasarela`.

## Decisión

- `subscriptions_billing`/`platform_billing` mantiene el ownership de suscripciones, invoices, partidas, ledger, estados financieros e idempotencia de negocio.
- `platform_billing_wompi` es el adapter Wompi de plataforma. Usa cuenta y credenciales pertenecientes a Payvia, procesa cobros propios y recibe sus callbacks verificados en plataforma. No invoca `bacekend-pasarela` ni comparte sus credenciales, grants o estado con `gateway_wompi`.
- `electronic_invoicing` es dueño del estado y referencias fiscales de los documentos electrónicos de Payvia; `invoice_factus` es su adapter inicial para Factus. No reemplaza la invoice financiera del dominio de billing.
- `notifications` es dueño de políticas/estado de notificación; `notification_resend` es el adapter inicial para el envío mediante Resend.
- Los dominios llaman contratos neutrales al proveedor. El composition root registra los adapters por identificador/capacidades. Añadir o retirar un proveedor requiere implementar/desactivar su adapter y cambiar su registro/configuración, sin modificar las reglas generales de dominio ni los otros adapters.
- Desactivar un proveedor impide nuevas operaciones según una transición explícita; conserva ledger, invoices, auditoría, identificadores externos y capacidad de consulta/reconciliación histórica.
- Los secretos de proveedor se inyectan desde configuración protegida del despliegue y no se guardan en PostgreSQL de dominio, repositorios, eventos, logs ni respuestas. Los tokens OAuth temporales se mantienen con expiración/renovación seguras.
- Cada adapter verifica en su subfase la documentación oficial vigente, sandbox, autenticación, idempotencia, límites, callbacks y comportamiento ante resultados inciertos. No se asume soporte de cobro recurrente automático hasta verificarlo para la cuenta/API concreta.

## Consecuencias

- La facturación SaaS no depende del backend de pagos de comercios y tiene identidad, credenciales, rutas callback, auditoría y ciclo de despliegue propios.
- El estado financiero, el documento fiscal y el estado de entrega de correo son responsabilidades separadas, conectadas mediante contratos internos y eventos durables cuando aplique.
- Nuevos proveedores pueden reemplazar o complementar a los iniciales mediante adapters, sin ramificaciones por proveedor en las reglas centrales.
- La implementación y la validación sandbox siguen pendientes; esta ADR no declara capacidades disponibles.

## Referencias oficiales a verificar nuevamente al implementar

- [Wompi — ambientes y llaves](https://docs.wompi.co/docs/colombia/ambientes-y-llaves/)
- [Factus API](https://developers.factus.com.co/)
- [Resend — envío de correo](https://resend.com/docs/api-reference/emails/send-email)
- [Resend — claves de idempotencia](https://resend.com/changelog/idempotency-keys)
