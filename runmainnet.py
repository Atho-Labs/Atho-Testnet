#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright (c) Atho contributors

"""Launch Atho mainnet with the desktop client and managed local node."""

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.runtime_launcher import main


if __name__ == "__main__":
    raise SystemExit(main("mainnet", sys.argv[1:], prog="runmainnet.py"))
