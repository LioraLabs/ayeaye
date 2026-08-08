#!/usr/bin/env python3
"""Full transcript-message lookup behind clipped cards."""
import json
import os
import sys
import tempfile
import unittest
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

sys.dont_write_bytecode = True
ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
path = os.path.join(ROOT, "bin", "ayeaye")
spec = spec_from_file_location("ayeaye_transcript_message_test", path,
                              loader=SourceFileLoader("ayeaye_transcript_message_test", path))
ayeaye = module_from_spec(spec)
sys.modules[spec.name] = ayeaye
spec.loader.exec_module(ayeaye)


class FullMessage(unittest.TestCase):
    def test_current_codex_assistant_message_becomes_a_transcript_card(self):
        raw = json.dumps({"type": "response_item",
                          "timestamp": "2026-08-08T12:00:00Z",
                          "payload": {"type": "message", "role": "assistant",
                                      "phase": "final_answer", "content": [
                                          {"type": "output_text",
                                           "text": "**visible answer**"}]}})
        self.assertEqual([{"cls": "assistant", "ts": "12:00:00",
                           "label": "codex", "text": "**visible answer**"}],
                         ayeaye.rows_for(raw, "codex"))

    def test_stream_row_is_clipped_but_reference_resolves_original_markdown(self):
        text = "# Whole message\n\n" + ("long text " * 600)
        raw = json.dumps({"timestamp": "2026-08-08T12:00:00Z", "payload": {
            "type": "agent_message", "message": text}})
        with tempfile.NamedTemporaryFile("w", delete=False) as fh:
            fh.write(raw + "\n")
            path = fh.name
        self.addCleanup(os.unlink, path)
        session = {"kind": "codex", "path": path}

        row = ayeaye.transcript_rows(raw, "codex", 0)[0]
        self.assertEqual("0:0", row["ref"])
        self.assertLess(len(row["text"]), len(text))
        self.assertEqual(text, ayeaye.transcript_message(session, "0:0")["text"])

    def test_invalid_or_non_conversation_reference_is_not_returned(self):
        self.assertIsNone(ayeaye.transcript_message(
            {"kind": "codex", "path": "/does/not/matter"}, "../0"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
