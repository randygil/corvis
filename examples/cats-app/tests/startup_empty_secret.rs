//! An empty or blank `JWT_SECRET` is rejected like a missing one.

#[corvis::test]
async fn refuses_to_start_with_an_empty_jwt_secret() {
    // SAFETY: this test binary has a single test and sets the environment before the app reads it.
    unsafe {
        std::env::set_var("JWT_SECRET", "");
    }
    let dir = std::env::temp_dir().join(format!("cats-app-empty-secret-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let err = cats_app::create_app().await.err().expect("the application started with an empty JWT_SECRET");
    assert!(format!("{err:#}").contains("JWT_SECRET"), "{err:#}");

    // SAFETY: same as above, a single test in this binary.
    unsafe {
        std::env::set_var("JWT_SECRET", "   ");
    }
    let err = cats_app::create_app().await.err().expect("the application started with a blank JWT_SECRET");
    assert!(format!("{err:#}").contains("JWT_SECRET"), "{err:#}");
}
