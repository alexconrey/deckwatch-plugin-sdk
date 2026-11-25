# deckwatch-plugin-sdk

Shared Rust types for [deckwatch](https://github.com/alexconrey/deckwatch) plugins. A plugin is a WebAssembly module, built with [`extism-pdk`](https://github.com/extism/rust-pdk), that deckwatch runs to inject configuration into Kubernetes workloads and to provision infrastructure for applications. This crate defines the JSON that crosses the plugin/host boundary in both directions.

| | |
|---|---|
| SDK version | 0.10.0 ([release notes](https://github.com/alexconrey/deckwatch-plugin-sdk/releases/tag/v0.10.0)). Items added in 0.10.0 and later are marked with their version, such as *0.10.0* |
| Rust | 1.82 or newer (`rust-version`) |
| Target | `wasm32-unknown-unknown` |
| Dependencies | `serde`, `serde_json` |
| Host behaviour documented here | deckwatch 0.7.0: `main` at `604a36f` (2026-11-25). Its `Cargo.toml` says 0.7.0 and its CHANGELOG dates the release 2026-11-25, but it isn't tagged yet; the latest release is v0.6.7, which differs a lot. See [differences in the v0.6.7 release](#differences-in-the-v067-release) |

deckwatch keeps its own hand-written copy of these types. Where the two differ, the [type reference](#type-reference) says so.

## Contents

- [How a plugin works](#how-a-plugin-works)
- [Quick start](#quick-start)
- [Events and tracing](#events-and-tracing)
- [Type reference](#type-reference)
- [Host behaviour](#host-behaviour)
- [Known host issues](#known-host-issues)
- [Testing](#testing)
- [Releasing your plugin](#releasing-your-plugin)
- [Versioning and compatibility](#versioning-and-compatibility)
- [Developing this crate](#developing-this-crate)

## How a plugin works

### Exports

A plugin exports up to four functions. Each takes JSON input and returns JSON output.

| Export | Input | Output | Required | When deckwatch calls it |
|---|---|---|---|---|
| `metadata` | none (empty input) | [`PluginMetadata`](#pluginmetadata) | No. If it is missing or fails, deckwatch uses `PluginMetadata::default()` with `name` set to the plugin's configured name | Once, when the plugin is loaded |
| `apply` | [`PluginContext`](#plugincontext) | [`PluginResult`](#pluginresult) | Yes | On Deployment create and update, and on the reconcile loop. See [when `apply()` runs](#when-apply-runs) |
| `provision` | [`ResourceProvisionRequest`](#resourceprovisionrequest) | [`ResourceProvisionResult`](#resourceprovisionresult) | Only if `metadata` declares `resources` | When an operator provisions or refreshes a resource, and while it is pending. See [provisioning](#provisioning) |
| `deprovision` | [`ResourceDeprovisionRequest`](#resourcedeprovisionrequest) | [`ResourceDeprovisionResult`](#resourcedeprovisionresult) | No. A missing export counts as success with an empty result | Before a provisioned resource's record is deleted |

### The sandbox

- **No WASI.** There is no filesystem, no environment, no threads, and no system clock.
- **Network:** only extism's HTTP host function (`extism_pdk::http::request`), and only to hosts in the plugin's `allowed_hosts` setting. An empty list leaves extism's default in place, which allows no hosts.
  - extism checks only the first request's host. It then follows up to 10 redirects to any host ([deckwatch#318](https://github.com/alexconrey/deckwatch/issues/318)), so `allowed_hosts` doesn't guarantee where a request ends up.
  - HTTP error statuses come back to the plugin as normal responses.
  - **A request that can't be made at all traps the whole export call.** That covers a host not in `allowed_hosts`, an invalid URL, DNS or connection failures, and a response body over 50 MiB (extism's default `max_http_response_bytes`, which deckwatch doesn't change). extism handles these in the host, so the plugin can't catch them; `extism_pdk::http::request`'s `Err` only covers plugin-side problems.
  - **No HTTP timeout of its own.** extism limits a request only to what's left of the call's time limit (the manifest's `timeout_ms`), and running out traps the call. deckwatch sets no limit ([deckwatch#301](https://github.com/alexconrey/deckwatch/issues/301)), so a server that never answers blocks the call indefinitely.
  - **When a call traps, deckwatch treats it as failed**, and deckwatch 0.7.0 reports a `plugin.trap` error event:
    - `apply` is skipped for that Deployment, which still goes ahead;
    - `provision` stores nothing, and on submit or refresh the request fails: HTTP 502 in deckwatch 0.7.0, HTTP 400 in v0.6.7;
    - after a failed `deprovision`, the resource's record is deleted anyway, but no Kubernetes objects are, since there's no `delete_kubernetes_resources` list to read.
  - Checked in extism 1.41 (deckwatch 0.7.0's version, `src/pdk.rs`, `http_request`) and with a test plugin. extism 1.30, in v0.6.7, has the same `http_request`. Validate URLs yourself, and put outbound calls only where a failed call is acceptable.
- **Time:**
  - Call [`deckwatch_plugin_sdk::now()`](#now), which returns Unix seconds from deckwatch's clock. `std::time::SystemTime::now()` panics in `wasm32-unknown-unknown`.
  - Or read the `CURRENT_TIMESTAMP` config key. deckwatch sets it to Unix seconds for `apply`, `provision` and `deprovision`, but **not** for `metadata`.
- **Logging:** three channels.
  - **Events** (the [event helpers](#reporting-events) `info`, `warn` and `error`). These are the preferred way to report problems. deckwatch 0.7.0 reads them on every call:
    - it logs each one once (target `deckwatch::plugin_events`, at the event's own level, with fields `plugin` (the registered name), `export`, `code`, `retryable` and `resource_id`), rate-limited per plugin to bursts of 60, then 1 per second;
    - it returns them wherever the call has a response. MCP tools append them after their JSON, as `Plugin events (N):` inside a fence that marks them as untrusted data;
    - it records them on the plugin's [status](#status-kubernetes-events-and-notifications).

    Each helper also copies the event into `errors` or `warnings` (on deprovision, `message`) for older hosts, and deckwatch 0.7.0 drops those copies instead of reporting them twice. The v0.6.7 release ignores `events` and logs only the copies.
  - **`errors` and `warnings` you fill yourself.** deckwatch 0.7.0 turns each one into an event with code `plugin.unspecified`: an error event or a warning event respectively. v0.6.7 logs them at `ERROR` or `WARN`.
  - **`extism_pdk::log!`** and the level macros `info!`, `warn!`, `error!`, `debug!` and `trace!`. extism turns each call into a host `tracing` event at the same level, with target `extism::pdk` and one field, `plugin`, which holds extism's per-call instance id.
    - **Attribution:** deckwatch 0.7.0 emits it inside the call's `plugin {export}` span, whose fields include `deckwatch.plugin.name` (the registered name), the namespace, application, Deployment and resource. deckwatch's JSON log output prints the span, so each line can be attributed to its plugin.
    - **Which deckwatch keeps them:** the default log filter keeps `extism::pdk` since `36e6637` (deckwatch#256). Earlier commits, including the v0.6.7 release, drop these lines unless `RUST_LOG` includes `extism::pdk`.
  - `apply` runs on every 10-second reconcile for applications with provisioned resources, so `info!` lines there repeat constantly. Prefer `debug!` for routine messages. Operators can keep only plugin warnings with `extism::pdk=warn` in `RUST_LOG`.
- **Configuration:** read values with `extism_pdk::config::get("KEY")`. Keys and values come from the plugin's settings entry, in this order (later entries override earlier ones):
  1. `config` (static key/value pairs, including values saved through the settings form; see [`ConfigField`](#configfield));
  2. `CURRENT_TIMESTAMP`;
  3. `inherit_env_keys`: each named environment variable of the deckwatch process, under its own name;
  4. `inherit_env_file_keys`: a map from config key to the name of an environment variable that holds a file path. deckwatch reads the file and passes its trimmed contents.

  `metadata` receives only `config`, without steps 2–4.

  **deckwatch 0.7.0 refuses its own secrets in steps 3 and 4.** That covers `DECKWATCH_ENCRYPTION_KEY`, `DECKWATCH_DATABASE_URL`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `OTEL_EXPORTER_OTLP_HEADERS` and the per-signal `OTEL_EXPORTER_OTLP_*_HEADERS`. Each refused key adds a `plugin.policy_rejected` warning to every call, and the plugin runs without it. v0.6.7 passes any variable it's configured to.

  A plugin that needs AWS credentials needs its own: a workload identity token through `inherit_env_file_keys` (for example `{"AWS_IDENTITY_TOKEN": "AWS_WEB_IDENTITY_TOKEN_FILE"}`), or keys in its `config`.

### `parse_key_value_field`

```rust
pub fn parse_key_value_field(raw: &str) -> Result<BTreeMap<String, String>, String>
```

Parses the stored value of a [`key_value`](#configfieldtype) config field. Returns `Ok` with an empty map for an empty or whitespace-only value, `Ok` with the pairs for a JSON object of string values, and `Err` with a message for anything else (invalid JSON, an array, non-string values, or `Key=Value` text), so a plugin can report a misconfiguration instead of ignoring it.

```rust
let tags = match extism_pdk::config::get("RESOURCE_TAGS")? {
    Some(raw) => deckwatch_plugin_sdk::parse_key_value_field(&raw)
        .map_err(|e| extism_pdk::Error::msg(format!("RESOURCE_TAGS: {e}")))?,
    None => Default::default(),
};
```

### `now()`

```rust
#[cfg(target_arch = "wasm32")]
pub fn now() -> u64
```

Imported from the host as `extism:host/user::now`. It returns the current Unix time in seconds. It only exists when compiling for `wasm32`.

## Quick start

### 1. Create a crate

```bash
cargo new --lib my-deckwatch-plugin
cd my-deckwatch-plugin
```

### 2. Set the crate type and dependencies

```toml
# Cargo.toml
[package]
name = "my-deckwatch-plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]   # produces a WASM module with exported functions

[dependencies]
extism-pdk = "1"
deckwatch-plugin-sdk = { git = "https://github.com/alexconrey/deckwatch-plugin-sdk", tag = "v0.10.0" }
serde_json = "1"
```

- Pin the SDK with `tag = "vX.Y.Z"`. Don't use `branch = "main"`, and avoid a bare `rev`.
- Use `crate-type = ["cdylib"]` on its own. With `"rlib"` added, `cargo test` on the host fails to link (extism's host imports only exist in wasm) unless every `#[plugin_fn]` is also marked `#[cfg(target_arch = "wasm32")]`, as some plugins do.

### 3. Set the default build target

```toml
# .cargo/config.toml
[build]
target = "wasm32-unknown-unknown"
```

### 4. Implement `metadata` and `apply`

```rust
// src/lib.rs
use deckwatch_plugin_sdk::{EnvVarSpec, PluginContext, PluginMetadata, PluginResult};
use extism_pdk::*;

#[plugin_fn]
pub fn metadata() -> FnResult<Json<PluginMetadata>> {
    Ok(Json(PluginMetadata {
        name: "vault".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: "Injects VAULT_ADDR into opted-in deployments".into(),
        ..Default::default()
    }))
}

#[plugin_fn]
pub fn apply(Json(ctx): Json<PluginContext>) -> FnResult<Json<PluginResult>> {
    Ok(Json(apply_inner(&ctx)))
}

// Plain function so it can be unit-tested on the host (see Testing).
fn apply_inner(ctx: &PluginContext) -> PluginResult {
    let mut result = PluginResult::default();
    if ctx.annotations.get("myorg.io/vault").map(String::as_str) == Some("enabled") {
        result
            .env_vars
            .push(EnvVarSpec::value("VAULT_ADDR", "https://vault.myorg.internal:8200"));
    }
    result
}
```

### 5. Build

```bash
cargo build --release
# artifact: target/wasm32-unknown-unknown/release/my_deckwatch_plugin.wasm
```

### 6. Register the plugin with deckwatch

Add an entry to `plugins` in deckwatch's settings (Settings → Plugins):

```json
{
  "plugins": [
    {
      "name": "vault",
      "enabled": true,
      "source": {
        "type": "github",
        "repo": "myorg/my-deckwatch-plugin",
        "ref": "v1.0.0",
        "path": "my_deckwatch_plugin.wasm",
        "use_release": true
      },
      "sha256": "<sha256 of the .wasm file>",
      "allowed_hosts": [],
      "config": {}
    }
  ]
}
```

| Field | Default | Meaning |
|---|---|---|
| `name` | required | Unique name. Used in tracking annotation keys and as the key other plugins use to read this plugin's `outputs`. To upload a binary, deckwatch 0.7.0 needs a name of ASCII letters, digits, `.`, `_` and `-` that doesn't start with `.` |
| `enabled` | `true` | Disabled plugins are not loaded |
| `source` | required | One of the three shapes below |
| `token_secret` | none | Name of a configured git token, sent as a bearer token when fetching |
| `allowed_hosts` | `[]` | Hosts the plugin may reach over HTTP |
| `config` | `{}` | Config keys and values passed to the plugin |
| `inherit_env_keys` | `[]` | deckwatch environment variables to pass through, under the same name. deckwatch 0.7.0 refuses its own secrets (see [the sandbox](#the-sandbox)) |
| `inherit_env_file_keys` | `{}` | Config key → environment variable holding a file path; the file's trimmed contents are passed. The same refusals apply |
| `sha256` | none | Expected SHA-256 of the binary. A mismatch rejects it; without it deckwatch logs a warning and loads the binary anyway. In deckwatch 0.7.0, changing or removing it, or changing `source`, means re-entering the plugin's masked config values (see [settings form and secrets](#settings-form-and-secrets)) |
| `mcp_tuning` | `{}` | Saved values for the plugin's [`mcp_tuning_fields`](#mcptuningfield) |

`source` shapes:

| `type` | Fields | Fetched from |
|---|---|---|
| `"github"`, `use_release: true` | `repo` (`owner/name`), `ref`, `path` | `https://github.com/{repo}/releases/download/{ref}/{path}` |
| `"github"`, `use_release` false or absent | same | `https://raw.githubusercontent.com/{repo}/{ref}/{path}` |
| `"url"` | `url` | `url` |
| `"upload"` | `filename` | A file uploaded with `POST /api/plugins/{name}/upload`, which also switches the plugin's `source` to this shape. Kept in `/data/uploads`. In deckwatch 0.7.0 an upload must match `sha256` when one is set; otherwise it clears the plugin's masked config values and returns their keys as `cleared_config_keys` |

### 7. Opt deployments in

This example plugin acts only on Deployments with its annotation (on the Deployment's `metadata.annotations`, not the pod template):

```yaml
metadata:
  annotations:
    myorg.io/vault: "enabled"
```

deckwatch itself runs **every** loaded plugin on every Deployment it creates or updates. Opt-in is up to the plugin.

## Events and tracing

*0.10.0.* Two additions let a host show what plugins report and trace what they do, without the SDK depending on OpenTelemetry:

- results carry structured [events](#pluginevent): a severity, a stable code, a message, and optional details;
- inputs carry the host's trace context, which the plugin forwards on its own HTTP calls.

The host does the rest. **deckwatch 0.7.0** (`main` at `604a36f`, not yet tagged):
- wraps each plugin call in a `plugin {export}` span;
- reads `events` (see [status, Kubernetes Events and notifications](#status-kubernetes-events-and-notifications));
- fills `trace_context` while it exports OTLP traces.

**The v0.6.7 release supports neither.** It ignores `events`, sends no `trace_context`, and only logs the legacy fields that the helpers below also fill in: `errors`, `warnings`, and the deprovision `message`.

### Reporting events

Add events with the helpers on [`PluginResult`](#pluginresult), [`ResourceProvisionResult`](#resourceprovisionresult) and [`ResourceDeprovisionResult`](#resourcedeprovisionresult) rather than pushing to `events` directly. Each helper also copies the event into the legacy fields that older hosts read, so they still log it:

| Helper | Severity | Copied to, on `PluginResult` and `ResourceProvisionResult` | Copied to, on `ResourceDeprovisionResult` |
|---|---|---|---|
| `info(code, message)` | `info` | nothing: there is no legacy field for it | `message` |
| `warn(code, message)` | `warning` | `warnings` | `message`: there is no `warnings` field |
| `error(code, message)` | `error` | `errors` | `errors` |
| `push_event(event)` | the event's | by severity, as above; `unknown` like `warning` | by severity, as above |

- The copy is `event.legacy_text()`: `"<code>: <message>"`, or only the message when `code` is empty. On `message`, each copy is appended after `"; "`, unless `message` was empty.
- Only the code and message are copied. `resource_id`, `object`, `retryable` and `hint` exist only in `events`.
- Each helper returns the stored `&mut PluginEvent`, so you can set the optional fields. Changing the event afterwards doesn't change its copy.

```rust
use deckwatch_plugin_sdk::PluginResult;

fn report(result: &mut PluginResult) {
    result.warn("vault.addr_default", "VAULT_ADDR is not set; using the default");
    let event = result.error("vault.token_missing", "No Vault token is configured");
    event.hint = Some("Set VAULT_TOKEN in the plugin's settings".into());
    // result.warnings: ["vault.addr_default: VAULT_ADDR is not set; using the default"]
    // result.errors:   ["vault.token_missing: No Vault token is configured"]
}
```

**Codes** are stable identifiers that alerts and documentation can match on. Use `"<plugin-name>.<code>"`, e.g. `"aws.assume_role_failed"`, and keep a code's meaning when you reword its message. The `plugin.` prefix belongs to the host (see [`telemetry` constants](#telemetry-constants)).

**Hosts that read `events`** should:

- treat `unknown` as `warning`;
- not report the copies twice: drop each `errors` or `warnings` entry that equals the `legacy_text()` of one of the result's events, and turn the remaining entries into events with code `plugin.unspecified`. On a deprovision result, the copies of info and warning events are at the end of `message`, in order, joined with `"; "`;
- treat event text as untrusted: cap `message` (about 2 KB) and the number of events per call, and strip control characters.

**deckwatch 0.7.0 follows these rules,** and also:
- **Caps:** `message` at 2048 bytes, `hint` at 512, `code` at 128, and ids and object fields at 253.
- **Events per call:** it keeps at most 20, host events first, then the most severe.
- **Malformed events:** it skips one with a `plugin.output_invalid` warning instead of rejecting the whole result.
- **Text:** it puts event text on one line. It masks the values of `secret` config fields and of file-sourced keys, and the userinfo, query values and fragment of every URL (`scheme://…`). It doesn't mask undeclared config keys or values passed through `inherit_env_keys` ([deckwatch#326](https://github.com/alexconrey/deckwatch/issues/326)), so keep secrets out of event text.
- **Codes:** it lowercases them. It rewrites one that starts with `plugin.`, or has no dot, to `<registered-name>.<code>`. It doesn't check that a dotted code's prefix is the plugin's own name.

### Forwarding the trace

[`PluginContext`](#plugincontext), [`ResourceProvisionRequest`](#resourceprovisionrequest) and [`ResourceDeprovisionRequest`](#resourcedeprovisionrequest) have a `trace_context`: the W3C `traceparent` of the host's span around the call. Their `trace_headers()` method returns `[("traceparent", <trace_context>)]` when the value is a valid version-00 `traceparent`, and nothing otherwise (no context, or a malformed one). Add those headers to every request the plugin makes, so its API calls join the host's trace:

```rust
use deckwatch_plugin_sdk::PluginContext;
use extism_pdk::{http, FnResult, HttpRequest};

fn get(ctx: &PluginContext, url: &str) -> FnResult<Vec<u8>> {
    let mut req = HttpRequest::new(url).with_method("GET");
    for (name, value) in ctx.trace_headers() {
        req = req.with_header(name, value);
    }
    Ok(http::request::<()>(&req, None)?.body())
}
```

`metadata()` takes no input, so it gets no trace context.

**Which deckwatch sends it:** deckwatch 0.7.0 fills `trace_context` on every `apply`, `provision` and `deprovision` call while it exports OTLP traces (`OTEL_EXPORTER_OTLP_ENDPOINT` or `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` set, http/protobuf). Otherwise it leaves the field unset. The v0.6.7 release never sends it.

### `telemetry` constants

The `deckwatch_plugin_sdk::telemetry` module holds the names that hosts and plugins share.

| Constant | Value | Meaning |
|---|---|---|
| `attribute::DECKWATCH_PLUGIN_NAME` | `deckwatch.plugin.name` | Span attribute: the plugin's name |
| `attribute::DECKWATCH_PLUGIN_VERSION` | `deckwatch.plugin.version` | Span attribute: the plugin's version |
| `attribute::DECKWATCH_PLUGIN_EXPORT` | `deckwatch.plugin.export` | Span attribute: the call, such as `apply`. The span is named `plugin {export}` |
| `TRACEPARENT_HEADER` | `traceparent` | The header `trace_headers()` returns |
| `code::RESERVED_PREFIX` | `plugin.` | Prefix of every host event code. Plugins must not use it |
| `code::UNSPECIFIED` | `plugin.unspecified` | A legacy `errors`, `warnings` or `message` string, which has no code |
| `code::FETCH_FAILED` | `plugin.fetch_failed` | The binary couldn't be fetched |
| `code::CHECKSUM_MISMATCH` | `plugin.checksum_mismatch` | The binary doesn't match the configured `sha256` |
| `code::LOAD_FAILED` | `plugin.load_failed` | The binary didn't load or instantiate, or it needs a newer host (`min_deckwatch_version`) |
| `code::TRAP` | `plugin.trap` | The plugin trapped during a call: it panicked, ran out of memory, or made an HTTP request that failed in the host (see [the sandbox](#the-sandbox)) |
| `code::TIMEOUT` | `plugin.timeout` | The call ran past the host's time limit. deckwatch 0.7.0 sets no limit, so it never reports this ([deckwatch#301](https://github.com/alexconrey/deckwatch/issues/301)) |
| `code::OUTPUT_INVALID` | `plugin.output_invalid` | The output isn't valid JSON for the expected type. The message should include the deserializer's error path |
| `code::K8S_APPLY_FAILED` | `plugin.k8s_apply_failed` | Applying a Kubernetes object the plugin returned failed |
| `code::POLICY_REJECTED` | `plugin.policy_rejected` | A host policy refused something the plugin asked for |
| `code::FIELDS_RELEASED` | `plugin.fields_released` | *0.10.1.* Informational: the host released the fields that provisioned resources had added to a workload, because its application has none left (see [provisioning](#provisioning)) |

---

## Type reference

How to read the **JSON** column:

- **required**: deserialization fails if the key is missing.
- **default**: a missing key deserializes to the type's default (`""`, `false`, `0`, empty list or map, `None`, or the enum's default variant).
- **omitted when …**: serialization leaves the key out in that case.

"Host" in descriptions means deckwatch 0.7.0 (`main` at `604a36f`); the [host behaviour](#host-behaviour) section has the details, including how the v0.6.7 release differs.

### `PluginMetadata`

Returned by `metadata()`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | Display name. deckwatch uses it only in the MCP tuning block, falling back to the registered name when empty. Logs, events, status and `GET /api/plugins` use the name the plugin is registered under |
| `version` | `String` | default | Plugin version, e.g. `env!("CARGO_PKG_VERSION")`. Informational; the host never checks it |
| `description` | `String` | default | Human-readable description |
| `provides` | `Vec<String>` | default | Capability IDs this plugin provides. Used only for ordering and `depends_on` checks |
| `depends_on` | `Vec<String>` | default | Capabilities that must be provided by some loaded plugin. If any is missing, the host skips this plugin and logs an error |
| `optional_depends_on` | `Vec<String>` | default | Capabilities that, if provided, must run before this plugin. They only affect ordering |
| `config_schema` | `Vec<ConfigField>` | default | Settings form fields for this plugin's `config` |
| `resources` | `Vec<PluginResource>` | default | Infrastructure this plugin can provision. Each one renders as a card on the Application page |
| `mcp_tuning_fields` | `Vec<McpTuningField>` | default | Operator-tunable MCP guidance fields |
| `min_deckwatch_version` | `Option<String>` | default, omitted when `None` | *0.10.0.* Minimum deckwatch version, e.g. `"0.7.0"`. Checked once at load time; an older host skips the plugin and logs an error. Only integer parts are compared, so don't add a `v` prefix (`"v0.7.0"` reads as 7.0.0) |

- Ordering uses Kahn's algorithm over `provides`, `depends_on` and `optional_depends_on`. Plugins with no edges keep their configured order. If two plugins provide the same capability, the later one in the configured order is the provider. A cycle is logged and its members run in their configured order.
- Use namespaced capability IDs (`"domain:capability"`). The AWS plugin family uses `"aws:iam-role"`, `"aws:service-account"`, `"aws:rds-connection"` and `"aws:s3-bucket"`.
- **`min_deckwatch_version` compares against deckwatch's own `Cargo.toml` version, which isn't tied to a tag.** deckwatch `main` has reported 0.7.0 since `f836fec`, before v0.7.0 is tagged, so `"0.7.0"` loads on those builds and is refused by v0.6.7 and by `main` builds before `f836fec` ([deckwatch#292](https://github.com/alexconrey/deckwatch/issues/292) proposes bumping the version right after each release). Both enforce it, and deckwatch 0.7.0 also reports a refusal as a `plugin.load_failed` event on the plugin's status.

### `ConfigField`

One field of a settings form, in `PluginMetadata.config_schema` or `PluginResource.fields`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `key` | `String` | required | Config key; the saved value is passed to the plugin under this key. Use letters, digits and `_` |
| `label` | `String` | required | Form label |
| `description` | `String` | default | Help text under the field |
| `field_type` | `ConfigFieldType` | required | Form control to render |
| `default` | `Option<String>` | default, omitted when `None` | Value pre-filled in the form. The host does **not** pass it to the plugin; read a missing key as "use your default" |
| `required` | `bool` | default | The settings form refuses to save without a value. Enforced only in the UI, not by the host API |
| `options` | `Vec<String>` | default | Choices for `select` fields; ignored otherwise |
| `env_source` | `Option<String>` | default, omitted when `None` | Display only: the UI shows the field read-only with this environment variable's name. To actually receive the value, the operator must list the variable in `inherit_env_keys` (the config key is then the variable name) or `inherit_env_file_keys` |

### `ConfigFieldType`

Enum, serialized in `snake_case`. Derives `Default` (`String`). `#[non_exhaustive]` (*0.10.0*): `match` on it needs a `_` arm. Every value is stored in plugin config as a string.

| JSON value | Variant | Form control | Stored value |
|---|---|---|---|
| `"string"` | `String` (default) | Text input | As entered |
| `"secret"` | `Secret` | Masked input | As entered; encrypted with AES-256-GCM when deckwatch has `DECKWATCH_ENCRYPTION_KEY` set. See [known host issues](#known-host-issues) |
| `"bool"` | `Bool` | Checkbox | `"true"` or `"false"` |
| `"key_value"` | `KeyValue` | One row per pair (planned, [deckwatch#267](https://github.com/alexconrey/deckwatch/issues/267); deckwatch 0.7.0 shows a text input of the JSON) | A JSON object of string keys to string values, e.g. `{"Team":"platform"}`. Empty means no pairs. Read it with [`parse_key_value_field`](#parse_key_value_field) |
| `"select"` | `Select` | Dropdown over `options` | The chosen option |
| `"unknown"` | `Unknown` | Text input | As entered |

**`Unknown` and forward compatibility.** Any `field_type` this SDK doesn't recognise deserializes to `Unknown` (`#[serde(other)]`) instead of failing, so a host built on this SDK can still load metadata from a plugin built on a newer one. The conversion is lossy: `Unknown` serializes as `"unknown"`, not as the original string. Don't return `Unknown` from a plugin.

**Host support for `key_value`:**

| deckwatch | Behaviour |
|---|---|
| deckwatch 0.7.0 (`main` since `278de71`; not yet tagged) | Accepts `key_value` and unknown types, rendered as text inputs (a row-per-pair editor is [deckwatch#267](https://github.com/alexconrey/deckwatch/issues/267)). Validates `key_value` settings on save, in the settings form and in MCP `update_plugin_config`; an invalid value rejects the whole save. Over MCP the value may be the JSON object itself or its JSON string. **Provisioning forms** don't validate `key_value` fields. MCP `provision_resource` takes only string values, so send `key_value` fields as JSON strings there: since [deckwatch#298](https://github.com/alexconrey/deckwatch/pull/298) any other value fails the call, and before it one non-string value dropped every field ([deckwatch#302](https://github.com/alexconrey/deckwatch/issues/302)) |
| v0.6.7 release and older | Doesn't know `key_value` or `Unknown`. One unrecognised `field_type` makes the plugin's whole `metadata()` fail to parse, and deckwatch falls back to empty metadata, logging the failure only at `DEBUG`. So the plugin loses **every** config field and resource it declares, and its `min_deckwatch_version` isn't checked. It still loads, and its exports still run with the stored `config` values |

Only use `key_value` in a plugin once the deckwatch installs it targets have a release that includes `278de71`.

### `McpTuningField`

An operator-tunable line of guidance for AI agents using deckwatch's MCP server, in `PluginMetadata.mcp_tuning_fields`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `key` | `String` | required | Key in the plugin's saved `mcp_tuning` map. Use letters, digits and `_` |
| `label` | `String` | required | Form label, and the line prefix in the MCP instructions |
| `description` | `String` | **required** | Help text under the field |
| `placeholder` | `String` | default | Input placeholder |
| `default` | `Option<String>` | default, omitted when `None` | Value pre-filled in the form. The host does not use it otherwise |

For each enabled, loaded plugin with saved values, the host appends this block to the MCP `initialize` instructions, with one line per field that has a non-empty saved value:

```text
[Plugin: <PluginMetadata.name>]
<label>: <saved value>
```

### `PluginResource`

A kind of resource the plugin can provision, in `PluginMetadata.resources`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `id` | `String` | required | Unique within the plugin, e.g. `"rds"`. Sent back as `resource_id` |
| `label` | `String` | required | Title of the resource's card on the application's Infrastructure tab, and of its provisioning dialog ("Add <label>"), e.g. `"RDS Database"` |
| `icon` | `String` | default | MDI icon name for the card and the dialog, e.g. `"mdi-database"`. deckwatch falls back to `mdi-cube-outline` |
| `description` | `String` | default | Shown on the resource's card |
| `singleton` | `bool` | default | The host refuses a second one before calling `provision()`. deckwatch allows only one of each resource per application anyway (a unique index), and the UI offers **Provision** only while there's none. So without `singleton`, a second REST or MCP submit calls `provision()` and applies its `kubernetes_resources`, then fails to record them ([deckwatch#336](https://github.com/alexconrey/deckwatch/issues/336)) |
| `fields` | `Vec<ConfigField>` | default | Provisioning form fields. Values arrive in `ResourceProvisionRequest.fields` |
| `output_keys` | `Vec<String>` | default | The `state` keys this resource will produce. Documentation only: deckwatch (0.7.0 and v0.6.7) neither checks nor shows it |

### `PluginContext`

Input to `apply()`. Derives `Default` (*0.10.0*).

| Field | Type | JSON | Description |
|---|---|---|---|
| `namespace` | `String` | required | Namespace of the Deployment |
| `deployment_name` | `String` | required | Name of the Deployment |
| `annotations` | `HashMap<String, String>` | required | The Deployment's `metadata.annotations` (not the pod template's) |
| `labels` | `HashMap<String, String>` | required | The pod template's `spec.template.metadata.labels` |
| `plugin_outputs` | `HashMap<String, HashMap<String, String>>` | default | `outputs` of plugins that already ran in this invocation, keyed by plugin `name`. Empty on the reconcile path |
| `provisioned_resources` | `HashMap<String, HashMap<String, String>>` | default | State of every provisioned resource in the application, keyed by `"{plugin_name}/{resource_id}"`. Filled only on the reconcile path; empty on create and update |
| `trace_context` | `Option<String>` | default, omitted when `None` | *0.10.0.* W3C `traceparent` of the host's span around this call. Forward it with `trace_headers()` (see [forwarding the trace](#forwarding-the-trace)). deckwatch 0.7.0 sends it while exporting OTLP traces; v0.6.7 never does |

On create and update, the context reflects the Deployment after the operator's changes and before any plugin runs.

### `PluginResult`

Output of `apply()`. Derives `Default`; return `PluginResult::default()` to change nothing.

| Field | Type | JSON | Description |
|---|---|---|---|
| `env_vars` | `Vec<EnvVarSpec>` | default | Env vars for the Deployment's **first** container |
| `sidecars` | `Vec<SidecarSpec>` | default | Containers appended to the pod |
| `kubernetes_resources` | `Vec<serde_json::Value>` | default | Kubernetes objects to server-side apply after the Deployment is saved. Each needs `apiVersion`, `kind`, `metadata.name`, and `metadata.namespace` if the kind is namespaced |
| `outputs` | `HashMap<String, String>` | default | Values for later plugins, which read them as `ctx.plugin_outputs["<this plugin's name>"]` |
| `service_account_name` | `Option<String>` | default, omitted when `None` | ServiceAccount for the pod template, applied only if the current one is unset or `"default"` |
| `wants_service_account` | `Option<WantsServiceAccount>` | default, omitted when `None` | ServiceAccount for deckwatch to create or patch, and to bind like `service_account_name`. Preferred over a raw ServiceAccount in `kubernetes_resources` |
| `deployment_annotations` | `HashMap<String, String>` | default | Annotations for the Deployment's `metadata.annotations`. Applied only on the reconcile path |
| `deployment_labels` | `HashMap<String, String>` | default | Labels for the Deployment's `metadata.labels`. Applied only on the reconcile path |
| `errors` | `Vec<String>` | default, omitted when empty | Never blocks the Deployment. deckwatch 0.7.0 turns each entry that isn't a helper's copy into an error event (`plugin.unspecified`), which marks the plugin **failing** for the Deployment's application. v0.6.7 logs it at `ERROR`. Prefer the [`error` helper](#reporting-events) |
| `warnings` | `Vec<String>` | default, omitted when empty | Like `errors`, but a warning event, which marks the plugin **degraded**. v0.6.7 logs it at `WARN` |
| `init_containers` | `Vec<SidecarSpec>` | default, omitted when empty | *0.10.0.* Init containers for the pod. Appended, not prepended (see [how results are merged](#how-results-are-merged-create-and-update)) |
| `events` | `Vec<PluginEvent>` | default, omitted when empty | *0.10.0.* Structured events. Add them with the [helpers](#reporting-events), which also copy them into `errors` and `warnings`. deckwatch 0.7.0 reads them (see [status, Kubernetes Events and notifications](#status-kubernetes-events-and-notifications)); v0.6.7 ignores them |

Event helpers (*0.10.0*): `info`, `warn`, `error` and `push_event`; see [reporting events](#reporting-events).

### `WantsServiceAccount`

Used in `PluginResult.wants_service_account`. Derives `Default` (*0.10.0*).

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | ServiceAccount name |
| `irsa_role_arn` | `String` | default | IAM role ARN for the `eks.amazonaws.com/role-arn` annotation. Empty means no IAM binding |

### `EnvVarSpec`

An environment variable. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | Variable name |
| `value` | `String` | default, omitted when empty | Literal value. An empty value is sent to Kubernetes as no value |
| `value_from` | `Option<EnvVarSource>` | default, omitted when `None` | Secret or ConfigMap reference. Takes precedence over `value` |

Constructors:

| Constructor | Result |
|---|---|
| `EnvVarSpec::value(name, value)` | Literal value |
| `EnvVarSpec::secret_key_ref(name, secret, key)` | `value_from.secret_key_ref` = `{ name: secret, key }` |
| `EnvVarSpec::config_map_key_ref(name, config_map, key)` | `value_from.config_map_key_ref` = `{ name: config_map, key }` |

Set exactly one of `value` and `value_from`. If `value_from` has both references, the Secret wins. The host emits references with `optional: false`, so the pod won't start if the key is missing.

### `EnvVarSource`

Derives `Default` (*0.10.0*).

| Field | Type | JSON | Description |
|---|---|---|---|
| `secret_key_ref` | `Option<SecretKeyRef>` | default, omitted when `None` | Read the value from a Secret key |
| `config_map_key_ref` | `Option<ConfigMapKeyRef>` | default, omitted when `None` | Read the value from a ConfigMap key |

### `SecretKeyRef`

Derives `Default` (*0.10.0*).

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | Secret name, in the workload's namespace |
| `key` | `String` | required | Key in the Secret's `data` |

### `ConfigMapKeyRef`

Derives `Default` (*0.10.0*).

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | ConfigMap name, in the workload's namespace |
| `key` | `String` | required | Key in the ConfigMap's `data` |

### `SidecarSpec`

A container to add to the pod. Derives `Default` (*0.10.0*). Also used for init containers.

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | Container name; must be unique in the pod |
| `image` | `String` | required | Image reference, e.g. `registry.myorg.internal/agent:v2` |
| `env` | `Vec<EnvVarSpec>` | default | The container's environment |
| `port` | `Option<i32>` | default, omitted when `None` | Becomes one `containerPort`, with no name and the default protocol (TCP) |
| `cpu_request` | `Option<String>` | default, omitted when `None` | *0.10.0.* CPU request, e.g. `"50m"` |
| `cpu_limit` | `Option<String>` | default, omitted when `None` | *0.10.0.* CPU limit, e.g. `"200m"` |
| `memory_request` | `Option<String>` | default, omitted when `None` | *0.10.0.* Memory request, e.g. `"64Mi"` |
| `memory_limit` | `Option<String>` | default, omitted when `None` | *0.10.0.* Memory limit, e.g. `"128Mi"` |
| `cpu` | `Option<String>` | default, omitted when `None` | Deprecated. Sets both the CPU request and the limit, where `cpu_request` / `cpu_limit` are unset |
| `memory` | `Option<String>` | default, omitted when `None` | Deprecated. Sets both the memory request and the limit, where `memory_request` / `memory_limit` are unset |
| `volumes` | `Vec<VolumeSpec>` | default | *0.10.0.* Volumes added to the pod, so this container (and the primary one) can mount them |
| `volume_mounts` | `Vec<VolumeMountSpec>` | default | *0.10.0.* Mounts for this container |
| `primary_volume_mounts` | `Vec<VolumeMountSpec>` | default | *0.10.0.* Mounts added to the pod's first container on this sidecar's behalf |

Before 0.10.0, `port`, `cpu` and `memory` were serialized as `null` when `None`; hosts accept both.

### `VolumeSpec`

*0.10.0.* A pod volume, in `SidecarSpec.volumes`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | Volume name; must be unique in the pod |
| `volume_type` | `String` | required | `"emptyDir"`, `"secret"` or `"configMap"`. Any other value (`"projected"` included) produces a volume with only a name, which Kubernetes defaults to `emptyDir` |
| `source_name` | `Option<String>` | default, omitted when `None` | Secret name (`secretName`) or ConfigMap name, for those two types |
| `size_limit` | `Option<String>` | default, omitted when `None` | `emptyDir` size limit, e.g. `"256Mi"` |

### `VolumeMountSpec`

*0.10.0.* A volume mount, in `SidecarSpec.volume_mounts` and `SidecarSpec.primary_volume_mounts`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | Name of a volume in the pod |
| `mount_path` | `String` | required | Absolute path in the container |
| `read_only` | `bool` | default | Mount read-only |
| `sub_path` | `Option<String>` | default, omitted when `None` | Mount this path within the volume instead of its root |

### `ResourceProvisionRequest`

Input to `provision()`. Derives `Default` (*0.10.0*).

| Field | Type | JSON | Description |
|---|---|---|---|
| `application_name` | `String` | required | Application the resource belongs to |
| `namespace` | `String` | required | The application's namespace |
| `resource_id` | `String` | required | A `PluginResource.id` from this plugin's metadata |
| `fields` | `HashMap<String, String>` | required | Form values keyed by `ConfigField.key`. On refresh and re-polls, the values stored at provisioning time |
| `trace_context` | `Option<String>` | default, omitted when `None` | *0.10.0.* W3C `traceparent` of the host's span around this call; forward it with `trace_headers()`. deckwatch 0.7.0 sends it while exporting OTLP traces; v0.6.7 never does |

### `ResourceProvisionResult`

Output of `provision()`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `state` | `HashMap<String, String>` | default | Stored with the resource. Each key becomes an env var in the first container of every Deployment and CronJob labelled `deckwatch.io/application=<application>`, and the whole map is passed back in `ResourceDeprovisionRequest.state` and `PluginContext.provisioned_resources` |
| `deployment_annotations` | `HashMap<String, String>` | default | Stored with the resource and applied to the application's Deployments (not CronJobs) |
| `sidecars` | `Vec<SidecarSpec>` | default | Stored with the resource and applied to the application's Deployments and CronJobs |
| `init_containers` | `Vec<SidecarSpec>` | default, omitted when empty | *0.10.0.* Meant to be stored and applied like `sidecars`, but deckwatch never stores it, so it has no effect ([deckwatch#249](https://github.com/alexconrey/deckwatch/issues/249)) |
| `kubernetes_resources` | `Vec<serde_json::Value>` | default | Kubernetes objects to server-side apply, with the same rules as `PluginResult.kubernetes_resources` |
| `errors` | `Vec<String>` | default | Provisioning still succeeds. deckwatch 0.7.0 turns each entry that isn't a helper's copy into an error event, which marks the resource **failing** (a `PluginProvisionFailed` Kubernetes Event, and a notification on the transition). v0.6.7 logs it at `ERROR` |
| `warnings` | `Vec<String>` | default, omitted when empty | Like `errors`, but a warning event, which marks the resource **degraded** (`PluginProvisionWarning`). v0.6.7 logs it at `WARN` |
| `status` | `ProvisioningStatus` | default (`Pending`) | See [provisioning](#provisioning) for exactly what the host does with it |
| `retry_after_secs` | `Option<u64>` | default, omitted when `None` | Minimum seconds before the host re-polls this resource. Honoured only by the background re-poll |
| `events` | `Vec<PluginEvent>` | default, omitted when empty | *0.10.0.* Structured events. Add them with the [helpers](#reporting-events), which also copy them into `errors` and `warnings`. deckwatch 0.7.0 reads them (see [status, Kubernetes Events and notifications](#status-kubernetes-events-and-notifications)); v0.6.7 ignores them |

`errors` is always serialized, unlike `PluginResult.errors`. Event helpers (*0.10.0*): `info`, `warn`, `error` and `push_event`, as on `PluginResult`.

### `ProvisioningStatus`

Enum, serialized as the variant name (`"Pending"`, `"Available"`, `"Failed"`; **not** snake_case). Derives `Default` (`Pending`), and `PartialEq`/`Eq` (*0.10.0*). `#[non_exhaustive]` (*0.10.0*): `match` on it needs a `_` arm.

| JSON value | Meaning |
|---|---|
| `"Pending"` (default) | Still being created |
| `"Available"` | Ready |
| `"Failed"` | Provisioning failed; explain in `errors` |

### `ResourceDeprovisionRequest`

Input to `deprovision()`. Derives `Default` (*0.10.0*).

| Field | Type | JSON | Description |
|---|---|---|---|
| `application_name` | `String` | required | Application the resource belongs to |
| `namespace` | `String` | required | The application's namespace |
| `resource_id` | `String` | required | The resource's `PluginResource.id` |
| `state` | `HashMap<String, String>` | default | The stored `state` |
| `fields` | `HashMap<String, String>` | default | The stored form values |
| `trace_context` | `Option<String>` | default, omitted when `None` | *0.10.0.* W3C `traceparent` of the host's span around this call; forward it with `trace_headers()`. deckwatch 0.7.0 sends it while exporting OTLP traces; v0.6.7 never does |

### `ResourceDeprovisionResult`

Output of `deprovision()`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `errors` | `Vec<String>` | default | The record is deleted anyway. deckwatch 0.7.0 returns them as events in the DELETE response, with a `PluginDeprovisionFailed` Kubernetes Event; v0.6.7 logs them at `ERROR` |
| `message` | `String` | default | What was and wasn't cleaned up. deckwatch 0.7.0 returns it in the DELETE response, shows it in the UI, and uses it as the note of the `PluginDeprovisioned` Kubernetes Event. Any part that isn't a helper's copy becomes an info event. v0.6.7 only logs it, at `INFO` |
| `delete_kubernetes_resources` | `Vec<KubernetesObjectRef>` | default, omitted when empty | *0.10.0.* Kubernetes objects for deckwatch to delete after `deprovision()` returns. deckwatch 0.7.0 deletes them (see [`KubernetesObjectRef`](#kubernetesobjectref)); v0.6.7 ignores the field |
| `events` | `Vec<PluginEvent>` | default, omitted when empty | *0.10.0.* Structured events. The [helpers](#reporting-events) copy errors into `errors` and every other severity into `message`. deckwatch 0.7.0 returns them in the DELETE response; v0.6.7 ignores them |

Event helpers (*0.10.0*): `info`, `warn`, `error` and `push_event`; see [reporting events](#reporting-events).

### `KubernetesObjectRef`

*0.10.0.* An object in the application's namespace, used in `ResourceDeprovisionResult.delete_kubernetes_resources`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `api_version` | `String` | required | API version, e.g. `"v1"` or `"batch/v1"`. The JSON key is `api_version`, not Kubernetes' `apiVersion` |
| `kind` | `String` | required | Object kind, e.g. `"ServiceAccount"` or `"Job"` |
| `name` | `String` | required | Object name. There is no namespace field: the object is always in the application's namespace |

Constructors:

| Constructor | Result |
|---|---|
| `KubernetesObjectRef::new(api_version, kind, name)` | Any object |
| `KubernetesObjectRef::service_account(name)` | `v1` `ServiceAccount` |
| `KubernetesObjectRef::job(name)` | `batch/v1` `Job` |

What deckwatch 0.7.0 does with `delete_kubernetes_resources` (the v0.6.7 release ignores the field):
- **Kinds:** it deletes only `v1` `ServiceAccount` and `batch/v1` `Job`, matched exactly; anything else is refused.
- **Namespace:** only the application's.
- **Order:** Jobs first, with background propagation so their pods go too, then ServiceAccounts. deckwatch doesn't wait for the Jobs' pods to finish terminating.
- **ServiceAccounts it keeps:**
  - one still used by any Deployment, CronJob or Job in the namespace (not just the application's);
  - the namespace's `default` ServiceAccount, always.
- **Validation:** a name that isn't a valid Kubernetes name is refused.
- **Already gone** counts as deleted.
- **No ownership check** ([deckwatch#329](https://github.com/alexconrey/deckwatch/issues/329)). deckwatch deletes whatever ServiceAccount or Job you name, so only list objects your plugin created. Its in-use check doesn't look at StatefulSets, DaemonSets, ReplicaSets or bare Pods either.
- **Outcomes** come back in the DELETE response as `kubernetes_resource_deletions`, each `deleted`, `not_found`, `refused` or `failed`. Refusals and failures are logged and counted in metrics, but aren't plugin events.
- **When nothing is deleted:** if `deprovision()` itself failed, or the plugin isn't loaded.
- **The record** is removed either way.

### `PluginEvent`

*0.10.0.* A structured notice, warning or error, in the `events` of `PluginResult`, `ResourceProvisionResult` and `ResourceDeprovisionResult`. Derives `Default` and `PartialEq`. Every key may be missing (`#[serde(default)]` on the struct), so a partial event still parses instead of failing the whole result. Serialization always writes `severity`, `code`, `message` and `retryable`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `severity` | `EventSeverity` | default (`info`) | How serious the event is |
| `code` | `String` | default | Stable machine code, `"<plugin-name>.<code>"`, e.g. `"aws.assume_role_failed"`. Codes that start with `plugin.` are reserved for the host |
| `message` | `String` | default | Human-readable text. Hosts cap its length (about 2 KB). Don't put secrets in it: hosts log, store and export it |
| `resource_id` | `Option<String>` | default, omitted when `None` | The `PluginResource.id` the event is about |
| `object` | `Option<KubernetesObjectRef>` | default, omitted when `None` | The Kubernetes object, in the application's namespace, the event is about |
| `retryable` | `bool` | default | Repeating the same call may succeed with nothing changed (throttling, a timeout), unlike a failure that needs a fix first |
| `hint` | `Option<String>` | default, omitted when `None` | A suggested fix for the operator |

| Constructor or method | Result |
|---|---|
| `PluginEvent::new(severity, code, message)` | An event with no optional fields set |
| `event.legacy_text()` | `"<code>: <message>"`, or only the message when `code` is empty: the copy the helpers put in legacy fields |

### `EventSeverity`

*0.10.0.* Enum, serialized in `snake_case`. Derives `Default` (`Info`), `Copy`, `PartialEq` and `Eq`. `#[non_exhaustive]`: `match` on it needs a `_` arm.

| JSON value | Variant | Meaning |
|---|---|---|
| `"info"` | `Info` (default) | Worth knowing; nothing is wrong |
| `"warning"` | `Warning` | The call did its job, but something needs attention |
| `"error"` | `Error` | Something the plugin was asked to do failed |
| `"unknown"` | `Unknown` | Any other value. Hosts treat it as `warning` |

**`Unknown` and forward compatibility.** Any severity this SDK doesn't recognise, such as one added by a newer SDK, deserializes to `Unknown` (`#[serde(other)]`) instead of failing. Without it, one new severity would make an older host reject the plugin's entire result. Matching is exact, so `"Error"` is `Unknown` too. `Unknown` serializes as `"unknown"`, not as the original string. Don't return it from a plugin.

---

## Host behaviour

This section describes what deckwatch does with plugin output, as read from its code on `main` at `604a36f` (2026-11-25). That's deckwatch 0.7.0: its CHANGELOG dates the release 2026-11-25, but it isn't tagged yet. The latest release, v0.6.7, behaves differently in many places (see below). None of this is a promise about future versions. Behaviour that contradicts deckwatch's own types or documentation is listed under [known host issues](#known-host-issues).

### Differences in the v0.6.7 release

The v0.6.7 release (2026-11-06) predates deckwatch's plugin events and tracing work. These differences matter to plugins and the operators who run them:

| Area | v0.6.7 release | deckwatch 0.7.0 (`main` at `604a36f`) |
|---|---|---|
| `inherit_env_keys` / `inherit_env_file_keys` | **Passes any variable, `DECKWATCH_ENCRYPTION_KEY` and `DECKWATCH_DATABASE_URL` included,** if configured to | Refuses those two (`910e9bb`), static AWS credentials and the OTLP exporter's headers ([deckwatch#298](https://github.com/alexconrey/deckwatch/pull/298)). Each refused key adds a `plugin.policy_rejected` warning to every call while it's configured |
| `GET /api/settings` (unauthenticated) | **Returns plugin config values unmasked**, secret fields included; saving the plugin page could overwrite stored secrets with the masked placeholder | Masks values as `"configured"` unless the schema declares the key non-secret (`bf332ae`) |
| Changing a plugin's `source` or `sha256` | Stored config values, secrets included, stay and reach the new code | The settings save is refused (`400`) until the masked values are re-entered or removed; an upload to an unpinned plugin clears them ([deckwatch#342](https://github.com/alexconrey/deckwatch/pull/342)) |
| `key_value` and unknown `field_type`s | One unknown type fails the whole `metadata()` parse, and the plugin loses all its config fields and resources | Supported; see [`ConfigFieldType`](#configfieldtype) (`278de71`) |
| MCP tuning settings | `GET /api/plugins` drops `mcp_tuning_fields`, so the settings page never shows a plugin's tuning fields | Shown (`723492c`) |
| `"upload"` sources | Uploading a plugin unloads every other plugin until the next full reload | Only the uploaded plugin is reloaded (`13a0bfc`) |
| Plugin events | `events` ignored; `errors` and `warnings` only logged | Read on every call: logged, returned in responses, and recorded on a status table that drives Kubernetes Events and notifications ([deckwatch#261](https://github.com/alexconrey/deckwatch/pull/261), [deckwatch#263](https://github.com/alexconrey/deckwatch/pull/263), [deckwatch#265](https://github.com/alexconrey/deckwatch/pull/265), [deckwatch#270](https://github.com/alexconrey/deckwatch/pull/270)) |
| Tracing | None | Each call runs in a `plugin {export}` span with `deckwatch.plugin.*` attributes; `trace_context` is filled while OTLP traces are exported ([deckwatch#260](https://github.com/alexconrey/deckwatch/pull/260), [deckwatch#261](https://github.com/alexconrey/deckwatch/pull/261)) |
| `provision()` call fails | HTTP 400 | HTTP 502, with `plugin_events` |
| Deprovision response | `204 No Content`; `message` only logged | `200` with `message`, `plugin_events` and `kubernetes_resource_deletions` |
| `delete_kubernetes_resources` | Ignored | Deleted ([deckwatch#235](https://github.com/alexconrey/deckwatch/pull/235)); see [`KubernetesObjectRef`](#kubernetesobjectref) |
| An application's last resource is removed | Its env vars, sidecars and annotations stay on the workloads | Released by the field sweep ([deckwatch#229](https://github.com/alexconrey/deckwatch/pull/229)); see [provisioning](#provisioning) |
| MCP `validate_plugin` | Doesn't call `metadata()` | Loads the plugin first, including `metadata()` and the version check. Runs it with no network access, git token or inherited values, and refuses `token_secret` and `allowed_hosts` ([deckwatch#298](https://github.com/alexconrey/deckwatch/pull/298)) |
| GitHub sources | Always github.com and raw.githubusercontent.com | Configurable download bases (`endpoints.github_web_base`, `endpoints.github_raw_base`) |

Everything else in this section applies to both versions, unless it says otherwise. Each [known host issue](#known-host-issues) says which versions have it.

### When `apply()` runs

| Path | Trigger | Context |
|---|---|---|
| **Create** | REST `POST /api/namespaces/{ns}/deployments`, before the Deployment is created | `provisioned_resources` empty; `plugin_outputs` chained between plugins |
| **Update** | REST `PUT /api/namespaces/{ns}/deployments/{name}`, after the operator's edits and before the replace | Same as create |
| **Reconcile** | Every 10 s, once per Deployment labelled `deckwatch.io/application=<app>`, for each application with at least one provisioned resource. v0.6.7 skips applications whose resources have no state or sidecars ([deckwatch#345](https://github.com/alexconrey/deckwatch/pull/345) changed that) | `provisioned_resources` filled; `plugin_outputs` always empty |
| **Validate** | MCP `validate_plugin` dry run; nothing is applied | Built from the tool's test arguments. deckwatch 0.7.0 loads the plugin (including `metadata()`) first, under the name `__validate__`, with only the tool's `config`: no `allowed_hosts`, so any HTTP request traps the call, and nothing inherited. v0.6.7 doesn't call `metadata()` |

- Every loaded plugin runs on every path, in dependency order. Associating a plugin with an application doesn't limit where it runs.
- `apply()` is never called for CronJobs, StatefulSets, DaemonSets or Jobs.
- MCP `create_deployment`/`update_deployment`, YAML edits, application seeding and cloning don't run plugins ([deckwatch#283](https://github.com/alexconrey/deckwatch/issues/283)).

### Which `PluginResult` fields each path uses

| Field | Create / update | Reconcile |
|---|---|---|
| `env_vars`, `sidecars`, `init_containers`, `kubernetes_resources`, `service_account_name`, `wants_service_account` | used | ignored |
| `outputs` | passed to later plugins | ignored |
| `deployment_annotations`, `deployment_labels` | **ignored** | applied with server-side apply (field manager `deckwatch-plugin-resources`) |
| `events` (*0.10.0*), `errors`, `warnings` | events (0.7.0); `errors` and `warnings` only logged (v0.6.7) | same |

If `apply()` traps or returns invalid JSON, the host logs an error and skips that plugin; the Deployment still goes ahead. deckwatch 0.7.0 also reports a `plugin.trap` or `plugin.output_invalid` event.

### How results are merged (create and update)

- **Env vars** go into the first container. A variable already present there, by name, is left alone: operator values win.
- **Sidecars** are appended. A sidecar is skipped if a container with that name exists.
- **Volumes** from every sidecar's `volumes` are added to the pod, skipping names that already exist. Each sidecar's `primary_volume_mounts` are added to the first container, skipping any with the same name and mount path.
- **Resources:** the request is `cpu_request`, else `cpu`; the limit is `cpu_limit`, else `cpu`. Memory works the same way. On the reconcile path, provisioned sidecars always get both `requests` and `limits` maps, even when one is empty.
- **Init containers** are appended to `initContainers`, so they run after the workload's own, and skipped if one with the same name exists. Their `volumes` aren't added to the pod, their `primary_volume_mounts` are ignored, and no tracking annotation is written ([deckwatch#249](https://github.com/alexconrey/deckwatch/issues/249)). Their `volume_mounts` are applied, so they can mount a volume a sidecar declares.
- **ServiceAccount:**
  - Set only when the pod's `serviceAccountName` is unset or `"default"`.
  - When several plugins set one, the first in run order wins.
  - deckwatch 0.7.0 reports a `plugin.policy_rejected` warning when it keeps a different, non-default account. So the second of two plugins asking for different accounts shows as degraded, and so does a plugin on a Deployment where the operator chose the account.
  - An empty name is not filtered out, so don't return `Some("")`.
- **`wants_service_account`:** after the Deployment is saved, the host creates or patches the ServiceAccount, adding the IRSA annotation when `irsa_role_arn` is set. Failures are only logged.
- **Tracking annotations** on the Deployment's `metadata.annotations`, where `<plugin>` is the registered name:
  - `deckwatch.plugin-env/<plugin>`: comma-separated names of the env vars injected in this call. Written only if at least one was injected.
  - `deckwatch.plugin-sidecar/<plugin>`: same, for sidecars.
  - `deckwatch.plugin-sa/<plugin>`: the ServiceAccount name that was set.
- **Nothing injected on create or update is ever removed.** The host doesn't read the tracking annotations back. Disabling a plugin, or a plugin no longer returning something, leaves the injected env vars, sidecars, ServiceAccount and annotations in place ([deckwatch#248](https://github.com/alexconrey/deckwatch/issues/248)).

### `kubernetes_resources`

- Server-side applied with field manager `deckwatch-plugin` and `force`.
- The kind must be known to the cluster's API discovery; an uninstalled CRD fails.
- Without `metadata.namespace` the object is applied cluster-wide, which fails for namespaced kinds. The host does not fill in the namespace.
- **When they're applied:**
  - on create and update, in the background after the Deployment is saved, before `wants_service_account`;
  - from `provision()` on submit and refresh, before the resource record is written;
  - on a re-poll, after the record is updated.
- **Failures** are logged per object and never fail the request. deckwatch 0.7.0 also reports a `plugin.k8s_apply_failed` event per failed object:
  - in the response, for provision and refresh;
  - on the application's status, for create, update and re-polls. They aren't in the create/update response, because the objects are applied after it's sent.

### Provisioning

1. **Submit.** An operator submits a resource's form (REST or MCP `provision_resource`). The host:
   - checks that the plugin is loaded and declares `resource_id`, and, for a `singleton` resource, that the application doesn't have one yet. Every resource is limited to one per application by a unique index, `singleton` or not, but without `singleton` the duplicate only fails after `provision()` ran and its objects were applied ([deckwatch#336](https://github.com/alexconrey/deckwatch/issues/336));
   - calls `provision()`, applies `kubernetes_resources`, then stores `state`, `deployment_annotations` and `sidecars`.

   If `provision()` itself fails, the request fails: HTTP 502 `plugin_failed` with `plugin_events` in deckwatch 0.7.0, and HTTP 400 in v0.6.7. `errors` don't fail it. On success, deckwatch 0.7.0 returns 201 with `plugin_events`.
2. **Refresh** (REST or MCP `refresh_resource`) calls `provision()` again with the stored `fields`, and replaces the stored values.
3. **Re-poll:** every 30 s, the host calls `provision()` again for each resource whose stored `state` looks pending (see below). `status` and `retry_after_secs` only matter here. deckwatch 0.7.0 updates the resource's status on every call, even when it doesn't store the result.
4. **Reconcile:** every 10 s, the stored `state` of all of an application's resources is merged and applied as plain env vars, together with the stored sidecars and annotations, to the application's workloads (field manager `deckwatch-plugin-resources`). On a key collision one resource's value wins, but which one isn't defined.
5. **Deprovision:** `DELETE` of the resource (REST or MCP `deprovision_resource`) calls `deprovision()` with the stored `state` and `fields`, then deletes the record whatever the outcome. deckwatch 0.7.0 then deletes `delete_kubernetes_resources` and returns `message`, `plugin_events` and the deletion outcomes. If the plugin isn't loaded, deckwatch deletes the record without calling `deprovision()`, and MCP still reports `deprovisioned` ([deckwatch#336](https://github.com/alexconrey/deckwatch/issues/336)). Deleting a Deployment or an application does not call `deprovision()`; deleting an application deletes its resource records.
6. **Field sweep** (deckwatch 0.7.0, [deckwatch#229](https://github.com/alexconrey/deckwatch/pull/229), [deckwatch#345](https://github.com/alexconrey/deckwatch/pull/345)). On each reconcile tick, deckwatch looks for workloads that no longer belong to an application with resources: the application's last resource was removed, or the workload lost its `deckwatch.io/application` label. It also sweeps on the first cycle after startup.
   - **Which workloads it touches:** the reconciler stamps every workload it patches with `plugins.deckwatch.io/fields-owner`, a per-database id. The sweep releases a workload only if it carries this database's id or, for fields applied before the stamp existed, if it belongs to an application this database knows. So a new or reset database, or another deckwatch, never strips fields it didn't apply. A database restored from a backup keeps its id, though ([deckwatch#380](https://github.com/alexconrey/deckwatch/issues/380)).
   - **What it releases** on those workloads: everything the reconcile loop applied under field manager `deckwatch-plugin-resources`. That's state env vars, provisioned sidecars with their volumes and mounts, stored annotations (including `deckwatch.io/retry-after-secs`), and `PluginResult` deployment annotations and labels.
   - **What it reports:** a `plugin.fields_released` event, and a `PluginFieldsReleased` Kubernetes Event.
   - **What it leaves alone:** whatever `apply()` injected on create or update, `kubernetes_resources`, and ServiceAccounts.

**What makes a resource "pending"** is the stored `state`, not `status`. It is pending if:

- any key ending in `_STATUS` has the value `provisioning`, `creating`, `modifying` or `backing-up`; or
- it has a `DB_ENGINE` or `DB_STATUS` key and `DB_HOST` is missing or empty.

**How `status` is used on a re-poll:**
- `Available` and `Failed` results are always stored. A `Pending` result is stored only if it has more `state` keys than before, or no longer looks pending.
- `Failed` is not stored or shown separately. In deckwatch 0.7.0, a `Failed` result with no error events leaves the resource's status healthy.

**`retry_after_secs`** is stored on the resource and makes the re-poll skip it until `updated_at + retry_after_secs` has passed. A refresh clears it.

So, to be re-polled, put a `<SOMETHING>_STATUS` key with an in-progress value in `state`. When the resource is ready, change that value and return `Available`.

### Settings form and secrets

- Values saved through the settings form are stored in `config` under `ConfigField.key`.
- `required` is checked by the form only ([deckwatch#290](https://github.com/alexconrey/deckwatch/issues/290)).
- `default` and `env_source` are display-only (see [`ConfigField`](#configfield); [deckwatch#291](https://github.com/alexconrey/deckwatch/issues/291)).
- The settings API returns every config value masked as `"configured"`, except keys the schema declares as non-secret.
- **Provisioning forms** (`PluginResource.fields`) store their values as plain text, `secret` fields included. Don't ask for secrets there.
- **Changing a plugin's code** (deckwatch 0.7.0, [deckwatch#342](https://github.com/alexconrey/deckwatch/pull/342)):
  - a settings save that changes `source`, or changes or removes `sha256`, is refused with `400` until the plugin's masked config values are re-entered or removed; the error names them;
  - an upload to a plugin without `sha256` clears those values and returns their keys as `cleared_config_keys`, and an upload to a pinned plugin must match the pin.

  Masked values are every config value except the keys the loaded plugin declares non-secret in `config_schema`. **Declare every non-secret key in `config_schema`,** or operators re-enter it on every upgrade.
- **Event text masking:** deckwatch 0.7.0 masks `secret` settings, file-sourced values and URL credentials in event text, but not undeclared config keys or values passed through `inherit_env_keys` ([deckwatch#326](https://github.com/alexconrey/deckwatch/issues/326)), and it never masks plugin log lines. Keep secrets out of events and logs.

### Status, Kubernetes Events and notifications

deckwatch 0.7.0 only; the v0.6.7 release has none of this.

- **Status rows.** Every plugin call's events are recorded on one status row per target:
  - plugin-wide, for loading and fetching;
  - per application, for `apply()`;
  - per provisioned resource.

  A row is **failing** after a failed call or any error event, **degraded** after a warning or `unknown` event, and **healthy** otherwise. Rows keep the last 10 events. The UI shows them as badges and a problems panel on the application page, and in a Problems view.
- **The `apply()` row** exists only for Deployments labelled `deckwatch.io/application`. On create and update the last Deployment written wins. Each reconcile cycle replaces the row with the worst result across the application's Deployments. `apply()` on a Deployment without that label gets no status row, Kubernetes Event or notification; its events still reach the response and the logs.
- **Kubernetes Events** (events.k8s.io, reporting controller `deckwatch`) go on the affected Deployments, or on CronJobs for released fields. There's one per outcome, not one per plugin event, and info events never become one:
  - a failing call: `PluginApplyFailed`, `PluginProvisionFailed` or `PluginDeprovisionFailed`, with the first error;
  - a degraded call: `PluginApplyWarning` or `PluginProvisionWarning`, with the first warning;
  - a recovery: `PluginRecovered`;
  - an operator's successful provision or refresh: `PluginProvisioned`;
  - a deprovision without errors: `PluginDeprovisioned`, with the deprovision `message`;
  - released fields: `PluginFieldsReleased`.

  For failures and warnings the note reads `plugin <name> (<resource>): <code>: <message>`, plus ` (+N more)` when the call reported other events. Notes are capped at 1 KB, and the same event is published at most every 5 minutes.
- **Notifications:**
  - `plugin_failing` is sent when a target turns failing, with the first error's code and message. `plugin_recovered` is sent when it recovers.
  - A target that fails again within 10 minutes of its last `plugin_failing` sends nothing, and neither does its next recovery.
  - Deprovision failures never notify.
- **What this means for plugins.** An `error` event, or `errors` entry, on a call that otherwise succeeded still turns the target red and notifies. Use `warn` for problems the operator doesn't have to act on.

### Security notes for plugin authors

deckwatch issues that plugin code should allow for. Unless noted, they apply to deckwatch 0.7.0 and v0.6.7:
- **Annotations and labels are tenant input.** Anyone who can edit a Deployment sets them, and deckwatch runs every plugin on every Deployment. Don't let them pick cloud resource names, policies or roles that a plugin acts on with deckwatch's identity ([deckwatch#306](https://github.com/alexconrey/deckwatch/issues/306)).
- **Output isn't policy-checked yet.** deckwatch applies a plugin's pod-spec changes, ServiceAccounts and `kubernetes_resources` of any kind, in any namespace ([deckwatch#308](https://github.com/alexconrey/deckwatch/issues/308)). Return only namespaced objects in the application's namespace, so a future policy gate doesn't break the plugin.
- **Network and resource limits are weak.** `allowed_hosts` doesn't stop redirects ([deckwatch#318](https://github.com/alexconrey/deckwatch/issues/318)), and calls have no time or memory limit ([deckwatch#319](https://github.com/alexconrey/deckwatch/issues/319)).
- **Text leaves the cluster** (deckwatch 0.7.0). Event text reaches Kubernetes Events (readable by anyone with `get events` in the namespace), notifications, MCP clients and OTLP exports, and log lines aren't scrubbed ([deckwatch#326](https://github.com/alexconrey/deckwatch/issues/326)). Never put secrets in events or logs.

---

## Known host issues

These are places where deckwatch's behaviour differs from what its own types or documentation say, as of deckwatch 0.7.0 (`main` at `604a36f`). Each links to its deckwatch issue. Unless noted, the v0.6.7 release has the same issue. Until they're fixed, plugins should work around them.

1. **`secret` config values reach the plugin encrypted.** ([deckwatch#251](https://github.com/alexconrey/deckwatch/issues/251)) With `DECKWATCH_ENCRYPTION_KEY` set, values saved through the settings form for `secret` fields are stored encrypted, and nothing decrypts them before calling the plugin. Values set through MCP `update_plugin_config` are stored in plaintext.
2. **`PluginResult.deployment_annotations` and `deployment_labels` are ignored on create and update.** ([deckwatch#244](https://github.com/alexconrey/deckwatch/issues/244)) They are applied only by reconcile, which only covers applications with at least one provisioned resource.
3. **`ProvisioningStatus` doesn't drive re-polling.** ([deckwatch#245](https://github.com/alexconrey/deckwatch/issues/245)) The pending check on `state` does (see [provisioning](#provisioning)). A `Pending` result whose `state` doesn't look pending is never re-polled.
4. **`retry_after_secs` leaks onto Deployments** ([deckwatch#246](https://github.com/alexconrey/deckwatch/issues/246)) as the annotation `deckwatch.io/retry-after-secs`, because it's stored with the resource's annotations. deckwatch 0.7.0's field sweep releases it, along with everything else, once the application has no resources left.
5. **Provisioned sidecars lose `value_from`.** ([deckwatch#247](https://github.com/alexconrey/deckwatch/issues/247)) On the reconcile path, sidecar env vars are written as `{name, value}` only. `apply()` sidecars keep it.
6. **No cleanup of anything injected through `apply()`** ([deckwatch#248](https://github.com/alexconrey/deckwatch/issues/248)); see [how results are merged](#how-results-are-merged-create-and-update). In v0.6.7, an application's provisioned env vars, sidecars and annotations also stay after its last resource is removed. deckwatch 0.7.0's field sweep ([deckwatch#229](https://github.com/alexconrey/deckwatch/pull/229)) fixes that part.
7. **`init_containers` are appended, not prepended** ([deckwatch#249](https://github.com/alexconrey/deckwatch/issues/249)), so a plugin's init container runs after the workload's own. Those returned by `provision()` are never stored, so they have no effect.
8. **No time limit on plugin calls.** ([deckwatch#301](https://github.com/alexconrey/deckwatch/issues/301)) deckwatch's docs list a `plugin.timeout` event, but no timeout is configured. A plugin that hangs blocks its caller: the request, or the reconcile loop. So does one waiting on an HTTP server that never answers, because extism has no HTTP timeout of its own (see [the sandbox](#the-sandbox)).
9. **`allowed_hosts` is checked only on the first request.** ([deckwatch#318](https://github.com/alexconrey/deckwatch/issues/318)) extism follows redirects to any host.
10. **Event text isn't scrubbed of every secret.** ([deckwatch#326](https://github.com/alexconrey/deckwatch/issues/326)) Undeclared config keys, `inherit_env_keys` values and plugin log lines aren't masked.
11. **A failed duplicate provision leaves untracked objects, and deprovisioning an unloaded plugin reports success.** ([deckwatch#336](https://github.com/alexconrey/deckwatch/issues/336)) See [provisioning](#provisioning).

Fixed in deckwatch 0.7.0, but still in v0.6.7:
- the deprovision `message` was only logged ([deckwatch#250](https://github.com/alexconrey/deckwatch/issues/250));
- MCP `provision_resource` dropped every field when one value wasn't a string ([deckwatch#302](https://github.com/alexconrey/deckwatch/issues/302), fixed by [deckwatch#298](https://github.com/alexconrey/deckwatch/pull/298)). It now rejects the call.

---

## Testing

Put your logic in a plain function and test that on the host. Build test inputs so they keep compiling when the SDK adds fields: from JSON (below; works with every SDK version), or, from 0.10.0, with `..Default::default()` (`PluginContext { namespace: "production".into(), ..Default::default() }`). A struct literal that names every field stops compiling the moment a field is added.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_annotation_injects_addr() {
        let ctx: PluginContext = serde_json::from_value(serde_json::json!({
            "namespace": "production",
            "deployment_name": "api",
            "annotations": { "myorg.io/vault": "enabled" },
            "labels": {}
        }))
        .unwrap();
        let result = apply_inner(&ctx);
        assert!(result.env_vars.iter().any(|e| e.name == "VAULT_ADDR"));
    }
}
```

Run tests for the host target. `.cargo/config.toml` makes wasm32 the default, and wasm32 test binaries can't run without a wasm runtime:

```bash
cargo test --target "$(rustc -vV | sed -n 's/host: //p')"
```

The Quick start plugin and this test were built and run against this SDK with `extism-pdk` 1.4.1 and Rust 1.99.

## Releasing your plugin

Publish the `.wasm` as a GitHub Release asset and point deckwatch at the tag with `"use_release": true`. Also publish the SHA-256 and set it as `sha256` in the plugin's settings.

```yaml
# .github/workflows/release.yml
name: Release
on:
  push:
    tags: ['v*']
permissions:
  contents: read
jobs:
  release:
    runs-on: ubuntu-latest
    permissions:
      contents: write   # create the release
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      # Installs exactly what rust-toolchain.toml pins (channel and the
      # wasm32-unknown-unknown target). Don't use a floating `stable`.
      - run: rustup toolchain install --no-self-update
      - run: cargo build --locked --release --target wasm32-unknown-unknown
      - run: |
          cd target/wasm32-unknown-unknown/release
          sha256sum *.wasm > SHA256SUMS
      - uses: softprops/action-gh-release@8a0cd651e5b2365bd640854a3fa5dfc70e4875f3 # v3.0.4
        with:
          files: |
            target/wasm32-unknown-unknown/release/*.wasm
            target/wasm32-unknown-unknown/release/SHA256SUMS
```

This assumes your plugin has a `rust-toolchain.toml` with `targets = ["wasm32-unknown-unknown"]` and a committed `Cargo.lock`. Pin actions by commit SHA, as above, and let Dependabot keep them current.

**Upgrading a plugin in deckwatch 0.7.0** changes its `source` (the new tag) and its `sha256`. So the operator has to re-enter every masked config value in that same save, or remove it (see [settings form and secrets](#settings-form-and-secrets)). Declare every non-secret config key in `config_schema` to keep that list to the real secrets, and say in your release notes which settings must be re-entered.

## Versioning and compatibility

- The SDK follows semantic versioning within 0.x:
  - a **minor** bump (0.9 → 0.10) may break you: a removed or renamed type, field or constructor, a changed field type, or a higher minimum Rust version;
  - a **patch** bump only adds things or fixes bugs.
- **Wire compatibility.** Every field added to a type that crosses the boundary has a default, so an older host can read a newer plugin's output and vice versa. Unknown JSON keys are ignored on both sides; no type uses `deny_unknown_fields`. `ConfigFieldType` and `EventSeverity` also read values they don't know as `Unknown`, so a new variant doesn't break an older reader. `ProvisioningStatus` has no such catch-all.
- **Source compatibility.** From 0.10.0:
  - **Enum variants:** `ConfigFieldType`, `ProvisioningStatus` and `EventSeverity` are `#[non_exhaustive]`, so the compiler requires a `_` arm in every `match`, and adding a variant is not a breaking change.
  - **Struct fields:** every struct derives `Default`. Build structs with `..Default::default()`, never with a literal that names every field, and new fields won't break your build. Adding a public field still counts as breaking in Rust's semver rules (it breaks full literals), so the SDK releases it in a minor version, and CI's `cargo semver-checks` enforces that.

  Upgrading from 0.9 to 0.10 needs a `_` arm in any exhaustive `match` on those enums, and updates to struct literals of changed types.
- **Pin by tag:** `tag = "vX.Y.Z"`. From 0.10.0 every tag has a [GitHub Release](https://github.com/alexconrey/deckwatch-plugin-sdk/releases) with its notes, and v0.8.1, v0.8.2 and v0.9.0 have releases created afterwards from the changelog. Older tags have none, but [`CHANGELOG.md`](CHANGELOG.md) covers every tag.

## Developing this crate

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --target wasm32-unknown-unknown -- -D warnings
cargo test --locked
cargo build --locked --target wasm32-unknown-unknown
RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps
```

CI runs these on Rust 1.99 (`rust-toolchain.toml`), builds and tests on the minimum version, 1.82, and runs `cargo semver-checks` against the latest release tag. Dependabot proposes dependency and GitHub Actions updates weekly. `tests/wire_compat.rs` checks that JSON stays readable in both directions between this version and the v0.9.0 and v0.3.0 releases. `tests/readme.rs` checks that every type in the [type reference](#type-reference) lists exactly the type's JSON fields, so a change to a type has to update this README in the same PR. Agent and contributor rules are in [`CLAUDE.md`](CLAUDE.md).
