---
type: fan_in
title: Join Judges
runtime:
  max_visits: 64
routes:
  next:
    to: tally
    description: Every judge has settled; tally the scores, counting any judge that did not score as not passing.
---
