//! Names that deckwatch and plugins share for telemetry and plugin events.
//!
//! Plugins don't depend on OpenTelemetry; the host traces them. The design
//! (deckwatch's observability plan) is that deckwatch wraps every plugin call
//! in a span named `plugin {export}` carrying the [`attribute`] names below,
//! and extism turns `extism_pdk` log lines into host `tracing` events inside
//! that span. A plugin's part is to:
//!
//! - report problems as [`PluginEvent`](crate::PluginEvent)s, with codes of its
//!   own (`"<plugin-name>.<code>"`), never the host's [`code`]s;
//! - forward the host's trace on its own HTTP calls, with `trace_headers()` on
//!   [`PluginContext`](crate::PluginContext::trace_headers),
//!   [`ResourceProvisionRequest`](crate::ResourceProvisionRequest::trace_headers)
//!   and [`ResourceDeprovisionRequest`](crate::ResourceDeprovisionRequest::trace_headers).
//!
//! As of deckwatch `fc03ec2` the host side doesn't exist yet: no span, no
//! `trace_context`, and `events` are ignored.

/// Attribute names on the host's span around a plugin call.
///
/// Each constant is named after its attribute, as in
/// `opentelemetry-semantic-conventions`.
pub mod attribute {
    /// The plugin's name.
    pub const DECKWATCH_PLUGIN_NAME: &str = "deckwatch.plugin.name";
    /// The plugin's version
    /// ([`PluginMetadata::version`](crate::PluginMetadata::version)).
    pub const DECKWATCH_PLUGIN_VERSION: &str = "deckwatch.plugin.version";
    /// The call the span covers, such as `apply` or `provision`. The span is
    /// named `plugin {export}`.
    pub const DECKWATCH_PLUGIN_EXPORT: &str = "deckwatch.plugin.export";
}

/// [`PluginEvent::code`](crate::PluginEvent::code) values reserved for the host.
///
/// Every code that starts with [`RESERVED_PREFIX`](code::RESERVED_PREFIX)
/// belongs to the host, for failures it detects around a plugin call. Plugins
/// use `"<plugin-name>.<code>"` instead.
pub mod code {
    /// The prefix of every host code. Plugins must not use it.
    pub const RESERVED_PREFIX: &str = "plugin.";

    /// A legacy `errors`, `warnings` or `message` string, which has no code
    /// of its own.
    pub const UNSPECIFIED: &str = "plugin.unspecified";
    /// The plugin's binary couldn't be fetched.
    pub const FETCH_FAILED: &str = "plugin.fetch_failed";
    /// The fetched binary doesn't match the configured `sha256`.
    pub const CHECKSUM_MISMATCH: &str = "plugin.checksum_mismatch";
    /// The binary didn't load or instantiate as a plugin.
    pub const LOAD_FAILED: &str = "plugin.load_failed";
    /// The plugin trapped (for example, it panicked) during a call.
    pub const TRAP: &str = "plugin.trap";
    /// The call ran past the host's time limit.
    pub const TIMEOUT: &str = "plugin.timeout";
    /// The plugin's output isn't valid JSON for the expected type. The
    /// message should include the deserializer's error path, so drift between
    /// the SDK and the host can be debugged.
    pub const OUTPUT_INVALID: &str = "plugin.output_invalid";
    /// Applying a Kubernetes object the plugin returned failed.
    pub const K8S_APPLY_FAILED: &str = "plugin.k8s_apply_failed";
    /// A host policy refused something the plugin asked for.
    pub const POLICY_REJECTED: &str = "plugin.policy_rejected";
}

/// The W3C Trace Context HTTP header that carries a `trace_context` value.
pub const TRACEPARENT_HEADER: &str = "traceparent";

/// The outbound HTTP headers for a `trace_context` value: [`TRACEPARENT_HEADER`]
/// when it's a valid version-00 W3C `traceparent`, otherwise none.
pub(crate) fn trace_headers(trace_context: Option<&str>) -> Vec<(&'static str, String)> {
    match trace_context {
        Some(value) if is_valid_traceparent(value) => {
            vec![(TRACEPARENT_HEADER, value.to_owned())]
        }
        _ => Vec::new(),
    }
}

/// Whether `value` is a version-00 W3C `traceparent`:
/// `00-<trace-id>-<parent-id>-<trace-flags>`, with 32, 16 and 2 lowercase hex
/// digits, and neither id all zeros. Anything else (other versions, uppercase,
/// whitespace, extra fields) is refused, so the value is always safe to send
/// as a header.
fn is_valid_traceparent(value: &str) -> bool {
    let b = value.as_bytes();
    let lower_hex = |s: &[u8]| s.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
    let not_all_zeros = |s: &[u8]| s.iter().any(|&c| c != b'0');
    b.len() == 55
        && b.starts_with(b"00-")
        && b[35] == b'-'
        && b[52] == b'-'
        && lower_hex(&b[3..35])
        && not_all_zeros(&b[3..35])
        && lower_hex(&b[36..52])
        && not_all_zeros(&b[36..52])
        && lower_hex(&b[53..55])
}
