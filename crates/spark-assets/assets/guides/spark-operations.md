# Spark Operations Guide

This guide is the packaged control-surface reference for agents operating Spark through its CLI and HTTP API.

Use it for launch, inspection, human-gate, trigger, mission, and standing-role tasks. OpenAPI remains the exhaustive schema source:

- Attractor: `/attractor/docs`, `/attractor/openapi.json`
- Workspace: `/workspace/docs`, `/workspace/openapi.json`

Control-surface contract:

- Inside the assistant runtime, the canonical Spark agent control surface is the `spark` CLI. `spark <domain> --help` lists every command and its flags; this guide says when to use them.
- In a human source-checkout shell, run the same CLI commands through `cargo run -p spark-cli --bin spark --` before installing the binaries.
- Commands print the server's JSON payload; `--text` opts into a readable rendering where offered. A failure prints one JSON object on stderr with the error, the exit code (2 for usage, 3 for not found) and, for server errors, the target it called.
- The examples below use the assistant-runtime form unless a source-checkout example is called out explicitly.

## Environment And Bootstrap

Installed or stable Spark instance:

```bash
spark-server init
spark-server serve --host 127.0.0.1 --port 8000
```

Source checkout workflow:

```bash
SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- init
SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- serve --port 8010
SPARK_API_BASE_URL=http://127.0.0.1:8010 cargo run -p spark-cli --bin spark -- flow list
```

Operational rules:

- From a source checkout, always set `SPARK_HOME` before `spark-server init` or `spark-server serve`.
- From a source checkout, always set `SPARK_API_BASE_URL` before `spark` commands that talk to a running server.
- `spark flow validate --dir <flow directory>` is local and does not require a running server.
- `SPARK_UI_DIR` overrides the served built UI directory when needed.

## Discover And Validate Flows

Flows resolve project first, then bundled: a project's definitions
(`<project>/.spark/definitions/flows`) replace any bundled flow of the same name
in `$SPARK_HOME/bundled/flows`. Pass `--project <path>` (or `project_path=` on
the HTTP routes) to include a project's flows; without it you see the bundled
library only. Every flow can be launched.

Registering a project writes nothing under `.spark/definitions`: a flow,
role or playbook the project has no file for resolves from the bundled
library. Copy a bundled file into the project's definitions to change it
there; the editor does this when you save a bundled flow. The bundled library
also holds `$SPARK_HOME/bundled/guides`, read-only references such as the
flow authoring guide; guides take no part in project lookup.
To choose models for one flow or node in a project, copy the flow into the
project's definitions and set them there; there is no project settings file.
Spark writes nothing else into `.spark`: run history, memory and working data
are private to the installation (see Storage, Removal And Reset). Spark runs
no Git in `.spark`; versioning the definitions is the operator's choice.

Read one flow before launching it, and validate flows you edit:

```bash
spark flow list --project /absolute/path/to/project --text
spark flow get examples/simple-linear --inputs
spark flow get examples/simple-linear --text
spark flow validate examples/simple-linear --text
spark flow validate --dir /absolute/path/to/flows/my-flow --text
```

`flow list` gives each flow's name, title and description. `flow get` prints
a flow's files (`--text` prints each under a `==> <file> <==` header), and
`--inputs` prints only the inputs its `flow.md` declares. `flow validate --dir`
checks a directory you are editing, with no server; `--project` names the
project whose definitions resolve its roles, which otherwise is the project
whose `.spark/definitions` holds the directory.

## Launch Runs

An agent launches from its conversation. The run reports back to that
conversation and counts against its mission's or role's budget:

```bash
spark run start --conversation amber-otter --flow examples/simple-linear --summary "Inspect the repo and summarize next steps."
```

A detached launch names a project instead, shows no launch card in any chat
and reports nowhere:

```bash
spark run start --project /absolute/path/to/project --flow examples/simple-linear --summary "Inspect the repo and summarize next steps."
```

Notes:

- Give `--conversation` or `--project`, not both: a conversation's runs use its project.
- Before launching, read the flow's inputs with `spark flow get <name> --inputs` and supply the required ones with `--context`, keyed by input key (for example `context.request.source_ref`). `--summary` describes the request; it does not populate workflow inputs.
- Every JSON flag takes inline JSON, a file path, or `-` for stdin; `--goal -` reads the goal from stdin.
- Leave the model and execution flags out unless you need a launch-time override; the flow defaults and project settings then apply. They apply run-wide but do not change nodes whose own or role execution sets those fields. `--node-overrides` changes specific `agent_task` nodes, keyed by node id from `spark flow get <name>`; every member of a shared `runtime.session` must end up on the same provider, profile and model. `spark run start --help` lists the flags.
- A launch over its mission's or role's budget is refused with the limit named.

