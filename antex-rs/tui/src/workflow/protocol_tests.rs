use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

fn manifest() -> WorkflowManifest {
    serde_json::from_value(json!({
        "id": "ruff-cleanup",
        "title": "Ruff cleanup",
        "fields": [
            {"id": "scope", "label": "Scope", "type": "text", "default": "."},
            {"id": "parallelism", "label": "Parallelism", "type": "integer", "min": 1, "max": 8, "default": 4},
            {"id": "verify", "label": "Verify", "type": "boolean", "default": true},
            {"id": "model", "label": "Model", "type": "select", "options": [
                {"value": "fast", "label": "Fast"},
                {"value": "default", "label": "Default"}
            ], "default": "fast"}
        ]
    }))
    .expect("manifest should deserialize")
}

#[test]
fn validates_and_parses_all_field_kinds() {
    let manifest = manifest();
    assert_eq!(manifest.validate(), Ok(()));
    assert_eq!(manifest.fields[0].parse_answer("src"), Ok(json!("src")));
    assert_eq!(manifest.fields[1].parse_answer("6"), Ok(json!(6)));
    assert_eq!(manifest.fields[2].parse_answer("false"), Ok(json!(false)));
    assert_eq!(
        manifest.fields[3].parse_answer("default"),
        Ok(json!("default"))
    );
}

#[test]
fn rejects_duplicate_fields_and_out_of_range_defaults() {
    let mut duplicate = manifest();
    duplicate.fields.push(duplicate.fields[0].clone());
    assert_eq!(
        duplicate.validate(),
        Err("workflow `ruff-cleanup` repeats field `scope`".to_string())
    );

    let mut out_of_range = manifest();
    out_of_range.fields[1].default = Some(json!(20));
    assert_eq!(
        out_of_range.validate(),
        Err("integer field `parallelism` default or answer is outside its range".to_string())
    );
}

#[test]
fn supports_month_long_persistent_workflows() {
    let mut manifest = manifest();
    manifest.guardrails.max_agent_calls = 50_000;
    manifest.guardrails.timeout_seconds = 2_592_000;
    assert_eq!(manifest.validate(), Ok(()));

    manifest.guardrails.max_agent_calls += 1;
    assert_eq!(
        manifest.validate(),
        Err("max_agent_calls must be between 1 and 50000".to_string())
    );
}

#[test]
fn model_fields_preserve_selected_identifiers_and_validate_defaults() {
    let mut field: WorkflowField = serde_json::from_value(json!({
        "id": "reviewer", "label": "Reviewer", "type": "model", "default": "custom-model"
    }))
    .expect("field");
    assert!(field.uses_model_picker());
    assert_eq!(field.validate(), Ok(()));
    assert_eq!(field.parse_answer(""), Ok(json!("")));
    assert_eq!(field.parse_answer("model-id"), Ok(json!("model-id")));
    field.required = true;
    assert!(field.parse_answer("").is_err());
    field.default = Some(json!(42));
    assert!(field.validate().is_err());
}
