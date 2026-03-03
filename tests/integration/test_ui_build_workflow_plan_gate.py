from pathlib import Path


def test_build_workflow_launch_requires_approved_plan_state_item_8_5_04() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const activeProjectScope = useStore((state) =>",
        "const buildWorkflowLaunchReady = Boolean(activeProjectScope?.planId) && activeProjectScope?.planStatus === 'approved'",
        "if (!buildWorkflowLaunchReady) {",
        "setRunStartError('Build workflow launch requires an approved plan state.')",
    ]

    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing build workflow approved-plan gate snippet: {snippet}"
