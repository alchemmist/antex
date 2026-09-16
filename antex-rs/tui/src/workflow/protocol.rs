use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

const MAX_MANIFEST_FIELDS: usize = 32;
const MAX_OPTIONS_PER_FIELD: usize = 64;
const MAX_TEXT_LEN: usize = 4_096;
const MAX_AGENT_CALLS: u32 = 50_000;
const MAX_WORKFLOW_TIMEOUT_SECONDS: u64 = 2_592_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkflowDefinition {
    pub(crate) manifest: WorkflowManifest,
    pub(crate) script_path: PathBuf,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkflowManifest {
    pub(crate) id: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default = "default_version")]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) fields: Vec<WorkflowField>,
    #[serde(default)]
    pub(crate) guardrails: WorkflowGuardrails,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkflowField {
    pub(crate) id: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(flatten)]
    pub(crate) kind: WorkflowFieldKind,
    #[serde(default)]
    pub(crate) required: bool,
    #[serde(default)]
    pub(crate) default: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum WorkflowFieldKind {
    Model,
    Text {
        #[serde(default)]
        placeholder: String,
    },
    Integer {
        #[serde(default)]
        min: Option<i64>,
        #[serde(default)]
        max: Option<i64>,
    },
    Boolean,
    Select {
        options: Vec<WorkflowOption>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkflowOption {
    pub(crate) value: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkflowGuardrails {
    #[serde(default = "default_max_agent_calls")]
    pub(crate) max_agent_calls: u32,
    #[serde(default = "default_max_shell_calls")]
    pub(crate) max_shell_calls: u32,
    #[serde(default = "default_max_parallel_agents")]
    pub(crate) max_parallel_agents: usize,
    #[serde(default = "default_timeout_seconds")]
    pub(crate) timeout_seconds: u64,
}

impl Default for WorkflowGuardrails {
    fn default() -> Self {
        Self {
            max_agent_calls: default_max_agent_calls(),
            max_shell_calls: default_max_shell_calls(),
            max_parallel_agents: default_max_parallel_agents(),
            timeout_seconds: default_timeout_seconds(),
        }
    }
}

impl WorkflowManifest {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_id("workflow", &self.id)?;
        validate_text("workflow title", &self.title, /*required*/ true)?;
        validate_text(
            "workflow description",
            &self.description,
            /*required*/ false,
        )?;
        if self.fields.len() > MAX_MANIFEST_FIELDS {
            return Err(format!(
                "workflow `{}` declares {} fields; at most {MAX_MANIFEST_FIELDS} are allowed",
                self.id,
                self.fields.len()
            ));
        }
        let mut field_ids = HashSet::new();
        for field in &self.fields {
            validate_id("field", &field.id)?;
            if !field_ids.insert(field.id.as_str()) {
                return Err(format!(
                    "workflow `{}` repeats field `{}`",
                    self.id, field.id
                ));
            }
            validate_text("field label", &field.label, /*required*/ true)?;
            validate_text(
                "field description",
                &field.description,
                /*required*/ false,
            )?;
            field.validate()?;
        }
        if self.guardrails.max_agent_calls == 0 || self.guardrails.max_agent_calls > MAX_AGENT_CALLS
        {
            return Err(format!(
                "max_agent_calls must be between 1 and {MAX_AGENT_CALLS}"
            ));
        }
        if self.guardrails.max_shell_calls == 0 || self.guardrails.max_shell_calls > 10_000 {
            return Err("max_shell_calls must be between 1 and 10000".to_string());
        }
        if !(1..=16).contains(&self.guardrails.max_parallel_agents) {
            return Err("max_parallel_agents must be between 1 and 16".to_string());
        }
        if !(1..=MAX_WORKFLOW_TIMEOUT_SECONDS).contains(&self.guardrails.timeout_seconds) {
            return Err(format!(
                "timeout_seconds must be between 1 and {MAX_WORKFLOW_TIMEOUT_SECONDS}"
            ));
        }
        Ok(())
    }
}

impl WorkflowField {
    pub(crate) fn uses_model_picker(&self) -> bool {
        matches!(self.kind, WorkflowFieldKind::Model)
            || (matches!(self.kind, WorkflowFieldKind::Text { .. })
                && (self.id == "model" || self.id.ends_with("_model")))
    }

    fn validate(&self) -> Result<(), String> {
        match &self.kind {
            WorkflowFieldKind::Text { placeholder } => {
                validate_text("field placeholder", placeholder, /*required*/ false)?;
                if let Some(default) = &self.default
                    && !default.is_string()
                {
                    return Err(format!("text field `{}` has a non-string default", self.id));
                }
            }
            WorkflowFieldKind::Model => {
                if self
                    .default
                    .as_ref()
                    .is_some_and(|value| !value.is_string())
                {
                    return Err(format!(
                        "model field `{}` has a non-string default",
                        self.id
                    ));
                }
            }
            WorkflowFieldKind::Integer { min, max } => {
                if let (Some(min), Some(max)) = (min, max)
                    && min > max
                {
                    return Err(format!(
                        "integer field `{}` has min greater than max",
                        self.id
                    ));
                }
                if let Some(default) = &self.default {
                    let value = default.as_i64().ok_or_else(|| {
                        format!("integer field `{}` has a non-integer default", self.id)
                    })?;
                    validate_integer_range(&self.id, value, *min, *max)?;
                }
            }
            WorkflowFieldKind::Boolean => {
                if let Some(default) = &self.default
                    && !default.is_boolean()
                {
                    return Err(format!(
                        "boolean field `{}` has a non-boolean default",
                        self.id
                    ));
                }
            }
            WorkflowFieldKind::Select { options } => {
                if options.is_empty() || options.len() > MAX_OPTIONS_PER_FIELD {
                    return Err(format!(
                        "select field `{}` must declare between 1 and {MAX_OPTIONS_PER_FIELD} options",
                        self.id
                    ));
                }
                let mut values = HashSet::new();
                for option in options {
                    validate_text("option value", &option.value, /*required*/ true)?;
                    validate_text("option label", &option.label, /*required*/ true)?;
                    validate_text(
                        "option description",
                        &option.description,
                        /*required*/ false,
                    )?;
                    if !values.insert(option.value.as_str()) {
                        return Err(format!(
                            "select field `{}` repeats option value `{}`",
                            self.id, option.value
                        ));
                    }
                }
                if let Some(default) = &self.default {
                    let default = default.as_str().ok_or_else(|| {
                        format!("select field `{}` has a non-string default", self.id)
                    })?;
                    if !values.contains(default) {
                        return Err(format!(
                            "select field `{}` default `{default}` is not an option",
                            self.id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn parse_answer(&self, answer: &str) -> Result<Value, String> {
        match &self.kind {
            WorkflowFieldKind::Text { .. } | WorkflowFieldKind::Model => {
                if self.required && answer.trim().is_empty() {
                    return Err(format!("{} is required", self.label));
                }
                Ok(Value::String(answer.to_string()))
            }
            WorkflowFieldKind::Integer { min, max } => {
                let value = answer
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| format!("{} must be an integer", self.label))?;
                validate_integer_range(&self.id, value, *min, *max)?;
                Ok(Value::Number(value.into()))
            }
            WorkflowFieldKind::Boolean => answer
                .parse::<bool>()
                .map(Value::Bool)
                .map_err(|_| format!("{} must be true or false", self.label)),
            WorkflowFieldKind::Select { options } => options
                .iter()
                .find(|option| option.value == answer)
                .map(|option| Value::String(option.value.clone()))
                .ok_or_else(|| format!("{} has an unknown option", self.label)),
        }
    }
}

fn validate_integer_range(
    id: &str,
    value: i64,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<(), String> {
    if min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max) {
        return Err(format!(
            "integer field `{id}` default or answer is outside its range"
        ));
    }
    Ok(())
}

fn validate_id(kind: &str, id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        });
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{kind} id `{id}` must contain only lowercase ASCII letters, digits, '-' or '_'"
        ))
    }
}

fn validate_text(kind: &str, text: &str, required: bool) -> Result<(), String> {
    if required && text.trim().is_empty() {
        return Err(format!("{kind} cannot be empty"));
    }
    if text.len() > MAX_TEXT_LEN {
        return Err(format!("{kind} exceeds {MAX_TEXT_LEN} bytes"));
    }
    Ok(())
}

const fn default_version() -> u32 {
    1
}

const fn default_max_agent_calls() -> u32 {
    1_000
}

const fn default_max_shell_calls() -> u32 {
    1_000
}

const fn default_max_parallel_agents() -> usize {
    4
}

const fn default_timeout_seconds() -> u64 {
    43_200
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
