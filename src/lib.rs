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

use std::collections::{BTreeMap, HashMap};

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
/// Returned in [`ResourceProvisionResult::status`]. Serialized as the variant
/// name (`"Pending"`, `"Available"`, `"Failed"`), not snake_case.
///
/// In deckwatch 0.6.7 this does **not** decide whether a resource is re-polled:
/// the host re-polls resources whose stored `state` looks pending (a `*_STATUS`
/// key with an in-progress value, or `DB_ENGINE`/`DB_STATUS` without
/// `DB_HOST`). On a re-poll, `Available` and `Failed` results are always
/// stored, and a `Pending` result only if it adds `state` keys or no longer
/// looks pending. See the README's "Provisioning" section.
///
/// Old plugins that do not emit this field deserialize to `Pending`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum ProvisioningStatus {
    /// Resource creation is still in progress.
    #[default]
    Pending,
    /// Resource is fully provisioned and available.
    Available,
    /// Resource provisioning failed. Explain why in `errors`.
    Failed,
}

/// The data type of a plugin configuration field.
///
/// Used by the deckwatch UI to render the appropriate form control for each
/// field in a plugin's `config_schema`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    /// Plain text input. Stored in plugin config as-is.
    #[default]
    String,
    /// Masked text input. Values saved through the settings form are stored
    /// AES-256-GCM encrypted when deckwatch has `DECKWATCH_ENCRYPTION_KEY`
    /// set. deckwatch 0.6.7 does not decrypt them before calling the plugin
    /// (see the README's "Known host issues").
    Secret,
    /// Checkbox. Stored as `"true"` or `"false"`.
    Bool,
    /// A set of key/value pairs, edited as one row per pair (e.g. resource
    /// tags). Stored in plugin config as a JSON object of string keys to string
    /// values, e.g. `{"Team":"platform","CostCenter":"1234"}`. An empty value
    /// means no pairs. Read it with [`parse_key_value_field`].
    KeyValue,
    /// Dropdown. The field must have a non-empty `options` list.
    Select,
    /// A field type this version of the SDK doesn't know.
    ///
    /// Deserializing an unrecognised `field_type` yields this variant instead of
    /// an error, so a host built against an older SDK can still load a newer
    /// plugin's metadata. Without it, one new field type makes the whole
    /// `metadata()` fail to parse, and the host loses every config field and
    /// resource the plugin declares. Hosts should render it as a plain text input.
    #[serde(other)]
    Unknown,
}

/// A single field in a plugin's self-declared configuration schema.
///
/// Plugins export a `metadata()` function that returns a [`PluginMetadata`]
/// containing a `config_schema` vec. Deckwatch reads this at load time and
/// renders a dynamic settings form — no hardcoded knowledge of any specific
/// plugin is required in the deckwatch core.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigField {
    /// Config key. The saved value reaches the plugin under this key
    /// (`extism_pdk::config::get(key)`). Use letters, digits and `_`.
    pub key: String,
    /// Human-readable label for the settings form.
    pub label: String,
    /// Help text rendered below the field.
    #[serde(default)]
    pub description: String,
    /// The type of form control to render.
    pub field_type: ConfigFieldType,
    /// Value pre-filled in the form when nothing is saved. The host does not
    /// pass it to the plugin, so treat a missing key as "use the default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// The settings form refuses to save without a value. Enforced only in
    /// the UI; the host API does not check it.
    #[serde(default)]
    pub required: bool,
    /// Allowed values for `select` fields. Ignored for other field types.
    #[serde(default)]
    pub options: Vec<String>,
    /// Display only: the UI renders the field read-only with a badge naming
    /// this environment variable. The host does not read it. To receive the
    /// value, the operator lists the variable in `inherit_env_keys` (the
    /// config key is then the variable name) or `inherit_env_file_keys`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_source: Option<String>,
}

