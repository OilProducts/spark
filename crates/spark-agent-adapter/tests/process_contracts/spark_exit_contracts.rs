//! What Spark starts through `spawn_process_group` ends with Spark's process,
//! even when that is killed without warning, and runs on until then. Here a
//! helper process acts as Spark; it and its child and grandchild are this test
//! binary, run again in another role.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use spark_agent_adapter::local_environment::{configure_process_group, spawn_process_group};

const ROLE_ENV: &str = "SPARK_EXIT_CONTRACT_ROLE";
const DIR_ENV: &str = "SPARK_EXIT_CONTRACT_DIR";
const DEADLINE: Duration = Duration::from_secs(5);

fn role(role: &str, dir: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "spark_exit_contracts::spark_exit_role"])
        .env(ROLE_ENV, role)
        .env(DIR_ENV, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Entry point of the processes the contract starts. A no-op in the ordinary
/// test run.
#[test]
fn spark_exit_role() {
    let (Ok(role_name), Some(dir)) = (std::env::var(ROLE_ENV), std::env::var_os(DIR_ENV)) else {
        return;
    };
    let dir = PathBuf::from(dir);
    match role_name.as_str() {
        // Starts the child as Spark starts agents and tools.
        "spark" => {
            let mut command = role("child", &dir);
            configure_process_group(&mut command);
            spawn_process_group(&mut command).unwrap();
        }
        // Starts the grandchild plainly, as an agent starts its tools. On unix
        // it ignores TERM, so only the KILL that follows stops it.
        "child" => {
            #[cfg(unix)]
            let mut grandchild = {
                let mut command = Command::new("sh");
                command.args(["-c", "trap '' TERM; exec sleep 60"]);
                command
            };
            #[cfg(windows)]
            let mut grandchild = role("grandchild", &dir);
            let grandchild = grandchild.spawn().unwrap();
            let pids = format!("{} {}", std::process::id(), grandchild.id());
            std::fs::write(dir.join("pids.tmp"), pids).unwrap();
            std::fs::rename(dir.join("pids.tmp"), dir.join("pids")).unwrap();
        }
        _ => {}
    }
    std::thread::sleep(Duration::from_secs(60));
}

#[cfg(unix)]
fn running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&output.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

#[cfg(windows)]
fn running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0;
        let queried = GetExitCodeProcess(process, &mut code) != 0;
        CloseHandle(process);
        queried && code == STILL_ACTIVE as u32
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

#[test]
fn what_spark_started_stops_when_spark_is_killed_without_warning() {
    let dir = tempfile::tempdir().unwrap();
    let mut spark = role("spark", dir.path()).spawn().unwrap();
    let mut pids = Vec::new();
    assert!(
        wait_until(|| {
            pids = std::fs::read_to_string(dir.path().join("pids"))
                .unwrap_or_default()
                .split_whitespace()
                .filter_map(|pid| pid.parse::<u32>().ok())
                .collect();
            pids.len() == 2
        }),
        "the child and grandchild never started"
    );

    // Longer than Spark waits between checks on what it started.
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        pids.iter().all(|pid| running(*pid)),
        "{pids:?} stopped while Spark ran"
    );

    // SIGKILL on unix, TerminateProcess on windows.
    spark.kill().unwrap();
    spark.wait().unwrap();
    assert!(
        wait_until(|| !pids.iter().any(|pid| running(*pid))),
        "{pids:?} outlived Spark"
    );
}

/// The lifeline goes once the process it guards has ended, so nothing of the
/// group stays behind.
#[cfg(unix)]
#[test]
fn nothing_stays_behind_once_the_process_spark_started_ends() {
    let mut command = Command::new("sh");
    command.args(["-c", "exit 0"]);
    configure_process_group(&mut command);
    let mut child = spawn_process_group(&mut command).unwrap();
    let group = child.id().to_string();
    child.wait().unwrap();
    let group_running = || {
        let output = Command::new("ps")
            .args(["-A", "-o", "pgid=,stat="])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some(group.as_str())
                && fields.next().is_some_and(|stat| !stat.starts_with('Z'))
        })
    };
    assert!(wait_until(|| !group_running()), "group {group} still runs");
}
