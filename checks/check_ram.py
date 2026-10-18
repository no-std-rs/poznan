"""Read compiler-folded sizes for the actual ESP32-C3 HTTPS/cache workflow."""
import json
import os
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parents[1]
env = os.environ.copy()
target_dir = root / "target" / "ram-proof"
env["CARGO_TARGET_DIR"] = str(target_dir)
target = "riscv32imc-unknown-none-elf"
subprocess.run([
    "cargo", "rustc", "--manifest-path", str(root / "checks/ram/Cargo.toml"),
    "--locked", "--offline", "--release", "--target", target, "--", "--emit=llvm-ir",
], check=True, env=env)
ir_file = max((target_dir / target / "release/deps").glob("poznan_ram_probe-*.ll"), key=lambda p: p.stat().st_mtime)
ir = ir_file.read_text()
def constant(name):
    match = re.search(r"^define[^\n]*" + name + r"[^\n]*\n(.*?)^}", ir, re.M | re.S)
    if not match:
        raise RuntimeError(f"missing {name} in {ir_file}")
    result = re.search(r"ret i32 (\d+)", match[1])
    if not result:
        raise RuntimeError(f"compiler did not fold {name}")
    return int(result[1])
workspace = constant("workspace_bytes")
task = constant("task_bytes")
reserves = constant("reserves_bytes")
total = workspace + task + reserves
print(json.dumps(dict(target=target, workspace_bytes=workspace, task_bytes=task,
    reserves_bytes=reserves, total_bytes=total, budget_bytes=100000,
    headroom_bytes=100000-total, fits=total <= 100000), indent=2))
if total > 100000:
    raise SystemExit("HTTPS/cache pipeline exceeds 100,000 bytes")
