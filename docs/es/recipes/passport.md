---
status: unavailable
---

# Passport (autenticación)

Corvis **no tiene una integración con Passport** ni un sistema equivalente de "estrategias" de autenticación intercambiables (local, JWT, OAuth...). Tampoco hay todavía clientes OAuth2/OpenID Connect integrados.

## Alternativa: `JwtModule` + guards

Las piezas para autenticar con tokens están en el framework y cubren el caso más común, el de una API con JWT:

- `JwtModule::register(JwtModuleOptions::new(secret))` o `JwtModule::register_async(global, |ctx| ...)` para configurar la firma;
- `JwtService::sign` y `JwtService::verify` (o `verify_or_unauthorized`, que ya devuelve un `401`);
- `extract_bearer_token(&request)` para leer la cabecera `Authorization: Bearer ...`;
- un [guard](/es/guards) que verifica el token y deja el usuario en la request con `request.insert(user)`;
- un parámetro `FromContext` (el equivalente a un decorador `@User()`) que lo recupera en el handler.

El guard de `examples/tertulia` (`src/auth/auth_guard.rs`) hace exactamente eso:

```rust
#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

El flujo completo (login, guard, usuario actual, roles) se explica en [Autenticación](/es/security/authentication) y [Autorización](/es/security/authorization). Para un inicio de sesión con usuario y contraseña, el "paso local" de Passport es un endpoint propio que verifica la contraseña (ver [Cifrado y hashing](/es/security/encryption-and-hashing)) y devuelve el token.
