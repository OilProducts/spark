//! Keeps a test process away from the real Spark home and server. Agents'
//! shells export `SPARK_HOME` and `SPARK_API_BASE_URL`, and without them Spark
//! falls back to `~/.spark` and a local default port, so every integration test
//! binary loads this with `#[path]`. It runs before `main`, ahead of the test
//! threads and of any child process a test starts.
//!
//! A test that wants its own Spark home still sets one on its child or through
//! an env guard; a re-executed test child keeps what its parent chose.
//!
//! ponytail: unix only, like the tests' process controls; add a `.CRT$XCU`
//! section here when the suite runs on Windows.
#![cfg(unix)]

#[used]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
#[cfg_attr(not(target_os = "macos"), link_section = ".init_array")]
static ISOLATE_SPARK_ENV: extern "C" fn() = {
    extern "C" fn isolate() {
        const ISOLATED: &str = "SPARK_TEST_ENV_ISOLATED";
        if std::env::var_os(ISOLATED).is_some() {
            return;
        }
        std::env::set_var(ISOLATED, "1");
        std::env::set_var(
            "SPARK_HOME",
            std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("spark-test-home"),
        );
        std::env::remove_var("SPARK_API_BASE_URL");
    }
    isolate
};
