---
status: no-disponible
---

# Almacenamiento de archivos

Corvis no tiene una capa de almacenamiento de archivos con controladores intercambiables (disco local, S3 u otros servicios) detrás de una misma API.

## Alternativa

Lo que sí existe cubre el caso de guardar y servir archivos en disco:

- **Recibir archivos**: `FileInterceptor` y el resto de interceptores de `corvis::multer` aceptan `multipart/form-data`. Con `disk_storage(DiskStorageOptions::new().destination(..).filename(..))` cada archivo se escribe en disco, y con `memory_storage()` queda en memoria. Para otro destino (un bucket, por ejemplo), implementa el trait `StorageEngine` (`handle_file` y `remove_file`). Ver [Subida de archivos y streaming](/http/file-upload).
- **Enviar archivos**: `StreamableFile::from_path("informe.pdf").await?` lee el archivo por partes y lo devuelve desde un handler.
- **Servir una carpeta**: `corvis::serve_static(raíz)` o `ServeStaticModule::for_root(...)` (ver [Archivos estáticos](/recipes/serve-static)).
