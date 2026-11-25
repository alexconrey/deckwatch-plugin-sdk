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
//! ## Events and tracing
//!
//! Report problems as structured [`PluginEvent`]s with the result helpers
//! ([`PluginResult::warn`], [`PluginResult::error`] and friends), which also
//! copy each event into the legacy `errors`/`warnings` fields for hosts that
//! don't read `events`. Forward the host's trace on outbound HTTP with
//! [`PluginContext::trace_headers`]. Names shared with the host live in
//! [`telemetry`].
//!
//! See the [README](https://github.com/alexconrey/deckwatch-plugin-sdk) for full
//! build, test, and release instructions.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

pub mod telemetry;

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
/// `kubernetes_resources`: deckwatch creates or patches the account after the
/// Deployment is saved and binds the pod to it. Failures are only logged.
///
/// Mirrors `deckwatch::plugins::WantsServiceAccount` — kept in sync manually.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
/// In deckwatch 0.7.0 and the v0.6.7 release this does **not** decide
/// whether a resource is re-polled: the host re-polls resources whose
/// stored `state` looks pending (a `*_STATUS` key with an in-progress value,
/// or `DB_ENGINE`/`DB_STATUS` without `DB_HOST`). On a re-poll, `Available`
/// and `Failed` results are always stored, and a `Pending` result only if it
/// adds `state` keys or no longer looks pending. See the README's
/// "Provisioning" section.
///
/// Old plugins that do not emit this field deserialize to `Pending`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
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
#[non_exhaustive]
pub enum ConfigFieldType {
    /// Plain text input. Stored in plugin config as-is.
    #[default]
    String,
    /// Masked text input. Values saved through the settings form are stored
    /// AES-256-GCM encrypted when deckwatch has `DECKWATCH_ENCRYPTION_KEY`
    /// set. deckwatch (0.7.0 and v0.6.7) does not decrypt them before calling
    /// the plugin (deckwatch#251; see the README's "Known host issues").
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
    /// Display name. deckwatch uses it only in the MCP tuning block, falling
    /// back to the registered name when empty; logs, events and status use the
    /// name the plugin is registered under.
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
    /// Minimum deckwatch version this plugin needs, e.g. `"0.7.0"`.
    ///
    /// Deckwatch checks it once, at load time, and skips the plugin with an
    /// error log when it is older. The comparison splits both versions on
    /// `.`, keeps only the parts that parse as integers, and compares the
    /// first three (missing parts count as 0). So don't add a `v` prefix:
    /// `"v0.7.0"` parses as 7.0.0. Pre-release suffixes such as `-beta` are
    /// dropped with the part they're attached to. `None` means no constraint.
    ///
    /// Mirrors `deckwatch::plugins::PluginMetadata::min_deckwatch_version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_deckwatch_version: Option<String>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceProvisionRequest {
    /// Name of the application being provisioned for.
    pub application_name: String,
    /// Kubernetes namespace the application lives in.
    pub namespace: String,
    /// Plugin-declared resource type id (matches [`PluginResource::id`]).
    pub resource_id: String,
    /// User-submitted form values, keyed by [`ConfigField::key`].
    pub fields: HashMap<String, String>,
    /// W3C `traceparent` of the host's span around this call. Forward it with
    /// [`trace_headers`](Self::trace_headers); see
    /// [`PluginContext::trace_context`].
    ///
    /// Old hosts don't send it, and it deserializes to `None`; `None` is left
    /// out of the JSON. deckwatch 0.7.0 sends it while it exports OTLP traces;
    /// v0.6.7 never does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<String>,
}

impl ResourceProvisionRequest {
    /// Headers that put an outbound HTTP request in the host's trace. Same as
    /// [`PluginContext::trace_headers`].
    pub fn trace_headers(&self) -> Vec<(&'static str, String)> {
        telemetry::trace_headers(self.trace_context.as_deref())
    }
}

