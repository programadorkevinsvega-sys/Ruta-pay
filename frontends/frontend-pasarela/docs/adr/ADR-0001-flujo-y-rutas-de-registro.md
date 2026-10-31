# ADR-0001: Flujo y rutas de registro del comercio

- Estado: Aceptada
- Fecha: 2026-10-22
- Alcance: `frontends/frontend-pasarela`

## Contexto

El formulario unificaba registro e inicio de sesión en la misma vista raíz. Al cambiar de modo mantenía el título “Iniciar sesión”, usaba el nombre de comercio y un slug como campos independientes, y no confirmaba la contraseña ni mostraba la política que el backend exige.

## Decisión

- La pantalla de registro vive en `/register`; inicio de sesión vive en `/login`; `/` redirige a `/login`. Los enlaces permiten cambiar entre ambas rutas.
- El formulario de registro pide nombre del comercio. Deriva y muestra el identificador/slug de solo lectura a partir del nombre; no pide al usuario inventar un ID interno.
- Solicita tipo de documento NIT/CC y número, correo, contraseña y confirmación. El contrato y propiedad del documento están definidos en el ADR de plataforma sobre identidad legal del tenant.
- El cliente valida confirmación y muestra en vivo los cinco requisitos de contraseña: 12–128 caracteres, mayúscula, minúscula, número y signo de puntuación. El backend vuelve a validar y es la autoridad.
- El modo de login muestra exclusivamente los campos de acceso; títulos, descripciones y rutas corresponden al modo visible.
- La UI consume los endpoints REST reales del BFF con cookies seguras y CSRF; no persiste contraseñas, documentos ni tokens en almacenamiento del navegador.

## Consecuencias

- Los links directos `/register` y `/login` deben servir la SPA también en el servidor de desarrollo/preview y en el despliegue web.
- La generación visual del slug es determinista, pero el backend valida formato y unicidad.
- Los tests de componente verifican derivación/validaciones; Playwright verifica rutas y onboarding contra el BFF real.

## Verificación

Probar navegación directa y desde enlaces en ambas rutas, redirección de `/`, slug derivado para nombres con tildes, rechazo de contraseña incompleta/no coincidente y registro real exitoso.
