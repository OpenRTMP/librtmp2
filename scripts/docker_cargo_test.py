#!/usr/bin/env python3
"""Run librtmp2 cargo test in rust:latest (C: mount)."""
from __future__ import annotations

import os
import shutil
import subprocess
import sys

SRC = r"X:\AWDev\GitHub\OpenRTMP\librtmp2"
DEST = r"C:\Users\alexg\AppData\Local\Temp\openrtmp-librtmp2-test"
LOG = "librtmp2-test.log"


def main() -> int:
    if os.path.exists(DEST):
        shutil.rmtree(DEST, ignore_errors=True)
    os.makedirs(DEST)
    shutil.copytree(
        SRC,
        os.path.join(DEST, "librtmp2"),
        ignore=shutil.ignore_patterns("target", ".git", "*.log"),
    )
    mount = DEST.replace("\\", "/")
    bash = (
        "export PATH=/usr/local/cargo/bin:$PATH CARGO_TERM_COLOR=never "
        "DEBIAN_FRONTEND=noninteractive; "
        "apt-get update -qq; apt-get install -y -qq pkg-config libssl-dev >/dev/null; "
        "cd /src/librtmp2; "
        "cargo test --all-targets > /src/librtmp2-test.log 2>&1; "
        "echo EXIT=$? >> /src/librtmp2-test.log; "
        "grep -E '^(test result:|error|EXIT=)' /src/librtmp2-test.log || true; "
        "tail -n 25 /src/librtmp2-test.log"
    )
    r = subprocess.run(
        [
            "docker",
            "run",
            "--rm",
            "-v",
            f"{mount}:/src",
            "-w",
            "/src/librtmp2",
            "rust:latest",
            "bash",
            "-c",
            bash,
        ]
    )
    src_log = os.path.join(DEST, LOG)
    if os.path.exists(src_log):
        shutil.copy(src_log, os.path.join(SRC, LOG))
        print(f"copied log bytes={os.path.getsize(src_log)}", flush=True)
    return r.returncode


if __name__ == "__main__":
    sys.exit(main())
