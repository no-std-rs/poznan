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
        # LLVM can merge equal size functions into aliases. Follow the emitted
        # symbol rather than duplicating or hardcoding a measured size.
        alias = re.search(r"^@[^ \n]*" + re.escape(name)
            + r"[^ \n]* = [^\n]*\balias\b[^\n]*ptr @([^,\s]+)", ir, re.M)
        if alias:
            return constant(alias[1])
        raise RuntimeError(f"missing {name} in {ir_file}")
    result = re.search(r"ret i32 (\d+)", match[1])
    if not result:
        raise RuntimeError(f"compiler did not fold {name}")
    return int(result[1])
reserves = constant("reserves_bytes")
profiles = {}
for name, workspace_fn, task_fn in [
    ("waste", "workspace_bytes", "task_bytes"),
    ("mobility", "mobility_storage_bytes", "mobility_future_bytes"),
]:
    workspace = constant(workspace_fn)
    task = constant(task_fn)
    total = workspace + task + reserves
    profiles[name] = dict(workspace_bytes=workspace, task_bytes=task,
        reserves_bytes=reserves, total_bytes=total,
        headroom_bytes=100000-total, fits=total <= 100000)
print(json.dumps(dict(target=target, budget_bytes=100000, profiles=profiles), indent=2))
if any(not profile["fits"] for profile in profiles.values()):
    raise SystemExit("HTTPS pipeline exceeds 100,000 bytes")
