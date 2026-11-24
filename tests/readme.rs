//! Keeps the README's type reference in sync with the types.
//!
//! Every `pub struct` / `pub enum` in `src/lib.rs` must have a `### `Name``
//! section under `## Type reference`, and the first table in that section must
//! list exactly the type's JSON keys (structs) or JSON values (enums). Adding,
//! removing or renaming a type, field or variant without updating the README
//! fails this test.

use std::collections::{BTreeMap, BTreeSet};

use deckwatch_plugin_sdk::*;
use serde::Serialize;

const README: &str = include_str!("../README.md");
const LIB: &str = include_str!("../src/lib.rs");

/// Names of all `pub struct` and `pub enum` items declared in `src/lib.rs`.
fn declared_types() -> BTreeSet<String> {
    LIB.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            l.strip_prefix("pub struct ")
                .or_else(|| l.strip_prefix("pub enum "))
        })
        .map(|rest| {
            rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// Type name -> first-column entries of the first table in its README section,
/// with backticks and quotes stripped.
fn readme_tables() -> BTreeMap<String, BTreeSet<String>> {
    let start = README
        .find("\n## Type reference")
        .expect("README has a `## Type reference` section");
    let section = &README[start + 1..];
    let end = section[1..].find("\n## ").map_or(section.len(), |i| i + 1);
    let section = &section[..end];

    let mut tables = BTreeMap::new();
    for chunk in section.split("\n### `").skip(1) {
        let name = chunk.split('`').next().unwrap().to_string();
        let mut rows = BTreeSet::new();
        let mut in_table = false;
        for line in chunk.lines().skip(1) {
            if line.starts_with("|---") {
                in_table = true;
                continue;
            }
            if in_table {
                if !line.starts_with('|') {
                    break;
                }
                let cell = line.split('|').nth(1).unwrap();
                let entry = cell.split('`').nth(1).unwrap_or_else(|| {
                    panic!("README `{name}` row has no backticked first cell: {line}")
                });
                rows.insert(entry.trim_matches('"').to_string());
            }
        }
        assert!(!rows.is_empty(), "README section `{name}` has no table");
        tables.insert(name, rows);
    }
    tables
}

fn keys<T: Serialize>(value: &T) -> BTreeSet<String> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .expect("struct serializes to a JSON object")
        .keys()
        .cloned()
        .collect()
}

fn variants<T: Serialize>(values: &[T]) -> BTreeSet<String> {
    values
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

/// JSON keys / values of every public type. Optional fields that are skipped
/// when empty are populated so that every key appears.
fn actual() -> BTreeMap<String, BTreeSet<String>> {
    let secret = SecretKeyRef {
        name: "s".into(),
        key: "k".into(),
    };
    let config_map = ConfigMapKeyRef {
        name: "c".into(),
        key: "k".into(),
    };
    let source = EnvVarSource {
        secret_key_ref: Some(secret.clone()),
        config_map_key_ref: Some(config_map.clone()),
    };
    let sa = WantsServiceAccount {
        name: "sa".into(),
        irsa_role_arn: "arn".into(),
    };

    let mut m = BTreeMap::new();
    let mut put = |name: &str, set: BTreeSet<String>| {
        m.insert(name.to_string(), set);
    };
    put(
        "PluginMetadata",
        keys(&PluginMetadata {
            min_deckwatch_version: Some("0.7.0".into()),
            ..Default::default()
        }),
    );
    put(
        "ConfigField",
        keys(&ConfigField {
            default: Some("d".into()),
            env_source: Some("E".into()),
            ..Default::default()
        }),
    );
    put(
        "ConfigFieldType",
        variants(&[
            ConfigFieldType::String,
            ConfigFieldType::Secret,
            ConfigFieldType::Bool,
            ConfigFieldType::KeyValue,
            ConfigFieldType::Select,
            ConfigFieldType::Unknown,
        ]),
    );
    put(
        "McpTuningField",
        keys(&McpTuningField {
            default: Some("d".into()),
            ..Default::default()
        }),
    );
    put("PluginResource", keys(&PluginResource::default()));
    put(
        "PluginContext",
        keys(
            &serde_json::from_value::<PluginContext>(serde_json::json!({
                "namespace": "ns",
                "deployment_name": "d",
                "annotations": {},
                "labels": {},
                "trace_context": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
            }))
            .unwrap(),
        ),
    );
    put(
        "PluginResult",
        keys(&PluginResult {
            service_account_name: Some("sa".into()),
            wants_service_account: Some(sa.clone()),
            errors: vec!["e".into()],
            warnings: vec!["w".into()],
            init_containers: vec![SidecarSpec::default()],
            events: vec![PluginEvent::default()],
            ..Default::default()
        }),
    );
    put("WantsServiceAccount", keys(&sa));
    put(
        "EnvVarSpec",
        keys(&EnvVarSpec {
            name: "N".into(),
            value: "v".into(),
            value_from: Some(source.clone()),
        }),
    );
    put("EnvVarSource", keys(&source));
    put("SecretKeyRef", keys(&secret));
    put("ConfigMapKeyRef", keys(&config_map));
    put(
        "SidecarSpec",
        keys(&SidecarSpec {
            name: "s".into(),
            image: "i".into(),
            port: Some(1),
            cpu_request: Some("c".into()),
            cpu_limit: Some("c".into()),
            memory_request: Some("m".into()),
            memory_limit: Some("m".into()),
            cpu: Some("c".into()),
            memory: Some("m".into()),
            ..Default::default()
        }),
    );
    put(
        "VolumeSpec",
        keys(&VolumeSpec {
            name: "v".into(),
            volume_type: "emptyDir".into(),
            source_name: Some("s".into()),
            size_limit: Some("1Mi".into()),
        }),
    );
    put(
        "VolumeMountSpec",
        keys(&VolumeMountSpec {
            name: "v".into(),
            mount_path: "/v".into(),
            read_only: true,
            sub_path: Some("p".into()),
        }),
    );
    put(
        "ResourceProvisionRequest",
        keys(&ResourceProvisionRequest {
            application_name: "a".into(),
            namespace: "ns".into(),
            resource_id: "r".into(),
            fields: Default::default(),
            trace_context: Some("t".into()),
        }),
    );
    put(
        "ResourceProvisionResult",
        keys(&ResourceProvisionResult {
            warnings: vec!["w".into()],
            retry_after_secs: Some(1),
            init_containers: vec![SidecarSpec::default()],
            events: vec![PluginEvent::default()],
            ..Default::default()
        }),
    );
    put(
        "ProvisioningStatus",
        variants(&[
            ProvisioningStatus::Pending,
            ProvisioningStatus::Available,
            ProvisioningStatus::Failed,
        ]),
    );
    put(
        "ResourceDeprovisionRequest",
        keys(&ResourceDeprovisionRequest {
            application_name: "a".into(),
            namespace: "ns".into(),
            resource_id: "r".into(),
            state: Default::default(),
            fields: Default::default(),
            trace_context: Some("t".into()),
        }),
    );
    put(
        "ResourceDeprovisionResult",
        keys(&ResourceDeprovisionResult {
            delete_kubernetes_resources: vec![KubernetesObjectRef::job("j")],
            events: vec![PluginEvent::default()],
            ..Default::default()
        }),
    );
    put(
        "KubernetesObjectRef",
        keys(&KubernetesObjectRef::service_account("sa")),
    );
    put(
        "PluginEvent",
        keys(&PluginEvent {
            resource_id: Some("r".into()),
            object: Some(KubernetesObjectRef::job("j")),
            hint: Some("h".into()),
            ..Default::default()
        }),
    );
    put(
        "EventSeverity",
        variants(&[
            EventSeverity::Info,
            EventSeverity::Warning,
            EventSeverity::Error,
            EventSeverity::Unknown,
        ]),
    );
    m
}

#[test]
fn readme_type_reference_matches_types() {
    let declared = declared_types();
    let actual = actual();
    let readme = readme_tables();

    let covered: BTreeSet<String> = actual.keys().cloned().collect();
    assert_eq!(
        declared, covered,
        "tests/readme.rs must cover every public type in src/lib.rs"
    );

    let documented: BTreeSet<String> = readme.keys().cloned().collect();
    assert_eq!(
        declared, documented,
        "README `## Type reference` must have a section for every public type"
    );

    for (name, fields) in &actual {
        assert_eq!(
            &readme[name], fields,
            "README table for `{name}` must list exactly its JSON keys or values"
        );
    }
}
