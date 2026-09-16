#![allow(clippy::expect_used)]

use std::process::Command;

use divan::Bencher;

fn main() {
    divan::main();
}

/// Exercises the Bazel-backed end-to-end benchmark path with a cheap,
/// deterministic Antex invocation. Richer scenarios can add separate
/// benchmark binaries without making the shared harness depend on them.
#[divan::bench(sample_count = 20, sample_size = 1)]
fn antex_help(bencher: Bencher) {
    let antex = antex_utils_cargo_bin::cargo_bin("antex")
        .expect("antex binary should be available through Bazel runfiles");

    bencher.bench_local(move || {
        let output = Command::new(&antex)
            .arg("--help")
            .output()
            .expect("antex --help should run");
        assert!(output.status.success(), "antex --help should succeed");
    });
}