/// Parse the stored value of a [`ConfigFieldType::KeyValue`] field.
///
/// The value is a JSON object of string keys to string values. An empty or
/// whitespace-only value means no pairs. Anything else (invalid JSON, an array,
/// non-string values) is an error, so plugins can report a misconfiguration
/// instead of silently ignoring it.
///
/// ```
/// use deckwatch_plugin_sdk::parse_key_value_field;
///
/// let pairs = parse_key_value_field(r#"{"Team":"platform","CostCenter":"1234"}"#).unwrap();
/// assert_eq!(pairs.get("Team").map(String::as_str), Some("platform"));
/// assert!(parse_key_value_field("").unwrap().is_empty());
/// assert!(parse_key_value_field("Team=platform").is_err());
/// ```
pub fn parse_key_value_field(raw: &str) -> Result<BTreeMap<String, String>, String> {
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("not a JSON object: {e}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "expected a JSON object of string keys to string values".to_string())?;
    object
        .iter()
        .map(|(k, v)| match v {
            serde_json::Value::String(s) => Ok((k.clone(), s.clone())),
            other => Err(format!("value for {k:?} must be a string, got {other}")),
        })
        .collect()
}

/// A field that an operator fills in to tune MCP guidance for a specific plugin.
///
/// Plugins declare these in [`PluginMetadata::mcp_tuning_fields`] to describe
/// org-specific conventions (naming schemes, preferred instance classes, etc.)
/// that the AI agent should follow. For each field with a non-empty saved
/// value, deckwatch appends a line to the MCP `initialize` instructions, in a
/// block per plugin:
///
/// ```text
/// [Plugin: AWS]
/// RDS Naming Convention: k2-{namespace}-{app}
/// S3 Bucket Prefix: k2-
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpTuningField {
    /// Machine-readable key (alphanumeric + `_`). Used as the map key in
    /// `PluginConfig.mcp_tuning` when an admin saves a value in Settings.
    pub key: String,
    /// Human-readable label for the MCP Tuning settings form.
    pub label: String,
    /// Help text rendered below the field in the settings form.
    pub description: String,
    /// Example value shown as the input placeholder when no value is saved.
    #[serde(default)]
    pub placeholder: String,
    /// Value pre-filled in the form. `None` means no pre-fill. Not used
    /// otherwise: unsaved fields are left out of the instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Static metadata returned by the `metadata()` export.
///
/// Deckwatch calls this once at plugin load time. The export is optional: if
/// it is missing or fails, deckwatch uses `PluginMetadata::default()` with
/// `name` set to the plugin's configured name. Before each run, deckwatch
/// orders plugins by `provides`, `depends_on` and `optional_depends_on`.
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
    /// Display name for logs, the UI and the MCP tuning block.
    pub name: String,
    /// Plugin version string (e.g. from `env!("CARGO_PKG_VERSION")`).
    /// Informational; the host never checks it.
    #[serde(default)]
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Capability IDs this plugin provides. Used only for ordering and for
    /// other plugins' `depends_on` checks. (Data for later plugins goes in
    /// [`PluginResult::outputs`], keyed by plugin name, not capability.)
    #[serde(default)]
    pub provides: Vec<String>,
    /// Capabilities that **must** be provided by some loaded plugin. If any
    /// of them is missing, deckwatch skips this plugin and logs an error.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Capabilities that, when provided, must run before this plugin. They
    /// only affect ordering; an unmet one is not an error.
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
    /// Operator-facing fields for MCP-level guidance tuning.
    ///
    /// Deckwatch reads these at load time to render a "Plugin MCP Tuning"
    /// subsection in Settings. Saved values are appended to every MCP
    /// `initialize` response as a `[Plugin: {name}]` block.
    ///
    /// Old plugins that do not export this field deserialize to an empty `Vec`.
    #[serde(default)]
    pub mcp_tuning_fields: Vec<McpTuningField>,
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
    /// If true, only one instance of this resource may exist per application:
    /// the UI hides the "+" button and the host refuses a second one.
    #[serde(default)]
    pub singleton: bool,
    /// Form fields the operator fills out when provisioning.
    /// Reuses [`ConfigField`] — same type/label/required/default semantics.
    #[serde(default)]
    pub fields: Vec<ConfigField>,
    /// The [`ResourceProvisionResult::state`] keys this resource will produce,
    /// e.g. `["DB_HOST", "DB_PORT", "DB_NAME", "DB_IAM_AUTH"]`. Shown as a
    /// "provides these env vars" preview in the UI; not enforced.
    #[serde(default)]
    pub output_keys: Vec<String>,
}

/// Request sent to the plugin's `provision()` WASM export.
///
/// Built by deckwatch when an operator submits the provisioning form (or MCP
/// `provision_resource`), on refresh, and on each background re-poll of a
/// pending resource. On refresh and re-poll, `fields` holds the values stored
/// at provisioning time.
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
/// Deckwatch stores `state`, `deployment_annotations` and `sidecars` with the
/// resource and applies them to the application's workloads on its reconcile
/// loop (every 10 s in deckwatch 0.6.7).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceProvisionResult {
    /// Stored with the resource. Each key becomes a plain env var in the
    /// first container of every Deployment and CronJob labelled
    /// `deckwatch.io/application=<application>`. The map is passed back in
    /// [`ResourceDeprovisionRequest::state`] and
    /// [`PluginContext::provisioned_resources`].
    #[serde(default)]
    pub state: HashMap<String, String>,
    /// Annotations to stamp on every Deployment (not CronJob) in the
    /// application.
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
    /// Deckwatch stores these alongside `state` and server-side applies them
    /// to `spec.template.spec.containers` of the application's Deployments and
    /// CronJobs on each reconcile. The container `name` is the merge key, so
    /// applying the same sidecar twice is idempotent. In deckwatch 0.6.7 the
    /// sidecars' env vars lose `value_from` on this path, and deleting the
    /// application's last resource does not remove them.
    ///
    /// Use this to inject process sidecars (Redis, PostgreSQL, Fluent Bit, etc.)
    /// that should run alongside every workload in the application.
    /// Old plugins that do not emit this field deserialize to an empty vec.
    #[serde(default)]
    pub sidecars: Vec<SidecarSpec>,
    /// Kubernetes objects to server-side apply (e.g. a ServiceAccount with an
    /// IRSA annotation), with the same rules as
    /// [`PluginResult::kubernetes_resources`].
    #[serde(default)]
    pub kubernetes_resources: Vec<serde_json::Value>,
    /// Logged at `ERROR` after `provision()` returns. Provisioning still
    /// succeeds.
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
    /// Provisioning lifecycle status. See [`ProvisioningStatus`] for what
    /// deckwatch 0.6.7 actually does with it (it does not, by itself, cause
    /// or stop re-polling). Old plugins that do not emit this field
    /// deserialize to `Pending`.
    #[serde(default)]
    pub status: ProvisioningStatus,
    /// Minimum seconds to wait before deckwatch re-polls this resource.
    ///
    /// Return this for long-running operations (e.g. RDS creation takes
    /// several minutes). The background re-poll skips the resource while
    /// `updated_at + retry_after_secs > now`. `None` means re-poll every
    /// cycle. Ignored on submit and refresh; a refresh clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

/// Input to a plugin's `deprovision()` WASM export.
///
/// Deckwatch calls this when an operator deletes the resource, before
/// removing its record, passing the stored state and form fields so the
/// plugin knows what to clean up. The record is removed whatever the outcome,
/// and a missing export counts as success. Deleting a Deployment or an
/// application does not call `deprovision()`. The plugin decides whether to actually delete the cloud
/// resource — for S3 buckets with data this is typically a no-op with a warning;
/// for RDS the plugin can take a final snapshot then delete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDeprovisionRequest {
    /// Name of the application the resource belongs to.
    pub application_name: String,
    /// The application's Kubernetes namespace.
    pub namespace: String,
    /// The resource's [`PluginResource::id`].
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
    /// Logged at `ERROR`. Does not block removal of the resource record.
    #[serde(default)]
    pub errors: Vec<String>,
    /// What was and was not cleaned up. deckwatch 0.6.7 logs it at `INFO`;
    /// it is not shown in the UI.
    #[serde(default)]
    pub message: String,
    /// Kubernetes objects for deckwatch to delete once `deprovision()` returns.
    ///
    /// Use this for objects deckwatch created on the plugin's behalf (through
    /// `kubernetes_resources` or `wants_service_account`) that the plugin
    /// can't delete itself, such as a ServiceAccount or a one-off Job.
    ///
    /// Deckwatch only deletes in the application's namespace (a
    /// [`KubernetesObjectRef`] has no namespace) and only the kinds it allows:
    /// `v1` `ServiceAccount` and `batch/v1` `Job`. Jobs are deleted together
    /// with their pods, and before ServiceAccounts. A ServiceAccount that a
    /// Deployment, CronJob or Job in the namespace still uses is kept, because
    /// deleting it would stop new pods from being created. An object that is
    /// already gone counts as deleted. Refusals and failures are logged and
    /// audited, and never block removal of the resource record.
    ///
    /// Old plugins that do not emit this field deserialize to an empty vec, and
    /// an empty vec is left out of the JSON. Deckwatch versions that predate
    /// this field ignore it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delete_kubernetes_resources: Vec<KubernetesObjectRef>,
}

