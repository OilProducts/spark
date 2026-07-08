use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::environment::{CommandOptions, ExecutionEnvironment};
use crate::profiles::{instruction_provider_family, InstructionProviderFamily, ProviderProfile};

pub const PROJECT_INSTRUCTION_BYTE_BUDGET: usize = 32 * 1024;
pub const PROJECT_INSTRUCTION_TRUNCATION_MARKER: &str = "[Project instructions truncated at 32KB]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDocument {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub truncated: bool,
}

impl ProjectDocument {
    pub fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            path: normalize_path_text(path.into()),
            content: content.into(),
            truncated: false,
        }
    }

    pub fn truncated(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            path: normalize_path_text(path.into()),
            content: content.into(),
            truncated: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProjectDocuments {
    #[serde(default)]
    pub documents: Vec<ProjectDocument>,
    #[serde(default)]
    pub truncated: bool,
}

impl ProjectDocuments {
    pub fn new(documents: Vec<ProjectDocument>, truncated: bool) -> Self {
        Self {
            documents,
            truncated,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProjectDocument> {
        self.documents.iter()
    }
}

impl IntoIterator for ProjectDocuments {
    type Item = ProjectDocument;
    type IntoIter = std::vec::IntoIter<ProjectDocument>;

    fn into_iter(self) -> Self::IntoIter {
        self.documents.into_iter()
    }
}

impl<'a> IntoIterator for &'a ProjectDocuments {
    type Item = &'a ProjectDocument;
    type IntoIter = std::slice::Iter<'a, ProjectDocument>;

    fn into_iter(self) -> Self::IntoIter {
        self.documents.iter()
    }
}

pub fn discover_project_documents(
    environment: &ExecutionEnvironment,
    profile: &ProviderProfile,
) -> ProjectDocuments {
    discover_project_documents_with_budget(environment, profile, PROJECT_INSTRUCTION_BYTE_BUDGET)
}

pub fn discover_project_documents_with_budget(
    environment: &ExecutionEnvironment,
    profile: &ProviderProfile,
    budget_bytes: usize,
) -> ProjectDocuments {
    apply_budget(
        collect_project_documents(environment, profile),
        budget_bytes,
    )
}

pub fn render_project_documents(project_documents: &ProjectDocuments) -> String {
    if project_documents.is_empty() {
        return String::new();
    }

    let mut sections = project_documents
        .documents
        .iter()
        .map(|document| format!("### {}\n{}", document.path, document.content))
        .collect::<Vec<_>>();
    if project_documents.truncated || project_documents.documents.iter().any(|doc| doc.truncated) {
        sections.push(PROJECT_INSTRUCTION_TRUNCATION_MARKER.to_string());
    }
    sections.join("\n\n")
}

pub fn load_project_documents(
    environment: &ExecutionEnvironment,
    profile: &ProviderProfile,
) -> String {
    render_project_documents(&discover_project_documents(environment, profile))
}

fn collect_project_documents(
    environment: &ExecutionEnvironment,
    profile: &ProviderProfile,
) -> Vec<ProjectDocument> {
    let working_directory_text = normalize_path_text(environment.working_directory());
    let working_directory = canonicalize_path(resolve_path_text(&working_directory_text));
    let git_root_text = exec_command_candidates(
        environment,
        "git rev-parse --show-toplevel",
        &working_directory_text,
    );
    let root = git_root_text
        .as_deref()
        .map(resolve_path_text)
        .map(canonicalize_path)
        .unwrap_or_else(|| working_directory.clone());
    let filenames = recognized_filenames(instruction_provider_family(profile));
    let directories = path_chain(&root, &working_directory);

    let mut documents = Vec::new();
    for directory in directories {
        for filename in &filenames {
            let target = directory.join(filename);
            for candidate in candidate_path_texts(&target, &working_directory) {
                if !environment.file_exists(&candidate) {
                    continue;
                }
                let Ok(content) = environment.read_file(&candidate, None, None) else {
                    continue;
                };
                documents.push(ProjectDocument::new(display_path(&root, &target), content));
                break;
            }
        }
    }
    documents
}

fn recognized_filenames(provider_family: Option<InstructionProviderFamily>) -> Vec<&'static str> {
    match provider_family {
        Some(InstructionProviderFamily::OpenAI) => vec!["AGENTS.md", ".codex/instructions.md"],
        Some(InstructionProviderFamily::Anthropic) => vec!["AGENTS.md", "CLAUDE.md"],
        Some(InstructionProviderFamily::Gemini) => vec!["AGENTS.md", "GEMINI.md"],
        None => vec!["AGENTS.md"],
    }
}

fn apply_budget(documents: Vec<ProjectDocument>, budget_bytes: usize) -> ProjectDocuments {
    let mut loaded_documents = Vec::new();
    let mut remaining_bytes = budget_bytes;
    let mut truncated = false;

    for document in documents {
        if document.truncated {
            loaded_documents.push(document);
            truncated = true;
            continue;
        }
        if remaining_bytes == 0 {
            truncated = true;
            break;
        }

        let encoded_size = document.content.len();
        if encoded_size <= remaining_bytes {
            remaining_bytes -= encoded_size;
            loaded_documents.push(document);
            continue;
        }

        loaded_documents.push(ProjectDocument::truncated(
            document.path,
            truncate_to_byte_budget(&document.content, remaining_bytes),
        ));
        truncated = true;
        break;
    }

    ProjectDocuments::new(loaded_documents, truncated)
}

fn truncate_to_byte_budget(text: &str, remaining_bytes: usize) -> String {
    if remaining_bytes == 0 {
        return String::new();
    }
    if text.len() <= remaining_bytes {
        return text.to_string();
    }

    let mut end = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        if next > remaining_bytes {
            break;
        }
        end = next;
    }
    text[..end].to_string()
}

fn exec_command_candidates(
    environment: &ExecutionEnvironment,
    command: &str,
    working_directory: &str,
) -> Option<String> {
    let mut candidates = vec![working_directory.to_string()];
    let resolved = resolve_path_text(working_directory)
        .to_string_lossy()
        .to_string();
    if !candidates.contains(&resolved) {
        candidates.push(resolved);
    }

    candidates.into_iter().find_map(|candidate| {
        let result = environment
            .exec_command(
                command,
                CommandOptions {
                    working_dir: Some(PathBuf::from(candidate)),
                    ..CommandOptions::default()
                },
            )
            .ok()?;
        if result.exit_code != 0 {
            return None;
        }
        let output = result.stdout.trim();
        (!output.is_empty()).then(|| output.to_string())
    })
}

fn path_chain(root: &Path, working_directory: &Path) -> Vec<PathBuf> {
    let Ok(relative) = working_directory.strip_prefix(root) else {
        return vec![working_directory.to_path_buf()];
    };

    let mut directories = vec![root.to_path_buf()];
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        directories.push(current.clone());
    }
    directories
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn candidate_path_texts(target: &Path, working_directory: &Path) -> Vec<String> {
    let mut candidates = Vec::new();
    for candidate in [
        relative_path_text(target, working_directory),
        target.to_string_lossy().to_string(),
    ] {
        let normalized = normalize_path_text(candidate);
        if !candidates.contains(&normalized) {
            candidates.push(normalized);
        }
    }
    candidates
}

fn relative_path_text(target: &Path, base: &Path) -> String {
    let target_components = normal_components(target);
    let base_components = normal_components(base);
    let common = target_components
        .iter()
        .zip(base_components.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let mut parts = Vec::new();
    parts.extend(std::iter::repeat("..".to_string()).take(base_components.len() - common));
    parts.extend(target_components.into_iter().skip(common));
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn normal_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            Component::ParentDir => Some("..".to_string()),
            _ => None,
        })
        .collect()
}

// Resolve symlinks so the git root (physical path, e.g. /private/var on macOS)
// and the working directory (possibly symlinked, e.g. /var) share a prefix for
// path_chain and display_path. Falls back to the input for virtual backends
// whose paths do not exist on the host filesystem.
fn canonicalize_path(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn resolve_path_text(path_text: &str) -> PathBuf {
    let path = PathBuf::from(path_text);
    if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path),
        )
    }
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn normalize_path_text(path: impl Into<String>) -> String {
    path.into().replace('\\', "/")
}
