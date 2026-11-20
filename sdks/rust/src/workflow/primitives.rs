use super::*;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaitEventOptions {
    pub event_type: String,
    pub correlation_expr: String,
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookback_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildCancelPolicy {
    Propagate,
    Detach,
    RequestCancel,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChildOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_policy: Option<ChildCancelPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SwitchCase {
    pub id: String,
    pub when: String,
    pub target: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CollectionOptions {
    pub items: String,
    pub body: String,
    pub result: String,
    pub max_items: u64,
    pub max_concurrency: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LoopOptions {
    pub condition: String,
    pub body: String,
    pub initial: String,
    pub carry: String,
    pub max_iterations: u64,
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
}
#[derive(Clone, Debug, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalRole {
    Admin,
    Developer,
    Operator,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApprovalPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subjects: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<ApprovalRole>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_initiator: Option<bool>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScriptOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensate_with: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_instances: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u64>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TriggerKind {
    Manual,
    Event,
    Webhook,
    Message,
    Cron,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub kind: TriggerKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}
fn bounded_duration(value: u64) -> bool {
    (1..=31_622_400_000).contains(&value)
}
impl Builder {
    pub fn description(&self, value: &str) -> &Self {
        if !self.root_only() {
            return self;
        }
        self.change(|d| {
            d.metadata.insert("description".into(), value.into());
        });
        self
    }
    pub fn label(&self, key: &str, value: &str) -> &Self {
        if !self.root_only() {
            return self;
        }
        self.change(|d| {
            let labels = d
                .metadata
                .entry("labels".into())
                .or_insert_with(|| json!({}));
            labels[key] = value.into();
        });
        self
    }
    pub fn input_schema(&self, value: Value) -> &Self {
        if !self.root_only() {
            return self;
        }
        self.change(|d| {
            d.fields.insert("inputSchema".into(), value);
        });
        self
    }
    pub fn output_schema(&self, value: Value) -> &Self {
        if !self.root_only() {
            return self;
        }
        self.change(|d| {
            d.fields.insert("outputSchema".into(), value);
        });
        self
    }
    pub fn trigger(&self, value: Trigger) -> &Self {
        if !self.root_only() {
            return self;
        }
        if let Ok(value) = serde_json::to_value(value) {
            self.change(|d| {
                if let Some(list) = d
                    .fields
                    .entry("triggers".into())
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                {
                    list.push(value);
                }
            });
        } else {
            return self.fail("invalid trigger");
        }
        self
    }
    pub fn limits(&self, value: WorkflowLimits) -> &Self {
        if !self.root_only() {
            return self;
        }
        if value
            .max_instances
            .is_some_and(|n| !(1..=100000).contains(&n))
        {
            return self.fail("limits maxInstances must be between 1 and 100000");
        }
        if value.max_depth.is_some_and(|n| !(1..=8).contains(&n)) {
            return self.fail("limits maxDepth must be between 1 and 8");
        }
        if let Ok(value) = serde_json::to_value(value) {
            self.change(|d| {
                d.fields.insert("limits".into(), value);
            });
        }
        self
    }
    pub fn chain_policy(&self, max_segments: u64, timeout_ms: u64) -> &Self {
        if !self.root_only() {
            return self;
        }
        if !(1..=10000).contains(&max_segments) {
            return self.fail("chainPolicy maxSegments must be between 1 and 10000");
        }
        if !bounded_duration(timeout_ms) {
            return self.fail("chainPolicy timeoutMs must be between 1 and 31622400000");
        }
        self.change(|d| {
            d.fields.insert(
                "chainPolicy".into(),
                json!({"maxSegments":max_segments,"timeoutMs":timeout_ms}),
            );
        });
        self
    }
    pub fn wait_for_event(&self, id: &str, options: WaitEventOptions) -> &Self {
        if options.event_type.is_empty() || options.correlation_expr.is_empty() {
            return self.fail("waitForEvent needs an eventType and a correlationExpr");
        }
        if options.timeout_ms < 1 {
            return self.fail("waitForEvent needs a timeout of at least 1 ms");
        }
        self.options(json!({"id":id,"type":"waitEvent"}), options)
    }
    pub fn invoke_child(&self, id: &str, workflow: &str, options: ChildOptions) -> &Self {
        if workflow.is_empty() {
            return self.fail("invokeChild needs the name of a workflow");
        }
        self.options(
            json!({"id":id,"type":"child","workflowId":workflow}),
            options,
        )
    }
    pub fn switch(&self, id: &str, cases: Vec<SwitchCase>, default_target: &str) -> &Self {
        if !(1..=32).contains(&cases.len()) {
            return self.fail("switch needs between 1 and 32 cases");
        }
        if default_target.is_empty() {
            return self.fail("switch needs a default target");
        }
        let mut ids = BTreeSet::new();
        for case in &cases {
            if case.id.is_empty() || !ids.insert(&case.id) {
                return self.fail("switch has an invalid or duplicate case id");
            }
            if case.when.is_empty() {
                return self.fail("a switch case needs a when expression");
            }
            if case.target.is_empty() {
                return self.fail("a switch case needs a target");
            }
        }
        self.push(json!({"id":id,"type":"switch","cases":cases,"default":default_target}))
    }
    pub fn for_each(&self, id: &str, options: CollectionOptions) -> &Self {
        if options.items.is_empty() || options.body.is_empty() || options.result.is_empty() {
            return self.fail("forEach needs items, body and result expressions");
        }
        if !(1..=10000).contains(&options.max_items) {
            return self.fail("forEach maxItems must be between 1 and 10000");
        }
        if !(1..=256).contains(&options.max_concurrency) {
            return self.fail("forEach maxConcurrency must be between 1 and 256");
        }
        self.options(json!({"id":id,"type":"forEach"}), options)
    }
    pub fn map(&self, id: &str, options: CollectionOptions) -> &Self {
        self.for_each(id, options)
    }
    fn loop_node(&self, id: &str, kind: &str, options: LoopOptions) -> &Self {
        if options.condition.is_empty()
            || options.body.is_empty()
            || options.initial.is_empty()
            || options.carry.is_empty()
        {
            return self.fail("a loop needs condition, body, initial and carry expressions");
        }
        if !(1..=10000).contains(&options.max_iterations) {
            return self.fail("loop maxIterations must be between 1 and 10000");
        }
        if !bounded_duration(options.timeout_ms) {
            return self.fail("loop timeoutMs must be between 1 and 31622400000");
        }
        if options.delay_ms.is_some_and(|v| !bounded_duration(v)) {
            return self.fail("loop delayMs must be between 1 and 31622400000");
        }
        self.options(json!({"id":id,"type":kind}), options)
    }
    pub fn while_loop(&self, id: &str, options: LoopOptions) -> &Self {
        self.loop_node(id, "while", options)
    }
    pub fn repeat_until(&self, id: &str, options: LoopOptions) -> &Self {
        self.loop_node(id, "repeatUntil", options)
    }
    pub fn return_value(&self, id: &str, result: &str) -> &Self {
        if result.is_empty() {
            return self.fail("return needs a result expression");
        }
        self.push(json!({"id":id,"type":"return","result":result}))
    }
    pub fn body(&self, name: &str, body: &Builder) -> &Self {
        if !self.root_only() {
            return self;
        }
        if name.is_empty() {
            return self.fail("a body needs a name");
        }
        if body.reaches(self, &mut BTreeSet::new()) {
            return self.fail("joined body cycle is not allowed");
        }
        self.change(|d| {
            if d.bodies.contains_key(name) {
                if d.error.is_none() {
                    d.error = Some(invalid("the body is defined twice"));
                }
            } else {
                d.bodies.insert(name.into(), body.clone());
            }
        });
        self
    }
    pub fn continue_as_new(&self, id: &str, input: &str, state: Option<&str>) -> &Self {
        if input.is_empty() {
            return self.fail("continueAsNew needs an input expression");
        }
        if state == Some("") {
            return self.fail("continueAsNew state must be a non-empty expression");
        }
        let mut node = json!({"id":id,"type":"continueAsNew","input":input});
        if let Some(state) = state {
            node["state"] = state.into();
        }
        self.push(node)
    }
    pub fn approval(
        &self,
        id: &str,
        policy: ApprovalPolicy,
        timeout_ms: u64,
        on_approve: &str,
        on_reject: &str,
        on_expire: &str,
    ) -> &Self {
        let subjects = policy.subjects.as_deref().unwrap_or_default();
        let roles = policy.roles.as_deref().unwrap_or_default();
        if !(1..=32).contains(&(subjects.len() + roles.len())) {
            return self.fail("approval policy needs between 1 and 32 subjects and roles");
        }
        let mut seen = BTreeSet::new();
        for subject in subjects {
            if !seen.insert(subject)
                || subject.trim().is_empty()
                || subject.len() > 255
                || subject.to_lowercase().starts_with("system:")
                || subject.chars().any(char::is_control)
            {
                return self.fail("approval policy has an invalid or duplicate subject");
            }
        }
        if roles.iter().collect::<BTreeSet<_>>().len() != roles.len() {
            return self.fail("approval policy has a duplicate role");
        }
        if !bounded_duration(timeout_ms) {
            return self.fail("approval timeoutMs must be between 1 and 31622400000");
        }
        for (name, value) in [
            ("onApprove", on_approve),
            ("onReject", on_reject),
            ("onExpire", on_expire),
        ] {
            if value.is_empty() {
                return self.fail(&format!("approval needs {name}"));
            }
        }
        self.push(json!({"id":id,"type":"approval","policy":policy,"timeoutMs":timeout_ms,"onApprove":on_approve,"onReject":on_reject,"onExpire":on_expire}))
    }
    pub fn script(&self, id: &str, artifact_digest: &str, options: ScriptOptions) -> &Self {
        if !artifact_digest.strip_prefix("sha256:").is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        }) {
            return self.fail("script needs a sha256 artifact digest");
        }
        if options
            .secrets
            .as_ref()
            .is_some_and(|names| names.iter().any(|n| !valid_secret(n)))
        {
            return self.fail("script names a secret that is not a name");
        }
        self.options(
            json!({"id":id,"type":"script","artifactDigest":artifact_digest}),
            options,
        )
    }
}
