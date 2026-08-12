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

/// The data type of a plugin configuration field.
///
/// Used by the deckwatch UI to render the appropriate form control for each
/// field in a plugin's `config_schema`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    /// Plain text input. Stored in plugin config as-is.
    String,
    /// Masked text input. Stored encrypted in `PluginConfig.config` using the
    /// same AES-256-GCM envelope as `DeckwatchSettings.credentials`.
    Secret,
    /// Checkbox. Stored as `"true"` or `"false"`.
    Bool,
    /// Dropdown. The field must have a non-empty `options` list.
    Select,
}

impl Default for ConfigFieldType {
    fn default() -> Self {
        Self::String
    }
}

/// A single field in a plugin's self-declared configuration schema.
///
/// Plugins export a `metadata()` function that returns a [`PluginMetadata`]
/// containing a `config_schema` vec. Deckwatch reads this at load time and
/// renders a dynamic settings form — no hardcoded knowledge of any specific
/// plugin is required in the deckwatch core.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigField {
    /// Machine-readable key. Must be a valid extism config key (alphanumeric + `_`).
    /// Matched against keys in `PluginConfig.config` and `inherit_env_keys`.
    pub key: String,
    /// Human-readable label for the settings form.
    pub label: String,
    /// Help text rendered below the field.
    #[serde(default)]
    pub description: String,
    /// The type of form control to render.
    pub field_type: ConfigFieldType,
    /// Default value rendered in the form when no saved value exists.
    /// For `secret` fields this is typically `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Whether the field must be non-empty before deckwatch accepts settings.
    #[serde(default)]
    pub required: bool,
    /// Allowed values for `select` fields. Ignored for other field types.
    #[serde(default)]
    pub options: Vec<String>,
    /// When `Some`, this field is populated from an environment variable rather
    /// than direct user input. The value is the env var name. The UI renders
    /// the field as read-only with a badge showing the source name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_source: Option<String>,
}

