#!/usr/bin/env python3
"""
BitTorrent-rs Cross-Platform Packaging Dispatcher
Automatically detects the host platform (Windows, macOS, or Linux) and executes
the corresponding release packaging script.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description="Build BitTorrent distribution packages for the current platform.")
    parser.add_argument("--version", default="0.2.0", help="Release version string (default: 0.2.0)")
    parser.add_argument("--skip-build", action="store_true", help="Skip cargo release compilation")
    parser.add_argument("--target-os", choices=["auto", "windows", "macos", "linux"], default="auto",
                        help="Target OS packaging suite to execute (default: auto)")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent

    target_os = args.target_os
    if target_os == "auto":
        if sys.platform == "win32":
            target_os = "windows"
        elif sys.platform == "darwin":
            target_os = "macos"
        else:
            target_os = "linux"

    print(f"=== BitTorrent-rs Cross-Platform Packaging ===")
    print(f"  Target OS: {target_os}")
    print(f"  Version:   {args.version}")
    print(f"  Skip Build:{args.skip_build}")
    print(f"===============================================\n")

    if target_os == "windows":
        script = repo_root / "packaging" / "windows" / "package_release.ps1"
        cmd = [
            "powershell",
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-File", str(script),
            "-Version", args.version,
        ]
        if args.skip_build:
            cmd.append("-SkipBuild")
    elif target_os == "macos":
        script = repo_root / "packaging" / "macos" / "package_release.sh"
        cmd = ["bash", str(script), args.version]
        if args.skip_build:
            cmd.append("--skip-build")
    elif target_os == "linux":
        script = repo_root / "packaging" / "linux" / "package_release.sh"
        cmd = ["bash", str(script), args.version]
        if args.skip_build:
            cmd.append("--skip-build")
    else:
        print(f"Unsupported OS: {target_os}", file=sys.stderr)
        return 1

    return subprocess.run(cmd, cwd=str(repo_root)).returncode


if __name__ == "__main__":
    sys.exit(main())
