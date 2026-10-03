"""Exercise rebuild transactions without building images or starting guests."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import prepare


class PreparationTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.lock = json.loads(prepare.LOCK.read_text())
        self.lock_path = self.root / "environment.lock.json"
        self.original = json.dumps(self.lock).encode()
        self.lock_path.write_bytes(self.original)
        (self.root / "build").mkdir()
        for kind in ("base", "claude", "codex"):
            (self.root / f"{kind}.Dockerfile").write_text("FROM trusted-fixture\n")
            (self.root / "build" / f"{kind}.tar").write_bytes(b"original bundle")
        for name, value in (("HERE", self.root), ("LOCK", self.lock_path)):
            mock = patch.object(prepare, name, value)
            mock.start()
            self.addCleanup(mock.stop)
        self.builds = []

    def command(self, *args, capture=False):
        if args == ("sbx", "version"):
            return "sbx version: v0.45.0 fixture"
        if args[:2] == ("docker", "build"):
            self.builds.append(args)
            self.assertEqual(sorted(path.name for path in Path(args[-1]).iterdir()),
                             ["base.Dockerfile", "claude.Dockerfile", "codex.Dockerfile"])
        elif args[:3] == ("docker", "image", "inspect"):
            kind = args[3].split(":")[0].removeprefix("spaceship-bench-")
            layers = ["base-layer"] + ([] if kind == "base" else [kind + "-layer"])
            return json.dumps([{"Id": "sha256:" + {"base": "a", "claude": "b", "codex": "c"}[kind] * 64,
                                "RootFS": {"Layers": layers}, "Architecture": "amd64"}])
        elif args[:3] == ("docker", "image", "save"):
            Path(args[4]).write_bytes(b"candidate bundle")
        elif args[:3] != ("sbx", "template", "load"):
            self.fail(f"Unexpected command: {args}")

    def run_prepare(self, rebuild=False, failure=None):
        argv = ["prepare.py"] + (["--rebuild"] if rebuild else [])
        with patch("sys.argv", argv), patch.object(prepare.shutil, "which", return_value="fixture"), \
                patch.object(prepare, "command", side_effect=self.command), \
                patch.object(prepare, "verify_runtime", side_effect=failure) as verify:
            prepare.main()
        return verify

    def test_resolved_images_require_explicit_rebuild(self):
        with self.assertRaisesRegex(SystemExit, "--rebuild"):
            self.run_prepare()
        self.assertEqual(self.builds, [])
        self.assertEqual(self.lock_path.read_bytes(), self.original)

    def test_rebuild_uses_current_base_and_both_package_pins(self):
        verify = self.run_prepare(rebuild=True)
        self.assertEqual([call.args[0] for call in verify.call_args_list], ["base", "claude", "codex"])
        self.assertEqual(len(self.builds), 3)
        for build, kind in zip(self.builds[1:], ("claude", "codex")):
            self.assertIn("BASE_IMAGE=spaceship-bench-base:linux-rust", build)
            self.assertIn(f"AGENT_VERSION={self.lock['agents'][kind]['version']}", build)
            self.assertIn(f"AGENT_NPM_INTEGRITY={self.lock['agents'][kind]['npm_integrity']}", build)
        resolved = json.loads(self.lock_path.read_text())
        self.assertEqual(resolved["images"]["base"]["image_id"], "sha256:" + "a" * 64)
        self.assertEqual(resolved["environment"], "linux-rust")
        for kind in ("base", "claude", "codex"):
            bundle = self.root / "build" / f"{kind}.tar"
            self.assertEqual(bundle.read_bytes(), b"candidate bundle")
            self.assertEqual(bundle.stat().st_mode & 0o777, 0o600)

    def test_failed_runtime_preserves_lock_and_existing_bundles(self):
        def fail_last(kind, *_):
            if kind == "codex":
                raise SystemExit("runtime mismatch")
        with self.assertRaisesRegex(SystemExit, "runtime mismatch"):
            self.run_prepare(rebuild=True, failure=fail_last)
        self.assertEqual(self.lock_path.read_bytes(), self.original)
        for kind in ("base", "claude", "codex"):
            self.assertEqual((self.root / "build" / f"{kind}.tar").read_bytes(), b"original bundle")


if __name__ == "__main__":
    unittest.main()
