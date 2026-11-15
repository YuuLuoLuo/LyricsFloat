"""抓词板观感：在词板后面垫一块纯白窗口，再把那块屏幕区域抓下来存成 BMP。

为什么要垫白底：词板本身是透明的，PrintWindow 抓出来四周全黑，
根本看不出面板边界、圆角和投影，也就没法核对"指针排布"的构图。

用法：python scripts/shot.py out.bmp [宽 高]
宽高可选，默认按 700~900 x 350~470 的窗口自动找。
"""

import ctypes
import struct
import sys
import time
from ctypes import wintypes

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32

# 进程默认 DPI 无感知，坐标会被系统虚拟化，量出来的和真实像素对不上
try:
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:
    pass


class BITMAPINFOHEADER(ctypes.Structure):
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


def grab(x, y, w, h):
    hdc = user32.GetDC(0)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
    gdi32.SelectObject(mem, bmp)
    gdi32.BitBlt(mem, 0, 0, w, h, hdc, x, y, 0x00CC0020)

    bi = BITMAPINFOHEADER()
    bi.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    bi.biWidth = w
    bi.biHeight = -h  # 负数 = 自上而下，省得再翻转
    bi.biPlanes = 1
    bi.biBitCount = 32
    buf = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bi), 0)

    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(0, hdc)
    return buf.raw, w, h


def save_bmp(path, x, y, w, h):
    raw, gw, gh = grab(x, y, w, h)
    hdr = struct.pack("<2sIHHI", b"BM", 14 + 40 + len(raw), 0, 0, 14 + 40)
    info = struct.pack("<IiiHHIIiiII", 40, gw, -gh, 1, 32, 0, len(raw), 2835, 2835, 0, 0)
    with open(path, "wb") as f:
        f.write(hdr + info + raw)
    print(f"saved {path} {gw}x{gh}")


def make_backdrop(x, y, w, h):
    """一块不抢焦点、纯白的窗口，垫在词板后面"""
    WS_POPUP = 0x80000000
    WS_EX_TOPMOST = 0x00000008
    user32.CreateWindowExW.restype = wintypes.HWND
    hwnd = user32.CreateWindowExW(
        WS_EX_TOPMOST, "STATIC", "", WS_POPUP, x, y, w, h, None, None, None, None
    )
    user32.ShowWindow(hwnd, 8)  # SW_SHOWNA
    user32.SetWindowPos(hwnd, ctypes.c_void_p(-1), x, y, w, h, 0x0010)  # HWND_TOPMOST
    return hwnd


def find_board(lo=600, hi=1000, hlo=250, hhi=520):
    found = []

    def cb(hwnd, _):
        if not user32.IsWindowVisible(hwnd):
            return True
        r = wintypes.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(r))
        w, h = r.right - r.left, r.bottom - r.top
        if lo < w < hi and hlo < h < hhi:
            found.append((hwnd, r.left, r.top, w, h))
        return True

    user32.EnumWindows(ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)(cb), 0)
    return found


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "shot.bmp"
    cands = find_board()
    if not cands:
        print("no board window")
        sys.exit(1)
    hwnd, x, y, w, h = cands[0]
    print("board", x, y, w, h)
    pad = 40
    back = make_backdrop(max(0, x - pad), max(0, y - pad), w + pad * 2, h + pad * 2)
    time.sleep(0.4)
    # 鼠标停在框内，让磨砂玻璃显形
    user32.SetCursorPos(x + w // 2, y + h // 2)
    time.sleep(1.4)
    save_bmp(out, max(0, x - pad), max(0, y - pad), w + pad * 2, h + pad * 2)
    user32.DestroyWindow(back)
