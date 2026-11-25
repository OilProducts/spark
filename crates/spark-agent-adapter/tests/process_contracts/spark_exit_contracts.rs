//! What Spark starts through `spawn_process_group`, and everything that starts,
//! ends with Spark's process, even when that is killed without warning, and
//! runs on until then. Here a helper process acts as Spark; it and what it
//! starts are this test binary, run again in another role.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use spark_agent_adapter::local_environment::{
    configure_process_group, spawn_process_group, terminate_process_group,
};

const ROLE_ENV: &str = "SPARK_EXIT_CONTRACT_ROLE";
const DIR_ENV: &str = "SPARK_EXIT_CONTRACT_DIR";
const DEADLINE: Duration = Duration::from_secs(5);

/// The directory is also an argument, so a process in the role shows it.
fn role(role: &str, dir: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "spark_exit_contracts::spark_exit_role"])
        .arg(dir)
        .env(ROLE_ENV, role)
        .env(DIR_ENV, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn start(role_name: &str, dir: &Path) -> std::io::Result<Child> {
    let mut command = role(role_name, dir);
    configure_process_group(&mut command);
    spawn_process_group(command)
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
        // Starts a child as Spark starts agents and tools, and reaps it.
        "spark" | "spark-of-brief-child" => {
            let child = if role_name == "spark" {
                "child"
            } else {
                "brief-child"
            };
            let outcome = start(child, &dir).and_then(|mut child| child.wait());
            std::fs::write(dir.join("outcome"), format!("{outcome:?}")).unwrap();
        }
        // Starts one process after another until it is killed.
        "launching-spark" => {
            for _ in 0..4 {
                let dir = dir.clone();
                std::thread::spawn(move || loop {
                    std::mem::forget(start("launched", &dir));
                });
            }
        }
        "launched" => {
            std::fs::write(dir.join(format!("launched-{}", std::process::id())), "").unwrap();
        }
        // Starts the grandchild plainly, as an agent starts its tools. On unix
        // it ignores TERM, so only the KILL that follows stops it.
        "child" | "brief-child" => {
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
            if role_name == "brief-child" {
                return;
            }
        }
        _ => {}
    }
    std::thread::sleep(Duration::from_secs(60));
}

#[cfg(unix)]
pub(crate) fn running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&output.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

#[cfg(windows)]
pub(crate) fn running(pid: u32) -> bool {
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

/// The live processes `spark` started, on windows, where a process keeps its
/// parent's ID.
#[cfg(windows)]
fn children_of(spark: u32) -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let mut children = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        assert_ne!(snapshot, INVALID_HANDLE_VALUE);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut listed = Process32FirstW(snapshot, &mut entry) != 0;
        while listed {
            if entry.th32ParentProcessID == spark && running(entry.th32ProcessID) {
                children.push(entry.th32ProcessID);
            }
            listed = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    children
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

fn started_pids(dir: &Path) -> Vec<u32> {
    let mut pids = Vec::new();
    assert!(
        wait_until(|| {
            pids = std::fs::read_to_string(dir.join("pids"))
                .unwrap_or_default()
                .split_whitespace()
                .filter_map(|pid| pid.parse::<u32>().ok())
                .collect();
            pids.len() == 2
        }),
        "the child and grandchild never started"
    );
    pids
}

#[test]
fn what_spark_started_stops_when_spark_is_killed_without_warning() {
    let dir = tempfile::tempdir().unwrap();
    let mut spark = role("spark", dir.path()).spawn().unwrap();
    let pids = started_pids(dir.path());

    // Longer than Spark waits between checks on what it started.
    std::thread::sleep(Duration::from_secs(3));
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

/// As a tool's shell that leaves `sleep 60 &` behind and ends, before the flow
/// moves on to its next tool.
#[test]
fn what_outlives_the_process_spark_started_stops_when_spark_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let mut spark = role("spark-of-brief-child", dir.path()).spawn().unwrap();
    let [child, grandchild] = started_pids(dir.path())[..] else {
        unreachable!()
    };
    assert!(wait_until(|| !running(child)), "the child never ended");

    // Longer than Spark takes to let go of a group with nothing left in it.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        running(grandchild),
        "the grandchild stopped while Spark ran"
    );

    spark.kill().unwrap();
    spark.wait().unwrap();
    assert!(
        wait_until(|| !running(grandchild)),
        "the grandchild outlived Spark"
    );
}

/// Killed at any point of a start, Spark leaves nothing behind, since
/// everything starts in Spark's job. Unix leaves a start's first milliseconds
/// untied; see `spawn_with_lifeline`.
#[cfg(windows)]
#[test]
fn nothing_spark_was_starting_survives_its_kill() {
    let dir = tempfile::tempdir().unwrap();
    let mut spark = role("launching-spark", dir.path()).spawn().unwrap();
    let launched = || {
        std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                name.to_string_lossy().starts_with("launched-")
            })
            .count()
    };
    assert!(wait_until(|| launched() >= 8), "Spark started too little");

    spark.kill().unwrap();
    spark.wait().unwrap();
    let mut left = Vec::new();
    assert!(
        wait_until(|| {
            left = children_of(spark.id());
            left.is_empty()
        }),
        "{left:?} outlived Spark"
    );
}

/// A stop still ends the group as soon as its processes have gone, not at the
/// end of the grace period.
#[cfg(unix)]
#[test]
fn a_stop_still_ends_the_group_as_soon_as_its_processes_go() {
    let mut command = Command::new("sleep");
    command.arg("60");
    configure_process_group(&mut command);
    let mut child = spawn_process_group(command).unwrap();
    // A stop in a start's first moments can land while the lifeline forks its
    // watcher, which then misses the TERM and waits out the grace period (see
    // `spawn_with_lifeline`). Stop once the start has settled: the command
    // and the lifeline's two shells are in the group.
    let group = child.id().to_string();
    assert!(
        wait_until(|| group_members(&group) >= 3),
        "the lifeline never joined group {group}"
    );
    let stopped = Instant::now();
    terminate_process_group(child.id(), Some(&mut child));
    let took = stopped.elapsed();
    assert!(took < Duration::from_secs(1), "the stop took {took:?}");
}

/// Live (not zombie) processes in process group `group`.
#[cfg(unix)]
fn group_members(group: &str) -> usize {
    let output = Command::new("ps")
        .args(["-A", "-o", "pgid=,stat="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some(group) && fields.next().is_some_and(|stat| !stat.starts_with('Z'))
        })
        .count()
}

/// The lifeline goes once what Spark started, and what that started, has gone,
/// so nothing of the group stays behind.
#[cfg(unix)]
#[test]
fn nothing_stays_behind_once_what_spark_started_has_ended() {
    let mut command = Command::new("sh");
    command.args(["-c", "exit 0"]);
    configure_process_group(&mut command);
    let mut child = spawn_process_group(command).unwrap();
    let group = child.id().to_string();
    child.wait().unwrap();
    assert!(
        wait_until(|| group_members(&group) == 0),
        "group {group} still runs"
    );
}
