#[cfg(target_os = "windows")]
fn main() -> anyhow::Result<()> {
    antex_windows_sandbox::setup_helper_main()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("antex-windows-sandbox-setup is Windows-only");
}