/// Result returned from the plugin's `provision()` WASM export.
///
/// Deckwatch stores `state`, `deployment_annotations` and `sidecars` with the
/// resource and applies them to the application's workloads on its reconcile
/// loop (every 10 s).
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
    /// applying the same sidecar twice is idempotent. On this path the
    /// sidecars' env vars lose `value_from` (deckwatch#247). When the
    /// application's last resource is removed, deckwatch 0.7.0 releases the
    /// sidecars; v0.6.7 leaves them in place.
    ///
    /// Use this to inject process sidecars (Redis, PostgreSQL, Fluent Bit, etc.)
    /// that should run alongside every workload in the application.
    /// Old plugins that do not emit this field deserialize to an empty vec.
    #[serde(default)]
    pub sidecars: Vec<SidecarSpec>,
    /// Init containers to add to every Deployment in the application.
    ///
    /// Intended to be prepended to `spec.template.spec.initContainers` on each
    /// reconcile. deckwatch (0.7.0 and v0.6.7) never stores this field for
    /// provisioned resources, so it has no effect (deckwatch#249). Old plugins
    /// that do not emit this field deserialize to an empty vec.
    ///
    /// Mirrors `deckwatch::plugins::ResourceProvisionResult::init_containers`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub init_containers: Vec<SidecarSpec>,
    /// Kubernetes objects to server-side apply (e.g. a ServiceAccount with an
    /// IRSA annotation), with the same rules as
    /// [`PluginResult::kubernetes_resources`].
    #[serde(default)]
    pub kubernetes_resources: Vec<serde_json::Value>,
    /// Errors to report; provisioning still succeeds. deckwatch 0.7.0 turns
    /// each entry that isn't a copy made by the event helpers into an error
    /// event, which marks the resource failing (a Kubernetes Event, and a
    /// notification on the transition); v0.6.7 logs them at `ERROR`. Prefer
    /// [`error`](Self::error).
    #[serde(default)]
    pub errors: Vec<String>,
    /// Non-fatal warnings worth the operator's attention (e.g. a resource was
    /// created but a non-critical tag is missing). deckwatch 0.7.0 turns each
    /// into a warning event, which marks the resource degraded; v0.6.7 logs
    /// them at `WARN`. Prefer [`warn`](Self::warn). Old plugins deserialize to
    /// an empty vec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Provisioning lifecycle status. See [`ProvisioningStatus`] for what
    /// deckwatch actually does with it (it does not, by
    /// itself, cause or stop re-polling). Old plugins that do not emit this
    /// field deserialize to `Pending`.
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
    /// Structured events about this call. Add them with
    /// [`info`](Self::info), [`warn`](Self::warn), [`error`](Self::error) or
    /// [`push_event`](Self::push_event), which also copy them into `errors`
    /// and `warnings` for hosts that don't read `events`.
    ///
    /// Old plugins don't send it, and it deserializes to an empty vec; an
    /// empty vec is left out of the JSON. deckwatch 0.7.0 reads it (see the
    /// README's "Status, Kubernetes Events and notifications"); v0.6.7
    /// ignores it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<PluginEvent>,
}

impl ResourceProvisionResult {
    /// Reports an [`EventSeverity::Info`] event. It has no legacy copy, so
    /// hosts that don't read `events` don't see it. Returns the stored event,
    /// so you can set its optional fields.
    pub fn info(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Info, code, message))
    }

    /// Reports an [`EventSeverity::Warning`] event, copied to `warnings`.
    /// Returns the stored event, so you can set its optional fields.
    pub fn warn(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Warning, code, message))
    }

    /// Reports an [`EventSeverity::Error`] event, copied to `errors`. Returns
    /// the stored event, so you can set its optional fields.
    pub fn error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Error, code, message))
    }

    /// Adds `event` to `events` and copies it into the legacy fields, exactly
    /// like [`PluginResult::push_event`]: `Error` to `errors`, `Warning` and
    /// `Unknown` to `warnings`, `Info` nowhere.
    pub fn push_event(&mut self, event: PluginEvent) -> &mut PluginEvent {
        match event.severity {
            EventSeverity::Error => self.errors.push(event.legacy_text()),
            EventSeverity::Warning | EventSeverity::Unknown => {
                self.warnings.push(event.legacy_text())
            }
            EventSeverity::Info => {}
        }
        append_event(&mut self.events, event)
    }
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// W3C `traceparent` of the host's span around this call. Forward it with
    /// [`trace_headers`](Self::trace_headers); see
    /// [`PluginContext::trace_context`].
    ///
    /// Old hosts don't send it, and it deserializes to `None`; `None` is left
    /// out of the JSON. deckwatch 0.7.0 sends it while it exports OTLP traces;
    /// v0.6.7 never does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<String>,
}

impl ResourceDeprovisionRequest {
    /// Headers that put an outbound HTTP request in the host's trace. Same as
    /// [`PluginContext::trace_headers`].
    pub fn trace_headers(&self) -> Vec<(&'static str, String)> {
        telemetry::trace_headers(self.trace_context.as_deref())
    }
}

/// What the plugin returns from its `deprovision()` export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceDeprovisionResult {
    /// Does not block removal of the resource record. deckwatch 0.7.0 returns
    /// them as events in the DELETE response; v0.6.7 logs them at `ERROR`.
    #[serde(default)]
    pub errors: Vec<String>,
    /// What was and was not cleaned up. deckwatch 0.7.0 returns it in the
    /// DELETE response and shows it in the UI; v0.6.7 only logs it, at `INFO`.
    #[serde(default)]
    pub message: String,
    /// Kubernetes objects for deckwatch to delete once `deprovision()` returns.
    ///
    /// Use this for objects deckwatch created on the plugin's behalf (through
    /// `kubernetes_resources` or `wants_service_account`) that the plugin
    /// can't delete itself, such as a ServiceAccount or a one-off Job.
    ///
    /// deckwatch 0.7.0 only deletes in the application's namespace (a
    /// [`KubernetesObjectRef`] has no namespace) and only `v1`
    /// `ServiceAccount` and `batch/v1` `Job`. Jobs are deleted together with
    /// their pods, and before ServiceAccounts. A ServiceAccount that any
    /// Deployment, CronJob or Job in the namespace still uses is kept, because
    /// deleting it would stop new pods from being created; the namespace's
    /// `default` ServiceAccount is never deleted. Invalid names are refused,
    /// and an object that is already gone counts as deleted. There is no
    /// ownership check, so only list objects this plugin created. Each
    /// outcome is returned in the DELETE response; refusals and failures are
    /// logged and never block removal of the resource record.
    ///
    /// Old plugins that do not emit this field deserialize to an empty vec, and
    /// an empty vec is left out of the JSON. The v0.6.7 release ignores it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delete_kubernetes_resources: Vec<KubernetesObjectRef>,
    /// Structured events about this call. Add them with
    /// [`info`](Self::info), [`warn`](Self::warn), [`error`](Self::error) or
    /// [`push_event`](Self::push_event), which also copy them into `errors`
    /// or `message` for hosts that don't read `events`.
    ///
    /// Old plugins don't send it, and it deserializes to an empty vec; an
    /// empty vec is left out of the JSON. deckwatch 0.7.0 reads it (see the
    /// README's "Status, Kubernetes Events and notifications"); v0.6.7
    /// ignores it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<PluginEvent>,
}

