# Claude assistant instructions: deckwatch-plugin-sdk

> **Bootstrap status: DONE (2026-11-24).**

This crate holds the shared Rust types and helpers that every deckwatch plugin compiles against, alongside `extism-pdk` and targeting `wasm32-unknown-unknown`. Its types are a **wire contract** with the deckwatch host (alexconrey/deckwatch, `src/plugins.rs`). A wrong field name or a missing `#[serde(default)]` breaks every plugin or every deckwatch install at once, so treat changes here like a public API.

**[`PLAN.md`](PLAN.md)** is the working document: the CI, security and release roadmap with acceptance criteria. It is local-only, listed in `.git/info/exclude`. This file is the durable reference meant to be committed.

## 0. First session: bootstrap from PLAN.md

1. **Read `PLAN.md` in full** before changing anything. If it's missing (a fresh clone), continue from this file and say so.
2. **Verify, don't trust.** Re-check every claim against the code, GitHub (`gh pr list`, `gh release list`, `gh api repos/alexconrey/deckwatch-plugin-sdk/rulesets`) and deckwatch's copy of the plugin types. deckwatch lives at `/Users/alexconrey/git/kore/deckwatch`. Read it with `git show origin/main:src/plugins.rs`; never edit that checkout.
3. **Fold durable knowledge into this file.** That means commands, CI jobs, rules, the release process and the compatibility policy. Replace every `<!-- bootstrap: … -->` marker. Keep the roadmap and its progress in `PLAN.md`.
4. **Finish:** set the status line to `DONE (date)`, and commit `CLAUDE.md` in a PR (feature branch, PR, the maintainer merges).
5. **Every later session:** read this file first. Open `PLAN.md` for roadmap items.

## Keeping documentation current

- **Same-PR rule:** any change to types, behaviour, commands, CI or process updates the docs in the same PR. That means README (usage and field reference), `CHANGELOG.md` (create it with the first release), and **this file** when a rule or command changes.
- **Write down what you learn the hard way.** When a bug or compatibility break costs you time, add a rule below with a one-line note of what happened.
- When a `PLAN.md` item is done, tick it. Remove stale text instead of appending contradictions.
- Before finishing a session, re-read the sections you touched and make sure `CLAUDE.md`, README and `PLAN.md` agree with the code.

## What's here

- **`src/lib.rs`** (~950 lines) is the whole crate: public types, one host function, and unit tests (`mod tests`, ~20 tests).
- **`tests/readme.rs`** checks that the README's type reference lists exactly the JSON keys and variants of every public type (rule 10).
- **`README.md`** is the plugin author's reference: exports, sandbox, every field, and deckwatch's actual host behaviour and known host issues.
- **Dependencies:** `serde` and `serde_json` only. Keep it that way: plugins pay for every dependency in wasm size.
- **`Cargo.lock` is committed** (decided 2026-11-24), so CI is reproducible. CI passes `--locked` everywhere; after changing dependencies, commit the updated lock file in the same PR. Plugins don't use this lock file; they resolve the SDK's dependencies in their own. Keep it at lockfile format v3, which Cargo writes automatically while `rust-version` is below 1.83, so the MSRV toolchain can read it.

### Public API and its deckwatch mirror

deckwatch copies these types by hand in `src/plugins.rs` (line refs at deckwatch `origin/main`, 2026-11-24). Field names are the wire contract; Rust type names may differ.

| SDK item | Purpose | deckwatch `src/plugins.rs` |
|---|---|---|
| `now()` (wasm32 only) | Host function `extism:host/user::now`, Unix seconds. Use it instead of `SystemTime::now()`, which panics in the sandbox | host side, not a type |
| `PluginMetadata` | Plugin self-description: name, version, `provides`/`depends_on` ordering, config schema, provisionable resources, MCP tuning fields | `:300` (**ahead**: `min_deckwatch_version`) |
| `ConfigField`, `ConfigFieldType` | Plugin config schema entries | `:253`, `:236` |
| `McpTuningField` | A per-plugin tuning input shown in deckwatch's MCP settings (key, label, placeholder, default) | `:281` |
| `PluginContext` | Input to `apply()`: namespace, deployment name, annotations, labels, earlier plugins' `plugin_outputs`, `provisioned_resources` | `:36` |
| `PluginResult` | Output of `apply()`: env vars, sidecars, raw Kubernetes resources, outputs for dependent plugins, service account, deployment annotations/labels, errors and warnings | `:77` (**ahead**: `init_containers`) |
| `WantsServiceAccount` | Structured ServiceAccount request inside `PluginResult` | `:68` |
| `EnvVarSpec`, `EnvVarSource`, `SecretKeyRef`, `ConfigMapKeyRef` | Container env vars, with `value` / `secret_key_ref` / `config_map_key_ref` constructors | `:134`; deckwatch names the others `EnvVarSourceSpec` `:143`, `SecretKeyRefSpec` `:151`, `ConfigMapKeyRefSpec` `:157` |
| `SidecarSpec` | Sidecar container to inject | `:187` (**ahead**: `volumes`, `volume_mounts`, `primary_volume_mounts`, `cpu_/memory_` `request`/`limit`) |
| (missing) | Volumes and mounts | deckwatch-only `VolumeSpec` `:165`, `VolumeMountSpec` `:177`; SDK PR #13 |
| `PluginResource` | A resource type a plugin can provision (fields, `singleton`, `output_keys`) | `:336` |
| `ResourceProvisionRequest` / `ResourceProvisionResult` | `provision()` input and output | `:354` / `:364` (**ahead**: `init_containers`) |
| `ResourceDeprovisionRequest` / `ResourceDeprovisionResult` | `deprovision()` input and output | `:399` / `:429` (**behind**: no `delete_kubernetes_resources` yet) |
| `KubernetesObjectRef` | An object in the application's namespace (API version, kind, name) for `deprovision()` to ask deckwatch to delete; constructors `new`, `service_account`, `job` | not mirrored yet (SDK-first, added in v0.10.0) |
| `ProvisioningStatus` | Provisioning state enum, serialized as the variant name (`"Pending"`, not snake_case) on both sides | `:416` |

