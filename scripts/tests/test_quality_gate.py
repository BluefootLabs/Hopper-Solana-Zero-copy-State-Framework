from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("quality_gate", Path(__file__).resolve().parents[1] / "record-quality-gate.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class QualityGateTests(unittest.TestCase):
    def test_evidence_checks_new_untracked_bundles(self):
        evidence_spec = importlib.util.spec_from_file_location("evidence", Path(__file__).resolve().parents[1] / "verify-evidence.py")
        evidence = importlib.util.module_from_spec(evidence_spec)
        evidence_spec.loader.exec_module(evidence)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            bundle = root / "audit/new"
            bundle.mkdir(parents=True)
            (bundle / "result.log").write_bytes(b"passed")
            (bundle / "SHA256SUMS").write_text(f"{gate.digest(bundle / 'result.log')}  result.log\n", encoding="utf-8")
            with patch.object(evidence, "ROOT", root), patch.object(sys, "argv", ["verify-evidence.py"]):
                self.assertEqual(evidence.main(), 0)
                (bundle / "result.log").write_bytes(b"tampered")
                self.assertEqual(evidence.main(), 1)

    def test_pass_failure_and_source_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            (root / "lib.rs").write_text("// fixture\n", encoding="utf-8")
            (root / "README.md").write_text("included documentation\n", encoding="utf-8")
            for name, code, expected in [
                ("pass", "print('actual output')", 0),
                ("fail", "raise SystemExit(7)", 7),
                ("mutate", "from pathlib import Path; Path('added.rs').write_text('')", 125),
                ("edit-doc", "from pathlib import Path; Path('README.md').write_text('changed')", 125),
                ("add-doc", "from pathlib import Path; Path('GUIDE.md').write_text('new')", 125),
                ("remove-doc", "from pathlib import Path; Path('README.md').unlink()", 125),
            ]:
                out = root / "audit" / f"{name}.json"
                self.assertEqual(gate.record(root, out, [sys.executable, "-c", code]), expected)
                receipt = json.loads(out.read_text(encoding="utf-8"))
                self.assertEqual(receipt["exitCode"], expected)
                self.assertEqual(receipt["log"]["sha256"], gate.digest(root / "audit" / f"{name}.log"))
                self.assertIn("lib.rs", receipt["sourceFiles"])
                self.assertIn("README.md", receipt["sourceFiles"])
                with self.assertRaises(RuntimeError):
                    gate.record(root, out, [sys.executable, "-c", "pass"])

    def test_inventory_includes_new_sources_and_excludes_archives(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            (root / "Cargo.lock").write_text("lock", encoding="utf-8")
            (root / "new.rs").write_text("source", encoding="utf-8")
            (root / "README.md").write_text("documentation\n", encoding="utf-8")
            (root / "audit").mkdir()
            (root / "audit/old.rs").write_text("archive", encoding="utf-8")
            (root / "audit/README.md").write_text("archive documentation", encoding="utf-8")
            self.assertEqual(set(gate.sources(root)), {"Cargo.lock", "new.rs", "README.md"})
            (root / "new.rs").write_bytes(b"source\r\n")
            (root / "README.md").write_bytes(b"documentation\r\n")
            windows = gate.sources(root)
            (root / "new.rs").write_bytes(b"source\n")
            (root / "README.md").write_bytes(b"documentation\n")
            self.assertEqual(gate.sources(root), windows)


if __name__ == "__main__":
    unittest.main()