impl ResourceDeprovisionResult {
    /// Reports an [`EventSeverity::Info`] event, appended to `message`.
    /// Returns the stored event, so you can set its optional fields.
    pub fn info(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Info, code, message))
    }

    /// Reports an [`EventSeverity::Warning`] event, appended to `message`
    /// (this result has no `warnings` field). Returns the stored event, so
    /// you can set its optional fields.
    pub fn warn(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Warning, code, message))
    }

    /// Reports an [`EventSeverity::Error`] event, copied to `errors`. Returns
    /// the stored event, so you can set its optional fields.
    pub fn error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Error, code, message))
    }

    /// Adds `event` to `events` and copies its
    /// [`legacy_text`](PluginEvent::legacy_text) into the field older hosts
    /// read. `message` is the only channel here besides `errors`:
    ///
    /// | Severity | Copied to |
    /// |---|---|
    /// | `Error` | `errors` |
    /// | `Warning`, `Unknown`, `Info` | appended to `message`, after `"; "` unless `message` is empty |
    ///
    /// Only the code and message are copied. Returns the event as stored in
    /// `events`; changing it afterwards doesn't change the copy.
    ///
    /// ```
    /// use deckwatch_plugin_sdk::ResourceDeprovisionResult;
    ///
    /// let mut result = ResourceDeprovisionResult::default();
    /// result.info("aws.role_deleted", "Deleted IAM role shop-role");
    /// result.warn("aws.bucket_kept", "Bucket shop-uploads is not empty; kept it");
    ///
    /// assert_eq!(
    ///     result.message,
    ///     "aws.role_deleted: Deleted IAM role shop-role; \
    ///      aws.bucket_kept: Bucket shop-uploads is not empty; kept it"
    /// );
    /// assert!(result.errors.is_empty());
    /// ```
    pub fn push_event(&mut self, event: PluginEvent) -> &mut PluginEvent {
        match event.severity {
            EventSeverity::Error => self.errors.push(event.legacy_text()),
            EventSeverity::Warning | EventSeverity::Unknown | EventSeverity::Info => {
                if !self.message.is_empty() {
                    self.message.push_str("; ");
                }
                self.message.push_str(&event.legacy_text());
            }
        }
        append_event(&mut self.events, event)
    }
}

/// A Kubernetes object in the application's namespace, identified by API
/// version, kind and name.
///
/// Used in [`ResourceDeprovisionResult::delete_kubernetes_resources`]. Use
/// [`KubernetesObjectRef::service_account`] or [`KubernetesObjectRef::job`] to
/// construct the kinds deckwatch deletes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

// ── Events ────────────────────────────────────────────────────────────────────

/// How serious a [`PluginEvent`] is.
///
/// Serialized in `snake_case`: `"info"`, `"warning"`, `"error"`.
///
/// Any other value, such as a severity added by a newer SDK, deserializes to
/// [`EventSeverity::Unknown`] instead of failing. Without that catch-all, a
/// host built on an older SDK would reject a newer plugin's entire result
/// because of one event.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventSeverity {
    /// Worth knowing; nothing is wrong. For example, a resource was created,
    /// or a step was skipped on purpose.
    #[default]
    Info,
    /// The call did its job, but something needs attention. For example, an
    /// optional setting is missing, or a cloud resource is still warming up.
    Warning,
    /// Something the plugin was asked to do failed.
    Error,
    /// A severity this SDK doesn't know, from a plugin built on a newer one.
    ///
    /// Hosts treat it as [`Warning`](EventSeverity::Warning). It serializes as
    /// `"unknown"`, not as the original value. Don't return it from a plugin.
    #[serde(other)]
    Unknown,
}

/// A structured notice, warning or error that a plugin reports.
///
/// Results carry these in `events` ([`PluginResult::events`],
/// [`ResourceProvisionResult::events`], [`ResourceDeprovisionResult::events`]).
/// Add them with the results' `info`, `warn`, `error` and `push_event`
/// helpers, which also copy each event into the legacy `errors`, `warnings` or
/// `message` field, so hosts that don't read `events` still log it.
///
/// Every key may be missing when deserializing (`#[serde(default)]` on the
/// struct), so a partial or hand-written event still parses instead of making
/// the host reject the whole result. Serializing always writes `severity`,
/// `code`, `message` and `retryable`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct PluginEvent {
    /// How serious the event is. A missing key reads as `info`, and an
    /// unrecognised value as [`EventSeverity::Unknown`].
    pub severity: EventSeverity,
    /// Stable machine-readable code that hosts, alerts and docs can match on:
    /// `"<plugin-name>.<code>"`, e.g. `"aws.assume_role_failed"`. Codes that
    /// start with `plugin.` are reserved for the host (see
    /// [`telemetry::code`]).
    pub code: String,
    /// Human-readable text for operators. Hosts cap its length (about 2 KB),
    /// so put the essentials first. Don't put secrets in it: hosts log, store
    /// and export it.
    pub message: String,
    /// The [`PluginResource::id`] this event is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// The Kubernetes object, in the application's namespace, this event is
    /// about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<KubernetesObjectRef>,
    /// Whether repeating the same call may succeed with nothing changed (for
    /// example after throttling or a timeout), as opposed to a failure that
    /// needs a fix first (bad configuration, a missing permission).
    #[serde(default)]
    pub retryable: bool,
    /// A suggested fix for the operator, e.g. `"Grant iam:PassRole to the
    /// deckwatch role"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl PluginEvent {
    /// An event with the given severity, code and message, and no optional
    /// fields set.
    pub fn new(
        severity: EventSeverity,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            ..Default::default()
        }
    }

    /// The text the result helpers copy into the legacy `errors`, `warnings`
    /// and `message` fields: `"<code>: <message>"`, or just the message when
    /// `code` is empty.
    ///
    /// Hosts that read `events` can use it to recognise those copies and not
    /// report the same problem twice.
    pub fn legacy_text(&self) -> String {
        if self.code.is_empty() {
            self.message.clone()
        } else {
            format!("{}: {}", self.code, self.message)
        }
    }
}

