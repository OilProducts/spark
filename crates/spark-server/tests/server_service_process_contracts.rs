#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn service_install_status_remove_process_uses_systemd_unit_contract() {
    let harness = ServiceHarness::new();

    let install = harness
        .spark_server()
        .args([
            "service",
            "install",
            "--host",
            "127.0.0.1",
            "--port",
            "8123",
            "--data-dir",
            harness.data_dir_text.as_str(),
            "--flows-dir",
            harness.flows_dir_text.as_str(),
            "--ui-dir",
            harness.ui_dir_text.as_str(),
        ])
        .output()
        .expect("service install output");
    assert_success(&install);
    assert_eq!(
        stdout_text(&install),
        format!(
            "Installed Spark user service: {}\nService name: spark.service\nListening on http://127.0.0.1:8123\nInitialized Spark at {}\nSeeded flows: {}\ncreated=9 updated=0 skipped=0\n",
            harness.unit_path.display(),
            harness.data_dir.display(),
            harness.flows_dir.display()
        )
    );

    assert_eq!(count_dot_files(&harness.flows_dir), 9);
    assert!(harness
        .data_dir
        .join("config")
        .join("flow-catalog.toml")
        .is_file());
    let unit = fs::read_to_string(&harness.unit_path).expect("service unit");
    assert!(unit.contains("[Unit]\n"));
    assert!(unit.contains("[Service]\n"));
    assert!(unit.contains("[Install]\n"));
    assert!(unit.contains("Type=simple\n"));
    assert!(unit.contains("Restart=on-failure\n"));
    assert!(unit.contains("RestartSec=2\n"));
    assert!(unit.contains("WantedBy=default.target\n"));
    assert!(unit.contains(&format!(
        "EnvironmentFile=-{}",
        harness.data_dir.join("config/provider.env").display()
    )));
    assert!(unit.contains("Environment=PYTHONUNBUFFERED=1\n"));
    assert!(unit.contains("Environment=PATH="));
    assert!(unit.contains(&format!(
        "Environment=SPARK_HOME={}\n",
        harness.data_dir.display()
    )));
    assert!(unit.contains(&format!(
        "Environment=SPARK_FLOWS_DIR={}\n",
        harness.flows_dir.display()
    )));
    assert!(unit.contains(&format!(
        "Environment=SPARK_UI_DIR={}\n",
        harness.ui_dir.display()
    )));
    assert!(unit.contains(&format!(
        "ExecStart={} serve --host 127.0.0.1 --port 8123 --data-dir {} --flows-dir {} --ui-dir {}\n",
        env!("CARGO_BIN_EXE_spark-server"),
        harness.data_dir.display(),
        harness.flows_dir.display(),
        harness.ui_dir.display()
    )));

    let status = harness
        .spark_server()
        .args(["service", "status"])
        .output()
        .expect("service status output");
    assert_success(&status);
    assert_eq!(
        stdout_text(&status),
        "spark.service - Spark compatibility fake status\n   Active: active (running)\n"
    );
    assert_eq!(stderr_text(&status), "");

    let remove = harness
        .spark_server()
        .args(["service", "remove"])
        .output()
        .expect("service remove output");
    assert_success(&remove);
    assert_eq!(
        stdout_text(&remove),
        format!(
            "Removed Spark user service: {}\n",
            harness.unit_path.display()
        )
    );
    assert!(!harness.unit_path.exists());

    assert_eq!(
        systemctl_invocations(&harness.systemctl_log),
        vec![
            invocation(&["--user", "daemon-reload"]),
            invocation(&["--user", "enable", "spark.service"]),
            invocation(&["--user", "restart", "spark.service"]),
            invocation(&["--user", "--no-pager", "--full", "status", "spark.service"]),
            invocation(&["--user", "disable", "--now", "spark.service"]),
            invocation(&["--user", "daemon-reload"]),
            invocation(&["--user", "reset-failed", "spark.service"]),
        ]
    );
}

