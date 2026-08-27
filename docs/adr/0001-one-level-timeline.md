# ADR 0001: One append-only timeline per level

## Status

Accepted.

## Context

The Unity editor has local snapshot Undo/Redo. Oreak adds live collaboration,
auditing, contribution metrics, review snapshots, and element-level blame.
Git-like level branches and arbitrary merges would add conflict semantics for
spatial geometry and raster artwork without improving the primary workflow.

## Decision

Each level has one shared, server-ordered command timeline. Commands are atomic
and contain deterministic touched-target information. Gesture previews stay
local and pointer-up submits one command.

Undo, restoration, and rollback append compensating events. Review candidates
and release artifacts freeze specific timeline sequences without forking it.

## Consequences

- Blame and contribution attribution are explainable from one sequence.
- History is never rewritten.
- Collaboration does not require CRDT document merging.
- Reverting a change can conflict when a later active command touched the same
  target; Oreak reports that conflict instead of overwriting it.
- Experimental work uses a separate duplicated level rather than an invisible
  branch of the production level.
