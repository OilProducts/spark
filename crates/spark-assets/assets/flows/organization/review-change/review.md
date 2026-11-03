---
type: agent_task
title: Review
role: adversarial-reviewer
contracts:
  reads_context:
  - internal.org_root
  - context.request.proposal_path
  - context.request.objective
  writes_context:
  - context.review.verdict
  - context.review.summary
  response: decision_envelope
routes:
  approve:
    to: done
    description: The completed review accepts its subject under every applicable acceptance criterion; write any required review outputs.
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  reject:
    to: done
    description: The organization change is unnecessary or violates the authority and safety rule; record the rejection and its evidence.
  revise:
    to: done
    description: The completed review requires revision of its subject within the existing scope; record the specific corrections and their evidence.
  unfinished:
    to: review
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Under review: a proposed change to the organization, the unified diff at context.request.proposal_path, against the live definitions at internal.org_root. Do not modify any files. Read the diff, the current files it touches, and the rest of the organization's spec, roles and operator brief, so that you judge the change against the whole organization, not the diff alone. Check that the diff applies cleanly to the live definitions (`git apply --check <proposal>` from the project root) and touches nothing outside them; a diff that does not apply or reaches outside the definitions is revise. context.request.objective says why the change is needed; it is a claim, so check it against the evidence it cites.

Lean toward fewer, more general rules. Every rule, step, check and role the organization adds is carried by its agents from then on, and an organization that answers each failure with a new rule accumulates process for edge cases. An addition has to earn its place. Ask these questions in order and stop at the first one the change fails:

1. Is it needed? It must answer the operator's request, or a problem that has occurred more than once, and its evidence must show that. A single failure is fixed where it happened, in the node or prompt that failed, not with a new rule.
2. Does something already cover it? If an existing rule, role, flow or step covers the case, the change amends or applies that one instead of adding another.
3. Is it general? It states the principle, not the incident: no run ids, dates or one-off cases in the rule itself.
4. Is it the smallest change that works? Count what it adds and what it removes (requirements, steps, nodes, checks, roles), and name anything it makes redundant but leaves in place.
5. Does it conflict with any other rule of the organization?

Simplicity never justifies removing or weakening authority, limits, safety constraints, credential handling or independent review. Reject a change that widens what the organization may do without asking, loosens a limit or safety constraint, or weakens a requirement that protects them, unless the operator brief records the operator approving that change. Also check that the change names the role that proposed it, attributes nothing an agent wrote to the operator, and appears in the organization's changelog if it keeps one.

Choose `approve` when the change passes every question; `revise` when a smaller or more general change would do, describing that change; `reject` when it is not needed or breaks the rule on authority and safety. Choose `blocked` only when you cannot perform the review at all, such as when the proposal cannot be read. Cite the file and line for each finding. With approve, revise or reject, set context_updates for context.review.verdict (approve, revise or reject, matching your decision) and context.review.summary (the findings and, for revise, the smaller change). Respond with ONLY a JSON decision envelope.
