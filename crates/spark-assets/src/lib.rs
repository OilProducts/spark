#![forbid(unsafe_code)]

//! Bundled resource discovery for the Rust rewrite.
//!
//! Frontend builds, authored flows, guides, model/profile data, icons, service
//! files, and container resources remain in their existing source locations.
//! This crate owns resource discovery for those surfaces.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use include_dir::{include_dir, Dir, DirEntry};

static FRONTEND_DIST: Dir<'_> = include_dir!("$SPARK_FRONTEND_DIST_DIR");
static STARTER_FLOWS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/flows");
static GUIDES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/guides");
static PLAYBOOKS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/playbooks");
static ROLES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/roles");
static ROOT_ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../assets");
static MODEL_CATALOG_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/unified_llm/data/models.json"
));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceSource {
    ExplicitUiDir,
    SourceTree,
    Packaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceFile {
    logical_path: String,
    source: ResourceSource,
    filesystem_path: Option<PathBuf>,
    bytes: Cow<'static, [u8]>,
}

impl ResourceFile {
    fn from_filesystem(
        logical_path: impl Into<String>,
        source: ResourceSource,
        path: PathBuf,
    ) -> Option<Self> {
        let bytes = std::fs::read(&path).ok()?;
        Some(Self {
            logical_path: logical_path.into(),
            source,
            filesystem_path: Some(path),
            bytes: Cow::Owned(bytes),
        })
    }

    fn from_packaged(logical_path: impl Into<String>, bytes: &'static [u8]) -> Self {
        Self {
            logical_path: logical_path.into(),
            source: ResourceSource::Packaged,
            filesystem_path: None,
            bytes: Cow::Borrowed(bytes),
        }
    }

    fn from_packaged_text(logical_path: impl Into<String>, text: &'static str) -> Self {
        Self::from_packaged(logical_path, text.as_bytes())
    }

    pub fn logical_path(&self) -> &str {
        &self.logical_path
    }

    pub fn source(&self) -> ResourceSource {
        self.source
    }

    pub fn filesystem_path(&self) -> Option<&Path> {
        self.filesystem_path.as_deref()
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    pub fn text(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.bytes())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePathError {
    UnsafePath,
}

fn validate_relative_path(relative: &str) -> Result<(), ResourcePathError> {
    let path = Path::new(relative);
    if relative.is_empty() || path.is_absolute() {
        return Err(ResourcePathError::UnsafePath);
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return Err(ResourcePathError::UnsafePath),
        }
    }
    Ok(())
}

fn packaged_file(dir: &'static Dir<'static>, logical_path: &str) -> Option<ResourceFile> {
    validate_relative_path(logical_path).ok()?;
    let file = dir.get_file(logical_path)?;
    Some(ResourceFile::from_packaged(
        logical_path.to_string(),
        file.contents(),
    ))
}

fn collect_file_names(dir: &'static Dir<'static>, output: &mut Vec<String>) {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(child) => collect_file_names(child, output),
            DirEntry::File(file) => {
                output.push(file.path().to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Authored flow resource boundary: each flow is a directory holding
/// `flow.md` and one markdown file per node.
pub mod flows {
    use std::collections::BTreeMap;

    use include_dir::{Dir, DirEntry};

    use super::{validate_relative_path, STARTER_FLOWS};

    const FLOW_FILE: &str = "flow.md";

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct StarterFlowAsset {
        /// File path relative to the flows directory, e.g. `examples/simple-linear/flow.md`.
        pub name: String,
        pub content: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FlowResourceError {
        Missing,
        InvalidName,
        InvalidUtf8,
    }

    /// Names of the bundled flow directories, sorted.
    pub fn starter_flow_names() -> Result<Vec<String>, FlowResourceError> {
        fn collect(dir: &'static Dir<'static>, names: &mut Vec<String>) {
            for child in dir.dirs() {
                if child.get_file(child.path().join(FLOW_FILE)).is_some() {
                    names.push(child.path().to_string_lossy().replace('\\', "/"));
                } else {
                    collect(child, names);
                }
            }
        }
        let mut names = Vec::new();
        collect(&STARTER_FLOWS, &mut names);
        names.sort();
        if names.is_empty() {
            return Err(FlowResourceError::Missing);
        }
        Ok(names)
    }

    /// The files of one bundled flow directory: file name -> text.
    pub fn starter_flow_files(name: &str) -> Result<BTreeMap<String, String>, FlowResourceError> {
        validate_relative_path(name).map_err(|_| FlowResourceError::InvalidName)?;
        let dir = STARTER_FLOWS
            .get_dir(name)
            .filter(|dir| dir.get_file(dir.path().join(FLOW_FILE)).is_some())
            .ok_or(FlowResourceError::Missing)?;
        dir.files()
            .map(|file| {
                let file_name = file
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or(FlowResourceError::InvalidName)?;
                let text = file.contents_utf8().ok_or(FlowResourceError::InvalidUtf8)?;
                Ok((file_name.to_string(), text.to_string()))
            })
            .collect()
    }

    /// The manifest recording the packaged version last installed for each file.
    pub const PACKAGED_FLOW_HASHES: &str = ".packaged-flow-hashes.json";

    /// Installed flows that seeding keeps as local edits: some file differs
    /// from the bundled copy and from the version Spark last installed, so
    /// newer bundled versions never reach it.
    pub fn locally_edited_flows(flows_dir: &std::path::Path) -> Vec<String> {
        use sha2::{Digest, Sha256};
        let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        let manifest: BTreeMap<String, String> =
            std::fs::read(flows_dir.join(PACKAGED_FLOW_HASHES))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
        let mut edited = starter_flow_assets()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|asset| {
                let installed = hash(&std::fs::read(flows_dir.join(&asset.name)).ok()?);
                (installed != hash(asset.content.as_bytes())
                    && manifest.get(&asset.name) != Some(&installed))
                .then(|| {
                    asset
                        .name
                        .rsplit_once('/')
                        .map_or(asset.name.clone(), |(flow, _)| flow.to_string())
                })
            })
            .collect::<Vec<_>>();
        edited.dedup();
        edited
    }

    /// Every bundled flow file, for seeding.
    pub fn starter_flow_assets() -> Result<Vec<StarterFlowAsset>, FlowResourceError> {
        fn collect(
            dir: &'static Dir<'static>,
            assets: &mut Vec<StarterFlowAsset>,
        ) -> Result<(), FlowResourceError> {
            for entry in dir.entries() {
                match entry {
                    DirEntry::Dir(child) => collect(child, assets)?,
                    DirEntry::File(file) => assets.push(StarterFlowAsset {
                        name: file.path().to_string_lossy().replace('\\', "/"),
                        content: file
                            .contents_utf8()
                            .ok_or(FlowResourceError::InvalidUtf8)?
                            .to_string(),
                    }),
                }
            }
            Ok(())
        }
        let mut assets = Vec::new();
        collect(&STARTER_FLOWS, &mut assets)?;
        assets.sort_by(|left, right| left.name.cmp(&right.name));
        if assets.is_empty() {
            return Err(FlowResourceError::Missing);
        }
        Ok(assets)
    }
}

/// Bundled role files (`roles/<name>.md`): `(file name, Markdown text)`, sorted.
pub mod roles {
    /// The manifest recording the packaged version last installed for each role.
    pub const PACKAGED_ROLE_HASHES: &str = ".packaged-role-hashes.json";

    pub fn assets() -> Vec<(String, String)> {
        super::markdown_assets(&super::ROLES)
    }
}

/// Frontend asset boundary.
pub mod frontend {
    use std::path::{Path, PathBuf};

    use spark_common::settings::SparkSettings;

    use super::{
        packaged_file, validate_relative_path, ResourceFile, ResourcePathError, ResourceSource,
        FRONTEND_DIST,
    };

    const INDEX_FILE: &str = "index.html";
    const FAVICON_ASSET: &str = "assets/spark-app-icon.png";

    pub type AssetPathError = ResourcePathError;

    /// Resolves the filesystem UI root when one is configured or installed.
    pub fn resolve_ui_root(settings: &SparkSettings) -> Option<PathBuf> {
        filesystem_ui_root(settings).map(|(path, _)| path)
    }

    pub fn resolve_index_path(settings: &SparkSettings) -> Option<PathBuf> {
        let ui_root = resolve_ui_root(settings)?;
        let index_path = ui_root.join(INDEX_FILE);
        index_path.exists().then_some(index_path)
    }

    pub fn resolve_favicon_path(settings: &SparkSettings) -> Option<PathBuf> {
        resolve_asset_path(settings, FAVICON_ASSET).ok().flatten()
    }

    pub fn resolve_asset_path(
        settings: &SparkSettings,
        relative: &str,
    ) -> Result<Option<PathBuf>, AssetPathError> {
        validate_relative_path(relative)?;
        let Some((ui_root, _)) = filesystem_ui_root(settings) else {
            return Ok(None);
        };
        let candidate = ui_root.join(relative);
        if !candidate.exists() || !candidate.is_file() {
            return Ok(None);
        }

        let root = ui_root
            .canonicalize()
            .map_err(|_| AssetPathError::UnsafePath)?;
        let canonical_candidate = candidate
            .canonicalize()
            .map_err(|_| AssetPathError::UnsafePath)?;
        if canonical_candidate.starts_with(&root) {
            Ok(Some(candidate))
        } else {
            Err(ResourcePathError::UnsafePath)
        }
    }

    pub fn load_index(settings: &SparkSettings) -> Option<ResourceFile> {
        if let Some((ui_root, source)) = filesystem_ui_root(settings) {
            return ResourceFile::from_filesystem(INDEX_FILE, source, ui_root.join(INDEX_FILE));
        }
        packaged_file(&FRONTEND_DIST, INDEX_FILE)
    }

    pub fn load_favicon(settings: &SparkSettings) -> Option<ResourceFile> {
        load_asset(settings, FAVICON_ASSET).ok().flatten()
    }

    pub fn load_asset(
        settings: &SparkSettings,
        relative: &str,
    ) -> Result<Option<ResourceFile>, AssetPathError> {
        validate_relative_path(relative)?;
        if let Some((_, source)) = filesystem_ui_root(settings) {
            return resolve_asset_path(settings, relative).map(|path| {
                path.and_then(|path| ResourceFile::from_filesystem(relative, source, path))
            });
        }
        Ok(packaged_file(&FRONTEND_DIST, relative))
    }

    pub fn packaged_asset_names() -> Vec<String> {
        let mut names = Vec::new();
        super::collect_file_names(&FRONTEND_DIST, &mut names);
        names.sort();
        names
    }

    fn has_index(path: &Path) -> bool {
        path.join(INDEX_FILE).is_file()
    }

    fn filesystem_ui_root(settings: &SparkSettings) -> Option<(PathBuf, ResourceSource)> {
        if let Some(ui_dir) = settings.ui_dir.as_deref() {
            if has_index(ui_dir) {
                return Some((ui_dir.to_path_buf(), ResourceSource::ExplicitUiDir));
            }
        }

        let source_dist = settings.project_root.join("frontend/dist");
        if has_index(&source_dist) {
            return Some((source_dist, ResourceSource::SourceTree));
        }

        let installed_dist = settings.project_root.join("ui_dist");
        if has_index(&installed_dist) {
            return Some((installed_dist, ResourceSource::Packaged));
        }

        None
    }
}

/// Guide resource boundary.
pub mod guides {
    use super::{collect_file_names, packaged_file, ResourceFile, GUIDES};

    pub const FLOW_DEFINITION_AUTHORING_GUIDE_NAME: &str = "flow-definition-authoring.md";
    pub const SPARK_OPERATIONS_GUIDE_NAME: &str = "spark-operations.md";

    pub fn guide_names() -> Vec<String> {
        let mut names = Vec::new();
        collect_file_names(&GUIDES, &mut names);
        names.retain(|name| name.ends_with(".md"));
        names.sort();
        names
    }

    pub fn load_guide(name: &str) -> Option<ResourceFile> {
        packaged_file(&GUIDES, name)
    }

    pub fn flow_definition_authoring_guide() -> Option<ResourceFile> {
        load_guide(FLOW_DEFINITION_AUTHORING_GUIDE_NAME)
    }

    pub fn dot_authoring_guide() -> Option<ResourceFile> {
        flow_definition_authoring_guide()
    }

    pub fn spark_operations_guide() -> Option<ResourceFile> {
        load_guide(SPARK_OPERATIONS_GUIDE_NAME)
    }
}

/// Bundled mission playbooks: `(file name, Markdown text)`, sorted by name.
pub mod playbooks {
    pub fn assets() -> Vec<(String, String)> {
        super::markdown_assets(&super::PLAYBOOKS)
    }
}

fn markdown_assets(dir: &'static Dir<'static>) -> Vec<(String, String)> {
    let mut assets: Vec<(String, String)> = dir
        .files()
        .filter_map(|file| {
            Some((
                file.path().to_str()?.to_string(),
                file.contents_utf8()?.to_string(),
            ))
        })
        .collect();
    assets.sort();
    assets
}

/// Icon resource boundary.
pub mod icons {
    use super::{collect_file_names, packaged_file, ResourceFile, FRONTEND_DIST, ROOT_ASSETS};

    pub const FAVICON_NAME: &str = "assets/spark-app-icon.png";

    pub fn favicon() -> Option<ResourceFile> {
        packaged_file(&FRONTEND_DIST, FAVICON_NAME)
    }

    pub fn root_image_names() -> Vec<String> {
        let mut names = Vec::new();
        collect_file_names(&ROOT_ASSETS, &mut names);
        names.sort();
        names
    }

    pub fn load_root_image(name: &str) -> Option<ResourceFile> {
        packaged_file(&ROOT_ASSETS, name)
    }
}

/// Model and provider resource boundary.
pub mod models {
    use super::{ResourceFile, MODEL_CATALOG_JSON};

    pub const MODEL_CATALOG_NAME: &str = "unified_llm/data/models.json";

    pub fn model_catalog_json() -> &'static str {
        MODEL_CATALOG_JSON
    }

    pub fn model_catalog_resource() -> ResourceFile {
        ResourceFile::from_packaged_text(MODEL_CATALOG_NAME, MODEL_CATALOG_JSON)
    }
}

/// Service and container resource boundary.
pub mod resources {
    use super::ResourceFile;

    pub const PROVIDER_ENV_TEMPLATE_NAME: &str = "provider.env.example";
    const PROVIDER_ENV_TEMPLATE: &str = "\
# Optional provider configuration for SPARK_HOME/config/provider.env.
# Leave unset values blank or omit them. Runtime credentials are read from the process environment.
OPENAI_API_KEY=
OPENAI_BASE_URL=
OPENAI_ORG_ID=
OPENAI_PROJECT_ID=
ANTHROPIC_API_KEY=
ANTHROPIC_BASE_URL=
GEMINI_API_KEY=
GEMINI_BASE_URL=
GOOGLE_API_KEY=
OPENROUTER_API_KEY=
OPENROUTER_BASE_URL=
OPENROUTER_HTTP_REFERER=
OPENROUTER_TITLE=
LITELLM_BASE_URL=
LITELLM_API_KEY=
OPENAI_COMPATIBLE_BASE_URL=
OPENAI_COMPATIBLE_API_KEY=
";

    pub fn provider_env_template() -> ResourceFile {
        ResourceFile::from_packaged_text(PROVIDER_ENV_TEMPLATE_NAME, PROVIDER_ENV_TEMPLATE)
    }
}
