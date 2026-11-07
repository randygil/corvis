---
status: no-disponible
---

# Testing

Corvis no incluye integración con modelos de lenguaje: no hay módulo de IA, clientes de proveedores, herramientas, agentes ni embeddings.

## Alternativa

Un cliente de cualquier proveedor se puede registrar como provider con una fábrica asíncrona (`Provider::use_factory`) y leer su clave con `ConfigService`; ver [Providers personalizados](/fundamentals/custom-providers) y [Configuración](/application/configuration). Corvis no aporta ese cliente.