/// A Kubernetes object in the application's namespace, identified by API
/// version, kind and name.
///
/// Used in [`ResourceDeprovisionResult::delete_kubernetes_resources`]. Use
/// [`KubernetesObjectRef::service_account`] or [`KubernetesObjectRef::job`] to
/// construct the kinds deckwatch deletes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesObjectRef {
    /// API version, e.g. `"v1"` or `"batch/v1"`.
    pub api_version: String,
    /// Object kind, e.g. `"ServiceAccount"` or `"Job"`.
    pub kind: String,
    /// Object name.
    pub name: String,
}

impl KubernetesObjectRef {
    /// Reference an object by API version, kind and name.
    pub fn new(
        api_version: impl Into<String>,
        kind: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            api_version: api_version.into(),
            kind: kind.into(),
            name: name.into(),
        }
    }

    /// Reference a `v1` `ServiceAccount`.
    pub fn service_account(name: impl Into<String>) -> Self {
        Self::new("v1", "ServiceAccount", name)
    }

    /// Reference a `batch/v1` `Job`.
    pub fn job(name: impl Into<String>) -> Self {
        Self::new("batch/v1", "Job", name)
    }
}

/// Context passed to a plugin's `apply` function.
///
/// Deckwatch serializes this as JSON and passes it as input to the WASM
/// `apply` function. On Deployment create and update, the fields reflect the
/// Deployment **after** the operator's changes and before any plugin runs. On
/// the reconcile loop they come from the live Deployment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContext {
    /// Kubernetes namespace of the target deployment.
    pub namespace: String,

    /// Name of the target deployment.
    pub deployment_name: String,

    /// The Deployment's `metadata.annotations` (not the pod template's).
    ///
    /// Use these as opt-in signals, e.g. `myorg.io/vault: "enabled"`.
    /// Keys and values are passed verbatim from the Kubernetes object.
    pub annotations: HashMap<String, String>,

    /// Pod template labels (from `spec.template.metadata.labels`).
    pub labels: HashMap<String, String>,

    /// Outputs from plugins that have already run this invocation, keyed by
    /// plugin **name** (the `name` field from `PluginMetadata`, which is also
    /// the `name` field in `PluginConfig`). Empty on the reconcile loop,
    /// where plugin outputs are not chained.
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
    /// Filled only on the reconcile loop; empty on Deployment create and
    /// update. Old hosts that do not populate this field deserialize to an
    /// empty map.
    #[serde(default)]
    pub provisioned_resources: HashMap<String, HashMap<String, String>>,
}