/// Appends `event` to `events` and returns it, as stored.
fn append_event(events: &mut Vec<PluginEvent>, event: PluginEvent) -> &mut PluginEvent {
    let index = events.len();
    events.push(event);
    &mut events[index]
}

/// Context passed to a plugin's `apply` function.
///
/// Deckwatch serializes this as JSON and passes it as input to the WASM
/// `apply` function. On Deployment create and update, the fields reflect the
/// Deployment **after** the operator's changes and before any plugin runs. On
/// the reconcile loop they come from the live Deployment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

    /// W3C `traceparent` of the host's span around this call, e.g.
    /// `"00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"`. Send it on
    /// the plugin's own HTTP requests with [`trace_headers`](Self::trace_headers),
    /// so its API calls join the host's trace.
    ///
    /// `None` when the host doesn't trace the call. Old hosts don't send it,
    /// and it deserializes to `None`; `None` is left out of the JSON. deckwatch
    /// 0.7.0 sends it while it exports OTLP traces; v0.6.7 never does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<String>,
}

impl PluginContext {
    /// Headers that put an outbound HTTP request in the host's trace:
    /// `[("traceparent", <trace_context>)]` when
    /// [`trace_context`](Self::trace_context) is a valid version-00 W3C
    /// `traceparent`, and nothing otherwise (no context, or a malformed one).
    /// Add them to every request the plugin makes, for example with
    /// `extism_pdk`:
    ///
    /// ```rust,ignore
    /// let mut req = extism_pdk::HttpRequest::new(url).with_method("GET");
    /// for (name, value) in ctx.trace_headers() {
    ///     req = req.with_header(name, value);
    /// }
    /// ```
    ///
    /// ```
    /// use deckwatch_plugin_sdk::PluginContext;
    ///
    /// let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    /// let ctx = PluginContext {
    ///     trace_context: Some(traceparent.into()),
    ///     ..Default::default()
    /// };
    /// assert_eq!(ctx.trace_headers(), [("traceparent", traceparent.to_string())]);
    /// assert!(PluginContext::default().trace_headers().is_empty());
    /// ```
    pub fn trace_headers(&self) -> Vec<(&'static str, String)> {
        telemetry::trace_headers(self.trace_context.as_deref())
    }
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
    /// in `kubernetes_resources`. deckwatch creates or patches it after the
    /// Deployment is saved and binds the pod to it; failures are only logged.
    ///
    /// Set `irsa_role_arn` to the IAM role ARN when using IRSA; leave it
    /// empty for a plain ServiceAccount with no IAM binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wants_service_account: Option<WantsServiceAccount>,

    /// Annotations to stamp on the deployment's `metadata.annotations`.
    ///
    /// Keys should be namespaced (e.g. `"myorg.io/vault-injected": "true"`).
    /// deckwatch (0.7.0 and v0.6.7) applies these only on the reconcile loop
    /// (server-side apply), which only covers applications with a provisioned
    /// resource; Deployment create and update ignore them (deckwatch#244).
    /// Old plugins deserialize to an empty map.
    #[serde(default)]
    pub deployment_annotations: HashMap<String, String>,

    /// Labels to stamp on the deployment's `metadata.labels`.
    ///
    /// Keys should be namespaced (e.g. `"myorg.io/managed-by-vault": "true"`).
    /// Applied under the same conditions as `deployment_annotations`.
    /// Old plugins deserialize to an empty map.
    #[serde(default)]
    pub deployment_labels: HashMap<String, String>,

    /// Non-fatal errors the plugin wants the operator to see: IAM errors, RDS
    /// provisioning failures, etc. The deployment is never blocked.
    ///
    /// deckwatch 0.7.0 turns each entry that isn't a copy made by the event
    /// helpers into an error event, which marks the plugin failing for the
    /// Deployment's application. v0.6.7 logs it at `ERROR`. Prefer
    /// [`error`](Self::error), which records a structured event and fills this
    /// field too.
    ///
    /// `extism_pdk::log!` works too: extism turns it into a host `tracing`
    /// event (target `extism::pdk`) whose only field is extism's per-call
    /// instance id. deckwatch 0.7.0 emits it inside the call's span, which
    /// names the plugin, and its default log filter keeps those events since
    /// `36e6637` (deckwatch#256); see the README's "The sandbox".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,

