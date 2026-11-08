---
status: documented
---

# Subida de archivos y streaming

Las subidas `multipart/form-data` se procesan con interceptores de archivos, al estilo de `multer`
sobre Express: un interceptor lee el cuerpo, guarda cada archivo con un *storage* y deja los archivos
disponibles para los parámetros `#[uploaded_file]` y `#[uploaded_files]`. Todo vive en
`corvis::multer`, y los tipos más usados se reexportan en la raíz de `corvis`.

## Un archivo

```rust
use corvis::prelude::*;
use corvis::{FileInterceptor, UploadedFile};

#[controller("files")]
pub struct FilesController;

#[routes]
impl FilesController {
    #[post("avatar")]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn avatar(&self, #[uploaded_file] file: Option<UploadedFile>, #[body] body: Value) -> Value {
        json!({
            "name": file.as_ref().map(|f| f.originalname.clone()),
            "size": file.as_ref().map(|f| f.size),
            "fields": body,
        })
    }
}
```

- `FileInterceptor::new("file")` acepta un archivo en el campo `file`. Un archivo en otro campo
  responde `400 Unexpected field - <campo>`.
- `#[uploaded_file] file: Option<UploadedFile>` recibe `None` si no llegó archivo. Con
  `file: UploadedFile` (sin `Option`) el archivo es obligatorio.
- `#[body]` recibe los campos de texto del formulario. Las claves `tags[]`, `user[name]` o `items[0]`
  y las claves repetidas se convierten en arrays y objetos, como con `append-field`.

`UploadedFile` tiene los campos de un archivo de multer: `fieldname`, `originalname`, `encoding`,
`mimetype`, `size` y, según el storage, `buffer` (memoria) o `destination`, `filename` y `path`
(disco). `file.bytes()` devuelve el contenido cuando está en memoria. Si el handler devuelve un `UploadedFile`, se
serializa con el mismo orden de claves que multer y `buffer` como `{ "type": "Buffer", "data": [...] }`.

## Varios archivos

| Interceptor | Parámetro |
|---|---|
| `FilesInterceptor::new("photos").max_count(12)` | `#[uploaded_files] files: Vec<UploadedFile>` |
| `FileFieldsInterceptor::new([MulterField::new("avatar").max_count(1), MulterField::new("docs")])` | `#[uploaded_files] files: HashMap<String, Vec<UploadedFile>>` |
| `AnyFilesInterceptor::new()` | `#[uploaded_files] files: Vec<UploadedFile>` |
| `NoFilesInterceptor::new()` | Solo campos de texto; cualquier archivo responde `400` |

```rust
use std::collections::HashMap;
use corvis::{FileFieldsInterceptor, MulterField};

#[post("profile")]
#[use_interceptors(FileFieldsInterceptor::new([MulterField::new("avatar").max_count(1), MulterField::new("docs")]))]
fn profile(&self, #[uploaded_files] files: HashMap<String, Vec<UploadedFile>>) -> Value {
    json!({ "fields": files.keys().collect::<Vec<_>>() })
}
```

## Límites y opciones

Cada interceptor acepta `.limits(..)`, `.storage(..)` y `.options(MulterOptions)`:

```rust
use corvis::multer::{DiskStorageOptions, Limits, MulterOptions, disk_storage};

#[post(":id/evidence")]
#[use_interceptors(FileInterceptor::new("file").options(
    MulterOptions::new()
        .storage(disk_storage(
            DiskStorageOptions::new()
                .destination("./uploads")
                .filename(|_req, file| Ok(format!("{}-{}", chrono::Utc::now().timestamp_millis(), file.originalname))),
        ))
        .limits(Limits::new().file_size(5 * 1024 * 1024).files(1).fields(20).parts(25)),
))]
async fn upload(&self, #[param("id")] id: String, #[uploaded_file] file: UploadedFile) -> HttpResult<Value> {
    Ok(json!({ "id": id, "path": file.path }))
}
```

(El ejemplo usa el crate `chrono` para el nombre; cualquier generador de nombres sirve.)

| `Limits` | Efecto al superarlo |
|---|---|
| `.file_size(bytes)` | `413 File too large` (también al **alcanzar** el tamaño, como busboy) |
| `.files(n)` | `400 Too many files` |
| `.fields(n)`, `.parts(n)` | `400` con el mensaje de multer |
| `.field_size(bytes)` | `400 Field value too long - <campo>` (1 MiB por defecto) |
| `.field_name_size(bytes)` | Longitud máxima del nombre de un campo |
| `.field_nesting_depth(n)` | `400 Field name nesting too deep - <field>` cuando un nombre tiene más de `n` corchetes |

