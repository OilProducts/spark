from tests.contracts.frontend.frontend_behavior_runner import assert_frontend_behavior_contract_passed
from tests.contracts.frontend._support.dot_probe import run_pipeline_start_payload_probe
from tests.contracts.frontend._support.static_contracts import missing_required_ui_endpoints


def test_required_ui_api_endpoints_have_runtime_coverage_item_12_1_01() -> None:
    assert missing_required_ui_endpoints() == []


def test_typed_client_adapters_and_runtime_schema_validation_item_12_1_02() -> None:
    assert_frontend_behavior_contract_passed("12.1.02")


def test_endpoint_integration_happy_path_and_common_error_cases_item_12_1_03() -> None:
    assert_frontend_behavior_contract_passed("12.1.03")


def test_degraded_state_ux_when_endpoint_unavailable_or_incompatible_item_12_2_01() -> None:
    assert_frontend_behavior_contract_passed("12.2.01")


def test_non_dependent_ui_surfaces_remain_functional_under_partial_api_failure_item_12_2_02() -> None:
    assert_frontend_behavior_contract_passed("12.2.02")


def test_save_paths_remain_non_destructive_during_api_contract_drift_item_12_2_03() -> None:
    assert_frontend_behavior_contract_passed("12.2.03")


def test_project_selection_and_active_project_identity_persist_in_ui_client_state_item_12_3_01() -> None:
    assert_frontend_behavior_contract_passed("12.3.01")


def test_execution_payload_project_identity_resolves_to_working_directory_context_item_12_3_02() -> None:
    assert_frontend_behavior_contract_passed("12.3.02")


def test_project_conversation_retrieval_is_keyed_by_project_identity_item_12_3_03() -> None:
    assert_frontend_behavior_contract_passed("12.3.03")


def test_execution_profile_response_metadata_contract_item_12_3_04() -> None:
    assert_frontend_behavior_contract_passed("12.3.04")


def test_execution_profile_launch_payload_ignores_dot_authored_placement_item_m4_i5() -> None:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.PIPELINE_START_PAYLOAD_JS_PATH).href)
const flowContent = `
digraph G {
  graph [
    execution_profile_id="dot-remote",
    execution_mode="remote_worker",
    execution_container_image="dot-image",
    worker="dot-worker"
  ]
  start [shape=Mdiamond]
  done [shape=Msquare]
  start -> done
}
`
const payload = mod.buildPipelineStartPayload({
  projectPath: '/repo',
  flowSource: 'guardrail.dot',
  workingDirectory: '.',
  model: null,
  executionProfileId: 'launch-profile',
  projectDefaultExecutionProfileId: 'project-default'
}, flowContent)
if (payload.execution_profile_id !== 'launch-profile') {
  throw new Error(`expected launch profile, got ${payload.execution_profile_id}`)
}
if ('project_default_execution_profile_id' in payload) {
  throw new Error('explicit launch profile must take precedence over project default')
}
for (const key of ['execution_mode', 'execution_container_image', 'execution_worker_id', 'worker']) {
  if (key in payload) {
    throw new Error(`DOT-authored placement key leaked into launch payload: ${key}`)
  }
}
console.log(JSON.stringify(payload))
""".strip()

    output = run_pipeline_start_payload_probe(
        probe_script,
        temp_prefix=".tmp-pipeline-start-payload-contract-",
        error_context="pipeline start payload execution placement contract",
    )

    assert '"execution_profile_id":"launch-profile"' in output


def test_build_invocation_from_approved_plan_contract_and_error_paths_item_12_4_05() -> None:
    assert_frontend_behavior_contract_passed("12.4.05")
