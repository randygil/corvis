---
status: no-disponible
---

# Panel

Corvis no incluye un panel web para ver peticiones, errores o métricas de la aplicación.

## Alternativa

Envía los logs JSON (`ConsoleLogger::new().json(true)`) a la herramienta de visualización que ya uses. Para el estado de la instancia, expón un endpoint con [Health checks](/reliability/terminus).
