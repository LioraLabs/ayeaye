#!/usr/bin/env python3
"""Tracked-file reference resolution and its HTTP boundary."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
import urllib.parse
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_file_location

sys.dont_write_bytecode = True
REPO_ROOT = os.environ.get("REPO_ROOT") or os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ.setdefault("AYEAYE_TOKEN", "test-token")
spec = spec_from_file_location(
    "ayeaye_file_preview_test", os.path.join(REPO_ROOT, "bin", "ayeaye"),
    loader=SourceFileLoader("ayeaye_file_preview_test",
                            os.path.join(REPO_ROOT, "bin", "ayeaye")))
ayeaye = module_from_spec(spec)
sys.modules[spec.name] = ayeaye
spec.loader.exec_module(ayeaye)


class RepositoryTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="ayeaye-files-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        subprocess.run(("git", "init", "-q", self.tmp), check=True)
        self.cwd = os.path.join(self.tmp, "src", "deep")
        os.makedirs(self.cwd)
        self.original_panes, self.original_tmux = ayeaye.list_panes, ayeaye.tmux
        ayeaye.list_panes = lambda: [{"id": "%7"}]
        ayeaye.tmux = lambda *args: self.cwd if args[0] == "display-message" else ""
        self.addCleanup(self.restore)

    def restore(self):
        ayeaye.list_panes, ayeaye.tmux = self.original_panes, self.original_tmux

    def tracked(self, path, data=b"x"):
        full = os.path.join(self.tmp, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "wb") as fh:
            fh.write(data)
        subprocess.run(("git", "-C", self.tmp, "add", "--", path), check=True)

    def resolve(self, reference):
        return ayeaye.resolve_file_reference("%7", reference)


class ResolverMatches(RepositoryTest):
    def test_exact_suffix_and_basename_matches_are_ranked_in_that_order(self):
        for path in ("guide.md", "docs/guide.md", "src/deep/guide.md"):
            self.tracked(path)
        self.assertEqual(["guide.md", "src/deep/guide.md", "docs/guide.md"],
                         [c["path"] for c in self.resolve("guide.md")["candidates"]])
        self.assertEqual(["docs/guide.md", "src/deep/guide.md", "guide.md"],
                         [c["path"] for c in self.resolve("docs/guide.md")["candidates"]])

    def test_ambiguous_matches_use_proximity_then_length_then_lexical_order(self):
        for path in ("src/deep/a/readme.txt", "src/z/readme.txt",
                     "aa/readme.txt", "zz/readme.txt"):
            self.tracked(path)
        self.assertEqual(
            ["src/deep/a/readme.txt", "src/z/readme.txt",
             "aa/readme.txt", "zz/readme.txt"],
            [c["path"] for c in self.resolve("readme.txt")["candidates"]])

    def test_path_suffixes_rank_ahead_of_basename_only_matches(self):
        for path in ("archive/docs/guide.md", "src/deep/guide.md"):
            self.tracked(path)
        self.assertEqual(["archive/docs/guide.md", "src/deep/guide.md"],
                         [c["path"] for c in self.resolve("docs/guide.md")["candidates"]])

    def test_results_are_capped_at_twenty(self):
        for n in range(25):
            self.tracked("d%02d/same.txt" % n)
        self.assertEqual(20, len(self.resolve("same.txt")["candidates"]))

    def test_nul_delimited_inventory_preserves_unusual_names_and_excludes_untracked(self):
        unusual_names = ("odd name\nwith-tab\t.txt", "windows\\is-a-name.txt")
        for unusual in unusual_names:
            self.tracked(unusual)
        with open(os.path.join(self.tmp, "untracked.txt"), "w") as fh:
            fh.write("no")
        for unusual in unusual_names:
            self.assertEqual(unusual,
                             self.resolve(unusual)["candidates"][0]["path"])
        self.assertEqual([], self.resolve("untracked.txt")["candidates"])


class ResolverMetadata(RepositoryTest):
    def test_positive_line_suffix_is_returned_without_misreading_path_punctuation(self):
        self.tracked("docs/chapter:one.md")
        self.assertEqual(19, self.resolve("docs/chapter:one.md:19")["line"])
        self.assertEqual("docs/chapter:one.md",
                         self.resolve("docs/chapter:one.md:19")["candidates"][0]["path"])
        self.assertIsNone(self.resolve("docs/chapter:one.md:0")["line"])

    def test_types_and_sizes_are_classified_without_opening_bodies(self):
        files = (("a.py", "text"), ("a.png", "image"),
                 ("a.svg", "svg"), ("a.bin", "binary"))
        for path, _ in files:
            self.tracked(path, b"1234")
        for path, kind in files:
            candidate = self.resolve(path)["candidates"][0]
            self.assertEqual((kind, 4), (candidate["kind"], candidate["size"]))

    def test_missing_pane_and_non_repository_are_quiet(self):
        ayeaye.list_panes = lambda: []
        self.assertEqual({"candidates": [], "line": None}, self.resolve("a.py"))
        ayeaye.list_panes = lambda: [{"id": "%7"}]
        ayeaye.tmux = lambda *args: tempfile.gettempdir()
        self.assertEqual({"candidates": [], "line": None}, self.resolve("a.py"))


class ResolverEndpoint(RepositoryTest):
    def setUp(self):
        super().setUp()
        self.tracked("docs/guide.md")
    def post(self, body, authed=True, origin_ok=True):
        class Request:
            path = "/api/files/resolve"
            headers = {"Content-Length": str(len(json.dumps(body)))}
            close_connection = False
            _auth = "-"

            def _host_ok(self): return True
            def _origin_ok(self): return origin_ok
            def _authed(self): return authed
            def _body(self): return json.dumps(body).encode()
            def _json(self, value, code=200): self.answer = (code, value)
            def _forbidden(self): self._json({"error": "forbidden"}, 403)
            def _unauthorized(self): self._json({"error": "unauthorized"}, 401)

        request = Request()
        ayeaye.Handler.do_POST(request)
        return request.answer

    def test_markdown_and_backtick_references_are_normalized_at_the_api(self):
        for reference in ("`docs/guide.md:7`", "[guide](docs/guide.md:7)"):
            status, body = self.post({"pane": "%7", "reference": reference})
            self.assertEqual(200, status)
            self.assertEqual(("docs/guide.md", 7),
                             (body["candidates"][0]["path"], body["line"]))

    def test_endpoint_requires_auth_and_same_origin(self):
        self.assertEqual(401, self.post({"pane": "%7", "reference": "guide.md"},
                                        authed=False)[0])
        self.assertEqual(403, self.post({"pane": "%7", "reference": "guide.md"},
                                        origin_ok=False)[0])

    def test_endpoint_rejects_bad_json_shapes(self):
        status, body = self.post({"pane": "%7"})
        self.assertEqual(400, status)
        self.assertIn("error", body)
        for payload in ([], "reference", 7, None):
            status, body = self.post(payload)
            self.assertEqual(400, status)
            self.assertIn("error", body)


class PreviewEndpoint(RepositoryTest):
    def get(self, path, line=None, authed=True):
        query = {"pane": "%7", "path": path}
        if line is not None:
            query["line"] = str(line)

        class Request:
            headers = {}
            close_connection = False
            _auth = "-"

            def _host_ok(self): return True
            def _authed(self): return authed
            def _query(self): return {key: [value] for key, value in query.items()}
            def _send(self, code, body, ctype, headers=None):
                if isinstance(body, str):
                    body = body.encode()
                self.answer = (code, body, ctype, headers or {})
            def _json(self, value, code=200):
                self._send(code, json.dumps(value), "application/json")
            def _forbidden(self): self._json({"error": "forbidden"}, 403)
            def _unauthorized(self): self._json({"error": "unauthorized"}, 401)

        request = Request()
        request.path = "/api/files/preview?" + urllib.parse.urlencode(query)
        ayeaye.Handler.do_GET(request)
        code, body, ctype, headers = request.answer
        decoded = json.loads(body) if ctype == "application/json" else body
        return code, decoded, ctype, headers

    def test_revalidates_tracking_existence_and_regular_file_type(self):
        self.tracked("safe.txt", b"safe")
        self.assertEqual(200, self.get("safe.txt")[0])

        subprocess.run(("git", "-C", self.tmp, "rm", "--cached", "safe.txt"),
                       check=True, capture_output=True)
        self.assertEqual(404, self.get("safe.txt")[0])
        subprocess.run(("git", "-C", self.tmp, "add", "safe.txt"), check=True)

        with open(os.path.join(self.tmp, "untracked.txt"), "wb") as fh:
            fh.write(b"secret")
        self.assertEqual(404, self.get("untracked.txt")[0])

        os.unlink(os.path.join(self.tmp, "safe.txt"))
        self.assertEqual(404, self.get("safe.txt")[0])
        os.mkdir(os.path.join(self.tmp, "safe.txt"))
        self.assertEqual(404, self.get("safe.txt")[0])

    def test_rejects_absolute_traversal_and_every_symlink(self):
        self.tracked("plain.txt", b"plain")
        for path in ("/etc/passwd", "../plain.txt", "src/../../plain.txt"):
            self.assertEqual(400, self.get(path)[0], path)

        outside = os.path.join(os.path.dirname(self.tmp), "outside.txt")
        with open(outside, "wb") as fh:
            fh.write(b"outside")
        self.addCleanup(lambda: os.path.exists(outside) and os.unlink(outside))
        os.symlink(outside, os.path.join(self.tmp, "outside-link.txt"))
        subprocess.run(("git", "-C", self.tmp, "add", "outside-link.txt"), check=True)
        self.assertEqual(404, self.get("outside-link.txt")[0])

        with open(os.path.join(self.tmp, "inside-untracked.txt"), "wb") as fh:
            fh.write(b"inside secret")
        os.symlink("inside-untracked.txt", os.path.join(self.tmp, "inside-link.txt"))
        subprocess.run(("git", "-C", self.tmp, "add", "inside-link.txt"), check=True)
        self.assertEqual(404, self.get("inside-link.txt")[0])

    def test_valid_tracked_posix_backslash_name_is_previewable(self):
        self.tracked("odd\\name.txt", b"backslash name\n")
        code, body, _, _ = self.get("odd\\name.txt")
        self.assertEqual(200, code)
        self.assertEqual("odd\\name.txt", body["path"])

    def test_text_is_numbered_centered_and_clamped(self):
        data = "".join("line %03d\n" % n for n in range(1, 401)).encode()
        self.tracked("many.txt", data)
        code, body, _, _ = self.get("many.txt", 250)
        self.assertEqual(200, code)
        self.assertEqual(("many.txt", len(data), "text", 250),
                         (body["path"], body["size"], body["kind"], body["line"]))
        self.assertLessEqual(len(body["lines"]), 200)
        numbers = [row["number"] for row in body["lines"]]
        self.assertIn(250, numbers)
        self.assertGreater(numbers[0], 1)
        self.assertEqual(numbers[0], body["start"])

        first = self.get("many.txt")[1]
        self.assertNotIn("line", first)
        self.assertEqual(1, first["start"])
        self.assertEqual(1, first["lines"][0]["number"])
        end = self.get("many.txt", 9999)[1]
        self.assertEqual(400, end["lines"][-1]["number"])

    def test_text_bytes_are_bounded_and_invalid_utf8_decodes_safely(self):
        self.tracked("huge.txt", b"a" * (300 * 1024) + b"\nlast\n")
        body = self.get("huge.txt")[1]
        encoded_text = "".join(row["text"] for row in body["lines"]).encode("utf-8")
        self.assertLessEqual(len(encoded_text), 256 * 1024)

        self.tracked("invalid.txt", b"hello\xffworld\n")
        invalid = self.get("invalid.txt")[1]
        self.assertIn("\ufffd", invalid["lines"][0]["text"])

    def test_text_io_is_bounded_before_a_whole_file_is_materialized(self):
        self.tracked("enormous.txt", b"x" * (2 * 1024 * 1024))
        original_fdopen = ayeaye.os.fdopen
        observed = {"bytes": 0}

        class BoundedReader:
            def __init__(self, wrapped): self.wrapped = wrapped
            def __enter__(self): return self
            def __exit__(self, *args): self.wrapped.close()
            def read(self, *args):
                raise AssertionError("preview must not read the whole text file")
            def readline(self, size=-1):
                self.assert_bounded(size)
                data = self.wrapped.readline(size)
                observed["bytes"] += len(data)
                return data
            @staticmethod
            def assert_bounded(size):
                if size < 0:
                    raise AssertionError("text reads must carry a byte bound")

        def tracked_fdopen(*args, **kwargs):
            return BoundedReader(original_fdopen(*args, **kwargs))

        ayeaye.os.fdopen = tracked_fdopen
        self.addCleanup(setattr, ayeaye.os, "fdopen", original_fdopen)
        code, body, _, _ = self.get("enormous.txt")
        self.assertEqual(200, code)
        self.assertLessEqual(observed["bytes"], 256 * 1024 + 1)
        self.assertLessEqual(
            len("".join(row["text"] for row in body["lines"]).encode()),
            256 * 1024)

    def test_far_line_scan_has_an_aggregate_io_cap_and_falls_back_to_start(self):
        self.tracked("millions.txt", b"x\n" * 3_000_000)
        original_fdopen = ayeaye.os.fdopen
        observed = {"bytes": 0}

        class CountingReader:
            def __init__(self, wrapped): self.wrapped = wrapped
            def __enter__(self): return self
            def __exit__(self, *args): self.wrapped.close()
            def readline(self, size=-1):
                if size < 0:
                    raise AssertionError("text reads must carry a byte bound")
                data = self.wrapped.readline(size)
                observed["bytes"] += len(data)
                return data

        ayeaye.os.fdopen = lambda *args, **kwargs: CountingReader(
            original_fdopen(*args, **kwargs))
        self.addCleanup(setattr, ayeaye.os, "fdopen", original_fdopen)
        code, body, _, _ = self.get("millions.txt", 2_000_000)
        self.assertEqual(200, code)
        self.assertLessEqual(observed["bytes"], 1024 * 1024)
        self.assertEqual(1, body["start"])
        self.assertEqual(1, body["lines"][0]["number"])
        self.assertNotIn(2_000_000,
                         [row["number"] for row in body["lines"]])

    def test_html_remains_plain_json_data_and_binary_has_no_body_preview(self):
        self.tracked("page.html", b"<script>alert(1)</script>\n")
        body = self.get("page.html")[1]
        self.assertEqual("<script>alert(1)</script>\n", body["lines"][0]["text"])

        self.tracked("archive.bin", b"\x00\x01\x02")
        binary = self.get("archive.bin")[1]
        self.assertEqual({"path": "archive.bin", "size": 3, "kind": "binary",
                          "preview": "unavailable"}, binary)

    def test_supported_images_use_exact_allowlisted_types_and_safe_headers(self):
        expected = {"a.png": "image/png", "a.jpg": "image/jpeg",
                    "a.jpeg": "image/jpeg", "a.gif": "image/gif",
                    "a.webp": "image/webp", "a.svg": "image/svg+xml"}
        for path, mime in expected.items():
            self.tracked(path, ("bytes-" + path).encode())
            code, body, ctype, headers = self.get(path)
            self.assertEqual((200, ("bytes-" + path).encode(), mime),
                             (code, body, ctype))
            self.assertEqual("inline", headers.get("Content-Disposition"))
            self.assertEqual("nosniff", headers.get("X-Content-Type-Options"))
            if path.endswith(".svg"):
                self.assertIn("sandbox", headers.get("Content-Security-Policy", ""))

    def test_images_over_eight_megabytes_are_refused(self):
        self.tracked("large.png", b"x" * (8 * 1024 * 1024 + 1))
        self.assertEqual(413, self.get("large.png")[0])

    def test_preview_requires_auth_and_positive_line(self):
        self.tracked("a.txt", b"a\n")
        self.assertEqual(401, self.get("a.txt", authed=False)[0])
        for line in (0, -1, "wat"):
            self.assertEqual(400, self.get("a.txt", line)[0])


if __name__ == "__main__":
    unittest.main(verbosity=2)
