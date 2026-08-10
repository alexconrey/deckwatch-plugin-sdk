//! # deckwatch-plugin-sdk
//!
//! Shared types for [deckwatch](https://github.com/alexconrey/deckwatch) plugins.
//!
//! A deckwatch plugin is a compiled WASM binary that deckwatch fetches from a
//! Git repository and executes for every deployment create/update. The plugin
//! receives a [`PluginContext`] describing the target deployment and returns a
//! [`PluginResult`] containing env vars, sidecars, and Kubernetes resources to apply.
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
//!         result.env_vars.push(EnvVarSpec::value("VAULT_ADDR", "https://vault.myorg.internal:8200"));
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

/// Context passed to a plugin's `apply` function.
///
/// Deckwatch serializes this as JSON and passes it as input to the WASM
/// `apply` function. All fields are populated from the Deployment object
/// **after** user-supplied changes are applied, so the plugin sees the
/// operator's final intent before any plugin injection happens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContext {
    /// Kubernetes namespace of the target deployment.
    pub namespace: String,

    /// Name of the target deployment.
    pub deployment_name: String,

    /// Deployment-level annotations.
    ///
    /// Use these as opt-in signals, e.g. `myorg.io/vault: "enabled"`.
    /// Keys and values are passed verbatim from the Kubernetes object.
    pub annotations: HashMap<String, String>,

    /// Pod template labels (from `spec.template.metadata.labels`).
    pub labels: HashMap<String, String>,
}

/// What the plugin returns from its `apply` function.
///
/// All fields default to empty / None. Return [`PluginResult::default()`] to
/// inject nothing for this deployment.
///
/// ### Conflict resolution
///
/// - **Env vars**: if a variable with the same `name` already exists on the
///   primary container, the plugin's value is silently skipped.
/// - **Sidecars**: if a container with the same `name` already exists in the
///   pod spec, the plugin's sidecar is silently skipped.
/// - **Kubernetes resources**: applied via server-side apply; conflicts are
///   resolved by the API server using field ownership rules.
/// - **Service account**: only applied when the pod's current
///   `serviceAccountName` is `None` or `"default"` — user-specified accounts
///   are never overwritten.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginResult {
    /// Env vars to inject into the primary (first) container.
    #[serde(default)]
    pub env_vars: Vec<EnvVarSpec>,

    /// Sidecar containers to append to the pod's container list.
    #[serde(default)]
    pub sidecars: Vec<SidecarSpec>,

    /// Arbitrary Kubernetes resources to apply to the cluster via server-side
    /// apply. Each element must be a valid Kubernetes object with `apiVersion`,
    /// `kind`, and `metadata.name`. Resources are applied after the deployment
    /// is committed; failures are logged but do not block the deployment.
    ///
    /// Use this for infrastructure CRDs that should be created alongside the
    /// deployment — RDS instances, IAM ServiceAccounts, snapshot schedules, etc.
    #[serde(default)]
    pub kubernetes_resources: Vec<serde_json::Value>,

    /// Kubernetes ServiceAccount name to bind to the pod template
    /// (`spec.template.spec.serviceAccountName`). Only applied when the
    /// current value is `None` or `"default"` — an existing non-default
    /// service account is never overwritten by a plugin.
    ///
    /// Combine with a `ServiceAccount` in `kubernetes_resources` so the
    /// account is created and bound in a single plugin invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account_name: Option<String>,
}

/// A single environment variable to inject into the primary container.
///
/// Exactly one of `value` or `value_from` should be set. If both are set,
/// `value_from` takes precedence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvVarSpec {
    /// Environment variable name.
    pub name: String,

    /// Static string value. Use [`EnvVarSpec::value`] to construct.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,

    /// Reference to a Kubernetes Secret or ConfigMap key. Use
    /// [`EnvVarSpec::secret_key_ref`] or [`EnvVarSpec::config_map_key_ref`]
    /// to construct. Takes precedence over `value` when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_from: Option<EnvVarSource>,
}

