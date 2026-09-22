"""Read-only, non-billable probe of the observed public dependency destinations."""
import hashlib
import json
import socket
import urllib.error
import urllib.request

with urllib.request.urlopen("https://index.crates.io/config.json", timeout=5) as response:
    assert response.status == 200
    assert json.load(response)["dl"] == "https://static.crates.io/crates"
with urllib.request.urlopen("https://index.crates.io/it/oa/itoa", timeout=5) as response:
    records = [json.loads(line) for line in response.read(200000).splitlines()]
    package = next(record for record in records if record["vers"] == "1.0.15")
with urllib.request.urlopen("https://static.crates.io/crates/itoa/itoa-1.0.15.crate", timeout=5) as response:
    assert response.status == 200
    assert hashlib.sha256(response.read(100000)).hexdigest() == package["cksum"]
# A positive control with HTTP(S)_PROXY ignored checks the transparent route too.
direct = urllib.request.build_opener(urllib.request.ProxyHandler({}))
with direct.open("https://index.crates.io/config.json", timeout=5) as response:
    assert response.status == 200
    assert json.load(response)["dl"] == "https://static.crates.io/crates"
for target in ["https://example.com", "https://github.com",
               "https://api.anthropic.com", "https://api.openai.com"]:
    for open_url in (urllib.request.urlopen, direct.open):
        try:
            open_url(target, timeout=3)
        except urllib.error.HTTPError as error:
            assert error.code == 403
            continue
        except urllib.error.URLError as error:
            # The direct route filters DNS for denied hosts. The successful
            # direct index request above rules out universally broken DNS.
            if open_url == direct.open and isinstance(error.reason, socket.gaierror):
                assert error.reason.errno in (socket.EAI_NONAME, socket.EAI_NODATA)
                continue
            raise
        raise SystemExit("unexpected access outside dependency policy")
print("dependency connectivity verified")
