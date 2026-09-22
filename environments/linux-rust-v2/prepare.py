#!/usr/bin/env python3
"""Build trusted images only, resolve their identities, load into local sbx.
No generated submission is ever built by Docker on the host. No publishing/login.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import uuid

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
LOCK = HERE / "environment.lock.json"


def command(*args, capture=False):
    return subprocess.run(args, cwd=HERE, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout


def verify_runtime(kind, image, lock):
    """Reject unusable candidate images before freezing the environment lock."""
    name = "bench-image-" + uuid.uuid4().hex
    agent = "shell" if kind == "base" else kind
    env = {key: value for key, value in os.environ.items() if key in (
        "PATH", "HOME", "USER", "LOGNAME", "XDG_CONFIG_HOME", "XDG_DATA_HOME",
        "XDG_STATE_HOME", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "LANG")}
    env["DOCKER_SANDBOXES_ROOT_SIZE"] = "2048m"

    def control(*args, check=True):
        return subprocess.run(["sbx", *args], env=env, cwd=HERE,
                              stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, timeout=60, check=check)

    try:
        control("create", "--name", name, "--skills", "off", "--deny-network", "**",
                "--cpus", "2", "--memory", "2048m", "--pull", "never",
                "--template", image["reference"], agent)
        observed = json.loads(control("inspect", name, "--json").stdout)
        if observed["image_digest"] != image["image_id"]:
            raise SystemExit("Candidate runtime image identity changed.")
        cli = "rustc" if kind == "base" else kind
        version = control("exec", name, cli, "--version").stdout.decode().strip()
        expected = {"base": "rustc 1.97.0 (2d8144b78 2026-07-07)",
                    "claude": lock["agents"]["claude"]["version"] + " (Claude Code)",
                    "codex": "codex-cli " + lock["agents"]["codex"]["version"]}[kind]
        if version != expected:
            raise SystemExit("Candidate runtime CLI version mismatch.")
        print(f"Verified {kind} runtime startup/version (isolation still checked separately).")
    finally:
        cleanup = control("rm", "--force", name, check=False)
        if cleanup.returncode:
            listing = json.loads(control("ls", "--json").stdout)
            if any(guest["name"] == name for guest in listing["sandboxes"]):
                raise SystemExit(f"Candidate cleanup failed. Run: sbx rm --force {name}")


def main():
    if any((REPO / "runs").glob("*/run.json")):
        raise SystemExit("Environment has archived attempts: preserve these images; create a new environment version before rebuilding.")
    if not shutil.which("docker") or not shutil.which("sbx"):
        raise SystemExit("Install Docker image-building tools and local sbx 0.45.0 first. See docs/SETUP.md. Nothing installed automatically.")
    if not command("sbx", "version", capture=True).startswith("sbx version: v0.45.0 "):
        raise SystemExit("Expected sbx 0.45.0; changed backends need verification.")
    lock = json.loads(LOCK.read_text())
    if any(lock["images"].values()):
        raise SystemExit("Resolved images already recorded; preserve them. Use a new environment version for intentional rebuilds.")
    # Build contexts contain ONLY these Dockerfiles, never the parent checkout.
    build = HERE / "build"
    build.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="context-", dir=build) as context:
        for file in ("base.Dockerfile", "claude.Dockerfile", "codex.Dockerfile"):
            shutil.copyfile(HERE / file, Path(context) / file)
        resolved = {}
        base_layers = None
        for kind in ("base", "claude", "codex"):
            tag = f"spaceship-bench-{kind}:{lock['environment']}"
            args = ["docker", "build", "--tag", tag, "--file", str(Path(context) / f"{kind}.Dockerfile")]
            predecessor = lock["predecessor_images"][kind]
            def check_predecessor():
                current = json.loads(command("docker", "image", "inspect", predecessor["reference"], capture=True))[0]
                if current["Id"] != predecessor["image_id"]:
                    raise SystemExit("Preserved predecessor image identity changed; load its original archive.")
            check_predecessor()
            args += ["--build-arg", f"PREDECESSOR_IMAGE={predecessor['reference']}"]
            command(*args, context)
            check_predecessor()
            record = json.loads(command("docker", "image", "inspect", tag, capture=True))[0]
            image_id = record["Id"]
            if not image_id.startswith("sha256:") or len(image_id) != 71:
                raise SystemExit("Docker did not report a valid content ID.")
            layers = record["RootFS"]["Layers"]
            if kind == "base":
                base_layers = layers
            elif layers[:len(base_layers)] != base_layers:
                raise SystemExit("Agent image does not inherit the recorded base layers.")
            resolved[kind] = {"reference": tag, "image_id": image_id}
            # No container is executed on the host, even for environment probing.
            output = build / f"{kind}.tar"
            command("docker", "image", "save", "--output", str(output), tag)
            output.chmod(0o600)
            command("sbx", "template", "load", str(output))
            verify_runtime(kind, resolved[kind], lock)
        lock["images"] = resolved
        lock["guest_architecture"] = record["Architecture"]
        lock["build_status"] = "images_built_and_loaded; live_isolation_unverified"
        lock["notes"] = "Real upstream registry pins and local Docker image IDs are recorded. Docker's containerd image store may report OCI index IDs rather than configuration IDs. All three images were built and loaded; this does not certify guest isolation."
        lock["native_packages"] = {"location_in_image": "/opt/bench/native-packages.tsv", "content_pinned_by": resolved["base"]["image_id"]}
        # Preserve Docker's actual IDs: the containerd store may report an OCI
        # index ID, whereas classic stores report an image configuration ID.
        path = LOCK.with_suffix(".json.partial")
        with path.open("w") as f:
            json.dump(lock, f, indent=2); f.write("\n"); f.flush(); os.fsync(f.fileno())
        path.replace(LOCK)
    print("Images prepared. Keep build/*.tar outside Git. bench doctor still requires live backend certification.")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError:
        sys.exit("Image preparation failed; lock not updated. No benchmark attempt was started.")
