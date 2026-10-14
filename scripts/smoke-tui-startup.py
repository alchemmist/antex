"""Check ordinary packaged TUI startup with an isolated embedded engine."""

import argparse
import errno
import fcntl
import json
import os
import pty
import re
import select
import signal
import socket
import struct
import tempfile
import termios
import time
from pathlib import Path


def smoke(binary, *, startup_timeout=30):
    # Keep the Unix path below macOS sun_path limits as well as Linux's.
    with (
        tempfile.TemporaryDirectory(
            prefix="antex-tui-startup-", dir="/tmp"
        ) as temporary,
        socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as legacy_socket,
    ):
        home = Path(temporary).resolve()
        legacy_control = home / "app-server-control"
        legacy_control.mkdir()
        legacy_socket.bind(str(legacy_control / "app-server-control.sock"))
        legacy_socket.listen(1)
        (home / "config.toml").write_text(
            'model = "gpt-6-astra"\nmodel_provider = "smoke"\n'
            "check_for_update_on_startup = false\n"
            '[model_providers.smoke]\nname = "smoke"\n'
            'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\n'
            f'[projects.{json.dumps(str(home))}]\ntrust_level = "trusted"\n'
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
        deadline = time.monotonic() + startup_timeout
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
                if select.select([legacy_socket], [], [], 0)[0]:
                    raise RuntimeError(
                        "foreground startup connected to a legacy daemon socket"
                    )
                if b"Ask Antex to do anything" in plain:
                    print(
                        "Installed TUI passed: normal embedded startup reached the composer",
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
            if not stopped:
                raise RuntimeError("installed TUI did not terminate after SIGKILL")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--timeout", type=float, default=30)
    args = parser.parse_args()
    smoke(args.binary.absolute(), startup_timeout=args.timeout)
