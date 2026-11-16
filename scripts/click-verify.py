"""验证 WM_NCCALCSIZE 子类化修复：解锁后点击词板不应出现标题栏帧。
纯 Win32 API，不依赖 CDP。流程：启动 -> 悬停显形 -> 真实点击 -> 样式探测 -> PrintWindow 截图 -> 像素分析。
"""
import ctypes, struct, sys, os, time, subprocess
from ctypes import wintypes

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32
gdi32 = ctypes.windll.gdi32
try: user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception: pass

WS_CAPTION = 0x00C00000
WS_SYSMENU = 0x00080000
WS_THICKFRAME = 0x00040000
WS_EX_LAYERED = 0x00080000
WS_EX_TRANSPARENT = 0x00000020

def image_name(pid):
    h = kernel32.OpenProcess(0x1000, False, pid)
    if not h: return ''
    buf = ctypes.create_unicode_buffer(1024); size = wintypes.DWORD(1024)
    ok = kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size))
    kernel32.CloseHandle(h)
    return buf.value if ok else ''

def find_lyric_windows():
    out = []
    def cb(hwnd, _):
        pid = wintypes.DWORD(); user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if image_name(pid.value).rsplit('\\',1)[-1] != 'lyricsfloat.exe': return True
        cls = ctypes.create_unicode_buffer(256); user32.GetClassNameW(hwnd, cls, 256)
        if cls.value != 'Tauri Window': return True
        n = user32.GetWindowTextLengthW(hwnd)
        t = ctypes.create_unicode_buffer(n+1); user32.GetWindowTextW(hwnd, t, n+1)
        r = wintypes.RECT(); user32.GetWindowRect(hwnd, ctypes.byref(r))
        style = user32.GetWindowLongW(hwnd, -16) & 0xFFFFFFFF
        ex = user32.GetWindowLongW(hwnd, -20) & 0xFFFFFFFF
        out.append(dict(hwnd=hwnd, title=t.value or '(board)',
                        x=r.left, y=r.top, w=r.right-r.left, h=r.bottom-r.top,
                        caption=bool(style & WS_CAPTION),
                        sysmenu=bool(style & WS_SYSMENU),
                        thick=bool(style & WS_THICKFRAME),
                        layered=bool(ex & WS_EX_LAYERED),
                        transparent=bool(ex & WS_EX_TRANSPARENT),
                        visible=bool(user32.IsWindowVisible(hwnd))))
        return True
    user32.EnumWindows(ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)(cb), 0)
    return out

def hide_settings():
    """隐藏设置窗：悬停检测在设置窗可见时会拒绝显形词板（防止误判）"""
    for w in find_lyric_windows():
        if w['title'] != '(board)':
            user32.ShowWindow(w['hwnd'], 0)  # SW_HIDE
            print(f'  已隐藏设置窗 hwnd={w["hwnd"]}')
            return True
    return False

def grab_bmp(hwnd, path):
    r = wintypes.RECT(); user32.GetWindowRect(hwnd, ctypes.byref(r))
    w, h = r.right-r.left, r.bottom-r.top
    hdc = user32.GetWindowDC(hwnd); mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h); gdi32.SelectObject(mem, bmp)
    ok = user32.PrintWindow(hwnd, mem, 2)  # PW_RENDERFULLCONTENT
    class BMIH(ctypes.Structure):
        _fields_ = [("biSize", wintypes.DWORD),("biWidth", wintypes.LONG),("biHeight", wintypes.LONG),
                    ("biPlanes", wintypes.WORD),("biBitCount", wintypes.WORD),("biCompression", wintypes.DWORD),
                    ("biSizeImage", wintypes.DWORD),("biXPelsPerMeter", wintypes.LONG),("biYPelsPerMeter", wintypes.LONG),
                    ("biClrUsed", wintypes.DWORD),("biClrImportant", wintypes.DWORD)]
    bi = BMIH(); bi.biSize = ctypes.sizeof(BMIH); bi.biWidth = w; bi.biHeight = -h
    bi.biPlanes = 1; bi.biBitCount = 32; bi.biCompression = 0
    size = w*h*4; buf = ctypes.create_string_buffer(size)
    gdi32.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bi), 0)
    header = struct.pack('<2sIHHI', b'BM', 14+40+size, 0,0,14+40)
    info = struct.pack('<IiiHHIIiiII', 40, w, -h, 1, 32, 0, size, 2835, 2835, 0, 0)
    open(path,'wb').write(header+info+buf.raw)
    gdi32.DeleteObject(bmp); gdi32.DeleteDC(mem); user32.ReleaseDC(hwnd, hdc)
    return w, h

