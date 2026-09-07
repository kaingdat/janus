#![cfg(unix)]

use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("binding an ephemeral port")
        .local_addr()
        .expect("reading the bound address")
        .port()
}

fn wait_for(what: &str, timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out after {timeout:?} waiting for {what}");
}

fn is_running(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .expect("running kill -0")
        .success()
}

fn read_log(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn daemon_logs_from_boot_through_shutdown() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let dir: PathBuf = std::env::temp_dir().join(format!("janus-daemon-logging-{unique}"));
    std::fs::create_dir_all(&dir).expect("creating the test directory");

    let conf = dir.join("main.yaml");
    let log = dir.join("janus.log");
    let pid_file = dir.join("janus.pid");
    let port = free_port();

    std::fs::write(
        &conf,
        format!(
            "version: 1\n\
             threads: 1\n\
             pid_file: {pid}\n\
             upgrade_sock: {sock}\n\
             grace_period_seconds: 0\n\
             graceful_shutdown_timeout_seconds: 1\n\
             proxy_address_http: 127.0.0.1:{port}\n\
             log_level: info\n\
             log_file: {log}\n",
            pid = pid_file.display(),
            sock = dir.join("upgrade.sock").display(),
            log = log.display(),
        ),
    )
    .expect("writing the test config");

    let launch = Command::new(env!("CARGO_BIN_EXE_janus"))
        .args(["-c", conf.to_str().expect("utf-8 config path"), "-d"])
        .status()
        .expect("launching janus");
    assert!(launch.success(), "launcher exited with {launch}");

    wait_for("the pid file", Duration::from_secs(10), || {
        pid_file.exists()
    });
    let pid: u32 = read_log(&pid_file)
        .trim()
        .parse()
        .expect("pid file holds a pid");

    wait_for(
        "the daemon to accept connections",
        Duration::from_secs(10),
        || TcpStream::connect(("127.0.0.1", port)).is_ok(),
    );
    assert!(
        is_running(pid),
        "daemon exited before it could be signalled"
    );

    let signalled = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("sending SIGTERM");
    assert!(signalled.success(), "SIGTERM failed with {signalled}");

    wait_for("the daemon to exit", Duration::from_secs(30), || {
        !is_running(pid)
    });

    let logged = read_log(&log);
    let expected = [
        "Daemonizing the server",
        "file logging switched to the non-blocking writer",
        "All runtimes exited, exiting now",
    ];
    for line in expected {
        assert!(
            logged.contains(line),
            "log file is missing {line:?}; it holds:\n{logged}"
        );
    }

    if let Err(e) = std::fs::remove_dir_all(&dir) {
        if e.kind() != ErrorKind::NotFound {
            eprintln!("could not clean up {}: {e}", dir.display());
        }
    }
}
