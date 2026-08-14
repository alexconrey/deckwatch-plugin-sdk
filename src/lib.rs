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

// ── Host functions ────────────────────────────────────────────────────────────

/// Returns the current UTC time as a Unix timestamp (seconds since epoch).
///
/// Provided by deckwatch as a host function (`extism:host/user::now`). Call
/// this instead of `std::time::SystemTime::now()` — the latter panics in
/// `wasm32-unknown-unknown` because the extism WASM sandbox has no system clock.
///
/// # Example
///
/// ```rust,ignore
/// let secs = deckwatch_plugin_sdk::now();
/// ```
#[cfg(target_arch = "wasm32")]
pub fn now() -> u64 {
    #[link(wasm_import_module = "extism:host/user")]
    unsafe extern "C" {
        fn now() -> i64;
    }
    unsafe { now() as u64 }
}

/// Structured ServiceAccount request returned from a plugin's `apply()` export.
///
/// Preferred over including a raw `ServiceAccount` manifest in
/// `kubernetes_resources` — deckwatch handles create-or-patch with retry
/// semantics and surfaces the result in the UI and audit log.
///
/// Mirrors `deckwatch::plugins::WantsServiceAccount` — kept in sync manually.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WantsServiceAccount {
    /// Name of the Kubernetes ServiceAccount to create or patch.
    pub name: String,
    /// IRSA role ARN stamped on the `eks.amazonaws.com/role-arn` annotation.
    /// Leave empty for a plain ServiceAccount with no IAM binding.
    #[serde(default)]
    pub irsa_role_arn: String,
}

/// Provisioning lifecycle status of a plugin-managed infrastructure resource.
///
/// Returned by the plugin's `provision()` export to signal whether the resource
/// is ready, still being created, or has failed. Deckwatch uses this to drive
/// the polling loop in `reconcile_pending_resources`:
///
/// - `Pending`   → re-poll next cycle (respecting `retry_after_secs`)
/// - `Available` → resource is fully provisioned; stop polling
/// - `Failed`    → surface in the UI and stop polling
///
/// Old plugins that do not emit this field default to `Pending` so the
/// existing magic-string heuristics (`is_pending`) continue to work.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum ProvisioningStatus {
    /// Resource creation is still in progress.
    #[default]
    Pending,
    /// Resource is fully provisioned and available.
    Available,
    /// Resource provisioning failed — check `errors` for details.
    Failed,
}

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
    /// Infrastructure resources this plugin can provision on behalf of an
    /// application (e.g. RDS databases, S3 buckets). Each entry renders as
    /// a card on the Application detail page with a provisioning form.
    ///
    /// Old plugins that do not export this field deserialize to an empty `Vec`.
    #[serde(default)]
    pub resources: Vec<PluginResource>,
}

/// A provisionable infrastructure resource declared by a plugin.
///
/// Plugins include these in [`PluginMetadata::resources`] to describe the
/// infrastructure they can create on behalf of an application. Deckwatch
/// renders a card per resource on the Application detail page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginResource {
    /// Machine-readable identifier, unique within this plugin. e.g. `"rds"`, `"s3"`.
    pub id: String,
    /// Button/tab label shown in the deckwatch UI. e.g. `"RDS Database"`.
    pub label: String,
    /// Optional MDI icon name. e.g. `"mdi-database"`.
    #[serde(default)]
    pub icon: String,
    /// Human-readable description shown in the provisioning dialog.
    #[serde(default)]
    pub description: String,
    /// If true, only one instance of this resource may exist per application.
    /// The "+" button is hidden once provisioned.
    #[serde(default)]
    pub singleton: bool,
    /// Form fields the operator fills out when provisioning.
    /// Reuses [`ConfigField`] — same type/label/required/default semantics.
    #[serde(default)]
    pub fields: Vec<ConfigField>,
    /// Env var keys this resource will emit into application state once provisioned.
    /// e.g. `["DB_HOST", "DB_PORT", "DB_NAME", "DB_IAM_AUTH"]`.
    /// Deckwatch uses this to display a "provides these env vars" preview in the UI.
    #[serde(default)]
    pub output_keys: Vec<String>,
}

/// Request sent to the plugin's `provision()` WASM export.
///
/// Built by deckwatch when an operator clicks "Add {resource.label}" and
/// submits the provisioning form on the Application detail page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceProvisionRequest {
    /// Name of the application being provisioned for.
    pub application_name: String,
    /// Kubernetes namespace the application lives in.
    pub namespace: String,
    /// Plugin-declared resource type id (matches [`PluginResource::id`]).
    pub resource_id: String,
    /// User-submitted form values, keyed by [`ConfigField::key`].
    pub fields: HashMap<String, String>,
}

