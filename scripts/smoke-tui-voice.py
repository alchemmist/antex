import argparse
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import tempfile
import termios
import time
from pathlib import Path


def smoke(binary):
    with tempfile.TemporaryDirectory(prefix="antex-voice-command-") as temporary:
        home = Path(temporary).resolve()
        (home / "config.toml").write_text(
            'model = "gpt-6-astra"\nmodel_provider = "smoke"\n'
            "check_for_update_on_startup = false\n"
            '[model_providers.smoke]\nname = "smoke"\n'
            'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\n'
            f'[projects.{json.dumps(str(home))}]\ntrust_level = "trusted"\n'
        )
        pid, terminal = pty.fork()
        if pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 110, 0, 0))
            os.environ.update(ANTEX_HOME=str(home), TERM="xterm-256color")
            for key in ("TMUX", "TMUX_PANE", "ZELLIJ", "ANTEX_APP_SERVER_URL"):
                os.environ.pop(key, None)
            os.chdir(home)
            os.execv(
                str(binary),
                [
                    str(binary),
                    "--no-alt-screen",
                    "--dangerously-bypass-approvals-and-sandbox",
                ],
            )
        output = bytearray()
        pending = b""
        ready_at = None
        submitted = False
        enter_at = None
        deadline = time.monotonic() + 30
        try:
            while time.monotonic() < deadline:
                if select.select([terminal], [], [], 0.05)[0]:
                    data = os.read(terminal, 65536)
                    if not data:
                        break
                    output.extend(data)
                    pending += data
                    while b"\x1b[6n" in pending:
                        pending = pending.split(b"\x1b[6n", 1)[1]
                        os.write(terminal, b"\x1b[1;1R")
                    pending = pending[-8:]
                plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", bytes(output))
                if ready_at is None and b"gpt-6-astra" in plain:
                    ready_at = time.monotonic()
                if (
                    ready_at is not None
                    and not submitted
                    and time.monotonic() - ready_at > 3
                ):
                    os.write(terminal, b"/voice settings")
                    enter_at = time.monotonic() + 0.5
                    submitted = True
                if enter_at is not None and time.monotonic() >= enter_at:
                    os.write(terminal, b"\r")
                    enter_at = None
                if b"Unrecognized command '/voice'" in plain:
                    raise RuntimeError("installed TUI does not recognize /voice")
                if b"Select voice" in plain and b"next voice conversation" in plain:
                    print(
                        "Installed TUI passed: /voice settings opens the voice picker"
                    )
                    return
            raise RuntimeError(
                f"installed TUI did not open the voice picker: {plain[-1200:]!r}"
            )
        finally:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            os.waitpid(pid, 0)
            os.close(terminal)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    smoke(args.binary.absolute())
