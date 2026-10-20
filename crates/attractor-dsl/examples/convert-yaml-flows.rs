#![forbid(unsafe_code)]

//! Converts retired YAML flow files into flow directories, using the same
//! writer as editor saves.
//!
//! cargo run -p attractor-dsl --example convert-yaml-flows -- <yaml-root> <flows-dir> [--catalog <flow-catalog.toml>]
//!
//! Each `<yaml-root>/<name>.yaml` becomes `<flows-dir>/<name>/`. Flows whose
//! directory already exists are skipped and reported. Subflow `flow_ref`s
//! drop their extension. `--catalog` rewrites `flows."<name>.yaml"` keys.

use std::path::{Path, PathBuf};

use attractor_core::{FlowDefinition, NodeConfig};

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (yaml_root, flows_dir) = match args.as_slice() {
        [yaml_root, flows_dir, ..] => (PathBuf::from(yaml_root), PathBuf::from(flows_dir)),
        _ => {
            eprintln!(
                "usage: convert-yaml-flows <yaml-root> <flows-dir> [--catalog <flow-catalog.toml>]"
            );
            std::process::exit(2);
        }
    };
    let catalog = match args.get(2).map(String::as_str) {
        Some("--catalog") => args.get(3).map(PathBuf::from),
        _ => None,
    };
    let mut files = Vec::new();
    collect_yaml(&yaml_root, &mut files);
    files.sort();
    let mut failed = false;
    for file in files {
        let relative = file.strip_prefix(&yaml_root).expect("under root");
        let name = strip_extension(&relative.to_string_lossy().replace('\\', "/"));
        let target = flows_dir.join(&name);
        if target.exists() {
            println!("skipped {name}: {} already exists", target.display());
            continue;
        }
        match convert(&file, &target) {
            Ok(()) => println!("converted {name}"),
            Err(error) => {
                failed = true;
                eprintln!("failed {name}: {error}");
            }
        }
    }
    if let Some(catalog) = catalog {
        if let Err(error) = rewrite_catalog(&catalog) {
            failed = true;
            eprintln!("failed to rewrite {}: {error}", catalog.display());
        }
    }
    if failed {
        std::process::exit(1);
    }
}

fn convert(file: &Path, target: &Path) -> Result<(), String> {
    let source = std::fs::read_to_string(file).map_err(|error| error.to_string())?;
    let mut flow = FlowDefinition::from_yaml_str(&source).map_err(|error| error.to_string())?;
    for node in flow.nodes.values_mut() {
        if let Some(NodeConfig::Subflow { flow_ref, .. }) = node.config.as_mut() {
            *flow_ref = strip_extension(flow_ref);
        }
    }
    let flow = flow.normalize();
    attractor_dsl::validate_flow_definition(&flow).map_err(|error| error.to_string())?;
    attractor_dsl::write_flow_dir(target, &flow).map_err(|error| error.detail().to_string())?;
    Ok(())
}

fn rewrite_catalog(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document = text
        .parse::<toml::Table>()
        .map_err(|error| error.to_string())?;
    if let Some(toml::Value::Table(flows)) = document.get_mut("flows") {
        *flows = std::mem::take(flows)
            .into_iter()
            .map(|(name, entry)| (strip_extension(&name), entry))
            .collect();
    }
    let text = toml::to_string_pretty(&document).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())?;
    println!("rewrote flow names in {}", path.display());
    Ok(())
}

fn strip_extension(name: &str) -> String {
    name.strip_suffix(".yaml")
        .or_else(|| name.strip_suffix(".yml"))
        .unwrap_or(name)
        .to_string()
}

fn collect_yaml(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            collect_yaml(&path, files);
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        ) {
            files.push(path);
        }
    }
}
