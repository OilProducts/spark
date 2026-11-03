---
type: agent_task
title: Ingest Spec
contracts:
  response: decision_envelope
routes:
  done:
    to: extract_declarations
    description: The spec source is loaded and working state is prepared.
---

Load the test spec source and prepare working state.
