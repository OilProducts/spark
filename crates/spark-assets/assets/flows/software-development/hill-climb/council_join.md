---
type: fan_in
title: Join Council
runtime:
  max_visits: 64
routes:
  next:
    to: tally
    description: Every council member has settled; tally the scores, counting any member that did not score as not passing.
---
