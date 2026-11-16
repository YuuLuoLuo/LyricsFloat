"""把窗口内容渲染成 BMP（PrintWindow）。仅用于本机观感核对，不参与程序运行。

注意：必须先声明 per-monitor DPI 感知，否则 GetWindowRect 给的是被系统
虚拟化过的坐标，抓出来只有窗口左上角一块，会被误读成"布局错了"。
"""

import ctypes
import struct
import sys
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


def find_by_size(want_w, want_h, tol=60):
    found = []

    def cb(hwnd, _):
        if not user32.IsWindowVisible(hwnd):
            return True
        r = wintypes.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(r))
        w, h = r.right - r.left, r.bottom - r.top
        if abs(w - want_w) <= tol and abs(h - want_h) <= tol:
            found.append(hwnd)
        return True

    user32.EnumWindows(ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)(cb), 0)
    return found[0] if found else None


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
    if w <= 0 or h <= 0:
        print("bad rect", w, h)
        return False

    hdc = user32.GetWindowDC(hwnd)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
    gdi32.SelectObject(mem, bmp)

    ok = user32.PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT)

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
    bi.biHeight = -h  # 负数 = 自顶向下
    bi.biPlanes = 1
    bi.biBitCount = 32
    bi.biCompression = 0

    size = w * h * 4
    buf = ctypes.create_string_buffer(size)
    gdi32.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bi), 0)

    header = struct.pack("<2sIHHI", b"BM", 14 + 40 + size, 0, 0, 14 + 40)
    info = struct.pack("<IiiHHIIiiII", 40, w, -h, 1, 32, 0, size, 2835, 2835, 0, 0)
    with open(path, "wb") as f:
        f.write(header + info + buf.raw)

    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    print(f"saved {path} {w}x{h} printwindow={ok}")
    return True


if __name__ == "__main__":
    prefix = sys.argv[1] if len(sys.argv) > 1 else "LyricsFloat"
    out = sys.argv[2] if len(sys.argv) > 2 else "win.bmp"
    if prefix.startswith("size:"):
        w, hh = prefix[5:].split("x")
        h = find_by_size(int(w), int(hh))
    else:
        h = find_window(prefix)
    if not h:
        print("no window:", prefix)
        sys.exit(1)
    grab(h, out)