/// What the plugin returns from its `apply` function.
///
/// All fields default to empty / None. Return [`PluginResult::default()`] to
/// inject nothing for this deployment.
///
/// Which fields deckwatch uses depends on the path: on Deployment create and
/// update it uses everything except `deployment_annotations` and
/// `deployment_labels`; on the reconcile loop it uses only those two (plus
/// `errors` and `warnings`). See the README's "Host behaviour" section.
///
/// ### Conflict resolution (create and update)
///
/// - **Env vars**: if a variable with the same `name` already exists on the
///   primary container, the plugin's value is silently skipped.
/// - **Sidecars**: if a container with the same `name` already exists in the
///   pod spec, the plugin's sidecar is silently skipped.
/// - **Kubernetes resources**: applied via server-side apply; conflicts are
///   resolved by the API server using field ownership rules.
/// - **Service account**: only applied when the pod's current
///   `serviceAccountName` is unset or `"default"`. User-specified accounts
///   are never overwritten; between plugins, the first one wins.
/// - **Removal**: none. Things a plugin stops returning, or injected by a
///   plugin that is later disabled, stay in the Deployment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginResult {
    /// Env vars to inject into the primary (first) container.
    #[serde(default)]
    pub env_vars: Vec<EnvVarSpec>,

    /// Sidecar containers to append to the pod's container list.
    #[serde(default)]
    pub sidecars: Vec<SidecarSpec>,

    /// Arbitrary Kubernetes objects to server-side apply (field manager
    /// `deckwatch-plugin`, forced). Each needs `apiVersion`, `kind` and
    /// `metadata.name`, plus `metadata.namespace` for namespaced kinds: the
    /// host does not fill it in. They are applied after the Deployment is
    /// saved; failures are logged and do not block the Deployment.
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
    /// deckwatch 0.6.7 applies these only on the reconcile loop (server-side
    /// apply), which only covers applications with a provisioned resource;
    /// Deployment create and update ignore them. Old plugins deserialize to an
    /// empty map.
    #[serde(default)]
    pub deployment_annotations: HashMap<String, String>,

    /// Labels to stamp on the deployment's `metadata.labels`.
    ///
    /// Keys should be namespaced (e.g. `"myorg.io/managed-by-vault": "true"`).
    /// Applied under the same conditions as `deployment_annotations`.
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

