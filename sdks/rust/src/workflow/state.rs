use super::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum DisclosurePolicy {
    #[serde(rename = "declared-business-result-v1")]
    DeclaredBusinessResultV1,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueryDefinition {
    pub args_schema: Value,
    pub result_schema: Value,
    pub expression: String,
    pub disclosure: Option<DisclosurePolicy>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateDefinition {
    pub args_schema: Value,
    pub result_schema: Value,
    pub state: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate: Option<String>,
    pub disclosure: Option<DisclosurePolicy>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StateDefinition {
    pub schema: Value,
    pub initial: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub queries: BTreeMap<String, QueryDefinition>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub updates: BTreeMap<String, UpdateDefinition>,
}
fn expression(value: &str, subject: &str) -> Result<(), SagatError> {
    if value.is_empty() {
        return Err(invalid(&format!("{subject} needs an expression")));
    }
    if value.len() > 1024 {
        return Err(invalid(&format!(
            "{subject} expression exceeds 1024 UTF-8 bytes"
        )));
    }
    Ok(())
}
fn bounded_integer(value: &Value, ceiling: u64, subject: &str) -> Result<u64, SagatError> {
    value
        .as_u64()
        .filter(|n| *n <= ceiling)
        .ok_or_else(|| invalid(&format!("{subject} must be a bounded nonnegative integer")))
}
fn schema(value: &Value, subject: &str) -> Result<(), SagatError> {
    let fields = value
        .as_object()
        .ok_or_else(|| invalid(&format!("{subject} must be a closed bounded schema")))?;
    if fields
        .get("description")
        .is_some_and(|v| v.as_str().is_none_or(|s| s.chars().count() > 1024))
    {
        return Err(invalid(&format!(
            "{subject}.description must be a string of at most 1024 characters"
        )));
    }
    let mut allowed = BTreeSet::from(["type", "description"]);
    match value["type"].as_str().unwrap_or_default() {
        "object" => {
            allowed.extend(["properties", "required", "additionalProperties"]);
            if value["additionalProperties"] != false {
                return Err(invalid(&format!(
                    "{subject}.additionalProperties must be false"
                )));
            }
            let properties = value["properties"]
                .as_object()
                .ok_or_else(|| invalid(&format!("{subject}.properties must be an object")))?;
            for (name, nested) in properties {
                schema(nested, &format!("{subject}.properties.{name}"))?;
            }
            if let Some(required) = fields.get("required") {
                let list = required
                    .as_array()
                    .ok_or_else(|| invalid("required must name unique declared properties"))?;
                let mut seen = BTreeSet::new();
                if list.len() > 10000 {
                    return Err(invalid("required must name unique declared properties"));
                }
                for value in list {
                    if value
                        .as_str()
                        .is_none_or(|name| !properties.contains_key(name) || !seen.insert(name))
                    {
                        return Err(invalid("required must name unique declared properties"));
                    }
                }
            }
        }
        "array" | "string" => {
            let array = value["type"] == "array";
            let (minimum, maximum, ceiling) = if array {
                ("minItems", "maxItems", 10000)
            } else {
                ("minLength", "maxLength", 262144)
            };
            allowed.extend([minimum, maximum]);
            if array {
                allowed.insert("items");
                schema(&value["items"], &format!("{subject}.items"))?;
            }
            let max = bounded_integer(&value[maximum], ceiling, &format!("{subject}.{maximum}"))?;
            if let Some(value) = fields.get(minimum)
                && bounded_integer(value, ceiling, &format!("{subject}.{minimum}"))? > max
            {
                return Err(invalid(&format!(
                    "{subject}.{minimum} must not exceed {maximum}"
                )));
            }
        }
        "integer" | "number" => {
            allowed.extend(["minimum", "maximum"]);
            for key in ["minimum", "maximum"] {
                if fields
                    .get(key)
                    .is_some_and(|value| value.as_f64().is_none_or(|n| !n.is_finite()))
                {
                    return Err(invalid(&format!("{subject}.{key} must be a finite number")));
                }
            }
            if let (Some(min), Some(max)) = (value["minimum"].as_f64(), value["maximum"].as_f64())
                && min > max
            {
                return Err(invalid(&format!(
                    "{subject}.minimum must not exceed maximum"
                )));
            }
        }
        "boolean" | "null" => {}
        _ => {
            return Err(invalid(&format!(
                "{subject}.type is not a supported closed result type"
            )));
        }
    }
    if fields.keys().any(|key| !allowed.contains(key.as_str())) {
        return Err(invalid(&format!(
            "{subject} contains an unsupported closed schema field"
        )));
    }
    if canonical_json(value).map_or(true, |text| text.len() > 65536) {
        return Err(invalid(&format!("{subject} exceeds 65536 UTF-8 bytes")));
    }
    Ok(())
}
fn operation(
    name: &str,
    args: &Value,
    disclosure: Option<DisclosurePolicy>,
    result: &Value,
) -> Result<(), SagatError> {
    if !(1..=128).contains(&name.len())
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'-'))
    {
        return Err(invalid("operation name is invalid"));
    }
    if !args.is_object() {
        return Err(invalid("operation argsSchema must be an object"));
    }
    if disclosure.is_none() {
        return Err(invalid(
            "an operation needs disclosure declared-business-result-v1",
        ));
    }
    schema(result, "operation.resultSchema")
}
impl Builder {
    pub fn state(&self, definition: StateDefinition) -> &Self {
        if !self.root_only() {
            return self;
        }
        let validate = || -> Result<Value, SagatError> {
            schema(&definition.schema, "state.schema")?;
            expression(&definition.initial, "state.initial")?;
            if definition.queries.len() > 32 {
                return Err(invalid("state supports at most 32 queries"));
            }
            if definition.updates.len() > 32 {
                return Err(invalid("state supports at most 32 updates"));
            }
            for (name, query) in &definition.queries {
                operation(
                    name,
                    &query.args_schema,
                    query.disclosure,
                    &query.result_schema,
                )?;
                expression(&query.expression, "query")?;
            }
            for (name, update) in &definition.updates {
                operation(
                    name,
                    &update.args_schema,
                    update.disclosure,
                    &update.result_schema,
                )?;
                expression(&update.state, "update.state")?;
                expression(&update.result, "update.result")?;
                if let Some(value) = &update.validate
                    && !value.is_empty()
                {
                    expression(value, "update.validate")?;
                }
            }
            let mut value = serde_json::to_value(definition)
                .map_err(|_| invalid("invalid state definition"))?;
            if let Some(updates) = value.get_mut("updates").and_then(Value::as_object_mut) {
                for update in updates.values_mut() {
                    if update["validate"] == ""
                        && let Some(fields) = update.as_object_mut()
                    {
                        fields.remove("validate");
                    }
                }
            }
            Ok(value)
        };
        match validate() {
            Ok(value) => self.change(|d| {
                d.fields.insert("state".into(), value);
            }),
            Err(error) => {
                self.fail(&error.message);
            }
        }
        self
    }
}
