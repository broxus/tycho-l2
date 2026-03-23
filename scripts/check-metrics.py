#!/usr/bin/env python3
import os
import re
import subprocess
import sys
from pathlib import Path

def find_metrics(root_dir: Path, blacklisted_paths: list[str]) -> set[str]:
    metric_names: set[str] = set()
    constants: dict[str, str] = {}

    for root, _, files in os.walk(root_dir):
        for file_name in files:
            if not file_name.endswith(".rs"):
                continue
            file_path = Path(root) / file_name
            process_file(file_path, metric_names, constants, blacklisted_paths)

    return metric_names


def process_file(
    file_path: Path,
    metric_names: set[str],
    constants: dict[str, str],
    blacklisted_paths: list[str],
) -> None:
    content = file_path.read_text()

    const_pattern = r'const\s+([A-Z_]+)\s*:\s*&str\s*=\s*"([^"]+)";'
    for match in re.finditer(const_pattern, content):
        constants[match.group(1)] = match.group(2)

    patterns = [
        r"metrics::gauge!\s*\(\s*([^)]+)\s*\)",
        r"metrics::histogram!\s*\(\s*([^)]+)\s*\)",
        r"metrics::counter!\s*\(\s*([^)]+)\s*\)",
        r"HistogramGuard::begin\s*\(\s*([^)]+)\s*\)",
        r'HistogramGuard::begin_with_labels\(\s*(?:([A-Z_]+)|("(?:[^"\\]|\\.)*"))',
    ]
    patterns = [re.compile(pattern, re.MULTILINE) for pattern in patterns]

    lines = content.splitlines()
    for pattern in patterns:
        for match in re.finditer(pattern, content):
            arg = next(group for group in match.groups() if group is not None).strip()
            line_number = content[: match.start()].count("\n") + 1
            line = lines[line_number - 1].strip()

            if not line.startswith("//"):
                process_metric_arg(
                    arg,
                    metric_names,
                    constants,
                    file_path,
                    blacklisted_paths,
                )


def process_metric_arg(
    arg: str,
    metric_names: set[str],
    constants: dict[str, str],
    file_path: Path,
    blacklisted_paths: list[str],
) -> None:
    arg = arg.split(",")[0].strip()
    if "$" in arg:
        return
    if arg.startswith('"') and arg.endswith('"'):
        metric_names.add(arg[1:-1])
        return
    if arg in constants:
        metric_names.add(constants[arg])
        return
    if "::" in arg:
        constant_name = arg.split("::")[-1]
        constant_value = find_constant_in_imports(file_path, constant_name)
        if constant_value:
            metric_names.add(constant_value)
        elif not any(path in str(file_path) for path in blacklisted_paths):
            print(f"Warning: Unresolved metric name '{arg}' in {file_path}")
        return

    constant_value = find_constant_in_imports(file_path, arg)
    if constant_value:
        metric_names.add(constant_value)
    elif not any(path in str(file_path) for path in blacklisted_paths):
        print(f"Warning: Unresolved metric name '{arg}' in {file_path}")


def find_constant_in_imports(file_path: Path, constant_name: str) -> str:
    content = file_path.read_text()

    use_pattern = r"use\s+([^;]+);"
    for match in re.finditer(use_pattern, content):
        module_path = match.group(1)
        if "::" not in module_path:
            continue
        module_parts = module_path.split("::")
        potential_file = file_path.parent.joinpath(
            *module_parts[:-1], f"{module_parts[-1]}.rs"
        )
        if not potential_file.exists():
            continue

        imported_content = potential_file.read_text()
        const_pattern = rf'const\s+{constant_name}\s*:\s*&str\s*=\s*"([^"]+)";'
        const_match = re.search(const_pattern, imported_content)
        if const_match:
            return const_match.group(1)

    return ""


def root_directory() -> Path:
    root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
    ).stdout.strip()
    if root:
        return Path(root)
    root = subprocess.run(
        ["jj", "root"],
        capture_output=True,
        text=True,
    ).stdout.strip()
    if root:
        return Path(root)
    return Path(__file__).resolve().parent.parent


def render_dashboard(root_dir: Path) -> str:
    res = subprocess.run(
        [sys.executable, str(root_dir / "scripts" / "gen-dashboard.py")],
        text=True,
        capture_output=True,
        timeout=30,
        check=True,
    )
    return res.stdout


def main() -> int:
    root_dir = root_directory()
    blacklisted_paths: list[str] = []
    metrics = find_metrics(root_dir, blacklisted_paths)

    ignore_missing = """
    """
    ignore_missing = {
        metric.strip() for metric in ignore_missing.splitlines() if metric.strip()
    }

    try:
        dashboard_data = render_dashboard(root_dir)
    except subprocess.TimeoutExpired:
        print("failed to build dashboard: timeout")
        return 2
    except subprocess.CalledProcessError as err:
        print("failed to build dashboard")
        if err.stdout:
            print(err.stdout)
        if err.stderr:
            print(err.stderr)
        return 2

    exit_code = 0
    for metric in sorted(metrics):
        if metric in ignore_missing:
            continue
        if metric not in dashboard_data:
            exit_code = 1
            print(f"Missing metric: {metric}")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
