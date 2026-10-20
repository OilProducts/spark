---
type: parallel
title: Audit Lenses
config:
  join_policy: wait_all
  max_parallel: 4
routes:
- to: lens_correctness
- to: lens_minimality
- to: lens_safety
- to: lens_test_gaps
---
