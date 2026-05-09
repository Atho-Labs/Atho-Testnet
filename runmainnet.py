#!/usr/bin/env python3
"""Compatibility wrapper for the clearer `mainnet.py` launcher."""

from runtime_launcher import main


if __name__ == "__main__":
    raise SystemExit(
        main(
            "mainnet",
            prog="runmainnet.py",
            compatibility_note="runmainnet.py is kept for compatibility; prefer mainnet.py.",
        )
    )
