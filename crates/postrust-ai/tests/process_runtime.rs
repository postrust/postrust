//! The process runtime starting a real child through the manager. No
//! database. Needs `python3`, which CI has: a missing one fails loudly.

mod common;

use std::time::Duration;

use common::manager;
use postrust_ai::manager::AcquireOptions;
use postrust_ai::queue::Priority;
use postrust_ai::{Flavor, ModelKind, ModelSpec, RuntimeSpec};

fn http_server_model(id: &str, dir: &std::path::Path) -> ModelSpec {
    let mut spec = ModelSpec::new(
        id,
        ModelKind::Text,
        Flavor::Generic,
        RuntimeSpec::Process {
            command: "python3".into(),
            args: ["-m", "http.server", "{port}", "--bind", "127.0.0.1"]
                .map(String::from)
                .to_vec(),
            env: Default::default(),
            workdir: Some(dir.display().to_string()),
        },
    );
    spec.memory_mb = 50;
    spec.health_path = Some("/".into());
    spec.load_timeout_secs = 20;
    spec
}

/// Pids of live processes running `http.server {port}`, from `/proc`.
fn servers_on(port: u16) -> Vec<u32> {
    let wanted = format!("http.server\0{port}\0");
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .map(|c| String::from_utf8_lossy(&c).contains(&wanted))
                .unwrap_or(false)
        })
        .collect()
}

fn port_of(base_url: &str) -> u16 {
    base_url.rsplit(':').next().unwrap().parse().unwrap()
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn a_process_model_becomes_ready_and_unload_leaves_no_child() {
    let dir = std::env::temp_dir().join(common::unique("postrust-ai-proc"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hello.txt"), "hi").unwrap();
    let m = manager("proc-test", &[http_server_model("py", &dir)]).await;

    let lease = m
        .acquire(
            "py",
            AcquireOptions::new(Priority::Normal, Duration::from_secs(30)),
        )
        .await
        .unwrap();
    let port = port_of(lease.base_url());
    assert!(m.ready_models().contains(&"py".to_string()));
    let body = reqwest::get(format!("{}/hello.txt", lease.base_url()))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "hi", "the child serves from its workdir");
    assert_eq!(servers_on(port).len(), 1, "one child on the assigned port");

    // A lease in hand makes the instance busy: unload leaves it alone.
    assert_eq!(m.unload("py").unwrap(), (0, 1));
    assert_eq!(servers_on(port).len(), 1);

    drop(lease);
    assert_eq!(m.unload("py").unwrap(), (1, 0));
    wait_until("the instance to go", || m.instances().is_empty()).await;
    wait_until("the child to exit", || servers_on(port).is_empty()).await;
    assert!(
        reqwest::get(format!("http://127.0.0.1:{port}/"))
            .await
            .is_err(),
        "nothing listens on the port any more"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn shutdown_stops_every_child() {
    let dir = std::env::temp_dir().join(common::unique("postrust-ai-proc"));
    std::fs::create_dir_all(&dir).unwrap();
    let m = manager("proc-test", &[http_server_model("py", &dir)]).await;
    let lease = m
        .acquire(
            "py",
            AcquireOptions::new(Priority::Normal, Duration::from_secs(30)),
        )
        .await
        .unwrap();
    let port = port_of(lease.base_url());
    assert_eq!(servers_on(port).len(), 1);
    drop(lease);

    m.shutdown().await;
    wait_until("the child to exit", || servers_on(port).is_empty()).await;
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn maintain_measures_a_running_childs_memory() {
    let dir = std::env::temp_dir().join(common::unique("postrust-ai-proc"));
    std::fs::create_dir_all(&dir).unwrap();
    let m = manager("proc-test", &[http_server_model("py", &dir)]).await;
    let lease = m
        .acquire(
            "py",
            AcquireOptions::new(Priority::Normal, Duration::from_secs(30)),
        )
        .await
        .unwrap();
    assert_eq!(
        m.instances()[0].measured_mem_mb,
        None,
        "nothing is measured before maintain runs"
    );

    m.maintain().await;
    let info = m.instances().remove(0);
    let mem = info
        .measured_mem_mb
        .expect("a python child has a resident set");
    assert!(mem > 0, "python is bigger than 1 MiB");
    assert_eq!(
        info.measured_gpu_mb.is_some(),
        std::process::Command::new("nvidia-smi")
            .arg("-L")
            .output()
            .is_ok_and(|o| o.status.success()),
        "VRAM is measured exactly when nvidia-smi can answer"
    );
    // Information only: the reservation is still the declared one.
    assert_eq!(info.reservation.mem_mb, 50);
    drop(lease);
    m.shutdown().await;
    let _ = std::fs::remove_dir_all(dir);
}
