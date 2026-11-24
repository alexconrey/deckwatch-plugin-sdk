//! Wire compatibility with released SDK versions.
//!
//! deckwatch and plugins are built against different SDK versions, so every
//! type that crosses the boundary must stay readable in both directions:
//!
//! - **old → new:** a newer host reads output from a plugin built on an older
//!   SDK (and a newer plugin reads input from an older host);
//! - **new → old:** an older host reads output from a plugin built on this SDK.
//!
//! Each test serializes a fully populated value of one version and reads it
//! as the other version's type. Old → new: every key the old version sent must
//! survive unchanged ([`old_to_new`]). New → old: what the old type reads must
//! equal the same data built with the old type ([`new_to_old`]). Both builders
//! therefore use identical data for the fields they share. The old types come from the real
//! releases, pulled in as dev-dependencies (`v0_9`, `v0_3` in `Cargo.toml`):
//! v0.9.0 is the last release, and v0.3.0 is the oldest one plugins still pin.
//!
//! When you add a field, populate it in the `new` builders below. When you
//! cut a release, consider adding it as a new baseline.

use std::collections::HashMap;

use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

/// Checks that everything in `sent` is in `read` with the same value,
/// recursively. `read` may have extra keys (fields only the reader has).
fn assert_contained(sent: &Value, read: &Value, path: &str) {
    match (sent, read) {
        (Value::Object(s), Value::Object(r)) => {
            for (k, sv) in s {
                let p = format!("{path}.{k}");
                let rv = r
                    .get(k)
                    .unwrap_or_else(|| panic!("{p}: lost (reader has no such key)"));
                assert_contained(sv, rv, &p);
            }
        }
        (Value::Array(s), Value::Array(r)) => {
            assert_eq!(s.len(), r.len(), "{path}: array length differs");
            for (i, (sv, rv)) in s.iter().zip(r).enumerate() {
                assert_contained(sv, rv, &format!("{path}[{i}]"));
            }
        }
        _ => assert_eq!(sent, read, "{path}: value changed"),
    }
}

/// A newer reader must keep every field an older writer sent: serialize
/// `old`, read it as `New`, and check all of `old`'s JSON survives.
fn old_to_new<Old: Serialize, New: Serialize + DeserializeOwned>(old: &Old, what: &str) {
    let sent = serde_json::to_value(old).unwrap();
    let read: New = serde_json::from_value(sent.clone())
        .unwrap_or_else(|e| panic!("{what}: new type can't parse old JSON: {e}\n{sent}"));
    assert_contained(&sent, &serde_json::to_value(read).unwrap(), what);
}

/// An older reader must get every field it knows from a newer writer:
/// serialize `new`, read it as `Old`, and check the result equals `expected`,
/// the same data built with the old type.
fn new_to_old<New: Serialize, Old: Serialize + DeserializeOwned>(
    new: &New,
    expected: &Old,
    what: &str,
) {
    let sent = serde_json::to_value(new).unwrap();
    let read: Old = serde_json::from_value(sent.clone())
        .unwrap_or_else(|e| panic!("{what}: old type can't parse new JSON: {e}\n{sent}"));
    assert_eq!(
        serde_json::to_value(read).unwrap(),
        serde_json::to_value(expected).unwrap(),
        "{what}: old reader lost or changed data"
    );
}

fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn manifest() -> Value {
    json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "c", "namespace": "ns"}})
}

/// Fully populated values of the current types.
mod new {
    use super::*;
    use deckwatch_plugin_sdk::*;

    pub fn env_vars() -> Vec<EnvVarSpec> {
        vec![
            EnvVarSpec::value("PLAIN", "v"),
            EnvVarSpec::secret_key_ref("FROM_SECRET", "s", "k"),
            EnvVarSpec::config_map_key_ref("FROM_CM", "c", "k"),
        ]
    }

    pub fn sidecar() -> SidecarSpec {
        SidecarSpec {
            name: "agent".into(),
            image: "registry/agent:v1".into(),
            env: env_vars(),
            port: Some(8080),
            cpu_request: Some("50m".into()),
            cpu_limit: Some("200m".into()),
            memory_request: Some("64Mi".into()),
            memory_limit: Some("128Mi".into()),
            cpu: Some("100m".into()),
            memory: Some("96Mi".into()),
            volumes: vec![VolumeSpec {
                name: "shared".into(),
                volume_type: "emptyDir".into(),
                source_name: Some("src".into()),
                size_limit: Some("1Gi".into()),
            }],
            volume_mounts: vec![mount()],
            primary_volume_mounts: vec![mount()],
        }
    }

