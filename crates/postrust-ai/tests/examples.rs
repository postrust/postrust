//! The files under `examples/` are the first thing a reader copies, so they
//! are parsed and validated here, and cannot drift from the code unnoticed.

use std::collections::HashSet;

use postrust_ai::queue::PauseMode;
use postrust_ai::workflow::WorkflowDef;
use postrust_ai::AiConfig;

const CONFIG: &str = include_str!("../examples/postrust-ai.toml");
const WORKFLOW: &str = include_str!("../examples/video-summary.workflow.json");

/// Models the workflow uses that the example config deliberately does not
/// declare: a generic HTTP service that runs ffmpeg, and an embedding model.
/// docs/ai-workers.md shows how to declare them.
const EXTRA_MODELS: [&str; 2] = ["audio-extractor", "embedder"];

#[test]
fn example_config_parses_and_validates() {
    // `from_toml` applies environment overrides (DATABASE_URL and friends),
    // so this checks that the file validates, and the strict parse below
    // checks what the file itself says, whatever the environment holds.
    let config = AiConfig::from_toml(CONFIG).expect("example config validates");
    let ids: Vec<&str> = config.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        ["qwen2-7b", "whisper-large-v3-turbo", "llama3-8b"],
        "the example declares exactly these models"
    );

    let file: AiConfig = toml::from_str(CONFIG).expect("example config has no unknown keys");
    let queues: Vec<(&str, PauseMode)> = file
        .queues
        .iter()
        .map(|q| (q.name.as_str(), q.pause_mode))
        .collect();
    assert_eq!(
        queues,
        [
            ("interactive", PauseMode::Checkpoint),
            ("background", PauseMode::Requeue)
        ]
    );
    assert_eq!(file.queue("background").unwrap().concurrency, 1);
    assert_eq!(file.models.len(), 3);
    assert_eq!(
        file.model("qwen2-7b").unwrap().served_name(),
        "Qwen/Qwen2-7B-Instruct"
    );
    assert!(file.model("whisper-large-v3-turbo").unwrap().result_cache);
    assert_eq!(
        file.model("llama3-8b").unwrap().served_name(),
        "llama3.1:8b"
    );
}

#[test]
fn example_workflow_deserializes_and_validates() {
    let def: WorkflowDef =
        serde_json::from_str(WORKFLOW).expect("example workflow is a WorkflowDef");
    assert_eq!(def.steps.len(), 4);

    // The engine validates against the node's model registry: here, the
    // example config's models plus the two the workflow documents.
    let config = AiConfig::from_toml(CONFIG).unwrap();
    let mut known: HashSet<String> = config.models.iter().map(|m| m.id.clone()).collect();
    known.extend(EXTRA_MODELS.iter().map(|m| m.to_string()));
    def.validate(&known).expect("example workflow validates");

    // And it names no model beyond those, so a rename in either file fails
    // here rather than at the first run.
    for step in &def.steps {
        assert!(known.contains(&step.model), "{}", step.model);
    }
    let fan_out: Vec<&str> = def
        .steps
        .iter()
        .filter(|s| s.for_each.is_some())
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(fan_out, ["embed_segments"]);
}
