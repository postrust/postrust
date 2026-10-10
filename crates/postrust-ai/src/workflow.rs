//! Workflows: steps chained into a graph, each step a job.
//!
//! ```json
//! {
//!   "steps": [
//!     { "id": "transcribe", "model": "whisper",
//!       "input": { "request": { "multipart": { "file_url": "${input.audio_url}" } } } },
//!     { "id": "summarise", "model": "llama", "depends_on": ["transcribe"],
//!       "input": { "request": { "body": { "messages": [
//!         { "role": "user", "content": "Summarise: ${steps.transcribe.output.text}" } ] } } } },
//!     { "id": "embed_chunks", "model": "embedder", "depends_on": ["transcribe"],
//!       "for_each": "${steps.transcribe.output.segments}",
//!       "input": { "request": { "body": { "input": "${item.text}" } } } }
//!   ],
//!   "output": { "summary": "${steps.summarise.output}" }
//! }
//! ```
//!
//! - `${path}` as a whole string is replaced by the value at `path`, keeping
//!   its JSON type; inside a longer string it is interpolated as text. Paths
//!   start at `input`, `steps.<id>.output`, `item` or `index`.
//! - `when` skips a step unless a condition holds. Dependents of a skipped
//!   step still run, and see its output as `null`.
//! - `for_each` fans a step out to one job per array element; its output is
//!   the array of their outputs, in order. Steps that depend on it wait for
//!   all of them: that is the fan-in.
//!
//! # Advancing
//!
//! A run's state is never trusted on its own: [`WorkflowEngine::advance`]
//! rebuilds it from the run's jobs every time, under a row lock on the run,
//! and enqueues whatever has become ready. Step jobs carry idempotency keys,
//! so advancing twice -- after a crash, or from two nodes -- cannot enqueue a
//! step twice. It runs whenever a step's job finishes, and periodically for
//! every running run as a backstop.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{AiError, AiResult};
use crate::queue::{JobStatus, JobStore, NewJob, Priority};

/// A workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct WorkflowDef {
    /// Steps, in any order; `depends_on` decides the order they run in.
    pub steps: Vec<StepDef>,
    /// Output template. Absent: an object of every leaf step's output.
    #[serde(default)]
    pub output: Option<Value>,
}

/// One step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StepDef {
    /// Unique within the workflow.
    pub id: String,
    /// Model to run on.
    pub model: String,
    /// Queue; the run's if absent.
    #[serde(default)]
    pub queue: Option<String>,
    /// Priority class; the run's if absent.
    #[serde(default)]
    pub priority: Option<Priority>,
    /// Steps that must finish first.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Job input template.
    pub input: Value,
    /// Job params template.
    #[serde(default)]
    pub params: Option<Value>,
    /// Run only if this holds.
    #[serde(default)]
    pub when: Option<Condition>,
    /// Template resolving to an array: one job per element.
    #[serde(default)]
    pub for_each: Option<String>,
}

/// A step condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Condition {
    /// The path resolves to something other than null.
    Exists {
        /// Path.
        exists: String,
    },
    /// The path equals a value.
    Equals {
        /// Path.
        path: String,
        /// Value.
        equals: Value,
    },
    /// The path does not equal a value.
    NotEquals {
        /// Path.
        path: String,
        /// Value.
        not_equals: Value,
    },
}

impl WorkflowDef {
    /// Reject definitions that cannot run: duplicate ids, unknown
    /// dependencies, cycles, unknown models.
    pub fn validate(&self, known_models: &HashSet<String>) -> AiResult<()> {
        let bad = |m: String| Err(AiError::BadRequest(m));
        if self.steps.is_empty() {
            return bad("a workflow needs at least one step".into());
        }
        let mut ids = HashSet::new();
        for step in &self.steps {
            if step.id.is_empty() || step.id.contains(['.', ':']) {
                return bad(format!(
                    "step id '{}' must be non-empty without '.' or ':'",
                    step.id
                ));
            }
            if !ids.insert(step.id.as_str()) {
                return bad(format!("step '{}' declared twice", step.id));
            }
            if !known_models.is_empty() && !known_models.contains(&step.model) {
                return bad(format!(
                    "step '{}': unknown model '{}'",
                    step.id, step.model
                ));
            }
        }
        for step in &self.steps {
            for dep in &step.depends_on {
                if !ids.contains(dep.as_str()) {
                    return bad(format!(
                        "step '{}' depends on unknown step '{dep}'",
                        step.id
                    ));
                }
            }
        }
        self.topo_order().map(|_| ())
    }

