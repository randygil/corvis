---
status: no-disponible
---

# Sesión

Corvis no tiene sesiones de servidor: no hay un middleware equivalente a `express-session`, ni un
almacén de sesiones, ni un objeto `req.session`.

Alternativas reales, según lo que necesites:

- **Autenticación sin estado.** Un JWT firmado con `JwtService` y enviado en
  `Authorization: Bearer` cubre la mayoría de las APIs. Ver [Autenticación](/es/security/authentication).
- **Sesión propia con cookie.** Emite una cookie `HttpOnly` con un identificador aleatorio
  (`res.cookie(...)`, ver [Cookies](/es/http/cookies)), guarda los datos en un provider inyectable (un
  mapa en memoria o tu base de datos con [corvis-orm](/es/data/corvis-orm)) y léela en un guard con
  `req.cookie("sid")`. El guard puede adjuntar los datos a la request con `req.insert(valor)` para que
  el handler los lea con un parámetro `FromContext`.
- **Datos que solo viven durante una petición.** `req.insert(valor)` y `req.get::<T>()` (las
  extensiones de la request) sirven para pasar datos entre middleware, guards, interceptors y el
  handler. No sobreviven a la petición.

Si guardas sesiones en memoria, recuerda que cada instancia de la aplicación tiene su propio mapa y
que se pierden al reiniciar.
