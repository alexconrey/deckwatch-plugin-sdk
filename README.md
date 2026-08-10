# deckwatch-plugin-sdk

Shared types for [deckwatch](https://github.com/kore/deckwatch) plugins. Add this crate to your plugin alongside `extism-pdk` to get the request/response structs that deckwatch passes to and reads from your WASM binary.

## What is a deckwatch plugin?

A plugin is a compiled WASM binary that deckwatch fetches from a Git repository and executes every time a Kubernetes Deployment is created or updated through deckwatch. Plugins can:

- **Inject environment variables** into the primary container (for external service providers — databases, secret managers, feature flag SDKs)
- **Inject sidecar containers** into the pod spec (for in-cluster service providers — mesh agents, log shippers, monitoring collectors)

Plugins are opt-in per-deployment via Kubernetes annotations. Deckwatch never overwrites values the operator already set.

## Quick start

### 1. Create a new crate

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
crate-type = ["cdylib"]   # required: produces a WASM module with exported functions

[dependencies]
extism-pdk = "1"
deckwatch-plugin-sdk = { git = "https://github.com/alexconrey/deckwatch-plugin-sdk", tag = "v0.1.0" }
serde_json = "1"
```

### 3. Set the default build target

```toml
# .cargo/config.toml
[build]
target = "wasm32-unknown-unknown"
```

### 4. Implement the `apply` function

```rust
// src/lib.rs
use deckwatch_plugin_sdk::{EnvVarSpec, PluginContext, PluginResult, SidecarSpec};
use extism_pdk::*;

#[plugin_fn]
pub fn apply(Json(ctx): Json<PluginContext>) -> FnResult<Json<PluginResult>> {
    let mut result = PluginResult::default();

    // Inject env vars when the deployment opts in via annotation
    if ctx.annotations.get("myorg.io/vault").map(|v| v == "enabled").unwrap_or(false) {
        result.env_vars.push(EnvVarSpec {
            name: "VAULT_ADDR".into(),
            value: "https://vault.myorg.internal:8200".into(),
        });
    }

    Ok(Json(result))
}
```

### 5. Build

```bash
cargo build --release --target wasm32-unknown-unknown
# artifact: target/wasm32-unknown-unknown/release/my_deckwatch_plugin.wasm
```

### 6. Register with deckwatch

Add the plugin to deckwatch's settings (Settings → Plugins):

```json
{
  "plugins": [
    {
      "name": "my-vault-provider",
      "enabled": true,
      "source": {
        "type": "github",
        "repo": "myorg/my-deckwatch-plugin",
        "ref": "v1.0.0",
        "path": "plugin.wasm",
        "use_release": true
      }
    }
  ]
}
```

### 7. Opt deployments in

Add an annotation to any deployment you want the plugin to act on:

```yaml
metadata:
  annotations:
    myorg.io/vault: "enabled"
```

Or through the deckwatch UI: Deployment → YAML editor → add the annotation.

---

## Type reference

### `PluginContext`

Passed as JSON to your `apply` function. Contains everything needed to decide what to inject.

| Field | Type | Description |
|---|---|---|
| `namespace` | `String` | Kubernetes namespace of the target deployment |
| `deployment_name` | `String` | Name of the target deployment |
| `annotations` | `HashMap<String, String>` | Deployment-level annotations — use these as opt-in signals |
| `labels` | `HashMap<String, String>` | Pod template labels |

### `PluginResult`

Returned from your `apply` function. All fields default to empty/None — return `PluginResult::default()` to inject nothing.

| Field | Type | Description |
|---|---|---|
| `env_vars` | `Vec<EnvVarSpec>` | Env vars to inject into the primary container |
| `sidecars` | `Vec<SidecarSpec>` | Sidecar containers to append to the pod spec |
| `kubernetes_resources` | `Vec<serde_json::Value>` | Arbitrary K8s resources applied via server-side apply after the deployment is committed |
| `service_account_name` | `Option<String>` | ServiceAccount to bind to the pod template. Only applied when the pod's current SA is `None` or `"default"`. Combine with a `ServiceAccount` in `kubernetes_resources` to create and bind in one shot (e.g. for IRSA/IAM auth). |

### `EnvVarSpec`

| Field | Type | Description |
|---|---|---|
| `name` | `String` | Environment variable name |
| `value` | `String` | Environment variable value |

**Note:** If a variable with the same name already exists on the container, the plugin's value is skipped. User-supplied env vars always win.

### `SidecarSpec`

| Field | Type | Description |
|---|---|---|
| `name` | `String` | Container name — must be unique within the pod |
| `image` | `String` | Full image reference, e.g. `registry.myorg.internal/agent:v2` |
| `env` | `Vec<EnvVarSpec>` | Env vars for the sidecar container (optional) |
| `port` | `Option<i32>` | Port to expose on the sidecar (optional) |
| `cpu` | `Option<String>` | CPU request and limit, e.g. `"50m"` (same value applied to both) |
| `memory` | `Option<String>` | Memory request and limit, e.g. `"64Mi"` (same value applied to both) |

**Note:** If a container with the same name already exists in the pod, the plugin's sidecar is skipped. This prevents duplicate injection if a plugin is re-applied.

---

## Annotation tracking

Deckwatch records exactly what each plugin injected using deployment annotations:

- `deckwatch.plugin-env/<plugin-name>` — comma-separated list of env var names injected by the plugin
- `deckwatch.plugin-sidecar/<plugin-name>` — comma-separated list of sidecar names injected by the plugin

These annotations are how deckwatch knows what to clean up if a plugin is later disabled. You do not need to manage them from your plugin.

---

## Testing

Because `#[plugin_fn]` wraps the function in an extism ABI, unit tests work best against an inner pure function:

```rust
fn apply_inner(ctx: &PluginContext) -> PluginResult {
    // your real logic here
}

#[plugin_fn]
pub fn apply(Json(ctx): Json<PluginContext>) -> FnResult<Json<PluginResult>> {
    Ok(Json(apply_inner(&ctx)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_annotation_injects_addr() {
        let ctx = PluginContext {
            namespace: "production".into(),
            deployment_name: "api".into(),
            annotations: [("myorg.io/vault".into(), "enabled".into())].into(),
            labels: Default::default(),
        };
        let result = apply_inner(&ctx);
        assert!(result.env_vars.iter().any(|e| e.name == "VAULT_ADDR"));
    }
}
```

Run tests against the host target (not WASM) since WASM tests require a browser or WASI runtime:

```bash
# Assuming .cargo/config.toml sets target = "wasm32-unknown-unknown" by default
cargo test --target $(rustc -vV | sed -n 's/host: //p')
```

---

## Releasing

The recommended pattern is to commit the compiled `plugin.wasm` as a GitHub Release asset and pin deckwatch to a specific tag:

```yaml
# .github/workflows/release.yml
on:
  push:
    tags: ['v*']
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown
      - run: cargo build --release --target wasm32-unknown-unknown
      - uses: softprops/action-gh-release@v2
        with:
          files: target/wasm32-unknown-unknown/release/*.wasm
```

Then in deckwatch settings, set `"use_release": true` and `"ref": "v1.2.0"`.

---

## Versioning

This SDK follows [Semantic Versioning](https://semver.org). The `PluginContext` and `PluginResult` types are the public API:

- **Minor versions** may add new optional fields to `PluginContext` (new data your plugin can read) or to `PluginResult` (new injection capabilities).
- **Patch versions** are bug fixes with no type changes.
- **Major versions** are breaking changes to the function signature or type layout.

Pin to a tag in your `Cargo.toml` (`tag = "v0.1.0"`) to avoid unexpected breakage.