impl EnvVarSpec {
    /// Construct a static-value env var.
    pub fn value(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self { name: name.into(), value: value.into(), value_from: None }
    }

    /// Construct an env var sourced from a Kubernetes Secret key.
    pub fn secret_key_ref(name: impl Into<String>, secret: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: String::new(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeyRef { name: secret.into(), key: key.into() }),
                config_map_key_ref: None,
            }),
        }
    }

    /// Construct an env var sourced from a Kubernetes ConfigMap key.
    pub fn config_map_key_ref(name: impl Into<String>, config_map: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: String::new(),
            value_from: Some(EnvVarSource {
                secret_key_ref: None,
                config_map_key_ref: Some(ConfigMapKeyRef { name: config_map.into(), key: key.into() }),
            }),
        }
    }
}

/// The source of an env var's value — a Secret or ConfigMap key reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarSource {
    /// Pull the value from a Kubernetes Secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key_ref: Option<SecretKeyRef>,

    /// Pull the value from a Kubernetes ConfigMap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_map_key_ref: Option<ConfigMapKeyRef>,
}

/// Reference to a specific key within a Kubernetes Secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretKeyRef {
    /// Name of the Kubernetes Secret.
    pub name: String,
    /// Key within the Secret's `data` map.
    pub key: String,
}

/// Reference to a specific key within a Kubernetes ConfigMap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMapKeyRef {
    /// Name of the Kubernetes ConfigMap.
    pub name: String,
    /// Key within the ConfigMap's `data` map.
    pub key: String,
}

/// A sidecar container to inject into the pod alongside the primary container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarSpec {
    /// Container name. Must be unique within the pod.
    pub name: String,

    /// Full image reference, e.g. `registry.myorg.internal/agent:v2`.
    pub image: String,

    /// Environment variables for the sidecar container.
    #[serde(default)]
    pub env: Vec<EnvVarSpec>,

    /// Port to expose on the sidecar.
    #[serde(default)]
    pub port: Option<i32>,

    /// CPU request and limit (same value applied to both). e.g. `"50m"`.
    #[serde(default)]
    pub cpu: Option<String>,

    /// Memory request and limit (same value applied to both). e.g. `"64Mi"`.
    #[serde(default)]
    pub memory: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_result_default_has_no_service_account() {
        let r = PluginResult::default();
        assert!(r.service_account_name.is_none());
        assert!(r.env_vars.is_empty());
        assert!(r.sidecars.is_empty());
        assert!(r.kubernetes_resources.is_empty());
    }

    #[test]
    fn service_account_name_round_trips() {
        let r = PluginResult {
            service_account_name: Some("my-app-rds-sa".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PluginResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.service_account_name.as_deref(), Some("my-app-rds-sa"));
    }

    #[test]
    fn service_account_name_none_is_omitted_from_json() {
        let r = PluginResult::default();
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("service_account_name"));
    }

    #[test]
    fn env_var_constructors_work() {
        let static_var = EnvVarSpec::value("FOO", "bar");
        assert_eq!(static_var.name, "FOO");
        assert_eq!(static_var.value, "bar");
        assert!(static_var.value_from.is_none());

        let secret_var = EnvVarSpec::secret_key_ref("DB_PASS", "my-secret", "password");
        assert_eq!(secret_var.name, "DB_PASS");
        assert!(secret_var.value.is_empty());
        let vf = secret_var.value_from.as_ref().unwrap();
        let skr = vf.secret_key_ref.as_ref().unwrap();
        assert_eq!(skr.name, "my-secret");
        assert_eq!(skr.key, "password");
    }

    #[test]
    fn deserialize_missing_service_account_defaults_to_none() {
        // Ensure older plugin WASM that don't emit service_account_name still
        // deserialize cleanly — backwards compatibility.
        let json = r#"{"env_vars":[],"sidecars":[],"kubernetes_resources":[]}"#;
        let r: PluginResult = serde_json::from_str(json).unwrap();
        assert!(r.service_account_name.is_none());
    }
}
