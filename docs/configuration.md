# Persisted settings

This page describes the unified settings operations. Validation and delivery details are recorded in [the implementation result](../changes/CR-2026-0117-unify-spark-configuration-and-settings/result.md).

Core runtime paths and workspace model defaults live in `$SPARK_HOME/config/spark.toml`. Bootstrap home selection remains external. Explicit runtime arguments override environment variables, which override persisted paths. The runtime settings response separates stored choices from the running process's effective paths and lists fields requiring restart.

```toml
[models]
provider = "codex"
reasoning_effort = "high"
```

A model group selects exactly one `provider` or `llm_profile`. Omitted model and reasoning fields mean provider defaults. A project group in `project.toml`, or a conversation group in `conversation.json`, replaces the entire inherited group. Clearing an override restores inheritance. Conversation messages resolve workspace → project → conversation immediately before starting.

Workflow launches read project/workspace defaults at start, below existing explicit launch, launch-context, flow and node values. Incompatible explicit provider/profile selectors do not borrow the inherited group's model. Runs capture LLM-profile definitions and the selected execution profile in their checkpoint context. The Rust LLM backend uses captured profile definitions; retries use captured execution-profile contents. Older runs without captures retain the existing recovery compatibility checks. Profile snapshots contain environment-variable references, never resolved credential values.

## API and CLI

Read the document before writing and send its opaque `revision` as `expected_revision`. A stale update returns HTTP 409. Reload with Discard to abandon a draft; failed saves preserve it.

```sh
spark settings get --base-url http://127.0.0.1:8000
spark settings get --base-url http://127.0.0.1:8000 --project /absolute/project
spark settings get --base-url http://127.0.0.1:8000 --project /absolute/project --conversation conversation-id
spark settings validate --base-url http://127.0.0.1:8000 --json settings-update.json
spark settings set --base-url http://127.0.0.1:8000 --json settings-update.json
```

Pass a filename to `--json`, or use `--json -` to read stdin. The corresponding endpoints are `GET /workspace/api/settings`, `POST /workspace/api/settings/validate`, and `PATCH /workspace/api/settings`. Reads accept `project_path` and `conversation_id` query parameters. Supported update sections are `runtime`, `connections`, `providers`, `agents`, `models`, `import_models`, `project_models`, `project_execution`, `conversation_models`, `client_preferences`, `import_client_preferences`, `llm_profiles`, and `execution_profiles`.

Example conversation override update:

```json
{
  "expected_revision": "revision-from-read",
  "section": "conversation_models",
  "value": {
    "project_path": "/absolute/project",
    "conversation_id": "conversation-id",
    "model_settings": { "provider": "claude-code" }
  }
}
```

Omitting `model_settings` leaves the override unchanged; explicit `null` restores inheritance. Project updates use the same payload without `conversation_id`, with section `project_models`.

Project reads also return an `execution` section with stored/effective profile, source, and document revision. Use `project_execution` with `{ "project_path": "/absolute/project", "execution_profile_id": "profile-id" }`; omission leaves it unchanged and null restores the workspace default. The existing `PATCH /workspace/api/projects/state` adapter requires `expected_revision` when changing or clearing `execution_profile_id`. Operational updates such as favorites do not require a settings revision, but share the document lock. Registration can choose an initial profile only for a new project; use the revision-checked update for an existing project.

Native Desktop saves publish through the same settings live stream as HTTP saves. The native editor refetches changes, preserves dirty drafts with their original revision, and reports external changes. Discard reloads the latest stored value.

The existing `PUT /workspace/api/conversations/<id>/settings` adapter also requires `expected_revision` for model and `chat_mode` changes, including the legacy `provider`, `llm_profile`, `model`, and `reasoning_effort` selectors. Missing revisions return HTTP 400; stale revisions return HTTP 409. Read the revision from the conversation response's `settings.models.revision`. Mode-only settings updates use the same revision checks; stale settings mutations are rejected under the conversation commit lock. Operational transcript/event mutations retain rebasing.

