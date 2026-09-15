#[cfg(antex_bazel)]
pub use code_mode_proto::codex::code_mode::v1::*;

#[cfg(not(antex_bazel))]
tonic::include_proto!("codex.code_mode.v1");

pub const MAX_IDENTIFIER_BYTES: usize = 256;