    /// Steps ordered so each comes after its dependencies.
    fn topo_order(&self) -> AiResult<Vec<&StepDef>> {
        let by_id: HashMap<&str, &StepDef> =
            self.steps.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut order = Vec::with_capacity(self.steps.len());
        let mut done: HashSet<&str> = HashSet::new();
        let mut visiting: HashSet<&str> = HashSet::new();
        fn visit<'a>(
            id: &'a str,
            by_id: &HashMap<&'a str, &'a StepDef>,
            done: &mut HashSet<&'a str>,
            visiting: &mut HashSet<&'a str>,
            order: &mut Vec<&'a StepDef>,
        ) -> AiResult<()> {
            if done.contains(id) {
                return Ok(());
            }
            if !visiting.insert(id) {
                return Err(AiError::BadRequest(format!(
                    "workflow has a cycle through '{id}'"
                )));
            }
            let step = by_id[id];
            for dep in &step.depends_on {
                visit(dep.as_str(), by_id, done, visiting, order)?;
            }
            visiting.remove(id);
            done.insert(id);
            order.push(step);
            Ok(())
        }
        for step in &self.steps {
            visit(
                step.id.as_str(),
                &by_id,
                &mut done,
                &mut visiting,
                &mut order,
            )?;
        }
        Ok(order)
    }
}

// ============================================================================
// Templates
// ============================================================================

/// The value at a dotted path, e.g. `steps.a.output.results.0.text`.
pub fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.').filter(|s| !s.is_empty()) {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Fill a template from `context`.
pub fn render(template: &Value, context: &Value) -> Value {
    match template {
        Value::String(s) => render_string(s, context),
        Value::Array(items) => Value::Array(items.iter().map(|v| render(v, context)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), render(v, context)))
                .collect::<Map<String, Value>>(),
        ),
        other => other.clone(),
    }
}

