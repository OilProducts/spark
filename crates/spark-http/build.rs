//! Stamps `SPARK_BUILD_ID`, the build id a binary announces when it cannot
//! hash its own executable: the `SPARK_BUILD_ID` set for the build, or else
//! the time this script ran.
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Any rerun-if line replaces Cargo's default of rerunning on any package
    // change, so name the inputs that make a new stamp.
    // ponytail: a change only in another crate keeps this stamp; it is the
    // fallback, behind the executable hash.
    for path in ["src", "Cargo.toml"] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=SPARK_BUILD_ID");
    let id = std::env::var("SPARK_BUILD_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| {
            let since = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before 1970");
            format!("{:x}", since.as_nanos())
        });
    println!("cargo:rustc-env=SPARK_BUILD_ID={id}");
}
