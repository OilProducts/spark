import Mathlib

/-- Smoke check: Mathlib is importable and the kernel accepts a proof. -/
theorem mathlab_smoke (n : ℕ) : n ≤ n + 1 := Nat.le_succ n
