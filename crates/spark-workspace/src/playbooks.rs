//! Mission playbooks: `$SPARK_HOME/playbooks/<name>.md`, Markdown with YAML
//! frontmatter (`title`, `description`) and free-form instructions in the body.
use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use spark_common::settings::SparkSettings;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playbook {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The instructions: the file's body after the frontmatter. Omitted
    /// from listings.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

fn dir(settings: &SparkSettings) -> std::path::PathBuf {
    settings.data_dir.join("playbooks")
}

fn parse(name: &str, source: &str) -> WorkspaceResult<Playbook> {
    let invalid =
        || WorkspaceError::Validation(format!("Playbook `{name}` needs YAML frontmatter"));
    let rest = source.strip_prefix("---").ok_or_else(invalid)?;
    let (front, body) = rest.split_once("\n---").ok_or_else(invalid)?;
    let mut playbook: Playbook = serde_yaml::from_str(front)
        .map_err(|error| WorkspaceError::Validation(format!("Playbook `{name}`: {error}")))?;
    playbook.name = name.into();
    playbook.text = body
        .split_once('\n')
        .map_or("", |(_, text)| text)
        .trim()
        .into();
    Ok(playbook)
}

/// Installed playbooks without their text, sorted by name; unreadable files
/// are skipped.
pub fn list(settings: &SparkSettings) -> WorkspaceResult<Vec<Playbook>> {
    let Ok(entries) = std::fs::read_dir(dir(settings)) else {
        return Ok(vec![]);
    };
    let mut playbooks: Vec<Playbook> = entries
        .filter_map(|entry| {
            let name = entry
                .ok()?
                .file_name()
                .to_str()?
                .strip_suffix(".md")?
                .to_string();
            let playbook = get(settings, &name).ok()?;
            Some(Playbook {
                text: String::new(),
                ..playbook
            })
        })
        .collect();
    playbooks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(playbooks)
}

pub fn get(settings: &SparkSettings, name: &str) -> WorkspaceResult<Playbook> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(WorkspaceError::Validation(format!(
            "Invalid playbook name `{name}`"
        )));
    }
    let source = std::fs::read_to_string(dir(settings).join(format!("{name}.md")))
        .map_err(|_| WorkspaceError::NotFound(format!("Unknown playbook `{name}`")))?;
    parse(name, &source)
}
