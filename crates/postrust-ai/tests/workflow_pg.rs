//! Workflows against a live PostgreSQL, with a scheduler and a stub model
//! server running the step jobs.
//!
//!     DATABASE_URL=postgres://postgres:postgres@localhost:5432/postrust_test \
//!         cargo test -p postrust-ai --test workflow_pg -- --ignored

mod common;

use std::time::Duration;

use common::{harness, stub_model, stub_server, unique, wait_for, Harness};
use postrust_ai::queue::{JobStatus, Priority, QueueSpec};
use postrust_ai::workflow::{WorkflowDef, WorkflowRun};
use serde_json::{json, Value};
use uuid::Uuid;

fn def(value: Value) -> WorkflowDef {
    serde_json::from_value(value).unwrap()
}

/// A harness with one queue and one stub model, and a started server.
async fn setup(delay: Duration, concurrency: usize) -> (Harness, String, String) {
    let url = stub_server(delay).await;
    let model = unique("model");
    let queue = unique("wf");
    let h = harness(
        vec![QueueSpec::new(&queue)],
        vec![stub_model(&model, &url, concurrency)],
    )
    .await;
    (h, queue, model)
}

async fn wait_for_run(h: &Harness, id: Uuid, want: &str) -> WorkflowRun {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let run = h.engine.run(id).await.unwrap().unwrap();
        if run.status == want {
            return run;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("run {id} is {}, wanted {want}: {run:?}", run.status);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn job_count(h: &Harness, run: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM ai.jobs WHERE workflow_run_id = $1")
        .bind(run)
        .fetch_one(&h.pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore]
async fn linear_chain_passes_a_step_output_to_the_next() {
    let (h, queue, model) = setup(Duration::ZERO, 2).await;
    let name = unique("chain");
    h.engine
        .put(
            &name,
            &def(json!({ "steps": [
                { "id": "a", "model": model,
                  "input": { "request": { "path": "/echo", "body": { "n": 7 } } } },
                { "id": "b", "model": model, "depends_on": ["a"],
                  "input": { "request": { "path": "/echo", "body": {
                      "n": "${steps.a.output.n}", "label": "n is ${steps.a.output.n}" } } } },
            ] })),
        )
        .await
        .unwrap();

    let run = h
        .engine
        .start(&name, json!({}), &queue, Priority::Normal)
        .await
        .unwrap();
    let done = wait_for_run(&h, run.id, "completed").await;
    // A whole-string placeholder keeps its JSON type; inside text it is
    // interpolated. The output defaults to the leaf steps.
    assert_eq!(
        done.output,
        Some(json!({ "b": { "n": 7, "label": "n is 7" } }))
    );
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn fan_out_then_fan_in_keeps_index_order() {
    let (h, queue, model) = setup(Duration::ZERO, 3).await;
    let name = unique("fan");
    h.engine
        .put(
            &name,
            &def(json!({ "steps": [
                { "id": "fan", "model": model, "for_each": "${input.items}",
                  "input": { "request": { "path": "/echo", "body": "${item}" } } },
                { "id": "join", "model": model, "depends_on": ["fan"],
                  "input": { "request": { "path": "/echo",
                                          "body": { "all": "${steps.fan.output}" } } } },
            ], "output": { "joined": "${steps.join.output.all}" } })),
        )
        .await
        .unwrap();

    // The first element is the slowest, so completion order is the reverse
    // of index order; the output must follow the index.
    let items = json!([
        { "v": "x", "delay_ms": 600 },
        { "v": "y", "delay_ms": 300 },
        { "v": "z", "delay_ms": 0 },
    ]);
    let run = h
        .engine
        .start(&name, json!({ "items": items }), &queue, Priority::Normal)
        .await
        .unwrap();
    let done = wait_for_run(&h, run.id, "completed").await;
    assert_eq!(done.output, Some(json!({ "joined": items })));
    assert_eq!(
        job_count(&h, run.id).await,
        4,
        "three fan-out jobs and one join"
    );
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn for_each_over_an_empty_array_completes_without_jobs() {
    let (h, queue, model) = setup(Duration::ZERO, 2).await;
    let name = unique("empty");
    h.engine
        .put(
            &name,
            &def(json!({ "steps": [
                { "id": "fan", "model": model, "for_each": "${input.items}",
                  "input": { "request": { "path": "/echo", "body": "${item}" } } },
                { "id": "after", "model": model, "depends_on": ["fan"],
                  "input": { "request": { "path": "/echo",
                                          "body": { "got": "${steps.fan.output}" } } } },
            ] })),
        )
        .await
        .unwrap();

    let run = h
        .engine
        .start(&name, json!({ "items": [] }), &queue, Priority::Normal)
        .await
        .unwrap();
    let done = wait_for_run(&h, run.id, "completed").await;
    assert_eq!(done.output, Some(json!({ "after": { "got": [] } })));
    assert_eq!(done.state["steps"]["fan"]["status"], json!("completed"));
    assert_eq!(done.state["steps"]["fan"]["output"], json!([]));
    let fan_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ai.jobs WHERE workflow_run_id = $1 AND workflow_step = 'fan'",
    )
    .bind(run.id)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(fan_jobs, 0);
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn a_false_condition_skips_the_step_and_dependents_see_null() {
    let (h, queue, model) = setup(Duration::ZERO, 2).await;
    let name = unique("when");
    h.engine
        .put(
            &name,
            &def(json!({ "steps": [
                { "id": "detect", "model": model,
                  "input": { "request": { "path": "/echo", "body": { "lang": "fr" } } } },
                { "id": "translate", "model": model, "depends_on": ["detect"],
                  "when": { "path": "steps.detect.output.lang", "equals": "en" },
                  "input": { "request": { "path": "/echo", "body": { "never": true } } } },
                { "id": "after", "model": model, "depends_on": ["translate"],
                  "input": { "request": { "path": "/echo",
                                          "body": { "saw": "${steps.translate.output}" } } } },
            ] })),
        )
        .await
        .unwrap();

    let run = h
        .engine
        .start(&name, json!({}), &queue, Priority::Normal)
        .await
        .unwrap();
    let done = wait_for_run(&h, run.id, "completed").await;
    assert_eq!(done.output, Some(json!({ "after": { "saw": null } })));
    assert_eq!(done.state["steps"]["translate"]["status"], json!("skipped"));
    let skipped_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ai.jobs WHERE workflow_run_id = $1 AND workflow_step = 'translate'",
    )
    .bind(run.id)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(skipped_jobs, 0);
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn a_permanent_step_failure_fails_the_run_and_cancels_outstanding_jobs() {
    let (h, queue, model) = setup(Duration::ZERO, 2).await;
    // A second queue nothing serves, so the job placed there stays queued
    // until the run fails and has to cancel it.
    let idle_queue = unique("wf-idle");
    h.store
        .upsert_queue(&QueueSpec::new(&idle_queue))
        .await
        .unwrap();
    let name = unique("fail");
    h.engine
        .put(
            &name,
            &def(json!({ "steps": [
                { "id": "bad", "model": model,
                  "input": { "request": { "path": "/bad", "body": {} } } },
                { "id": "parked", "model": model, "queue": idle_queue,
                  "input": { "request": { "path": "/echo", "body": {} } } },
                { "id": "downstream", "model": model, "depends_on": ["bad"],
                  "input": { "request": { "path": "/echo", "body": {} } } },
            ] })),
        )
        .await
        .unwrap();

    let run = h
        .engine
        .start(&name, json!({}), &queue, Priority::Normal)
        .await
        .unwrap();
    let failed = wait_for_run(&h, run.id, "failed").await;
    assert!(
        failed.error.as_deref().unwrap_or("").contains("step 'bad'"),
        "{failed:?}"
    );

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT workflow_step, status FROM ai.jobs WHERE workflow_run_id = $1")
            .bind(run.id)
            .fetch_all(&h.pool)
            .await
            .unwrap();
    let status_of = |step: &str| {
        rows.iter()
            .find(|(s, _)| s == step)
            .map(|(_, status)| status.as_str())
    };
    assert_eq!(status_of("bad"), Some("failed"));
    assert_eq!(status_of("parked"), Some("cancelled"));
    assert_eq!(status_of("downstream"), None, "never enqueued");
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn advancing_twice_at_once_enqueues_each_job_once() {
    let (h, _queue, model) = setup(Duration::ZERO, 2).await;
    let name = unique("race");
    let workflow = def(json!({ "steps": [
        { "id": "fan", "model": model, "for_each": "${input.items}",
          "input": { "request": { "path": "/echo", "body": "${item}" } } },
    ] }));
    h.engine.put(&name, &workflow).await.unwrap();

    // Insert the run directly: `start` advances once itself, which would
    // leave the two racing calls nothing to race over. Nothing serves the
    // queue, so the jobs just sit there to be counted.
    let idle_queue = unique("wf-idle");
    h.store
        .upsert_queue(&QueueSpec::new(&idle_queue))
        .await
        .unwrap();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ai.workflow_runs (id, workflow, definition, input, queue, priority_class)
         VALUES ($1, $2, $3, $4, $5, 'normal')",
    )
    .bind(id)
    .bind(&name)
    .bind(serde_json::to_value(&workflow).unwrap())
    .bind(json!({ "items": [{"i": 0}, {"i": 1}, {"i": 2}] }))
    .bind(&idle_queue)
    .execute(&h.pool)
    .await
    .unwrap();

    let (a, b) = tokio::join!(h.engine.advance(id), h.engine.advance(id));
    a.unwrap();
    b.unwrap();
    assert_eq!(job_count(&h, id).await, 3);

    // And again, as the reconcile backstop would.
    h.engine.advance(id).await.unwrap();
    assert_eq!(job_count(&h, id).await, 3);
    h.shutdown.cancel();
}

#[tokio::test]
#[ignore]
async fn cancelling_a_running_run_cancels_its_jobs() {
    let (h, queue, model) = setup(Duration::from_millis(200), 2).await;
    let name = unique("cancel");
    h.engine
        .put(
            &name,
            &def(json!({ "steps": [
                { "id": "long", "model": model,
                  "input": { "requests": (0..50)
                      .map(|i| json!({ "path": "/run", "body": { "n": i } }))
                      .collect::<Vec<_>>() } },
            ] })),
        )
        .await
        .unwrap();

    let run = h
        .engine
        .start(&name, json!({}), &queue, Priority::Normal)
        .await
        .unwrap();
    let job_id: Uuid = sqlx::query_scalar("SELECT id FROM ai.jobs WHERE workflow_run_id = $1")
        .bind(run.id)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    wait_for(&h.store, job_id, JobStatus::Running, Duration::from_secs(5)).await;

    assert!(h.engine.cancel(run.id).await.unwrap());
    let cancelled = wait_for_run(&h, run.id, "cancelled").await;
    assert!(cancelled.finished_at.is_some());
    wait_for(
        &h.store,
        job_id,
        JobStatus::Cancelled,
        Duration::from_secs(10),
    )
    .await;
    // A run that is no longer running cannot be cancelled again, and
    // advancing it leaves it alone.
    assert!(!h.engine.cancel(run.id).await.unwrap());
    h.engine.advance(run.id).await.unwrap();
    assert_eq!(
        h.engine.run(run.id).await.unwrap().unwrap().status,
        "cancelled"
    );
    h.shutdown.cancel();
}
