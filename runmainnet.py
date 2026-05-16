#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright (c) Atho contributors

"""Launch Atho mainnet with the desktop client and managed local node."""

from scripts.runtime_launcher import main


if __name__ == "__main__":
    raise SystemExit(main("mainnet", prog="runmainnet.py"))
