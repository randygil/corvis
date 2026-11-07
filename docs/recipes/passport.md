---
status: unavailable
---

# Passport (authentication)

Corvis **has no Passport integration** and no equivalent system of interchangeable authentication "strategies" (local, JWT, OAuth...). There are also no built-in OAuth2/OpenID Connect clients yet.

## Alternative: `JwtModule` + guards

The pieces for authenticating with tokens are in the framework and cover the most common case, an API with JWT:

- `JwtModule::register(JwtModuleOptions::new(secret))` or `JwtModule::register_async(global, |ctx| ...)` to configure signing;
- `JwtService::sign` and `JwtService::verify` (or `verify_or_unauthorized`, which already returns a `401`);
- `extract_bearer_token(&request)` to read the `Authorization: Bearer ...` header;
- a [guard](/guards) that verifies the token and leaves the user on the request with `request.insert(user)`;
- a `FromContext` parameter (the equivalent of a `@User()` decorator) that retrieves it in the handler.

The `examples/tertulia` guard (`src/auth/auth_guard.rs`) does exactly that:

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

The full flow (login, guard, current user, roles) is explained in [Authentication](/security/authentication) and [Authorization](/security/authorization). For a username and password login, Passport's "local step" is an endpoint of your own that verifies the password (see [Encryption and hashing](/security/encryption-and-hashing)) and returns the token.