Flow launch policy remains in `flow-catalog.toml`. Read a flow's detail response to obtain the catalog `revision`, then include `expected_revision` when calling `PUT /workspace/api/flows/<name>/launch-policy`. The dedicated graph controls use explicit Save/Discard. Catalog writes use the shared atomic document lock and preserve unrelated fields. Policy updates publish `settings.changed`; dirty editors retain drafts and report external changes.

The settings sections and dedicated profile/flow-policy editors provide explicit Save/Discard. Remote-access changes retain Desktop's native confirmation.

## Migration boundaries

Server and Desktop bootstrap run the same locked core version transition before loading persisted runtime choices. Desktop can import its platform-owned remote-access source after server bootstrap; an existing core Desktop section wins. Original core and imported source bytes are backed up before replacement. The implemented transition also backs up conversation metadata before replacement. Migrated conversations inherit model settings while preserving mode and historical activity. Accessible browser model defaults import only when the workspace has no authoritative model group. Historical run snapshots, transcripts, tasks, caches and session state remain outside settings documents.

The earlier defaults file and accessible browser defaults/layouts are imported through versioned, backed-up migrations. Do not run old and new binaries concurrently against migrated workspace metadata.

Trigger definitions remain in their existing TOML files. Trigger reads include `revision`; PATCH bodies require `expected_revision`, and DELETE requires the `expected_revision` query parameter. Both use the shared per-document lock, reject stale writes with HTTP 409, and validate definition writes before atomic replacement. New definitions can only create an absent document. Runtime trigger state remains separate. CLI updates carry the revision in `--json`; deletion uses `spark trigger delete --id <id> --expected-revision <revision> --base-url <url>`. Trigger drafts retain their original revision across live changes and failed saves; Discard reloads current values.

Client preference reads use `GET /workspace/api/settings?client_id=<id>` or
`spark settings get --client <id>`. Save with the same settings endpoint/CLI and
`{"section":"client_preferences","expected_revision":"<revision>","value":{"client_id":"<id>","preferences":{"editor_mode":"structured","editor_sidebar_width":288}}}`.
The client document is `config/clients/<id>.toml`; editor mode and sidebar width
are currently supported. Null uses the built-in default. Width must be an integer
from 256 to 560 pixels. Reads return stored/effective values and revision. Updates
preserve other document sections, reject stale revisions, and notify settings
subscribers. Client IDs name preference documents; they are not authentication
credentials and do not change the server's existing access policy.

Desktop stores its generated identity in its app-owned configuration directory's
`client-id` file, and returns it through the native bootstrap boundary. It is
independent of the embedded HTTP port. Browsers persist a separate generated
identity in `spark.client_id` in localStorage. Corrupt identities produce an error
instead of silently switching preference ownership. Inaccessible browser origins
cannot be migrated automatically.

The Preferences editor saves explicitly and retains drafts on conflicts. Editor
mode changes and completed sidebar resize interactions persist without opening
Settings; pointer movement and viewport-size adjustments do not write preferences.
Selected records, raw YAML drafts and route restoration remain session state.
Additional client fields are `show_advanced_controls`, `expand_child_flows`,
`graph_settings_open`, `runs_scope`, and `triggers_scope`. The first three default
to false and supply initial presentation for newly opened flows/nodes. Run scope
defaults to `active`; trigger scope defaults to `all`. Scopes accept only `active`
or `all`. These controls support explicit Save/Discard in Preferences, and their
contextual toggles persist when the interaction completes. Existing per-flow and
per-node view state remains session state.

Other split/sort choices, graph layouts and browser preference migration remain
outside this implementation and required by CR-2026-0117.

### Home sidebar split and legacy imports

Client preferences also store `home_sidebar_primary_split_ratio` (a finite number from 0 to 1, or omitted for automatic sizing). The home sidebar saves a drag on pointer release and keyboard adjustments on completion. The Preferences editor supports explicit Save/Discard and validates the ratio. Minimum pane heights still apply. Conversation scrolling, selected records and drafts remain session state.

