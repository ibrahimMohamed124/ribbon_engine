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
RETIRED_PREFIX = "voxelworks-"


def package_layer(manifest_path: str) -> str | None:
    """Classify a workspace package by its repository ownership boundary."""
    try:
        relative = Path(manifest_path).resolve().relative_to(ROOT)
    except ValueError:
        return None
    if not relative.parts:
        return None
    first = relative.parts[0]
    if first in {"crates", "plugins", "examples", "games"}:
        return first
    return None


def package_owner(manifest_path: str) -> str | None:
    """Return the game owner for packages under games/<game>/... ."""
    try:
        relative = Path(manifest_path).resolve().relative_to(ROOT)
    except ValueError:
        return None
    if len(relative.parts) >= 2 and relative.parts[0] == "games":
        return relative.parts[1]
    return None


def layer_edge_error(
    source_layer: str,
    dependency_layer: str,
    source_owner: str | None = None,
    dependency_owner: str | None = None,
) -> str | None:
    """Return a reason when a workspace dependency points upward or sideways illegally."""
    if dependency_layer == "games":
        same_game = (
            source_layer == "games"
            and source_owner is not None
            and source_owner == dependency_owner
        )
        if same_game:
            return None
        return "engine/example packages and other games may not depend on game-owned packages"
    if source_layer == "crates" and dependency_layer != "crates":
        return "engine crates may depend only on other engine crates"
    if source_layer == "plugins" and dependency_layer not in {"crates", "plugins"}:
        return "plugins may depend only on engine crates or other plugins"
    if source_layer == "examples" and dependency_layer not in {"crates", "plugins"}:
        return "examples may depend only on engine crates or reusable plugins"
    if source_layer == "games" and dependency_layer not in {"crates", "plugins", "games"}:
        return "game packages may depend only on engine APIs and packages owned by the same game"
    return None


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
    layers = {package["name"]: package_layer(package["manifest_path"]) for package in packages}
    owners = {package["name"]: package_owner(package["manifest_path"]) for package in packages}
    failed = False
    for package in packages:
        name = package["name"]
        source_layer = layers.get(name)
        if source_layer is None:
            print(
                "workspace package is outside the supported repository layers: "
                f"{name} ({package['manifest_path']})"
            )
            failed = True
        if name.startswith(RETIRED_PREFIX):
            print(f"retired compatibility package is still in the workspace: {name}")
            failed = True
            continue
        rule = policy.get(name)
        if rule is None:
            print(f"unlisted workspace package: {name}")
            failed = True
            continue

        allowed_workspace = set(rule.get("workspace", []))
        allowed_external = set(rule.get("external", []))
        if "*" in allowed_workspace or "*" in allowed_external:
            print(f"wildcard allowances are not permitted: {name}")
            failed = True
        for dependency in package["dependencies"]:
            package_name = dependency["name"]
            dep_name = dependency.get("rename") or package_name
            if package_name in names:
                dependency_layer = layers.get(package_name)
                if source_layer is not None and dependency_layer is not None:
                    reason = layer_edge_error(
                        source_layer, dependency_layer, owners.get(name), owners.get(package_name)
                    )
                    if reason is not None:
                        print(f"forbidden layer edge: {name} -> {package_name}: {reason}")
                        failed = True
                if dep_name not in allowed_workspace:
                    print(f"offending workspace edge: {name} -> {dep_name}")
                    failed = True
            elif dep_name not in allowed_external:
                print(f"offending external edge: {name} -> {dep_name}")
                failed = True
            if dep_name == "hecs" and name != "ribbon_ecs":
                print(f"only ribbon_ecs may depend on hecs: {name} -> hecs")
                failed = True

    if not failed:
        print(f"dependency policy passed for {len(packages)} workspace packages")
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
