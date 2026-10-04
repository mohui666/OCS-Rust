import json
from pathlib import Path
import threading
import unittest
from concurrent.futures import Future, ThreadPoolExecutor
from http.server import ThreadingHTTPServer
from unittest.mock import Mock, patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from bridge import Bridge, BridgeError, codex_answer, format_answer, make_handler, normalize_question, question_key


def result(*answers, confident=True):
    return {"answers": list(answers), "confident": confident, "explanation": "测试解释", "sources": []}


def batch_result(questions, answers=None):
    return {"results": [dict(result(*(answers[i] if answers else ["3"])), id=question_key(q))
                        for i, q in enumerate(questions)], "web_search_calls": 0}


class BatchTests(unittest.TestCase):
    def setUp(self):
        self.questions = [{"title": "题目 " + str(i), "options": "", "type": "completion"} for i in range(4)]
        self.calls = []
        def runner(questions, config):
            self.calls.append(questions)
            return batch_result(questions, [[q["title"]] for q in questions])
        self.bridge = Bridge({"port": 0, "token": "test", "model": "test"}, runner)

    def test_one_page_one_call_and_subsequent_cache_hits(self):
        for q in self.questions:
            value = self.bridge.answer(dict(q, pageQuestions=self.questions))
            self.assertEqual(value["answer"], q["title"])
            self.assertEqual(value["batch_size"], 4)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.bridge.cache_hits, 3)

    def test_85_question_exam_is_one_call_and_all_ids_are_cached(self):
        questions = [{"title": "整卷题目 " + str(i), "options": "", "type": "completion"}
                     for i in range(85)]
        for q in questions:
            value = self.bridge.answer(dict(q, pageQuestions=questions))
            self.assertEqual(value["answer"], q["title"])
            self.assertEqual(value["batch_size"], 85)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.bridge.cache_hits, 84)

    def test_200_question_batch_preserves_last_question(self):
        questions = [{"title": "最大批次 " + str(i), "options": "", "type": "completion"}
                     for i in range(200)]
        self.assertEqual(self.bridge.answer(dict(questions[0], pageQuestions=questions))["batch_size"], 200)
        self.assertEqual(self.bridge.answer(questions[-1])["answer"], questions[-1]["title"])
        self.assertEqual(len(self.calls), 1)

    def test_out_of_order_ids_map_to_their_questions(self):
        def runner(questions, config):
            out = batch_result(questions, [[q["title"]] for q in questions])
            out["results"].reverse()
            return out
        self.bridge.runner = runner
        self.assertEqual(self.bridge.answer(dict(self.questions[0], pageQuestions=self.questions))["answer"], "题目 0")
        self.assertEqual(self.bridge.answer(self.questions[3])["answer"], "题目 3")

    def test_missing_or_duplicate_id_does_not_populate_cache(self):
        for mode in ("missing", "duplicate", "foreign"):
            def runner(questions, config):
                out = batch_result(questions)
                if mode == "missing": out["results"].pop()
                if mode == "duplicate": out["results"][1]["id"] = out["results"][0]["id"]
                if mode == "foreign": out["results"][1]["id"] = "wrong-id"
                return out
            self.bridge.runner = runner
            with self.assertRaises(BridgeError):
                self.bridge.answer(dict(self.questions[0], pageQuestions=self.questions))
            self.assertFalse(self.bridge.cache)
            self.assertFalse(self.bridge.inflight)

    def test_parallel_request_joins_existing_batch(self):
        entered, release = threading.Event(), threading.Event()
        def runner(questions, config):
            self.calls.append(questions)
            entered.set()
            self.assertTrue(release.wait(5))
            return batch_result(questions)
        self.bridge.runner = runner
        with ThreadPoolExecutor(max_workers=2) as pool:
            first = pool.submit(self.bridge.answer, dict(self.questions[0], pageQuestions=self.questions))
            self.assertTrue(entered.wait(3))
            second = pool.submit(self.bridge.answer, dict(self.questions[1], pageQuestions=self.questions))
            release.set()
            self.assertEqual(first.result(5)["code"], 1)
            self.assertEqual(second.result(5)["code"], 1)
        self.assertEqual(len(self.calls), 1)

    def test_unlimited_same_page_wait_has_no_deadline(self):
        entered, waiting, release = threading.Event(), threading.Event(), threading.Event()
        timeouts = []
        class TrackedFuture(Future):
            def result(self, timeout=None):
                timeouts.append(timeout)
                waiting.set()
                return super().result(timeout)
        def runner(questions, config):
            self.calls.append(questions)
            entered.set()
            if not release.wait(3):
                raise RuntimeError("test runner was not released")
            return batch_result(questions)
        self.bridge.config["timeout_seconds"] = 0
        self.bridge.runner = runner
        with patch("bridge.Future", TrackedFuture), ThreadPoolExecutor(max_workers=2) as pool:
            first = pool.submit(self.bridge.answer, dict(self.questions[0], pageQuestions=self.questions))
            try:
                self.assertTrue(entered.wait(2))
                second = pool.submit(self.bridge.answer, dict(self.questions[1], pageQuestions=self.questions))
                self.assertTrue(waiting.wait(2))
                self.assertEqual(timeouts, [None])
            finally:
                release.set()
            self.assertEqual(first.result(3)["code"], 1)
            self.assertEqual(second.result(3)["code"], 1)
        self.assertEqual(len(self.calls), 1)

    def test_cached_page_still_distinguishes_changed_options(self):
        self.bridge.answer(dict(self.questions[0], pageQuestions=self.questions))
        changed = dict(self.questions[1], options="different context")
        self.bridge.answer(changed)
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(len(self.calls[1]), 1)

    def test_title_only_manual_search_enriches_unique_question(self):
        self.bridge.answer({"title": self.questions[0]["title"], "pageQuestions": self.questions})
        self.assertEqual(len(self.calls[0]), 4)
        self.assertEqual(self.calls[0][0]["type"], "completion")

    def test_invalid_batch_is_rejected_before_model_call(self):
        for page in (None, {}, self.questions * 51, [{"title": ""}]):
            with self.assertRaises(ValueError):
                self.bridge.answer(dict(self.questions[0], pageQuestions=page))
        self.assertFalse(self.calls)


