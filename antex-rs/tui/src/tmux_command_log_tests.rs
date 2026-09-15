use super::*;

#[test]
fn command_log_renders_streaming_output_stdin_fallback_and_statuses() {
    let mut output = Vec::new();
    {
        let mut log = LogWriter::new(
            &mut output,
            "12345abc-0000-4000-8000-000000000000",
            "/workspace",
        )
        .expect("create log writer");
        log.set_separator_width(45);
        let messages = [
            LogMessage::Started {
                call_id: "call-1".to_string(),
                command: "printf hello".to_string(),
                cwd: "/workspace".to_string(),
            },
            LogMessage::Output {
                call_id: "call-1".to_string(),
                output: "hello".to_string(),
            },
            LogMessage::Stdin {
                process_id: "42".to_string(),
                stdin: "yes\n".to_string(),
            },
            LogMessage::Completed {
                call_id: "call-1".to_string(),
                status: CommandExecutionStatus::Completed,
                aggregated_output: Some("hello".to_string()),
                exit_code: Some(0),
                duration_ms: Some(1_250),
            },
            LogMessage::Started {
                call_id: "call-2".to_string(),
                command: "missing-command".to_string(),
                cwd: "/tmp".to_string(),
            },
            LogMessage::Completed {
                call_id: "call-2".to_string(),
                status: CommandExecutionStatus::Failed,
                aggregated_output: Some("command not found\n".to_string()),
                exit_code: Some(127),
                duration_ms: Some(8),
            },
        ];
        for message in messages {
            assert!(log.handle(message).expect("write log message"));
        }
        log.write_dropped_marker_if_needed(&AtomicBool::new(true))
            .expect("write dropped marker");
        assert!(!log.handle(LogMessage::Shutdown).expect("write session end"));
    }

    let output = String::from_utf8(output).expect("UTF-8 log output");
    assert!(output.contains(GREEN_COMMAND_PROMPT));
    insta::assert_snapshot!(output.replace(GREEN_COMMAND_PROMPT, "$"));
}

#[cfg(unix)]
#[test]
fn tmux_command_log_child_process() {
    if std::env::var_os("ANTEX_TMUX_COMMAND_LOG_TEST_CHILD").is_none() {
        return;
    }

    let thread_id =
        ThreadId::try_from("12345abc-0000-4000-8000-000000000000").expect("valid thread id");
    let log = TmuxCommandLog::start(thread_id, "/workspace").expect("start tmux command log");
    log.send(LogMessage::Started {
        call_id: "call-e2e".to_string(),
        command: "printf e2e-output".to_string(),
        cwd: "/workspace".to_string(),
    });
    log.send(LogMessage::Output {
        call_id: "call-e2e".to_string(),
        output: "e2e-output\n".to_string(),
    });
    log.send(LogMessage::Completed {
        call_id: "call-e2e".to_string(),
        status: CommandExecutionStatus::Completed,
        aggregated_output: Some("e2e-output\n".to_string()),
        exit_code: Some(0),
        duration_ms: Some(12),
    });

    let window_id = wait_for_command_log_window();
    let status = std::process::Command::new("tmux")
        .args(["kill-window", "-t", &window_id])
        .status()
        .expect("close command log window");
    assert!(status.success());
    assert_eq!(log.ensure_viewer(), Ok(TmuxViewerState::Reopened));

    log.send(LogMessage::Started {
        call_id: "call-after-reopen".to_string(),
        command: "printf after-reopen".to_string(),
        cwd: "/workspace".to_string(),
    });
    log.send(LogMessage::Output {
        call_id: "call-after-reopen".to_string(),
        output: "after-reopen\n".to_string(),
    });
    log.send(LogMessage::Completed {
        call_id: "call-after-reopen".to_string(),
        status: CommandExecutionStatus::Completed,
        aggregated_output: Some("after-reopen\n".to_string()),
        exit_code: Some(0),
        duration_ms: Some(14),
    });
    drop(log);
}