#[test]
fn service_install_defaults_to_wildcard_host() {
    let harness = ServiceHarness::new();

    let install = harness
        .spark_server()
        .args([
            "service",
            "install",
            "--data-dir",
            harness.data_dir_text.as_str(),
            "--flows-dir",
            harness.flows_dir_text.as_str(),
        ])
        .output()
        .expect("service install output");
    assert_success(&install);
    assert!(stdout_text(&install).contains("Listening on http://0.0.0.0:8000\n"));

    let unit = fs::read_to_string(&harness.unit_path).expect("service unit");
    assert!(unit.contains(&format!(
        "ExecStart={} serve --host 0.0.0.0 --port 8000 --data-dir {} --flows-dir {}\n",
        env!("CARGO_BIN_EXE_spark-server"),
        harness.data_dir.display(),
        harness.flows_dir.display()
    )));
    assert!(!unit.contains("SPARK_UI_DIR"));
}

struct ServiceHarness {
    _temp: tempfile::TempDir,
    data_dir: PathBuf,
    data_dir_text: String,
    flows_dir: PathBuf,
    flows_dir_text: String,
    ui_dir: PathBuf,
    ui_dir_text: String,
    unit_path: PathBuf,
    systemctl_log: PathBuf,
    path_env: OsString,
    xdg_config_home: PathBuf,
    home: PathBuf,
}

impl ServiceHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let fake_bin = temp.path().join("fake-bin");
        fs::create_dir_all(&fake_bin).expect("fake bin");
        let systemctl_log = temp.path().join("systemctl.log");
        let fake_systemctl = fake_bin.join("systemctl");
        fs::write(
            &fake_systemctl,
            format!(
                "#!/bin/sh\n\
{{\n\
  printf 'cwd=%s\\n' \"$PWD\"\n\
  for arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\"; done\n\
  printf '%s\\n' '---'\n\
}} >> \"{}\"\n\
for arg in \"$@\"; do\n\
  if [ \"$arg\" = status ]; then\n\
    printf '%s\\n' 'spark.service - Spark compatibility fake status'\n\
    printf '%s\\n' '   Active: active (running)'\n\
  fi\n\
done\n\
exit 0\n",
                systemctl_log.display()
            ),
        )
        .expect("fake systemctl");
        let mut permissions = fs::metadata(&fake_systemctl)
            .expect("fake systemctl metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_systemctl, permissions).expect("fake systemctl executable");

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let path_env = std::env::join_paths(
            std::iter::once(fake_bin.clone()).chain(std::env::split_paths(&original_path)),
        )
        .expect("joined PATH");
        let data_dir = temp.path().join("spark-home");
        let flows_dir = temp.path().join("flows");
        let ui_dir = temp.path().join("ui");
        fs::create_dir_all(&ui_dir).expect("ui dir");
        fs::write(ui_dir.join("index.html"), "<!doctype html>\n").expect("ui index");
        let xdg_config_home = temp.path().join("xdg-config");
        let unit_path = xdg_config_home
            .join("systemd")
            .join("user")
            .join("spark.service");
        let home = temp.path().join("home");

        Self {
            _temp: temp,
            data_dir_text: data_dir.to_string_lossy().into_owned(),
            flows_dir_text: flows_dir.to_string_lossy().into_owned(),
            ui_dir_text: ui_dir.to_string_lossy().into_owned(),
            data_dir,
            flows_dir,
            ui_dir,
            unit_path,
            systemctl_log,
            path_env,
            xdg_config_home,
            home,
        }
    }

    fn spark_server(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spark-server"));
        command
            .current_dir(repo_root())
            .env("PATH", &self.path_env)
            .env("XDG_CONFIG_HOME", &self.xdg_config_home)
            .env("HOME", &self.home)
            .env_remove("SPARK_HOME")
            .env_remove("SPARK_FLOWS_DIR")
            .env_remove("SPARK_UI_DIR");
        command
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        stdout_text(output),
        stderr_text(output)
    );
    assert_eq!(stderr_text(output), "");
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn systemctl_invocations(log_path: &Path) -> Vec<Vec<String>> {
    let log = fs::read_to_string(log_path).expect("systemctl log");
    log.split("---\n")
        .filter_map(|entry| {
            let args = entry
                .lines()
                .filter_map(|line| line.strip_prefix("arg="))
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!args.is_empty()).then_some(args)
        })
        .collect()
}

fn invocation(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

fn count_dot_files(root: &Path) -> usize {
    fs::read_dir(root)
        .expect("read flow dir")
        .map(|entry| entry.expect("entry").path())
        .map(|path| {
            if path.is_dir() {
                count_dot_files(&path)
            } else if path.extension().and_then(|value| value.to_str()) == Some("dot") {
                1
            } else {
                0
            }
        })
        .sum()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}
