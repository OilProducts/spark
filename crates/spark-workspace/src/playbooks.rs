//! Mission playbooks: `playbooks/<name>.md`, resolved project first (the org
//! repo), then bundled (`$SPARK_HOME/bundled/playbooks`). Markdown with YAML
//! frontmatter (`title`, `description`) and free-form instructions in the body.
use crate::{WorkspaceError, WorkspaceResult};
use serde::{Deserialize, Serialize};
use spark_common::settings::SparkSettings;
use std::path::Path;

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

/// The roots for `project` (none or blank: bundled only).
fn roots(settings: &SparkSettings, project: Option<&str>) -> attractor_dsl::FlowRoots {
    let project = project.map(str::trim).filter(|path| !path.is_empty());
    attractor_dsl::FlowRoots::new(settings.bundled_dir(), project.map(Path::new))
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

/// The playbooks `project` sees (none: bundled only), without their text,
/// sorted by name; unreadable files are skipped.
pub fn list(settings: &SparkSettings, project: Option<&str>) -> WorkspaceResult<Vec<Playbook>> {
    Ok(roots(settings, project)
        .list_playbook_names()
        .into_iter()
        .filter_map(|name| {
            let playbook = get(settings, project, &name).ok()?;
            Some(Playbook {
                text: String::new(),
                ..playbook
            })
        })
        .collect())
}

pub fn get(
    settings: &SparkSettings,
    project: Option<&str>,
    name: &str,
) -> WorkspaceResult<Playbook> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(WorkspaceError::Validation(format!(
            "Invalid playbook name `{name}`"
        )));
    }
    let (_, source) = roots(settings, project)
        .playbook_text(name)
        .ok_or_else(|| WorkspaceError::NotFound(format!("Unknown playbook `{name}`")))?;
    parse(name, &source)
}
