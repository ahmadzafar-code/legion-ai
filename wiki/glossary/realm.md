---
title: Realm
slug: realm
summary: The low-level, event-based portability runtime beneath Legion; provides processors, memories, region instances, copies, and events as primitives Legion composes into its higher-level model.
tags: [execution, synchronization]
subsystem: realm
layer: runtime-internals
status: stub
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/event.md
  - wiki/concepts/physical-instance.md
  - wiki/concepts/operation-pipeline.md
  - wiki/concepts/region-instance.md
  - wiki/concepts/dma-system.md
  - wiki/concepts/reservation.md
  - wiki/concepts/realm-machine-model.md
---

Realm is the substrate. Paper: `raw/publications/pdfs/realm2014.pdf`. Lectures: `raw/youtube_transcripts/realm_school_2023/`. The Legion runtime (`runtime/legion/`) is built on top of Realm (`runtime/realm/`). Realm's primitives have their own concept pages: [`event`](../concepts/event.md), [`region-instance`](../concepts/region-instance.md), [`dma-system`](../concepts/dma-system.md), [`reservation`](../concepts/reservation.md), [`realm-machine-model`](../concepts/realm-machine-model.md).