    fn mount() -> VolumeMountSpec {
        VolumeMountSpec {
            name: "shared".into(),
            mount_path: "/shared".into(),
            read_only: true,
            sub_path: Some("sub".into()),
        }
    }

    /// Only the variants v0.9.0 knows; `KeyValue` is tested separately.
    pub fn config_field(field_type: ConfigFieldType) -> ConfigField {
        ConfigField {
            key: "K".into(),
            label: "L".into(),
            description: "D".into(),
            field_type,
            default: Some("d".into()),
            required: true,
            options: strings(&["a", "b"]),
            env_source: Some("ENV".into()),
        }
    }

    pub fn metadata() -> PluginMetadata {
        PluginMetadata {
            name: "p".into(),
            version: "1.0.0".into(),
            description: "d".into(),
            provides: strings(&["x:a"]),
            depends_on: strings(&["x:b"]),
            optional_depends_on: strings(&["x:c"]),
            config_schema: vec![
                config_field(ConfigFieldType::String),
                config_field(ConfigFieldType::Secret),
                config_field(ConfigFieldType::Bool),
                config_field(ConfigFieldType::Select),
            ],
            resources: vec![PluginResource {
                id: "rds".into(),
                label: "RDS".into(),
                icon: "mdi-database".into(),
                description: "d".into(),
                singleton: true,
                fields: vec![config_field(ConfigFieldType::String)],
                output_keys: strings(&["DB_HOST"]),
            }],
            mcp_tuning_fields: vec![McpTuningField {
                key: "k".into(),
                label: "l".into(),
                description: "d".into(),
                placeholder: "p".into(),
                default: Some("d".into()),
            }],
            min_deckwatch_version: Some("0.7.0".into()),
        }
    }

    pub fn context() -> PluginContext {
        PluginContext {
            namespace: "ns".into(),
            deployment_name: "app".into(),
            annotations: map(&[("a", "1")]),
            labels: map(&[("l", "1")]),
            plugin_outputs: HashMap::from([("aws".into(), map(&[("role_arn", "arn")]))]),
            provisioned_resources: HashMap::from([("aws/rds".into(), map(&[("DB_HOST", "h")]))]),
        }
    }

    pub fn result() -> PluginResult {
        PluginResult {
            env_vars: env_vars(),
            sidecars: vec![sidecar()],
            kubernetes_resources: vec![manifest()],
            outputs: map(&[("o", "1")]),
            service_account_name: Some("sa".into()),
            wants_service_account: Some(WantsServiceAccount {
                name: "sa".into(),
                irsa_role_arn: "arn".into(),
            }),
            deployment_annotations: map(&[("a", "1")]),
            deployment_labels: map(&[("l", "1")]),
            errors: strings(&["e"]),
            warnings: strings(&["w"]),
            init_containers: vec![sidecar()],
        }
    }

    pub fn provision_request() -> ResourceProvisionRequest {
        ResourceProvisionRequest {
            application_name: "app".into(),
            namespace: "ns".into(),
            resource_id: "rds".into(),
            fields: map(&[("f", "1")]),
        }
    }

    pub fn provision_result(status: ProvisioningStatus) -> ResourceProvisionResult {
        ResourceProvisionResult {
            state: map(&[("DB_HOST", "h")]),
            deployment_annotations: map(&[("a", "1")]),
            sidecars: vec![sidecar()],
            init_containers: vec![sidecar()],
            kubernetes_resources: vec![manifest()],
            errors: strings(&["e"]),
            warnings: strings(&["w"]),
            status,
            retry_after_secs: Some(30),
        }
    }

    pub fn deprovision_request() -> ResourceDeprovisionRequest {
        ResourceDeprovisionRequest {
            application_name: "app".into(),
            namespace: "ns".into(),
            resource_id: "rds".into(),
            state: map(&[("DB_HOST", "h")]),
            fields: map(&[("f", "1")]),
        }
    }

    pub fn deprovision_result() -> ResourceDeprovisionResult {
        ResourceDeprovisionResult {
            errors: strings(&["e"]),
            message: "m".into(),
            delete_kubernetes_resources: vec![
                KubernetesObjectRef::service_account("sa"),
                KubernetesObjectRef::job("j"),
            ],
        }
    }
}

/// Fully populated values of the v0.9.0 types.
mod old09 {
    use super::*;
    use v0_9::*;

