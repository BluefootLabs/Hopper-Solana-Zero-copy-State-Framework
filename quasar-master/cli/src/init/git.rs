use {
    super::types::GitSetup,
    std::{path::Path, process::Command},
};

pub(super) fn maybe_initialize_git_repo(name: &str, git_setup: GitSetup) {
    if matches!(git_setup, GitSetup::Skip) {
        return;
    }

    let root = Path::new(name);
    let already_git = if name == "." {
        Path::new(".git").exists()
    } else {
        root.join(".git").exists()
    };

    if !already_git {
        let _ = initialize_git_repo(root, git_setup);
    }
}

fn initialize_git_repo(root: &Path, git_setup: GitSetup) -> bool {
    run_git(root, &["init", "--quiet"])
        && match git_setup {
            GitSetup::InitializeAndCommit => {
                run_git(root, &["add", "."])
                    && run_git(root, &["commit", "-am", "chore: initial commit", "--quiet"])
            }
            GitSetup::Initialize | GitSetup::Skip => true,
        }
}

fn run_git(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .ok()
        .is_some_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{
            env, fs,
            path::PathBuf,
            sync::Mutex,
            time::{SystemTime, UNIX_EPOCH},
        },
    };

    static PATH_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn initialize_git_repo_runs_init_add_and_commit() {
        let _guard = PATH_LOCK.lock().unwrap();
        let sandbox = create_test_sandbox("success");
        let _env = TestGitEnv::new(&sandbox, None);
        let root = sandbox.join("repo");
        fs::create_dir_all(&root).unwrap();

        let ok = initialize_git_repo(&root, GitSetup::InitializeAndCommit);

        assert!(ok);
        assert_eq!(
            read_git_log(&sandbox),
            vec![
                "init --quiet",
                "add .",
                "commit -am chore: initial commit --quiet",
            ]
        );
    }

    #[test]
    fn initialize_git_repo_can_skip_initial_commit() {
        let _guard = PATH_LOCK.lock().unwrap();
        let sandbox = create_test_sandbox("init-only");
        let _env = TestGitEnv::new(&sandbox, None);
        let root = sandbox.join("repo");
        fs::create_dir_all(&root).unwrap();

        let ok = initialize_git_repo(&root, GitSetup::Initialize);

        assert!(ok);
        assert_eq!(read_git_log(&sandbox), vec!["init --quiet"]);
    }

    #[test]
    fn initialize_git_repo_stops_when_git_init_fails() {
        let _guard = PATH_LOCK.lock().unwrap();
        let sandbox = create_test_sandbox("fail-init");
        let _env = TestGitEnv::new(&sandbox, Some("init"));
        let root = sandbox.join("repo");
        fs::create_dir_all(&root).unwrap();

        let ok = initialize_git_repo(&root, GitSetup::InitializeAndCommit);

        assert!(!ok);
        assert_eq!(read_git_log(&sandbox), vec!["init --quiet"]);
    }

    fn create_test_sandbox(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "quasar-init-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join("bin")).unwrap();
        dir
    }

    fn read_git_log(sandbox: &Path) -> Vec<String> {
        fs::read_to_string(sandbox.join("git.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| line.to_string())
            .collect()
    }

    struct TestGitEnv {
        old_path: Option<std::ffi::OsString>,
        old_log: Option<std::ffi::OsString>,
        old_fail_on: Option<std::ffi::OsString>,
    }

    impl TestGitEnv {
        fn new(sandbox: &Path, fail_on: Option<&str>) -> Self {
            let bin_dir = sandbox.join("bin");
            let log_path = sandbox.join("git.log");
            write_fake_git(&bin_dir.join("git"));

            let old_path = env::var_os("PATH");
            let old_log = env::var_os("QUASAR_TEST_GIT_LOG");
            let old_fail_on = env::var_os("QUASAR_TEST_GIT_FAIL_ON");

            let mut path = std::ffi::OsString::new();
            path.push(bin_dir.as_os_str());
            path.push(":");
            if let Some(existing) = &old_path {
                path.push(existing);
            }

            unsafe {
                env::set_var("PATH", path);
                env::set_var("QUASAR_TEST_GIT_LOG", &log_path);
            }
            if let Some(cmd) = fail_on {
                unsafe {
                    env::set_var("QUASAR_TEST_GIT_FAIL_ON", cmd);
                }
            } else {
                unsafe {
                    env::remove_var("QUASAR_TEST_GIT_FAIL_ON");
                }
            }

            Self {
                old_path,
                old_log,
                old_fail_on,
            }
        }
    }

    impl Drop for TestGitEnv {
        fn drop(&mut self) {
            unsafe {
                restore_env_var("PATH", self.old_path.as_ref());
                restore_env_var("QUASAR_TEST_GIT_LOG", self.old_log.as_ref());
                restore_env_var("QUASAR_TEST_GIT_FAIL_ON", self.old_fail_on.as_ref());
            }
        }
    }

    unsafe fn restore_env_var(key: &str, value: Option<&std::ffi::OsString>) {
        if let Some(value) = value {
            env::set_var(key, value);
        } else {
            env::remove_var(key);
        }
    }

    fn write_fake_git(path: &Path) {
        fs::write(
            path,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$QUASAR_TEST_GIT_LOG\"\nif [ \"$1\" = \
             \"$QUASAR_TEST_GIT_FAIL_ON\" ]; then\n  exit 1\nfi\nexit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
    }
}
