#!/usr/bin/env python3
"""Compatibility wrapper for the clearer `testnet.py` launcher."""

from runtime_launcher import main


if __name__ == "__main__":
    raise SystemExit(
        main(
            "testnet",
            prog="runtestnet.py",
            compatibility_note="runtestnet.py is kept for compatibility; prefer testnet.py.",
        )
    )