    pub fn env_vars() -> Vec<EnvVarSpec> {
        vec![
            EnvVarSpec::value("PLAIN", "v"),
            EnvVarSpec::secret_key_ref("FROM_SECRET", "s", "k"),
            EnvVarSpec::config_map_key_ref("FROM_CM", "c", "k"),
        ]
    }

    pub fn sidecar() -> SidecarSpec {
        SidecarSpec {
            name: "agent".into(),
            image: "registry/agent:v1".into(),
            env: env_vars(),
            port: Some(8080),
            cpu: Some("100m".into()),
            memory: Some("96Mi".into()),
        }
    }

    fn config_field(field_type: ConfigFieldType) -> ConfigField {
        ConfigField {
            key: "K".into(),
            label: "L".into(),
            description: "D".into(),
            field_type,
            default: Some("d".into()),
            required: true,
            options: strings(&["a", "b"]),
            env_source: Some("ENV".into()),
        }
    }

    pub fn metadata() -> PluginMetadata {
        PluginMetadata {
            name: "p".into(),
            version: "1.0.0".into(),
            description: "d".into(),
            provides: strings(&["x:a"]),
            depends_on: strings(&["x:b"]),
            optional_depends_on: strings(&["x:c"]),
            config_schema: vec![
                config_field(ConfigFieldType::String),
                config_field(ConfigFieldType::Secret),
                config_field(ConfigFieldType::Bool),
                config_field(ConfigFieldType::Select),
            ],
            resources: vec![PluginResource {
                id: "rds".into(),
                label: "RDS".into(),
                icon: "mdi-database".into(),
                description: "d".into(),
                singleton: true,
                fields: vec![config_field(ConfigFieldType::String)],
                output_keys: strings(&["DB_HOST"]),
            }],
        }
    }

    pub fn context() -> PluginContext {
        PluginContext {
            namespace: "ns".into(),
            deployment_name: "app".into(),
            annotations: map(&[("a", "1")]),
            labels: map(&[("l", "1")]),
            plugin_outputs: HashMap::from([("aws".into(), map(&[("role_arn", "arn")]))]),
            provisioned_resources: HashMap::from([("aws/rds".into(), map(&[("DB_HOST", "h")]))]),
        }
    }

    pub fn result() -> PluginResult {
        PluginResult {
            env_vars: env_vars(),
            sidecars: vec![sidecar()],
            kubernetes_resources: vec![manifest()],
            outputs: map(&[("o", "1")]),
            service_account_name: Some("sa".into()),
            wants_service_account: Some(WantsServiceAccount {
                name: "sa".into(),
                irsa_role_arn: "arn".into(),
            }),
            deployment_annotations: map(&[("a", "1")]),
            deployment_labels: map(&[("l", "1")]),
            errors: strings(&["e"]),
            warnings: strings(&["w"]),
        }
    }

    pub fn provision_request() -> ResourceProvisionRequest {
        ResourceProvisionRequest {
            application_name: "app".into(),
            namespace: "ns".into(),
            resource_id: "rds".into(),
            fields: map(&[("f", "1")]),
        }
    }

    pub fn provision_result(status: ProvisioningStatus) -> ResourceProvisionResult {
        ResourceProvisionResult {
            state: map(&[("DB_HOST", "h")]),
            deployment_annotations: map(&[("a", "1")]),
            sidecars: vec![sidecar()],
            kubernetes_resources: vec![manifest()],
            errors: strings(&["e"]),
            warnings: strings(&["w"]),
            status,
            retry_after_secs: Some(30),
        }
    }

    pub fn deprovision_request() -> ResourceDeprovisionRequest {
        ResourceDeprovisionRequest {
            application_name: "app".into(),
            namespace: "ns".into(),
            resource_id: "rds".into(),
            state: map(&[("DB_HOST", "h")]),
            fields: map(&[("f", "1")]),
        }
    }

    pub fn deprovision_result() -> ResourceDeprovisionResult {
        ResourceDeprovisionResult {
            errors: strings(&["e"]),
            message: "m".into(),
        }
    }
}

/// Fully populated values of the v0.3.0 types (the oldest SDK plugins pin).
mod old03 {
    use super::*;
    use v0_3::*;

    fn env_vars() -> Vec<EnvVarSpec> {
        vec![
            EnvVarSpec::value("PLAIN", "v"),
            EnvVarSpec::secret_key_ref("FROM_SECRET", "s", "k"),
            EnvVarSpec::config_map_key_ref("FROM_CM", "c", "k"),
        ]
    }