/// Static metadata returned by the `metadata()` export.
///
/// Deckwatch calls this once at plugin load time to build the dependency graph
/// and sort plugins before any `apply()` calls happen. Operators never need to
/// think about ordering — deckwatch resolves it automatically.
///
/// # Capability IDs
///
/// Well-known IDs for the AWS plugin ecosystem:
/// - `"aws:iam-role"` — an IAM role has been created for this workload
/// - `"aws:service-account"` — a ServiceAccount with IRSA annotation exists
/// - `"aws:rds-connection"` — an RDS instance has been provisioned
/// - `"aws:s3-bucket"` — an S3 bucket has been provisioned
///
/// Use namespaced IDs (`"domain:capability"`) to avoid collisions with
/// third-party plugins.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginMetadata {
    /// Plugin display name (for logs and UI).
    pub name: String,
    /// Plugin version string (e.g. from `env!("CARGO_PKG_VERSION")`).
    #[serde(default)]
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Capability IDs this plugin produces, available to downstream plugins
    /// via `ctx.plugin_outputs["<plugin-name>"]`.
    #[serde(default)]
    pub provides: Vec<String>,
    /// Capabilities that **must** be present — deckwatch skips this plugin
    /// with an error if none of the loaded plugins provides them.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Capabilities that improve behaviour if present but are not required —
    /// the plugin runs in reduced-functionality mode if unmet.
    #[serde(default)]
    pub optional_depends_on: Vec<String>,
    /// Schema for this plugin's operator-supplied configuration.
    ///
    /// Deckwatch reads this at load time and renders a dynamic settings form.
    /// Old plugins that do not export this field deserialize to an empty `Vec`.
    #[serde(default)]
    pub config_schema: Vec<ConfigField>,
}

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

    /// Outputs from plugins that have already run this invocation, keyed by
    /// plugin **name** (the `name` field from `PluginMetadata`, which is also
    /// the `name` field in `PluginConfig`).
    ///
    /// Example — reading the IAM role ARN produced by `deckwatch-plugin-aws`:
    /// ```rust,ignore
    /// if let Some(aws_out) = ctx.plugin_outputs.get("aws") {
    ///     if let Some(role_arn) = aws_out.get("role_arn") {
    ///         // use role_arn
    ///     }
    /// }
    /// ```
    #[serde(default)]
    pub plugin_outputs: HashMap<String, HashMap<String, String>>,
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

    /// Key-value data this plugin shares with plugins that run after it.
    /// Downstream plugins read these via `ctx.plugin_outputs["<this-plugin-name>"]`.
    ///
    /// Common outputs from the AWS plugin:
    /// - `"role_arn"` — the IAM role ARN created for this workload
    /// - `"service_account_name"` — the ServiceAccount name
    /// - `"rds_endpoint"` — the RDS instance endpoint (once available)
    /// - `"s3_bucket"` — the full bucket name (with prefix applied)
    #[serde(default)]
    pub outputs: HashMap<String, String>,

    /// Kubernetes ServiceAccount name to bind to the pod template
    /// (`spec.template.spec.serviceAccountName`). Only applied when the
    /// current value is `None` or `"default"` — an existing non-default
    /// service account is never overwritten by a plugin.
    ///
    /// Combine with a `ServiceAccount` in `kubernetes_resources` so the
    /// account is created and bound in a single plugin invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account_name: Option<String>,

    /// Non-fatal errors and warnings the plugin wants deckwatch to surface
    /// in its own structured logs.
    ///
    /// Use this instead of `extism_pdk::log!()` — extism's log pipe cannot
    /// be bridged into deckwatch's tracing subscriber without a global
    /// callback conflict. Errors listed here are logged at `ERROR` level by
    /// deckwatch after `apply()` returns; the deployment is not blocked.
    ///
    /// Plugin authors should use this for any recoverable failure that an
    /// operator needs to see: IAM errors, RDS provisioning failures, etc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
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
    fn plugin_metadata_defaults_are_empty() {
        let m = PluginMetadata::default();
        assert!(m.provides.is_empty());
        assert!(m.depends_on.is_empty());
        assert!(m.optional_depends_on.is_empty());
        assert!(m.config_schema.is_empty());
    }

    #[test]
    fn config_field_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConfigFieldType::Secret).unwrap(),
            "\"secret\""
        );
        assert_eq!(
            serde_json::to_string(&ConfigFieldType::Select).unwrap(),
            "\"select\""
        );
    }

    #[test]
    fn config_schema_round_trips() {
        let field = ConfigField {
            key: "AWS_REGION".into(),
            label: "AWS Region".into(),
            description: "The AWS region to use".into(),
            field_type: ConfigFieldType::String,
            default: Some("us-east-1".into()),
            required: true,
            options: vec![],
            env_source: Some("AWS_REGION".into()),
        };
        let json = serde_json::to_string(&field).unwrap();
        let back: ConfigField = serde_json::from_str(&json).unwrap();
        assert_eq!(back.key, "AWS_REGION");
        assert_eq!(back.field_type, ConfigFieldType::String);
        assert_eq!(back.default.as_deref(), Some("us-east-1"));
        assert!(back.required);
    }

    #[test]
    fn plugin_metadata_missing_config_schema_defaults_to_empty() {
        // Simulate an old plugin binary that does not include config_schema.
        let json = r#"{"name":"old-plugin","version":"0.1.0","description":"","provides":[],"depends_on":[],"optional_depends_on":[]}"#;
        let m: PluginMetadata = serde_json::from_str(json).unwrap();
        assert!(m.config_schema.is_empty());
    }

    #[test]
    fn outputs_round_trip() {
        let mut r = PluginResult::default();
        r.outputs.insert("role_arn".into(), "arn:aws:iam::123:role/app".into());
        let json = serde_json::to_string(&r).unwrap();
        let back: PluginResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.outputs.get("role_arn").unwrap(), "arn:aws:iam::123:role/app");
    }

    #[test]
    fn plugin_outputs_in_context_round_trip() {
        let mut ctx = PluginContext {
            namespace: "ns".into(),
            deployment_name: "app".into(),
            annotations: HashMap::new(),
            labels: HashMap::new(),
            plugin_outputs: HashMap::new(),
        };
        let mut aws_out = HashMap::new();
        aws_out.insert("role_arn".into(), "arn:aws:iam::123:role/app".into());
        ctx.plugin_outputs.insert("aws".into(), aws_out);

        let json = serde_json::to_string(&ctx).unwrap();
        let back: PluginContext = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.plugin_outputs["aws"]["role_arn"],
            "arn:aws:iam::123:role/app"
        );
    }

    #[test]
    fn plugin_context_missing_plugin_outputs_defaults_to_empty() {
        let json = r#"{"namespace":"ns","deployment_name":"app","annotations":{},"labels":{}}"#;
        let ctx: PluginContext = serde_json::from_str(json).unwrap();
        assert!(ctx.plugin_outputs.is_empty());
    }

    #[test]
    fn deserialize_missing_service_account_defaults_to_none() {
        let json = r#"{"env_vars":[],"sidecars":[],"kubernetes_resources":[]}"#;
        let r: PluginResult = serde_json::from_str(json).unwrap();
        assert!(r.service_account_name.is_none());
    }

    #[test]
    fn errors_absent_in_older_payload_deserializes_to_empty_vec() {
        // Deckwatch must handle plugin WASM built against an older SDK that
        // does not emit the `errors` field — deserialize to empty vec.
        let json = r#"{"env_vars":[],"sidecars":[],"kubernetes_resources":[]}"#;
        let r: PluginResult = serde_json::from_str(json).unwrap();
        assert!(r.errors.is_empty());
    }

    #[test]
    fn errors_omitted_from_json_when_empty() {
        let r = PluginResult::default();
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("errors"), "empty errors should be skipped in JSON");
    }

    #[test]
    fn errors_round_trip() {
        let mut r = PluginResult::default();
        r.errors.push("ensure_role failed: AccessDenied".into());
        let json = serde_json::to_string(&r).unwrap();
        let back: PluginResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.errors.len(), 1);
        assert!(back.errors[0].contains("AccessDenied"));
    }
}