## Recover Runs

Use the workspace recovery commands instead of calling Attractor recovery endpoints directly. They preserve conversation and project scoping and return stable JSON payloads for agents.

Retry executes a fresh attempt under the same run ID with a restored automatic-retry
allowance. Retrying a parent recovers its linked failed children and nested
descendants in place; successful children are consumed without rerunning. Existing
working directories, snapshots, lineage, and prior attempt artifacts are preserved.
`started` reports execution acceptance, not success. Continue creates a new run.

```bash
spark run retry <run_id> --conversation amber-otter
spark run continue <run_id> --from run_milestone --conversation amber-otter
spark run continue <run_id> --from run_milestone --flow examples/simple-linear --conversation amber-otter
```

Notes:

- Only a failed run can be retried. To resume a run that completed or was canceled, continue it from a node.
- Continue runs the source run's flow snapshot; `--flow` runs that flow's current definition instead.
- When `--conversation` is supplied, Spark resolves the handle, validates that any explicit `--project` matches the conversation project, creates a conversation-visible `run_recovery` artifact, and publishes the updated conversation snapshot.
- The recovery artifact records the operation, source run, result run, status, project path, conversation context, continuation node and flow source, optional model/provider/profile/reasoning overrides, and any recovery error.
- Retry records the same source and result run id. Continue records the original run as `source_run_id` and the newly created run as `result_run_id`.
- For detached continue, omit `--project` to let Attractor inherit the source run working directory. Include `--project /absolute/path/to/project` only when intentionally overriding that working directory.
- For conversation-scoped continue, omit `--project` unless validating an explicit project path against the conversation.

## Inspect Runs

```bash
spark run list --project /absolute/path/to/project
spark run get <run_id>
spark run events <run_id> --after 1200 --until-done
spark run cancel <run_id>
```

- `spark run get <run_id>` is the authoritative run detail.
- `run events` prints one JSON object per event line (`--text` prints readable lines). It stops after 30 seconds unless `--until-done` keeps it running until the run ends; `--after` skips events up to a sequence you already have.
- `GET /workspace/api/live/events` is the browser/operator live delivery stream. It multiplexes observed resources: conversations use `conversation_id` with required `conversation_project_path`, runs overview uses optional active-project `runs_project_path` and omits it for all-project runs, triggers use `triggers_project_path`, and selected-run tailing uses `run_id` only after selected-run detail plus durable journal hydration establish a cursor. `run_sequence` and `conversation_revision` are reconnect/catch-up cursors, not steady-state stream identity.
- `GET /attractor/pipelines/{id}/events` is deprecated for Spark UI/operator clients. Prefer `spark run events <run_id>`.

The durable journal and other selected-run surfaces have no CLI command:

```bash
curl 'http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/journal?limit=50'
curl 'http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/journal?limit=50&before_sequence=1200'
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/checkpoint
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/context
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/artifacts
curl http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/graph-preview
curl -OJ 'http://127.0.0.1:8000/attractor/pipelines/<pipeline_id>/artifacts/path/to/file.txt?download=true'
```

The journal reads newest first; `before_sequence` loads older history.

## Answer Pending Human Gates

```bash
spark run questions <run_id>
spark run answer <run_id> --question <question_id> --option approve
spark run answer <run_id> --question <question_id> --option request_fixes --note "Tighten the error handling in the retry path."
```

Use an option's `value` (the gate's decision ID) from the question payload;
labels and keys are display only and are not accepted. The option picks the
route; it is not a feedback channel — anything other than a declared
decision ID is rejected and the gate keeps waiting. Send feedback with the
selection as `--note`.

The note reaches downstream nodes as the `human.gate.note` context value;
nodes that should see it declare it in `contracts.reads_context`. Each
answered gate overwrites `human.gate.note` (empty when no note was given).

## Manage Triggers

A trigger belongs to a project: its definition is `triggers/<id>.toml` in the project's definitions (`<project>/.spark/definitions`), and a trigger is identified by its project plus its id.

