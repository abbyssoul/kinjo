# Avahi TUI Implementation Plan

## Feasibility Review

The project is clearly enough defined to implement an MVP. The core product is a Rust terminal UI that discovers local mDNS/DNS-SD services through Avahi-compatible discovery, displays them live, matches each service against user-defined command files, and launches the selected action.

The current docs define the major architecture:

- Rust CLI application.
- Ratatui-based async TUI.
- Clap for command-line parsing.
- Zeroconf/mDNS browser for service discovery.
- A TEA-style split between state, view, events, and commands.
- A `plumber` module for immutable service-to-command matching built from modular TOML files.

The underspecified areas are manageable if treated as explicit MVP assumptions rather than blockers.

## MVP Assumptions

- The first version targets Linux systems with Avahi/mDNS available on the local network.
- The service discovery backend can be abstracted behind a trait so the exact crate can be swapped if `zeroconf` has platform or async limitations.
- The matcher language uses structured TOML match blocks rather than a custom expression string:
  - String fields support equality and regex matching.
  - `port` supports exact values first, with ranges added after the core flow works.
  - `address` supports exact IP matching first, with CIDR support added after the core flow works.
- Config files define one command each.
- Built-in defaults should be loaded first, then user config files should extend or override by command id/name.
- Command interpolation should support the documented service fields and must shell-escape or avoid shell interpretation where possible.
- `fork` means spawn a child process and return to the TUI.
- `execute` means restore the terminal and replace the current process with the command on Unix.
- Duplicate records for the same logical service are grouped in the service list, with the UI showing the number of discovered instances.
- Unresolved services are displayed while hostname, address, or port details are still pending.
- Keybindings are configurable, with Vim-style defaults where there is no stronger convention.
- Filtering is a central UI feature:
  - free text fuzzy search filters services by display name and useful identifying fields.
  - service type filtering uses a checklist-style menu where all discovered service types are enabled by default.
  - text search and type filters are combined.
- Grouping is configurable at runtime so users can inspect services by logical service, host, service type, port, or address.

## Open Decisions

These do not block an MVP, but should be resolved before polishing behavior:

- Whether command strings are executed through a shell or parsed as argv.
- Exact schema for configurable keybindings.
- Exact grouping identities for each grouping mode. The default logical-service key is likely service name, service type, domain, hostname, and port, with address kept at the instance level.

## Proposed Architecture

### Modules

- `cli`
  - Parses flags and subcommands.
  - Initial flags:
    - `--config-dir <path>`: additional config directory.
    - `--no-default-config`: disable bundled defaults.
    - `--service-type <type>`: optional discovery filter for development/testing.

- `app`
  - Owns application state and the main async loop.
  - Receives terminal events, discovery events, matcher reload events, and command completion events.
  - Keeps raw discovered records, filtered/grouped view state, selected row, active modal, status messages, and service list.
  - Recomputes visible rows when services, filters, or grouping mode change.

- `events`
  - Merges crossterm events, render ticks, discovery events, and internal app events.
  - Follows the async Ratatui pattern used by `ref/crates-tui`.

- `discovery`
  - Defines a `ServiceDiscovery` trait and event stream.
  - Converts backend-specific records into internal `ServiceRecord` values.
  - Emits service added, updated, and removed events.

- `service`
  - Defines the normalized service model:
    - `name`
    - `service_type`
    - `domain`
    - `hostname`
    - `address`
    - `port`
    - `txt`
  - Defines grouped display models that collapse records according to the selected grouping mode while preserving individual instances for action execution.
  - Provides grouping keys for logical service, host, service type, port, and address.

- `filter`
  - Stores the active free text query, selected service types, and selected grouping mode.
  - Applies fuzzy matching and service-type inclusion filters to raw service records.
  - Produces filtered records before grouping.

- `plumber`
  - Loads command config files.
  - Validates config structure and external requirements.
  - Builds an immutable `Matcher`.
  - Returns matching actions for a selected `ServiceGroup` or exact `ServiceRecord`.
  - Reports whether an action requires an exact instance because its matcher or command template depends on instance-specific fields such as address or port.

- `process`
  - Executes selected actions.
  - Handles terminal restore before `execute`.
  - Handles child spawning for `fork`.

- `ui`
  - Renders service table, filter/search input, service-type filter menu, grouping selector, details panel, status bar, help popup, and action picker modal.
  - Shows grouped rows with counts, for example `printer.local (10)` or `host-a.local (4 services)`.
  - Shows unresolved fields as pending/unknown rather than hiding the service.

- `config`
  - Resolves XDG config paths.
  - Loads bundled defaults and user config directories.
  - Watches/reloads config later if desired.

## Data Model

### ServiceRecord

