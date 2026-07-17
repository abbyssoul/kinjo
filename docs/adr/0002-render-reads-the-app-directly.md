# ADR 0002: Render reads the app directly, not a projected view

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-07-17 |
| Decider | Ivan Ryabov (project owner) |
| Context | [review-backlog task 015](../review-backlog/tasks/015-app-encapsulation.md) |

## Context

Task 015 required that "render consumes an immutable, explicit view/interface
rather than the full `App`". The stated worry was real: `App` had 30 public
fields, `render(frame, app: &App)` could read all of them, and nothing recorded
which ones it actually needed.

Two things changed the shape of that problem before this decision was taken.

Task 014 had already made rendering immutable. `render` takes `&App` and writes
nothing back, so "not the entire *mutable* App state" was already true; what
remained was only "not the *entire* App".

Then task 015's privatisation (`f71d2c8`) made every field either `pub(crate)`
or private. Private now means render cannot see it at all — `matcher`,
`picker_anchor`, `should_quit`, `config_loader`, `discovery_factory`,
`reload_requested`, and `reload_diagnostics` are outside its reach, enforced by
the compiler. Of the 21 fields left visible, **20 are read by production
render**. The exception is `records`, reachable so that render's own tests can
arrange the state render displays.

So the question became concrete: what would a projected view add, given that?

It would list those same ~20 values behind a struct. That is not a smaller
interface; it is the same interface with a copy step and a second type to keep
in sync. Every render function signature would change, `App` would gain a
projection method whose job is to restate its own fields, and a reader wanting
to know what render needs would consult a struct that says "all of it" instead
of a `pub(crate)` marker that already says exactly that. Task 015's own
constraints warned against precisely this: "avoid a monolithic replacement with
an equally broad interface" and "depth means App asks for meaningful operations
and view projections, not getters for every old field."

The objective is readability, extendability, and hackability. A 20-field view
struct improves none of them.

## Decision

`render` keeps `&App`.

The boundary between the app's state and its rendering is expressed by field
visibility rather than by a projected type:

- **private** — render has no business with it, and the compiler agrees.
- **`pub(crate)`** — render draws it.

That is a compiler-checked statement of what rendering depends on, which is what
the view struct was wanted for, at no cost.

This decision is scoped to the *whole-App view projection*. It does not license
render to grow: a new `pub(crate)` field is a claim that rendering needs it, and
should be read as one in review.

## Consequences

Positive:

- No per-frame projection, no second type tracking `App`'s fields, no signature
  churn across ~1700 lines of render.
- The visibility split already carries the information a view would have.
- Render stays immutable by type (`&App`), which is the guarantee that actually
  mattered.

Negative, accepted knowingly:

- `pub(crate)` is coarser than a view: render *may* read any of the 21 visible
  fields, and nothing stops a future render function reading one it has no
  business with. The guard is review, not the compiler.
- `records` is visible to render though production render never reads it, purely
  so render's tests can arrange state. That is one field's worth of overshoot,
  recorded here rather than hidden.
- If `App` grows state that render must not see, the answer is a private field,
  and if that becomes common the projection question is worth reopening.

## Notes

Task 015's other structural requirement — that parallel representations be made
construction-atomic — *was* implemented (`eb88766`), because there the
abstraction paid for itself: `visible_groups` and `group_matches` were kept in
correspondence by index, render hedged against a desync it could not rule out,
and tests could set one without the other. Merging them into `BrowseRow` deleted
those hedges. The contrast is the point of this ADR: an abstraction earns its
place by removing a way to be wrong, not by satisfying a shape.

For the same reason, the browse-model extraction task 015 also proposed —
moving `records`, `filter`, `rows`, and `selected` behind a `BrowseModel` — was
not done. With the state private and rows atomic, it would have been code motion
without new leverage. If `App` later grows a second consumer of that state, or
its invariants start being reconstructed in more than one place, that is the
evidence to revisit it.
