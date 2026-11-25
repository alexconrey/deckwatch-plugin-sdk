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

- **Same-PR rule:** any change to types, behaviour, commands, CI or process updates the docs in the same PR. That means README (usage and field reference), `CHANGELOG.md` (a line under the upcoming version's section, in Keep a Changelog categories, marking anything breaking), and **this file** when a rule or command changes.
- **Write down what you learn the hard way.** When a bug or compatibility break costs you time, add a rule below with a one-line note of what happened.
- When a `PLAN.md` item is done, tick it. Remove stale text instead of appending contradictions.
- Before finishing a session, re-read the sections you touched and make sure `CLAUDE.md`, README and `PLAN.md` agree with the code.

## What's here

- **`src/lib.rs`** (~2,150 lines, about a third of it tests) holds the wire types and their helpers, one host function, and unit tests (`mod tests`, ~45 tests). Keep every public `struct` and `enum` here: `tests/readme.rs` only scans this file.
- **`src/telemetry.rs`** is the `telemetry` module (0.10.0): names shared with the host (span attributes, host-reserved event codes, the `traceparent` header) and the `traceparent` check behind `trace_headers()`. Constants only, no types.
- **`tests/wire_compat.rs`** checks wire compatibility with the v0.9.0 and v0.3.0 releases in both directions (rule 6). The old releases are dev-dependencies (`v0_9`, `v0_3`), so plugins never see them.
- **`tests/readme.rs`** checks that the README's type reference lists exactly the JSON keys and variants of every public type (rule 10).
- **`README.md`** is the plugin author's reference: exports, sandbox, every field, and deckwatch's actual host behaviour and known host issues.
- **Dependencies:** `serde` and `serde_json` only. Keep it that way: plugins pay for every dependency in wasm size.
- **`Cargo.lock` is committed** (decided 2026-11-24), so CI is reproducible. CI passes `--locked` everywhere; after changing dependencies, commit the updated lock file in the same PR. Plugins don't use this lock file; they resolve the SDK's dependencies in their own. Keep it at lockfile format v3, which Cargo writes automatically while `rust-version` is below 1.83, so the MSRV toolchain can read it.

### Public API and its deckwatch mirror

deckwatch copies these types by hand in `src/plugins.rs`. Line refs are at deckwatch `604a36f` (2026-11-25; version 0.7.0, CHANGELOG dated 2026-11-25, not yet tagged). Field names are the wire contract; Rust type names may differ.

**As of `604a36f`, every SDK type matches deckwatch's copy field for field.** In 2026-11, deckwatch was first ahead of the SDK (rule 2), then behind it after v0.10.0, until its plugin-events work caught up. Re-check after any change on either side: diff the `pub` fields of both files per type. PLAN Phase 3 and alexconrey/deckwatch#288 (depend on the crate) would make this check unnecessary.

| SDK item | Purpose | deckwatch |
|---|---|---|
| `now()` (wasm32 only) | Host function `extism:host/user::now`, Unix seconds. Use it instead of `SystemTime::now()`, which panics in the sandbox | `host_now_fn`, `src/plugins.rs:868` |
| `PluginMetadata` | Plugin self-description: name, version, `provides`/`depends_on` ordering, config schema, provisionable resources, MCP tuning fields, `min_deckwatch_version` | `:423` |
| `ConfigField`, `ConfigFieldType` | Plugin config schema entries; `KeyValue` and the `Unknown` catch-all | `:376`, `:310` |
| `parse_key_value_field` | Parses a `key_value` config value (JSON object of strings; empty means none) | deckwatch validates with its own `validate_key_value_field` (`:322`) |
| `McpTuningField` | A per-plugin tuning input shown in deckwatch's MCP settings (key, label, placeholder, default) | `:404` |
| `PluginContext` | Input to `apply()` (also on deckwatch's `validate_plugin` dry run): namespace, deployment name, annotations, labels, earlier plugins' `plugin_outputs`, `provisioned_resources`, `trace_context` with `trace_headers()` | `:43`. deckwatch 0.7.0 fills `trace_context` while it exports OTLP traces |
| `PluginResult` | Output of `apply()`: env vars, sidecars, init containers, raw Kubernetes resources, outputs for dependent plugins, service account, deployment annotations/labels, errors, warnings, `events` with the event helpers | `:91` |
| `WantsServiceAccount` | Structured ServiceAccount request inside `PluginResult` | `:82` |
| `EnvVarSpec`, `EnvVarSource`, `SecretKeyRef`, `ConfigMapKeyRef` | Container env vars, with `value` / `secret_key_ref` / `config_map_key_ref` constructors | `:208`; deckwatch names the others `EnvVarSourceSpec` `:217`, `SecretKeyRefSpec` `:225`, `ConfigMapKeyRefSpec` `:231` |
| `SidecarSpec` | Sidecar or init container to inject; volumes, mounts, split requests/limits | `:261` |
| `VolumeSpec`, `VolumeMountSpec` | Pod volumes and container mounts for sidecars | `:239`, `:251` |
| `PluginResource` | A resource type a plugin can provision (fields, `singleton`, `output_keys`) | `:459` |
| `ResourceProvisionRequest` / `ResourceProvisionResult` | `provision()` input and output, with `trace_context` / `events` | `:477` / `:490` |
| `ResourceDeprovisionRequest` / `ResourceDeprovisionResult` | `deprovision()` input and output, with `trace_context` / `delete_kubernetes_resources` / `events` | `:528` / `:561` |
| `KubernetesObjectRef` | An object in the application's namespace (API version, kind, name), for `deprovision()` to ask deckwatch to delete or for a `PluginEvent` to point at; constructors `new`, `service_account`, `job` | `:589` |
| `PluginEvent`, `EventSeverity` | Structured events in the three results' `events`. `info`/`warn`/`error`/`push_event` on each result also copy an event into `errors`, `warnings` or (deprovision) `message`, as `PluginEvent::legacy_text()` (rule 14). `EventSeverity::Unknown` is the catch-all (rule 13) | `:175`, `:156` |
| `telemetry` | Span attribute names (`deckwatch.plugin.name`/`.version`/`.export`), the host-reserved event codes (`plugin.*`), `TRACEPARENT_HEADER` | deckwatch defines its ten codes in `src/plugin_events.rs` (`:57`–`:87`), all listed by the SDK, and uses the attribute names in `src/plugins.rs` (`:2025`–`:2027`) |
| `ProvisioningStatus` | Provisioning state enum, serialized as the variant name (`"Pending"`, not snake_case) on both sides | `:548` |

deckwatch-only host types (`LoadedPlugin`, `ValidationResult`, `PluginCall`, `Trigger`, …) stay in deckwatch.

## Rules that prevent the bugs we've had (and the ones we refuse to have)

1. **Wire compatibility first.** Every new field on a type that crosses the plugin/host boundary is `Option<_>` or has `#[serde(default)]`, so an older host still parses newer plugin output and vice versa. Removing or renaming a field is a breaking change: bump the minor version (0.x) and note it in the CHANGELOG.
2. **deckwatch must not drift from the SDK.** In 2026-11, deckwatch's own copy of these types was *ahead* of SDK `main`. Sidecar volumes, split requests/limits and `min_deckwatch_version` existed only in open SDK PRs #13 and #11, so plugins had to hand-roll a shim (`deckwatch-plugin-otel/src/host.rs`). Land host-side type changes here **first**, release them, then update deckwatch (PLAN Phase 3).
3. **Tests must compile and pass before merging.** In 2026-11, `main`'s tests didn't compile (`E0063 missing field provisioned_resources` at `src/lib.rs:865`) because there was no CI. The CI from PLAN Phase 1 enforces this. Never merge with a red or missing check.
4. **The crate must build for `wasm32-unknown-unknown`.** Lint and build that target in CI, not just the host.
5. **Plugins pin the SDK by tag** (`tag = "vX.Y.Z"`), never `branch = "main"` and preferably not a bare `rev`. That requires real GitHub Releases for every tag (PLAN Phase 2).

6. **Wire compatibility is tested against real releases.** `tests/wire_compat.rs` pulls v0.9.0 (last release) and v0.3.0 (oldest pinned by a plugin) in as dev-dependencies and checks both directions: everything an old version sends survives in the new types, and an old reader gets exactly what it knows from new output. When you add a field, populate it in the test's `new` builders (they list every field, so the test won't compile until you do). After a release, add it as a baseline. The one accepted break, `ConfigFieldType::KeyValue` on v0.9.0 readers, is pinned by `key_value_field_type_is_rejected`. Don't add `#[serde(deny_unknown_fields)]`: it breaks forward compatibility.
7. **Code compiled for wasm32 avoids `std` features the sandbox lacks** (clock, filesystem, threads, randomness). `now()` exists because `SystemTime::now()` panics there.
8. **MSRV is 1.82** (`rust-version`). `now()` uses an `unsafe extern "C"` block: 1.81 rejects it, but only on the wasm32 build (host tests pass on 1.81), so the MSRV job must build wasm32. Raising the MSRV is a minor bump.
9. **Keep the doc comment saying "Mirrors `deckwatch::plugins::…`"** on mirrored types, and update the table above when you add or change one.
10. **Documentation is pedantic, and it changes in the same PR as the code.** This is an SDK; plugin authors build against the docs.
    - Every public type, field and variant appears in the README type reference. `tests/readme.rs` enforces this: a new type also needs an entry in that test's `actual()`. A new field with `skip_serializing_if` must also be **populated** in `actual()`, or it never appears in the JSON and the test can't see it.
    - State the JSON contract exactly: required, default, or omitted when empty.
    - Describe deckwatch's **actual** behaviour, verified in deckwatch's code, and name the deckwatch **commit** it was checked against. deckwatch doesn't bump its `Cargo.toml` version between releases, so in 2026-11 the README said "deckwatch 0.6.7" for behaviour from unreleased `main`, while the real v0.6.7 release was 100 commits older and still leaked deckwatch's encryption key to plugins. When release and `main` differ in a way plugins see, list the differences. In 2026-11, the README and rustdoc said deprovision `message` was shown in the UI, `ProvisioningStatus` drove re-polling, `deployment_annotations` were applied on every apply, and `extism_pdk::log!` couldn't reach deckwatch's logs. None of that was true. The last one was a claim about extism, so check those against extism's source (`~/.cargo/registry/src/*/extism-<version>/src/pdk.rs`) for the version deckwatch's `Cargo.lock` pins.
    - Where the host contradicts its own types or docs, list it under "Known host issues" rather than documenting the intent.
    - **Re-check before every release.** deckwatch `main` keeps moving. On 2026-11-25, 43 commits after the checked commit (`58ff03c`) had changed six plugin-facing behaviours: refused inherited variables, `validate_plugin`'s sandbox, re-entering secrets, the field sweep, URL masking in events, and MCP schema checks. Before a release PR, re-check the host-behaviour docs against deckwatch `origin/main` (or its newest tag) and re-pin the commit everywhere it's cited, CLAUDE.md's line refs included. Also check the UI labels you cite in deckwatch's `frontend/`. The docs once named a **Refresh** button that doesn't exist.
11. **Compile README examples before publishing them.** Build a scratch plugin crate from the snippets (wasm32 build plus host `cargo test`). That's how we learned that `crate-type = ["cdylib", "rlib"]` breaks host tests with a link error while `["cdylib"]` alone works. The break only happens when the `#[plugin_fn]` exports are compiled for the host too; plugins that gate them with `#[cfg(target_arch = "wasm32")]` link fine. Test claims like this both ways before writing them down.
12. **Every public struct derives `Default`, and every public enum is `#[non_exhaustive]`** (since 0.10.0). Adding an enum variant is then non-breaking. Adding a struct field still counts as breaking under Rust semver (`cargo semver-checks` flags it as `constructible_struct_adds_field`, because full struct literals stop compiling), so it needs a minor bump; `Default` just means plugins that use `..Default::default()` aren't affected. Don't make structs `#[non_exhaustive]`: plugins couldn't build them at all. A new struct without `Default`, or a new enum without `#[non_exhaustive]`, brings back avoidable breakage. Add new types to the round-trip test `every_struct_default_round_trips` too.
13. **An enum that plugins send to the host needs a `#[serde(other)] Unknown` catch-all** (`ConfigFieldType`, and `EventSeverity` from the start). Without one, an older host that meets a newer variant fails to parse the *whole* value it's in, not just that field: one new `field_type` used to cost a plugin every config field it declared, and one new severity would cost the whole `apply()` result. `#[non_exhaustive]` is the source half of this rule; `#[serde(other)]` is the wire half. Each `Unknown` has a documented host meaning (`EventSeverity::Unknown` is treated as `Warning`; `ConfigFieldType::Unknown` renders as a text input), serializes as `"unknown"` (lossy), and plugins must not return it. `ProvisioningStatus` has no catch-all, so a new variant there breaks every existing host: add `Unknown` in one release, and send the new variant only once hosts have it. For the same reason, a struct that appears in lists inside plugin output gets `#[serde(default)]` on the container (`PluginEvent`), so one partial entry can't sink the result.
14. **The event helpers' legacy copies are a contract.** A host that reads `events` drops each `errors`/`warnings` entry equal to an event's `PluginEvent::legacy_text()` (`"<code>: <message>"`, or the message when `code` is empty), and the copies at the end of a deprovision `message` (joined with `"; "`), instead of reporting each problem twice. Changing that format, the separator or the severity-to-field mapping is a wire change. Info events have no legacy field on `PluginResult` and `ResourceProvisionResult`, so older hosts never see them.

## Commands

The toolchain is pinned in `rust-toolchain.toml` (1.99, same as deckwatch, with clippy, rustfmt and the wasm32 target); rustup picks it up automatically. The MSRV is `rust-version` in `Cargo.toml` (1.82).

CI (`.github/workflows/ci.yml`) runs these. Run them all before opening a PR:

| Command | CI job | What |
|---|---|---|
| `cargo fmt --check` | Check | Formatting (the only command without `--locked`) |
| `cargo clippy --all-targets -- -D warnings` | Check | Host lint, tests included |
| `cargo clippy --target wasm32-unknown-unknown -- -D warnings` | Check | wasm lint (`now()` only compiles here) |
| `cargo test` | Check | Unit tests, `tests/readme.rs` (README drift) and `tests/wire_compat.rs` (old releases) on the host |
| `cargo build --target wasm32-unknown-unknown` | Check | wasm build |
| `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps` | Check | Docs build without warnings |
| `cargo +1.82 build --target wasm32-unknown-unknown` and `cargo +1.82 test` | MSRV | MSRV check; the job reads the version from `Cargo.toml` |
| `cargo semver-checks check-release --baseline-rev "$(git describe --tags --abbrev=0 --match 'v*')"` | Semver | Public API breaks must match the `Cargo.toml` bump (0.x: breaking needs a minor bump). Install with `cargo install cargo-semver-checks --locked`; CI pins 0.51.0 |

Not in CI yet:

| Command | What |
|---|---|
| `cargo deny check` | Advisories, licenses, bans, sources (needs `deny.toml`; PLAN Phase 4) |

CI adds `--locked` to every cargo command above; add it locally to reproduce CI exactly. Don't pipe a check through `tail` or `grep` without `set -o pipefail`. On 2026-11-25, `cargo doc … | tail -1` hid a failing doc build (an unclosed `<label>` tag in rustdoc), and the commit was pushed. Keep the MSRV job reading `rust-version`, not hard-coding it.

Edition 2021 means Cargo's resolver ignores `rust-version`, so `cargo update` or Dependabot can pull in a dependency that needs a newer rustc than 1.82. The MSRV job (`--locked` on 1.82) catches that. Don't fix it by setting `resolver = "3"`: Cargo 1.82 can't parse it. Hold the dependency back (`cargo update -p <crate> --precise <version>`, plus a narrow `ignore` in `.github/dependabot.yml` so Dependabot doesn't re-propose it) or raise the MSRV, which is a minor bump.

