//! The application refuses to start without `JWT_SECRET` (no well-known default secret).

#[corvis::test]
async fn refuses_to_start_without_jwt_secret() {
    // SAFETY: this test binary has a single test and sets the environment before the app reads it.
    unsafe {
        std::env::remove_var("JWT_SECRET");
    }
    let dir = std::env::temp_dir().join(format!("cats-app-startup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // No `.env` in the working directory either.
    std::env::set_current_dir(&dir).unwrap();
    let err = cats_app::create_app().await.err().expect("the application started without JWT_SECRET");
    assert!(format!("{err:#}").contains("JWT_SECRET"), "{err:#}");
}
