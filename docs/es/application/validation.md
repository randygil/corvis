---
status: documented
---

# Validación

Todo dato que entra por HTTP es sospechoso hasta que se comprueba. Corvis ofrece dos formas de validar los argumentos de un handler, y pueden convivir en la misma aplicación:

- **DTOs con atributos** (`#[derive(Validate)]` + `ValidationPipe`): reglas declarativas sobre cada campo, con los mensajes de class-validator.
- **Schemas** (`corvis::z` + `ZodValidationPipe`): un port de zod 4 que valida, transforma y aplica valores por defecto.

Ambos son [pipes](/es/pipes): reciben el valor JSON del argumento antes de que se deserialice en el tipo del parámetro y pueden rechazarlo con un `400`.

## DTOs con `#[derive(Validate)]`

```rust
use corvis::Validate;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    #[max(40)]
    pub age: u32,
    #[is_string]
    #[is_in("persian", "siamese", "maine coon", "mixed")]
    pub breed: String,
}
```

(Tomado de `examples/cats-app/src/cats/dto/mod.rs`.) Activa la validación con un pipe global en `main`:

```rust
app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
```

Con eso, cualquier `#[body]`, `#[query]` o `#[param]` cuyo tipo derive `Validate` se comprueba antes de llegar al handler. Si falla, la respuesta es:

```json
{ "message": ["name should not be empty", "age must not be less than 0"], "error": "Bad Request", "statusCode": 400 }
```

Los tipos que no derivan `Validate` pasan sin cambios.

### Reglas disponibles

| Grupo | Atributos |
|---|---|
| Tipo | `is_string`, `is_int`, `is_number`, `is_boolean`, `is_array`, `is_object`, `is_enum` |
| Presencia | `is_defined`, `is_optional`, `is_not_empty`, `is_empty` |
| Formato de texto | `is_email`, `is_url`, `is_uuid`, `is_alpha`, `is_alphanumeric`, `is_number_string` (alias `is_numeric_string`), `is_date_string` (alias `is_iso8601`), `is_hex_color`, `is_jwt`, `is_mongo_id`, `is_ip`, `is_lowercase`, `is_uppercase`, `matches("regex")` |
| Longitud | `min_length(n)`, `max_length(n)`, `length(min, max)` |
| Números | `min(n)`, `max(n)`, `is_positive`, `is_negative` |
| Contenido | `contains("s")`, `not_contains("s")`, `is_in(a, b, ...)`, `is_not_in(a, b, ...)`, `equals(v)`, `not_equals(v)` |
| Arreglos | `array_min_size(n)`, `array_max_size(n)`, `array_not_empty`, `array_unique` |
| Composición | `validate_nested`, `validate_if(f)`, `validate_with(f)` |

Detalles útiles:

- **Mensaje propio**: cualquier regla acepta `message = "..."`, por ejemplo `#[min_length(3, message = "too short")]`.
- **Cada elemento**: `each` aplica la regla a cada elemento de un arreglo (`#[max_length(20, each)]`); el mensaje empieza con `each value in ...`.
- **Opcionales**: `#[is_optional]` omite las demás reglas si el campo falta o es `null`. Úsalo con campos `Option<T>`.
- **Reglas implícitas**: un campo sin atributos se comprueba según su tipo de Rust (texto, entero, número, booleano).
- **Anidados**: `#[validate_nested]` valida un campo cuyo tipo también deriva `Validate` (o cada elemento de un `Vec` de ese tipo).
- **Condición**: `#[validate_if(|obj| ...)]` recibe el objeto JSON completo (`&serde_json::Map<String, Value>`) y solo valida el campo si devuelve `true`.
- **Regla libre**: `#[validate_with(f)]` recibe el valor del campo (`Option<&Value>`) y devuelve `bool`; el mensaje por defecto es `<campo> is invalid`.
- **Enums**: `#[is_enum]` comprueba que el valor sea una de las variantes de un enum que implementa `Deserialize` (el del propio campo o el indicado: `#[is_enum(Role)]`).
- Los nombres de los campos en los mensajes respetan `#[serde(rename = "...")]` y `#[serde(rename_all = "...")]`.

### Opciones del pipe

`ValidationPipe::new()` se configura con métodos encadenados (o con `ValidationPipe::with_options(ValidationPipeOptions { .. })`):

