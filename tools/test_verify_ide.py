"""Regression checks for bounded retries in the stock rust-analyzer verifier."""

from collections import deque
import importlib.util
from pathlib import Path
import queue
import unittest
from unittest.mock import Mock, patch


spec = importlib.util.spec_from_file_location(
    "verify_ide", Path(__file__).with_name("verify-ide.py")
)
verify_ide = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify_ide)


class Clock:
    def __init__(self):
        self.now = 0.0
        self.delays = []

    def monotonic(self):
        return self.now

    def sleep(self, duration):
        self.delays.append(duration)
        self.now += duration


class Incoming:
    """Use the real message handler without starting a process or waiting."""

    def __init__(self, clock):
        self.clock = clock
        self.messages = deque()
        self.received_at = []

    def get(self, timeout):
        self.received_at.append(self.clock.now)
        if self.messages:
            return self.messages.popleft()
        self.clock.now += timeout
        raise queue.Empty


class RequestRetryTests(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.time_patch = patch.object(verify_ide, "time", self.clock)
        self.time_patch.start()
        self.addCleanup(self.time_patch.stop)
        self.session = verify_ide.Lsp.__new__(verify_ide.Lsp)
        self.session.next_id = 0
        self.session.queue = Incoming(self.clock)
        self.session.messages = []
        self.session.diagnostics = {}
        self.session.sequence = 0
        self.session.process = Mock()
        self.session.process.poll.return_value = None
        self.sent = []

    def respond(self, responder):
        def send(request):
            self.sent.append((self.clock.now, request))
            self.session.queue.messages.extend(responder(request, len(self.sent)))

        self.session.send = send

    @staticmethod
    def cancelled(request, code=-32802, data=None):
        error = {"code": code, "message": "server cancelled the request"}
        if data is not None:
            error["data"] = data
        return {"jsonrpc": "2.0", "id": request["id"], "error": error}

    def test_diagnostic_cancellation_retries_same_request_with_a_new_id(self):
        result = {"kind": "full", "items": []}
        uri = "file:///fixture/main.rs"
        notification = {
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri, "diagnostics": []},
        }

        def responses(request, attempt):
            if attempt == 1:
                return [
                    self.cancelled(request, data={"retriggerRequest": True}),
                    notification,
                ]
            return [{"jsonrpc": "2.0", "id": request["id"], "result": result}]

        self.respond(responses)
        self.assertEqual(self.session.pull_diagnostics(uri), result)
        self.assertEqual([request["id"] for _, request in self.sent], [1, 2])
        self.assertEqual(self.sent[0][1]["method"], self.sent[1][1]["method"])
        self.assertEqual(self.sent[0][1]["params"], self.sent[1][1]["params"])
        self.assertEqual(self.session.diagnostics[uri][1], notification["params"])
        # The notification remains queued during the delay, then the normal
        # receive loop processes it before the successful response.
        self.assertEqual(self.session.queue.received_at, [0.0, 0.05, 0.05])

    def test_diagnostic_cancellation_without_explicit_true_does_not_retry(self):
        for data in [None, {}, {"retriggerRequest": False}, {"retriggerRequest": 1},
                     {"retriggerRequest": "true"}, "invalid"]:
            with self.subTest(data=data):
                self.sent.clear()
                self.respond(lambda request, _: [self.cancelled(request, data=data)])
                with self.assertRaisesRegex(AssertionError, "-32802"):
                    self.session.pull_diagnostics("file:///fixture/main.rs")
                self.assertEqual(len(self.sent), 1)
                self.assertEqual(self.clock.delays, [])

    def test_server_cancelled_on_other_methods_does_not_gain_diagnostic_retry(self):
        self.respond(lambda request, _: [
            self.cancelled(request, data={"retriggerRequest": True})
        ])
        with self.assertRaisesRegex(AssertionError, "textDocument/hover"):
            self.session.request("textDocument/hover")
        self.assertEqual(len(self.sent), 1)

    def test_other_errors_still_fail_even_with_retrigger_flag(self):
        self.respond(lambda request, _: [
            self.cancelled(request, code=-32603, data={"retriggerRequest": True})
        ])
        with self.assertRaisesRegex(AssertionError, "-32603"):
            self.session.pull_diagnostics("file:///fixture/main.rs")
        self.assertEqual(len(self.sent), 1)

    def test_existing_request_cancelled_and_content_modified_retries_are_preserved(self):
        def responses(request, attempt):
            if attempt <= 2:
                return [self.cancelled(request, code=[-32800, -32801][attempt - 1])]
            return [{"jsonrpc": "2.0", "id": request["id"], "result": "hover result"}]

        self.respond(responses)
        self.assertEqual(self.session.request("textDocument/hover"), "hover result")
        self.assertEqual([request["id"] for _, request in self.sent], [1, 2, 3])

    def test_repeated_cancellation_exhausts_original_deadline_including_delay(self):
        self.respond(lambda request, _: [
            self.cancelled(request, data={"retriggerRequest": True})
        ])
        with self.assertRaisesRegex(TimeoutError, "textDocument/diagnostic"):
            self.session.request("textDocument/diagnostic", timeout=0.12)
        self.assertAlmostEqual(self.clock.now, 0.12)
        self.assertEqual(len(self.sent), 3)
        self.assertTrue(all(sent_at < 0.12 for sent_at, _ in self.sent))
        self.assertAlmostEqual(sum(self.clock.delays), 0.12)

    def test_cancelled_response_at_deadline_does_not_send_another_request(self):
        self.respond(lambda request, _: [
            self.cancelled(request, data={"retriggerRequest": True})
        ])
        # Delivering the response uses the remaining budget before processing it.
        original_get = self.session.queue.get

        def receive(timeout):
            self.clock.now += timeout
            return original_get(timeout)

        self.session.queue.get = receive
        with self.assertRaises(TimeoutError):
            self.session.request("textDocument/diagnostic", timeout=0.1)
        self.assertEqual(len(self.sent), 1)
        self.assertEqual(self.clock.delays, [])


if __name__ == "__main__":
    unittest.main()
