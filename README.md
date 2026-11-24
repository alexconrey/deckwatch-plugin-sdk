# deckwatch-plugin-sdk

Shared Rust types for [deckwatch](https://github.com/alexconrey/deckwatch) plugins. A plugin is a WebAssembly module, built with [`extism-pdk`](https://github.com/extism/rust-pdk), that deckwatch runs to inject configuration into Kubernetes workloads and to provision infrastructure for applications. This crate defines the JSON that crosses the plugin/host boundary in both directions.

| | |
|---|---|
| SDK version | 0.10.0 on `main` (unreleased); latest tag `v0.9.0`. Items added after v0.9.0 are marked *0.10.0* |
| Rust | 1.82 or newer (`rust-version`) |
| Target | `wasm32-unknown-unknown` |
| Dependencies | `serde`, `serde_json` |
| Host behaviour documented here | deckwatch `main` at `fc03ec2` (2026-11-24, unreleased; its `Cargo.toml` still says 0.6.7). The v0.6.7 release differs: see [differences in the v0.6.7 release](#differences-in-the-v067-release) |

deckwatch keeps its own hand-written copy of these types. Where the two differ, the [type reference](#type-reference) says so.

## Contents

- [How a plugin works](#how-a-plugin-works)
- [Quick start](#quick-start)
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
- **Network:** only extism's HTTP host function, and only to hosts in the plugin's `allowed_hosts` setting. An empty list leaves extism's default in place, which allows no hosts.
- **Time:**
  - Call [`deckwatch_plugin_sdk::now()`](#now), which returns Unix seconds from deckwatch's clock. `std::time::SystemTime::now()` panics in `wasm32-unknown-unknown`.
  - Or read the `CURRENT_TIMESTAMP` config key. deckwatch sets it to Unix seconds for `apply`, `provision` and `deprovision`, but **not** for `metadata`.
- **Logging:** return messages in `errors` (logged at `ERROR`) or `warnings` (logged at `WARN`). Output from `extism_pdk::log!` does not reach deckwatch's logs.
- **Configuration:** read values with `extism_pdk::config::get("KEY")`. Keys and values come from the plugin's settings entry, in this order (later entries override earlier ones):
  1. `config` (static key/value pairs, including values saved through the settings form; see [`ConfigField`](#configfield));
  2. `CURRENT_TIMESTAMP`;
  3. `inherit_env_keys`: each named environment variable of the deckwatch process, under its own name;
  4. `inherit_env_file_keys`: a map from config key to the name of an environment variable that holds a file path. deckwatch reads the file and passes its trimmed contents.

  `metadata` receives only `config`, without steps 2–4. deckwatch never passes `DECKWATCH_ENCRYPTION_KEY` or `DECKWATCH_DATABASE_URL` through steps 3 or 4.

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
deckwatch-plugin-sdk = { git = "https://github.com/alexconrey/deckwatch-plugin-sdk", tag = "v0.9.0" }
serde_json = "1"
```

- Pin the SDK with `tag = "vX.Y.Z"`. Don't use `branch = "main"`, and avoid a bare `rev`.
- Use `crate-type = ["cdylib"]` on its own. Adding `"rlib"` makes `cargo test` on the host fail to link, because extism's host imports only exist in wasm.

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
| `name` | required | Unique name. Used in tracking annotation keys and as the key other plugins use to read this plugin's `outputs` |
| `enabled` | `true` | Disabled plugins are not loaded |
| `source` | required | One of the three shapes below |
| `token_secret` | none | Name of a configured git token, sent as a bearer token when fetching |
| `allowed_hosts` | `[]` | Hosts the plugin may reach over HTTP |
| `config` | `{}` | Config keys and values passed to the plugin |
| `inherit_env_keys` | `[]` | deckwatch environment variables to pass through, under the same name |
| `inherit_env_file_keys` | `{}` | Config key → environment variable holding a file path; the file's trimmed contents are passed |
| `sha256` | none | Expected SHA-256 of the binary. A mismatch rejects it; without it deckwatch logs a warning and loads the binary anyway |
| `mcp_tuning` | `{}` | Saved values for the plugin's [`mcp_tuning_fields`](#mcptuningfield) |

`source` shapes:

| `type` | Fields | Fetched from |
|---|---|---|
| `"github"`, `use_release: true` | `repo` (`owner/name`), `ref`, `path` | `https://github.com/{repo}/releases/download/{ref}/{path}` |
| `"github"`, `use_release` false or absent | same | `https://raw.githubusercontent.com/{repo}/{ref}/{path}` |
| `"url"` | `url` | `url` |
| `"upload"` | `filename` | A file uploaded to deckwatch (`/data/uploads`) |

### 7. Opt deployments in

This example plugin acts only on Deployments with its annotation (on the Deployment's `metadata.annotations`, not the pod template):

```yaml
metadata:
  annotations:
    myorg.io/vault: "enabled"
```

deckwatch itself runs **every** loaded plugin on every Deployment it creates or updates. Opt-in is up to the plugin.

---

## Type reference

How to read the **JSON** column:

- **required**: deserialization fails if the key is missing.
- **default**: a missing key deserializes to the type's default (`""`, `false`, `0`, empty list or map, `None`, or the enum's default variant).
- **omitted when …**: serialization leaves the key out in that case.

"Host" in descriptions means deckwatch `main` at `fc03ec2`; the [host behaviour](#host-behaviour) section has the details.

### `PluginMetadata`

Returned by `metadata()`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `name` | `String` | required | Display name for logs and the UI. The MCP tuning block uses it, falling back to the configured name when empty |
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
- `min_deckwatch_version` compares against deckwatch's own `Cargo.toml` version, which deckwatch doesn't bump between releases: unreleased `main` at `fc03ec2` still reports 0.6.7. Both the v0.6.7 release and `main` enforce it.

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
| `"key_value"` | `KeyValue` | One row per pair (planned; deckwatch currently shows a text input of the JSON) | A JSON object of string keys to string values, e.g. `{"Team":"platform"}`. Empty means no pairs. Read it with [`parse_key_value_field`](#parse_key_value_field) |
| `"select"` | `Select` | Dropdown over `options` | The chosen option |
| `"unknown"` | `Unknown` | Text input | As entered |

**`Unknown` and forward compatibility.** Any `field_type` this SDK doesn't recognise deserializes to `Unknown` (`#[serde(other)]`) instead of failing, so a host built on this SDK can still load metadata from a plugin built on a newer one. The conversion is lossy: `Unknown` serializes as `"unknown"`, not as the original string. Don't return `Unknown` from a plugin.

**Host support for `key_value`:**

| deckwatch | Behaviour |
|---|---|
| `main` since `278de71` (2026-11-24; not yet released) | Accepts `key_value` and unknown types (rendered as text inputs). Validates `key_value` values on save, in the settings form and in MCP `update_plugin_config`; an invalid value rejects the whole save. Over MCP the value may be the JSON object itself or its JSON string |
| v0.6.7 release and older | Doesn't know `key_value` or `Unknown`. One unrecognised `field_type` makes the plugin's whole `metadata()` fail to parse, and deckwatch falls back to empty metadata, so the plugin loses **every** config field and resource it declares |

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
| `label` | `String` | required | Button and card label, e.g. `"RDS Database"` |
| `icon` | `String` | default | MDI icon name, e.g. `"mdi-database"` |
| `description` | `String` | default | Text in the provisioning dialog |
| `singleton` | `bool` | default | At most one per application; the host refuses a second |
| `fields` | `Vec<ConfigField>` | default | Provisioning form fields. Values arrive in `ResourceProvisionRequest.fields` |
| `output_keys` | `Vec<String>` | default | The `state` keys this resource will produce, shown as a preview in the UI |

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
| `errors` | `Vec<String>` | default, omitted when empty | Logged at `ERROR`. Never blocks the Deployment |
| `warnings` | `Vec<String>` | default, omitted when empty | Logged at `WARN` |
| `init_containers` | `Vec<SidecarSpec>` | default, omitted when empty | *0.10.0.* Init containers for the pod. Appended, not prepended (see [how results are merged](#how-results-are-merged-create-and-update)) |

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

### `ResourceProvisionResult`

Output of `provision()`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `state` | `HashMap<String, String>` | default | Stored with the resource. Each key becomes an env var in the first container of every Deployment and CronJob labelled `deckwatch.io/application=<application>`, and the whole map is passed back in `ResourceDeprovisionRequest.state` and `PluginContext.provisioned_resources` |
| `deployment_annotations` | `HashMap<String, String>` | default | Stored with the resource and applied to the application's Deployments (not CronJobs) |
| `sidecars` | `Vec<SidecarSpec>` | default | Stored with the resource and applied to the application's Deployments and CronJobs |
| `init_containers` | `Vec<SidecarSpec>` | default, omitted when empty | *0.10.0.* Meant to be stored and applied like `sidecars`, but deckwatch never stores it, so it has no effect ([deckwatch#249](https://github.com/alexconrey/deckwatch/issues/249)) |
| `kubernetes_resources` | `Vec<serde_json::Value>` | default | Kubernetes objects to server-side apply, with the same rules as `PluginResult.kubernetes_resources` |
| `errors` | `Vec<String>` | default | Logged at `ERROR`. Provisioning still succeeds |
| `warnings` | `Vec<String>` | default, omitted when empty | Logged at `WARN` |
| `status` | `ProvisioningStatus` | default (`Pending`) | See [provisioning](#provisioning) for exactly what the host does with it |
| `retry_after_secs` | `Option<u64>` | default, omitted when `None` | Minimum seconds before the host re-polls this resource. Honoured only by the background re-poll |

`errors` is always serialized, unlike `PluginResult.errors`.

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

### `ResourceDeprovisionResult`

Output of `deprovision()`. Derives `Default`.

| Field | Type | JSON | Description |
|---|---|---|---|
| `errors` | `Vec<String>` | default | Logged at `ERROR`. The record is deleted anyway |
| `message` | `String` | default | What was and wasn't cleaned up. Logged at `INFO`; the host doesn't show it in the UI |
| `delete_kubernetes_resources` | `Vec<KubernetesObjectRef>` | default, omitted when empty | *0.10.0.* Kubernetes objects for deckwatch to delete after `deprovision()` returns. No deckwatch version supports this field yet (checked at `fc03ec2`); hosts ignore it |

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

What the SDK specifies for hosts that support the field (none yet; deckwatch ignores it as of `fc03ec2`):
- Only `v1` `ServiceAccount` and `batch/v1` `Job` are deleted.
- Jobs are deleted with their pods, and before ServiceAccounts.
- A ServiceAccount still used by a Deployment, CronJob or Job in the namespace is kept.
- An object that is already gone counts as deleted.
- Refusals and failures are logged and audited, and never block removal of the resource record.

---

## Host behaviour

This section describes what deckwatch does with plugin output, as read from deckwatch's code on `main` at `fc03ec2` (2026-11-24). That commit is unreleased, and deckwatch's `Cargo.toml` version (0.6.7) is not bumped between releases, so the version number alone doesn't identify the behaviour. None of this is a promise about future versions. Behaviour the host's own documentation contradicts is listed under [known host issues](#known-host-issues).

### Differences in the v0.6.7 release

The v0.6.7 release (2026-11-06) is about 100 commits older than `fc03ec2`. These differences matter to plugins and the operators who run them:

| Area | v0.6.7 release | `main` at `fc03ec2` |
|---|---|---|
| `inherit_env_keys` / `inherit_env_file_keys` | **Will pass `DECKWATCH_ENCRYPTION_KEY` and `DECKWATCH_DATABASE_URL` to a plugin** if configured to | Refuses them (`910e9bb`) |
| `GET /api/settings` (unauthenticated) | **Returns plugin config values unmasked**, secret fields included; saving the plugin page could overwrite stored secrets with the masked placeholder | Masks values as `"configured"` unless the schema declares the key non-secret (`bf332ae`) |
| `key_value` and unknown `field_type`s | One unknown type fails the whole `metadata()` parse, and the plugin loses all its config fields and resources | Supported; see [`ConfigFieldType`](#configfieldtype) (`278de71`) |
| MCP tuning settings | `GET /api/plugins` drops `mcp_tuning_fields`, so the settings page never shows a plugin's tuning fields | Shown (`723492c`) |
| `"upload"` sources | Uploading a plugin unloads every other plugin until the next full reload | Only the uploaded plugin is reloaded (`13a0bfc`) |

Everything else in this section, and every [known host issue](#known-host-issues), applies to both.

### When `apply()` runs

| Path | Trigger | Context |
|---|---|---|
| **Create** | REST `POST /api/namespaces/{ns}/deployments`, before the Deployment is created | `provisioned_resources` empty; `plugin_outputs` chained between plugins |
| **Update** | REST `PUT /api/namespaces/{ns}/deployments/{name}`, after the operator's edits and before the replace | Same as create |
| **Reconcile** | Every 10 s, for each application with at least one provisioned resource, once per Deployment labelled `deckwatch.io/application=<app>` | `provisioned_resources` filled; `plugin_outputs` always empty |
| **Validate** | MCP `validate_plugin` dry run; nothing is applied | Built from the tool's test arguments; `metadata()` is not called |

- Every loaded plugin runs on every path, in dependency order. Associating a plugin with an application doesn't limit where it runs.
- `apply()` is never called for CronJobs, StatefulSets, DaemonSets or Jobs.
- MCP `create_deployment`/`update_deployment` and YAML edits don't run plugins.

### Which `PluginResult` fields each path uses

| Field | Create / update | Reconcile |
|---|---|---|
| `env_vars`, `sidecars`, `init_containers`, `kubernetes_resources`, `service_account_name`, `wants_service_account` | used | ignored |
| `outputs` | passed to later plugins | ignored |
| `deployment_annotations`, `deployment_labels` | **ignored** | applied with server-side apply (field manager `deckwatch-plugin-resources`) |
| `errors`, `warnings` | logged | logged |

If `apply()` traps or returns invalid JSON, the host logs an error and skips that plugin; the Deployment still goes ahead.

### How results are merged (create and update)

- **Env vars** go into the first container. A variable already present there, by name, is left alone: operator values win.
- **Sidecars** are appended. A sidecar is skipped if a container with that name exists.
- **Volumes** from every sidecar's `volumes` are added to the pod, skipping names that already exist. Each sidecar's `primary_volume_mounts` are added to the first container, skipping any with the same name and mount path.
- **Resources:** the request is `cpu_request`, else `cpu`; the limit is `cpu_limit`, else `cpu`. Memory works the same way. On the reconcile path, provisioned sidecars always get both `requests` and `limits` maps, even when one is empty.
- **Init containers** are appended to `initContainers`, so they run after the workload's own, and skipped if one with the same name exists. Their `volumes` aren't added to the pod, their `primary_volume_mounts` are ignored, and no tracking annotation is written ([deckwatch#249](https://github.com/alexconrey/deckwatch/issues/249)).
- **ServiceAccount:** set only when the pod's `serviceAccountName` is unset or `"default"`. When several plugins set one, the first in run order wins. An empty name is not filtered out, so don't return `Some("")`.
- **`wants_service_account`:** after the Deployment is saved, the host creates or patches the ServiceAccount, adding the IRSA annotation when `irsa_role_arn` is set.
- **Tracking annotations** on the Deployment's `metadata.annotations`, where `<plugin>` is the configured name:
  - `deckwatch.plugin-env/<plugin>`: comma-separated names of the env vars injected in this call. Written only if at least one was injected.
  - `deckwatch.plugin-sidecar/<plugin>`: same, for sidecars.
  - `deckwatch.plugin-sa/<plugin>`: the ServiceAccount name that was set.
- **Nothing is ever removed.** The host doesn't read the tracking annotations back. Disabling a plugin, or a plugin no longer returning something, leaves the injected env vars, sidecars, ServiceAccount and annotations in place.

### `kubernetes_resources`

- Server-side applied with field manager `deckwatch-plugin` and `force`.
- The kind must be known to the cluster's API discovery; an uninstalled CRD fails.
- Without `metadata.namespace` the object is applied cluster-wide, which fails for namespaced kinds. The host does not fill in the namespace.
- On create and update they are applied in the background after the Deployment is saved, before `wants_service_account`. From `provision()` on submit and refresh, they are applied before the resource record is written; on a re-poll, after it is updated.
- Failures are logged per object and never fail the request.

### Provisioning

1. An operator submits a resource's form (REST or MCP `provision_resource`). The host checks that the plugin is loaded, that it declares `resource_id`, and `singleton`. It calls `provision()`, applies `kubernetes_resources`, then stores `state`, `deployment_annotations` and `sidecars`. If `provision()` itself fails, the request fails with HTTP 400. `errors` don't fail it.
2. **Refresh** (REST or MCP `refresh_resource`) calls `provision()` again with the stored `fields` and replaces the stored values.
3. **Re-poll:** every 30 s, the host calls `provision()` again for each resource whose stored `state` looks pending (see below). `status` and `retry_after_secs` only matter here.
4. **Reconcile:** every 10 s, the stored `state` of all of an application's resources is merged (on a key collision, a later resource wins) and applied as plain env vars, together with the stored sidecars and annotations, to the application's workloads (field manager `deckwatch-plugin-resources`).
5. **Deprovision:** `DELETE` of the resource (REST or MCP `deprovision_resource`) calls `deprovision()` with the stored `state` and `fields`, then deletes the record whatever the outcome. Deleting a Deployment or an application does not call `deprovision()`.

**What makes a resource "pending"** is the stored `state`, not `status`. It is pending if:

- any key ending in `_STATUS` has the value `provisioning`, `creating`, `modifying` or `backing-up`; or
- it has a `DB_ENGINE` or `DB_STATUS` key and `DB_HOST` is missing or empty.

**How `status` is used on a re-poll:** `Available` and `Failed` results are always stored. A `Pending` result is stored only if it has more `state` keys than before, or no longer looks pending. `Failed` is not stored or shown separately.

**`retry_after_secs`** is stored on the resource and makes the re-poll skip it until `updated_at + retry_after_secs` has passed. A refresh clears it.

So, to be re-polled, put a `<SOMETHING>_STATUS` key with an in-progress value in `state`. When the resource is ready, change that value and return `Available`.

### Settings form and secrets

- Values saved through the settings form are stored in `config` under `ConfigField.key`.
- `required` is checked by the form only.
- `default` and `env_source` are display-only (see [`ConfigField`](#configfield)).
- The settings API returns every config value masked as `"configured"`, except keys the schema declares as non-secret.

---

## Known host issues

These are places where deckwatch's behaviour differs from what its own types or documentation say. All of them exist both in the v0.6.7 release and on `main` at `fc03ec2`. Each links to its deckwatch issue. Until they're fixed, plugins should work around them.

1. **`secret` config values reach the plugin encrypted.** ([deckwatch#251](https://github.com/alexconrey/deckwatch/issues/251)) With `DECKWATCH_ENCRYPTION_KEY` set, values saved through the settings form for `secret` fields are stored encrypted, and nothing decrypts them before calling the plugin. Values set through MCP `update_plugin_config` are stored in plaintext.
2. **`PluginResult.deployment_annotations` and `deployment_labels` are ignored on create and update.** ([deckwatch#244](https://github.com/alexconrey/deckwatch/issues/244)) They are applied only by reconcile, which only covers applications with at least one provisioned resource.
3. **`ProvisioningStatus` doesn't drive re-polling.** ([deckwatch#245](https://github.com/alexconrey/deckwatch/issues/245)) The pending check on `state` does (see [provisioning](#provisioning)). A `Pending` result whose `state` doesn't look pending is never re-polled.
4. **`retry_after_secs` leaks onto Deployments** ([deckwatch#246](https://github.com/alexconrey/deckwatch/issues/246)) as the annotation `deckwatch.io/retry-after-secs`, because it's stored with the resource's annotations.
5. **Provisioned sidecars lose `value_from`.** ([deckwatch#247](https://github.com/alexconrey/deckwatch/issues/247)) On the reconcile path, sidecar env vars are written as `{name, value}` only. `apply()` sidecars keep it.
6. **No cleanup** ([deckwatch#248](https://github.com/alexconrey/deckwatch/issues/248)) of anything injected through `apply()` (see [how results are merged](#how-results-are-merged-create-and-update)). When an application's last resource is deleted, reconcile stops visiting it, so its env vars, sidecars and annotations stay too.
7. **`ResourceDeprovisionResult.message` isn't shown to the operator.** ([deckwatch#250](https://github.com/alexconrey/deckwatch/issues/250)) It's logged at `INFO`, and the delete request returns `204 No Content`.
8. **`init_containers` are appended, not prepended** ([deckwatch#249](https://github.com/alexconrey/deckwatch/issues/249)), so a plugin's init container runs after the workload's own. Those returned by `provision()` are never stored, so they have no effect.

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

## Versioning and compatibility

- The SDK follows semantic versioning within 0.x:
  - a **minor** bump (0.9 → 0.10) may break you: a removed or renamed type, field or constructor, a changed field type, or a higher minimum Rust version;
  - a **patch** bump only adds things or fixes bugs.
- **Wire compatibility.** Every field added to a type that crosses the boundary has a default, so an older host can read a newer plugin's output and vice versa. Unknown JSON keys are ignored on both sides; no type uses `deny_unknown_fields`.
- **Source compatibility.** From 0.10.0:
  - **Enum variants:** `ConfigFieldType` and `ProvisioningStatus` are `#[non_exhaustive]`, so the compiler requires a `_` arm in every `match`, and adding a variant is not a breaking change.
  - **Struct fields:** every struct derives `Default`. Build structs with `..Default::default()`, never with a literal that names every field, and new fields won't break your build. Adding a public field still counts as breaking in Rust's semver rules (it breaks full literals), so the SDK releases it in a minor version, and CI's `cargo semver-checks` enforces that.

  Upgrading from 0.9 to 0.10 needs a `_` arm in any exhaustive `match` on those enums, and updates to struct literals of changed types.
- **Pin by tag:** `tag = "vX.Y.Z"`. Not every tag has a GitHub Release yet; the tags themselves are stable.

## Developing this crate

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --target wasm32-unknown-unknown -- -D warnings
cargo test --locked
cargo build --locked --target wasm32-unknown-unknown
RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps
```

CI runs these on Rust 1.99 (`rust-toolchain.toml`), and builds and tests on the minimum version, 1.82. A unit test checks that every type in the [type reference](#type-reference) lists exactly the type's JSON fields, so a change to a type has to update this README in the same PR. Agent and contributor rules are in [`CLAUDE.md`](CLAUDE.md).
