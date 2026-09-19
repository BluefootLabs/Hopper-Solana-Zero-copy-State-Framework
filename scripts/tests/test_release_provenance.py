from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


def load_script(name: str):
    path = ROOT / "scripts" / name
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class SbfAttestationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.attest = load_script("attest-sbf-release.py")

    def test_build_command_owns_fresh_output_and_locks_dependencies(self) -> None:
        args = self.attest.build_command(
            "cargo-build-sbf",
            Path("program/Cargo.toml"),
            Path("target/release/sbf"),
            "v3",
        )
        self.assertEqual(
            args,
            [
                "cargo-build-sbf",
                "--manifest-path",
                str(Path("program/Cargo.toml")),
                "--sbf-out-dir",
                str(Path("target/release/sbf")),
                "--arch",
                "v3",
                "--",
                "--locked",
            ],
        )

    def test_command_evidence_binds_stdout_stderr_and_exit(self) -> None:
        first = self.attest.command_evidence(
            self.attest.CommandResult(["tool"], 0, "stdout", "stderr")
        )
        second = self.attest.command_evidence(
            self.attest.CommandResult(["tool"], 0, "stdout", "changed")
        )
        self.assertNotEqual(first["transcriptSha256"], second["transcriptSha256"])
        self.assertEqual(
            first["stdoutSha256"], hashlib.sha256(b"stdout").hexdigest()
        )

    def test_existing_output_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "existing"
            output.write_text("old", encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "already exists"):
                self.attest.require_absent(output, "attestation")


class UnsafeScannerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.scanner = load_script("check-unsafe-safety-comments.py")

    def test_detects_const_public_unsafe_fn_trait_and_impl(self) -> None:
        source = """
/// Constructor.
///
/// # Safety
/// The pointer must be valid.
pub const unsafe fn construct() {}

/// Marker.
///
/// # Safety
/// Implementers preserve the layout.
pub unsafe trait Marker {}

// SAFETY: The fixture satisfies Marker.
unsafe impl Marker for () {}
"""
        inventory, failures = self.scanner.scan_source(Path("fixture.rs"), source)
        self.assertEqual(inventory.public_unsafe_functions, 1)
        self.assertEqual(inventory.unsafe_traits, 1)
        self.assertEqual(inventory.unsafe_impls, 1)
        self.assertEqual(failures, [])

    def test_masks_comments_strings_and_requires_block_comment(self) -> None:
        source = 'const TEXT: &str = "unsafe { fake(); }"; // unsafe { fake(); }\nunsafe { real(); }\n'
        inventory, failures = self.scanner.scan_source(Path("fixture.rs"), source)
        self.assertEqual(inventory.unsafe_blocks, 1)
        self.assertEqual(len(failures), 1)


if __name__ == "__main__":
    unittest.main()
