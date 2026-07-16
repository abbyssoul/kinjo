# Release Notes

## Next release

### Breaking CLI change: sample discovery is a backend

The `--fake-discovery` flag has been removed. Build or install Kinjo with the
off-by-default `fake` Cargo feature, then select the sample backend through the
same exhaustive interface as every other discovery backend:

```sh
cargo install kinjo --features fake
kinjo --backend fake
```

For source-tree development, use:

```sh
cargo run --features fake -- --backend fake
```

The old flag was removed rather than retained as a deprecated alias because an
alias would preserve two independent ways to select one backend, including
ambiguous combinations with `--backend`. Kinjo is still pre-1.0; making the
backend model unambiguous now is preferable to carrying that ambiguity into the
stable interface.

Library callers must make the same migration. `DiscoveryConfig::fake` and
`DiscoveryOptions::fake()` have been removed; enable the `fake` Cargo feature
and set `DiscoveryConfig::backend` to `DiscoveryBackend::Fake`. The selected
backend is now the only discovery-mode source of truth for both CLI and library
callers.
