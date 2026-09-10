#!/usr/bin/env python3
"""假上游：记录每个请求的 method/path/headers/body 到 JSONL，返回固定响应。

2.1-A 新增：metadata.user_id 以 "effort-reject" 开头的请求，只要带 output_config
就回 400（模拟不认 output_config.effort 的兼容端点），用于验证 §3.11.1 的整流重试。

2.1-C 新增：metadata.user_id == "slow-stream" 的请求返回一个先沉默 SILENCE_SECS
再吐事件的 SSE 流，用于验证 §3.2 的心跳合流（沉默期间下游应持续收到 `: ping`）。

2.1-C+ 新增：
- "budget-reject"：thinking.budget_tokens < 1024 就回 400（§3.11.2）
- "sig-reject"：messages 里出现 thinking 块就回 400（§3.11.3）
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

# 上游「思考中」的沉默时长。心跳间隔固定 15s（不再是设置项），
# 所以这里必须超过 15s 才能观察到心跳；取 20s，机制与文档验收里的 60s 一致。
SILENCE_SECS = 20

REJECT_BUDGET = json.dumps({
    "type": "error",
    "error": {"type": "invalid_request_error",
              "message": "thinking.budget_tokens: Input should be greater than or equal to 1024"},
}).encode()

REJECT_SIG = json.dumps({
    "type": "error",
    "error": {"type": "invalid_request_error",
              "message": "messages.1.content.0: invalid signature on thinking block"},
}).encode()


def has_thinking_block(body):
    for m in (body or {}).get("messages", []):
        content = m.get("content")
        if isinstance(content, list):
            if any(b.get("type") in ("thinking", "redacted_thinking") for b in content):
                return True
    return False


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
        if tag == "chain-reject":
            # 连环拒收：先嫌 budget 太小，去掉 output_config 之前先修 budget；
            # 修完再嫌 thinking 块签名 —— 用来验「单请求最多整流 2 次」的组合路径
            if ((body or {}).get("thinking") or {}).get("budget_tokens", 99999) < 1024:
                self.reply(400, REJECT_BUDGET)
            elif has_thinking_block(body):
                self.reply(400, REJECT_SIG)
            else:
                self.reply(200, OK)
        elif tag == "loop-reject":
            # 无论怎么改都拒 —— 验证防循环上限：最多两次整流，共三次转发
            self.reply(400, REJECT_BUDGET)
        elif tag == "slow-stream":
            self.silent_sse()
        elif tag.startswith("budget-reject") and \
                ((body or {}).get("thinking") or {}).get("budget_tokens", 99999) < 1024:
            self.reply(400, REJECT_BUDGET)
        elif tag.startswith("sig-reject") and has_thinking_block(body):
            self.reply(400, REJECT_SIG)
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
