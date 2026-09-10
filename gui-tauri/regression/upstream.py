#!/usr/bin/env python3
"""假上游：记录每个请求的 method/path/headers/body 到 JSONL，返回固定响应。

2.1-A 新增：metadata.user_id 以 "effort-reject" 开头的请求，只要带 output_config
就回 400（模拟不认 output_config.effort 的兼容端点），用于验证 §3.11.1 的整流重试。
"""
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

OUT = sys.argv[1]
PORT = int(sys.argv[2])

REJECT_EFFORT = json.dumps({
    "type": "error",
    "error": {"type": "invalid_request_error",
              "message": "output_config: extra inputs are not permitted"},
}).encode()

OK = json.dumps({"id": "msg_test", "type": "message",
                 "content": [{"type": "text", "text": "ok"}]}).encode()


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
        if tag.startswith("effort-reject") and "output_config" in (body or {}):
            self.reply(400, REJECT_EFFORT)
        else:
            self.reply(200, OK)

    def reply(self, code, payload):
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("x-upstream-echo", "1")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *a):
        pass


HTTPServer(("127.0.0.1", PORT), H).serve_forever()
