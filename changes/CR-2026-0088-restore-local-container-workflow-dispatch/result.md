# Result

- API pipeline execution now builds a `ContainerizedNodeExecutor` from the selected execution profile for both waited and detached fresh runs, preserving one executor (and therefore one container lifecycle) per pipeline.
- Continue, retry, and startup recovery resolve the run's recorded profile ID explicitly, independent of the current configured default, and require its configured mode and image to match the run record before execution. Invalid, missing, disabled, or changed recorded profiles fail the run without native fallback.
- Native profiles continue to execute in process through the container-aware executor without invoking Docker.
- Existing API lifecycle and recovery fixtures now identify their recorded native placement explicitly.
- API contract tests use a fake Docker boundary to cover waited and detached starts, native bypass, continue, retry, startup recovery, placement metadata, and reconstruction failures for missing, disabled, invalid, mode-changed, and image-changed profiles.
- Container worker outcomes now return only the worker's declared context updates; the parent request context remains owned by the pipeline runtime instead of being incorrectly re-declared as node output.

Validation: `cargo test -p attractor-execution -p attractor-api --all-features`
