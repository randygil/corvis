---
status: no-disponible
---

# SDK

Corvis no tiene un SDK de observabilidad que instrumente la aplicación automáticamente (trazas, métricas y errores enviados a un servicio).

## Alternativa

Combina los logs JSON de `ConsoleLogger`, un interceptor global que mida cada petición y un filtro de excepciones global (ver [Visión general](/observability/overview)).
