import argparse
import json
import os
import select
import struct
import subprocess
import time
from pathlib import Path


def smoke(binary, commit):
    package = binary.resolve().parent.parent
    helper = package / "antex-resources/voice/bin/antex-voice-host"
    actual = subprocess.check_output(
        [str(helper), "--build-commit"], timeout=20, text=True
    ).strip()
    if actual != commit:
        raise RuntimeError("voice helper build does not match the application")
    environment = {
        key: value
        for key, value in os.environ.items()
        if key in ("HOME", "TMPDIR", "TMP", "TEMP")
    }
    environment.update(
        {
            "GST_PLUGIN_PATH": "",
            "GST_PLUGIN_PATH_1_0": "",
            "GST_PLUGIN_SYSTEM_PATH": "",
            "GST_PLUGIN_SYSTEM_PATH_1_0": "",
            "GST_REGISTRY": "/dev/null",
            "GST_REGISTRY_UPDATE": "no",
            "GST_REGISTRY_FORK": "no",
        }
    )
    process = subprocess.Popen(
        [str(helper)],
        cwd=package,
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        for request, expected in [
            ({"type": "hello", "protocol": 1, "buildCommit": commit}, "ready"),
            ({"type": "initializeRuntime"}, "runtimeReady"),
            ({"type": "close"}, "closed"),
        ]:
            data = json.dumps(request).encode()
            process.stdin.write(struct.pack(">I", len(data)) + data)
            process.stdin.flush()
            response = bytearray()
            length = 4
            deadline = time.monotonic() + 30
            while len(response) < length:
                remaining = deadline - time.monotonic()
                if (
                    remaining <= 0
                    or not select.select([process.stdout], [], [], remaining)[0]
                ):
                    raise RuntimeError("voice helper response timed out")
                chunk = os.read(process.stdout.fileno(), length - len(response))
                if not chunk:
                    raise RuntimeError(f"voice helper exited before {expected}")
                response.extend(chunk)
                if len(response) == 4:
                    length = 4 + struct.unpack(">I", response)[0]
                    if length > 128 * 1024 + 4:
                        raise RuntimeError("oversized voice helper response")
            if json.loads(response[4:])["type"] != expected:
                raise RuntimeError(f"unexpected response to {request['type']}")
        if process.wait(timeout=10) != 0:
            raise RuntimeError("voice helper did not shut down cleanly")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
    print(
        "Voice package passed: matching build, native runtime initialization, clean shutdown"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("commit")
    args = parser.parse_args()
    smoke(args.binary, args.commit)
