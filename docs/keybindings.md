# Custom Keybindings

All built-in UI commands can be rebound from a keybindings file. The file is
loaded from:

```sh
$XDG_CONFIG_HOME/kinjo/keybindings.toml
```

If `XDG_CONFIG_HOME` is not set, the fallback path is:

```sh
~/.config/kinjo/keybindings.toml
```

The file overlays the default bindings. You only need to include commands you
want to change.

## Format

Keybindings are grouped by UI mode. Each command is assigned an array of quoted
key names:

```toml
[browse]
down = ["down", "j", "n"]
up = ["up", "k", "p"]
invoke = ["enter", "o"]
search = ["/"]
help = ["?"]
```

Setting a command to an empty array disables that command's binding in that
mode:

```toml
[browse]
same_host = []
```

The file is standard TOML. Mode and command names are validated against the
set listed under [Bindable Commands](#bindable-commands); a typo such as
`[brwose]` is reported as an error instead of being silently ignored. A
configuration that unbinds every quit key (`common.quit` and `browse.quit`) is
rejected, so the app always remains quittable.

## Conflicts

A key triggers exactly one command. If two commands that are active at the same
time are bound to the same key, the file is rejected with an error naming the
key and both commands, rather than letting one of them silently win:

```text
keybinding conflict in `browse` mode: `z` is bound to both
`browse.same_host` and `browse.refresh`
```

Because `[common]` commands stay active inside every mode, a common binding
conflicts with any mode's binding on the same key. Binding `common.quit` to
`space`, for example, conflicts with the default `type_filter.toggle`. Rebinding
whichever command you did not want resolves it:

```toml
[common]
quit = ["space"]

[type_filter]
toggle = ["enter"]
```

The same key in two *different* modes is not a conflict: only one mode is
listening at a time, so `browse.same_host = ["z"]` and `picker.select = ["z"]`
happily coexist.

## Hints

The footer, the help overlay, and the modal hints are generated from the
bindings that are actually in effect, so they follow your customization. Where
space is tight a command is shown with its first key; the help overlay lists
every key bound to it. An unbound command is dropped from the hints entirely
rather than advertising a key that does nothing.

## Mouse

The mouse works alongside the keyboard while browsing: the wheel over the
service list moves the selection, the wheel over the details pane scrolls its
content, and a left click on a list row selects it. Modal dialogs (pickers,
help) remain keyboard-driven. Mouse behavior is not configurable.

## Key Names

Supported special keys:

- `up`
- `down`
- `left`
- `right`
- `enter`
- `pageup`
- `pagedown`
- `esc` or `escape`
- `space`
- `tab`
- `backtab` (Shift+Tab)
- `backspace`
- `f1` … `f12` (function keys)

Single-character keys are written as the character itself:

```toml
[browse]
quit = ["q"]
search = ["/"]
help = ["?"]
```

Control keys use the `ctrl-` prefix:

```toml
[common]
quit = ["ctrl-c"]

[search]
clear = ["ctrl-u"]
```

Key names are case-insensitive when parsed.

## Bindable Commands

### Browse Mode

These commands apply in the main service browser:

```toml
[browse]
quit = ["q"]
up = ["up", "k"]
down = ["down", "j"]
invoke = ["enter"]
search = ["/"]
type_filter = ["t"]
tab_next = ["tab", "right"]
tab_prev = ["backtab", "left"]
same_host = ["s"]
refresh = ["r", "f5"]
details_down = ["d", "pagedown", "ctrl-d"]
details_up = ["u", "pageup", "ctrl-u"]
help = ["?"]
```

`tab_next` and `tab_prev` switch the active top-panel view tab
(services / hosts / types / commands), wrapping around the ends.

`same_host` narrows the list to the selected row's host. It needs a row with a
single host, so it applies in the services and hosts tabs; the types and
commands tabs report it unavailable rather than guessing a host from one of a
row's children. An active host filter can be cleared from any tab.

`refresh` restarts service discovery from scratch: the list empties and
repopulates as the fresh browse reports services, exactly like app startup.
Filters and the active view are kept.

### Search Mode

These commands apply while editing the fuzzy search:

```toml
[search]
close = ["esc", "enter"]
clear = ["ctrl-u"]
```

`close` leaves the search editor but **keeps** the query: it stays the active
filter and the list stays narrowed. `clear` is the only command that empties
the query, and it leaves you in the editor ready to type a new one.

Printable characters, backspace, and delete are handled by the search input
itself; they are not configured here. The search field is append-only, so
backspace and delete both remove the last character — there is no cursor to
move. Binding one of them to a `[search]` command takes precedence over that
built-in behavior.

### Type Filter Mode

These commands apply in the service type checklist:

```toml
[type_filter]
close = ["esc", "t"]
up = ["up", "k"]
down = ["down", "j"]
toggle = ["space", "enter"]
```

### Picker Mode

These commands apply in action, instance, and service pickers:

```toml
[picker]
close = ["esc"]
up = ["up", "k"]
down = ["down", "j"]
select = ["enter"]
```

`up` and `down` move the selection, and the picker follows it: the selected
entry is on screen at every terminal size, so the target you are about to run is
always the one you can see. A picker with more entries than fit shows a
`first-last/total` chip in its title and a scrollbar on its right border.

### Help Mode

These commands apply in the help overlay:

```toml
[help]
close = ["esc", "?", "q"]
up = ["up", "k", "pageup"]
down = ["down", "j", "pagedown"]
```

The help overlay lists every key bound to every command, so how tall it is
depends on your bindings. When it does not all fit — on a short terminal, or
after adding aliases — `up` and `down` scroll it a line at a time, the title
shows which lines you are looking at, and the bottom border names the keys that
scroll. Help that fits on screen says none of that, because there is nowhere to
scroll to.

### Common Commands

Common commands stay active in every mode, including inside the modal views:

```toml
[common]
quit = ["ctrl-c"]
```

Because they are always active, they must not share a key with any mode's own
command — see [Conflicts](#conflicts).

## Examples

Use arrow keys only in the browser and disable Vim navigation there:

```toml
[browse]
up = ["up"]
down = ["down"]
details_up = ["pageup"]
details_down = ["pagedown"]
```

Use Emacs-style navigation for lists:

```toml
[browse]
up = ["up", "ctrl-p"]
down = ["down", "ctrl-n"]

[type_filter]
up = ["up", "ctrl-p"]
down = ["down", "ctrl-n"]

[picker]
up = ["up", "ctrl-p"]
down = ["down", "ctrl-n"]

[help]
up = ["up", "ctrl-p"]
down = ["down", "ctrl-n"]
```

Move search from `/` to `f`:

```toml
[browse]
search = ["f"]
```

Use `x` to close modal views:

```toml
[type_filter]
close = ["esc", "x"]

[picker]
close = ["esc", "x"]

[help]
close = ["esc", "x"]
```
