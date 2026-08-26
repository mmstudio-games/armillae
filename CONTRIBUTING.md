# Contributing to Armillae

Thank you for contributing to Armillae.

## Design and scope

Read [.agents/DESIGN.md](.agents/DESIGN.md) before proposing or implementing a change. It routes
changes to an active specification or an accepted RFC. LLM Bridge work is governed by
[.agents/specs/llm-bridge.md](.agents/specs/llm-bridge.md). Simulation work is governed by the
[Simulate Active Spec](.agents/specs/simulate.md) and
[accepted RFC 0002](.agents/rfcs/0002-simulate.md). Agentic runtime discovery remains in
[RFC 0001](.agents/rfcs/0001-agentic-runtime.md). [.agents/TODO.md](.agents/TODO.md) is the
project-wide checklist index; detailed implementation differences live under `.agents/todos/`.
The `docs/` directory is reserved for stable user-facing installation, concepts, guides, and API
documentation. Do not create or maintain standalone user guides until the public interfaces are
frozen and work on a stable release is explicitly approved.

When a change introduces an architectural decision, public protocol change, Provider compatibility
policy, security boundary, dependency choice, or scope change, update files in this order:

1. `.agents/DESIGN.md` for cross-layer changes;
2. a draft RFC for unresolved decisions, followed by any affected active specification after the
   RFC is accepted;
3. the corresponding `.agents/todos/*.md` implementation checklist;
4. code, configuration, tests, examples, and affected user documentation.

The Agentic runtime remains in discovery, and state/persistence work is paused. Simulate's public
API and protocol are frozen in its active specification, but product crates remain blocked on the
Bevy P0 spike in section 16. Do not infer Hosted loaders, persistence schemas, Agent harnesses,
automatic Tool loops,
memory, or scheduling policy from the Simulate contract. Keep Rig types inside
`armillae-llm-rig`.

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

Semifold manages changesets, versions, and the current alpha release channels. Do not bump versions,
change a release channel, publish to a registry, or create a release unless that action is explicitly
authorized. Beta requires the repository's documented scope, compatibility, Live, downstream, and
publish-readiness gates; stable is not promoted directly from alpha. A publish-readiness change
should run `cargo publish --dry-run` separately for every affected crate.

## License

By submitting a contribution, you agree that it is licensed under the repository's
[GNU Affero General Public License v3.0 only](LICENSE), SPDX identifier `AGPL-3.0-only`.
