# Changelog

All notable changes to this crate. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate follows semantic versioning within 0.x: a minor bump (0.9 → 0.10) may break plugins, and a patch bump doesn't.

"Wire" means the JSON exchanged with deckwatch; "source" means Rust code that uses these types. Plugins pin the SDK by tag (`tag = "vX.Y.Z"`), so each section below corresponds to a git tag.

## [0.10.1] - Unreleased

### Documentation

- **Host behaviour now documented for deckwatch 0.7.0** (`main` at `58ff03c`, not yet tagged), which reads plugin `events`, deletes `delete_kubernetes_resources` objects, returns the deprovision `message`, fills `trace_context` while exporting OTLP traces, and releases an application's provisioned fields when its last resource goes:
  - README: the sandbox's logging channels, the events and tracing section, the type reference, "Host behaviour" (with a v0.6.7 comparison and a new "Status, Kubernetes Events and notifications" section), and "Known host issues" (two new: deckwatch#301 and deckwatch#302; deckwatch#250 is fixed);
  - rustdoc in `src/lib.rs` and `src/telemetry.rs`;
  - CLAUDE.md's type table: every SDK type now matches deckwatch's copy field for field.
- Corrected two older claims: `PluginMetadata.name` isn't used for logs or the UI (only the MCP tuning block), and `wants_service_account` results aren't shown in the UI or audit log.
- README, "The sandbox": a request that extism can't make traps the whole export call, and the plugin can't catch it. That covers a host not in `allowed_hosts`, a bad URL, DNS or connection failures, and timeouts. Also what deckwatch does with each export when that happens. Found while building deckwatch-plugin-example's webhook.

## [0.10.0] - 2026-11-24

0.10.0 catches the SDK up with deckwatch's own copy of these types: sidecar volumes and mounts, split resource requests and limits, init containers, and `min_deckwatch_version`. It also adds structured plugin events and W3C trace context, and a `key_value` config field type. Future additions break plugins less: every struct derives `Default`, and enums are `#[non_exhaustive]`. Upgrading is a mechanical change in plugin code (below). It is wire-compatible with older hosts and plugins, except for `key_value`.

### Breaking (source)

Upgrading from 0.9 needs, at most, these changes in plugin code. All nine live deckwatch plugins needed them, in test code as well as the plugin itself: tests that build inputs (`PluginContext`, the request types) break too. The compiler reports `E0063` (missing fields) for each struct literal, and adding `..Default::default()` fixes it.

- **`match` on `ConfigFieldType` or `ProvisioningStatus` needs a `_` arm.** Both are now `#[non_exhaustive]`, and `ConfigFieldType` gains two variants. In exchange, future variants won't break plugins.
- **Struct literals that name every field** of `PluginMetadata`, `PluginContext`, `PluginResult`, `ResourceProvisionRequest`, `ResourceProvisionResult`, `ResourceDeprovisionRequest`, `ResourceDeprovisionResult` or `SidecarSpec` need the new fields, or switch to `..Default::default()`. Every struct now derives `Default`.
- `ConfigFieldType::Select`'s discriminant changed from 3 to 4 (it only matters if you cast the enum to an integer).

### Breaking (wire)

- **`ConfigFieldType::KeyValue` (`"key_value"`) can't be read by hosts built on SDK 0.9 or earlier, or by deckwatch releases up to v0.6.7.** They reject the whole `PluginMetadata`, so the plugin loses all its config fields and resources. Don't use `key_value` until your deckwatch release supports it (deckwatch `main` since `278de71`). Every other change in this release is wire-compatible in both directions with 0.9.0 and 0.3.0; `tests/wire_compat.rs` checks this.

### Added

