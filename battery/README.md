# battery — the live half of the ergonomics experiment

Static atom counts (ergonomics.md) are a proxy. This directory holds
the materials for the real measurement: a model performs tasks against
the suite, and we record **total tokens until green**, pass@1, and
retry count — per language variant, per model.

## Protocol

For each task in `tasks.md`:

1. Give the model: `ssa.md` (the variant under test), the task prompt,
   and nothing else about the codebase.
2. The model writes/edits the target `.ssa` file.
3. Run `cargo run -q test <dir>` (or the task's stated command). Green =
   done; otherwise feed back the failure output verbatim and let it
   retry (cap: 5 attempts).
4. Record: tokens in + out per attempt (from the API/CLI usage report),
   attempts to green, wall time.

Variant comparison: the same battery against (a) the current sugared
language, (b) the pre-sugar language (checkout `369df52`-era ssa.md and
disable sugar via a flag if we add one — or just instruct "flat forms
only"). Cross-model runs separate structural ergonomics from
training-prior familiarity.

Scoring: E[cost] = Σ tokens over all attempts. Report per-task and
aggregate; watch the retry-tail distribution, not just the mean —
the hypothesis is that sugar wins mostly by shrinking attempt 1 and
types/verifier win by cutting the tail.

## Task design notes

- write-new tasks state ONLY the directives (the behavioral contract);
  the model chooses everything else.
- edit tasks hand the model a real corpus function and a behavior
  change; measures reading cost of existing code, which greenfield
  tasks miss.
- debug tasks seed one defect into a working function; the suite's
  failure output is the only clue.
- Tasks avoid arithmetic-in-the-head in EXPECTED values (the horner
  lesson: compute directives with a reference, don't trust mental math).