class AnswerTests(unittest.TestCase):
    def test_literal_letter_option_is_not_reinterpreted(self):
        q = normalize_question({"title": "哪一个字母？", "options": "A. C\nB. A", "type": "single"})
        self.assertEqual(format_answer(q, result("A"))["answer"], "A")

    def test_letters_and_exact_option_text(self):
        q = normalize_question({"title": "质数", "options": ["A. 2", "B. 4", "C. 3"], "type": "multiple"})
        self.assertEqual(format_answer(q, result("A", "C. 3"))["answer"], "2#3")

    def test_mismatch_is_rejected(self):
        q = normalize_question({"title": "选择", "options": "A. 苹果\nB. 香蕉", "type": "single"})
        with self.assertRaises(BridgeError):
            format_answer(q, result("橘子"))

    def test_single_does_not_accept_multiple_answers(self):
        with self.assertRaises(BridgeError):
            format_answer({"title": "题目", "options": "", "type": "single"}, result("甲", "乙"))

    def test_fill_blanks_preserve_order_and_repetition(self):
        q = {"title": "填空", "options": "", "type": "completion"}
        self.assertEqual(format_answer(q, result("甲", "乙", "甲"))["answer"], "甲#乙#甲")

    def test_judgement_normalization(self):
        q = {"title": "判断", "options": "", "type": "judgement"}
        self.assertEqual(format_answer(q, result("false"))["answer"], "错误")

    def test_uncertain_answers_are_not_returned(self):
        self.assertEqual(format_answer({"title": "缺图"}, result("臆测", confident=False))["code"], 0)

    def test_bad_input(self):
        for data in ([], {"title": " "}, {"title": "a", "options": {}}, {"title": "a" * 16001}):
            with self.assertRaises(ValueError):
                normalize_question(data)


