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

Printable characters, backspace, and delete are handled by the search input
itself; they are not configured here.

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

### Help Mode

These commands apply in the help overlay:

```toml
[help]
close = ["esc", "?", "q"]
```

### Common Commands

Common commands are checked before mode-specific bindings:

```toml
[common]
quit = ["ctrl-c"]
```

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
