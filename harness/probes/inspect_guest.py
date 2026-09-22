"""Trusted pre-task probe. Report properties, never environment/token values."""
import json
import os
import platform
import pathlib
import stat
import subprocess

paths = ["/", "/workspace", "/home/agent", "/tmp"]
storage = []
for path in paths:
    info = os.statvfs(path)
    storage.append({"device": os.stat(path).st_dev,
                    "bytes": info.f_blocks * info.f_frsize})
mounts = json.loads(subprocess.check_output([
    "findmnt", "--json", "--output", "TARGET,SOURCE,FSTYPE,OPTIONS"], text=True))
runtime_storage = {}


def inspect_storage(items):
    for item in items:
        if item["fstype"] == "ext4":
            path = item["target"]
            info = os.statvfs(path)
            device = os.stat(path).st_dev
            sectors = pathlib.Path(
                f"/sys/dev/block/{os.major(device)}:{os.minor(device)}/size")
            runtime_storage[path] = {"device": device,
                                     "bytes": info.f_blocks * info.f_frsize,
                                     "device_bytes": int(sectors.read_text()) * 512}
        inspect_storage(item.get("children", []))


inspect_storage(mounts["filesystems"])
with open("/proc/meminfo", encoding="ascii") as source:
    memory = int(source.readline().split()[1]) * 1024
socket_path = os.environ.get("SSH_AUTH_SOCK", "")
socket_present = bool(socket_path) and os.path.exists(socket_path)
socket_present = socket_present and stat.S_ISSOCK(os.stat(socket_path).st_mode)
state_paths = ["/home/agent/.claude", "/home/agent/.codex", "/home/agent/.agents",
               "/home/agent/.config/claude", "/home/agent/.config/codex",
               "/workspace/AGENTS.md", "/workspace/CLAUDE.md",
               "/home/agent/AGENTS.md", "/home/agent/CLAUDE.md",
               "/AGENTS.md", "/CLAUDE.md"]
print(json.dumps({
    "workspace_empty": not os.listdir("/workspace"),
    "personal_state_absent": not any(os.path.exists(p) for p in state_paths),
    "ssh_socket_present": socket_present,
    "host_socket_present": any(os.path.exists(p) for p in
        ["/var/run/docker.sock", "/run/podman/podman.sock", "/run/host-services"]),
    "environment_keys": sorted(os.environ),
    "cpus": os.cpu_count(), "memory_bytes": memory, "storage": storage,
    "runtime_storage": runtime_storage,
    "rust": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "architecture": platform.machine(),
    "mounts": mounts
}))
