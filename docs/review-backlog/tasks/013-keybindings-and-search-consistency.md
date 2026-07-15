# 013: Make Keybindings, Hints, and Search Behavior Consistent

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `ready` |
| Priority | `P1` |
| Workstream | UI / Configuration |
| Depends on | — |
| Likely conflicts | 011, 012, 014 |
| Owner | Unclaimed |

## Why This Matters

Input dispatch uses configurable bindings, while footer/help/popup hints hard-code
defaults. After rebinding, the UI can instruct users to press keys that no longer
work. The keymap also accepts same-mode collisions; match order then silently makes
one action unreachable. Search documentation promises Delete support, and help
claims Escape clears search, but the state machine implements neither claim.

The keybinding module already provides real leverage and should be deepened: it
should own typed actions, resolution, collision validation, and display labels.

## Evidence

- `src/ui/keymap.rs:17-70`: stringly `(mode, command)` mappings drive dispatch.
- `src/ui/keymap.rs:82-107`: file loading validates names but not key collisions.
- `src/ui/app.rs:328-383`: ordered `is(...)` checks determine hidden precedence.
- `src/ui/render.rs:642-660,700-856`: footer, popup, and help keys are hard-coded.
- `src/ui/app.rs:388-405`: search handles Backspace but not Delete; close retains
  the query.
- `docs/keybindings.md:123-134`: Delete is documented as handled by search input.
- `src/ui/render.rs:830`: help says Escape clears search, contradicting close semantics.

## Required Outcome

- Give modes/actions a typed representation shared by keymap and application input.
- Resolve a key to at most one active action for the current mode, accounting for
  common bindings.
- Reject user configurations where active actions collide in a mode, with an error
  naming the key and both actions. Common-versus-mode collisions count.
- Expose formatted effective bindings so footer, help, and popup hints reflect
  customization. When several keys bind one action, choose a deterministic compact
  representation and retain a complete representation in help where space permits.
- Implement Delete like Backspace in the append-only search editor.
- Keep Escape/Enter semantics: close editing while preserving the active query.
  The configured clear action remains the only full clear operation.
- Correct all hints and `docs/keybindings.md` to match those semantics.

## Implementation Constraints

- Preserve current default bindings and aliases unless collision validation reveals
  a real default ambiguity.
- SHIFT remains encoded in printable `KeyCode::Char`; ALT support is out of scope.
- Key display formatting belongs to the keymap module, not duplicated in render.
- Ensure at least one reachable quit action after customization.
- The deletion test says KeyBindings earns its keep: deleting it would spread real
  parsing/resolution behavior. Deepen it rather than replacing it with App logic.

## Suggested Implementation Sequence

1. Add collision and effective-binding display tests.
2. Introduce typed mode/action keys and one resolution method.
3. Replace ordered App checks with resolved action dispatch.
4. Generate footer/help/popup hints from effective bindings.
5. Implement Delete and correct Escape/clear language in docs/render.

## Non-Goals

- Adding ALT/meta or multi-key chord syntax.
- Interactive key rebinding.
- Mouse behavior.
- Changing search from append-only to cursor-based editing.

## Acceptance Criteria / Definition of Done

- [ ] Custom bindings change every relevant on-screen hint/help entry.
- [ ] Same-mode and common-versus-mode collisions fail with actionable errors.
- [ ] Every key dispatch resolves to one action without order-dependent checks.
- [ ] Default bindings behave unchanged and quit remains reachable.
- [ ] Delete removes the final search character.
- [ ] Escape/Enter retain the query while leaving search mode; clear removes it.
- [ ] Documentation and UI text agree.
- [ ] Full validation passes.

## Required Tests

- Rebind browse navigation/invoke/help and assert TestBackend hint text.
- Collision between two browse actions; collision between common quit and modal action.
- Default and custom binding formatting, including function/control keys.
- Delete, Backspace, Escape, Enter, and clear search semantics.
- Empty arrays/unbound actions and reachable-quit validation.

## Validation

```sh
cargo test --locked ui::keymap
cargo test --locked ui::app::tests::search
cargo test --locked ui::render
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
```

## Completion Record

- **Implemented:**
- **Tests added/updated:**
- **Documentation updated:**
- **Validation evidence:**
- **Follow-ups:**