/// Result returned from the plugin's `provision()` WASM export.
///
/// Deckwatch persists `state` to the `application_plugin_resources` table
/// and injects its key-value pairs as env vars into all deployments and
/// cronjobs in the application on the next reconcile cycle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceProvisionResult {
    /// Persisted at application level. Keys become env vars injected into
    /// all deployments and cronjobs in the application.
    #[serde(default)]
    pub state: HashMap<String, String>,
    /// Annotations to stamp on every deployment in the application.
    ///
    /// Keys should be namespaced (e.g. `"deckwatch.io/aws-s3-bucket"`).
    /// Deckwatch persists these alongside `state` and patches
    /// `metadata.annotations` on all application deployments during the
    /// next reconciler cycle — making provisioned resources visible via
    /// `kubectl get deployment -o yaml` without querying the deckwatch DB.
    ///
    /// Old plugins that do not emit this field deserialize to an empty map.
    #[serde(default)]
    pub deployment_annotations: HashMap<String, String>,
    /// Sidecar containers to inject into every deployment in the application.
    ///
    /// Deckwatch persists these alongside `state` and patches
    /// `spec.template.spec.containers` on all application deployments during
    /// the next reconciler cycle. The container `name` field is used as the
    /// key for server-side apply — adding the same sidecar twice is idempotent,
    /// and removing the resource causes the sidecar to be removed on the next
    /// reconcile.
    ///
    /// Use this to inject process sidecars (Redis, PostgreSQL, Fluent Bit, etc.)
    /// that should run alongside every workload in the application.
    /// Old plugins that do not emit this field deserialize to an empty vec.
    #[serde(default)]
    pub sidecars: Vec<SidecarSpec>,
    /// Any Kubernetes resources to apply (e.g. a ServiceAccount with IRSA annotation).
    #[serde(default)]
    pub kubernetes_resources: Vec<serde_json::Value>,
    /// Errors to surface in deckwatch logs and UI.
    #[serde(default)]
    pub errors: Vec<String>,
    /// Non-fatal warnings to surface in deckwatch logs.
    ///
    /// Logged at `WARN` level after `provision()` returns. Use this for
    /// conditions that are not fatal but worth operator attention (e.g. a
    /// resource was created but a non-critical tag is missing).
    /// Old plugins deserialize to an empty vec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Provisioning lifecycle status.
    ///
    /// `Pending` means the resource is still being created and deckwatch
    /// should re-poll. `Available` means fully provisioned. `Failed` surfaces
    /// the failure in the UI. Old plugins that do not emit this field
    /// deserialize to `Pending` (the existing retry behaviour is preserved).
    #[serde(default)]
    pub status: ProvisioningStatus,
    /// Minimum seconds to wait before deckwatch re-polls this resource.
    ///
    /// Plugins should return this for long-running provisioning operations
    /// (e.g. RDS creation takes several minutes). Deckwatch skips re-polling
    /// the resource until `updated_at + retry_after_secs > now`.
    /// `None` means poll every cycle (the existing behaviour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

/// Input to a plugin's `deprovision()` WASM export.
///
/// Deckwatch calls this before removing the resource record from the DB,
/// passing the persisted state and original form fields so the plugin knows
/// what to clean up. The plugin decides whether to actually delete the cloud
/// resource — for S3 buckets with data this is typically a no-op with a warning;
/// for RDS the plugin can take a final snapshot then delete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDeprovisionRequest {
    pub application_name: String,
    pub namespace: String,
    pub resource_id: String,
    /// The state map persisted when the resource was provisioned
    /// (e.g. `{"S3_BUCKET": "myorg-uploads", "DB_HOST": "..."}`).
    #[serde(default)]
    pub state: HashMap<String, String>,
    /// The original operator-submitted form values.
    #[serde(default)]
    pub fields: HashMap<String, String>,
}

