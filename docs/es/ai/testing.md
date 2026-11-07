---
status: unavailable
---

# Testing

Corvis no incluye integración con modelos de lenguaje: no hay módulo de IA, clientes de proveedores, herramientas, agentes ni embeddings.

## Alternativa

Un cliente de cualquier proveedor se puede registrar como provider con una fábrica asíncrona (`Provider::use_factory`) y leer su clave con `ConfigService`; ver [Providers personalizados](/es/fundamentals/custom-providers) y [Configuración](/es/application/configuration). Corvis no aporta ese cliente.
