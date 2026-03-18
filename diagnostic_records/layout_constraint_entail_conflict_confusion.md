id: layout_constraint_entail_conflict_confusion
title: Confusing layout constraint entailment vs conflict causes missed or wrong instance matches
source: Transcript 004 (Instance Allocation, Memory Managers, GC)
confidence: low
user_type: legion_cpp

symptoms:
  what_you_see: |
    [INCOMPLETE — needs review: no specific profiler signature described.
    Manifests as either (a) instances that should match a request never matching
    (causing unnecessary allocations) or (b) instances matching that shouldn't
    (causing incorrect data layouts). The profiler would show unexpected allocation
    patterns or data corruption.]

  key_metrics: |
    - Instance reuse rate lower than expected (entailment confused with conflicts)
    - OR incorrect instance sharing (conflicts confused with entailment)
    - Unexpected allocation events despite available instances

  distinguishing_features: |
    This is a mapper logic error, not a runtime performance issue. Distinguished from
    instance churn (which is about create/destroy frequency) by the root cause being
    in constraint specification, not allocation strategy.

root_cause: |
  Layout constraints use two distinct tests: entailment (is layout A a kind of
  layout B? — the type hierarchy test) and conflicts (can layout A never be layout B?
  — the mutual exclusion test). The instructor uses the animal/dog/cat type hierarchy
  analogy. Using conflicts where entailment was intended causes instances to never
  match. Using entailment where conflicts was intended causes incorrect matches.

gotchas:
  - "The animal/dog/cat analogy: 'dog entails animal' (true), 'cat conflicts dog' (true), 'animal entails dog' (false). Getting the direction wrong breaks matching."
  - "Constraint errors are silent — you get wrong behavior, not error messages."
  - "This is a mapper-level logic error that the runtime cannot detect or correct."

fix:
  primary: |
    Review all layout constraint specifications in the mapper. Use entails() when
    testing 'is A a subtype of B?' and conflicts() when testing 'can A never be B?'.
    Refer to the type hierarchy analogy: entailment is subset/subtype, conflict is
    mutual exclusion.

  alternatives: |
    Use the DefaultMapper's constraint specifications as a reference for correct usage.

  what_not_to_do: |
    Do NOT assume entailment and conflicts are interchangeable — they test fundamentally
    different properties of the type hierarchy.

verification: |
  After correcting, instance reuse patterns should change. Verify with Legion Spy that
  the correct instances are being selected for each task's requirements.

real_cases:
  - case: "[No specific case cited]"
    app: "[not specified]"
    scale: "[not specified]"
    result: "[not specified]"
    key_detail: "The instructor emphasizes 'keep this analogy in your head' — suggesting this is a common confusion"

related_patterns:
  - "instance_churn_expensive_gc"