/// An environment variable, for the primary container
/// ([`PluginResult::env_vars`]) or a sidecar ([`SidecarSpec::env`]).
///
/// Exactly one of `value` or `value_from` should be set. If both are set,
/// `value_from` takes precedence. deckwatch emits references with
/// `optional: false`, so the pod won't start if the key is missing.
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
        Self {
            name: name.into(),
            value: value.into(),
            value_from: None,
        }
    }

    /// Construct an env var sourced from a Kubernetes Secret key.
    pub fn secret_key_ref(
        name: impl Into<String>,
        secret: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: String::new(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeyRef {
                    name: secret.into(),
                    key: key.into(),
                }),
                config_map_key_ref: None,
            }),
        }
    }

    /// Construct an env var sourced from a Kubernetes ConfigMap key.
    pub fn config_map_key_ref(
        name: impl Into<String>,
        config_map: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: String::new(),
            value_from: Some(EnvVarSource {
                secret_key_ref: None,
                config_map_key_ref: Some(ConfigMapKeyRef {
                    name: config_map.into(),
                    key: key.into(),
                }),
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

    /// Port to expose on the sidecar. Becomes one `containerPort` with no
    /// name and the default protocol.
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
    fn key_value_field_type_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConfigFieldType::KeyValue).unwrap(),
            "\"key_value\""
        );
        let back: ConfigFieldType = serde_json::from_str("\"key_value\"").unwrap();
        assert_eq!(back, ConfigFieldType::KeyValue);
    }

    // A field type the host doesn't know must not make the plugin's whole
    // metadata unparseable (the host would lose every field and resource).
    #[test]
    fn unrecognised_field_type_deserializes_as_unknown() {
        let back: ConfigFieldType = serde_json::from_str("\"color_picker\"").unwrap();
        assert_eq!(back, ConfigFieldType::Unknown);

        let meta: PluginMetadata = serde_json::from_str(
            r#"{"name":"p","config_schema":[
                {"key":"A","label":"A","field_type":"string"},
                {"key":"B","label":"B","field_type":"some_future_type"}
            ]}"#,
        )
        .expect("metadata with a future field type must still parse");
        assert_eq!(meta.config_schema.len(), 2);
        assert_eq!(meta.config_schema[0].field_type, ConfigFieldType::String);
        assert_eq!(meta.config_schema[1].field_type, ConfigFieldType::Unknown);
    }

    #[test]
    fn parse_key_value_field_accepts_objects_and_empty() {
        let pairs = parse_key_value_field(r#"{"Team":"platform","CostCenter":"1234"}"#).unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs["Team"], "platform");
        assert_eq!(pairs["CostCenter"], "1234");
        assert!(parse_key_value_field("").unwrap().is_empty());
        assert!(parse_key_value_field("   ").unwrap().is_empty());
        assert!(parse_key_value_field("{}").unwrap().is_empty());
    }

    #[test]
    fn parse_key_value_field_rejects_non_objects_and_non_strings() {
        for bad in [
            "Team=platform",
            r#"["Team","platform"]"#,
            r#"{"Count":3}"#,
            r#"{"Nested":{"a":"b"}}"#,
            "{not json",
        ] {
            assert!(
                parse_key_value_field(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
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
        assert!(
            m.resources.is_empty(),
            "resources should default to [] when absent"
        );
    }

    #[test]
    fn plugin_metadata_missing_mcp_tuning_fields_defaults_to_empty() {
        // Old plugin binary without mcp_tuning_fields must still deserialize cleanly.
        let json = r#"{"name":"old-plugin","version":"0.5.1","description":"","provides":[],"depends_on":[],"optional_depends_on":[]}"#;
        let m: PluginMetadata = serde_json::from_str(json).unwrap();
        assert!(
            m.mcp_tuning_fields.is_empty(),
            "mcp_tuning_fields should default to [] when absent"
        );
    }

    #[test]
    fn mcp_tuning_field_round_trips() {
        let field = McpTuningField {
            key: "rds_naming_convention".into(),
            label: "RDS Naming Convention".into(),
            description: "Convention for naming RDS instances.".into(),
            placeholder: "k2-{namespace}-{app}".into(),
            default: Some("k2-{namespace}-{app}".into()),
        };
        let json = serde_json::to_string(&field).unwrap();
        let back: McpTuningField = serde_json::from_str(&json).unwrap();
        assert_eq!(back.key, "rds_naming_convention");
        assert_eq!(back.placeholder, "k2-{namespace}-{app}");
        assert_eq!(back.default.as_deref(), Some("k2-{namespace}-{app}"));
    }

    #[test]
    fn resource_provision_result_missing_errors_defaults_to_empty_vec() {
        // Plugins that return state but no errors field must deserialize cleanly.
        let json =
            r#"{"state":{"DB_HOST":"mydb.us-east-1.rds.amazonaws.com"},"kubernetes_resources":[]}"#;
        let r: ResourceProvisionResult = serde_json::from_str(json).unwrap();
        assert!(
            r.errors.is_empty(),
            "errors should default to [] when absent"
        );
        assert_eq!(
            r.state.get("DB_HOST").map(String::as_str),
            Some("mydb.us-east-1.rds.amazonaws.com")
        );
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
        assert!(back.singleton);
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
        assert_eq!(
            back.fields.get("db_name").map(String::as_str),
            Some("myapp")
        );
    }

    #[test]
    fn outputs_round_trip() {
        let mut r = PluginResult::default();
        r.outputs
            .insert("role_arn".into(), "arn:aws:iam::123:role/app".into());
        let json = serde_json::to_string(&r).unwrap();
        let back: PluginResult = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.outputs.get("role_arn").unwrap(),
            "arn:aws:iam::123:role/app"
        );
    }

    #[test]
    fn plugin_outputs_in_context_round_trip() {
        let mut ctx = PluginContext {
            namespace: "ns".into(),
            deployment_name: "app".into(),
            annotations: HashMap::new(),
            labels: HashMap::new(),
            plugin_outputs: HashMap::new(),
            provisioned_resources: HashMap::new(),
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
        assert!(
            !json.contains("errors"),
            "empty errors should be skipped in JSON"
        );
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

    #[test]
    fn deprovision_result_without_delete_kubernetes_resources_deserializes() {
        // A plugin built against an older SDK never emits the field.
        let json = r#"{"errors":["snapshot failed"],"message":"RDS kept"}"#;
        let r: ResourceDeprovisionResult = serde_json::from_str(json).unwrap();
        assert!(r.delete_kubernetes_resources.is_empty());
        assert_eq!(r.errors, vec!["snapshot failed"]);
        assert_eq!(r.message, "RDS kept");

        let r: ResourceDeprovisionResult = serde_json::from_str("{}").unwrap();
        assert!(r.delete_kubernetes_resources.is_empty());
    }

    #[test]
    fn delete_kubernetes_resources_omitted_from_json_when_empty() {
        let json = serde_json::to_value(ResourceDeprovisionResult::default()).unwrap();
        assert_eq!(json, serde_json::json!({ "errors": [], "message": "" }));
    }

    #[test]
    fn delete_kubernetes_resources_round_trips() {
        let r = ResourceDeprovisionResult {
            message: "Deleted IAM role".into(),
            delete_kubernetes_resources: vec![
                KubernetesObjectRef::job("shop-db-bootstrap-1a2b3c4d"),
                KubernetesObjectRef::service_account("shop-aws-sa"),
            ],
            ..Default::default()
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "errors": [],
                "message": "Deleted IAM role",
                "delete_kubernetes_resources": [
                    { "api_version": "batch/v1", "kind": "Job", "name": "shop-db-bootstrap-1a2b3c4d" },
                    { "api_version": "v1", "kind": "ServiceAccount", "name": "shop-aws-sa" }
                ]
            })
        );
        let back: ResourceDeprovisionResult = serde_json::from_value(json).unwrap();
        assert_eq!(
            back.delete_kubernetes_resources,
            r.delete_kubernetes_resources
        );
        assert_eq!(back.message, "Deleted IAM role");
    }

    #[test]
    fn kubernetes_object_ref_constructors() {
        assert_eq!(
            KubernetesObjectRef::service_account("shop-aws-sa"),
            KubernetesObjectRef {
                api_version: "v1".into(),
                kind: "ServiceAccount".into(),
                name: "shop-aws-sa".into(),
            }
        );
        assert_eq!(
            KubernetesObjectRef::job("shop-db-bootstrap"),
            KubernetesObjectRef::new("batch/v1", "Job", "shop-db-bootstrap")
        );
    }
}
