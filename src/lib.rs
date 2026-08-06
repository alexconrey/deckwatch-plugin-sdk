//! # deckwatch-plugin-sdk
//!
//! Shared types for [deckwatch](https://github.com/kore/deckwatch) plugins.
//!
//! A deckwatch plugin is a compiled WASM binary that deckwatch fetches from a
//! Git repository and executes for every deployment create/update. The plugin
//! receives a [`PluginContext`] describing the target deployment and returns a
//! [`PluginResult`] containing env vars and sidecars to inject.
//!
//! ## Minimal plugin
//!
//! ```rust,ignore
//! use deckwatch_plugin_sdk::{EnvVarSpec, PluginContext, PluginResult};
//! use extism_pdk::*;
//!
//! #[plugin_fn]
//! pub fn apply(Json(ctx): Json<PluginContext>) -> FnResult<Json<PluginResult>> {
//!     let mut result = PluginResult::default();
//!
//!     if ctx.annotations.get("myorg.io/vault").map(|v| v == "enabled").unwrap_or(false) {
//!         result.env_vars.push(EnvVarSpec {
//!             name: "VAULT_ADDR".into(),
//!             value: "https://vault.myorg.internal:8200".into(),
//!         });
//!     }
//!
//!     Ok(Json(result))
//! }
//! ```
//!
//! See the [README](https://github.com/alexconrey/deckwatch-plugin-sdk) for full
//! build, test, and release instructions.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Context passed to a plugin's [`apply`] function.
///
/// Deckwatch serializes this as JSON and passes it as input to the WASM
/// `apply` function. The plugin deserializes it using the `extism-pdk`
/// `Json` extractor.
///
/// All fields are populated from the Deployment object **after** user-supplied
/// changes are applied, so the plugin sees the final intent the operator
/// expressed before any plugin injection happens.
///
/// [`apply`]: https://github.com/alexconrey/deckwatch-plugin-sdk#minimal-plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContext {
    /// Kubernetes namespace of the target deployment.
    pub namespace: String,

    /// Name of the target deployment.
    pub deployment_name: String,

    /// Deployment-level annotations.
    ///
    /// Use these as the opt-in signal for your plugin. Convention is a
    /// domain-prefixed key with `"enabled"` as the value, e.g.:
    ///
    /// ```yaml
    /// metadata:
    ///   annotations:
    ///     myorg.io/vault: "enabled"
    /// ```
    ///
    /// Keys and values are passed verbatim from the Kubernetes object.
    /// Pod-template annotations are not included here.
    pub annotations: HashMap<String, String>,

    /// Pod template labels (from `spec.template.metadata.labels`).
    ///
    /// Useful for environment-specific behaviour — e.g. staging vs production
    /// is often expressed via a label rather than an annotation.
    pub labels: HashMap<String, String>,
}

/// What the plugin returns from its `apply` function.
///
/// Both fields default to empty. Return [`PluginResult::default()`] from
/// `apply` to inject nothing for this deployment.
///
/// ### Conflict resolution
///
/// Deckwatch applies the following rules when merging plugin results:
///
/// - **Env vars**: if a variable with the same `name` already exists on the
///   primary container (set by the operator), the plugin's value is silently
///   skipped. User-supplied values always win.
/// - **Sidecars**: if a container with the same `name` already exists in the
///   pod spec, the plugin's sidecar is silently skipped.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginResult {
    /// Env vars to inject into the **primary** (first) container of the pod.
    ///
    /// Names are matched case-sensitively. Any name that already exists on
    /// the container is skipped without error.
    #[serde(default)]
    pub env_vars: Vec<EnvVarSpec>,

    /// Sidecar containers to append to the pod's container list.
    ///
    /// A container whose `name` already exists in the pod is skipped without
    /// error. The primary container is always `containers[0]` and is never
    /// replaced by a plugin sidecar.
    #[serde(default)]
    pub sidecars: Vec<SidecarSpec>,
}

/// A single environment variable to inject into the primary container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarSpec {
    /// Environment variable name. Must be a valid POSIX env var name.
    pub name: String,

    /// Environment variable value. Injected as a static string literal
    /// (`value:` in the container spec, not `valueFrom:`).
    pub value: String,
}

/// A sidecar container to inject into the pod alongside the primary container.
///
/// Deckwatch translates this into a `k8s_openapi::api::core::v1::Container`
/// and appends it to `spec.template.spec.containers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarSpec {
    /// Container name. Must be unique within the pod and follow Kubernetes
    /// naming rules (lowercase alphanumeric + hyphens, max 63 chars).
    ///
    /// Deckwatch records injected sidecar names in the annotation
    /// `deckwatch.plugin-sidecar/<plugin-name>` for lifecycle tracking.
    pub name: String,

    /// Full image reference. Include the registry and tag explicitly —
    /// e.g. `registry.myorg.internal/mesh-agent:v2.1.0`. Avoid `latest`
    /// in production.
    pub image: String,

    /// Environment variables to set on this sidecar container specifically.
    /// These are distinct from the primary container's env vars.
    #[serde(default)]
    pub env: Vec<EnvVarSpec>,

    /// Container port to declare on the sidecar. Used for service discovery
    /// and readiness probes — does not affect network policy.
    #[serde(default)]
    pub port: Option<i32>,

    /// CPU request **and** limit (same value applied to both fields in the
    /// Kubernetes `ResourceRequirements`). Standard quantity format:
    /// `"50m"`, `"0.1"`, `"2"`.
    ///
    /// If `None`, no CPU resource constraints are set.
    #[serde(default)]
    pub cpu: Option<String>,

    /// Memory request **and** limit (same value applied to both fields).
    /// Standard quantity format: `"64Mi"`, `"1Gi"`.
    ///
    /// If `None`, no memory resource constraints are set.
    #[serde(default)]
    pub memory: Option<String>,
}
