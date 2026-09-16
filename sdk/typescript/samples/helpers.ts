import path from "node:path";

export function antexPathOverride() {
  return (
    process.env.ANTEX_EXECUTABLE ??
    path.join(process.cwd(), "..", "..", "antex-rs", "target", "debug", "codex")
  );
}