Los demás errores multipart llevan los mismos cuerpos que la app de referencia: `400 Field name missing`,
`400 Multipart: Boundary not found`, `400 Multipart: Unexpected end of form` y
`400 Multipart: Malformed part header`. Los que esa app no traduce (`Unsupported content type`,
`Field name nesting too deep`, fallos de escritura) son un `500` cuya causa es `MulterError` o
`BusboyError`. Si la subida falla, los archivos que ya se escribieron en disco se borran. Igual que en
busboy, `filename` se decodifica como latin1 salvo que llegue como `filename*=UTF-8''...` o se use
`def_param_charset("utf8")`.

Otras opciones de `MulterOptions`:

- `.dest("uploads")`: atajo para guardar en disco con nombres aleatorios.
- `.file_filter(|req, file| ...)`: `Ok(true)` acepta, `Ok(false)` omite el archivo y `Err(e)` corta
  la subida con ese error.
- `.preserve_path(true)` y `.def_param_charset("utf8")`: como en multer.
- `.body_limit(bytes)`: tope total del cuerpo multipart (10 MiB por defecto, o
  `file_size × files + 1 MiB` si los `limits` piden más). Se supera con `413 request entity too large`,
  antes de leer nada si el `Content-Length` ya lo excede. multer no tiene este tope; Corvis lo necesita
  porque cada archivo se guarda en memoria hasta que lo recibe el storage. Si subes varios archivos grandes
  en una misma petición, súbelo de forma explícita.
- `.remove_files_on_error(false)`: conserva en disco los archivos ya escritos si después falla un
  pipe o el handler (por defecto se borran; multer los deja).

`server.request_timeout` (300 s por defecto) también limita la lectura del cuerpo multipart y responde `408`.

Sin `limits` no hay tope de partes, campos ni archivos más allá de `body_limit` (y el `field_size` de 1 MiB).
En rutas públicas fija los que la ruta necesita, por ejemplo
`Limits::new().file_size(5 * 1024 * 1024).files(1).fields(20).parts(25)`.

Los nombres de campo también tienen tope: `x[1000000]=v` no reserva un array de un millón de `null`. Cada
petición puede crear como mucho 1000 huecos y un índice que necesita más convierte ese array en un objeto
(`{ "1000000": "v" }`), como hace `qs`. Los nombres con más de 32 niveles de corchetes (`a[b][b]...`) se
guardan como una clave literal, para que un cuerpo pequeño no construya un valor tan profundo que desborde
la pila. `limits.field_nesting_depth(n)` cambia ese tope (hasta 256) y además responde `400 Field name nesting too deep - <field>`. Consulta [Análisis de HTTP y compatibilidad](/es/security/http-parsing).

### Opciones por módulo

`MulterModule::register(MulterOptions::new()...)` (o `register_async`) define las opciones por
defecto de los interceptores de los controladores del módulo que lo importa.

## Storage

- `memory_storage()` (por defecto): el contenido queda en `file.buffer`.
- `disk_storage(DiskStorageOptions::new().destination(..).filename(..))`: escribe en disco.
  `destination_fn` decide la carpeta por archivo. El nombre devuelto por `filename` se valida y, si
  sale de `destination` (`..`, rutas absolutas, unidades, flujos NTFS, nombres reservados de
  Windows), la subida responde `400 Invalid file name`. En Windows también rechaza un flujo alternativo
  de NTFS (`nombre.txt:flujo`), un dispositivo reservado (`CON`, `NUL`, `COM1`, `nul.txt`...), los
  caracteres `<>"|?*` y los nombres que terminan en punto o espacio. Aun así, no uses `file.originalname`
  tal cual como nombre en disco: genera el nombre en el servidor.
- Con `preserve_path(true)`, `originalname` conserva la ruta que manda el cliente (`../../x`, `C:\...`).
  La disk storage rechaza los nombres que salen del destino, pero un `StorageEngine` propio debe
  validarlo por su cuenta.
