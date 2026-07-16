# 013: Make Keybindings, Hints, and Search Behavior Consistent

Shared context: [`CONTEXT.md`](../CONTEXT.md).

| Field | Value |
|---|---|
| Status | `done` |
| Priority | `P1` |
| Workstream | UI / Configuration |
| Depends on | — |
| Likely conflicts | 011, 012, 014 |
| Owner | agent-a97ffe5e80357d10e (branch `worktree-agent-a97ffe5e80357d10e`) |

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

- [x] Custom bindings change every relevant on-screen hint/help entry.
- [x] Same-mode and common-versus-mode collisions fail with actionable errors.
- [x] Every key dispatch resolves to one action without order-dependent checks.
- [x] Default bindings behave unchanged and quit remains reachable.
- [x] Delete removes the final search character.
- [x] Escape/Enter retain the query while leaving search mode; clear removes it.
- [x] Documentation and UI text agree.
- [x] Full validation passes.

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
  - `src/ui/keymap.rs` now owns typed `Mode` and `Action` enums. Each action maps
    to exactly one `mode.command` spelling, so the typed value and the file
    syntax are two views of one thing; `Action::parse` replaces the stringly
    `(mode, command)` keys and `KeyBindings` is keyed by `Action`.
  - `KeyBindings::resolve(mode, key) -> Option<Action>` is the single dispatch
    entry point. It considers the mode's own actions plus the always-active
    `common` ones, and returns at most one action. The ordered `is(...)` guard
    chains in `App` are gone; each handler now matches on a resolved `Action`.
  - `ensure_no_collisions` rejects any configuration where two actions active in
    one mode share a key, naming the mode, the key, and both actions. Common
    bindings are checked against every dispatch mode, so common-vs-mode
    collisions are caught. Cross-mode reuse of a key stays legal.
  - `KeySpec::label` is the one place a key is spelled for display; `compact`,
    `compact_group`, `describe`, and `describe_group` expose effective bindings.
    Footer, filter-bar placeholder, modal border hints, and the help overlay are
    all generated from them. Compact = the action's first key (deterministic);
    help keeps every alias. Unbound actions drop out of hints entirely.
  - The browse footer's quit hint falls back from `browse.quit` to `common.quit`,
    so the guaranteed-reachable quit is always the one advertised.
  - Search: `Delete` now removes the last character exactly like `Backspace`
    (the editor is append-only, so there is no cursor to delete forward from).
    `close` still leaves editing with the query intact; `clear` remains the only
    full clear. Bound `[search]` actions take precedence over the built-in
    editor keys, rather than `Backspace` being hard-coded ahead of them.
  - Fixed a latent input bug found while replacing the char fallback: an unbound
    control chord (e.g. `ctrl-w`) arrives as `Char('w')` with CONTROL set and
    used to type its letter into the query. `typed_char` now excludes
    CONTROL/ALT while keeping SHIFT (which is folded into the character).
  - Default bindings and aliases are unchanged. Collision validation found **no**
    genuine ambiguity in the defaults; a test now pins that.
- **Tests added/updated:** 27 net new tests (163 baseline → 270 `--all-targets`).
  - `ui::keymap`: single-action resolution per mode; common actions live in every
    mode; cross-mode key reuse allowed; two-browse-action collision; common-quit
    vs `type_filter.toggle` collision; resolving a collision by rebinding the
    other side; repeated key within one action is not a collision; defaults are
    collision-free and quit-reachable; unbinding one quit action is allowed;
    empty arrays unbind and produce no hint; unbound action drops out of a
    grouped hint; default and custom formatting incl. function/control keys;
    action spellings round-trip and are unique; every action has a default; every
    example in `docs/keybindings.md` is accepted by the loader.
  - `ui::app`: Delete removes the last character and recomputes rows; Escape and
    Enter both close while preserving the query and the filtered list; clear
    empties and stays in search; rebound clear replaces `ctrl-u`; unbound control
    chord does not type; shifted characters still type; rebound browse
    navigation/invoke/help dispatch while defaults stop working; unbound action
    is inert.
  - `ui::render` (TestBackend): default footer hints; rebound navigation/invoke/
    help/search change the footer and the old defaults disappear; unbound actions
    have no hint; footer falls back to the common quit hint; placeholder names
    the configured search key; rebound picker keys change the popup hint; help
    lists every alias; rebinding changes help; help no longer claims Escape
    clears search.
- **Documentation updated:** `docs/keybindings.md` — new "Conflicts" section
  (error shape, common-vs-mode conflicts, cross-mode reuse being legal) and a
  "Hints" section (hints follow customization; compact vs complete). Corrected
  the false claim that "common commands are checked before mode-specific
  bindings" — they no longer win by precedence; a shared key is now an error.
  Documented that `close` keeps the query while `clear` is the only full clear,
  and that the append-only field treats Backspace and Delete identically.
  The in-app help's "esc — close modal / clear search" row, which contradicted
  the code, is replaced by "leave search, keep filter" and "clear the search
  filter".
- **Validation evidence:**
  - `cargo fmt -- --check`: clean.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
  - `cargo test --locked --all-targets`: 270 passed.
  - `cargo test --locked --all-targets --all-features`: 276 passed.
  - Drove the real binary (not only tests). Invalid configs are rejected at
    startup with the intended messages:

    ```text
    keybinding conflict in `browse` mode: `z` is bound to both
    `browse.same_host` and `browse.refresh`

    keybinding conflict in `type_filter` mode: `space` is bound to both
    `common.quit` and `type_filter.toggle`

    keybindings leave no way to quit: bind `common.quit` or `browse.quit`
    ```

    With `search=["f"] down=["ctrl-n"] up=["ctrl-p"] help=["f1"]` the live TUI
    footer read `^n/^p move · ⏎ open · f search · … · F1 help · q quit`, the
    filter bar read `press f to search`, and `F1` opened a help overlay showing
    `esc / ⏎ leave search, keep filter` above `^u clear the search filter`.
  - Help overlay fit verified at 80x24, 100x30, 120x40, 160x50: the last row and
    the badges line render, so the longer key column is not clipped. The popup
    was widened to 72% and heightened to 80% to accommodate complete labels.
- **Follow-ups:**
  - The help overlay is a fixed-percentage popup, so its content still clips on
    terminals shorter than roughly 24 rows. Task 011 (scrollable pickers) covers
    the adjacent "selection visible at every supported size" invariant; extending
    it, or a successor, to the help overlay would close this. Not in this task's
    scope and not a regression — the previous hard-coded help clipped sooner.
  - `delete` is not a bindable key *name* even though `backspace` is: Delete is
    handled by the search editor only. Worth a small task if users want to bind
    it, and it would need a `KeySpec` label for `KeyCode::Delete`.
  - Mouse behavior remains unconfigurable and untouched (a stated Non-Goal).
