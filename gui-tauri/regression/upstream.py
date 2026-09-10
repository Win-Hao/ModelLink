#!/usr/bin/env python3
"""假上游：记录每个请求的 method/path/headers/body 到 JSONL，返回固定响应。

2.1-A 新增：metadata.user_id 以 "effort-reject" 开头的请求，只要带 output_config
就回 400（模拟不认 output_config.effort 的兼容端点），用于验证 §3.11.1 的整流重试。

2.1-C 新增：metadata.user_id == "slow-stream" 的请求返回一个先沉默 SILENCE_SECS
再吐事件的 SSE 流，用于验证 §3.2 的心跳合流（沉默期间下游应持续收到 `: ping`）。
"""
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

OUT = sys.argv[1]
PORT = int(sys.argv[2])

REJECT_EFFORT = json.dumps({
    "type": "error",
    "error": {"type": "invalid_request_error",
              "message": "output_config: extra inputs are not permitted"},
}).encode()

OK = json.dumps({"id": "msg_test", "type": "message",
                 "content": [{"type": "text", "text": "ok"}]}).encode()

# 上游「思考中」的沉默时长。取 7s 而非设计文档验收里的 60s：
# 回归配置把心跳间隔调到 2s，7s 沉默即可观察到 3 次心跳，机制完全一样但跑得快。
SILENCE_SECS = 7


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        raw = self.rfile.read(n)
        body = json.loads(raw) if raw else None
        # 只记录代理侧决定的头（顺序无关，转 dict 排序）；剔除逐跳头
        hdrs = {k.lower(): v for k, v in self.headers.items()
                if k.lower() not in ("host", "content-length", "accept", "accept-encoding", "connection")}
        rec = {"method": "POST", "path": self.path, "headers": dict(sorted(hdrs.items())),
               "body": body}
        with open(OUT, "a") as f:
            f.write(json.dumps(rec, ensure_ascii=False, sort_keys=True) + "\n")

        tag = ((body or {}).get("metadata") or {}).get("user_id", "")
        if tag == "slow-stream":
            self.silent_sse()
        elif tag.startswith("effort-reject") and "output_config" in (body or {}):
            self.reply(400, REJECT_EFFORT)
        else:
            self.reply(200, OK)

    def silent_sse(self):
        """先沉默一段时间再吐一个事件的流式响应（模拟长思考的上游）。"""
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("transfer-encoding", "chunked")
        self.end_headers()
        self.wfile.flush()
        time.sleep(SILENCE_SECS)
        for payload in (b"event: message_stop\ndata: {}\n\n",):
            self.wfile.write(b"%x\r\n" % len(payload) + payload + b"\r\n")
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()

    def reply(self, code, payload):
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("x-upstream-echo", "1")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *a):
        pass


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
