# How Kinjo works

See the [README](../README.md) to install and try Kinjo, or
[CONTRIBUTING.md](../CONTRIBUTING.md) to set up a development environment.

`kinjo` has five moving parts:

1. Discovery finds DNS-SD service records on the network. Each record can carry
   fields such as service name, service type, domain, hostname, address, port,
   and TXT values.
2. Filtering and view tabs organize those records in the UI. You can fuzzy-search
   the visible services, limit by service type, and switch the top-panel tab to
   view discovery by service, host, service type, or matching command.
3. Actions decide what can be done with a selected service. Action command files
   define match predicates such as "service type equals `_ssh._tcp`" or "TXT
   field contains a URL".
4. Command templates turn service fields into executable commands. For example,
   `ssh {hostname}` uses the selected service hostname, while
   `xdg-open http://{hostname}:{port}` builds a URL from the selected instance.
5. Keybindings control the TUI. The defaults use Vim-style navigation, and every
   built-in UI command can be rebound in `keybindings.toml`.

The result is a small local service browser that behaves like a configurable
launcher: discover services, narrow the list, choose a matching action, and run
the command built from that service's fields.

## Architecture

Internally those moving parts live in three deliberately decoupled modules, so
the project is easy to extend and hack on. Each is designed to be swapped or
reused independently:

1. **Discovery** (`src/discovery/`) — the producer of *entries*. An entry is a
   discovered record described entirely by its attributes (name, type, host,
   address, port, TXT, …); `Entry` is the only contract the rest of the program
   depends on. Starting discovery hands back a `DiscoverySession`: one value
   owning the running adapter, its events, its state, and its shutdown, so the
   caller cannot hold a receiver whose producer has silently died, and dropping
   it stops the browse. Adapters vary behind that session — the mDNS/Avahi
   backend is the default, with a feature-gated built-in sample backend selected
   by `--backend fake` — so a different DNS-SD source, a static file, or an
   SSDP/UPnP browser slots in beside them without disturbing anything above
   (see [Extending it](#extending-it) for where that seam is). Discovery options are
   checked once at that seam: a `DiscoveryConfig` is a request, and validating
   it yields the `DiscoveryOptions` that starting an adapter requires. A
   malformed service type, or a domain the selected backend cannot honour, is
   therefore refused before anything spawns — no adapter can be reached with a
   value it would have to quietly reinterpret.
2. **Plumber** (`src/plumber/`) — the rules engine. A serializable collection of
   command rules (the TOML command files) is matched against entries by their
   attributes; multiple rules can match one entry, and a matching rule can be
   executed. It depends only on `Entry`, never on the UI, and sits behind the
   `RuleEngine` trait so an alternative matching strategy can be substituted.
3. **UI** (`src/ui/`) — ties discovery and the rules engine together for a person
   at the terminal: CLI parsing, config and keymap loading, the application
   state machine, and rendering. It depends on the other two; they do not depend
   on it. `App` is the event loop and the state it decides from; its interface is
   six operations — build it, attach a discovery factory and a config loader,
   hand out its reload trigger, run it, and take any reload diagnostics that
   outlived the terminal. Its state is not public. Rendering is a pure function
   of it (`&App` in, a frame out), and the app/render boundary is field
   visibility rather than a projected view — see
   [ADR 0002](adr/0002-render-reads-the-app-directly.md).

The dependency flow is one-directional — `discovery ← plumber ← ui` — wired
together by `run` in `src/lib.rs`, which the `kinjo` binary is a thin wrapper
around.

## Extending it

The two layers are extensible in deliberately different ways, and the difference
is the interesting part:

- **A rule engine is substitutable from outside.** `RuleEngine` is public, and
  `App::new` accepts any implementor, so a crate depending on `kinjo` can bring
  its own matching strategy. `Matcher` is the engine kinjo ships, not the only
  one the interface admits. Doing so means writing your own composition root:
  `run` loads a `Matcher` and uses it concretely, so it is not generic over the
  engine — do what `run` does, with your engine in its place. See
  [ADR 0001](adr/0001-rule-engine-is-a-supported-extension-point.md).
- **A discovery backend is not.** Adapters vary *inside* `src/discovery/`, at
  the browse loop, behind one concrete `DiscoverySession` and a closed
  `DiscoveryBackend` enum. They differ in how they browse but not in how a
  caller runs and stops them, so there is no trait to implement: a new source —
  a static file, an SSDP/UPnP browser — is a module and an enum variant in this
  crate, not something a dependent adds from outside.

See [CONTRIBUTING.md](../CONTRIBUTING.md) for development setup.
