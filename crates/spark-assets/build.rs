use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let frontend_dist = manifest_dir.join("../../frontend/dist");
    let frontend_embed_dir = if frontend_dist.join("index.html").is_file() {
        frontend_dist
    } else {
        write_fallback_frontend_dist(&manifest_dir)
    };

    println!(
        "cargo:rustc-env=SPARK_FRONTEND_DIST_DIR={}",
        frontend_embed_dir.display()
    );

    for path in [
        "../../frontend/dist",
        "../../frontend/dist/index.html",
        "../../frontend/index.html",
        "../../frontend/public/assets/spark-app-icon.png",
        "../../src/spark/flows",
        "../../src/spark/guides",
        "../../src/unified_llm/data/models.json",
        "../../assets",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn write_fallback_frontend_dist(manifest_dir: &Path) -> PathBuf {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    let fallback_root = out_dir.join("frontend-dist-fallback");
    let assets_dir = fallback_root.join("assets");
    fs::create_dir_all(&assets_dir).expect("fallback asset dir");
    fs::write(
        fallback_root.join("index.html"),
        concat!(
            "<!doctype html>\n",
            "<html lang=\"en\">\n",
            "  <head>\n",
            "    <meta charset=\"UTF-8\" />\n",
            "    <link rel=\"icon\" type=\"image/png\" href=\"/assets/spark-app-icon.png\" />\n",
            "    <link rel=\"shortcut icon\" type=\"image/png\" href=\"/favicon.ico\" />\n",
            "    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\n",
            "    <title>Spark</title>\n",
            "  </head>\n",
            "  <body>\n",
            "    <div id=\"root\"></div>\n",
            "  </body>\n",
            "</html>\n",
        ),
    )
    .expect("fallback index");
    fs::copy(
        manifest_dir.join("../../frontend/public/assets/spark-app-icon.png"),
        assets_dir.join("spark-app-icon.png"),
    )
    .expect("fallback favicon");
    fallback_root
}