```bash
spark trigger list --project /absolute/path/to/project --text
spark trigger get <trigger_id> --project /absolute/path/to/project --text
spark trigger create --project /absolute/path/to/project --json /absolute/path/to/trigger.json --message "Nightly audit"
spark trigger update <trigger_id> --project /absolute/path/to/project --json /absolute/path/to/trigger-patch.json
spark trigger delete <trigger_id> --project /absolute/path/to/project --expected-revision <revision>
```

Update and delete name the trigger's current revision, from `spark trigger get <trigger_id> --project <path>`: update as `expected_revision` in its JSON, delete as `--expected-revision`. Without `--project`, `trigger list` covers every registered project.

Trigger operation notes:

- Every create, update and delete writes the trigger's file in the project's definitions; the optional message is descriptive and Spark makes no Git commit. A definition edited by hand is picked up on the next tick.
- A definition makes a trigger available; it fires only once this installation activates it. Creating or updating with `enabled: true` validates the action's target and execution profile, then activates; a failed activation leaves the trigger inactive with `state.activation_error`. Activation is private state and writes no definition. A definition added by hand or arriving with a clone starts inactive.
- Changing a trigger's source or action, including its execution profile, deactivates it until it is activated again; descriptive edits do not. Deleting a definition, through Spark or on disk, removes its activation and secrets, and recreating it restores neither. An unreadable or invalid definition is reported, never treated as deleted, and never fires.
- A `mission` action's `mission_id` and a poll source's headers are private bindings: Spark keeps them with the activation, not in the definition.
- Protected triggers reject edits and deletes through the same CLI and HTTP surfaces.
- Webhook triggers use the shared Workspace webhook ingress route and validate their secret before dispatch. Their key and secret hash live only in Spark's runtime state, never in the project's definitions; a webhook definition copied from another project has no key until its secret is regenerated.
- An action's `mode` is `static` (launch `flow_name` in the trigger's project, the default), `mission` (deliver to `mission_id` in the trigger's project), or `role` (deliver to the standing role `role` in the trigger's project). `flow_event` triggers fire only for runs in their own project.
- Production trigger runtime state, dispatch provenance, and live trigger events are emitted by the Rust trigger runtime. The browser/operator stream observes trigger changes through `/workspace/api/live/events` with `triggers_project_path`.
- Trigger-launched repository mutation policy is compatibility behavior, not a new safety policy defined by this guide.

## Packaged, Service, And Container Operation

Foreground packaged runtime:

```bash
spark-server init
spark-server serve --host 127.0.0.1 --port 8000
```

Linux user service (binds `127.0.0.1` by default; Spark does not authenticate requests, so `--host 0.0.0.0` exposes the unauthenticated server to the network):

```bash
spark-server service install --host 127.0.0.1 --port 8000
spark-server service status
spark-server service remove
```

Operational rules:

- Every Spark process, desktop, server and CLI alike, uses one data directory: `$SPARK_HOME` when it is set, else `~/.spark`. `spark-server --data-dir` overrides it for that server. Agents receive the server's data directory as `SPARK_HOME`, and their homes are `$SPARK_HOME/runtime/<agent>`.
- The service loads provider secrets from `$SPARK_HOME/config/provider.env` when that file exists.
- Source checkouts should use `SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- init` and `SPARK_HOME=~/.spark-dev cargo run -p spark-server --bin spark-server -- serve --port 8010` instead of mutating a stable packaged runtime.
## Storage, Removal And Reset

Spark stores data by owner. Below, `P` is a project's `.spark/` directory, `H`
is the data directory (`$SPARK_HOME`, default `~/.spark`) and `G` is
`H/projects/<project-id>/`. The project id derives from the project's
normalized path, so a checkout at another path is another project with its own
private history.

```text
P/definitions/{flows,roles,playbooks,triggers}/   authored; shareable, normally committed
H/config/spark.toml                              installation settings and profiles
H/config/provider.env                            provider secrets, operator-managed
H/state.sqlite3                                  private records (SQLite files beside it)
H/projects/<project-id>/
  conversations/<conversation-id>/               attachments, full tool outputs, traces
  memory/<role>/                                 standing-role memory
  runs/<run-id>/                                 pipeline run records and content
  work/                                          durable private working files
H/runtime/{codex,claude}/                        agent homes
H/bundled/                                       rewritten on every start
```

