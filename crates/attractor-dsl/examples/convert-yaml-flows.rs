#![forbid(unsafe_code)]

//! Converts retired YAML flow files into flow directories, using the same
//! writer as editor saves.
//!
//! cargo run -p attractor-dsl --example convert-yaml-flows -- <yaml-root> <flows-dir>
//!
//! Each `<yaml-root>/<name>.yaml` becomes `<flows-dir>/<name>/`. Flows whose
//! directory already exists are skipped and reported. Subflow `flow_ref`s
//! drop their extension. Each written directory is read back and must equal
//! the converted flow.

use std::path::{Path, PathBuf};

use attractor_core::{FlowDefinition, NodeConfig};

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (yaml_root, flows_dir) = match args.as_slice() {
        [yaml_root, flows_dir, ..] => (PathBuf::from(yaml_root), PathBuf::from(flows_dir)),
        _ => {
            eprintln!("usage: convert-yaml-flows <yaml-root> <flows-dir>");
            std::process::exit(2);
        }
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
    // Round trip: the written directory must assemble back to the same flow.
    let written =
        attractor_dsl::read_flow_dir(target).map_err(|error| error.detail().to_string())?;
    if written != flow {
        return Err(format!(
            "{} does not assemble back to the converted flow",
            target.display()
        ));
    }
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
