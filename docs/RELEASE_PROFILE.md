# Release Profile

## Goal

Reduce the size of published `dot` binaries without changing dependencies,
runtime features, or development and test builds.

## Design

The workspace-root `Cargo.toml` will define this profile:

```toml
[profile.release]
strip = "symbols"
lto = "fat"
codegen-units = 1
panic = "abort"
opt-level = "z"
```

Cargo applies the root profile to both `dot-core` and `dot-cli`. The settings
favor small published artifacts: symbols are omitted, LLVM can optimize across
the full crate graph, and code generation prioritizes size. Rust panics in a
release binary terminate the process instead of unwinding the stack. Ordinary
`Result`-based error handling remains unchanged, but panic diagnostics and exit
behavior may differ. The other accepted trade-off is a slower release build.

## Verification

Run these commands from the workspace root:

```console
cargo test --workspace --locked --all-targets --all-features
cargo build --workspace --locked --release
./target/release/dot --version
```

Use `.\target\release\dot.exe --version` for the Windows PowerShell smoke check. CI must run
the locked release build and version smoke check on Linux, macOS, and Windows.
The profile is accepted when all existing tests pass, each platform's release
binary starts successfully, and a controlled local comparison confirms that
the configured profile produces a smaller artifact than Cargo's default
release profile. No fixed byte threshold is imposed across compiler versions
or platforms.

On the current source and locked dependency graph, the local macOS artifact was
5,184,608 bytes with Cargo's default release profile and 1,953,168 bytes with
the proposed profile. With every non-LTO setting held constant, the same build
measured 2,966,976 bytes without LTO, 2,868,176 bytes with thin LTO, and
1,953,168 bytes with fat LTO. These measurements justify choosing fat LTO for
release assets.