/// What the plugin returns from its `deprovision()` export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceDeprovisionResult {
    /// Errors to surface in deckwatch logs and UI. Does not block DB record removal.
    #[serde(default)]
    pub errors: Vec<String>,
    /// Human-readable message shown to the operator in the UI after deprovisioning.
    /// Use this to communicate what was and was not cleaned up.
    #[serde(default)]
    pub message: String,
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

    /// Provisioned infrastructure resources for this application, keyed by
    /// `"{plugin_name}/{resource_id}"`. Each value is the state map that was
    /// persisted when the resource was provisioned (e.g.
    /// `{"DB_HOST": "mydb.us-east-1.rds.amazonaws.com", "DB_PORT": "5432"}`).
    ///
    /// Plugins read this to avoid redundant cloud API calls on every reconcile
    /// cycle. If `DB_HOST` is already present here the plugin can skip the RDS
    /// `DescribeDBInstances` call and return the cached value directly.
    ///
    /// Example:
    /// ```rust,ignore
    /// if let Some(rds) = ctx.provisioned_resources.get("aws/rds") {
    ///     if let Some(host) = rds.get("DB_HOST") {
    ///         // use cached host — no AWS API call needed
    ///     }
    /// }
    /// ```
    ///
    /// Old hosts that do not populate this field deserialize to an empty map.
    #[serde(default)]
    pub provisioned_resources: HashMap<String, HashMap<String, String>>,
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

    /// Structured ServiceAccount request — preferred over emitting a raw SA
    /// in `kubernetes_resources`. Deckwatch handles create/patch with retry
    /// semantics and surfaces the result in the UI and audit log.
    ///
    /// Set `irsa_role_arn` to the IAM role ARN when using IRSA; leave it
    /// empty for a plain ServiceAccount with no IAM binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wants_service_account: Option<WantsServiceAccount>,

    /// Annotations to stamp on the deployment's `metadata.annotations`.
    ///
    /// Keys should be namespaced (e.g. `"myorg.io/vault-injected": "true"`).
    /// Deckwatch merges these into the deployment patch alongside the
    /// plugin-env tracking annotations. Old plugins deserialize to an empty map.
    #[serde(default)]
    pub deployment_annotations: HashMap<String, String>,

    /// Labels to stamp on the deployment's `metadata.labels`.
    ///
    /// Keys should be namespaced (e.g. `"myorg.io/managed-by-vault": "true"`).
    /// Deckwatch includes these in the SSA patch `metadata.labels`.
    /// Old plugins deserialize to an empty map.
    #[serde(default)]
    pub deployment_labels: HashMap<String, String>,

    /// Non-fatal errors the plugin wants deckwatch to surface in its logs.
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

    /// Non-fatal warnings the plugin wants deckwatch to surface in its logs.
    ///
    /// Logged at `WARN` level after `apply()` returns; the deployment is not
    /// blocked. Use this for degraded-but-functional conditions (e.g. a
    /// non-critical config key is missing, or a cloud resource is still
    /// warming up). Old plugins deserialize to an empty vec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
    fn plugin_metadata_missing_resources_defaults_to_empty_vec() {
        // Old plugin binary without resources field must still deserialize cleanly.
        let json = r#"{"name":"old-plugin","version":"0.5.1","description":"","provides":[],"depends_on":[],"optional_depends_on":[],"config_schema":[]}"#;
        let m: PluginMetadata = serde_json::from_str(json).unwrap();
        assert!(m.resources.is_empty(), "resources should default to [] when absent");
    }

    #[test]
    fn resource_provision_result_missing_errors_defaults_to_empty_vec() {
        // Plugins that return state but no errors field must deserialize cleanly.
        let json = r#"{"state":{"DB_HOST":"mydb.us-east-1.rds.amazonaws.com"},"kubernetes_resources":[]}"#;
        let r: ResourceProvisionResult = serde_json::from_str(json).unwrap();
        assert!(r.errors.is_empty(), "errors should default to [] when absent");
        assert_eq!(r.state.get("DB_HOST").map(String::as_str), Some("mydb.us-east-1.rds.amazonaws.com"));
    }

    #[test]
    fn plugin_resource_round_trips() {
        let resource = PluginResource {
            id: "rds".into(),
            label: "RDS Database".into(),
            icon: "mdi-database".into(),
            description: "Provision an Amazon RDS instance".into(),
            singleton: true,
            fields: vec![ConfigField {
                key: "db_name".into(),
                label: "Database Name".into(),
                description: "Name for the initial database".into(),
                field_type: ConfigFieldType::String,
                default: Some("app".into()),
                required: true,
                options: vec![],
                env_source: None,
            }],
            output_keys: vec!["DB_HOST".into(), "DB_PORT".into()],
        };
        let json = serde_json::to_string(&resource).unwrap();
        let back: PluginResource = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "rds");
        assert_eq!(back.singleton, true);
        assert_eq!(back.output_keys, vec!["DB_HOST", "DB_PORT"]);
        assert_eq!(back.fields[0].key, "db_name");
    }

    #[test]
    fn resource_provision_request_round_trips() {
        let mut fields = HashMap::new();
        fields.insert("db_name".into(), "myapp".into());
        let req = ResourceProvisionRequest {
            application_name: "myapp".into(),
            namespace: "production".into(),
            resource_id: "rds".into(),
            fields,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ResourceProvisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.application_name, "myapp");
        assert_eq!(back.resource_id, "rds");
        assert_eq!(back.fields.get("db_name").map(String::as_str), Some("myapp"));
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