Bootstrap imports the flat legacy `config/ui-defaults.json` fields `llm_provider`, `llm_profile`, `llm_model`, and `reasoning_effort` into the core model group only when no authored core model group exists. Empty strings become omitted selections; a profile takes precedence over the legacy provider field. `defaults_migration_version = 1` prevents reimport. The source is backed up as `ui-defaults.json.v0.bak`, and an existing core document is backed up before replacement. Invalid source field types fail without replacing the core or echoing source values.

Conversation settings migration enumerates `workspace/projects/*/conversations/*` directly, including projects with missing or malformed registration metadata. It uses the existing conversation commit lock and changes only conversation settings metadata. It does not traverse symlinked directories. Historical turn files and nested historical metadata are preserved.

## Profile documents

`llm_profiles` and `execution_profiles` are workspace settings sections backed by
`llm-profiles.toml` and `execution-profiles.toml`. Their revisions are independent
of the core document. Reads expose stored profile definitions; LLM profiles also
report credential-reference status without resolving secrets into the response.
The Settings screen supports explicit Save/Discard, creation and deletion, and
keeps drafts after validation errors or conflicts. Profile pickers refresh after
settings notifications.

An LLM update's `value` is the complete list of profiles, each with `id`,
`provider`, `base_url`, `models`, and optional `label`, `api_key_env`, and
`default_model`. Currently the dedicated profile format supports
`openai_compatible`. Endpoints must use HTTP(S) without embedded credentials,
queries or fragments; credentials use environment-variable references.

An execution update's `value` contains `profiles` and nullable
`default_execution_profile_id`. Each profile contains `id`, `label`, `mode`,
`enabled`, `capabilities`, optional `image`, and a `metadata` object. Container
profiles require an image. Mounts remain in `metadata["container.mounts"]` as an
array of `host:container[:options]` strings. Metadata must be representable in
TOML, so JSON null is invalid. Defaults must identify an enabled profile.

Both updates use the standard `expected_revision`/`section`/`value` envelope.
Existing resource URLs also accept PATCH with `{ "expected_revision": "…",
"value": … }`: `/attractor/api/llm-profiles` and
`/attractor/api/execution-placement-settings`. Their GET responses preserve the
existing fields and add the document revision. CLI `settings validate` and
`settings set` accept the same typed sections.

Save validates before replacing the document and preserves unrelated top-level
sections. Removing an existing profile checks core/project/conversation model
overrides, project execution defaults, flow definitions and trigger actions.
Rejections identify references to update first. Historical turn/run snapshots
are not deletion references. Explicit selections on historical runs retain their
existing precedence even if the current workspace default is invalid.

## Server and client connections

The core `connections` section supports `server_host`, `server_port`, and
`client_api_base_url`. Omitted fields use the existing defaults. Server binding
resolves explicit `--host`/`--port`, then `SPARK_HOST`/`SPARK_PORT`, then persisted
choices, then `127.0.0.1:8000`. Port zero requests an available port. Changes to
binding require restart; settings responses retain the running effective binding
beside the newly saved values.

```toml
[connections]
server_host = "127.0.0.1"
server_port = 8000
client_api_base_url = "https://spark.example"
```

Installed CLI commands resolve `--base-url`, then `SPARK_API_BASE_URL`, then this
persisted target, then the existing default. Source checkouts still require an
explicit flag or environment API target, even when a persisted target exists.
The target must be HTTP(S) without embedded credentials, query parameters or
fragments. Errors do not repeat rejected URL contents.

The connection editor uses the same section revision and explicit Save/Discard
as runtime settings, with validation, pending protection and live conflict
retention. Desktop continues to choose its binding through native remote-access
confirmation, uses an automatically assigned port, and reports its native
choices as effective overrides. Saving server connection fields does not enable
Desktop remote access or redirect the embedded Desktop client.

Profile deletion and cooperating reference writers now acquire a workspace
`config/profile-references.lock` before their individual document locks. The
lock spans reference validation and persistence, preventing deletion from
passing a reference writer that has validated but is waiting to write. This
covers model/project/conversation edits and starts, flow saves and trigger
persistence. Hand editing files outside these cooperating writers still requires
valid references, just as other external configuration edits require valid data.