```rust
pub struct ServiceRecord {
    pub id: ServiceId,
    pub name: String,
    pub service_type: String,
    pub domain: String,
    pub hostname: Option<String>,
    pub address: Option<IpAddr>,
    pub port: Option<u16>,
    pub txt: BTreeMap<String, String>,
    pub last_seen: Instant,
}
```

### ServiceGroup

```rust
pub struct ServiceGroup {
    pub id: ServiceGroupId,
    pub name: String,
    pub service_type: String,
    pub domain: String,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub txt: BTreeMap<String, String>,
    pub instances: Vec<ServiceRecord>,
    pub last_seen: Instant,
}
```

### FilterState

```rust
pub struct FilterState {
    pub text_query: String,
    pub enabled_service_types: BTreeSet<String>,
    pub grouping: GroupingMode,
}

pub enum GroupingMode {
    LogicalService,
    Host,
    ServiceType,
    Port,
    Address,
}
```

All discovered service types are enabled by default. Newly discovered service types should be added to the enabled set unless the user has explicitly disabled them during the current session.

### CommandConfig

```rust
pub struct CommandConfig {
    pub metadata: CommandMetadata,
    pub action: CommandAction,
}

pub struct CommandMetadata {
    pub name: String,
    pub description: Option<String>,
    pub requirements: Vec<String>,
    pub matcher: MatchExpression,
}

pub struct CommandAction {
    pub description: Option<String>,
    pub command: String,
    pub mode: ActionMode,
}
```

### Structured Matcher Config

Command files use structured TOML match blocks. A command matches only when every configured field predicate matches the selected service or service instance.

```toml
[metadata]
name = "ssh"
description = "SSH into a service"
requirements = ["ssh"]

[match.service_type]
equals = "_ssh._tcp"

[match.hostname]
regex = ".*\\.local$"

[action]
description = "SSH into the selected service"
command = "ssh {hostname}"
mode = "execute"
```

If an action template references instance-specific fields such as `{address}` or `{port}` and the selected service group contains multiple instances, the UI should ask the user to choose the exact instance before executing. If the action only references group-level fields such as `{name}`, `{service_type}`, `{domain}`, or `{hostname}`, no instance picker is needed.

The same rule applies when the action matcher itself depends on instance-specific fields. For example, an action that matches `address` or `port` should be resolved against concrete instances rather than only the grouped service row.

When grouping by host, service type, port, or address, a rendered row can represent heterogeneous services. Invoking an action from such a row should first narrow to the concrete service records in that row that match the selected action. If more than one concrete record remains and the action needs instance-specific fields, the UI asks the user to select the exact service instance.

## Filtering and Grouping

The main screen is driven by a view pipeline:

1. Raw discovered `ServiceRecord` values.
2. Free text fuzzy search.
3. Service type inclusion filter.
4. Selected grouping mode.
5. Rendered rows.

Free text search should match at least:

- service name
- service type
- domain
- hostname
- address
- port
- TXT keys and values where practical

Service type filtering is presented as a checklist menu. By default every discovered type is enabled, so the UI starts by showing everything. Users can uncheck types to focus the list, for example showing only `_https._tcp` and `_ssh._tcp` services.

Grouping modes:

- `LogicalService`: default view. Groups duplicate records for the same advertised service while preserving instance addresses.
- `Host`: groups all visible services by hostname, including unresolved/unknown host buckets.
- `ServiceType`: groups visible services by DNS-SD service type.
- `Port`: groups visible services by port, including unknown-port buckets for unresolved records.
- `Address`: groups visible services by resolved IP address, including unknown-address buckets.

Changing filters or grouping should preserve selection where possible by stable row id; otherwise select the nearest visible row.

## Keybindings

Keybindings are loaded from configuration. Defaults should follow Vim conventions where practical:

- `up` / `k`: move selection up.
- `down` / `j`: move selection down.
- `enter`: invoke actions for the selected service.
- `/`: enter filter/search mode.
- `t`: open service type filter menu.
- `g`: open grouping selector.
- `esc`: close modal or leave filter/search mode.
- `q`: quit from the main service list.
- `?`: show help.

The exact keybinding config schema can be finalized during implementation, but it should map modes plus key sequences to app commands, following the reference app's configurable command pattern.

## Milestones

### 1. Project Foundation

- Add missing dependencies intentionally:
  - `crossterm`
  - `tokio-stream`
  - `toml`
  - `directories` or `etcetera`
  - `regex`
  - `shell-words` or equivalent argv parser
  - selected mDNS/zeroconf crate
- Replace `src/main.rs` hello world with CLI parsing and terminal setup.
- Add module skeletons and error handling conventions.
- Add basic `cargo fmt`, `cargo clippy`, and `cargo test` verification path.

Exit criteria:

- App starts and exits cleanly.
- Terminal is restored after quit or error.
- Empty UI renders without service discovery wired in.

### 2. TUI Event Loop