    /// Init containers to add to the pod's `initContainers` list, for one-shot
    /// setup tasks that must finish before the containers start.
    ///
    /// Used on Deployment create and update only. An init container is skipped
    /// if one with the same name exists. In deckwatch 0.7.0 and the v0.6.7
    /// release they are **appended**, so they run after the workload's
    /// own init containers (deckwatch#249); their `volumes` are not added to
    /// the pod, their `primary_volume_mounts` are ignored, and no tracking
    /// annotation is written. Old plugins deserialize to an empty vec.
    ///
    /// Mirrors `deckwatch::plugins::PluginResult::init_containers`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub init_containers: Vec<SidecarSpec>,

    /// Non-fatal warnings: degraded-but-functional conditions (e.g. a
    /// non-critical config key is missing, or a cloud resource is still
    /// warming up). The deployment is never blocked. deckwatch 0.7.0 turns
    /// each into a warning event, which marks the plugin degraded; v0.6.7 logs
    /// them at `WARN`. Prefer [`warn`](Self::warn). Old plugins deserialize to
    /// an empty vec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,

    /// Structured events about this call. Add them with
    /// [`info`](Self::info), [`warn`](Self::warn), [`error`](Self::error) or
    /// [`push_event`](Self::push_event), which also copy them into `errors`
    /// and `warnings` for hosts that don't read `events`.
    ///
    /// Old plugins don't send it, and it deserializes to an empty vec; an
    /// empty vec is left out of the JSON. deckwatch 0.7.0 reads it (see the
    /// README's "Status, Kubernetes Events and notifications"); v0.6.7
    /// ignores it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<PluginEvent>,
}

impl PluginResult {
    /// Reports an [`EventSeverity::Info`] event. It has no legacy copy, so
    /// hosts that don't read `events` don't see it. Returns the stored event,
    /// so you can set its optional fields.
    pub fn info(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Info, code, message))
    }

    /// Reports an [`EventSeverity::Warning`] event, copied to `warnings`.
    /// Returns the stored event, so you can set its optional fields.
    pub fn warn(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Warning, code, message))
    }

    /// Reports an [`EventSeverity::Error`] event, copied to `errors`. Returns
    /// the stored event, so you can set its optional fields.
    pub fn error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut PluginEvent {
        self.push_event(PluginEvent::new(EventSeverity::Error, code, message))
    }

    /// Adds `event` to `events` and copies its
    /// [`legacy_text`](PluginEvent::legacy_text) into the field older hosts
    /// read:
    ///
    /// | Severity | Copied to |
    /// |---|---|
    /// | `Error` | `errors` |
    /// | `Warning`, `Unknown` | `warnings` |
    /// | `Info` | nothing (there is no legacy field for it) |
    ///
    /// Only the code and message are copied; `resource_id`, `object`,
    /// `retryable` and `hint` exist only in `events`. Returns the event as
    /// stored in `events`; changing it afterwards doesn't change the copy.
    ///
    /// ```
    /// use deckwatch_plugin_sdk::PluginResult;
    ///
    /// let mut result = PluginResult::default();
    /// result.warn("vault.addr_default", "VAULT_ADDR is not set; using the default");
    /// let event = result.error("vault.token_missing", "No Vault token is configured");
    /// event.hint = Some("Set VAULT_TOKEN in the plugin's settings".into());
    ///
    /// assert_eq!(result.events.len(), 2);
    /// assert_eq!(
    ///     result.warnings,
    ///     ["vault.addr_default: VAULT_ADDR is not set; using the default"]
    /// );
    /// assert_eq!(result.errors, ["vault.token_missing: No Vault token is configured"]);
    /// ```
    pub fn push_event(&mut self, event: PluginEvent) -> &mut PluginEvent {
        match event.severity {
            EventSeverity::Error => self.errors.push(event.legacy_text()),
            EventSeverity::Warning | EventSeverity::Unknown => {
                self.warnings.push(event.legacy_text())
            }
            EventSeverity::Info => {}
        }
        append_event(&mut self.events, event)
    }
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvVarSource {
    /// Pull the value from a Kubernetes Secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key_ref: Option<SecretKeyRef>,

    /// Pull the value from a Kubernetes ConfigMap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_map_key_ref: Option<ConfigMapKeyRef>,
}

/// Reference to a specific key within a Kubernetes Secret.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretKeyRef {
    /// Name of the Kubernetes Secret.
    pub name: String,
    /// Key within the Secret's `data` map.
    pub key: String,
}

/// Reference to a specific key within a Kubernetes ConfigMap.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigMapKeyRef {
    /// Name of the Kubernetes ConfigMap.
    pub name: String,
    /// Key within the ConfigMap's `data` map.
    pub key: String,
}

/// A volume to attach to the pod, declared by a plugin sidecar.
///
/// Deckwatch collects all `VolumeSpec` entries from injected sidecars and
/// merges them into `spec.template.spec.volumes` on the target pod, skipping
/// any whose `name` already exists, so that both the sidecar and the primary
/// container can mount the same volume.
///
/// Mirrors `deckwatch::plugins::VolumeSpec`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VolumeSpec {
    /// Volume name — must be unique within the pod.
    pub name: String,
    /// Volume type. deckwatch understands exactly `"emptyDir"` (with
    /// `size_limit`), `"secret"` (`secretName` = `source_name`) and
    /// `"configMap"` (`name` = `source_name`). Any other value produces a
    /// volume with only a name, which Kubernetes defaults to `emptyDir`.
    pub volume_type: String,
    /// For `secret` and `configMap` volumes: the name of the source object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    /// For `emptyDir` volumes: optional size limit, e.g. `"256Mi"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_limit: Option<String>,
}

