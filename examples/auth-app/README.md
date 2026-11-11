# auth-app

A small application that is only about authentication. It uses every built-in strategy of `corvis::auth`, combined the
way the [authentication guide](https://corvis.randygil.com/security/authentication) describes, and it runs with one
command and no external account.

| Strategy | Where it is used |
|---|---|
| `local` (username and password, argon2id, one legacy bcrypt hash) | `POST /auth/login`, `POST /session/login` |
| `jwt` (bearer access token, 15 minutes) | `GET /me`, `GET /reports`, `GET /admin/users` |
| `api-key` (the key of a service, `X-Api-Key`) | `GET /reports` |
| `session` (server-side session, `__Host-sid` cookie) | `GET /session/me`, `POST /session/logout` |
| OAuth2 (any provider, `state` in a signed cookie, PKCE) | `GET /auth/provider`, `GET /auth/provider/callback` |

The guide's code samples are copied from this crate: each one starts with a comment that names its file, and a test
(`tests/docs_samples.rs`) fails when a sample and its file drift apart.

## Run it

```sh
cargo run -p auth-app -- --dev
```

`--dev` generates the secrets for this run, starts a mock OAuth2 provider on a free port of `127.0.0.1`, serves the
application on `127.0.0.1:3000` (set `PORT` to change it), and prints the demo users and the service API key. The
secrets are lost when the process stops, so every token and cookie of a previous run is refused. Cookies are not marked
`Secure` in this mode (plain HTTP on loopback), and the startup log says so.

Nothing ever listens on another interface, and nothing contacts the network.

### The demo users

| User | Password | Roles | Stored hash |
|---|---|---|---|
| `john` | `john-demo-pass` | admin | argon2id |
| `maria` | `maria-demo-pass` | user | argon2id |
| `walt` | `walt-demo-pass` | user | bcrypt, as a Node application with `bcryptjs` made it: replaced by an argon2id hash the first time `walt` logs in |

The user table is in memory (`src/users.rs`): a real application reads it from a database. The passwords above are for
the demo only; only their hashes are in the source.

## The routes

| Route | Guard | Shows |
|---|---|---|
| `GET /` | `#[public]` | a public route |
| `POST /auth/login` | `AuthGuard::new("local")`, throttled per client and per account | login for an access token |
| `GET /me` | `AuthGuard::new("jwt")` | a token route, `User<T>` |
| `GET /reports` | `AuthGuard::chain(["jwt", "api-key"])` | a chain: people and machine clients |
| `POST /session/login` | origin check, `AuthGuard::new("local").log_in()`, throttled | session login |
| `GET /session/me` | `AuthGuard::new("session")` | a session route |
| `POST /session/logout` | origin check, `AuthGuard::new("session")` | `Authenticator::log_out` |
| `GET /auth/provider`, `GET /auth/provider/callback` | `AuthGuard::new("provider").log_in()`, throttled | OAuth2 with PKCE and a state cookie, then a session |
| `GET /feed` | `#[public]` and an optional chain `["jwt", "session"]` | personalizing a public route |
| `GET /admin/users` | `AuthGuard::chain(["jwt", "session"])`, then `RolesGuard` | authorization after authentication |

There is no global `AuthGuard`. Each route states its guard, and `require_route_posture()` stops the application at
startup when a route has none and is not marked `#[public]`. The guide shows the global pattern.

## Walkthrough with `curl`

Run these in a second terminal while `--dev` is running. `KEY` is the service API key printed at startup.

```sh
B=http://127.0.0.1:3000
KEY=<the key printed at startup>

# A public route.
curl -s $B/
# {"name":"auth-app","login":"POST /auth/login","docs":"see README.md"}

# Password login for a token, and the token route.
TOKEN=$(curl -s -X POST $B/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"john","password":"john-demo-pass"}' | sed 's/.*"access_token":"\([^"]*\)".*/\1/')
curl -s $B/me -H "Authorization: Bearer $TOKEN"
# {"id":1,"username":"john","roles":["admin"]}

# Without a token, or with a wrong password: the same plain 401.
curl -s $B/me
# {"message":"Unauthorized","statusCode":401}
curl -s -X POST $B/auth/login -H 'Content-Type: application/json' -d '{"username":"john","password":"wrong"}'
# {"message":"Unauthorized","statusCode":401}

# A chain: a user's token or the service key.
curl -s $B/reports -H "X-Api-Key: $KEY"
# {"caller":{"service":"reports"}}
curl -s $B/reports -H "Authorization: Bearer $TOKEN"
# {"caller":{"id":1,"username":"john","roles":["admin"]}}
curl -si $B/reports -H "X-Api-Key: wrong" | head -1
# HTTP/1.1 401 Unauthorized

# An optional guard on a public route.
curl -s $B/feed                                   # {"message":"Hello, stranger"}
curl -s $B/feed -H "Authorization: Bearer $TOKEN" # {"message":"Hello, john"}

# A role: john is an admin, maria is not.
curl -s $B/admin/users -H "Authorization: Bearer $TOKEN"
# [{"id":1,"username":"john","roles":["admin"]},{"id":2,"username":"maria",...},...]
MARIA=$(curl -s -X POST $B/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"maria","password":"maria-demo-pass"}' | sed 's/.*"access_token":"\([^"]*\)".*/\1/')
curl -si $B/admin/users -H "Authorization: Bearer $MARIA" | head -1
# HTTP/1.1 403 Forbidden

# A session: the cookie jar keeps the session cookie.
curl -s -c jar -X POST $B/session/login -H 'Content-Type: application/json' \
  -d '{"username":"maria","password":"maria-demo-pass"}'
# {"username":"maria"}
curl -s -b jar $B/session/me
# {"id":2,"username":"maria","roles":["user"]}
curl -si -b jar -X POST $B/session/logout | head -1
# HTTP/1.1 204 No Content
curl -si -b jar $B/session/me | head -1
# HTTP/1.1 401 Unauthorized

# A login that a browser makes for another site is refused before the password is checked.
curl -si -X POST $B/session/login -H 'Sec-Fetch-Site: cross-site' -H 'Content-Type: application/json' \
  -d '{"username":"maria","password":"maria-demo-pass"}' | head -1
# HTTP/1.1 403 Forbidden

# OAuth2 against the mock provider: -L follows the redirects (the provider approves everything), and the
# cookie jar carries the state cookie to the callback.
curl -s -L -c jar -b jar $B/auth/provider
# {"username":"provider:Mock Person"}
curl -s -b jar $B/session/me
# {"id":1000,"username":"provider:Mock Person","roles":["user"]}
```

After five attempts for one account in five minutes (counted together on `POST /auth/login` and `POST /session/login`),
or ten login attempts from one address in a minute on one route, the login routes answer `429`. Restart the application and the token and the cookie above are refused: the secrets changed.

## Against a real provider

Set the OAuth2 variables to the provider's endpoints and your client (see `.env.example`) and run without `--dev`. The
same code logs in against it: `src/auth/provider.rs` takes everything from `Settings`. Register
`OAUTH_REDIRECT_URI` with the provider exactly as you set it. The user is found by `(provider, id)`, and an existing
account is linked only through an e-mail address the provider reports as verified.

## Run it without `--dev`

There are no default secrets. Copy `.env.example`, give every variable a value (a secret needs at least 32 bytes:
`openssl rand -hex 32`; the four secrets must differ from one another), load the file into the environment and run:

```sh
set -a; . ./.env; set +a
cargo run -p auth-app
```

The application refuses to start, naming the variable, when one is missing or too short. Without `--dev` the cookies
are `Secure` with the `__Host-` prefix, so a browser sends them only over HTTPS: put a reverse proxy that ends TLS in
front of it, and tell the application about the proxy: set `TRUST_PROXY` (read by `src/main.rs`, which calls
`app.set_trust_proxy(..)`) to the number of proxies in front of it (`1`) or a list of their addresses. Without it every client has the proxy's address and shares one rate-limit
counter. The origin check also assumes the public site is `https` outside `--dev`. The session store is in memory (one process only); see the guide for what a deployment needs.

## Tests

```sh
cargo test -p auth-app
```

They need neither the network nor Node. `tests/flows.rs` covers every flow above, including the OAuth2 login through
the mock provider and the linking rule; `tests/posture.rs`, `tests/throttle.rs`, `tests/csrf.rs`,
`tests/production.rs`, `tests/startup.rs` and `tests/startup_log.rs` cover the rest.
