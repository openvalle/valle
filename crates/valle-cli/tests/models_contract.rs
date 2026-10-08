#[path = "../../valle-media/test_support/models.rs"]
mod models;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn manifest_server(bytes: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "manifest request timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        // Accepted sockets can inherit the listener's nonblocking mode on macOS.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 1024];
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .unwrap();
        stream.write_all(&bytes).unwrap();
        String::from_utf8(request)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned()
    });
    (endpoint, task)
}

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_valle"));
    command
        .env("VALLE_MODEL_CACHE", root.join("models"))
        .env("VALLE_LEGACY_MODEL_CACHE", root.join("legacy"));
    command
}

#[test]
fn human_model_listing_and_verification_distinguish_ready_missing_and_corrupt() {
    let root = tempfile::tempdir().unwrap();
    let list = command(root.path())
        .args(["models", "list"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let text = String::from_utf8(list.stderr).unwrap();
    assert!(text.contains("birefnet"));
    assert!(text.contains("missing"));
    assert!(text.contains("install:"));
    let installed = models::install(&root.path().join("models"), "birefnet");
    let verified = command(root.path())
        .args([
            "models",
            "verify",
            "birefnet",
            "--version",
            models::VERSION,
            "--backend",
            "onnx",
        ])
        .output()
        .unwrap();
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    assert!(
        String::from_utf8(verified.stderr)
            .unwrap()
            .contains("ready")
    );
    let unrefreshed = command(root.path())
        .args([
            "models",
            "install",
            "birefnet",
            "--version",
            models::VERSION,
            "--backend",
            "onnx",
        ])
        .output()
        .unwrap();
    assert!(!unrefreshed.status.success());
    assert!(
        String::from_utf8(unrefreshed.stderr)
            .unwrap()
            .contains("--refresh-catalog")
    );
    for json in [false, true] {
        let (endpoint, server) =
            manifest_server(std::fs::read(installed.join("release.v1.json")).unwrap());
        let mut cmd = command(root.path());
        cmd.env("HF_ENDPOINT", endpoint).env_remove("HF_TOKEN").env(
            "VALLE_MODEL_CATALOG",
            root.path()
                .join("models/.metadata/catalogs/birefnet-contract.json"),
        );
        cmd.args([
            "models",
            "install",
            "birefnet",
            "--version",
            models::VERSION,
            "--backend",
            "onnx",
            "--refresh-catalog",
        ]);
        if json {
            cmd.arg("--json");
        }
        let output = cmd.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /"));
        assert!(request.contains(models::REVISION));
        if json {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["artifacts"][0]["version"], models::VERSION);
        } else {
            assert!(String::from_utf8(output.stderr).unwrap().contains("reused"));
        }
    }
    let unknown = command(root.path())
        .args(["models", "install", "not-a-model", "--json"])
        .output()
        .unwrap();
    assert!(!unknown.status.success());
    assert!(!unknown.stdout.is_empty());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(installed.join("release.v1.json")).unwrap()).unwrap();
    let artifact = manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == installed.file_name().unwrap().to_str().unwrap())
        .unwrap();
    let graph = installed.join(artifact["files"][0]["path"].as_str().unwrap());
    std::fs::write(graph, b"corrupt graph").unwrap();
    let corrupt = command(root.path())
        .args([
            "models",
            "verify",
            "birefnet",
            "--version",
            models::VERSION,
            "--backend",
            "onnx",
        ])
        .output()
        .unwrap();
    assert_eq!(corrupt.status.code(), Some(3));
    assert!(
        String::from_utf8(corrupt.stderr)
            .unwrap()
            .contains("corrupt")
    );
}
