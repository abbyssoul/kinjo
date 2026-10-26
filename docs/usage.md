# Usage reference

For installation and a first browse, start with the
[README](../README.md#getting-started). This guide explains the views, filters,
discovery backends, and discovery status indicators.

## UI

Default keys follow Vim-style conventions:

- `j` / `down`: move down
- `k` / `up`: move up
- `enter`: show or run matching actions
- `/`: fuzzy text filter
- `t`: service type checklist filter
- `tab` / `shift+tab` (or `←` / `→`): switch view tab
- `s`: narrow the list to the selected row's host (press again to clear)
- `d` / `u` (or `page down` / `page up`): scroll the details pane
- `r` / `F5`: refresh — restart service discovery from scratch
- `?`: help
- `q`: quit

The top panel exposes four tabs — services, hosts, types, and commands. Each tab
swaps the list and details panes to view discovery from that angle: individual
services, hosts and the services they offer, discovered service types, or
configured commands and the services they match. The list also supports fuzzy
text search and service type filtering.

Each tab shows how many rows it lists: logical services, hosts (plus one row for
the registrations that have not resolved a host yet), distinct service types, or
configured commands. The aggregate tabs report only what is true of a whole row.
A host's details name the host and list every service on it — each with its own
type, port, and TXT data — rather than presenting one service's fields as the
host's. A service type's details likewise list every host offering it. Actions
always run against a concrete discovered service, whichever tab they start from.

The indicator at the top left says what discovery is doing, and it only animates
while something is actually happening:

| Indicator | Meaning |
|---|---|
| `⠋` (spinning) | Browsing. The list may still change. |
| `✓` | A sample stream finished normally. Its records stay valid; nothing more will arrive. |
| `✗` | Discovery failed or stopped. Its records are no longer being confirmed. |

A still indicator therefore means the list is final until you refresh (`r`),
which starts a new browse and returns the indicator to spinning. The empty-list
message says the same thing in words, so the two can never disagree. Neither
ending retries by itself.

The type filter (`t`) lists the service types currently being advertised, and
its `types n/m` chip counts only those: `m` is how many types are on the link
right now, and `n` is how many of them you are showing. Switching a type off is
remembered rather than observed, so a device that drops off the link and comes
back stays switched off — but a type nobody is advertising is counted on neither
side of the chip.

The details pane keeps your scroll position while you stay on the same row,
including as discovery re-reports it. Moving to a different row starts its
details from the top, and a row that shortens or a terminal that grows pulls the
view back to the end of the content rather than past it.

The `s` (same-host) filter needs a row with a single host, so it is offered in
the services and hosts tabs. The types and commands tabs report it unavailable
instead of guessing a host; an active filter can be cleared from any tab.

Keybindings are fully customizable: all built-in UI commands can be rebound with
a keybindings config file. See [docs/keybindings.md](keybindings.md) for
the full keybinding reference and examples.

## Discovery backends

The app discovers services over mDNS/DNS-SD so no external CLI tools are
required. Backends are selected exclusively with `--backend`:

- `mdns-sd` (default): the `mdns-sd-discovery` crate. A single browser
  enumerates every service type on the link via the native DNS-SD meta-query.
  It accepts a custom `--domain` and uses the native DNS-SD stack: Avahi over
  system D-Bus on Linux, and built-in system APIs on macOS and Windows.
- `zeroconf`: the `zeroconf-tokio` crate, which talks to the system Avahi daemon
  on Linux. It browses one service type at a time, so a curated set of common
  types is swept in parallel when no `--service-type` is given. This backend is
  behind the off-by-default `zeroconf` cargo feature (it needs the Avahi client
  headers to build, e.g. `libavahi-client-dev` on Debian/Ubuntu).
- `fake`: a finite built-in sample stream for development and deterministic UI
  smoke tests. It is behind the off-by-default `fake` Cargo feature so release
  binaries do not ship actionable sample endpoints unless explicitly requested.

```sh
cargo install kinjo --features zeroconf
kinjo --backend zeroconf
```

Install or build with sample discovery explicitly:

```sh
cargo install kinjo --features fake
kinjo --backend fake
```

Selecting an optional backend in a binary built without it fails with an error
that names the Cargo feature to enable.

`--fake-discovery` has been removed; replace it with `--backend fake`. See the
[release notes](release-notes.md) for the migration decision.

**The `zeroconf` backend browses only the default `local` domain.** Its browser
exposes no way to select a domain, so rather than accept `--domain` and quietly
browse `local` anyway, it refuses the combination up front:

```console
$ kinjo --backend zeroconf --domain corp
error: invalid value for `--domain`: the `zeroconf` backend cannot browse the
`corp` domain: it can only browse the default `local` domain. Browse `local`, or
select the `mdns-sd` backend, which supports custom domains
```

Use the default `mdns-sd` backend to browse a custom domain. Empty, `local`, and
`local.` all name the default domain.

### Limiting discovery to one service type

When a `--service-type` is given, only that type is browsed:

```sh
kinjo --service-type _ssh._tcp
```

The value must be a DNS-SD service type — `_<name>._tcp` or `_<name>._udp`,
where `<name>` is 1–15 ASCII letters, digits, and internal hyphens, begins and
ends alphanumeric, and contains at least one letter. Service types are
case-insensitive, so `_SSH._TCP` and `_ssh._tcp` are the same browse.

A value that is not a service type is rejected before discovery starts, rather
than being ignored in favour of browsing everything — a filter is there to
narrow what the program observes, so a typo must never widen it:

```console
$ kinjo --service-type bogus
error: invalid value for `--service-type`: `bogus` is not a DNS-SD service type:
a service type begins with `_`. Use a type such as `_ssh._tcp` or `_dns-sd._udp`,
or omit it to browse every service type
```

Omit `--service-type` to browse every supported type.

Discovery never falls back to sample records. If mDNS discovery is unavailable,
the list stays empty and the status line explains why. If a running browse stops
unexpectedly, `kinjo` says so and clears the list rather than leaving stale
entries on screen: mDNS is edge-triggered, so once the browse is gone nothing can
report that a listed service has since disappeared, and a command launched at one
could be pointed at a host that is no longer there. Either way the failure and its
cause stay on screen rather than scrolling past — discovery is not retried
automatically. Refresh (`r`) restarts it, and is the recovery action from a
failure.

Select `--backend fake` in a build with the `fake` feature for sample records on
demand. Those samples are a short, finite stream; when it ends the status line
reports normal completion and the samples remain listed.

The sample set is chosen to exercise the behavior the real app has: a service
reachable at several addresses, a service with no resolved host yet, and SSH on
two different hosts — so the `_ssh._tcp` service-type row aggregates children
whose commands differ and asks which host to act on.
