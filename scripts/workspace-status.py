import os
import re

commit = os.environ.get("ANTEX_BUILD_COMMIT", "unknown")
if not re.fullmatch(r"(?:[0-9a-f]{8,40}(?:\+dirty)?|unknown|dev)", commit):
    raise SystemExit("Invalid Antex build commit")
print(f"STABLE_GIT_COMMIT {commit}")