deckwatch-only host types (`LoadedPlugin`, `ValidationResult`) stay in deckwatch. Re-check drift with a field-level diff of both files whenever either side changes; PLAN Phase 3 turns that into CI.

## Rules that prevent the bugs we've had (and the ones we refuse to have)

1. **Wire compatibility first.** Every new field on a type that crosses the plugin/host boundary is `Option<_>` or has `#[serde(default)]`, so an older host still parses newer plugin output and vice versa. Removing or renaming a field is a breaking change: bump the minor version (0.x) and note it in the CHANGELOG.
2. **deckwatch must not drift from the SDK.** In 2026-11, deckwatch's own copy of these types was *ahead* of SDK `main`. Sidecar volumes, split requests/limits and `min_deckwatch_version` existed only in open SDK PRs #13 and #11, so plugins had to hand-roll a shim (`deckwatch-plugin-otel/src/host.rs`). Land host-side type changes here **first**, release them, then update deckwatch (PLAN Phase 3).
3. **Tests must compile and pass before merging.** In 2026-11, `main`'s tests didn't compile (`E0063 missing field provisioned_resources` at `src/lib.rs:865`) because there was no CI. The CI from PLAN Phase 1 enforces this. Never merge with a red or missing check.
4. **The crate must build for `wasm32-unknown-unknown`.** Lint and build that target in CI, not just the host.
5. **Plugins pin the SDK by tag** (`tag = "vX.Y.Z"`), never `branch = "main"` and preferably not a bare `rev`. That requires real GitHub Releases for every tag (PLAN Phase 2).

6. **Every public type crossing the boundary gets a JSON round-trip test,** plus a fixture of the *old* shape that must still deserialize after a change (PLAN Phase 1.4). Don't add `#[serde(deny_unknown_fields)]`: it breaks forward compatibility.
7. **Code compiled for wasm32 avoids `std` features the sandbox lacks** (clock, filesystem, threads, randomness). `now()` exists because `SystemTime::now()` panics there.
8. **MSRV is 1.82** (`rust-version`). `now()` uses an `unsafe extern "C"` block: 1.81 rejects it, but only on the wasm32 build (host tests pass on 1.81), so the MSRV job must build wasm32. Raising the MSRV is a minor bump.
9. **Keep the doc comment saying "Mirrors `deckwatch::plugins::…`"** on mirrored types, and update the table above when you add or change one.
10. **Documentation is pedantic, and it changes in the same PR as the code.** This is an SDK; plugin authors build against the docs.
    - Every public type, field and variant appears in the README type reference. `tests/readme.rs` enforces this: a new type also needs an entry in that test's `actual()`.
    - State the JSON contract exactly: required, default, or omitted when empty.
    - Describe deckwatch's **actual** behaviour, verified in deckwatch's code, and name the deckwatch version it was checked against. In 2026-11, the README and rustdoc said deprovision `message` was shown in the UI, `ProvisioningStatus` drove re-polling, and `deployment_annotations` were applied on every apply. None of that was true.
    - Where the host contradicts its own types or docs, list it under "Known host issues" rather than documenting the intent.
11. **Compile README examples before publishing them.** Build a scratch plugin crate from the snippets (wasm32 build plus host `cargo test`). That's how we learned `crate-type = ["cdylib", "rlib"]` breaks host tests with a link error, while `["cdylib"]` alone works.

## Commands

