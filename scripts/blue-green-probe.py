"""Record public-origin availability throughout a blue-green cutover."""

import json
from pathlib import Path
import subprocess
import sys
import time

log_path, ready_path, stop_path = map(Path, sys.argv[1:])
failures = 0
samples = 0
with log_path.open("a", buffering=1) as log:
    while not stop_path.exists():
        started = time.monotonic()
        result = subprocess.run(
            ["curl", "--silent", "--show-error", "--output", "/dev/null",
             "--write-out", "%{http_code}", "--max-time", "5",
             "--resolve", "www.lynshen.org:443:64.90.22.212",
             "https://www.lynshen.org/"],
            capture_output=True, text=True,
        )
        ok = result.returncode == 0 and result.stdout.startswith("2")
        samples += 1
        failures += int(not ok)
        log.write(json.dumps({"time": time.time(), "status": result.stdout,
                              "curl_exit": result.returncode, "ok": ok}) + "\n")
        if samples == 1:
            if not ok:
                sys.exit(1)
            ready_path.touch()
        time.sleep(max(0, 0.4 - (time.monotonic() - started)))
    log.write(json.dumps({"samples": samples, "failures": failures}) + "\n")
sys.exit(int(failures > 0))
