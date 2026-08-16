use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hopper-fuzz-adapter-e2e-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn compile_adapter(out_dir: &Path) -> PathBuf {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adapter_process_fixture.rs");
    let executable = out_dir.join(if cfg!(windows) {
        "adapter-process-fixture.exe"
    } else {
        "adapter-process-fixture"
    });
    let output = Command::new("rustc")
        .arg("--edition=2021")
        .arg("-D")
        .arg("warnings")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert_success(&output, "compile process adapter fixture");
    executable
}

fn run_hopper(manifest: &Path, adapter: &Path, extra: &[&str], report: Option<&Path>) -> Output {
    let mut args = vec![
        OsString::from("fuzz"),
        OsString::from("run"),
        OsString::from("--program"),
        manifest.as_os_str().to_owned(),
        OsString::from("--case"),
        OsString::from("layout-0-truncate-0"),
        OsString::from("--adapter"),
        adapter.as_os_str().to_owned(),
        OsString::from("--require-invariant"),
        OsString::from("fixture-business-hook"),
    ];
    for value in extra {
        args.push(OsString::from("--adapter-arg"));
        args.push(OsString::from(value));
    }
    if let Some(path) = report {
        args.push(OsString::from("--report"));
        args.push(path.as_os_str().to_owned());
    }
    Command::new(env!("CARGO_BIN_EXE_hopper"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fuzz_run_crosses_a_real_process_boundary_and_fails_closed() {
    let temp = TestDir::new();
    let adapter = compile_adapter(&temp.0);
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fuzz-minimal.manifest.json");
    let report = temp.0.join("report.json");

    let passed = run_hopper(&manifest, &adapter, &[], Some(&report));
    assert_success(&passed, "execute process adapter fixture");
    let report_json = fs::read_to_string(&report).unwrap();
    assert!(report_json.contains("\"schema\": \"hopper.manifest-fuzz-report.v1\""));
    assert!(report_json.contains("\"passed\": 1"));
    assert!(report_json.contains("\"fixture-business-hook\""));

    let rejected = run_hopper(&manifest, &adapter, &["--omit-invariant"], None);
    assert!(!rejected.status.success());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("omitted invariants"), "stderr: {stderr}");
}