## Provider connections and agents

The `providers` section supports OpenAI, Anthropic, Gemini, OpenRouter, LiteLLM,
and OpenAI-compatible connections. Fields are `base_url`, `api_key_env`, OpenAI
`organization`/`project`, and OpenRouter `http_referer`/`title`. Standard provider
environment variables override saved fields. Credential responses expose the
chosen variable name and configured/missing status; values are resolved only
into transient adapter construction data.

```toml
[providers.openai]
base_url = "https://api.openai.com/v1"
api_key_env = "TEAM_OPENAI_KEY"

[agents]
max_turns = 20
max_tool_rounds_per_input = 10
default_command_timeout_ms = 10000
max_command_timeout_ms = 600000
enable_loop_detection = true
loop_detection_window = 10
max_subagent_depth = 1
environment_inheritance = "inherit_core_only"

[agents.tool_output_limits]
shell = 20000

[agents.line_limits]
shell = 200

[agents.native]
claude_permission_mode = "bypassPermissions"
codex_jsonrpc_trace = false
agent_trace = false
```

Optional native fields are `codex_binary`, `codex_runtime_root`, `codex_seed_dir`,
`claude_binary`, and `claude_config_dir`. Existing `SPARK_CODEX_APP_SERVER_BIN`,
`ATTRACTOR_CODEX_RUNTIME_ROOT`, `ATTRACTOR_CODEX_SEED_DIR`, `SPARK_CLAUDE_CODE_BIN`,
`SPARK_CLAUDE_CODE_CONFIG_DIR`, and `SPARK_CLAUDE_CODE_PERMISSION_MODE` overrides
retain precedence. Diagnostics retain `SPARK_DEBUG_CODEX_JSONRPC` and
`SPARK_DEBUG_AGENT_TRACE` precedence. Agent runtime/seed/config-home changes are
restart settings; reads distinguish saved values from the running locations.
Binary and permission choices apply to new work. Native model discovery uses
these choices and keys its short-lived operational cache by configuration.

The existing Codex standard service tier, approval policy, and sandbox policy
remain fixed and are reported by the editor. OAuth files remain owned by the
native agents. Tool environment inheritance retains the default core-variable
allowlist; explicit choices may select all variables or none. Per-tool limits
remain numeric maps consumed by the existing truncation and execution code.

New conversations and workflows capture resolved provider and agent settings,
startup runtime paths, all LLM-profile definitions, and the applicable model and
execution-profile selections before execution. Native launchers and built-in
sessions consume those captures. Continuations retain the original capture;
subsequent independent messages/runs read and validate current files. Credential
references are captured; credential values and OAuth material are not persisted
in snapshots.

## Reusable presentation and legacy layouts

Client documents also own run sort order, activity/inspector defaults, timeline
category/severity filters, graph height, and per-flow user node positions and edge port choices.
Explicit Settings edits use Save/Discard; contextual filter changes and completed
layout interactions save through the same client revision boundary. New run
sessions start with the client's presentation defaults. Selected runs and
existing run-session caches remain operational state.

Accessible `spark.saved_flow_layout.v1:*` browser records are backed up verbatim
as `.v0.bak` before a revision-checked, client-scoped import. The client document
records `browser_migration_version = 1`; existing authoritative positions win,
and rerunning the import is harmless. Existing client documents are backed up as
`.browser-import-v0.bak` before migration. Failed imports retain the original
browser record and backup. Only after success is the superseded browser record
removed. New layout writes never recreate it.

Only user node positions and edge side/slot choices enter preferences. Topology stamps, computed edge
routes, and viewport restoration stay in `spark.flow_layout_cache.v1:*` on the
current origin and can be recomputed. Desktop restores its durable positions
using its stable native identity even when the new HTTP port has no browser
cache. Other clients have their own documents. Browser origins that cannot be
accessed cannot be imported automatically.
