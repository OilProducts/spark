/// Points HOME at a fresh directory for this test process before the first
/// app starts, so Home's setup never writes into the developer's home. A
/// child test process started with `SPARK_TEST_HOME_SET` keeps the HOME its
/// parent chose.
pub(crate) fn isolate_home() {
    static HOME: std::sync::Once = std::sync::Once::new();
    HOME.call_once(|| {
        if std::env::var_os("SPARK_TEST_HOME_SET").is_some() {
            return;
        }
        let home = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("spark-test-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("test home");
        std::env::set_var("HOME", home);
    });
}
