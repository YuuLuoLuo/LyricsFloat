// 程序化生成应用图标（ICO，多尺寸）。
// 避免引入任何设计素材依赖：一个深色圆角底板 + 三条歌词行，
// 第一条按当前行高亮色处理，视觉上与词板本体一致。
import { writeFileSync, mkdirSync } from 'node:fs'
import { deflateSync } from 'node:zlib'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const OUT_DIR = join(__dirname, '..', 'src-tauri', 'icons')

function roundRectContains(px, py, x0, y0, x1, y1, r) {
  if (px < x0 || px > x1 || py < y0 || py > y1) return false
  const cx = Math.min(Math.max(px, x0 + r), x1 - r)
  const cy = Math.min(Math.max(py, y0 + r), y1 - r)
  const dx = px - cx
  const dy = py - cy
  return dx * dx + dy * dy <= r * r
}

function drawIcon(size) {
  const rgba = new Uint8Array(size * size * 4)
  const k = size / 256
  const pad = 12 * k
  const x0 = pad
  const y0 = pad
  const x1 = size - pad
  const y1 = size - pad
  const radius = 54 * k

  const barH = 17 * k
  const bars = [
    { w: 122, y: 86, color: [127, 212, 255], alpha: 1 },
    { w: 150, y: 127, color: [245, 246, 250], alpha: 0.96 },
    { w: 92, y: 168, color: [245, 246, 250], alpha: 0.4 },
  ]

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4

      let cover = 0
      for (let sy = 0; sy < 2; sy++) {
        for (let sx = 0; sx < 2; sx++) {
          const px = x + (sx + 0.5) / 2
          const py = y + (sy + 0.5) / 2
          if (roundRectContains(px, py, x0, y0, x1, y1, radius)) cover += 0.25
        }
      }
      if (cover <= 0) continue

      // 底板：自上而下由亮到暗的深蓝灰
      const t = y / size
      let r = Math.round(35 - 12 * t)
      let g = Math.round(40 - 13 * t)
      let b = Math.round(56 - 18 * t)

      // 三条歌词行
      for (const bar of bars) {
        const bw = bar.w * k
        const bx0 = size / 2 - bw / 2
        const bx1 = size / 2 + bw / 2
        const by0 = bar.y * k
        const by1 = by0 + barH

        let bc = 0
        for (let sy = 0; sy < 2; sy++) {
          for (let sx = 0; sx < 2; sx++) {
            const px = x + (sx + 0.5) / 2
            const py = y + (sy + 0.5) / 2
            if (roundRectContains(px, py, bx0, by0, bx1, by1, barH / 2)) bc += 0.25
          }
        }
        if (bc > 0) {
          const a = bc * bar.alpha
          r = Math.round(r * (1 - a) + bar.color[0] * a)
          g = Math.round(g * (1 - a) + bar.color[1] * a)
          b = Math.round(b * (1 - a) + bar.color[2] * a)
        }
      }

      rgba[i] = r
      rgba[i + 1] = g
      rgba[i + 2] = b
      rgba[i + 3] = Math.round(cover * 255)
    }
  }
  return rgba
}

/** 打包成 ICO（内部用 32bpp BMP，非 PNG，兼容性最好） */
function buildIco(sizes) {
  const images = sizes.map((size) => {
    const rgba = drawIcon(size)
    const maskRow = Math.ceil(size / 32) * 4
    const header = Buffer.alloc(40)
    header.writeUInt32LE(40, 0)
    header.writeInt32LE(size, 4)
    header.writeInt32LE(size * 2, 8)
    header.writeUInt16LE(1, 12)
    header.writeUInt16LE(32, 14)
    header.writeUInt32LE(0, 16)
    header.writeUInt32LE(size * size * 4 + maskRow * size, 20)

    // ICO 的 BMP 像素是自下而上、BGRA 顺序
    const pixels = Buffer.alloc(size * size * 4)
    for (let y = 0; y < size; y++) {
      const srcRow = (size - 1 - y) * size * 4
      for (let x = 0; x < size; x++) {
        const s = srcRow + x * 4
        const d = (y * size + x) * 4
        pixels[d] = rgba[s + 2]
        pixels[d + 1] = rgba[s + 1]
        pixels[d + 2] = rgba[s]
        pixels[d + 3] = rgba[s + 3]
      }
    }
    const mask = Buffer.alloc(maskRow * size) // 全 0：alpha 已由 32bpp 通道表达
    return { size, data: Buffer.concat([header, pixels, mask]) }
  })

  const dir = Buffer.alloc(6 + images.length * 16)
  dir.writeUInt16LE(0, 0)
  dir.writeUInt16LE(1, 2)
  dir.writeUInt16LE(images.length, 4)

  let offset = dir.length
  images.forEach((img, idx) => {
    const p = 6 + idx * 16
    dir.writeUInt8(img.size >= 256 ? 0 : img.size, p)
    dir.writeUInt8(img.size >= 256 ? 0 : img.size, p + 1)
    dir.writeUInt8(0, p + 2)
    dir.writeUInt8(0, p + 3)
    dir.writeUInt16LE(1, p + 4)
    dir.writeUInt16LE(32, p + 6)
    dir.writeUInt32LE(img.data.length, p + 8)
    dir.writeUInt32LE(offset, p + 12)
    offset += img.data.length
  })

  return Buffer.concat([dir, ...images.map((i) => i.data)])
}

mkdirSync(OUT_DIR, { recursive: true })
const ico = buildIco([16, 32, 48, 64, 128, 256])
writeFileSync(join(OUT_DIR, 'icon.ico'), ico)
console.log(`icon.ico written: ${ico.length} bytes -> ${OUT_DIR}`)

/* ---------------------------------------------------------------- PNG */

const CRC_TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c >>> 0
  }
  return t
})()

function crc32(buf) {
  let c = 0xffffffff
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

function chunk(type, data) {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length, 0)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body), 0)
  return Buffer.concat([len, body, crc])
}

/** RGBA + 逐行 filter=0，打包成标准 PNG */
function buildPng(size) {
  const rgba = drawIcon(size)
  const raw = Buffer.alloc(size * (size * 4 + 1))
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0
    for (let x = 0; x < size * 4; x++) {
      raw[y * (size * 4 + 1) + 1 + x] = rgba[y * size * 4 + x]
    }
  }
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(size, 0)
  ihdr.writeUInt32BE(size, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

for (const s of [32, 64, 256]) {
  const png = buildPng(s)
  const name = s === 32 ? '32x32.png' : `icon-${s}.png`
  writeFileSync(join(OUT_DIR, name), png)
  console.log(`${name} written: ${png.length} bytes`)
}
