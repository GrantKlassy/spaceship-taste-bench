"""Inspect only known broker properties before a task; never emit credentials."""
import json
import os
import pathlib

mode = os.environ.get("SBX_CRED_OPENAI_MODE", "none")
if mode not in ("none", "oauth", "apikey"):
    mode = "unknown"

auth_path = pathlib.Path("/home/agent/.codex/auth.json")
placeholder_auth = False
if auth_path.is_file() and not auth_path.is_symlink() and auth_path.stat().st_size <= 4096:
    try:
        placeholder_auth = json.loads(auth_path.read_text()) == {
            "OPENAI_API_KEY": "proxy-managed"}
    except (ValueError, OSError):
        pass

print(json.dumps({
    "mode": mode,
    "placeholder_auth": placeholder_auth,
    "no_api_key_override": not os.environ.get("CODEX_API_KEY") and
        os.environ.get("OPENAI_API_KEY", "") in ("", "proxy-managed"),
}))