fn render_string(s: &str, context: &Value) -> Value {
    let trimmed = s.trim();
    if let Some(path) = trimmed.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
        if !path.contains("${") && !path.contains('}') {
            return lookup(context, path.trim()).cloned().unwrap_or(Value::Null);
        }
    }
    if !s.contains("${") {
        return Value::String(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        match lookup(context, after[..end].trim()) {
            Some(Value::String(v)) => out.push_str(v),
            Some(Value::Null) | None => {}
            Some(other) => out.push_str(&other.to_string()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Value::String(out)
}

impl Condition {
    fn holds(&self, context: &Value) -> bool {
        let at = |path: &str| lookup(context, path.trim_start_matches("${").trim_end_matches('}'));
        match self {
            Condition::Exists { exists } => at(exists).is_some_and(|v| !v.is_null()),
            Condition::Equals { path, equals } => at(path) == Some(equals),
            Condition::NotEquals { path, not_equals } => at(path) != Some(not_equals),
        }
    }
}

// ============================================================================
// Runs
// ============================================================================

/// Run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// In progress.
    Running,
    /// Every step completed or was skipped.
    Completed,
    /// A step failed.
    Failed,
    /// Cancelled on request.
    Cancelled,
}

/// A run, for the API.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct WorkflowRun {
    /// Id.
    pub id: Uuid,
    /// Workflow name.
    pub workflow: String,
    /// Status.
    pub status: String,
    /// Input.
    pub input: Value,
    /// Per-step state.
    pub state: Value,
    /// Output, once completed.
    pub output: Option<Value>,
    /// Why it failed.
    pub error: Option<String>,
    /// Started.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Finished.
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One job of a step: fan-out index, status, output, error.
type StepJob = (usize, JobStatus, Option<Value>, Option<String>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StepStatus {
    Pending,
    Running,
    Completed,
    Skipped,
    Failed,
}

/// Starts and advances runs.
#[derive(Clone)]
pub struct WorkflowEngine {
    pool: PgPool,
    store: Arc<dyn JobStore>,
    known_models: HashSet<String>,
}

impl WorkflowEngine {
    /// An engine. `known_models` validates definitions; empty skips that
    /// check (for nodes that do not carry every model).
    pub fn new(pool: PgPool, store: Arc<dyn JobStore>, known_models: HashSet<String>) -> Self {
        Self {
            pool,
            store,
            known_models,
        }
    }

    /// Create or replace a workflow.
    pub async fn put(&self, name: &str, def: &WorkflowDef) -> AiResult<i32> {
        def.validate(&self.known_models)?;
        let version: i32 = sqlx::query_scalar(
            "INSERT INTO ai.workflows (name, definition) VALUES ($1, $2)
             ON CONFLICT (name) DO UPDATE SET definition = EXCLUDED.definition,
                version = ai.workflows.version + 1, updated_at = now()
             RETURNING version",
        )
        .bind(name)
        .bind(serde_json::to_value(def)?)
        .fetch_one(&self.pool)
        .await?;
        Ok(version)
    }

    /// A workflow.
    pub async fn get(&self, name: &str) -> AiResult<Option<(WorkflowDef, i32)>> {
        let row = sqlx::query("SELECT definition, version FROM ai.workflows WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| {
            let def: Value = r.try_get("definition")?;
            Ok((serde_json::from_value(def)?, r.try_get("version")?))
        })
        .transpose()
    }

    /// Start a run.
    pub async fn start(
        &self,
        name: &str,
        input: Value,
        queue: &str,
        priority: Priority,
    ) -> AiResult<WorkflowRun> {
        let (def, _) = self
            .get(name)
            .await?
            .ok_or_else(|| AiError::NotFound(format!("workflow '{name}'")))?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ai.workflow_runs (id, workflow, definition, input, queue, priority_class)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(name)
        .bind(serde_json::to_value(&def)?)
        .bind(&input)
        .bind(queue)
        .bind(priority.as_str())
        .execute(&self.pool)
        .await?;
        self.advance(id).await?;
        self.run(id)
            .await?
            .ok_or_else(|| AiError::NotFound(format!("run {id}")))
    }

    /// A run.
    pub async fn run(&self, id: Uuid) -> AiResult<Option<WorkflowRun>> {
        let row = sqlx::query(
            "SELECT id, workflow, status, input, state, output, error, created_at, finished_at
             FROM ai.workflow_runs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            Ok(WorkflowRun {
                id: r.try_get("id")?,
                workflow: r.try_get("workflow")?,
                status: r.try_get("status")?,
                input: r.try_get("input")?,
                state: r.try_get("state")?,
                output: r.try_get("output")?,
                error: r.try_get("error")?,
                created_at: r.try_get("created_at")?,
                finished_at: r.try_get("finished_at")?,
            })
        })
        .transpose()
    }

    /// Cancel a run and its outstanding jobs.
    pub async fn cancel(&self, id: Uuid) -> AiResult<bool> {
        let done = sqlx::query(
            "UPDATE ai.workflow_runs SET status = 'cancelled', finished_at = now(), updated_at = now()
             WHERE id = $1 AND status = 'running'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
        self.cancel_jobs(id).await?;
        Ok(true)
    }

    async fn cancel_jobs(&self, run_id: Uuid) -> AiResult<()> {
        let outstanding: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM ai.jobs WHERE workflow_run_id = $1
             AND status IN ('queued', 'running', 'paused')",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        for job in outstanding {
            // A job that finished in the meantime is not an error.
            let _ = self.store.request_cancel(job).await;
        }
        Ok(())
    }

    /// Advance every running run not touched recently. The backstop for
    /// advances lost to a crash.
    pub async fn reconcile(&self) -> AiResult<usize> {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM ai.workflow_runs
             WHERE status = 'running' AND updated_at < now() - interval '15 seconds'
             ORDER BY updated_at LIMIT 200",
        )
        .fetch_all(&self.pool)
        .await?;
        let n = ids.len();
        for id in ids {
            if let Err(error) = self.advance(id).await {
                tracing::warn!(run = %id, %error, "could not advance workflow run");
            }
        }
        Ok(n)
    }

    /// Rebuild a run's state from its jobs, enqueue whatever is ready, and
    /// finish the run if it is done.
    pub async fn advance(&self, run_id: Uuid) -> AiResult<()> {
        let mut tx = self.pool.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT definition, input, queue, priority_class, status, state
             FROM ai.workflow_runs WHERE id = $1 FOR UPDATE",
        )
        .bind(run_id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(());
        };
        if row.try_get::<String, _>("status")? != "running" {
            return Ok(());
        }
        let def: WorkflowDef = serde_json::from_value(row.try_get("definition")?)?;
        let input: Value = row.try_get("input")?;
        let run_queue: String = row.try_get("queue")?;
        let run_priority: Priority = row.try_get::<String, _>("priority_class")?.parse()?;
        let prev_state: Value = row.try_get("state")?;

        // Step jobs, with the fan-out index from their idempotency key.
        let jobs = sqlx::query(
            "SELECT workflow_step, status, output, error, idempotency_key
             FROM ai.jobs WHERE workflow_run_id = $1",
        )
        .bind(run_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut by_step: HashMap<String, Vec<StepJob>> = HashMap::new();
        for job in &jobs {
            let step: Option<String> = job.try_get("workflow_step")?;
            let Some(step) = step else { continue };
            let key: Option<String> = job.try_get("idempotency_key")?;
            let index = key
                .as_deref()
                .and_then(|k| k.rsplit(':').next())
                .and_then(|i| i.parse().ok())
                .unwrap_or(0);
            by_step.entry(step).or_default().push((
                index,
                job.try_get::<String, _>("status")?.parse()?,
                job.try_get("output")?,
                job.try_get("error")?,
            ));
        }

        let mut statuses: HashMap<String, StepStatus> = HashMap::new();
        let mut outputs = Map::new();
        let mut state_steps = Map::new();
        let mut failure: Option<String> = None;
        let mut to_enqueue: Vec<NewJob> = Vec::new();

        for step in def.topo_order()? {
            let expected = prev_state
                .pointer(&format!("/steps/{}/expected", step.id))
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let mut status;
            let mut output = Value::Null;

            // A fan-out whose job count was never recorded (a crash between
            // enqueueing and saving state) is treated as not yet started:
            // re-enqueueing is idempotent, and it records the count.
            let step_jobs = by_step
                .get(&step.id)
                .filter(|_| step.for_each.is_none() || expected.is_some());
            if let Some(step_jobs) = step_jobs {
                let expected = expected.unwrap_or(1);
                if let Some((_, s, _, e)) = step_jobs.iter().find(|(_, s, _, _)| {
                    matches!(
                        s,
                        JobStatus::Failed | JobStatus::Dead | JobStatus::Cancelled
                    )
                }) {
                    status = StepStatus::Failed;
                    failure.get_or_insert_with(|| {
                        format!(
                            "step '{}' {}: {}",
                            step.id,
                            s.as_str(),
                            e.clone().unwrap_or_default()
                        )
                    });
                } else if step_jobs.len() >= expected
                    && step_jobs
                        .iter()
                        .all(|(_, s, _, _)| *s == JobStatus::Completed)
                {
                    status = StepStatus::Completed;
                    output = if step.for_each.is_some() {
                        let mut sorted = step_jobs.clone();
                        sorted.sort_by_key(|(i, _, _, _)| *i);
                        Value::Array(
                            sorted
                                .into_iter()
                                .map(|(_, _, o, _)| o.unwrap_or(Value::Null))
                                .collect(),
                        )
                    } else {
                        step_jobs[0].2.clone().unwrap_or(Value::Null)
                    };
                } else {
                    status = StepStatus::Running;
                }
            } else if prev_state
                .pointer(&format!("/steps/{}/status", step.id))
                .and_then(Value::as_str)
                == Some("completed")
            {
                // An empty fan-out: completed with no jobs.
                status = StepStatus::Completed;
                output = prev_state
                    .pointer(&format!("/steps/{}/output", step.id))
                    .cloned()
                    .unwrap_or(Value::Array(vec![]));
            } else {
                status = StepStatus::Pending;
            }

            let mut expected_now = expected;
            if status == StepStatus::Pending && failure.is_none() {
                let deps: Vec<StepStatus> = step
                    .depends_on
                    .iter()
                    .map(|d| statuses.get(d).copied().unwrap_or(StepStatus::Pending))
                    .collect();
                let ready = deps
                    .iter()
                    .all(|s| matches!(s, StepStatus::Completed | StepStatus::Skipped));
                if ready {
                    let context = json!({ "input": input, "steps": outputs.clone() });
                    if step.when.as_ref().is_some_and(|c| !c.holds(&context)) {
                        status = StepStatus::Skipped;
                    } else {
                        let items: Vec<(usize, Value)> = match &step.for_each {
                            None => vec![(0, Value::Null)],
                            Some(path) => match render(&Value::String(path.clone()), &context) {
                                Value::Array(items) => items.into_iter().enumerate().collect(),
                                other => {
                                    status = StepStatus::Failed;
                                    failure.get_or_insert_with(|| {
                                        format!(
                                            "step '{}': for_each resolved to {} rather than an array",
                                            step.id,
                                            type_name(&other)
                                        )
                                    });
                                    vec![]
                                }
                            },
                        };
                        if status == StepStatus::Pending {
                            expected_now = Some(items.len());
                            if items.is_empty() {
                                status = StepStatus::Completed;
                                output = Value::Array(vec![]);
                            } else {
                                status = StepStatus::Running;
                                for (index, item) in items {
                                    let mut ctx = context.clone();
                                    ctx["item"] = item;
                                    ctx["index"] = json!(index);
                                    let mut job = NewJob::new(
                                        step.queue.clone().unwrap_or_else(|| run_queue.clone()),
                                        step.model.clone(),
                                        render(&step.input, &ctx),
                                    )
                                    .with_priority(step.priority.unwrap_or(run_priority))
                                    .with_idempotency_key(format!(
                                        "wf:{run_id}:{}:{index}",
                                        step.id
                                    ));
                                    if let Some(params) = &step.params {
                                        job.params = render(params, &ctx);
                                    }
                                    job.workflow_run_id = Some(run_id);
                                    job.workflow_step = Some(step.id.clone());
                                    to_enqueue.push(job);
                                }
                            }
                        }
                    }
                }
            }

            statuses.insert(step.id.clone(), status);
            if matches!(status, StepStatus::Completed | StepStatus::Skipped) {
                outputs.insert(step.id.clone(), json!({ "output": output }));
            }
            let mut entry = json!({ "status": status });
            if let Some(n) = expected_now {
                entry["expected"] = json!(n);
            }
            if status == StepStatus::Completed {
                entry["output"] = output;
            }
            state_steps.insert(step.id.clone(), entry);
        }

        if failure.is_none() {
            for job in to_enqueue {
                if !job.input.is_object() {
                    failure = Some(format!(
                        "step '{}': input template must render to an object",
                        job.workflow_step.as_deref().unwrap_or("?")
                    ));
                    break;
                }
                self.store.enqueue(job).await?;
            }
        }

        let state = json!({ "steps": state_steps });
        let all_done = statuses
            .values()
            .all(|s| matches!(s, StepStatus::Completed | StepStatus::Skipped));

        if let Some(error) = failure {
            sqlx::query(
                "UPDATE ai.workflow_runs SET status = 'failed', state = $2, error = $3,
                        finished_at = now(), updated_at = now() WHERE id = $1",
            )
            .bind(run_id)
            .bind(&state)
            .bind(&error)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            tracing::warn!(run = %run_id, %error, "workflow run failed");
            self.cancel_jobs(run_id).await?;
        } else if all_done {
            let context = json!({ "input": input, "steps": outputs });
            let output = match &def.output {
                Some(template) => render(template, &context),
                None => leaf_outputs(&def, &context),
            };
            sqlx::query(
                "UPDATE ai.workflow_runs SET status = 'completed', state = $2, output = $3,
                        finished_at = now(), updated_at = now() WHERE id = $1",
            )
            .bind(run_id)
            .bind(&state)
            .bind(&output)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            tracing::info!(run = %run_id, "workflow run completed");
        } else {
            sqlx::query("UPDATE ai.workflow_runs SET state = $2, updated_at = now() WHERE id = $1")
                .bind(run_id)
                .bind(&state)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(())
    }
}

/// Outputs of steps nothing depends on.
fn leaf_outputs(def: &WorkflowDef, context: &Value) -> Value {
    let depended: HashSet<&str> = def
        .steps
        .iter()
        .flat_map(|s| s.depends_on.iter().map(String::as_str))
        .collect();
    let leaves: BTreeMap<String, Value> = def
        .steps
        .iter()
        .filter(|s| !depended.contains(s.id.as_str()))
        .map(|s| {
            (
                s.id.clone(),
                lookup(context, &format!("steps.{}.output", s.id))
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        })
        .collect();
    serde_json::to_value(leaves).unwrap_or(Value::Null)
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn def(text: &str) -> WorkflowDef {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn whole_string_placeholders_keep_their_type() {
        let ctx = json!({"input": {"n": 3, "list": [1, 2], "name": "x"}});
        assert_eq!(render(&json!("${input.n}"), &ctx), json!(3));
        assert_eq!(render(&json!("${input.list}"), &ctx), json!([1, 2]));
        assert_eq!(render(&json!("${input.missing}"), &ctx), Value::Null);
        assert_eq!(
            render(&json!("n=${input.n}, ${input.name}!"), &ctx),
            json!("n=3, x!")
        );
        assert_eq!(
            render(&json!({"a": ["${input.list.1}"]}), &ctx),
            json!({"a": [2]})
        );
    }

    #[test]
    fn unterminated_placeholders_are_left_alone() {
        let ctx = json!({});
        assert_eq!(render(&json!("a ${b"), &ctx), json!("a ${b"));
    }

    #[test]
    fn cycles_and_unknown_deps_are_rejected() {
        let cyclic = def(r#"{"steps": [
            {"id": "a", "model": "m", "input": {}, "depends_on": ["b"]},
            {"id": "b", "model": "m", "input": {}, "depends_on": ["a"]}]}"#);
        assert!(cyclic
            .validate(&HashSet::new())
            .unwrap_err()
            .to_string()
            .contains("cycle"));

        let unknown =
            def(r#"{"steps": [{"id": "a", "model": "m", "input": {}, "depends_on": ["z"]}]}"#);
        assert!(unknown.validate(&HashSet::new()).is_err());

        let models: HashSet<String> = ["other".to_string()].into();
        let ok = def(r#"{"steps": [{"id": "a", "model": "m", "input": {}}]}"#);
        assert!(ok
            .validate(&models)
            .unwrap_err()
            .to_string()
            .contains("unknown model"));
    }

    #[test]
    fn topo_order_respects_dependencies() {
        let d = def(r#"{"steps": [
            {"id": "c", "model": "m", "input": {}, "depends_on": ["a", "b"]},
            {"id": "b", "model": "m", "input": {}, "depends_on": ["a"]},
            {"id": "a", "model": "m", "input": {}}]}"#);
        let order: Vec<&str> = d
            .topo_order()
            .unwrap()
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn conditions() {
        let ctx = json!({"steps": {"detect": {"output": {"lang": "en"}}}});
        let eq: Condition =
            serde_json::from_str(r#"{"path": "steps.detect.output.lang", "equals": "en"}"#)
                .unwrap();
        let ne: Condition =
            serde_json::from_str(r#"{"path": "steps.detect.output.lang", "not_equals": "en"}"#)
                .unwrap();
        let ex: Condition =
            serde_json::from_str(r#"{"exists": "steps.detect.output.lang"}"#).unwrap();
        let missing: Condition =
            serde_json::from_str(r#"{"exists": "steps.nope.output"}"#).unwrap();
        assert!(eq.holds(&ctx));
        assert!(!ne.holds(&ctx));
        assert!(ex.holds(&ctx));
        assert!(!missing.holds(&ctx));
    }

    #[test]
    fn leaves_are_steps_nothing_depends_on() {
        let d = def(r#"{"steps": [
            {"id": "a", "model": "m", "input": {}},
            {"id": "b", "model": "m", "input": {}, "depends_on": ["a"]},
            {"id": "c", "model": "m", "input": {}, "depends_on": ["a"]}]}"#);
        let ctx = json!({"steps": {"a": {"output": 1}, "b": {"output": 2}, "c": {"output": 3}}});
        assert_eq!(leaf_outputs(&d, &ctx), json!({"b": 2, "c": 3}));
    }
}