- `config/spark.toml` holds the core settings and the profiles: `[llm_profiles.<name>]`, `[execution_profiles.<name>]` and `[execution_defaults]` (`execution_profile_id`). A settings edit replaces its own section and keeps the others. Credentials and host or container details belong here, never in a project.
- `state.sqlite3` holds the project registry, with each project's private model and execution-profile defaults, plus conversations, standing roles, missions, trigger activations and secrets, and the workflow milestones. Runs have no index there: run files are the record, and run listings read their directories.
- Runs live in `G/runs/<run-id>/` unless `[runtime] runs_dir` in `spark.toml` names an external root, which holds `<runs_dir>/<project-id>/<run-id>/`.
- Every run of a project, root and child alike, gets `G/work` as `internal.project_work_dir` and as `SPARK_PROJECT_WORK_DIR`; a local container mounts it at the same path. Flows keep checkouts and ledgers there, each in its own subdirectory. A Git worktree there still depends on its source repository.
- Temporary `.spark-storage-*` and `.spark-deleting-*` entries appear beside definitions while Spark replaces or deletes them; definition listings skip them.

Removing a project (the Projects page, or `DELETE /workspace/api/projects?project_path=...`) interrupts its conversation turns, then deletes its registration and private defaults, its conversations and `G/conversations/`, its standing roles and missions, and its trigger activations and secrets. It keeps the project's files, `G/memory`, `G/work`, every run in either runs root, the milestones and anything else under `G`. Pipelines already running continue and keep writing their run files. Registering the same path again reuses the retained memory, work and runs; it restores no conversation, role, mission or activation. A standing role restarts only when started, in a new conversation with its retained memory; its removed conversation stays deleted.

Spark migrates, imports and purges nothing from older layouts. Stop Spark before deleting any of these, and save authored files and uncommitted work first:

| Deleted | Consequence |
| --- | --- |
| A project's `P` | Its definitions are gone; bundled flows still supply the names they have. Its private history, memory and work survive. Its triggers stop firing and lose their activations. |
| `state.sqlite3` | Registrations, private defaults, conversations, roles, missions, activations and milestones are gone. Surviving definitions never re-arm triggers; run and work files stay but are not imported. |
| `G/work` or `G/memory` | Uncommitted work, in-progress ledgers or learned knowledge are lost, and resuming a run that used them may fail. They are data, not cache. |
| All of `H` | Also loses the installation settings, default runs, work, memory and agent homes. An external runs root survives, but its `runs_dir` must be configured again. Startup creates fresh state and bundled assets. |

The Home project is the user's home folder, registered at startup. With the default data directory its `P` is `H` itself: `H/definitions/{flows,roles,playbooks}` are Home's project definitions, not an installation-wide fallback for other projects, so deleting `~/.spark` loses both. Home has no project settings file; its private defaults live in `state.sqlite3` like any project's.

## Project missions

A mission is a project objective plus a long-lived agent conversation that owns the mission's runs. A mission that has not started is a draft: record useful detail in the optional plain-text description (the objective); no template or mandatory sections are needed. Do not infer or import missions from change requests automatically.

```sh
spark mission list --project /absolute/project
spark mission get mission-ID --project /absolute/project
spark mission create --project /absolute/project --json mission.json
spark mission create --conversation <handle> --json mission.json
spark mission update mission-ID --project /absolute/project --json update.json
spark mission start mission-ID --project /absolute/project
spark mission close mission-ID --project /absolute/project --status done --reason "Shipped and verified"
```

Create requires only a title:

```json
{"fields":{"title":"Deliver search","description":"Search should return matching documents."},"actor":"assistant"}
```

Updates require the revision from get/list, with only changed fields. Editable fields are `title`, `description`, `archived`, `budget` (`{"concurrent_runs":4,"total_runs":25}` by default), and `playbook` (see below; fixed once the mission starts):

```json
{"revision":1,"fields":{"description":"Decide which document types are in scope."},"note":"Recorded the scope question","actor":"assistant"}
```

Create with `--conversation <handle>` instead of `--project` to make a mission from a chat: it takes the chat's project and records the chat as `source_conversation_id`. A `--project` that names a different project is rejected.

Archive with `fields.archived: true` and restore with `false`. On a revision conflict, reread and reconcile; never blindly retry stale edits. Listing returns `missions`, ordered by creation time then ID; `GET /workspace/api/missions` without `project_path` lists every project's missions.

