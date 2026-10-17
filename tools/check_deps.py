#!/usr/bin/env python3
"""Check direct workspace and external dependency edges against the policy file."""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
POLICY = ROOT / "tools" / "allowed_deps.toml"
ENGINE_PREFIXES = ("engine_", "plugin_", "voxelworks-")
GAME_PACKAGES = {"studs", "studs-bake-sprites"}


def main() -> int:
    try:
        policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))["allowed"]
        metadata = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        packages = json.loads(metadata.stdout)["packages"]
    except (OSError, KeyError, tomllib.TOMLDecodeError, subprocess.CalledProcessError) as error:
        print(f"dependency check could not read workspace metadata: {error}", file=sys.stderr)
        return 2

    names = {package["name"] for package in packages}
    failed = False
    for package in packages:
        name = package["name"]
        rule = policy.get(name) or policy.get(f"legacy.{name}")
        # TOML nested tables are decoded separately; legacy entries live under
        # the legacy table and have the same workspace/external schema.
        if rule is None:
            rule = policy.get("legacy", {}).get(name)
        if rule is None:
            print(f"unlisted workspace package: {name}")
            failed = True
            continue

        allowed_workspace = set(rule.get("workspace", []))
        allowed_external = set(rule.get("external", []))
        if "*" in allowed_workspace:
            allowed_workspace = names
        for dependency in package["dependencies"]:
            dep_name = dependency.get("rename") or dependency["name"]
            # Metadata dependency names are Cargo package names except for a
            # renamed dependency, for which `rename` is the local crate alias.
            if dep_name in names:
                if name in GAME_PACKAGES:
                    continue
                if dep_name in GAME_PACKAGES:
                    print(f"forbidden engine/game dependency: {name} -> {dep_name}")
                    failed = True
                elif dep_name not in allowed_workspace:
                    print(f"offending workspace edge: {name} -> {dep_name}")
                    failed = True
            elif dep_name not in allowed_external and "*" not in allowed_external:
                print(f"offending external edge: {name} -> {dep_name}")
                failed = True
            if dep_name == "hecs" and name != "engine_ecs":
                print(f"only engine_ecs may depend on hecs: {name} -> hecs")
                failed = True

    if not failed:
        print(f"dependency policy passed for {len(packages)} workspace packages")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
