use serde_json::{Value, json};
use std::{fs, process::Command, thread, time::Duration};
use tiny_http::{Header, Response, Server};

fn run_mock(
    args: &[&str],
    reset_failure: bool,
) -> (std::process::Output, Vec<(String, String, Value)>) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let config = tempfile::NamedTempFile::new().unwrap();
    fs::write(
        config.path(),
        format!(
            "url = 'http://{}'\nusername = 'admin'\npassword = 'test-only'\ntimeout_seconds = 2\n",
            server.server_addr()
        ),
    )
    .unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        loop {
            let mut req = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("CLI did not logout");
            let path = req.url().to_owned();
            let method = req.method().to_string();
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            if path != "/api/v1/login" {
                assert!(
                    req.headers()
                        .iter()
                        .any(|h| h.field.equiv("session") && h.value.as_str() == "test-session")
                );
            } else {
                assert_eq!(
                    body,
                    json!({"user":{"name":"admin","password":"test-only"}})
                );
            }
            requests.push((method, path.clone(), body));
            let mut response = match path.split('?').next().unwrap() {
                "/api/v1/login" => json!({"user":{"session":"test-session"}}),
                "/api/v1/swcfg_port" => {
                    json!({"switchPortConfig":{"unit":[{"id":1,"slot":[{"id":1,"port":[{"portNum":1,"linkState":0},{"portNum":2,"linkState":1},{"portNum":9,"linkState":0}]}]}]}})
                }
                "/api/v1/swcfg_poe" => {
                    json!({"poePortConfig":[{"unit":1,"portNum":1,"enable":true,"currentPower":5200},{"unit":1,"portNum":2,"enable":true,"currentPower":3000}]})
                }
                "/api/v1/neighbor" => json!({"lldpRemoteDevice":{"rows":[],"total":0}}),
                "/api/v1/swcfg_poe_reset" | "/api/v1/logout" => json!({}),
                unexpected => panic!("unexpected endpoint {unexpected}"),
            };
            let failure = reset_failure && path == "/api/v1/swcfg_poe_reset";
            response["resp"] = json!({"respCode":if failure {10003} else {0}});
            req.respond(
                Response::from_string(response.to_string())
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap()),
            )
            .unwrap();
            if path == "/api/v1/logout" {
                break;
            }
        }
        requests
    });
    let output = Command::new(env!("CARGO_BIN_EXE_netgear-poe"))
        .args(["--config", config.path().to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap();
    (output, handle.join().unwrap())
}

#[test]
fn dry_run_never_posts_reset() {
    let (output, requests) = run_mock(&["cycle", "1", "2", "--dry-run"], false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"]["dry_run"] == true);
    assert!(
        !requests
            .iter()
            .any(|(_, p, _)| p.ends_with("swcfg_poe_reset"))
    );
}

#[test]
fn cycle_deduplicates_and_sends_exact_payloads() {
    let (output, requests) = run_mock(&["cycle", "2", "1", "1", "--yes"], false);
    assert!(output.status.success());
    let resets: Vec<_> = requests
        .iter()
        .filter(|(_, p, _)| p.ends_with("swcfg_poe_reset"))
        .collect();
    assert_eq!(resets.len(), 2);
    for (req, port) in resets.iter().zip([1, 2]) {
        assert_eq!(req.0, "POST");
        assert_eq!(req.2, json!({"poePortConfig":{"portId":[port]}}));
    }
}

#[test]
fn invalid_selection_blocks_every_reset() {
    let (output, requests) = run_mock(&["cycle", "1", "9", "--yes"], false);
    assert!(!output.status.success());
    assert!(
        !requests
            .iter()
            .any(|(_, p, _)| p.ends_with("swcfg_poe_reset"))
    );
}

#[test]
fn reset_failure_stops_without_retry() {
    let (output, requests) = run_mock(&["cycle", "1", "2", "--yes"], true);
    assert!(!output.status.success());
    assert_eq!(
        requests
            .iter()
            .filter(|(_, p, _)| p.ends_with("swcfg_poe_reset"))
            .count(),
        1
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["ok"],
        false
    );
}

#[test]
fn missing_confirmation_fails_before_credentials_or_network() {
    let output = Command::new(env!("CARGO_BIN_EXE_netgear-poe"))
        .args(["--config", "/does/not/exist", "--json", "cycle", "1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = serde_json::from_slice::<Value>(&output.stdout).unwrap();
    assert!(error["error"].as_str().unwrap().contains("--yes"));
}

#[test]
fn tui_requires_interactive_terminal_and_rejects_json() {
    let binary = env!("CARGO_BIN_EXE_netgear-poe");
    let output = Command::new(binary).arg("tui").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a terminal"));
    let output = Command::new(binary)
        .args(["--json", "tui"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("--json")
    );
}