Creating a mission also creates its conversation (`conversation_id`, the mission ID) and handle, without starting a turn, so a draft can be messaged with `spark send` (see Conversation inboxes). A draft holds its inbox: messages and mission-targeted trigger deliveries wait until start, which sends the objective as the first turn together with them. From then on, the mission's inbox (see Conversation inboxes) delivers its events to that conversation as new turns, including while the mission waits; a closed mission still records events but never wakes. The mission agent launches flows with `spark run start --conversation <mission handle> --flow <name> --summary <text>`; each launch joins the mission's `runs` roster, so its events come back to the mission. A launch over budget is refused with the limit named. The agent closes the mission with `spark mission close <mission id> --project <path> --status done --reason <text>`; `POST /workspace/api/missions/{id}/cancel?project_path=...` cancels in-flight runs and closes it.

Questions from all descendants return to the mission with their owning run ID. List open questions with `spark run questions <run id>`. Answer using `spark run answer <owning run id> --question <question id> --option <value>`, or `--text <text>` for a free-text answer. The mission agent answers every question itself when the objective, playbook, run results and logs, and the repository give it the information, stating its reasoning; only when that information is missing does it ask the user for exactly what is missing, end the turn, and relay their reply without guessing. A mission is Running during its question-handling turn and Needs you if it ends with an unanswered question. Answers from the Runs view also clear the roster wait.

### Playbooks

A playbook is named, reusable guidance for a kind of mission: `playbooks/<name>.md`, Markdown with YAML frontmatter (`title`, `description`) and free-form instructions in the body. Playbooks resolve like flows and roles: the project's definitions (`<project>/.spark/definitions/playbooks/`) first, then the bundled library (`$SPARK_HOME/bundled/playbooks/`, rewritten on every start, with playbooks such as `bug-report`). A project copy replaces the bundled playbook of the same name; to change a playbook for a project, edit or add it in the definitions. A mission resolves its playbook through its own project. The mission agent follows a playbook as the broad shape of the work, decides each step from run results and logs, and departs from it when the evidence calls for it.

```sh
spark playbook list --project /absolute/project
spark playbook get bug-report --project /absolute/project
```

To run a playbook against an issue, list the playbooks, create a mission with the playbook and the issue as its objective, and start it:

```json
{"fields":{"title":"Fix dropped parser input","description":"<the issue>","playbook":"bug-report"},"actor":"assistant"}
```

Create rejects an unknown playbook. On start, the mission snapshots the playbook into its record (`playbook`: name, title, description, and text), so later edits to the file do not change it, and the agent's pinned instructions include the playbook text after the objective.

`status` is derived on read: `draft` (not started), `running` (a turn or run is in flight or events are pending, and no run waits on a human gate), `needs_you` (a run waits on a human gate, or nothing is in flight or pending), or `closed`.

## Standing roles

