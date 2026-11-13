---
'moqtail': minor
---

The player plays LOCMAF tracks (draft-einarsson-moq-locmaf-01, `locmafVersion` 0.3): a built-in `locmaf` source rebuilds each object into its canonical CMAF chunk for the MSE sink. Where a catalog offers the same media as CMAF and LOCMAF, the LOCMAF switching set is chosen; the two never share a set.