- `McpTuningField` and `PluginMetadata.mcp_tuning_fields`: operator-tunable lines of MCP guidance (#12).
- `PluginMetadata.min_deckwatch_version`: minimum deckwatch version; older hosts skip the plugin (#11).
- `ResourceDeprovisionResult.delete_kubernetes_resources` and `KubernetesObjectRef` (with `new`, `service_account` and `job` constructors): let `deprovision()` ask deckwatch to delete a ServiceAccount or Job (#14). No deckwatch version supports this yet; hosts ignore it.
- `ConfigFieldType::KeyValue` and `parse_key_value_field()`: a key/value-pairs config field, stored as a JSON object of strings (#16).
- `ConfigFieldType::Unknown`: any unrecognised `field_type` deserializes to it instead of failing (#16).
- `SidecarSpec`: `cpu_request`, `cpu_limit`, `memory_request`, `memory_limit`, `volumes`, `volume_mounts`, `primary_volume_mounts`; new types `VolumeSpec` and `VolumeMountSpec` (#13).
- `PluginResult.init_containers` and `ResourceProvisionResult.init_containers` (#13).
- `Default` on every public struct; `PartialEq` and `Eq` on `ProvisioningStatus` (#20).
- `PluginEvent` and `EventSeverity` (`info`, `warning`, `error`, plus an `Unknown` catch-all that hosts treat as `warning`): structured events with a stable `code` (`"<plugin-name>.<code>"`), a `message`, and optional `resource_id`, `object`, `retryable` and `hint`. A partial event still parses. New `events` field, omitted when empty, on `PluginResult`, `ResourceProvisionResult` and `ResourceDeprovisionResult` (#25).
- Event helpers `info`, `warn`, `error` and `push_event` on those three results. Each also copies the event (`PluginEvent::legacy_text()`, `"<code>: <message>"`) into `errors`, `warnings` or, on deprovision, `message`, so hosts that don't read `events` still log it. No deckwatch version reads `events` yet (#25).
- `trace_context`, a W3C `traceparent`, on `PluginContext`, `ResourceProvisionRequest` and `ResourceDeprovisionRequest` (omitted when `None`), and `trace_headers()` on each: the `traceparent` header for a plugin's outbound HTTP calls, when the value is valid. No deckwatch version sends it yet (#25).
- `telemetry` module: the span attribute names `deckwatch.plugin.name`, `deckwatch.plugin.version` and `deckwatch.plugin.export`, the `traceparent` header name, and the event codes reserved for the host (`plugin.unspecified`, `plugin.fetch_failed`, `plugin.checksum_mismatch`, `plugin.load_failed`, `plugin.trap`, `plugin.timeout`, `plugin.output_invalid`, `plugin.k8s_apply_failed`, `plugin.policy_rejected`) (#25).

### Changed

- `SidecarSpec.port`, `cpu` and `memory` are omitted from the JSON when `None` (previously `null`). Hosts accept both.
- `SidecarSpec.cpu` and `memory` are deprecated in favour of the split request/limit fields. They still set both values where the split fields are unset.
- `ConfigFieldType` and `ProvisioningStatus` are `#[non_exhaustive]` (#20).
- The minimum supported Rust version is declared: 1.82 (`rust-version`). The wasm32 build has needed it since 0.7.0 (`now()` uses an `unsafe extern` block); it is now stated and tested.

### Fixed

- The crate's own tests compile again (`E0063`, missing `provisioned_resources`).
- Rustdoc claims that deckwatch's code contradicts, about `ProvisioningStatus`, deprovision `message`, `deployment_annotations`/`deployment_labels`, MCP tuning, `required`, `provides`, `depends_on` and `retry_after_secs` (#18).
- The docs no longer say `extism_pdk::log!` can't reach deckwatch's logs. extism 1.x turns it into `tracing` events (target `extism::pdk`), which deckwatch keeps by default since deckwatch#256 (#24).

### Documentation

- README rewritten against the code and deckwatch's host behaviour (#18, #19): every type and field with its JSON contract, the plugin exports and sandbox, verified examples, host behaviour as of deckwatch `fc03ec2`, differences in the deckwatch v0.6.7 release, and known host issues (alexconrey/deckwatch#244–#251).
- README "Events and tracing": reporting events, forwarding the trace, the `telemetry` constants, and the `PluginEvent` and `EventSeverity` type reference (#25).
- README: the `crate-type` advice now says `"rlib"` breaks host tests only when the `#[plugin_fn]` exports aren't limited to wasm32 (#24).

### Build and CI

- CI on every PR and on `main`: fmt, clippy (host and wasm32), tests, wasm32 build, rustdoc, the MSRV job, and `cargo semver-checks` against the last release (#17, #21).
- `rust-toolchain.toml` pins Rust 1.99; `Cargo.lock` is committed and CI builds with `--locked` (#17).
- Tests that keep the README type reference in sync with the types (#18) and check wire compatibility with the 0.9.0 and 0.3.0 releases (#21, extended for events and trace context in #25).
- A release workflow creates the GitHub Release from a `v*` tag, after checking the tag against `Cargo.toml` and this file (#22).
- Dependabot: weekly grouped version updates for cargo and GitHub Actions, with a cooldown; Dependabot alerts and security updates enabled (#23).

## [0.9.0] - 2026-08-14

### Added

- `ProvisioningStatus` (`Pending`, `Available`, `Failed`), with `ResourceProvisionResult.status` and `retry_after_secs` (#10).
- `ResourceProvisionResult.warnings`, `PluginResult.warnings` (#10).
- `PluginResult.deployment_annotations` and `deployment_labels` (#10).
- `WantsServiceAccount` and `PluginResult.wants_service_account` (#10).
- `PluginContext.provisioned_resources` (#10).

## [0.8.2] - 2026-08-13

### Added

- `ResourceProvisionResult.sidecars`, for sidecar-based plugins (#9).

## [0.8.1] - 2026-08-13

### Added

- `ResourceDeprovisionRequest` and `ResourceDeprovisionResult`, for a `deprovision()` export (#8).

## [0.8.0] - 2026-08-12

### Added

- `ResourceProvisionResult.deployment_annotations` (#7).

The `v0.8.0` tag's `Cargo.toml` still says 0.6.0.

## [0.7.0] - 2026-08-12

### Added

- `now()`: the host's clock (`extism:host/user::now`), for wasm32, where `SystemTime::now()` panics (#6).

The `v0.7.0` tag's `Cargo.toml` still says 0.6.0.

## [0.6.0] - 2026-08-12

### Added

- `PluginResource` and `PluginMetadata.resources`: infrastructure a plugin can provision (#5).
- `ResourceProvisionRequest` and `ResourceProvisionResult`, for a `provision()` export (#5).

## [0.5.1] - 2026-08-12

### Added

- `PluginResult.errors`, for errors the host logs (#4). (A new public field is breaking under Rust semver; it shipped in a patch release.)

The `v0.5.1` tag's `Cargo.toml` still says 0.5.0.

## [0.5.0] - 2026-08-12

### Added

- `ConfigField`, `ConfigFieldType` (`String`, `Secret`, `Bool`, `Select`) and `PluginMetadata.config_schema` (#3).

## [0.4.0] - 2026-08-10

### Added

- `PluginMetadata` (name, version, description, `provides`, `depends_on`, `optional_depends_on`), for a `metadata()` export (#2).
- `PluginContext.plugin_outputs` and `PluginResult.outputs`, to pass data between plugins (#2).

## [0.3.0] - 2026-08-10

### Added

- `PluginResult.service_account_name` (#1).

## [0.2.0] - 2026-08-06

### Added

- `EnvVarSpec.value_from` (`EnvVarSource`, `SecretKeyRef`, `ConfigMapKeyRef`) and the `EnvVarSpec::value`, `secret_key_ref` and `config_map_key_ref` constructors.
- `PluginResult.kubernetes_resources`.

## 0.1.0 - untagged

The initial `PluginContext`, `PluginResult`, `EnvVarSpec` and `SidecarSpec`. There is no `v0.1.0` tag; it is part of `v0.2.0`'s history.

[0.10.1]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.10.0...main
[0.10.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.8.2...v0.9.0
[0.8.2]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/alexconrey/deckwatch-plugin-sdk/releases/tag/v0.2.0
