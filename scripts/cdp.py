"""极简 CDP 客户端：连到 WebView2 的远程调试端口，在真实窗口里执行 JS / 读控制台。

只依赖标准库。用法：
    python scripts/cdp.py eval "document.title"            # 在第一个页面里求值
    python scripts/cdp.py eval-all "..."                   # 所有页面
    python scripts/cdp.py pages                            # 列出页面
"""

import base64
import json
import os
import socket
import struct
import sys
import urllib.request

PORT = int(os.environ.get("CDP_PORT", "9333"))


class WS:
    def __init__(self, url):
        _, _, rest = url.partition("://")
        hostport, _, path = rest.partition("/")
        host, _, port = hostport.partition(":")
        self.sock = socket.create_connection((host, int(port or 80)), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("handshake failed")
            buf += chunk
        self.buf = buf.split(b"\r\n\r\n", 1)[1]
        self.msg_id = 0

    def _recv_exact(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, obj):
        data = json.dumps(obj).encode()
        header = bytearray([0x81])
        mask = os.urandom(4)
        n = len(data)
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header += struct.pack(">H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", n)
        header += mask
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        self.sock.sendall(bytes(header) + masked)

    def recv(self):
        b1, b2 = self._recv_exact(2)
        opcode = b1 & 0x0F
        length = b2 & 0x7F
        if length == 126:
            length = struct.unpack(">H", self._recv_exact(2))[0]
        elif length == 127:
            length = struct.unpack(">Q", self._recv_exact(8))[0]
        payload = self._recv_exact(length)
        if opcode == 0x8:
            raise RuntimeError("closed by peer")
        if opcode == 0x9:  # ping -> pong
            self.sock.sendall(b"\x8a\x80" + os.urandom(4))
            return None
        return payload

    def call(self, method, params=None):
        self.msg_id += 1
        mid = self.msg_id
        self.send({"id": mid, "method": method, "params": params or {}})
        while True:
            raw = self.recv()
            if raw is None:
                continue
            obj = json.loads(raw)
            if obj.get("id") == mid:
                return obj
            # 顺手把控制台错误打出来
            if obj.get("method") == "Runtime.consoleAPICalled":
                args = " ".join(
                    str(a.get("value", a.get("description", ""))) for a in obj["params"]["args"]
                )
                print(f"[console.{obj['params']['type']}] {args}")
            if obj.get("method") == "Runtime.exceptionThrown":
                d = obj["params"]["exceptionDetails"]
                print(f"[exception] {d.get('text')} {d.get('exception', {}).get('description', '')}")


def pages():
    with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json/list", timeout=5) as r:
        return json.load(r)


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "pages"
    if cmd == "pages":
        for p in pages():
            print(p.get("type"), p.get("title"), p.get("url"))
        return 0

    expr = sys.argv[2] if len(sys.argv) > 2 else "1"
    # 可选第 3 个参数：按标题筛窗口（两个窗口标题都叫"歌词板"，靠它区分）
    match = sys.argv[3] if len(sys.argv) > 3 else ""
    targets = [p for p in pages() if p.get("type") == "page"]
    if match:
        targets = [p for p in targets if (p.get("title") or "") == match]
    if cmd == "eval":
        if not match:
            # 没指定窗口时优先挑设置页
            targets.sort(key=lambda p: 0 if "设置" in (p.get("url") or "") + (p.get("title") or "") else 1)
        targets = targets[:1]

    for p in targets:
        ws = WS(p["webSocketDebuggerUrl"])
        ws.call("Runtime.enable")
        res = ws.call(
            "Runtime.evaluate",
            {"expression": expr, "returnByValue": True, "awaitPromise": True},
        )
        result = res.get("result", {})
        if "exceptionDetails" in result:
            print("ERROR:", json.dumps(result["exceptionDetails"], ensure_ascii=False)[:800])
        else:
            print(f"--- {p.get('title')} ---")
            print(json.dumps(result.get("result", {}).get("value"), ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