| Método | Efecto |
|---|---|
| `whitelist(true)` | Elimina las propiedades que el DTO no declara |
| `forbid_non_whitelisted(true)` | En lugar de eliminarlas, responde error |
| `skip_missing_properties(true)` | No valida propiedades ausentes o `null` |
| `stop_at_first_error(true)` | Deja de validar una propiedad tras su primer error |
| `disable_error_messages(true)` | Responde sin la lista de mensajes |
| `error_http_status_code(422)` | Cambia el código de estado (por defecto `400`) |
| `exception_factory(f)` | `fn(Vec<ValidationError>) -> HttpException` para construir tu propia respuesta |
| `transform(bool)` | **Obsoleto**, sin efecto: en Corvis la conversión al tipo declarado siempre ocurre; elimina la llamada |

En `#[query]` y `#[param]` el pipe convierte primero los textos numéricos y booleanos según el tipo del campo (`"2"` pasa a `2` en un `u32`). En `#[body]` no hay conversión: un JSON con `"age": "2"` falla en `#[is_int]`.

Para validar un DTO ya construido fuera de un handler, usa `corvis::validation::validate(&dto)`, que devuelve la lista de `ValidationError`.

## Schemas con `corvis::z`

`corvis::z` es un port de [zod](https://zod.dev) **4.4.3** y de su integración habitual con DTOs, pipes y serialización en frameworks de Node.js (versión **5.5.0**). Produce los mismos códigos de error, mensajes (locale `en`), orden de las claves, orden de los checks y forma JSON que zod 4, y `ZodValidationPipe` responde los mismos cuerpos de error que esa integración. El comportamiento de zod 3 sigue disponible con `z::set_version(z::ZodVersion::V3)` (global), `z::with_version(..)` (hilo actual) o `ParseOptions::v3()` (una sola llamada).

Reproduce la API de zod 4 con funciones de Rust. Un DTO se asocia a su schema con la macro `zod_dto!`:

```rust
use corvis::prelude::*;
use corvis::zod_dto;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomDto {
    pub name: String,
    pub topic: String,
    pub max_members: i64,
}

zod_dto!(
    CreateRoomDto,
    z::strict_object! {
        name: z::string().min(1).max(24).regex("^[a-z0-9][a-z0-9-]*$"),
        topic: z::string().trim().max(120).default(""),
        maxMembers: z::int().min(1.0).max(500.0).default(50),
    }
);
```

(Adaptado de `examples/tertulia/src/rooms/dto.rs`.) Con `ZodValidationPipe::auto()` como pipe global, cada parámetro cuyo tipo tiene schema se valida, y lo que se deserializa en el struct es la **salida** del schema: con los `default` aplicados, los textos recortados y las claves desconocidas eliminadas o rechazadas según el tipo de objeto.

```rust
app.use_global_pipes(ZodValidationPipe::auto());
```

Un error responde así:

```json
{
  "statusCode": 400,
  "message": "Validation failed",
  "errors": [
    { "origin": "string", "code": "too_small", "minimum": 1, "inclusive": true,
      "path": ["name"], "message": "Too small: expected string to have >=1 characters" }
  ]
}
```

Un body, query, parámetro o header ausente se valida como `undefined`, igual que en Express. `ZodValidationPipe` y `ValidationPipe` pueden convivir: cada uno ignora los tipos que no le corresponden.

Otras formas de usar el pipe:

- `ZodValidationPipe::new(schema)` valida un argumento concreto con un schema explícito, por ejemplo para query strings: `#[query(ZodValidationPipe::new(z::object! { page: z::coerce::number().int().min(1.0).default(1) }))] q: ListQuery`.
- `ZodValidationPipe::dto::<Dto>()` usa el schema de otro DTO.
- `z::create_zod_validation_pipe(ZodValidationPipeOptions::new().create_validation_exception(|e| ...).strict_schema_declaration(true))` personaliza la excepción y, con `strict_schema_declaration`, rechaza (500) los parámetros cuyo tipo no tiene schema.

### Query strings y `coerce`

Los valores de la query llegan como texto. Usa `z::coerce::number()` para convertirlos antes de validar. Ten cuidado con `z::coerce::boolean()`: sigue la regla de JavaScript (`Boolean(x)`), así que `"false"` da `true`. Para banderas de texto usa `z::stringbool()`.

### Validar sin pipe

Un schema se puede usar directamente:

```rust
let schema = z::object! { name: z::string().min(2), email: z::email() };
let result = schema.safe_parse(&json!({ "name": "A" }));
if let Some(error) = result.error {
    let flat = z::flatten_error(&error); // { formErrors, fieldErrors }
}
```

`parse`, `safe_parse`, `parse_as::<T>` (valida y deserializa), `flatten_error`, `treeify_error` y `prettify_error` están disponibles. También están `format_error`, `error.to_json()` y `error.message()`. El catálogo completo de tipos y métodos sigue la API de zod 4; las tablas de equivalencias están más abajo.

### Mensajes en español

Los mensajes de zod usan el locale `en` por defecto. `corvis::z` incluye también `es`:

```rust
z::configure(|c| c.locale_error = Some(z::locales::es()));
```

### Equivalencias con zod 4

| zod 4 | `corvis::z` |
|---|---|
| `z.string().min(3, "msg")`, `.min(3, { error: (iss) => ... })` | `z::string().min(3).message("msg")`, `.min(3).error(z::error_fn(\|iss\| Some(..)))` |
| `z.string({ error: "msg" })` | `z::string().error("msg")` (sin checks, aplica al schema) |
| `.max() .length() .nonempty() .regex(/re/i) .startsWith() .endsWith() .includes(s, { position })` | `.max() .length() .nonempty() .regex(("re", "i")) .starts_with() .ends_with() .includes_at(s, pos)` |
| `.trim() .toLowerCase() .toUpperCase() .normalize() .slugify() .lowercase() .uppercase()` | `.trim() .to_lower_case() .to_upper_case() .normalize("NFC") .slugify() .lowercase() .uppercase()` |
| `z.email()`, `z.email({ pattern: z.regexes.html5Email })` | `z::email()`, `z::email_with(z::regexes::HTML5_EMAIL.clone())` |
| `z.uuid() z.uuidv4() z.guid() z.url({ hostname, protocol, normalize }) z.httpUrl()` | `z::uuid() z::uuidv4() z::guid() z::url_with(UrlOptions {..}) z::http_url()` |
| `z.cuid() z.cuid2() z.ulid() z.xid() z.ksuid() z.nanoid() z.emoji()` | iguales |
| `z.ipv4() z.ipv6() z.cidrv4() z.cidrv6() z.mac({ delimiter }) z.hostname()` | `z::ipv4() ... z::mac(Some("-")) z::hostname()` |
| `z.base64() z.base64url() z.hex() z.hash("sha256") z.jwt({ alg }) z.e164()` | `z::base64() ... z::hash("sha256", None) z::jwt(Some("HS256")) z::e164()` |
| `z.iso.datetime({ offset, local, precision }) z.iso.date() z.iso.time() z.iso.duration()` | `z::iso::datetime_with(IsoDatetime {..})`, `z::iso::date()`, `z::iso::time_with(p)`, ... (`z::time_precision::MILLISECOND`) |
| `z.stringFormat("code", /re/ \| fn)` | `z::string_format("code", "re")`, `z::string_format_fn("code", \|s\| ..)` |
| `z.number().int().gt().gte().lt().lte().positive().multipleOf().step()` | iguales (`gt(0.0)`, ...); `z::int()`, `z::int32()`, `z::uint32()`, `z::float32()`, `z::float64()` |
| `z.bigint() z.int64() z.uint64()`, `z.boolean()`, `z.null()`, `z.undefined()`, `z.void()`, `z.nan()` | iguales |
| `z.any() z.unknown() z.never() z.date()` | iguales (`z::date()` recibe las fechas de `z::coerce::date()` o de un transform; con `ParseOptions::date_strings(true)` también acepta strings ISO) |
| `z.literal("a")`, `z.literal(["a", 1, null])` | `z::literal("a")`, `z::literal(json!(["a", 1, null]))` |
| `z.enum(["a", "b"])`, `z.enum({ A: "x" })`, `z.nativeEnum(E)`, `.extract() .exclude()` | `z::enum_(["a", "b"])`, `z::enum_entries([("A", json!("x"))])`, `z::native_enum::<E>()`, `.extract(&[..]) .exclude(&[..])` |
| `z.stringbool({ truthy, falsy, case })` | `z::stringbool()`, `z::stringbool_with(StringBoolOptions {..})` |
| `z.templateLiteral(["id-", z.number()])` | `z::template_literal([TemplatePart::from("id-"), z::number().into()])` |
| `z.object({...})`, `z.strictObject`, `z.looseObject` | `z::object! { campo: s, "con-guion": s }`, `z::strict_object! {..}`, `z::loose_object! {..}` |
| `.strict() .strip() .passthrough() .loose() .catchall(s)` | iguales |
| `.extend() .safeExtend() .merge() .pick({ a: true }) .omit() .partial() .required() .keyof() .shape` | `.extend(o) .safe_extend(o) .merge(o) .pick(&["a"]) .omit(&[..]) .partial() .partial_fields(&[..]) .required() .required_fields(&[..]) .keyof() .shape()` |
| `z.array(s).min().max().length().nonempty()`, `s.array()` | iguales |
| `z.tuple([a, b], rest)` | `z::tuple([a, b]).rest(rest)` |
| `z.record(k, v)`, `z.partialRecord(k, v)`, `z.looseRecord(k, v)` | `z::record_with(k, v)`, `z::partial_record(k, v)`, `z::loose_record(k, v)` (`z::record(v)` usa claves string) |
| `z.union([..])`, `z.xor([..])`, `z.discriminatedUnion("k", [..])`, `z.intersection(a, b)` | `z::union([..])`, `z::xor([..])`, `z::discriminated_union("k", [..])`, `z::intersection(a, b)`; `a.or(b)`, `a.and(b)` |
| `.optional() .exactOptional() .nullable() .nullish() .nonoptional() .readonly() .brand()` | iguales (`.exact_optional()`) |
| `.default(v) .default(() => v) .prefault(v) .catch(v) .catch((ctx) => v)` | `.default(v) .default_fn(\|\| v) .prefault(v) .catch(v) .catch_fn(\|ctx\| v)` |
| `.refine(fn, { message, path, abort, when, params })` | `.refine(fn, "msg")`, `.refine(fn, z::params().message(..).path([..]).abort().when(..).params(json!(..)))`, `.refine_at(fn, msg, [..])` |
| `.superRefine((v, ctx) => ctx.addIssue({...}))`, `.check((ctx) => ...)`, `.overwrite(fn)` | `.super_refine(\|v, ctx\| ctx.add(z::issue().code(..).message(..).path([..])))`, `.check_fn(\|payload\| ..)`, `.overwrite(\|v\| ..)` |
| `.transform(fn)`, `.transform((v, ctx) => { ctx.addIssue(..); return z.NEVER })` | `.transform(\|v\| ..)`, `.try_transform(\|v, ctx\| { ctx.add(..); None })` |
| `.pipe(s)`, `z.preprocess(fn, s)`, `z.codec(a, b, { decode, encode })`, `z.invertCodec(c)` | iguales (`z::invert_codec(&c)`); `schema.decode(&v)`, `schema.encode(&v)` |
| `z.lazy(() => s)`, getters recursivos, `z.custom(fn)`, `z.json()`, `z.success(s)` | `z::lazy(f)`, `z::custom(fn, msg)`, `z::json()`, `z::success(s)` |
| `z.coerce.number()/string()/boolean()/bigint()/date()` | `z::coerce::number()/...` (`boolean` es `Boolean(x)`: `"false"` da `true`) |
| `schema.parse / safeParse / parse(v, { reportInput, error })` | `parse`, `safe_parse`, `parse_with(&v, &ParseOptions::new().report_input(true).error(..))`, `parse_as::<T>`, `parse_str`, `is_valid`; con `undefined`: `parse_opt(None, ..)`, `safe_parse_opt(None, ..)` |
| `z.flattenError / treeifyError / prettifyError / formatError`, `error.issues`, `error.message` | `z::flatten_error(&e)` (o `e.flatten()`), `z::treeify_error`, `z::prettify_error`, `z::format_error`, `e.issues`, `e.message()` |
| `z.config({ customError })`, `z.config(z.locales.es())` | `z::configure(\|c\| c.custom_error = Some(..))`, `z::configure(\|c\| c.locale_error = Some(z::locales::es()))` |
| `.describe()`, `.meta({ id, title })`, `z.registry()`, `z.globalRegistry`, `.register(r, meta)` | iguales (`z::Registry::new()`, `z::global_registry()`) |
| `z.toJSONSchema(s, { target, io, unrepresentable, cycles, reused, override })` | `z::to_json_schema(&s, &JsonSchemaOptions::new().target("draft-07").input()...)` o `s.to_json_schema()` |

### Diferencias propias de Rust

- Los valores son `serde_json::Value`. `undefined` es la ausencia de valor (`parse_opt(None, ..)` o una clave que falta) y una salida `undefined` se omite en los objetos, igual que `JSON.stringify`. Los números siguen la semántica de JavaScript (`f64`, `Number(x)`, `String(x)`) y las claves de los objetos se ordenan como en V8 (los índices numéricos primero).
- `.refine()`, `.super_refine()`, `.overwrite()` y `.check_fn()` devuelven un `Schema`, así que los checks del tipo (`.min()`, ...) van antes que ellos. zod también acepta `z.string().refine(..).min(3)`; ese orden no se puede escribir en Rust.
- Lo que zod *lanza* como `Error` (una intersección que no se puede combinar, un `transform` al hacer `encode`) es un `panic!` en Rust. `ZodValidationPipe` y el serializer lo convierten en la misma respuesta que la integración original.
- Schemas recursivos: una función que devuelve el schema y `z::lazy(f)` donde se repite (`fn categoria() -> Schema { z::object! { hijos: z::array(z::lazy(categoria)) }.into() }`). Para que `to_json_schema` produzca `"$ref": "#"` como zod, usa `z::lazy(categoria)` como raíz.
- `z.map`, `z.set`, `z.symbol`, `z.file`, `z.promise`, `z.function` y `z.instanceof` no existen porque describen valores que no viajan en JSON. De los locales solo están `en` (por defecto) y `es`; otros idiomas se agregan con `locale_error` y `z::error_fn`.
- Profundidad de los schemas recursivos (`z.lazy`, `z.json()`): como mucho 128 niveles por parse. Ver [Límites de seguridad](#limites-de-seguridad).

### Comportamientos heredados de zod

Vienen de zod y de su integración con Node.js, y conviene tenerlos presentes:

- Los guards y los interceptors ven el cuerpo **sin validar** (ver [Qué ven los guards y los interceptors](#que-ven-los-guards-y-los-interceptors)).
- Un parámetro cuyo tipo no tiene schema (`zod_dto!`) pasa sin validar por `ZodValidationPipe::auto()`.
- `.strict()` no reporta la clave `__proto__`: la descarta en silencio, igual que zod. Con claves duplicadas en el JSON gana la última.
- `z::coerce::number()` convierte `""` y `null` en `0`; `z::coerce::boolean()` es `Boolean(x)`, así que `"false"` da `true` (para textos usa `z::stringbool()`).
- `z::jwt()` solo comprueba la forma del token; no verifica la firma.
- Única divergencia conocida: un número que no cabe en un `f64` (`1e309`) llega como `null` (JSON no tiene `Infinity`), así que `z::number().nullable()` lo acepta; en Node sería `Infinity` y zod lo rechazaría.

### Equivalencias con la integración de Node.js

| Node.js (integración zod 5) | Corvis |
|---|---|
| `class Dto extends createZodDto(schema)`, `{ codec: true }`, `Dto.create(input)` | `zod_dto!(Dto, schema)`, `zod_dto!(Dto, schema, codec = true)`, `Dto::create(&input)` |
| `new ZodValidationPipe(schemaOrDto)` | `ZodValidationPipe::new(schema)`, `ZodValidationPipe::dto::<Dto>()` |
| `{ provide: APP_PIPE, useClass: ZodValidationPipe }` | `app.use_global_pipes(ZodValidationPipe::auto())` |
| `createZodValidationPipe({ createValidationException, strictSchemaDeclaration })` | `z::create_zod_validation_pipe(ZodValidationPipeOptions::new().create_validation_exception(\|e\| ..).strict_schema_declaration(true))` |
| `ZodValidationException`, `getZodError()` | `z::ZodValidationException::new(e)`, `ZodValidationException::get_zod_error(&ex)` |
| `ZodSerializationException`, `ZodSchemaDeclarationException` | iguales (`z::...::new(..)`) |
| `@ZodSerializerDto(Dto)` / `([Dto])` + `ZodSerializerInterceptor` | `#[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, z::ZodSerializerDto::dto::<Dto>())]` (`dto_array`, `schema`, `schema_array`) + `app.use_global_interceptors(z::ZodSerializerInterceptor::new())` (ver [Serialización](/es/application/serialization)) |
| `createZodSerializerInterceptor({ reportInput })` | `z::ZodSerializerInterceptor::create(Some(true))` |
| `@ZodResponse({ status, type })` | `#[apply_decorators(corvis_swagger::decorators::zod_response::<Dto>(Some(status), None))]`: código de estado + `ZodSerializerDto` + la respuesta OpenAPI (`Dto_Output`) |
| `ZodGuard`, `UseZodGuard('body', Dto)`, `createZodGuard({ createValidationException })` | `#[use_guards(z::ZodGuard::dto::<Dto>(z::ZodGuardSource::Body))]`, `.create_validation_exception(..)` |
| `validate(value, schemaOrDto, createValidationException?)` | `z::validate(&value, &schema, None)` |

### Cómo se verifica la fidelidad

La fidelidad se verifica contra los paquetes reales: `tests/fixtures/zod4/cases.mjs` ejecuta zod 4.4.3 (y su build `zod/v3`) sobre casi 300 schemas con unas 600 entradas y casi 100 casos de `toJSONSchema`, y `tests/fixtures/zod4/pipe.mjs` ejecuta la integración original 5.5.0. `tests/zod4_fixtures.rs` y `tests/zod_pipe.rs` exigen el mismo resultado, incluido el orden de las claves.

## Qué ven los guards y los interceptors

Los pipes validan el argumento del handler; no reemplazan `req.body()`. Un guard o un interceptor que lee el cuerpo ve el valor **sin validar**. Si un guard necesita datos ya validados, valida ahí mismo o mueve la lógica al handler.

## Límites de seguridad

- **Profundidad de los schemas recursivos.** `z::lazy` y `z::json()` admiten como mucho 128 niveles de anidamiento por parse. Pasado ese límite, el parse no sigue recursando: devuelve un issue `custom` con el mensaje `Too deeply nested: the input exceeds the maximum depth of 128` en la ruta del primer valor que lo supera (sin `input`, para no copiar el subárbol). zod no tiene este límite: en Node.js el parse lanza `RangeError: Maximum call stack size exceeded`; en Rust un desbordamiento de pila aborta el proceso entero, así que el límite es obligatorio. Las uniones no lo envuelven en `invalid_union`, así que llega tal cual al `400` de `ZodValidationPipe`. Se cambia con `z::configure(|c| c.max_depth = n)` (global, `z::DEFAULT_MAX_DEPTH` = 128) o por parse con `ParseOptions::new().max_depth(n)`. Con 128 niveles hay margen: un árbol recursivo típico necesita unos 670 niveles para agotar los 2 MiB de pila de un worker de tokio en release (unos 220 en debug).
- **Costo lineal.** Objetos, records, arrays y tuplas mueven cada valor a su hijo en lugar de copiar el subárbol en cada nivel (antes, 3000 niveles tardaban minutos; ahora, milisegundos), y `z::json()` valida una entrada JSON en una sola pasada con el mismo resultado que la unión de `z.lazy` de zod. Solo se copia un valor cuando un issue lo necesita.
- **Expresiones regulares.** Los patrones sin lookarounds ni backreferences (incluidos los que usan `\b`) se ejecutan con el crate `regex`, en tiempo lineal. Los que los necesitan siguen en el motor con backtracking de `fancy-regex`, igual que en V8, con un tope de 1 000 000 de pasos por `test`: al alcanzarlo el texto se considera no válido y se registra un `WARN` de `[JsRegex]`. Evita backreferences y lookarounds en los `.regex()` que se aplican a texto del cliente.
- **`#[array_unique]`** compara con una tabla hash (lineal) en lugar de por pares.
- `z::url()` acepta cualquier esquema (`javascript:` incluido); para enlaces que se van a mostrar usa `z::http_url()`.
