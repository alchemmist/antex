use std::process::Command;

use image::DynamicImage;
use tempfile::Builder;

use super::PasteImageError;

const READ_PUBLIC_PNG_SCRIPT: &str = r#"
function run(argv) {
    ObjC.import("AppKit");
    const data = $.NSPasteboard.generalPasteboard.dataForType("public.png");
    if (!data) {
        throw new Error("public.png is unavailable");
    }
    if (!data.writeToFileAtomically($(argv[0]), true)) {
        throw new Error("failed to write public.png");
    }
}
"#;

pub(super) fn read_public_png() -> Result<DynamicImage, PasteImageError> {
    let file = Builder::new()
        .prefix("codex-clipboard-public-")
        .suffix(".png")
        .tempfile()
        .map_err(|error| PasteImageError::IoError(error.to_string()))?;
    let output = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", READ_PUBLIC_PNG_SCRIPT])
        .arg(file.path())
        .output()
        .map_err(|error| PasteImageError::ClipboardUnavailable(error.to_string()))?;
    if !output.status.success() {
        return Err(PasteImageError::NoImage(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let bytes =
        std::fs::read(file.path()).map_err(|error| PasteImageError::IoError(error.to_string()))?;
    decode_public_png(&bytes)
}

fn decode_public_png(bytes: &[u8]) -> Result<DynamicImage, PasteImageError> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| PasteImageError::EncodeFailed(error.to_string()))
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
