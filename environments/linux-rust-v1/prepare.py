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

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
LOCK = HERE / "environment.lock.json"


def command(*args, capture=False):
    return subprocess.run(args, cwd=HERE, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout


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
            tag = f"spaceship-bench-{kind}:linux-rust-v1"
            args = ["docker", "build", "--tag", tag, "--file", str(Path(context) / f"{kind}.Dockerfile")]
            if kind != "base":
                # BuildKit parses a bare sha256:... as a repository named sha256.
                # Use the local tag, and reject changes to its recorded identity.
                base = resolved["base"]
                current = json.loads(command("docker", "image", "inspect", base["reference"], capture=True))[0]
                if current["Id"] != base["image_id"]:
                    raise SystemExit("Base image changed during preparation.")
                args += ["--build-arg", f"BASE_IMAGE={base['reference']}"]
            command(*args, context)
            if kind != "base":
                current = json.loads(command("docker", "image", "inspect", base["reference"], capture=True))[0]
                if current["Id"] != base["image_id"]:
                    raise SystemExit("Base image changed during preparation.")
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
            command("sbx", "template", "load", str(output))
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
