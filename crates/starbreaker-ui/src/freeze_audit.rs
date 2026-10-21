//! Freeze-file delta audit (crates/starbreaker-ui/docs/ui-process-improvements.md item 5).
//!
//! Diffs two IR snapshot-freeze documents (`ui_snapshot_freeze.json` shape:
//! `targets[] -> baseline_snapshot.elements[]` keyed by `identity`) and
//! returns one [`FreezeDelta`] per changed scalar field, plus ADDED/REMOVED
//! identities and targets. The freeze tool prints this report and embeds it
//! in the written freeze so every re-freeze is self-auditing: the
//! `--reason` (and the commit message) must account for every delta.

use serde::Serialize;

/// One audited difference between the existing freeze and the new snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FreezeDelta {
    pub target: String,
    /// `<node_id>:<node_type>` element identity, or `<target>` itself for
    /// target-level additions/removals.
    pub identity: String,
    /// Changed field name, or `"<added>"` / `"<removed>"`.
    pub field: String,
    pub old: serde_json::Value,
    pub new: serde_json::Value,
}

/// Diff two freeze documents. Fields compared are the scalar (non-object,
/// non-array) members of each element — geometry, alpha, payloads, tints,
/// font identity/size, text tops; nested structures compare as wholes.
pub fn diff_freeze_documents(
    old_doc: &serde_json::Value,
    new_doc: &serde_json::Value,
) -> Vec<FreezeDelta> {
    let mut deltas = Vec::new();
    let old_targets = freeze_targets(old_doc);
    let new_targets = freeze_targets(new_doc);

    for (target_id, old_elements) in &old_targets {
        let Some(new_elements) = new_targets.get(target_id) else {
            deltas.push(FreezeDelta {
                target: target_id.clone(),
                identity: target_id.clone(),
                field: "<removed>".into(),
                old: serde_json::Value::String("target".into()),
                new: serde_json::Value::Null,
            });
            continue;
        };
        for (identity, old_el) in old_elements {
            let Some(new_el) = new_elements.get(identity) else {
                deltas.push(FreezeDelta {
                    target: target_id.clone(),
                    identity: identity.clone(),
                    field: "<removed>".into(),
                    old: serde_json::Value::String("element".into()),
                    new: serde_json::Value::Null,
                });
                continue;
            };
            let (Some(old_obj), Some(new_obj)) = (old_el.as_object(), new_el.as_object()) else {
                continue;
            };
            let mut fields: Vec<&String> = old_obj.keys().chain(new_obj.keys()).collect();
            fields.sort();
            fields.dedup();
            for field in fields {
                let old_v = old_obj.get(field.as_str()).cloned().unwrap_or(serde_json::Value::Null);
                let new_v = new_obj.get(field.as_str()).cloned().unwrap_or(serde_json::Value::Null);
                if old_v != new_v {
                    deltas.push(FreezeDelta {
                        target: target_id.clone(),
                        identity: identity.clone(),
                        field: field.clone(),
                        old: old_v,
                        new: new_v,
                    });
                }
            }
        }
        for identity in new_elements.keys() {
            if !old_elements.contains_key(identity) {
                deltas.push(FreezeDelta {
                    target: target_id.clone(),
                    identity: identity.clone(),
                    field: "<added>".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::Value::String("element".into()),
                });
            }
        }
    }
    for target_id in new_targets.keys() {
        if !old_targets.contains_key(target_id) {
            deltas.push(FreezeDelta {
                target: target_id.clone(),
                identity: target_id.clone(),
                field: "<added>".into(),
                old: serde_json::Value::Null,
                new: serde_json::Value::String("target".into()),
            });
        }
    }
    deltas
}

type ElementsByIdentity = std::collections::BTreeMap<String, serde_json::Value>;

fn freeze_targets(doc: &serde_json::Value) -> std::collections::BTreeMap<String, ElementsByIdentity> {
    let mut out = std::collections::BTreeMap::new();
    let Some(targets) = doc.get("targets").and_then(|v| v.as_array()) else {
        return out;
    };
    for target in targets {
        let Some(id) = target.get("id").and_then(|v| v.as_str()) else { continue };
        let mut elements = ElementsByIdentity::new();
        if let Some(els) = target
            .get("baseline_snapshot")
            .and_then(|s| s.get("elements"))
            .and_then(|v| v.as_array())
        {
            for el in els {
                if let Some(identity) = el.get("identity").and_then(|v| v.as_str()) {
                    elements.insert(identity.to_string(), el.clone());
                }
            }
        }
        out.insert(id.to_string(), elements);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(elements_a: serde_json::Value) -> serde_json::Value {
        json!({
            "schema_version": 1,
            "targets": [{
                "id": "target_a",
                "baseline_snapshot": { "elements": elements_a }
            }]
        })
    }

    /// The gold re-freeze that motivated this audit changed exactly one
    /// field of one element — the report must say precisely that.
    #[test]
    fn reports_changed_fields_per_identity() {
        let old = doc(json!([
            {"identity": "40:widget_custom_shape", "h": 0.18, "alpha": 0.1},
            {"identity": "41:widget_text_field", "text_payload": "OUTPUT"}
        ]));
        let new = doc(json!([
            {"identity": "40:widget_custom_shape", "h": 194.4, "alpha": 0.1},
            {"identity": "41:widget_text_field", "text_payload": "OUTPUT"}
        ]));
        let deltas = diff_freeze_documents(&old, &new);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].target, "target_a");
        assert_eq!(deltas[0].identity, "40:widget_custom_shape");
        assert_eq!(deltas[0].field, "h");
        assert_eq!(deltas[0].old, json!(0.18));
        assert_eq!(deltas[0].new, json!(194.4));
    }

    /// Added/removed elements and identical documents are reported correctly.
    #[test]
    fn reports_added_removed_and_no_change() {
        let old = doc(json!([{"identity": "1:a", "x": 1.0}]));
        let same = diff_freeze_documents(&old, &old);
        assert!(same.is_empty(), "identical docs produce no deltas");

        let new = doc(json!([{"identity": "2:b", "x": 1.0}]));
        let deltas = diff_freeze_documents(&old, &new);
        let fields: Vec<(&str, &str)> = deltas
            .iter()
            .map(|d| (d.identity.as_str(), d.field.as_str()))
            .collect();
        assert!(fields.contains(&("1:a", "<removed>")));
        assert!(fields.contains(&("2:b", "<added>")));
    }
}
