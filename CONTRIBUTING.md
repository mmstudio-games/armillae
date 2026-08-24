# Contributing to Armillae

Thank you for contributing to Armillae.

## Design and scope

Read [docs/DESIGN.md](docs/DESIGN.md) before proposing or implementing a change. It routes changes
to the authoritative subsystem design. LLM Bridge work is governed by
[docs/LLM_BRIDGE.md](docs/LLM_BRIDGE.md); Agentic runtime discovery is governed by
[docs/AGENTIC_RUNTIME.md](docs/AGENTIC_RUNTIME.md). [TODO.md](TODO.md) is the project-wide checklist
index; detailed implementation differences live in subsystem files under `todos/`.

When a change introduces an architectural decision, public protocol change, Provider compatibility
policy, security boundary, dependency choice, or scope change, update files in this order:

1. `docs/DESIGN.md` for cross-layer changes;
2. the affected subsystem design;
3. the corresponding `todos/*.md` implementation checklist;
4. code, configuration, tests, and examples.

The Agentic runtime is in discovery. Do not create runtime crates, APIs, persistence schemas,
automatic Tool loops, memory, or scheduling behavior before its scenarios and contracts are
confirmed. Keep Rig types inside `armillae-llm-rig`.

## Rust changes

- Production code must not use panic-capable `unwrap()`.
- Recoverable input, I/O, configuration, secret, network, and Provider failures must remain
  structured errors.
- Use Cargo commands to add or remove dependencies and workspace packages; inspect both manifests
  and the lockfile after each operation.
- Never expose secrets, authorization headers, complete message bodies, Tool arguments, Tool
  results, or unsanitized Provider responses in logs, errors, fixtures, or snapshots.
- Keep live Provider tests ignored by default and never commit real credentials.

## Validation

Run checks proportional to the change. Before submitting a complete code change, run the full
offline quality gate when practical:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo test --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

Protocol changes require serialization round trips, order and ToolCall ID preservation,
forward-compatibility coverage, and relevant schema snapshots. Adapter changes should reuse the
shared Bridge and Streaming contracts.

## Releases

Semifold manages changesets, versions, and alpha release channels. Do not bump versions, publish to
a registry, or create a release unless that action is explicitly authorized. A publish-readiness
change should run `cargo publish --dry-run` separately for every affected crate.

## License

By submitting a contribution, you agree that it is licensed under the repository's
[GNU Affero General Public License v3.0 only](LICENSE), SPDX identifier `AGPL-3.0-only`.
