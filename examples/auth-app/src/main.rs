use auth_app::Settings;
use corvis::prelude::*;

/// `cargo run -p auth-app -- --dev` starts everything on `127.0.0.1` with secrets made for this run and a mock OAuth2
/// provider. Without `--dev` the secrets and the provider come from the environment and nothing has a default.
#[corvis::main]
async fn main() -> corvis::Result<()> {
    let dev = std::env::args().any(|arg| arg == "--dev");
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let log = Logger::new("auth-app");

    let settings = if dev { Settings::dev(port).await? } else { Settings::from_env()? };
    if dev {
        log.warn("--dev: the secrets were generated for this run and are lost when it stops; cookies are not marked Secure");
        if let Some(mock) = settings.mock() {
            log.warn(format!("--dev: a mock OAuth2 provider listens on http://{}", mock.addr()));
        }
        println!("demo users: john / john-demo-pass (admin), maria / maria-demo-pass, walt / walt-demo-pass (legacy bcrypt hash)");
        println!("service API key for X-Api-Key: {}", settings.service_api_key);
    }

    let mut app = auth_app::create_app(settings).await?;
    app.enable_shutdown_hooks();
    // Always bind to the loopback interface.
    app.listen(format!("127.0.0.1:{port}")).await
}