/// A volume mount to attach inside a container.
///
/// Used in [`SidecarSpec::volume_mounts`] for the sidecar itself, and in
/// [`SidecarSpec::primary_volume_mounts`] for the pod's primary container.
///
/// Mirrors `deckwatch::plugins::VolumeMountSpec`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VolumeMountSpec {
    /// Name of the volume to mount (must match a `VolumeSpec::name` in the pod).
    pub name: String,
    /// Absolute path inside the container where the volume is mounted.
    pub mount_path: String,
    /// If `true`, the mount is read-only.
    #[serde(default)]
    pub read_only: bool,
    /// Mount only a sub-path of the volume rather than its root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_path: Option<String>,
}

/// A sidecar container to inject into the pod alongside the primary container.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<i32>,

    /// CPU request (e.g. `"50m"`). Takes precedence over the deprecated `cpu`
    /// field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_request: Option<String>,

    /// CPU limit (e.g. `"200m"`). Takes precedence over the deprecated `cpu`
    /// field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<String>,

    /// Memory request (e.g. `"64Mi"`). Takes precedence over the deprecated
    /// `memory` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_request: Option<String>,

    /// Memory limit (e.g. `"128Mi"`). Takes precedence over the deprecated
    /// `memory` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,

    /// Deprecated: use `cpu_request` and `cpu_limit` instead.
    ///
    /// Sets both the request and the limit, where `cpu_request` / `cpu_limit`
    /// are unset. Kept for plugins built against SDKs before 0.10.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,

    /// Deprecated: use `memory_request` and `memory_limit` instead.
    ///
    /// Sets both the request and the limit, where `memory_request` /
    /// `memory_limit` are unset. Kept for plugins built against SDKs before
    /// 0.10.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,

    /// Volumes required by this sidecar.
    ///
    /// Deckwatch collects these from all injected sidecars and merges them into
    /// `spec.template.spec.volumes` on the pod, skipping names that already
    /// exist, so the sidecar can mount them.
    #[serde(default)]
    pub volumes: Vec<VolumeSpec>,

    /// Volume mounts for this sidecar container.
    #[serde(default)]
    pub volume_mounts: Vec<VolumeMountSpec>,

    /// Volume mounts to add to the pod's **primary** (first) container on
    /// behalf of this sidecar.
    ///
    /// Use this when the primary container must read from a volume the sidecar
    /// shares. Deckwatch merges these into the first container's
    /// `volumeMounts`, skipping any with the same name and mount path.
    #[serde(default)]
    pub primary_volume_mounts: Vec<VolumeMountSpec>,
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
    fn min_deckwatch_version_round_trips() {
        let m = PluginMetadata {
            name: "my-plugin".into(),
            min_deckwatch_version: Some("0.7.0".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            json.contains("min_deckwatch_version"),
            "field must be present when Some"
        );
        let back: PluginMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.min_deckwatch_version.as_deref(), Some("0.7.0"));
    }

    #[test]
    fn min_deckwatch_version_none_is_omitted_from_json() {
        let m = PluginMetadata::default();
        let json = serde_json::to_string(&m).unwrap();
        assert!(
            !json.contains("min_deckwatch_version"),
            "field must be absent when None"
        );
    }

    #[test]
    fn min_deckwatch_version_missing_from_old_plugin_defaults_to_none() {
        // Old plugin WASM that predates this field must still deserialize cleanly.
        let json = r#"{"name":"old-plugin","version":"0.5.1","description":"","provides":[],"depends_on":[],"optional_depends_on":[]}"#;
        let m: PluginMetadata = serde_json::from_str(json).unwrap();
        assert!(
            m.min_deckwatch_version.is_none(),
            "min_deckwatch_version should default to None when absent"
        );
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
            trace_context: None,
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
            trace_context: None,
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

    /// Every public struct derives `Default` (so plugins can use
    /// `..Default::default()` and stay source-compatible when fields are
    /// added), and that default must survive a JSON round trip.
    #[test]
    fn every_struct_default_round_trips() {
        fn check<T: Default + Serialize + serde::de::DeserializeOwned>() {
            let json = serde_json::to_value(T::default()).unwrap();
            let back: T = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(back).unwrap(), json);
        }
        check::<WantsServiceAccount>();
        check::<ConfigField>();
        check::<McpTuningField>();
        check::<PluginMetadata>();
        check::<PluginResource>();
        check::<ResourceProvisionRequest>();
        check::<ResourceProvisionResult>();
        check::<ResourceDeprovisionRequest>();
        check::<ResourceDeprovisionResult>();
        check::<KubernetesObjectRef>();
        check::<PluginContext>();
        check::<PluginResult>();
        check::<EnvVarSpec>();
        check::<EnvVarSource>();
        check::<SecretKeyRef>();
        check::<ConfigMapKeyRef>();
        check::<VolumeSpec>();
        check::<VolumeMountSpec>();
        check::<SidecarSpec>();
        check::<PluginEvent>();
        check::<ProvisioningStatus>();
        check::<ConfigFieldType>();
        check::<EventSeverity>();
    }

    // ── Events, trace context and telemetry names (0.10.0) ──────────────────

    const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn event_severity_is_snake_case() {
        for (severity, json) in [
            (EventSeverity::Info, "\"info\""),
            (EventSeverity::Warning, "\"warning\""),
            (EventSeverity::Error, "\"error\""),
            (EventSeverity::Unknown, "\"unknown\""),
        ] {
            assert_eq!(serde_json::to_string(&severity).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<EventSeverity>(json).unwrap(),
                severity
            );
        }
    }

    // A severity from a newer SDK must not make the host reject the result.
    #[test]
    fn unrecognised_severity_deserializes_as_unknown() {
        for other in ["\"critical\"", "\"Error\"", "\"\""] {
            assert_eq!(
                serde_json::from_str::<EventSeverity>(other).unwrap(),
                EventSeverity::Unknown,
                "{other}"
            );
        }
        let r: PluginResult = serde_json::from_str(
            r#"{"events":[{"severity":"critical","code":"p.x","message":"m"}],"errors":["e"]}"#,
        )
        .expect("a result with a future severity must still parse");
        assert_eq!(r.events[0].severity, EventSeverity::Unknown);
        assert_eq!(r.events[0].code, "p.x");
        assert_eq!(r.errors, ["e"]);
    }

    #[test]
    fn minimal_event_json() {
        let json =
            serde_json::to_value(PluginEvent::new(EventSeverity::Error, "p.x", "m")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "severity": "error",
                "code": "p.x",
                "message": "m",
                "retryable": false
            })
        );
    }

    #[test]
    fn full_event_round_trips() {
        let event = PluginEvent {
            severity: EventSeverity::Warning,
            code: "aws.tag_missing".into(),
            message: "CostCenter tag is not set".into(),
            resource_id: Some("rds".into()),
            object: Some(KubernetesObjectRef::service_account("shop-aws-sa")),
            retryable: true,
            hint: Some("Add CostCenter to RESOURCE_TAGS".into()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "severity": "warning",
                "code": "aws.tag_missing",
                "message": "CostCenter tag is not set",
                "resource_id": "rds",
                "object": { "api_version": "v1", "kind": "ServiceAccount", "name": "shop-aws-sa" },
                "retryable": true,
                "hint": "Add CostCenter to RESOURCE_TAGS"
            })
        );
        assert_eq!(serde_json::from_value::<PluginEvent>(json).unwrap(), event);
    }

    // `#[serde(default)]` on the struct: one partial event must not make the
    // host reject the whole result.
    #[test]
    fn partial_event_deserializes_with_defaults() {
        let e: PluginEvent = serde_json::from_str("{}").unwrap();
        assert_eq!(e, PluginEvent::default());
        let e: PluginEvent = serde_json::from_str(r#"{"message":"m"}"#).unwrap();
        assert_eq!(e.severity, EventSeverity::Info);
        assert_eq!(e.code, "");
        assert_eq!(e.message, "m");
        let r: ResourceProvisionResult =
            serde_json::from_str(r#"{"state":{"K":"v"},"events":[{"severity":"error"}]}"#).unwrap();
        assert_eq!(r.events[0].severity, EventSeverity::Error);
        assert_eq!(r.state["K"], "v");
    }

    #[test]
    fn events_and_trace_context_omitted_from_json_when_empty() {
        for json in [
            serde_json::to_value(PluginResult::default()).unwrap(),
            serde_json::to_value(ResourceProvisionResult::default()).unwrap(),
            serde_json::to_value(ResourceDeprovisionResult::default()).unwrap(),
        ] {
            assert!(json.get("events").is_none(), "{json}");
        }
        for json in [
            serde_json::to_value(PluginContext::default()).unwrap(),
            serde_json::to_value(ResourceProvisionRequest::default()).unwrap(),
            serde_json::to_value(ResourceDeprovisionRequest::default()).unwrap(),
        ] {
            assert!(json.get("trace_context").is_none(), "{json}");
        }
    }

    #[test]
    fn trace_context_round_trips() {
        let ctx = PluginContext {
            namespace: "ns".into(),
            deployment_name: "app".into(),
            trace_context: Some(TRACEPARENT.into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["trace_context"], TRACEPARENT);
        let back: PluginContext = serde_json::from_value(json).unwrap();
        assert_eq!(back.trace_context.as_deref(), Some(TRACEPARENT));
    }

    #[test]
    fn legacy_text_is_code_and_message() {
        let mut e = PluginEvent::new(EventSeverity::Error, "p.x", "it broke");
        assert_eq!(e.legacy_text(), "p.x: it broke");
        e.code.clear();
        assert_eq!(e.legacy_text(), "it broke");
    }

    #[test]
    fn plugin_result_helpers_copy_events_to_errors_and_warnings() {
        let mut r = PluginResult::default();
        r.info("p.noted", "i");
        r.warn("p.warned", "w");
        r.error("p.failed", "e");
        r.push_event(PluginEvent::new(EventSeverity::Unknown, "p.odd", "u"));
        let severities: Vec<_> = r.events.iter().map(|e| e.severity).collect();
        assert_eq!(
            severities,
            [
                EventSeverity::Info,
                EventSeverity::Warning,
                EventSeverity::Error,
                EventSeverity::Unknown
            ]
        );
        assert_eq!(r.warnings, ["p.warned: w", "p.odd: u"]);
        assert_eq!(r.errors, ["p.failed: e"]);
    }

    #[test]
    fn provision_result_helpers_copy_events_to_errors_and_warnings() {
        let mut r = ResourceProvisionResult::default();
        r.info("p.noted", "i");
        r.warn("p.warned", "w");
        r.error("p.failed", "e");
        r.push_event(PluginEvent::new(EventSeverity::Unknown, "p.odd", "u"));
        assert_eq!(r.events.len(), 4);
        assert_eq!(r.warnings, ["p.warned: w", "p.odd: u"]);
        assert_eq!(r.errors, ["p.failed: e"]);
        assert_eq!(r.status, ProvisioningStatus::Pending, "status is untouched");
    }

    #[test]
    fn deprovision_result_helpers_copy_events_to_errors_and_message() {
        let mut r = ResourceDeprovisionResult::default();
        r.info("p.noted", "i");
        assert_eq!(
            r.message, "p.noted: i",
            "no separator when message was empty"
        );
        r.warn("p.warned", "w");
        r.error("p.failed", "e");
        r.push_event(PluginEvent::new(EventSeverity::Unknown, "p.odd", "u"));
        assert_eq!(r.events.len(), 4);
        assert_eq!(r.message, "p.noted: i; p.warned: w; p.odd: u");
        assert_eq!(r.errors, ["p.failed: e"]);

        let mut r = ResourceDeprovisionResult {
            message: "Deleted IAM role".into(),
            ..Default::default()
        };
        r.warn("aws.bucket_kept", "bucket not empty");
        assert_eq!(
            r.message,
            "Deleted IAM role; aws.bucket_kept: bucket not empty"
        );
    }

    #[test]
    fn helpers_return_the_stored_event() {
        let mut r = PluginResult::default();
        let e = r.error("p.failed", "e");
        e.retryable = true;
        e.resource_id = Some("rds".into());
        e.hint = Some("h".into());
        assert!(r.events[0].retryable);
        assert_eq!(r.events[0].resource_id.as_deref(), Some("rds"));
        assert_eq!(r.events[0].hint.as_deref(), Some("h"));
        assert_eq!(
            r.errors,
            ["p.failed: e"],
            "the legacy copy is only code and message"
        );
    }

    #[test]
    fn trace_headers_forward_only_a_valid_traceparent() {
        let ctx = |trace_context: &str| PluginContext {
            trace_context: Some(trace_context.into()),
            ..Default::default()
        };
        let expected = [("traceparent", TRACEPARENT.to_string())];
        assert_eq!(ctx(TRACEPARENT).trace_headers(), expected);
        // Any trace flags are valid.
        let unsampled = TRACEPARENT.replace("-01", "-00");
        assert_eq!(ctx(&unsampled).trace_headers()[0].1, unsampled);
        assert!(PluginContext::default().trace_headers().is_empty());

        for bad in [
            "",
            "00",
            "garbage",
            // uppercase hex
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01",
            // all-zero trace id, all-zero parent id
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            // forbidden and unsupported versions
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            // wrong length, separators, non-hex, whitespace, header injection
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-00",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1",
            "00_4bf92f3577b34da6a3ce929d0e0e4736_00f067aa0ba902b7_01",
            "00-4bf92f3577b34da6a3ce929d0e0e47g6-00f067aa0ba902b7-01",
            " 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01\r\nx-evil: 1",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-é",
        ] {
            assert!(ctx(bad).trace_headers().is_empty(), "{bad:?}");
        }

        let req = ResourceProvisionRequest {
            trace_context: Some(TRACEPARENT.into()),
            ..Default::default()
        };
        assert_eq!(req.trace_headers(), expected);
        let req = ResourceDeprovisionRequest {
            trace_context: Some(TRACEPARENT.into()),
            ..Default::default()
        };
        assert_eq!(req.trace_headers(), expected);
        assert!(ResourceDeprovisionRequest::default()
            .trace_headers()
            .is_empty());
    }

    // These names are a contract with deckwatch, which copies them by hand.
    #[test]
    fn telemetry_names_are_pinned() {
        use telemetry::{attribute, code};
        assert_eq!(attribute::DECKWATCH_PLUGIN_NAME, "deckwatch.plugin.name");
        assert_eq!(
            attribute::DECKWATCH_PLUGIN_VERSION,
            "deckwatch.plugin.version"
        );
        assert_eq!(
            attribute::DECKWATCH_PLUGIN_EXPORT,
            "deckwatch.plugin.export"
        );
        assert_eq!(telemetry::TRACEPARENT_HEADER, "traceparent");
        assert_eq!(code::RESERVED_PREFIX, "plugin.");
        let codes = [
            (code::UNSPECIFIED, "plugin.unspecified"),
            (code::FETCH_FAILED, "plugin.fetch_failed"),
            (code::CHECKSUM_MISMATCH, "plugin.checksum_mismatch"),
            (code::LOAD_FAILED, "plugin.load_failed"),
            (code::TRAP, "plugin.trap"),
            (code::TIMEOUT, "plugin.timeout"),
            (code::OUTPUT_INVALID, "plugin.output_invalid"),
            (code::K8S_APPLY_FAILED, "plugin.k8s_apply_failed"),
            (code::POLICY_REJECTED, "plugin.policy_rejected"),
        ];
        for (actual, expected) in codes {
            assert_eq!(actual, expected);
            assert!(actual.starts_with(code::RESERVED_PREFIX));
        }
    }
}
