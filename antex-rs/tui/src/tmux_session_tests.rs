thread_local! {
    pub(crate) static PUBLISHED_THREAD: std::cell::Cell<Option<antex_protocol::ThreadId>> = const { std::cell::Cell::new(None) };
}

use super::*;
use clap::Parser;
use pretty_assertions::assert_eq;

#[test]
fn binding_carries_root_identity_and_quoted_option_values() {
    let id = ThreadId::from_string("01a0d3e1-d263-75c0-8189-c98bc7266f6c").unwrap();
    let cli = crate::cli::Cli::parse_from([
        "antex",
        "--model",
        "model-name",
        "--profile",
        "work",
        "--no-alt-screen",
        "--add-dir",
        "/tmp/path with spaces",
        "a prompt that must not be replayed",
    ]);
    let argv = resume_argv(id, &resume_options(&cli));
    assert_eq!(
        argv,
        vec![
            "antex",
            "resume",
            "01a0d3e1-d263-75c0-8189-c98bc7266f6c",
            "--model",
            "model-name",
            "--profile",
            "work",
            "--no-alt-screen",
            "--add-dir",
            "/tmp/path with spaces",
        ]
    );
    let binding = Binding {
        version: 1,
        thread_id: id,
        pid: 123,
        process_started_at: "Mon Sep 28 12:00:00 2026",
        pane_id: "%19",
        socket_path: "/tmp/tmux-test/default",
        home: Path::new("/tmp/antex-home"),
        resume_argv: argv.clone(),
    };
    assert_eq!(
        serde_json::to_value(binding).unwrap(),
        serde_json::json!({
            "version": 1, "thread_id": id, "pid": 123,
            "process_started_at": "Mon Sep 28 12:00:00 2026", "pane_id": "%19",
            "socket_path": "/tmp/tmux-test/default", "home": "/tmp/antex-home",
            "resume_argv": argv,
        })
    );
}

#[test]
fn resume_target_changes_without_replaying_initial_prompt_or_creating_worktree() {
    let first = ThreadId::from_string("01a0d3e1-d263-75c0-8189-c98bc7266f6c").unwrap();
    let next = ThreadId::from_string("01a0cee1-2987-7b13-81cc-777ec4861be7").unwrap();
    let cli = crate::cli::Cli::parse_from(["antex", "--worktree", "initial prompt"]);
    assert_eq!(
        resume_argv(first, &resume_options(&cli)),
        vec!["antex", "resume", &first.to_string()]
    );
    assert_eq!(
        resume_argv(next, &resume_options(&cli)),
        vec!["antex", "resume", &next.to_string()]
    );
}

#[test]
fn ownership_check_escapes_tmux_format_delimiters() {
    assert_eq!(
        ownership_condition(r###"{"pid":42,"home":"/a#b}"}"###),
        r###"#{==:#{@antex_binding},{"pid":42#,"home":"/a##b#}"#}}"###
    );
}
