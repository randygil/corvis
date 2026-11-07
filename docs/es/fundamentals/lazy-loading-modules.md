---
status: no-disponible
---

# Carga diferida de módulos

Corvis no tiene carga diferida de módulos: no existe un cargador que añada un módulo al contenedor después del arranque. Todo el grafo de módulos se recorre y se instancia en `CorvisFactory::create`, antes de atender la primera petición.

En otros entornos la carga diferida existe sobre todo para reducir el tiempo de arranque en frío de funciones serverless, donde cargar código es caro. En un binario de Rust todo el código ya está compilado y enlazado, así que el coste que queda es el de **construir** los providers.

## Alternativas

- **Construir bajo demanda.** Si un provider es caro de crear y solo se usa a veces, no lo registres como singleton: márcalo transient y resuélvelo con `module_ref.resolve::<T>().await`, o constrúyelo cuando haga falta con `module_ref.create::<T>().await`. Ver [Referencia al módulo](/es/fundamentals/module-ref).
- **Inicialización diferida dentro del provider.** Un campo `tokio::sync::OnceCell` (con `#[default]`) permite crear el recurso caro la primera vez que se usa, manteniendo un único provider registrado.
- **Elegir módulos al compilar.** Las *features* de Cargo y `#[cfg(feature = "...")]` permiten incluir o excluir módulos enteros del binario.
- **Contextos separados.** `CorvisFactory::create_application_context::<OtroModulo>()` crea un contenedor independiente (sin servidor HTTP) a partir de otro módulo raíz, por ejemplo para una tarea puntual. Ver [Aplicaciones standalone](/es/standalone-applications).