The toolchain is pinned in `rust-toolchain.toml` (1.99, same as deckwatch, with clippy, rustfmt and the wasm32 target); rustup picks it up automatically. The MSRV is `rust-version` in `Cargo.toml` (1.82).

CI (`.github/workflows/ci.yml`) runs these. Run them all before opening a PR:

| Command | CI job | What |
|---|---|---|
| `cargo fmt --check` | Check | Formatting (the only command without `--locked`) |
| `cargo clippy --all-targets -- -D warnings` | Check | Host lint, tests included |
| `cargo clippy --target wasm32-unknown-unknown -- -D warnings` | Check | wasm lint (`now()` only compiles here) |
| `cargo test` | Check | Unit tests on the host |
| `cargo build --target wasm32-unknown-unknown` | Check | wasm build |
| `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps` | Check | Docs build without warnings |
| `cargo +1.82 build --target wasm32-unknown-unknown` and `cargo +1.82 test` | MSRV | MSRV check; the job reads the version from `Cargo.toml` |

Not in CI yet:

| Command | What |
|---|---|
| `cargo semver-checks check-release --baseline-rev <last tag>` | API break check (needs `cargo install cargo-semver-checks`; PLAN Phase 1.4) |
| `cargo deny check` | Advisories, licenses, bans, sources (needs `deny.toml`; PLAN Phase 4) |

CI adds `--locked` to every cargo command above; add it locally to reproduce CI exactly. Keep the MSRV job reading `rust-version`, not hard-coding it.

Edition 2021 means Cargo's resolver ignores `rust-version`, so `cargo update` or Dependabot can pull in a dependency that needs a newer rustc than 1.82. The MSRV job (`--locked` on 1.82) catches that. Don't fix it by setting `resolver = "3"`: Cargo 1.82 can't parse it. Hold the dependency back (`cargo update -p <crate> --precise <version>`) or raise the MSRV, which is a minor bump.

## CI and security baseline (from deckwatch, 2026-11-24)

These are the practices that caught or would have caught real breakage in deckwatch. Apply them here (PLAN Phase 1):

- **Triggers:** `pull_request` *and* `push` to `main`. The push run catches PRs that pass separately but break `main` together.
- **Concurrency:** a new push cancels a superseded PR run; runs on `main` are never cancelled.
- **Permissions:** `permissions: contents: read` by default.
- **Toolchain:** pinned in `rust-toolchain.toml` and installed exactly as pinned (`rustup toolchain install`, no arguments). Never `dtolnay/rust-toolchain@stable`. Floating `stable` hid a toolchain break in deckwatch's release build.
- **Required jobs:** fmt, clippy (host all-targets and wasm32) with `-D warnings`, `cargo test --locked`, and the wasm32 build. Here those are the `Check` and `MSRV` jobs. Make them **required status checks** in a `main` ruleset, so red CI blocks merging.
- **Dependabot:** cargo and github-actions, weekly, grouped, with a `cooldown` (3 days, 7 for majors). Any ignore rule gets a narrow version range and a comment saying why.
- **Supply chain:**
  - pin actions by commit SHA;
  - run `cargo deny check` (advisories, bans, licenses, sources) and a scheduled advisory scan;
  - audit build dependencies too (`cargo tree -e normal,build`).
- **API safety:** run `cargo semver-checks` against the last release tag on every PR.

## Releasing

Tags `v0.2.0` through `v0.9.0` exist, but **none has a GitHub Release** and there is no `CHANGELOG.md` or release workflow yet (PLAN Phase 2). Until then, steps 1 and 3 still apply and the maintainer creates the tag and Release by hand.

1. Bump the version in `Cargo.toml` and update `CHANGELOG.md` in one PR.
2. After merge, **the maintainer tags `vX.Y.Z`**. A release workflow creates the **GitHub Release** from the tag, with notes and a CHANGELOG excerpt, after checking that the tag matches `Cargo.toml`.
3. Tell downstream (deckwatch and each plugin) to move to the new tag. Track the plugins' pins in PLAN Phase 4. Find them with `grep deckwatch-plugin-sdk ~/git/github.com/alexconrey/deckwatch-plugin-*/Cargo.toml`.

**Versioning:** semver within 0.x. Removing or renaming a public item or wire field, or raising the MSRV, bumps the minor version; additive, defaulted fields bump the patch or minor version.

Agents never create tags or releases unless the maintainer explicitly asks.

## Git workflow and safety

- **Never** push or commit directly to `main`. All changes go through a feature branch and a PR.
- **Never** merge without passing CI. The maintainer merges.
- **No `Co-Authored-By` or Claude/AI attribution** in commits or PRs.
- **No force-pushes, tags or releases** without explicit maintainer instruction.
- Work in your own `git worktree` under `/tmp`. **Never `git stash`**: the stash is shared across sessions.
