//! Tests that run the real `huginn` binary as a subprocess.
//!
//! Every other test in this workspace exercises `run()` (or the web server) by
//! spawning it into the test's own Tokio runtime — a runtime that outlives the
//! thing under test. Production has no such runtime: when `main()` returns, the
//! runtime is dropped and every spawned task is cancelled. That gap is exactly
//! where the "daemon exits at startup, having probed nothing" bug lived, and it
//! is why no in-process test could see it.
//!
//! These tests close the gap by observing the shipped artefact: does the process
//! stay up, does it serve, does it actually probe.

use std::io::Write as _;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};

/// Bind port 0 and return the OS-assigned free port.
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// Write a config that probes a closed local port every second, so a probe
/// result appears quickly without touching the network. InfluxDB points at a
/// dead port on purpose — write failures must not take the process down.
/// `health.port` is written explicitly, and every caller passes a free one.
/// The default is a *fixed* port because the liveness listener is on by default
/// and must be findable without configuration — which is right for a container
/// with its own network namespace, and wrong for several huginns sharing a host.
/// These tests are the second case: they run in parallel, so without this they
/// race for one socket and whichever loses exits at startup.
fn write_config(dir: &std::path::Path, ui_port: u16, health_port: u16) -> std::path::PathBuf {
    let token = dir.join("token.txt");
    std::fs::write(&token, "test-token").unwrap();

    let cfg_path = dir.join("config.yaml");
    let mut f = std::fs::File::create(&cfg_path).unwrap();
    write!(
        f,
        r#"
influx:
  url: "http://127.0.0.1:1"
  org: "testorg"
  bucket: "testbucket"
  token_file: "{token}"
  batch_size: 1
  batch_timeout_ms: 100
ui:
  enabled: true
  port: {ui_port}
health:
  enabled: true
  port: {health_port}
log:
  format: json
  level: info
probes:
  - name: "closed-port"
    type: tcp
    target: "127.0.0.1:1"
    interval_secs: 1
    timeout_secs: 1
"#,
        token = token.display().to_string().replace('\\', "/"),
        ui_port = ui_port,
        health_port = health_port,
    )
    .unwrap();
    cfg_path
}

fn spawn_huginn(cfg: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_huginn"))
        .arg("--config")
        .arg(cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn the huginn binary")
}

/// Poll `f` until it returns true or `limit` elapses.
async fn wait_until<F, Fut>(limit: Duration, mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    tokio::time::timeout(limit, async {
        loop {
            if f().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .is_ok()
}

/// The binary must still be running a second after start.
///
/// Before the keep-alive fix this failed: the process logged "Web UI listening"
/// and exited 0 in the same millisecond, having run zero probes.
#[tokio::test]
async fn binary_stays_alive_after_startup() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path(), free_port(), free_port());
    let mut child = spawn_huginn(&cfg);

    tokio::time::sleep(Duration::from_secs(1)).await;

    if let Some(status) = child.try_wait().expect("try_wait failed") {
        let out = child.wait_with_output().await.unwrap();
        panic!(
            "binary exited on its own with {status} — it must run until signalled.\n\
             stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }

    child.kill().await.ok();
}

/// The binary must serve /health — the web task has to survive `main` finishing
/// its setup path.
#[tokio::test]
async fn binary_serves_health_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let cfg = write_config(dir.path(), port, free_port());
    let mut child = spawn_huginn(&cfg);

    let url = format!("http://127.0.0.1:{port}/health");
    let ok = wait_until(Duration::from_secs(10), || async {
        matches!(reqwest::get(&url).await, Ok(r) if r.status().is_success())
    })
    .await;

    child.kill().await.ok();

    assert!(
        ok,
        "/health never responded — the web server did not survive startup"
    );
}

/// `huginn healthcheck` against a running huginn — exactly what the image's
/// `HEALTHCHECK` runs, as a second process, which is the only way this can be
/// tested honestly.
///
/// Both directions matter. Against a live daemon it must exit 0, or the
/// container is marked unhealthy while it is working. Against nothing at all it
/// must exit non-zero, or the check reports healthy for a process that is gone —
/// which is worse than having no check, because it looks like one.
#[tokio::test]
async fn healthcheck_subcommand_reports_liveness_of_a_running_binary() {
    let dir = tempfile::tempdir().unwrap();
    let health_port = free_port();
    let cfg = write_config(dir.path(), free_port(), health_port);

    // Nothing running yet: the check must fail.
    let before = Command::new(env!("CARGO_BIN_EXE_huginn"))
        .arg("--config")
        .arg(&cfg)
        .arg("healthcheck")
        .output()
        .await
        .expect("failed to run the healthcheck subcommand");
    assert!(
        !before.status.success(),
        "healthcheck reported success with no huginn running: {}",
        String::from_utf8_lossy(&before.stderr)
    );

    let mut child = spawn_huginn(&cfg);

    // Poll the subcommand itself rather than the socket — the thing under test
    // is the command the container runs, not the port it happens to use.
    let healthy = wait_until(Duration::from_secs(15), || {
        let cfg = cfg.clone();
        async move {
            Command::new(env!("CARGO_BIN_EXE_huginn"))
                .arg("--config")
                .arg(&cfg)
                .arg("healthcheck")
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
    })
    .await;

    child.kill().await.ok();

    assert!(
        healthy,
        "healthcheck never reported a running huginn as healthy"
    );
}

/// Running the binary with no subcommand must still start the daemon.
///
/// The image's ENTRYPOINT carries no subcommand, so if adding one had made it
/// required, every existing deployment would fail at once — and the failure
/// would be a usage message, which reads nothing like "your monitor is down".
#[tokio::test]
async fn binary_without_a_subcommand_still_runs_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let cfg = write_config(dir.path(), port, free_port());
    let mut child = spawn_huginn(&cfg); // no subcommand — the ENTRYPOINT shape

    let url = format!("http://127.0.0.1:{port}/health");
    let serving = wait_until(Duration::from_secs(10), || async {
        matches!(reqwest::get(&url).await, Ok(r) if r.status().is_success())
    })
    .await;

    child.kill().await.ok();
    assert!(
        serving,
        "the bare binary did not start the daemon — the container contract is broken"
    );
}

/// The binary must actually execute probes and expose the result.
///
/// Liveness alone is not enough: a process that stays up but never probes is
/// just as broken for a monitor. This asserts the probe loop really ticks.
#[tokio::test]
async fn binary_executes_probes_and_reports_them() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let cfg = write_config(dir.path(), port, free_port());
    let mut child = spawn_huginn(&cfg);

    let url = format!("http://127.0.0.1:{port}/metrics/latest");
    let probed = wait_until(Duration::from_secs(15), || async {
        // The probe targets a closed port, so it reports down — that is still a
        // completed probe, which is what is being asserted here.
        match reqwest::get(&url).await {
            Ok(r) => matches!(r.text().await, Ok(body) if body.contains("closed-port")),
            Err(_) => false,
        }
    })
    .await;

    child.kill().await.ok();

    assert!(
        probed,
        "no probe result appeared within 15s — the scheduler never ran a probe"
    );
}
