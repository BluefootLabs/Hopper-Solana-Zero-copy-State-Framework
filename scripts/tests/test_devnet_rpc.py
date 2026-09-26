"""Transient read failures must not cause transaction resubmission."""
import io
from pathlib import Path
import runpy
import unittest
from unittest.mock import patch
from urllib.error import HTTPError


API = runpy.run_path(str(Path(__file__).resolve().parents[1] / "test-runtime-gate-devnet.py"))


class RpcRetries(unittest.TestCase):
    def test_read_retries_then_returns_result(self):
        failure = HTTPError(API["RPC"], 429, "rate limit", {}, None)
        with patch("urllib.request.urlopen", side_effect=[failure, io.BytesIO(b'{"result": 42}')]) as call, patch("time.sleep") as sleep:
            self.assertEqual(API["rpc"]("getBalance", ["address"]), 42)
            self.assertEqual(call.call_count, 2)
            sleep.assert_called_once_with(2)

    def test_retries_are_bounded(self):
        failure = HTTPError(API["RPC"], 503, "unavailable", {}, None)
        with patch("urllib.request.urlopen", side_effect=failure) as call, patch("time.sleep") as sleep:
            with self.assertRaises(HTTPError):
                API["rpc"]("getTransaction", ["signature"])
            self.assertEqual(call.call_count, 5)
            self.assertEqual([c.args[0] for c in sleep.call_args_list], [2, 4, 8, 16])

    def test_submission_unknown_and_permanent_errors_are_not_retried(self):
        for method, code in [("sendTransaction", 429), ("unknownMethod", 503), ("getBalance", 403)]:
            with self.subTest(method=method, code=code):
                failure = HTTPError(API["RPC"], code, "failure", {}, None)
                with patch("urllib.request.urlopen", side_effect=failure) as call, patch("time.sleep") as sleep:
                    with self.assertRaises(HTTPError):
                        API["rpc"](method, [])
                    call.assert_called_once()
                    sleep.assert_not_called()

    def test_rpc_error_is_not_retried(self):
        with patch("urllib.request.urlopen", return_value=io.BytesIO(b'{"error": {"code": -32602}}')) as call, patch("time.sleep") as sleep:
            with self.assertRaises(RuntimeError):
                API["rpc"]("getBalance", [])
            call.assert_called_once()
            sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
