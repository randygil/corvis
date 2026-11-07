---
status: no-disponible
---

# Reglas de producción

Corvis no tiene un sistema de reglas o alertas sobre el comportamiento de la aplicación en producción.

## Alternativa

Las alertas se configuran en la herramienta que recibe tus logs o consulta tu endpoint de salud. Desde la aplicación, lo más útil es emitir logs JSON con contexto claro (ver [Logging](/application/logger)) y exponer un endpoint de [Health checks](/reliability/terminus).
