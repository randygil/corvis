use corvis::prelude::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = cats_app::create_app().await?;
    app.enable_shutdown_hooks();

    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
    // Always bind to the loopback interface.
    app.listen(format!("127.0.0.1:{port}")).await
}