#[cfg(unix)]
fn wait_for_command_log_window() -> String {
    for _ in 0..50 {
        let output = std::process::Command::new("tmux")
            .args([
                "list-windows",
                "-a",
                "-F",
                "#{window_id}\t#{@codex_log_session_id}",
            ])
            .output()
            .expect("list tmux windows");
        assert!(output.status.success());
        let windows = String::from_utf8(output.stdout).expect("UTF-8 tmux windows");
        if let Some((window_id, _)) = windows.lines().find_map(|line| {
            let (window_id, thread_id) = line.split_once('\t')?;
            (thread_id == "12345abc-0000-4000-8000-000000000000")
                .then_some((window_id.to_string(), thread_id))
        }) {
            return window_id;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for tmux command log window");
}

#[cfg(unix)]
#[test]
fn tmux_command_log_creates_last_window_and_streams_output() {
    if std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .is_err()
    {
        return;
    }

    let socket_name = format!("codex-tmux-log-test-{}", Uuid::new_v4());
    let server = TmuxTestServer::start(socket_name);
    server.run(&["new-window", "-d", "-t", "logtest:5", "-n", "existing"]);
    let tmux = server.stdout(&[
        "display-message",
        "-p",
        "-t",
        "logtest:0",
        "#{socket_path},#{pid},0",
    ]);
    let pane_id = server
        .stdout(&["display-message", "-p", "-t", "logtest:0", "#{pane_id}"])
        .trim()
        .to_string();

    let child = std::process::Command::new(std::env::current_exe().expect("current test binary"))
        .args([
            "--exact",
            "tmux_command_log::tests::tmux_command_log_child_process",
        ])
        .env("ANTEX_TMUX_COMMAND_LOG_TEST_CHILD", "1")
        .env("TMUX", tmux.trim())
        .env("TMUX_PANE", pane_id)
        .output()
        .expect("run command log child test");
    assert!(
        child.status.success(),
        "child test failed: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    std::thread::sleep(std::time::Duration::from_millis(700));

    let windows = server.stdout(&[
        "list-windows",
        "-t",
        "logtest",
        "-F",
        "#{window_index}\t#{window_name}\t#{@codex_log_session_id}",
    ]);
    assert!(
        windows
            .lines()
            .any(|line| line == "6\tcl-12345abc\t12345abc-0000-4000-8000-000000000000"),
        "unexpected tmux windows: {windows}"
    );
    let captured = server.stdout(&["capture-pane", "-p", "-t", "logtest:6"]);
    assert!(captured.contains("$ printf e2e-output"));
    assert!(captured.contains("e2e-output"));
    assert!(captured.contains("$ printf after-reopen"));
    assert!(captured.contains("after-reopen"));
    assert!(captured.contains("[✓ exit 0 · 12 ms]"));
    assert!(captured.contains("[codex session ended]"));
    let pane_width = server
        .stdout(&["display-message", "-p", "-t", "logtest:6", "#{pane_width}"])
        .trim()
        .parse::<usize>()
        .expect("numeric pane width");
    assert!(captured.lines().any(|line| {
        !line.is_empty()
            && line.chars().all(|character| character == '─')
            && line.chars().count() == pane_width
    }));
}

#[cfg(unix)]
struct TmuxTestServer {
    socket_name: String,
}

#[cfg(unix)]
impl TmuxTestServer {
    fn start(socket_name: String) -> Self {
        let server = Self { socket_name };
        server.run(&["-f", "/dev/null", "new-session", "-d", "-s", "logtest"]);
        server
    }

    fn run(&self, args: &[&str]) {
        let output = std::process::Command::new("tmux")
            .arg("-L")
            .arg(&self.socket_name)
            .args(args)
            .output()
            .expect("run isolated tmux command");
        assert!(
            output.status.success(),
            "isolated tmux command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn stdout(&self, args: &[&str]) -> String {
        let output = std::process::Command::new("tmux")
            .arg("-L")
            .arg(&self.socket_name)
            .args(args)
            .output()
            .expect("run isolated tmux command");
        assert!(
            output.status.success(),
            "isolated tmux command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("UTF-8 tmux output")
    }
}

#[cfg(unix)]
impl Drop for TmuxTestServer {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .arg("-L")
            .arg(&self.socket_name)
            .arg("kill-server")
            .output();
    }
}