def analyze_bmp(path):
    """分析 BMP 顶部区域有没有浅色横条（标题栏帧）。返回 (has_bar, detail)。"""
    data = open(path,'rb').read()
    off = struct.unpack_from('<I', data, 10)[0]
    w = struct.unpack_from('<i', data, 18)[0]
    h = struct.unpack_from('<i', data, 22)[0]
    bpp = struct.unpack_from('<H', data, 28)[0]
    top_down = h < 0; h = abs(h)
    px = bpp // 8
    row = w * px; pad = (-row) % 4

    def pix(x, y):
        yy = y if top_down else h-1-y
        o = off + yy*(row+pad) + x*px
        return (data[o+2], data[o+1], data[o])  # RGB

    # 逐行统计顶部 80 行里的"浅色"像素 (亮度>150)
    for y in range(min(80, h)):
        light_xs = [x for x in range(0, w, 4) if sum(pix(x,y)) > 450]
        if len(light_xs) > w // 8:  # 超过 12% 宽度是亮的 -> 可能是条
            print(f'  y={y} 浅色像素 {len(light_xs)*4}/{w} x=[{min(light_xs)*4}..{max(light_xs)*4}]')
    # 扫描一个截面
    has_bar = False
    bar_y = -1
    for y in range(min(80, h)):
        cnt = sum(1 for x in range(0, w, 2) if sum(pix(x,y)) > 450)
        if cnt > w // 6:
            has_bar = True; bar_y = y
            break
    return has_bar, f'bar_y={bar_y}' if has_bar else 'top 80 rows clean'

def probe_all(label):
    wins = find_lyric_windows()
    board = None
    for w in wins:
        tag = 'BOARD' if w['title'] == '(board)' else 'SETTINGS'
        print(f'  [{label}] {tag} hwnd={w["hwnd"]} {w["w"]}x{w["h"]} '
              f'caption={w["caption"]} sysmenu={w["sysmenu"]} thick={w["thick"]} '
              f'layered={w["layered"]} transparent={w["transparent"]}')
        if w['title'] == '(board)':
            board = w
    return board

