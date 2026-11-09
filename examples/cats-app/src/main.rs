use corvis::prelude::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = cats_app::create_app().await?;
    app.enable_shutdown_hooks();

    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);

    // Optional HTTPS listener (`cargo run --features tls`): set CATS_TLS_CERT and CATS_TLS_KEY to PEM files.
    #[cfg(feature = "tls")]
    {
        let config = app.get::<ConfigService>()?;
        if let (Some(cert), Some(key)) = (config.get::<String>("CATS_TLS_CERT"), config.get::<String>("CATS_TLS_KEY")) {
            let tls_port: u16 = config.get_or("CATS_TLS_PORT", 3443);
            let tls = corvis::tls::TlsOptions::from_pem_files(cert, key)?;
            app.listen_tls_in_background(format!("127.0.0.1:{tls_port}"), tls).await?;
        }
    }

    // Always bind to the loopback interface.
    app.listen(format!("127.0.0.1:{port}")).await
}