A standing role is a role file (`roles/<name>.md`, resolved from the project's definitions, then bundled) bound to one long-lived conversation per project, `role-<project id>-<name>`. No role runs until it is started: a project with no started roles has no role conversations. Each role keeps its memory in `$SPARK_HOME/projects/<project id>/memory/<name>/` (`index.md`, `log.md`), outside the project; removing the project keeps it.

```bash
spark role list --project /absolute/project
spark role start <name> --project /absolute/project
spark role stop <name> --project /absolute/project
```

Only a role whose file declares `standing: true` can be started. Starting a role records it, creates its conversation if it has none and gives it a first turn to begin its work; starting a stopped role resumes delivery. The Roles tab in the app starts a standing role and opens its conversation the same way. The role file also sets the role's launch limit:

```yaml
standing: true
budget:
  concurrent_runs: 4
```

`budget` and its field are optional and default to the value shown; `budget` without `standing: true` is invalid. The budget is read from the role file as the project resolves it, so a project raises or lowers it by editing the role's file in its definitions. Spark's role record keeps only whether the role is stopped; its inbox and run roster belong to its conversation. `spark role list --project <path>` shows each role's `conversation_handle` once its conversation exists; message a role with `spark send --to <that handle> --from <your handle> --message <text>`.

The same operations are `GET /workspace/api/roles?project_path=...` and `POST /workspace/api/roles/{name}/start|stop?project_path=...`.

A role wakes on a new turn when the user writes in its conversation, when a run launched from it reports back, when a `role`-mode trigger fires, when a mission created from its conversation closes, and when another conversation sends it a message. Launches count against the role file's `budget`; a refused launch names the limit it hit. A stopped role still records events and messages but delivers none until it starts again, when it takes everything held; its triggers stay enabled, and a `role` trigger skips runs the role launched itself and runs of other projects. A role file's `execution` settings apply to its conversation below the conversation's own model settings and above the project's.

## Conversation inboxes

Every conversation, whether a chat, mission or standing role, has an inbox in the conversation store. Run reports, trigger deliveries and messages from other conversations arrive there and reach the conversation as turns: one marked batch per turn, oldest first, at most one turn in flight. An idle conversation starts a turn for a batch; a busy one takes it into the running turn when the provider accepts live input, and otherwise keeps it for the next turn. A batch is about earlier work or relays a message: it is not a new request from the user, and an agent assesses it against the user's latest instructions. A mission or standing role delivers through its own gate (draft, closed, stopped); a chat always does.

A run launched from a conversation (with `spark run start --conversation <handle> --flow <name> --summary <text>`, an approved run request, or an approved proposed plan) belongs to that conversation for good: Spark records `context.spark_owner` as `{"conversation_id": ...}` at launch and replaces any owner a caller supplies. Child runs report to their root's owner. Completion, failure, cancellation, questions and recovery-decision waits each arrive once; ordinary progress does not wake anyone. A report wakes its own conversation even after the user moved on to another topic or thread; it does not move to the thread on screen. A run launched with no conversation is unowned and reports nowhere. If the owner conversation is deleted, its reports are dropped.

Message any conversation by handle:

```sh
spark send --to <recipient handle> --from <your handle> --message "The review flow is fixed"
```

The same request is `POST /workspace/api/conversations/by-handle/{recipient handle}/messages` with `{"conversation_handle": "<your handle>", "message": "<text>"}`; it answers 202 once the message is stored, not once the recipient replies. Each call is a new message, so a repeated send arrives twice. The recipient sees your handle, your project and, for a mission or standing role, its label; it replies the same way, to your handle. Handles work across projects, and a conversation can message itself. Find a standing role's handle with `spark role list --project <path>`, a mission's with `spark mission get <mission id> --project <path>`, and your own in your instructions.

## Delegate To A Linked Environment

A delegation hands a whole task, a flow run or a new conversation, to a project
in a linked environment (see Link Environments). Its questions and its result
come back to the run or conversation that started it.

```bash
spark environment link list
spark environment link projects <environment_id>
spark flow list --environment <environment_id> --project /absolute/path/there
spark flow get examples/simple-linear --environment <environment_id> --project /absolute/path/there --inputs
spark delegation start --environment <environment_id> --target-project /absolute/path/there --kind flow --request-key nightly-checks --conversation <your handle> --flow examples/simple-linear --summary "Run the checks there."
spark delegation list --open
spark delegation get <delegation_id>
spark delegation wait <delegation_id> --timeout 600
spark delegation answer <delegation_id> --question <question_id> --option approve
spark delegation cancel <delegation_id>
```

- `--request-key` is your name for the task: starting again with the same key returns the same delegation, so a retried step never starts a second task.
- `--kind conversation` starts a conversation there with `--message` instead of running a flow.
- `spark delegation list` lists outgoing delegations (filter with `--environment`, `--phase`, `--open`, or `--paused`).
- `spark delegation get <delegation_id>` shows the open questions. Answer a conversation's several questions at once with `--answers`, keyed by question id. A question with a `form` takes one typed reply with `--form`, such as `{"action":"accept","content":{"count":0}}`, `{"action":"decline"}` or `{"action":"cancel"}`.
- `wait` prints the record once the delegation seals, or with `timed_out` when `--timeout` passes first; stopping it changes nothing on either side.

## Link Environments

Two environments link with a pairing code. On the environment to be reached,
inspect its id or create a code (which also prints its `environment_id`); on
the environment that will delegate, add the link with that code, the other
environment's address and its id:

```bash
spark environment show
spark environment pair-code --label "Build server"
spark environment link add --origin https://build.example:8000 --expected-id <environment_id> --code <code>
spark environment link list
spark environment link projects <environment_id>
spark environment link remove <environment_id>
spark environment tokens list
spark environment tokens revoke <token_id>
```

`tokens list` shows the tokens other environments reach this one with;
`tokens revoke` cuts one off.