    pub fn context() -> PluginContext {
        PluginContext {
            namespace: "ns".into(),
            deployment_name: "app".into(),
            annotations: map(&[("a", "1")]),
            labels: map(&[("l", "1")]),
        }
    }

    pub fn result() -> PluginResult {
        PluginResult {
            env_vars: env_vars(),
            sidecars: vec![SidecarSpec {
                name: "agent".into(),
                image: "registry/agent:v1".into(),
                env: env_vars(),
                port: Some(8080),
                cpu: Some("100m".into()),
                memory: Some("96Mi".into()),
            }],
            kubernetes_resources: vec![manifest()],
            service_account_name: Some("sa".into()),
        }
    }
}

mod v0_9_to_new {
    use super::*;
    use deckwatch_plugin_sdk as n;
    use v0_9 as o;

    #[test]
    fn plugin_output() {
        old_to_new::<_, n::PluginMetadata>(&old09::metadata(), "PluginMetadata");
        old_to_new::<_, n::PluginResult>(&old09::result(), "PluginResult");
        for s in [
            o::ProvisioningStatus::Pending,
            o::ProvisioningStatus::Available,
            o::ProvisioningStatus::Failed,
        ] {
            old_to_new::<_, n::ResourceProvisionResult>(
                &old09::provision_result(s),
                "ResourceProvisionResult",
            );
        }
        old_to_new::<_, n::ResourceDeprovisionResult>(
            &old09::deprovision_result(),
            "ResourceDeprovisionResult",
        );
    }

    #[test]
    fn host_input() {
        old_to_new::<_, n::PluginContext>(&old09::context(), "PluginContext");
        old_to_new::<_, n::ResourceProvisionRequest>(
            &old09::provision_request(),
            "ResourceProvisionRequest",
        );
        old_to_new::<_, n::ResourceDeprovisionRequest>(
            &old09::deprovision_request(),
            "ResourceDeprovisionRequest",
        );
    }
}

mod new_to_v0_9 {
    use super::*;
    use deckwatch_plugin_sdk as n;
    use v0_9 as o;

    #[test]
    fn plugin_output() {
        new_to_old(&new::metadata(), &old09::metadata(), "PluginMetadata");
        new_to_old(&new::result(), &old09::result(), "PluginResult");
        for (new_status, old_status) in [
            (
                n::ProvisioningStatus::Pending,
                o::ProvisioningStatus::Pending,
            ),
            (
                n::ProvisioningStatus::Available,
                o::ProvisioningStatus::Available,
            ),
            (n::ProvisioningStatus::Failed, o::ProvisioningStatus::Failed),
        ] {
            new_to_old(
                &new::provision_result(new_status),
                &old09::provision_result(old_status),
                "ResourceProvisionResult",
            );
        }
        new_to_old(
            &new::deprovision_result(),
            &old09::deprovision_result(),
            "ResourceDeprovisionResult",
        );
    }

    #[test]
    fn host_input() {
        new_to_old(&new::context(), &old09::context(), "PluginContext");
        new_to_old(
            &new::provision_request(),
            &old09::provision_request(),
            "ResourceProvisionRequest",
        );
        new_to_old(
            &new::deprovision_request(),
            &old09::deprovision_request(),
            "ResourceDeprovisionRequest",
        );
    }

    /// The one known break: v0.9.0's `ConfigFieldType` has no `key_value` and
    /// no catch-all, so it rejects the field, and the whole `PluginMetadata`
    /// with it. The README tells plugin authors not to use `key_value` until
    /// their hosts support it. If this starts passing, update the README.
    #[test]
    fn key_value_field_type_is_rejected() {
        let mut m = new::metadata();
        m.config_schema = vec![new::config_field(n::ConfigFieldType::KeyValue)];
        let json = serde_json::to_value(&m).unwrap();
        assert!(serde_json::from_value::<o::PluginMetadata>(json).is_err());
    }
}

mod v0_3_to_new {
    use super::*;
    use deckwatch_plugin_sdk as n;

    #[test]
    fn plugin_output() {
        old_to_new::<_, n::PluginResult>(&old03::result(), "PluginResult");
    }

    #[test]
    fn host_input() {
        old_to_new::<_, n::PluginContext>(&old03::context(), "PluginContext");
    }
}

mod new_to_v0_3 {
    use super::*;

    #[test]
    fn plugin_output() {
        new_to_old(&new::result(), &old03::result(), "PluginResult");
    }

    #[test]
    fn host_input() {
        new_to_old(&new::context(), &old03::context(), "PluginContext");
    }
}
