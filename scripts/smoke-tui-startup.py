"""Check ordinary packaged TUI startup with an isolated, disposable daemon."""

import argparse
import errno
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path


def smoke(binary):
    with tempfile.TemporaryDirectory(prefix="antex-tui-startup-") as temporary:
        home = Path(temporary).resolve()
        (home / "config.toml").write_text(
            'model = "gpt-6-astra"\nmodel_provider = "smoke"\n'
            "check_for_update_on_startup = false\n"
            '[model_providers.smoke]\nname = "smoke"\n'
            'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\n'
            f'[projects.{json.dumps(str(home))}]\ntrust_level = "trusted"\n'
        )
        state = home / "app-server-daemon"
        state.mkdir()
        (state / "settings.json").write_text(
            json.dumps(
                {
                    "shutdownGraceSeconds": 1,
                    "updater": {"autoUpdateEnabled": False},
                }
            )
        )
        environment = os.environ | {"ANTEX_HOME": str(home), "TERM": "xterm-256color"}
        for key in ("TMUX", "TMUX_PANE", "ZELLIJ", "ANTEX_APP_SERVER_URL"):
            environment.pop(key, None)
        pid, terminal = pty.fork()
        if pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 110, 0, 0))
            os.chdir(home)
            os.execve(str(binary), [str(binary), "--no-alt-screen"], environment)
        output = bytearray()
        pending = b""
        plain = b""
        deadline = time.monotonic() + 30
        try:
            while time.monotonic() < deadline:
                if select.select([terminal], [], [], 0.05)[0]:
                    try:
                        data = os.read(terminal, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        break
                    if not data:
                        break
                    output.extend(data)
                    pending += data
                    while b"\x1b[6n" in pending:
                        pending = pending.split(b"\x1b[6n", 1)[1]
                        os.write(terminal, b"\x1b[1;1R")
                    pending = pending[-8:]
                plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", bytes(output))
                if b"Ask Antex to do anything" in plain:
                    print(
                        "Installed TUI passed: normal startup reached the composer",
                        flush=True,
                    )
                    return
            raise RuntimeError(
                f"installed TUI did not reach the composer: {plain[-1200:]!r}"
            )
        finally:
            os.close(terminal)
            stopped = False
            for termination_signal in (signal.SIGTERM, signal.SIGKILL):
                if os.waitpid(pid, os.WNOHANG)[0] == pid:
                    stopped = True
                    break
                try:
                    os.killpg(pid, termination_signal)
                except PermissionError:
                    try:
                        os.kill(pid, termination_signal)
                    except ProcessLookupError:
                        pass
                except ProcessLookupError:
                    pass
                stop_deadline = time.monotonic() + 1
                while time.monotonic() < stop_deadline:
                    if os.waitpid(pid, os.WNOHANG)[0] == pid:
                        stopped = True
                        break
                    time.sleep(0.02)
                if stopped:
                    break
            subprocess.run(
                [str(binary), "app-server", "daemon", "stop"],
                env=environment,
                cwd=home,
                capture_output=True,
                timeout=30,
                check=True,
            )
            if not stopped:
                raise RuntimeError("installed TUI did not terminate after SIGKILL")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    smoke(args.binary.absolute())
