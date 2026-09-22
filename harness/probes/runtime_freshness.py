"""Fixed diagnostic only: no task, account data or arbitrary path input."""
import pathlib
import re
import sys

marker, mode = sys.argv[1:]
assert re.fullmatch(r"\.bench-[a-f0-9]{32}", marker)
assert mode in ("write", "check")
for name in ("projects", "sessions", "todos", "shell-snapshots", "statsig"):
    path = pathlib.Path("/home/agent/.claude") / name / marker
    assert not path.exists(), "previous runtime volume state was inherited"
    if mode == "write":
        with path.open("x") as out:
            out.write("trusted runtime freshness marker\n")
