---
status: no-disponible
---

# TypeORM

TypeORM es un ORM de Node.js y Corvis, que es un framework de Rust, no tiene una integración con él: no existe un módulo equivalente ni repositorios inyectables por entidad.

## Alternativa

Usa [corvis-orm](/es/data/corvis-orm), el ORM integrado del workspace (PostgreSQL y SQLite, con `OrmModule` y `OrmService` inyectables), o registra un pool de `sqlx` u otro crate como provider de fábrica (ver [Visión general](/es/data/overview)).
