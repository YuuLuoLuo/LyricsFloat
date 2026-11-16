"""对窗口做"点击 + 截图"的自动化小工具，只用于本机 UI 核对。

用法：
    python scripts/ui-probe.py "LyricsFloat 设置" 860 680 click:80,106 shot:a.bmp sleep:400 shot:b.bmp

坐标是逻辑像素（和 CSS 一致），内部按窗口实际宽度换算成物理像素。
"""

import ctypes
import struct
import sys
import time
from ctypes import wintypes

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32

try:
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
    except Exception:
        pass

PW_RENDERFULLCONTENT = 0x00000002


def find_window(prefix):
    found = []

    def cb(hwnd, _):
        if not user32.IsWindowVisible(hwnd):
            return True
        n = user32.GetWindowTextLengthW(hwnd)
        if n <= 0:
            return True
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.GetWindowTextW(hwnd, buf, n + 1)
        if buf.value.startswith(prefix):
            found.append(hwnd)
        return True

    user32.EnumWindows(ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)(cb), 0)
    return found[0] if found else None


def grab(hwnd, path):
    r = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    w, h = r.right - r.left, r.bottom - r.top
    hdc = user32.GetWindowDC(hwnd)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
    gdi32.SelectObject(mem, bmp)
    user32.PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT)

    class BMIH(ctypes.Structure):
        _fields_ = [
            ("biSize", wintypes.DWORD),
            ("biWidth", wintypes.LONG),
            ("biHeight", wintypes.LONG),
            ("biPlanes", wintypes.WORD),
            ("biBitCount", wintypes.WORD),
            ("biCompression", wintypes.DWORD),
            ("biSizeImage", wintypes.DWORD),
            ("biXPelsPerMeter", wintypes.LONG),
            ("biYPelsPerMeter", wintypes.LONG),
            ("biClrUsed", wintypes.DWORD),
            ("biClrImportant", wintypes.DWORD),
        ]

    bi = BMIH()
    bi.biSize = ctypes.sizeof(BMIH)
    bi.biWidth = w
    bi.biHeight = -h
    bi.biPlanes = 1
    bi.biBitCount = 32
    bi.biCompression = 0
    size = w * h * 4
    buf = ctypes.create_string_buffer(size)
    gdi32.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bi), 0)

    header = struct.pack("<2sIHHI", b"BM", 14 + 40 + size, 0, 0, 14 + 40)
    info = struct.pack("<IiiHHIIiiII", 40, w, -h, 1, 32, 0, size, 2835, 2835, 0, 0)
    open(path, "wb").write(header + info + buf.raw)

    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    print(f"shot {path} {w}x{h}")


def click(x, y):
    user32.SetCursorPos(x, y)
    time.sleep(0.06)
    user32.mouse_event(0x0002, 0, 0, 0, 0)  # LEFTDOWN
    time.sleep(0.06)
    user32.mouse_event(0x0004, 0, 0, 0, 0)  # LEFTUP
    print(f"click {x},{y}")


def main():
    prefix = sys.argv[1]
    logical_w = float(sys.argv[2])
    logical_h = float(sys.argv[3])
    actions = sys.argv[4:]

    hwnd = find_window(prefix)
    if not hwnd:
        print("no window:", prefix)
        return 1
    user32.SetForegroundWindow(hwnd)
    time.sleep(0.35)

    r = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    scale = (r.right - r.left) / logical_w

    cx, cy = (r.left + r.right) // 2, r.top + 30
    for act in actions:
        kind, _, arg = act.partition(":")
        if kind == "click":
            lx, ly = (float(v) for v in arg.split(","))
            click(int(r.left + lx * scale), int(r.top + ly * scale))
            time.sleep(0.5)
        elif kind == "move":
            lx, ly = (float(v) for v in arg.split(","))
            user32.SetCursorPos(int(r.left + lx * scale), int(r.top + ly * scale))
            time.sleep(0.4)
        elif kind == "shot":
            time.sleep(0.25)
            grab(hwnd, arg)
        elif kind == "sleep":
            time.sleep(int(arg) / 1000)
    return 0


if __name__ == "__main__":
    sys.exit(main())