- Port the async event-loop shape from `ref/crates-tui`.
- Add app actions:
  - quit
  - render
  - resize
  - move selection up/down
  - open action picker
  - close modal
  - invoke action
  - enter filter/search mode
  - update free text search
  - open service type filter menu
  - toggle service type visibility
  - open grouping selector
  - change grouping mode
  - select service instance when required
- Render:
  - filtered and grouped service list with row counts
  - active filter/search input
  - service type checklist modal
  - grouping selector modal
  - selected service details
  - instance picker modal
  - status bar
  - empty/loading state
  - unresolved/pending service fields

Exit criteria:

- Keyboard navigation works against a fake in-memory service list.
- Default Vim-style keybindings work, and keybinding lookup is config-driven.
- Free text search and service type filters combine correctly.
- Grouping can be switched at runtime without losing service records.
- Resize and quit behave correctly.

### 3. Service Discovery

- Implement `ServiceDiscovery` trait.
- Add a fake discovery backend for tests and local UI development.
- Implement the real mDNS backend.
- Normalize discovered services into `ServiceRecord`.
- Group records into `ServiceGroup` rows according to the active grouping mode.
- Update app state from discovery events.

Exit criteria:

- Real local services appear in the TUI.
- Multiple records for the same logical service display as one row with an instance count in the default grouping mode.
- Host, service type, port, and address grouping modes produce stable rows from the same raw service data.
- Unresolved services appear in the UI and update as details arrive.
- Fake backend tests cover add/update/remove behavior.

### 4. Plumber Config Loader

- Define TOML schema for command files.
- Implement `MatcherBuilder`.
- Load all `*.toml` files from config directories.
- Validate:
  - required fields
  - valid action mode
  - parseable structured match blocks
  - duplicate names
  - missing external requirements
- Build immutable `Matcher`.

Exit criteria:

- Unit tests cover valid config, invalid config, duplicate commands, and no-match behavior.
- Unit tests cover structured predicates for equality and regex.
- A bundled SSH opener config loads successfully.

### 5. Matching Engine

- Implement structured TOML predicates.
- Match a selected service group, filtered/grouped row, or service instance to zero, one, or many commands.
- Show no-action status when nothing matches.
- Open action picker modal when more than one action matches.
- Open instance picker modal only when the chosen action needs instance-specific fields and the selected group has multiple instances.

Exit criteria:

- Tests cover matching by service type, name, hostname, TXT fields, port, and address.
- UI can show and select matching actions for a fake service.
- UI can invoke actions from non-default grouping modes by narrowing grouped rows back to concrete service records.
- UI only asks for exact instance selection when the chosen action needs it.

### 6. Action Execution

- Interpolate service fields into command templates.
- Execute `fork` actions via child process.
- Execute `execute` actions by restoring the terminal and replacing the process on Unix.
- Report spawn failures in the UI for `fork`.

Exit criteria:

- `fork` can open or run a harmless command and return to the TUI.
- `execute` replaces the TUI process in an integration/manual test.
- Interpolation tests cover missing fields and escaping behavior.

### 7. Configuration Polish

- Implement XDG config lookup:
  - bundled defaults
  - `$XDG_CONFIG_HOME/avahi-tui/commands`
  - fallback `~/.config/avahi-tui/commands`
- Add sample configs:
  - SSH
  - HTTP/HTTPS browser opener
- Add a `--print-config-paths` or `config paths` command if useful for debugging.

Exit criteria:

- Fresh install has useful default actions.
- User-provided TOML files are discovered without editing a central config file.

### 8. Documentation and Release Readiness

- Update README with:
  - install/build instructions
  - command file schema
  - matcher syntax
  - action modes
  - keybindings
  - troubleshooting Avahi/mDNS discovery
- Add examples under an `examples/` or `.config/` directory.
- Add CI once the codebase has meaningful tests.

Exit criteria:

- A new user can build, run, discover services, and add a custom opener from README alone.

## Test Strategy

- Unit tests:
  - config parsing
  - matcher behavior
  - filter behavior
  - grouping behavior
  - interpolation
  - service record normalization
- Integration tests:
  - load multiple command files into one matcher
  - fake discovery events update app state
  - filters and grouping produce expected visible rows
  - command execution planner produces expected argv/mode
- Manual tests:
  - discover `_ssh._tcp`, `_http._tcp`, and `_https._tcp` services on a local network
  - terminal cleanup after quit, panic, and `execute`

## Initial Implementation Order

1. Build the TUI shell with fake services.
2. Build and test the plumber config/matcher module independently.
3. Connect matching results to the UI action picker.
4. Add action execution.
5. Add real service discovery.
6. Replace fake defaults with bundled config files and complete README documentation.

This order keeps the network-discovery uncertainty isolated until the UI and matcher behavior are already working.