## CI and security baseline (from deckwatch, 2026-11-24)

These are the practices that caught or would have caught real breakage in deckwatch. Apply them here (PLAN Phase 1):

- **Triggers:** `pull_request` *and* `push` to `main`. The push run catches PRs that pass separately but break `main` together.
- **Concurrency:** a new push cancels a superseded PR run; runs on `main` are never cancelled.
- **Permissions:** `permissions: contents: read` by default.
- **Toolchain:** pinned in `rust-toolchain.toml` and installed exactly as pinned (`rustup toolchain install`, no arguments). Never `dtolnay/rust-toolchain@stable`. Floating `stable` hid a toolchain break in deckwatch's release build.
- **Required jobs:** fmt, clippy (host all-targets and wasm32) with `-D warnings`, `cargo test --locked`, and the wasm32 build. Here those are the `Check` and `MSRV` jobs. Make them **required status checks** in a `main` ruleset, so red CI blocks merging.
- **Dependabot** (`.github/dependabot.yml`, since 2026-11-24): cargo and github-actions, weekly (Monday 06:00 UTC), minor and patch grouped, prefixes `chore(deps)` and `chore(ci)`. Cooldown is 3 days, 7 for cargo majors; github-actions supports only `default-days`. Dependabot alerts and security updates are enabled in the repository settings; security updates skip the schedule and cooldown. Any `ignore` rule gets a narrow version range and a comment saying why. The one exception is `deckwatch-plugin-sdk`, the test baselines from rule 6, which are pinned forever. `rust-toolchain` is deliberately not tracked: bump it by hand, together with deckwatch. Validate config changes against SchemaStore's `dependabot-2.0.json`; GitHub only reports errors after merge.
- **Supply chain:**
  - pin actions by commit SHA;
  - run `cargo deny check` (advisories, bans, licenses, sources) and a scheduled advisory scan;
  - audit build dependencies too (`cargo tree -e normal,build`).
