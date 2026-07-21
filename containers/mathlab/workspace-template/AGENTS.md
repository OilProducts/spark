# Agent guidance for MathLab

## Proof integrity

A mathematical claim exists only as its machine-checked artifact. The gates
are not negotiable and may not be redefined:

- A Lean result counts only if `lake build` succeeds on the exact stated
  theorem with **zero `sorry`/`admit`**, and `#print axioms <theorem>`
  reports nothing beyond the standard Mathlib axioms (`propext`,
  `Classical.choice`, `Quot.sound`). Introducing a new `axiom` to make a
  proof compile is fabrication, not progress.
- A SAT-based result counts only with a DRAT certificate that `drat-trim`
  verifies. An exhaustive search counts only if the search program, its
  exact inputs, and its output digest are committed alongside the claim.
- Weakening a theorem statement to make it provable is legitimate
  mathematics ONLY when the weakening is stated plainly in the result —
  never silently.
- Distinguish clearly between: (a) a new proof, (b) a formalization of a
  known proof, and (c) a result located in existing literature. All three
  are valuable; mislabeling one as another is a violation.

## Working style

- Compute before conjecturing: use the installed engines (PARI/GP, nauty —
  binaries are prefixed on Ubuntu: nauty-geng, nauty-directg, etc. —,
  cadical/kissat, z3/cvc5, HiGHS/CSDP, the Python stack at
  /opt/mathlab/venv) to build evidence before attempting proof.
- Start Lean work by copying /opt/mathlab/template (Mathlib is prebuilt at
  /opt/mathlab/mathlib4; the pinned commit is /opt/mathlab/MATHLIB_COMMIT).
  Never modify the shared Mathlib checkout.
- Small, committed increments: each verified lemma or checked certificate
  is a commit, not a chat message.
