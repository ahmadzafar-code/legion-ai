---
title: Shard
slug: shard
summary: One of N replicated copies of a task under control replication; each shard executes the same control flow but owns a slice of the work.
tags: [replication, distributed]
subsystem: legion
layer: runtime-internals
status: stub
created: 2026-05-15
updated: 2026-05-15
related:
  - wiki/concepts/control-replication.md
  - wiki/concepts/sharding-functor.md
---

A shard is the unit of replication. See [`control-replication`](../concepts/control-replication.md) for the full mechanism and [`sharding-functor`](../concepts/sharding-functor.md) for how shards agree on per-point ownership.