- Archivos huérfanos: si `ParseFilePipe` o el handler rechazan la petición, los archivos ya escritos se
  borran (ver `remove_files_on_error`). Para no escribir archivos inválidos, usa `limits.file_size` y
  `file_filter`, que actúan antes del storage.
- Un storage propio implementa el trait `StorageEngine` (`handle_file` y `remove_file`).

## Validar el archivo con `ParseFilePipe`

`#[uploaded_file(...)]` acepta pipes. `ParseFilePipe` ejecuta validadores sobre el archivo:

```rust
use corvis::{FileTypeValidator, MaxFileSizeValidator, ParseFilePipe};

#[post("validated")]
#[use_interceptors(FileInterceptor::new("file"))]
fn validated(
    &self,
    #[uploaded_file(ParseFilePipe::new()
        .validator(MaxFileSizeValidator::new(1_000_000))
        .validator(FileTypeValidator::new("image/png")))]
    file: UploadedFile,
) -> Value {
    json!({ "ok": file.originalname })
}
```

- `MaxFileSizeValidator::new(bytes)` con `.message(..)` o `.error_message(..)`.
- `FileTypeValidator::new("image/(png|jpeg)")` detecta el tipo real por la firma del archivo
  (*magic numbers*, como el paquete `file-type`): imágenes, PDF, ZIP y Office, audio, video, fuentes y
  comprimidos, entre otros. Para tipos de texto sin firma usa `.fallback_to_mimetype(true)` o
  `.skip_magic_numbers_validation(true)`.
- Dentro de los pipes el archivo viaja como JSON; `corvis::multer::buffer_from_value(&v["buffer"])`
  recupera los bytes en un pipe o validador propio. Desde un guard o interceptor posterior, `req.file()`
  y `req.files_value()` (trait `MulterRequestExt`) dan `req.file` / `req.files`.
- `ParseFilePipe` acepta `.file_is_required(false)`, `.error_http_status_code(422)` y
  `.exception_factory(..)`.
- `ParseFilePipeBuilder::new().add_max_size_validator(..).add_file_type_validator(..).build(ParseFilePipe::new())`
  es la forma con *builder*.
- Un validador propio implementa `FileValidator` (`is_valid` y `build_error_message`).

::: warning La firma no garantiza que el archivo sea inocuo
`FileTypeValidator` solo mira el inicio del archivo: un archivo políglota (cabecera GIF seguida de HTML)
puede pasar como imagen, y `fallback_to_mimetype(true)` confía en el `mimetype` que manda el cliente. Al
servir subidas, usa un `Content-Type` fijo, `Content-Disposition: attachment` y
`X-Content-Type-Options: nosniff`, y evita `fallback_to_mimetype` para tipos sensibles.
:::

## Cuándo se lee el cuerpo

El interceptor solo actúa si la petición trae `Content-Type: multipart/*`; si no, no hace nada.
El cuerpo multipart no lo lee el servidor sino el interceptor, después de los guards y solo en las
rutas que lo usan: una petición a una ruta sin interceptor, inexistente o rechazada por un guard se
responde sin leer el cuerpo. La lectura es en *streaming* y los límites cortan la subida en cuanto se
alcanzan.

Cada archivo, eso sí, se acumula en memoria hasta que termina su parte y se entrega entero al
storage: no hay streaming directo a disco. Por eso existe `body_limit`.

Para procesar el multipart a mano, `corvis::multer::read_body(&req, limite).await?` devuelve el
cuerpo completo; `req.raw_body()` está vacío en esas peticiones.

## Enviar archivos al cliente

Para el sentido contrario, devolver un archivo, usa `StreamableFile`:

```rust
#[get(":id/pdf")]
async fn pdf(&self, #[param("id", ParseUUIDPipe)] id: String) -> HttpResult<StreamableFile> {
    let file = StreamableFile::from_path(format!("./reports/{id}.pdf"))
        .await
        .map_err(|_| NotFoundException::new("Report not found"))?;
    Ok(file.content_type("application/pdf").filename(format!("{id}.pdf")))
}
```

`from_path` lee el archivo del disco por partes. Hay más formas en
[Respuestas en streaming](/es/http/streaming-responses).

## Pruebas

El cliente de pruebas arma el `multipart/form-data` (`attach` acepta texto o bytes):

```rust
let r = request(&app)
    .post("/files/avatar")
    .field("title", "Hola")
    .attach("file", "contenido", "notas.txt")
    .send()
    .await;
```