class HttpTests(unittest.TestCase):
    def setUp(self):
        self.calls = []
        def runner(questions, config):
            self.calls.append(questions)
            return batch_result(questions)
        self.bridge = Bridge({"port": 0, "token": "local-test-token", "model": "test"}, runner)
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(self.bridge))
        self.bridge.config["port"] = self.server.server_port
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base = self.bridge.base
        self.route = self.bridge.prefix

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def get(self, path, headers=None):
        with urlopen(Request(self.base + path, headers=headers or {}), timeout=5) as response:
            return json.load(response)

    def post(self, data, path=None, headers=None):
        request = Request(self.base + (path or self.route + "/answer"),
                          data=json.dumps(data).encode(),
                          headers=headers or {"Content-Type": "application/json"})
        with urlopen(request, timeout=5) as response:
            return json.load(response)

    def test_subscription_and_json_roundtrip(self):
        wrapper = self.get(self.route + "/config.json")[0]
        self.assertEqual(wrapper["method"], "post")
        q = {"title": "1+2", "options": "A. 2\nB. 3", "type": "single"}
        self.assertEqual(self.post(q)["answer"], "3")
        self.assertTrue(self.post(q)["cached"])
        self.assertEqual(len(self.calls), 1)
        q["options"] = "A. 3\nB. 2"
        self.post(q)
        self.assertEqual(len(self.calls), 2)

    def test_wrong_token_does_not_invoke_codex(self):
        with self.assertRaises(HTTPError) as error:
            self.post({"title": "1+2"}, "/t/wrong/answer")
        self.assertEqual(error.exception.code, 403)
        self.assertEqual(self.calls, [])

    def test_ocs_head_connectivity_probe(self):
        with urlopen(Request(self.base + "/?t=1", method="HEAD"), timeout=5) as response:
            self.assertEqual(response.status, 200)
            self.assertEqual(response.read(), b"")
        self.assertEqual(self.calls, [])

    def test_rebinding_host_is_rejected(self):
        with self.assertRaises(HTTPError) as error:
            self.get(self.route + "/config.json", {"Host": "evil.example"})
        self.assertEqual(error.exception.code, 403)

    def test_plain_form_is_rejected(self):
        with self.assertRaises(HTTPError) as error:
            self.post({"title": "1+2"}, headers={"Content-Type": "text/plain"})
        self.assertEqual(error.exception.code, 415)
        self.assertEqual(self.calls, [])

    def test_backend_error_is_visible_and_not_cached(self):
        def fail(question, config):
            raise BridgeError("登录失效")
        self.bridge.runner = fail
        data = self.post({"title": "1+2"})
        self.assertEqual(data["code"], 0)
        self.assertEqual(data["msg"], "登录失效")
        self.assertFalse(self.bridge.cache)

    def test_status_confirms_unlimited_deadline(self):
        self.bridge.config["timeout_seconds"] = 0
        status = self.get(self.route + "/status")
        self.assertIsNone(status["timeout_seconds"])
        self.assertTrue(status["timeout_unlimited"])


class CodexTimeoutTests(unittest.TestCase):
    def test_unlimited_and_finite_process_wait(self):
        question = {"title": "1+2", "options": "", "type": "completion"}
        for seconds, expected in [(0, None), (None, None), (600, 600)]:
            with self.subTest(seconds=seconds):
                process = Mock(returncode=0)
                process.communicate.return_value = ("", "")
                def launch(command, **kwargs):
                    output = Path(command[command.index("-o") + 1])
                    output.write_text(json.dumps(batch_result([question])))
                    return process
                config = {"codex_bin": "test-codex", "codex_home": "/tmp/ocs-test",
                          "model": "test", "reasoning_effort": "low", "timeout_seconds": seconds}
                with patch("bridge.subprocess.Popen", side_effect=launch):
                    self.assertEqual(len(codex_answer([question], config)["results"]), 1)
                self.assertEqual(process.communicate.call_args.kwargs["timeout"], expected)


if __name__ == "__main__":
    unittest.main()
