#!/usr/bin/env python3
"""Exercise regeneration and drift detection without modifying the checkout."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory() as directory:
    tree = Path(directory)
    for source in ("scripts/generate-abi", "native/abi.toml"):
        (tree / source).parent.mkdir(exist_ok=True)
        shutil.copyfile(root / source, tree / source)
    generator = [sys.executable, str(tree / "scripts/generate-abi")]
    spec = tree / "native/abi.toml"
    # A new signature must reach the C declaration, Rust type and symbol lookup.
    spec.write_text(spec.read_text().replace(
        "[functions]", '[functions]\nprobe = "uint64_t(const char* name, size_t length)"'
    ))
    subprocess.run(generator, check=True)
    header = tree / "native/bug.h"
    rust = tree / "src/engine/abi.rs"
    assert "uint64_t bug_probe(const char* name, size_t length);" in header.read_text()
    assert 'probe: unsafe extern "C" fn(*const c_char, usize) -> u64' in rust.read_text()
    assert 'c"bug_probe"' in rust.read_text()
    subprocess.run(generator + ["--check"], check=True)

    header.write_text("stale")
    rust.unlink()
    check = subprocess.run(generator + ["--check"], capture_output=True, text=True)
    assert check.returncode == 1 and all(p.name in check.stderr for p in (header, rust))
    assert header.read_text() == "stale" and not rust.exists()
    subprocess.run(generator, check=True)
    subprocess.run(generator + ["--check"], check=True)
