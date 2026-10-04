"""Compare isolated legacy/Rust bridge processes without calling a real model.

Run with the legacy service's Python interpreter. All ports and data are temporary.
The cache workload uses identical HTTP/1.1 requests with Connection: close.
"""
import argparse
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import platform
import socket
import statistics
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]


def summarize(samples):
    values = sorted(samples)
    return {
        "samples": len(values),
        "median_ms": statistics.median(values),
        "p95_ms": values[math.ceil(len(values) * .95) - 1],
        "min_ms": values[0],
        "max_ms": values[-1],
    }


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(port, body=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    start = time.perf_counter_ns()
    try:
        connection.request(
            "POST" if body else "GET",
            "/t/synthetic-benchmark-token/answer" if body else "/health",
            body=body,
            headers={"Content-Type": "application/json", "Connection": "close"},
        )
        response = connection.getresponse()
        raw = response.read()
        elapsed = (time.perf_counter_ns() - start) / 1e6
        assert response.status == 200, (response.status, raw)
        return elapsed, json.loads(raw)
    finally:
        connection.close()


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def rss_kib(process):
    return int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(process.pid)]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--legacy-root", type=Path, default=ROOT / "tests/oracle")
    parser.add_argument("--rust", type=Path, default=ROOT / "target/release/ocs-bridge")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/verification/bridge-benchmark.json")
    args = parser.parse_args()
    processes = []
    result = {
        "date": "2026-10-04",
        "host": {"os": platform.platform(), "architecture": platform.machine()},
        "python": {"executable": sys.executable, "version": platform.python_version()},
        "legacy_source_sha256": hashlib.sha256((args.legacy_root / "bridge.py").read_bytes()).hexdigest(),
        "rust_binary_sha256": hashlib.sha256(args.rust.read_bytes()).hexdigest(),
        "method": {
            "scope": "standalone bridge, not desktop UI or browser",
            "transport": "loopback HTTP; one new connection per request; Connection: close",
            "timing": "request start to complete response body; excludes client JSON encoding/decoding",
            "startup": "warm filesystem, process spawn to successful health response, 1 ms polling",
            "workload": "synthetic single-choice questions; full pageQuestions payload; all timed answers are cache hits",
            "rounds": 5,
            "requests_per_round_per_implementation": 100,
            "order": "alternating Python/Rust each round",
            "real_model_calls": 0,
            "memory": "ps RSS, only each standalone service PID; excludes desktop/browser/model",
        },
        "startup": {}, "idle_rss_kib": {}, "cache_http": [],
    }
    with tempfile.TemporaryDirectory(prefix="ocs-bridge-benchmark-") as directory:
        temp = Path(directory)
        calls = temp / "synthetic-calls.txt"
        fake = temp / "fake-codex"
        fake.write_text("#!" + sys.executable + "\n" + '''import json, os, sys
from pathlib import Path
data = json.loads(sys.stdin.read().rsplit("题目数据：\\n", 1)[1])
result = {"results": [{"id": q["id"], "answers": ["选项甲"], "confident": True, "explanation": "synthetic fixture", "sources": []} for q in data["questions"]]}
Path(sys.argv[sys.argv.index("-o") + 1]).write_text(json.dumps(result))
with open(os.environ["OCS_BENCH_CALLS"], "a") as f: f.write("synthetic\\n")
''')
        fake.chmod(0o700)
        bootstrap = temp / "legacy-service.py"
        bootstrap.write_text('''import sys
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import bridge, json
from http.server import ThreadingHTTPServer
config = json.load(open(sys.argv[2]))
server = ThreadingHTTPServer(("127.0.0.1", config["port"]), bridge.make_handler(bridge.Bridge(config)))
server.serve_forever()
''')
        env = dict(os.environ, OCS_BENCH_CALLS=str(calls), PYTHONDONTWRITEBYTECODE="1")
        configs = {}
        for name in ("python", "rust"):
            port = free_port()
            config = {"port": port, "token": "synthetic-benchmark-token", "model": "synthetic",
                      "reasoning_effort": "low", "codex_bin": str(fake), "codex_home": str(temp),
                      "timeout_seconds": 15, "max_concurrency": 2}
            path = temp / (name + ".json")
            path.write_text(json.dumps(config))
            configs[name] = (port, path)

        def launch(name):
            port, config = configs[name]
            command = ([sys.executable, str(bootstrap), str(args.legacy_root.resolve()), str(config)]
                       if name == "python" else [str(args.rust.resolve()), "serve", str(config)])
            started = time.perf_counter_ns()
            process = subprocess.Popen(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            processes.append(process)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(process.stderr.read().decode())
                try:
                    request(port)
                    return process, (time.perf_counter_ns() - started) / 1e6
                except (ConnectionError, OSError):
                    time.sleep(.001)
            raise RuntimeError("Service did not become ready")

        try:
            starts = {"python": [], "rust": []}
            # Discard the first filesystem/cache warmup for each executable.
            for name in starts:
                process, _ = launch(name)
                stop(process)
            for round_index in range(7):
                for name in (("python", "rust") if round_index % 2 == 0 else ("rust", "python")):
                    process, elapsed = launch(name)
                    starts[name].append(elapsed)
                    stop(process)
            result["startup"] = {name: summarize(values) for name, values in starts.items()}
            active = {name: launch(name)[0] for name in starts}
            result["idle_rss_kib"] = {name: rss_kib(process) for name, process in active.items()}
            for count in (1, 85, 200):
                questions = [{"title": "本地基准 %d：第 %d 题请选择选项甲。" % (count, i),
                              "options": "A. 选项甲\nB. 选项乙\nC. 选项丙\nD. 选项丁", "type": "single"}
                             for i in range(count)]
                payloads = [json.dumps(dict(q, pageQuestions=questions), ensure_ascii=False).encode()
                            for q in questions]
                samples = {"python": [], "rust": []}
                rounds = {"python": [], "rust": []}
                for name in samples:
                    _, value = request(configs[name][0], payloads[0])
                    assert value["answer"] == "选项甲" and not value["cached"]
                    for _ in range(20):
                        assert request(configs[name][0], payloads[0])[1]["cached"]
                for round_index in range(5):
                    for name in (("python", "rust") if round_index % 2 == 0 else ("rust", "python")):
                        current = []
                        for i in range(100):
                            elapsed, value = request(configs[name][0], payloads[i % count])
                            assert value["answer"] == "选项甲" and value["cached"] is True
                            current.append(elapsed)
                        samples[name].extend(current)
                        rounds[name].append(statistics.median(current))
                row = {"page_questions": count, "request_bytes": len(payloads[0]),
                       **{name: dict(summarize(values), round_medians_ms=rounds[name])
                          for name, values in samples.items()}}
                row["median_speedup"] = row["python"]["median_ms"] / row["rust"]["median_ms"]
                result["cache_http"].append(row)
                print(json.dumps(row), flush=True)
            result["warmed_rss_kib"] = {name: rss_kib(process) for name, process in active.items()}
            result["synthetic_cli_calls"] = len(calls.read_text().splitlines())
            assert result["synthetic_cli_calls"] == 6
        finally:
            for process in processes:
                stop(process)
                if process.stderr:
                    process.stderr.close()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"startup": result["startup"], "idle_rss_kib": result["idle_rss_kib"],
                      "warmed_rss_kib": result["warmed_rss_kib"], "output": str(args.output)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