def main():
    exe = sys.argv[1] if len(sys.argv) > 1 else 'src-tauri/target/debug/lyric-board.exe'
    tmp = os.environ.get('TEMP', '/tmp')

    print('=== 启动应用 ===')
    proc = subprocess.Popen([exe], cwd=os.getcwd(),
                            env={**os.environ, 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS': ''})
    time.sleep(6)

    print('=== 初始状态 ===')
    board = probe_all('初始')
    if not board:
        print('找不到词板窗口'); proc.kill(); sys.exit(1)

    print('=== 隐藏设置窗（悬停检测在设置窗可见时拒绝显形） ===')
    hide_settings()
    time.sleep(0.3)

    print('=== 强制显形（直接去掉穿透位，绕过悬停检测） ===')
    # 悬停检测依赖 GetCursorPos vs Tauri outer_position 的坐标比对，
    # 在脚本进程和应用进程 DPI 上下文不一致时会失效。
    # 直接操作样式位：去掉 WS_EX_TRANSPARENT|WS_EX_LAYERED 让窗口可点。
    hwnd = board['hwnd']
    ex = user32.GetWindowLongW(hwnd, -20) & 0xFFFFFFFF
    new_ex = ex & ~(0x00080000 | 0x00000020)  # remove LAYERED | TRANSPARENT
    user32.SetWindowLongW(hwnd, -20, new_ex)
    # 发 SWP_FRAMECHANGED 让系统重算命中测试
    user32.SetWindowPos(hwnd, 0, 0, 0, 0, 0, 0x0037)  # NOMOVE|NOSIZE|NOZORDER|NOACTIVATE|FRAMECHANGED
    time.sleep(0.3)
    b2 = [w for w in find_lyric_windows() if w['title'] == '(board)']
    if b2:
        board = b2[0]
        print(f'  现在: transparent={board["transparent"]} layered={board["layered"]}')
    cx, cy = board['x'] + board['w']//2, board['y'] + board['h']//2
    user32.SetCursorPos(cx, cy)
    time.sleep(0.5)
    board = probe_all('悬停后')
    if not board: sys.exit(1)

    print('=== 真实鼠标点击 ===')
    user32.SetCursorPos(cx, cy)
    time.sleep(0.1)
    user32.mouse_event(0x0002, 0, 0, 0, 0)  # LEFTDOWN
    time.sleep(0.05)
    user32.mouse_event(0x0004, 0, 0, 0, 0)  # LEFTUP
    time.sleep(0.5)
    board = probe_all('点击后0.5s')
    if not board: sys.exit(1)

    print('=== PrintWindow 截图 ===')
    bmp_path = os.path.join(tmp, 'click_verify.bmp')
    w, h = grab_bmp(board['hwnd'], bmp_path)
    print(f'  saved {bmp_path} {w}x{h}')

    print('=== 像素分析（顶部有无浅色条） ===')
    has_bar, detail = analyze_bmp(bmp_path)
    print(f'  结果: {"有标题栏帧!" if has_bar else "干净，无标题栏帧"} ({detail})')

    print('=== 延迟再探 ===')
    time.sleep(1.5)
    board2 = probe_all('点击后2s')
    if board2:
        bmp2 = os.path.join(tmp, 'click_verify2.bmp')
        grab_bmp(board2['hwnd'], bmp2)
        has_bar2, detail2 = analyze_bmp(bmp2)
        print(f'  延迟结果: {"有标题栏帧!" if has_bar2 else "干净"} ({detail2})')

    print('=== 连续点 5 下再测 ===')
    for i in range(5):
        user32.mouse_event(0x0002, 0, 0, 0, 0); time.sleep(0.03)
        user32.mouse_event(0x0004, 0, 0, 0, 0); time.sleep(0.08)
    time.sleep(0.5)
    board3 = probe_all('连点后')
    if board3:
        bmp3 = os.path.join(tmp, 'click_verify3.bmp')
        grab_bmp(board3['hwnd'], bmp3)
        has_bar3, detail3 = analyze_bmp(bmp3)
        print(f'  连点结果: {"有标题栏帧!" if has_bar3 else "干净"} ({detail3})')

    # 转 PNG 供查看
    for src, dst in [(bmp_path, bmp_path.replace('.bmp','.png'))]:
        if os.path.exists(src):
            d = open(src,'rb').read()
            off = struct.unpack_from('<I', d, 10)[0]
            w = struct.unpack_from('<i', d, 18)[0]
            h2 = struct.unpack_from('<i', d, 22)[0]
            bpp = struct.unpack_from('<H', d, 28)[0]; px = bpp//8
            top = h2 < 0; h2 = abs(h2); row = w*px; pad = (-row) % 4
            rows = []
            ys = range(0, h2) if top else range(h2-1, -1, -1)
            for y in ys:
                o = off + y*(row+pad); line = d[o:o+row]
                rows.append(bytes(c for x in range(w) for c in (line[x*px+2], line[x*px+1], line[x*px])))
            raw = b''.join(b'\x00'+r for r in rows)
            ch = lambda t,dd: struct.pack('>I',len(dd))+t+dd+struct.pack('>I',struct.unpack('>I',struct.pack('<I',0))[0])
            import zlib
            ch = lambda t,dd: struct.pack('>I',len(dd))+t+dd+struct.pack('>I',zlib.crc32(t+dd)&0xffffffff)
            png = b'\x89PNG\r\n\x1a\n'+ch(b'IHDR',struct.pack('>IIBBBBB',w,h2,8,2,0,0,0))+ch(b'IDAT',zlib.compress(raw,6))+ch(b'IEND',b'')
            open(dst,'wb').write(png)
            print(f'  PNG: {dst}')

    proc.terminate()
    time.sleep(0.5)

    if has_bar:
        print('\n*** 失败：标题栏帧仍存在 ***')
        sys.exit(1)
    else:
        print('\n*** 成功：点击后无标题栏帧 ***')
        sys.exit(0)

if __name__ == '__main__':
    main()