- **API safety:** run `cargo semver-checks` against the last release tag on every PR.

## Releasing

`.github/workflows/release.yml` runs when a `v*` tag is pushed:
1. **Verify tag** (`.github/scripts/release-notes.sh`): the tag is `v<version>`, `<version>` equals `Cargo.toml`, and `CHANGELOG.md` has a dated `## [<version>] - YYYY-MM-DD` section with content. `Unreleased` is refused on purpose.
2. **CI**: the whole of `ci.yml`, called with `workflow_call`, on the tagged commit.
3. **Create GitHub Release**: titled with the tag, notes from the CHANGELOG section plus a pinning snippet. This job alone has `contents: write`.

Run `.github/scripts/release-notes.sh vX.Y.Z` locally to preview the notes or the refusal.

Steps:
1. **Release PR:** collect everything planned for the release on `main` first (the maintainer prefers fewer, fuller minor releases). Re-check the host-behaviour docs against deckwatch first (rule 10). Then, in one PR, set `Cargo.toml` (and `Cargo.lock`, via any cargo command) to the version, and change the CHANGELOG heading from `## [X.Y.Z] - Unreleased` to today's date.
2. After merge, **the maintainer tags `vX.Y.Z` on `main` and pushes the tag**. The workflow does the rest.
3. Start the next section: `## [X.Y+1.0] - Unreleased` (or a patch version), and bump `Cargo.toml` when the first breaking change lands. Once `vX.Y.Z` exists, the Semver job fails any breaking change until `Cargo.toml` is bumped past it.
4. Move downstream (deckwatch and each plugin) to the new tag. **List the plugins from GitHub, not from local checkouts:** `gh repo list alexconrey --limit 200 --json name --jq '.[].name' | grep '^deckwatch-plugin-' | grep -v sdk`. Read each pin from the repo's `main` (`gh api repos/alexconrey/<repo>/contents/Cargo.toml --jq .content | base64 -d`). In 2026-11 the local checkouts listed two deleted repos (rds, s3) and missed five live ones (postgresql, fluent-bit, nginx-proxy, memcached, redis), and several were on feature branches whose pins differed from `main`. Build each plugin's `main` against the release first: v0.10.0 broke all nine with `E0063`. Track the pins in PLAN Phase 4.
5. Consider adding the release as a baseline in `tests/wire_compat.rs`.

History: tags `v0.2.0` to `v0.9.0` predate the workflow. `v0.8.1`, `v0.8.2` and `v0.9.0` got GitHub Releases afterwards (2026-11-24, by hand, notes from the CHANGELOG, marked as created retroactively); older tags have none, and the CHANGELOG backfills their notes. `v0.5.1`, `v0.7.0` and `v0.8.0` were cut with a stale `Cargo.toml` version (0.5.0, 0.6.0, 0.6.0), which is why the workflow now checks it.

**Versioning:** semver within 0.x. A minor bump for anything breaking: removing or renaming a public item or wire field, adding a public struct field (see rule 12), changing a type, or raising the MSRV. A patch bump for fixes and for additions semver-checks accepts (e.g. a new enum variant or a new type).

Agents never create tags or releases unless the maintainer explicitly asks.

## Git workflow and safety

- **Never** push or commit directly to `main`. All changes go through a feature branch and a PR.
- **Never** merge without passing CI. The maintainer merges.
- **No `Co-Authored-By` or Claude/AI attribution** in commits or PRs.
- **No force-pushes, tags or releases** without explicit maintainer instruction.
- Work in your own `git worktree` under `/tmp`. **Never `git stash`**: the stash is shared across sessions.
